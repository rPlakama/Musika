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

//! DB4 `media.db`: the offline audio cache index (CACHE-1, M0).
//!
//! Storage layer only. Blobs are fetched by the playback/prefetch path
//! (CACHE-2) and written through [`store`]; playback resolves local copies
//! with [`cached_path`]/[`path_map_of`] and never waits on the network for them.
//! Pruning happens only after a fully successful sync (the covers.rs policy):
//! vanished songs first, then rule-orphans; tier caps are CACHE-3.

// The fetcher/prefetch/status consumers land with CACHE-2/3/7; until then some
// of this surface is exercised by tests only (same stance as `covers.rs`).
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;

/// Playback-tier blob: fetched around the current queue, purgeable.
pub const KIND_PLAYBACK: &str = "playback";
/// Rolling-tier blob: bounded LRU cache, purgeable.
pub const KIND_ROLLING: &str = "rolling";
/// Permanent-tier blob: never auto-removed.
pub const KIND_PERMANENT: &str = "permanent";
/// Rule-owned blob: kept while its [`media_rule`] lives.
pub const KIND_RULE: &str = "rule";

/// The tier that owns the blob for `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    /// `~/.config/musika/media/` — permanent and rule blobs.
    Config,
    /// `~/.cache/musika/media/` — playback and rolling blobs.
    Cache,
}

pub(crate) fn tier_for(kind: &str) -> Tier {
    match kind {
        KIND_PERMANENT | KIND_RULE => Tier::Config,
        _ => Tier::Cache,
    }
}

fn tier_dir(kind: &str) -> Result<PathBuf> {
    match tier_for(kind) {
        Tier::Config => crate::db::media_dir(),
        Tier::Cache => crate::db::playback_dir(),
    }
}

/// The fingerprint invalidates a stored blob when the song's bytes metadata,
/// the server or the quality setting change — the `covers.hash` role.
pub fn fingerprint(
    size: Option<i64>,
    suffix: Option<&str>,
    bit_rate: Option<i64>,
    server: &str,
    quality: &str,
) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        size.unwrap_or(0),
        suffix.unwrap_or(""),
        bit_rate.unwrap_or(0),
        server,
        quality
    )
}

/// One stored blob row, resolved lazily against the tier directory.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Stored {
    song_id: String,
    kind: String,
    file: String,
}

async fn get(pool: &SqlitePool, song_id: &str) -> Result<Option<Stored>> {
    let row =
        sqlx::query_as::<_, Stored>("SELECT song_id, kind, file FROM media WHERE song_id = ?1")
            .bind(song_id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

/// Upserts one media row. `file` is the blob name relative to its tier dir.
#[allow(clippy::too_many_arguments)]
pub async fn record(
    pool: &SqlitePool,
    song_id: &str,
    kind: &str,
    file: &str,
    size: i64,
    quality: &str,
    fingerprint: &str,
    rule_id: Option<&str>,
) -> Result<()> {
    let now = crate::consts::now_secs().to_string();
    sqlx::query(
        "INSERT INTO media
             (song_id, kind, file, size, quality, fingerprint, rule_id, last_used, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT(song_id) DO UPDATE SET
             kind        = excluded.kind,
             file        = excluded.file,
             size        = excluded.size,
             quality     = excluded.quality,
             fingerprint = excluded.fingerprint,
             rule_id     = excluded.rule_id,
             last_used   = excluded.last_used",
    )
    .bind(song_id)
    .bind(kind)
    .bind(file)
    .bind(size)
    .bind(quality)
    .bind(fingerprint)
    .bind(rule_id)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// A unique `.part` path in the blob's tier directory.
///
/// A download streams into this file and [`store_streamed`] renames it into
/// place. Staying on the same filesystem keeps the rename atomic, and an
/// aborted fetch leaves only a `.part`, which [`purge`] already sweeps.
pub fn temp_path(kind: &str) -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_GEN: AtomicU64 = AtomicU64::new(0);
    let dir = tier_dir(kind)?;
    crate::db::ensure_dir(&dir)?;
    Ok(dir.join(format!(
        ".{}.{}.{}.part",
        std::process::id(),
        crate::consts::now_nanos(),
        TEMP_GEN.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Writes a fetched blob (atomic rename) and records it. Returns the final
/// path. A previous blob for the same song is replaced, never orphaned.
///
/// Kept for callers that already hold the bytes (tests and the future
/// playback prefetch); the download path uses [`temp_path`] +
/// [`store_streamed`] so a whole song is never resident.
#[allow(clippy::too_many_arguments)]
pub async fn store(
    pool: &SqlitePool,
    song_id: &str,
    bytes: &[u8],
    kind: &str,
    quality: &str,
    fingerprint: &str,
    extension: &str,
    rule_id: Option<&str>,
) -> Result<PathBuf> {
    let temp = temp_path(kind)?;
    if let Err(err) = std::fs::write(&temp, bytes) {
        let _ = std::fs::remove_file(&temp);
        return Err(err).with_context(|| format!("failed to write {}", temp.display()));
    }
    store_streamed(
        pool,
        song_id,
        &temp,
        bytes.len() as i64,
        kind,
        quality,
        fingerprint,
        extension,
        rule_id,
    )
    .await
}

/// Adopts an already-streamed temporary blob: renames it to the canonical
/// name and records the row. The source must live in the same tier directory
/// (see [`temp_path`]).
#[allow(clippy::too_many_arguments)]
pub async fn store_streamed(
    pool: &SqlitePool,
    song_id: &str,
    source: &Path,
    size: i64,
    kind: &str,
    quality: &str,
    fingerprint: &str,
    extension: &str,
    rule_id: Option<&str>,
) -> Result<PathBuf> {
    let dir = tier_dir(kind)?;
    crate::db::ensure_dir(&dir)?;
    store_in_dir(
        pool,
        &dir,
        song_id,
        source,
        size,
        kind,
        quality,
        fingerprint,
        extension,
        rule_id,
    )
    .await
}

/// The directory-taking half of [`store_streamed`] (also the test seam).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn store_in_dir(
    pool: &SqlitePool,
    dir: &Path,
    song_id: &str,
    source: &Path,
    size: i64,
    kind: &str,
    quality: &str,
    fingerprint: &str,
    extension: &str,
    rule_id: Option<&str>,
) -> Result<PathBuf> {
    let file = format!(
        "{}.{}",
        crate::digest::md5_hex(song_id.as_bytes())?,
        extension
    );
    let final_path = dir.join(&file);
    let previous = get(pool, song_id).await?;

    if let Err(err) = std::fs::rename(source, &final_path) {
        let _ = std::fs::remove_file(source);
        return Err(err).with_context(|| format!("failed to move {}", final_path.display()));
    }

    if let Err(err) = record(
        pool,
        song_id,
        kind,
        &file,
        size,
        quality,
        fingerprint,
        rule_id,
    )
    .await
    {
        // The row did not land. Drop the new blob unless it replaced the
        // previous row's own file (then the old row still names these bytes).
        if previous
            .as_ref()
            .is_none_or(|old| old.file != file || old.kind != kind)
        {
            let _ = std::fs::remove_file(&final_path);
        }
        return Err(err);
    }

    if let Some(old) = previous {
        if old.file != file || old.kind != kind {
            if let Ok(old_dir) = tier_dir(&old.kind) {
                let _ = std::fs::remove_file(old_dir.join(&old.file));
            }
        }
    }
    Ok(final_path)
}

/// Whether the song holds a config-tier download (permanent/rule row) — the
/// Download toggle's state. Distinct from [`cached_path`], which also counts
/// the purgeable playback tier a play may have left behind.
pub async fn downloaded(pool: &SqlitePool, song_id: &str) -> Result<bool> {
    Ok(get(pool, song_id)
        .await?
        .is_some_and(|row| tier_for(&row.kind) == Tier::Config))
}

/// Absolute path to the cached copy, when the row and the file both exist.
pub async fn cached_path(pool: &SqlitePool, song_id: &str) -> Result<Option<PathBuf>> {
    let Some(row) = get(pool, song_id).await? else {
        return Ok(None);
    };
    let path = tier_dir(&row.kind)?.join(&row.file);
    Ok(path.is_file().then_some(path))
}

/// Marks a blob as recently used (CACHE-3's LRU input).
pub async fn touch(pool: &SqlitePool, song_id: &str) -> Result<()> {
    sqlx::query("UPDATE media SET last_used = ?2 WHERE song_id = ?1")
        .bind(song_id)
        .bind(crate::consts::now_secs().to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// One auto-download container rule (CACHE-5).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, serde::Serialize)]
pub struct MediaRule {
    pub id: String,
    pub kind: String,
    pub ref_id: Option<String>,
    pub created_at: i64,
}

/// The rule selecting a container, if any.
pub async fn rule_for(pool: &SqlitePool, kind: &str, ref_id: &str) -> Result<Option<MediaRule>> {
    let row = sqlx::query_as::<_, MediaRule>(
        "SELECT id, kind, ref_id, created_at FROM media_rule
         WHERE kind = ?1 AND ref_id = ?2",
    )
    .bind(kind)
    .bind(ref_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Idempotently selects a container for download; returns the rule id
/// (deterministic, so re-selecting never duplicates).
pub async fn set_rule(pool: &SqlitePool, kind: &str, ref_id: &str) -> Result<String> {
    if let Some(rule) = rule_for(pool, kind, ref_id).await? {
        return Ok(rule.id);
    }
    let id = format!("{kind}-{}", crate::digest::md5_hex(ref_id.as_bytes())?);
    sqlx::query(
        "INSERT INTO media_rule (id, kind, ref_id, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(&id)
    .bind(kind)
    .bind(ref_id)
    .bind(crate::consts::now_secs().to_string())
    .execute(pool)
    .await?;
    Ok(id)
}

/// Drops one rule (its blobs/marks are the caller's next step).
pub async fn delete_rule(pool: &SqlitePool, rule_id: &str) -> Result<()> {
    crate::cache::delete_by(pool, "DELETE FROM media_rule WHERE id = ?1", rule_id).await
}

/// Removes every blob owned by a rule (rule removal, CACHE-5). Explicit
/// downloads are `permanent` with no rule id, so they are never touched.
///
/// shortcut: a song covered by two rules loses its blob with the first
/// removed rule; re-selecting the other container refetches it.
pub async fn remove_rule_media(pool: &SqlitePool, rule_id: &str) -> Result<usize> {
    let rows: Vec<Stored> =
        sqlx::query_as("SELECT song_id, kind, file FROM media WHERE rule_id = ?1")
            .bind(rule_id)
            .fetch_all(pool)
            .await?;
    for row in &rows {
        remove_blob(row);
        crate::cache::delete_by(pool, "DELETE FROM media WHERE song_id = ?1", &row.song_id).await?;
    }
    Ok(rows.len())
}

/// Removes the config-tier blobs of the given songs, whatever rule id the
/// row carries — the rule-removal net for blobs stored before the fetch
/// recorded rule ownership (the ledger still names the rule then). The
/// cache tiers (playback/rolling) are not the rule's; they keep their own
/// purge.
pub async fn remove_songs_media(pool: &SqlitePool, song_ids: &[String]) -> Result<usize> {
    let mut removed = 0;
    for song_id in song_ids {
        let Some(row) = get(pool, song_id).await? else {
            continue;
        };
        if tier_for(&row.kind) != Tier::Config {
            continue;
        }
        remove_blob(&row);
        crate::cache::delete_by(pool, "DELETE FROM media WHERE song_id = ?1", song_id).await?;
        removed += 1;
    }
    Ok(removed)
}

/// Reconciles rule-owned blobs against the selected containers: a downloaded
/// song must still fall inside a live artist/album rule — anything else (the
/// scope changed, the rule ref is stale) is removed. Explicit downloads
/// (`rule_id NULL`, permanent) and the playback tiers are never touched; rule
/// kinds without a scope check yet (playlist/genre/favourites) stay covered
/// until their kinds land (CACHE-5 continues).
///
/// Runs in the sync's purge phase, after [`purge`].
pub async fn prune_uncovered(dbs: &crate::db::Dbs) -> Result<usize> {
    let rules: Vec<MediaRule> =
        sqlx::query_as("SELECT id, kind, ref_id, created_at FROM media_rule")
            .fetch_all(&dbs.media)
            .await?;
    let artist_refs: HashSet<&str> = rules
        .iter()
        .filter(|rule| rule.kind == "artist")
        .filter_map(|rule| rule.ref_id.as_deref())
        .collect();
    let album_refs: HashSet<&str> = rules
        .iter()
        .filter(|rule| rule.kind == "album")
        .filter_map(|rule| rule.ref_id.as_deref())
        .collect();

    // Rule-owned rows and the kind of the rule that selected them.
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT m.song_id, m.kind, m.file, r.kind
         FROM media m JOIN media_rule r ON r.id = m.rule_id",
    )
    .fetch_all(&dbs.media)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let scopes = crate::library::song_scopes(
        &dbs.library,
        &rows
            .iter()
            .map(|(song, ..)| song.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    let mut removed = 0;
    for (song_id, kind, file, rule_kind) in rows {
        let scope = scopes.get(&song_id);
        let artist_covered = scope
            .and_then(|(artist, _)| artist.as_deref())
            .is_some_and(|artist| artist_refs.contains(artist));
        let album_covered = scope
            .and_then(|(_, album)| album.as_deref())
            .is_some_and(|album| album_refs.contains(album));
        let covered =
            artist_covered || album_covered || !matches!(rule_kind.as_str(), "artist" | "album");
        if covered {
            continue;
        }

        remove_blob(&Stored {
            song_id: song_id.clone(),
            kind,
            file,
        });
        crate::cache::delete_by(&dbs.media, "DELETE FROM media WHERE song_id = ?1", &song_id)
            .await?;
        removed += 1;
    }
    Ok(removed)
}

/// Used bytes per tier: `(config, cache)` (CACHE-7's accounting).
pub async fn used_bytes(pool: &SqlitePool) -> Result<(i64, i64)> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT kind, COALESCE(SUM(size), 0) FROM media GROUP BY kind")
            .fetch_all(pool)
            .await?;
    let mut config = 0;
    let mut cache = 0;
    for (kind, size) in rows {
        match tier_for(&kind) {
            Tier::Config => config += size,
            Tier::Cache => cache += size,
        }
    }
    Ok((config, cache))
}

fn remove_blob(row: &Stored) {
    if let Ok(dir) = tier_dir(&row.kind) {
        let _ = std::fs::remove_file(dir.join(&row.file));
    }
}

/// Drops blobs for songs that left the library and rows whose rule vanished.
///
/// Only called after a fully successful sync (the covers/lyrics/colors rule);
/// tier caps are CACHE-3. Returns the number of removed rows.
pub async fn purge(dbs: &crate::db::Dbs) -> Result<usize> {
    #[derive(Debug, sqlx::FromRow)]
    struct PurgeRow {
        song_id: String,
        kind: String,
        file: String,
        rule_id: Option<String>,
    }

    // MEMORY: one query carries the rule id too (the old code loaded the
    // whole media table a second time into a `song_id -> rule_id` map), and
    // the library is asked only about the ids this cache actually holds.
    let rows: Vec<PurgeRow> = sqlx::query_as("SELECT song_id, kind, file, rule_id FROM media")
        .fetch_all(&dbs.media)
        .await?;
    if rows.is_empty() {
        remove_temp_files_all();
        return Ok(0);
    }
    let media_ids: Vec<&str> = rows.iter().map(|row| row.song_id.as_str()).collect();
    let known = crate::library::existing_song_ids(&dbs.library, &media_ids).await?;

    // Rule-orphans: the row's rule no longer exists.
    let live_rules: HashSet<String> = sqlx::query_scalar("SELECT id FROM media_rule")
        .fetch_all(&dbs.media)
        .await?
        .into_iter()
        .collect();

    let mut removed = 0;
    for row in rows {
        let vanished = !known.contains(&row.song_id);
        let orphaned = row
            .rule_id
            .as_ref()
            .is_some_and(|rule| !live_rules.contains(rule));
        if !vanished && !orphaned {
            continue;
        }
        remove_blob(&Stored {
            song_id: row.song_id.clone(),
            kind: row.kind,
            file: row.file,
        });
        crate::cache::delete_by(
            &dbs.media,
            "DELETE FROM media WHERE song_id = ?1",
            &row.song_id,
        )
        .await?;
        removed += 1;
    }

    remove_temp_files_all();
    Ok(removed)
}

fn remove_temp_files_all() {
    for kind in [KIND_PERMANENT, KIND_PLAYBACK] {
        if let Ok(dir) = tier_dir(kind) {
            remove_temp_files(&dir);
        }
    }
}

fn remove_temp_files(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".tmp") || name.ends_with(".part") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The cached paths for only `ids`, in bounded batches: a play action never
/// walks the whole media cache.
pub async fn path_map_of(pool: &SqlitePool, ids: &[String]) -> Result<HashMap<String, PathBuf>> {
    let mut unique: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| !id.is_empty())
        .collect();
    unique.sort_unstable();
    unique.dedup();
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let mut map = HashMap::with_capacity(unique.len());
    for chunk in unique.chunks(crate::cache::IN_BATCH) {
        let mut builder =
            sqlx::QueryBuilder::new("SELECT song_id, kind, file FROM media WHERE song_id IN (");
        crate::cache::push_in_list(&mut builder, chunk);
        let rows: Vec<Stored> = builder.build_query_as().fetch_all(pool).await?;
        for row in rows {
            if let Ok(dir) = tier_dir(&row.kind) {
                map.insert(row.song_id, dir.join(&row.file));
            }
        }
    }
    Ok(map)
}
