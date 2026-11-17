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

//! Subsonic scrobbling.
//!
//! The server owns play counts. Playback reports a now-playing notification
//! on start and one completed submission once a track has been heard for
//! `min(4 minutes, half its duration)`. A failed/offline submission is queued
//! in `status.db` and retried by the health probe, oldest first. The local
//! count is not incremented: the next library sync receives the server's
//! authoritative value.

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;

use crate::db::Dbs;

/// Last.fm-style threshold: four minutes or half the track, whichever comes
/// first. Unknown durations wait four minutes; `0` would submit instantly.
pub fn threshold_secs(duration: f64) -> f64 {
    if duration > 0.0 {
        (duration * 0.5).clamp(1.0, 240.0)
    } else {
        240.0
    }
}

/// Best-effort now-playing notification. Offline is a silent no-op.
pub async fn now_playing(dbs: &Dbs, song_id: &str) {
    if song_id.is_empty() || !crate::session::online() {
        return;
    }
    let Ok(client) = crate::session::client(dbs).await else {
        return;
    };
    if let Err(error) = client.scrobble(song_id, false, None).await {
        crate::debug_log!("scrobble: now-playing {song_id} failed: {error:#}");
    }
}

/// Submits one completed play; queues it for retry if the server is not
/// reachable (or answers with an error).
pub async fn submit(dbs: &Dbs, song_id: &str, played_at: i64) {
    if song_id.is_empty() {
        return;
    }
    if !crate::session::online() {
        queue(dbs, song_id, played_at).await;
        return;
    }
    let Ok(client) = crate::session::client(dbs).await else {
        queue(dbs, song_id, played_at).await;
        return;
    };
    match client
        .scrobble(song_id, true, Some(played_at.saturating_mul(1000)))
        .await
    {
        Ok(()) => crate::debug_log!("scrobble: submitted {song_id}"),
        Err(error) => {
            crate::debug_log!("scrobble: {song_id} queued after failure: {error:#}");
            queue(dbs, song_id, played_at).await;
        }
    }
}

async fn queue(dbs: &Dbs, song_id: &str, played_at: i64) {
    let result = sqlx::query(
        "INSERT INTO pending_scrobble (song_id, played_at, created_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(song_id, played_at) DO NOTHING",
    )
    .bind(song_id)
    .bind(played_at)
    .bind(crate::consts::now_secs())
    .execute(&dbs.status)
    .await;
    if let Err(error) = result {
        eprintln!("scrobble: could not queue {song_id}: {error:#}");
    }
}

/// Flushes at most 50 queued submissions. Stops at the first failure so the
/// next probe retries in order instead of hammering a server that is down.
pub async fn flush(dbs: &Dbs) -> Result<usize> {
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, song_id, played_at FROM pending_scrobble
         ORDER BY created_at, id LIMIT 50",
    )
    .fetch_all(&dbs.status)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    if !crate::session::online() {
        return Ok(0);
    }
    let client = crate::session::client(dbs).await?;
    let mut sent = 0usize;
    for (id, song_id, played_at) in rows {
        match client
            .scrobble(&song_id, true, Some(played_at.saturating_mul(1000)))
            .await
        {
            Ok(()) => {
                sqlx::query("DELETE FROM pending_scrobble WHERE id = ?1")
                    .bind(id)
                    .execute(&dbs.status)
                    .await?;
                sent += 1;
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE pending_scrobble
                     SET attempts = attempts + 1, last_error = ?2
                     WHERE id = ?1",
                )
                .bind(id)
                .bind(format!("{error:#}"))
                .execute(&dbs.status)
                .await?;
                break;
            }
        }
    }
    Ok(sent)
}

/// Number of queued submissions (diagnostics/tests).
pub async fn pending_count(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM pending_scrobble")
        .fetch_one(pool)
        .await
        .context("count pending scrobbles")
}
