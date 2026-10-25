// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! DB9 `download.db`: the download ledger (CACHE-8).
//!
//! `media.db` owns the artifacts (`media`) and the container selections
//! (`media_rule`); this database owns the per-song intent and progress —
//! `queued → active → done | error`, the failure text, the selecting rule and
//! the timestamps. CACHE-2's fetcher is the writer, CACHE-7's UI the reader;
//! rows are pruned with the other caches after a successful sync.

// The prefetch/status consumers land with CACHE-2/7; until then some of this
// surface is exercised by tests only (same stance as `covers.rs`).
#![allow(dead_code)]

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

/// Waiting for the fetcher.
pub const QUEUED: &str = "queued";
/// A fetch is in flight.
pub const ACTIVE: &str = "active";
/// Blob stored; the row survives rule removal (artifacts are media.db's).
pub const DONE: &str = "done";
/// Last fetch failed; `error` carries the reason for the retry UI.
pub const ERROR: &str = "error";

/// One ledger row.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, serde::Serialize)]
pub struct DownloadState {
    pub song_id: String,
    pub state: String,
    pub error: Option<String>,
    pub rule_id: Option<String>,
    pub requested_at: i64,
    pub updated_at: i64,
}

async fn set_download_state(
    pool: &SqlitePool,
    song_id: &str,
    state: &str,
    error: Option<&str>,
    rule_id: Option<&str>,
) -> Result<()> {
    let now = crate::consts::now_secs() as i64;
    sqlx::query(
        "INSERT INTO download (song_id, state, error, rule_id, requested_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(song_id) DO UPDATE SET
             state      = CASE WHEN excluded.state = 'queued' AND download.state = 'done'
                               THEN download.state ELSE excluded.state END,
             error      = CASE WHEN excluded.state = 'queued' AND download.state = 'done'
                               THEN download.error ELSE excluded.error END,
             rule_id    = COALESCE(excluded.rule_id, download.rule_id),
             updated_at = excluded.updated_at",
    )
    .bind(song_id)
    .bind(state)
    .bind(error)
    .bind(rule_id)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Marks a song for download. A `done` row is never downgraded; anything else
/// resets to `queued` and clears the old error (re-selection retries it).
pub async fn mark_queued(pool: &SqlitePool, song_id: &str, rule_id: Option<&str>) -> Result<()> {
    set_download_state(pool, song_id, QUEUED, None, rule_id).await
}

/// Records a state transition (`active`/`done`/`error`), creating the row when
/// the fetcher reports progress for a song that was never explicitly marked.
pub async fn set_state(
    pool: &SqlitePool,
    song_id: &str,
    state: &str,
    error: Option<&str>,
) -> Result<()> {
    set_download_state(pool, song_id, state, error, None).await
}

/// One row; `None` when the song was never marked.
pub async fn get(pool: &SqlitePool, song_id: &str) -> Result<Option<DownloadState>> {
    let row = sqlx::query_as::<_, DownloadState>(
        "SELECT song_id, state, error, rule_id, requested_at, updated_at
         FROM download WHERE song_id = ?1",
    )
    .bind(song_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// The next `limit` queued songs, oldest request first (the fetcher's work
/// list).
pub async fn queued_ids(pool: &SqlitePool, limit: i64) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar(
        "SELECT song_id FROM download
         WHERE state = 'queued' ORDER BY requested_at LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// Every song whose download finished (CACHE-9) — the only catalog the app
/// offers while the server is unreachable.
pub async fn done_ids(pool: &SqlitePool) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT song_id FROM download WHERE state = 'done'")
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// The downloaded catalog while offline; `None` when online.
pub async fn offline_song_ids(
    dbs: &crate::db::Dbs,
    offline: bool,
) -> Result<Option<std::collections::HashSet<String>>> {
    if !offline {
        return Ok(None);
    }
    Ok(Some(done_ids(&dbs.download).await?.into_iter().collect()))
}

/// The finished subset of `ids` — the row marks' window read. One batched
/// query; an empty window costs nothing.
pub async fn done_ids_of(pool: &SqlitePool, ids: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(500) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT song_id FROM download WHERE state = 'done' AND song_id IN (",
        );
        crate::cache::push_in_list(&mut builder, chunk);
        let rows = builder
            .build_query_scalar::<String>()
            .fetch_all(pool)
            .await?;
        out.extend(rows);
    }
    Ok(out)
}

/// The in-flight subset of `ids` (`queued`/`active`) — the row marks show
/// the loading glyph for these while the fetch runs.
pub async fn pending_ids_of(pool: &SqlitePool, ids: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for chunk in ids.chunks(500) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT song_id FROM download WHERE state IN ('queued', 'active') AND song_id IN (",
        );
        crate::cache::push_in_list(&mut builder, chunk);
        let rows = builder
            .build_query_scalar::<String>()
            .fetch_all(pool)
            .await?;
        out.extend(rows);
    }
    Ok(out)
}

/// Per-state counts for the status UI (`CACHE-7`): queued / active / done /
/// error / total.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Counts {
    pub queued: i64,
    pub active: i64,
    pub done: i64,
    pub error: i64,
    pub total: i64,
}

pub async fn counts(pool: &SqlitePool) -> Result<Counts> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT state, COUNT(*) FROM download GROUP BY state")
            .fetch_all(pool)
            .await?;
    let mut counts = Counts::default();
    for (state, count) in rows {
        counts.total += count;
        match state.as_str() {
            QUEUED => counts.queued = count,
            ACTIVE => counts.active = count,
            DONE => counts.done = count,
            ERROR => counts.error = count,
            _ => {}
        }
    }
    Ok(counts)
}

/// Drops one song's mark (the user unselected it). The blob, when already
/// stored, is media.db's decision (kind/pin), not this layer's.
pub async fn unmark(pool: &SqlitePool, song_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM download WHERE song_id = ?1")
        .bind(song_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The songs a rule's marks cover (rule removal, CACHE-5): the ledger's
/// ownership is what identifies a pre-rule blob that must leave with the
/// rule too.
pub async fn rule_song_ids(pool: &SqlitePool, rule_id: &str) -> Result<Vec<String>> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT song_id FROM download WHERE rule_id = ?1")
        .bind(rule_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Drops every row a rule requested (rule removal, CACHE-5). Removing the
/// rule also removes its blobs, so pending *and* done marks go together;
/// explicit downloads carry no rule id and are never touched.
pub async fn unmark_rule(pool: &SqlitePool, rule_id: &str) -> Result<usize> {
    let result = sqlx::query("DELETE FROM download WHERE rule_id = ?1")
        .bind(rule_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() as usize)
}
