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
#![expect(
    dead_code,
    reason = "media cache consumers land with CACHE-2/3/7; tests exercise the surface first"
)]

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
///
/// The upsert refuses — atomically, in the same statement — to downgrade a
/// config-tier download (permanent/rule) to a purgeable playback-cache copy:
/// a stream-cache fetch racing a download must never delete the user's blob
/// (N-05). `last_used`/`created_at` are bound as INTEGERs so the LRU ordering
/// of [`enforce_playback_cap`] compares like values (N-09).
#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the media row; a wrapper struct would restate it"
)]
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
    let now = crate::consts::now_secs();
    let result = sqlx::query(
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
             last_used   = excluded.last_used
         WHERE NOT (excluded.kind = ?9 AND media.kind IN (?10, ?11))",
    )
    .bind(song_id)
    .bind(kind)
    .bind(file)
    .bind(size)
    .bind(quality)
    .bind(fingerprint)
    .bind(rule_id)
    .bind(now)
    .bind(KIND_PLAYBACK)
    .bind(KIND_PERMANENT)
    .bind(KIND_RULE)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        anyhow::bail!("refusing to replace {song_id}'s downloaded blob with a playback-cache copy");
    }
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

/// Adopts an already-streamed temporary blob: renames it to the canonical
/// name and records the row. The source must live in the same tier directory
/// (see [`temp_path`]).
#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the media row; a wrapper struct would restate it"
)]
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

/// Sanitizes a blob file extension so it can never introduce path traversal or
/// separator characters: strips leading `.`, keeps only ASCII alphanumeric
/// characters (up to 12), lowercases, and falls back to `"bin"` if empty.
pub(crate) fn sanitize_extension(ext: &str) -> String {
    let clean: String = ext
        .trim_start_matches('.')
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if clean.is_empty() {
        "bin".to_string()
    } else {
        clean
    }
}

fn safe_blob_path(dir: &Path, file: &str) -> Option<PathBuf> {
    if file.is_empty() || file.contains('\0') {
        return None;
    }
    let mut components = Path::new(file).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(part)), None) if part == std::ffi::OsStr::new(file) => {
            let path = dir.join(file);
            (path.parent() == Some(dir)).then_some(path)
        }
        _ => None,
    }
}

fn remove_blob_in(dir: &Path, file: &str) {
    if let Some(path) = safe_blob_path(dir, file) {
        _ = std::fs::remove_file(path);
    }
}

/// The directory-taking half of [`store_streamed`] (also the test seam).
#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the media row; a wrapper struct would restate it"
)]
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
    let ext = sanitize_extension(extension);
    let file = format!("{}.{}", crate::digest::md5_hex(song_id.as_bytes())?, ext);
    let Some(final_path) = safe_blob_path(dir, &file) else {
        anyhow::bail!("refusing unsafe media blob filename: {file}");
    };
    anyhow::ensure!(
        final_path.parent() == Some(dir),
        "refusing media blob outside target directory: {}",
        final_path.display()
    );
    let previous = get(pool, song_id).await?;

    if let Err(err) = std::fs::rename(source, &final_path) {
        _ = std::fs::remove_file(source);
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
            && final_path.parent() == Some(dir)
        {
            _ = std::fs::remove_file(&final_path);
        }
        return Err(err);
    }

    if let Some(old) = previous
        && (old.file != file || old.kind != kind)
        && let Ok(old_dir) = tier_dir(&old.kind)
    {
        remove_blob_in(&old_dir, &old.file);
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
    let dir = tier_dir(&row.kind)?;
    let Some(path) = safe_blob_path(&dir, &row.file) else {
        return Ok(None);
    };
    Ok(path.is_file().then_some(path))
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
    let mut removed_songs: Vec<String> = Vec::new();
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
        removed_songs.push(song_id);
        removed += 1;
    }

    // N-05: the ledger and the offline catalog must follow the blobs, or a
    // removed download stays visible/downloaded after its scope is gone.
    for song_id in &removed_songs {
        crate::cache::delete_by(
            &dbs.download,
            "DELETE FROM download WHERE song_id = ?1",
            song_id,
        )
        .await?;
    }
    if !removed_songs.is_empty() {
        crate::offline::clear_cache();
    }
    Ok(removed)
}

fn remove_blob(row: &Stored) {
    if let Ok(dir) = tier_dir(&row.kind) {
        remove_blob_in(&dir, &row.file);
    }
}

/// Clears `active` download-ledger rows left behind by a crashed process.
///
/// At most one fetch per song runs in-process (the RAII claims in
/// `media_actions`), so an `active` row seen outside a fetch is stale and would
/// otherwise block that song's retries forever (N-05). The daemon calls this
/// at startup; [`purge`] repeats it after a successful sync as a fallback.
pub async fn reset_stale_downloads(dbs: &crate::db::Dbs) -> Result<usize> {
    let result = sqlx::query(
        "UPDATE download
            SET state = ?2, error = COALESCE(error, 'interrupted by a restart'), updated_at = ?3
          WHERE state = ?1",
    )
    .bind(crate::download::ACTIVE)
    .bind(crate::download::ERROR)
    .bind(crate::consts::now_secs())
    .execute(&dbs.download)
    .await?;
    Ok(result.rows_affected() as usize)
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
    // N-05: a crash mid-download leaves `active` ledger rows; purge runs on
    // every successful sync, so recover them here as well as at startup.
    if let Err(err) = reset_stale_downloads(dbs).await {
        eprintln!("media: could not reset stale download rows: {err:#}");
    }

    let rows: Vec<PurgeRow> = sqlx::query_as("SELECT song_id, kind, file, rule_id FROM media")
        .fetch_all(&dbs.media)
        .await?;
    if rows.is_empty() {
        sweep_temp_files().await;
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

    sweep_temp_files().await;
    Ok(removed)
}

/// Temp files (`.part`/`.tmp`) younger than this are active transfers, never
/// garbage (N-04).
const TEMP_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Sweeps abandoned temp files on the blocking pool; a just-written `.part`
/// belongs to a running download and is left alone (N-04, N-18).
async fn sweep_temp_files() {
    if let Err(err) = tokio::task::spawn_blocking(remove_temp_files_all).await {
        eprintln!("media: temp sweep task failed: {err}");
    }
}

fn remove_temp_files_all() {
    for kind in [KIND_PERMANENT, KIND_RULE, KIND_PLAYBACK, KIND_ROLLING] {
        if let Ok(dir) = tier_dir(kind) {
            remove_temp_files(&dir);
        }
    }
}

/// Test seam: the per-directory stale-temp sweep (also runs on the blocking
/// pool from [`purge`]).
pub(crate) fn remove_temp_files(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.ends_with(".tmp") || name.ends_with(".part")) {
            continue;
        }
        let path = entry.path();
        // An active download/stream-cache writes into its `.part`; only sweep
        // files old enough to be abandoned (N-04).
        let stale = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= TEMP_MIN_AGE);
        if stale {
            _ = std::fs::remove_file(path);
        }
    }
}

/// The cached paths for only `ids`, in bounded batches: a play action never
/// walks the whole media cache.
pub async fn path_map_of(
    pool: &SqlitePool,
    ids: &[String],
    playback_quality: &str,
) -> Result<HashMap<String, PathBuf>> {
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
    let now = crate::consts::now_secs();
    for chunk in unique.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT song_id, kind, file, quality FROM media WHERE song_id IN (",
        );
        crate::cache::push_in_list(&mut builder, chunk);
        let rows: Vec<(String, String, String, String)> =
            builder.build_query_as().fetch_all(pool).await?;
        for (song_id, kind, file, quality) in rows {
            // Playback-cache rows only match the quality that produced them;
            // a later quality change must not replay stale bytes. Download
            // rows (permanent/rule/rolling) are always the user's copy.
            if kind == KIND_PLAYBACK && quality != playback_quality {
                continue;
            }
            if let Ok(dir) = tier_dir(&kind)
                && let Some(path) = safe_blob_path(&dir, &file)
                && path.is_file()
            {
                if kind == KIND_PLAYBACK {
                    _ = sqlx::query("UPDATE media SET last_used = ?2 WHERE song_id = ?1")
                        .bind(&song_id)
                        .bind(now)
                        .execute(pool)
                        .await;
                }
                map.insert(song_id, path);
            }
        }
    }
    Ok(map)
}

/// A playback-cache blob for `song_id` when its quality matches.
pub async fn playback_path(
    pool: &SqlitePool,
    song_id: &str,
    quality: &str,
) -> Result<Option<PathBuf>> {
    let Some(row) = get(pool, song_id).await? else {
        return Ok(None);
    };
    let quality_matches: Option<String> =
        sqlx::query_scalar("SELECT quality FROM media WHERE song_id = ?1 AND kind = 'playback'")
            .bind(song_id)
            .fetch_optional(pool)
            .await?;
    if row.kind != KIND_PLAYBACK || quality_matches.as_deref() != Some(quality) {
        return Ok(None);
    }
    let dir = tier_dir(&row.kind)?;
    let Some(path) = safe_blob_path(&dir, &row.file) else {
        return Ok(None);
    };
    Ok(path.is_file().then_some(path))
}

/// Marks a playback-cache row as recently used (the LRU clock).
pub async fn touch_playback(pool: &SqlitePool, song_id: &str) -> Result<()> {
    sqlx::query("UPDATE media SET last_used = ?2 WHERE song_id = ?1 AND kind = 'playback'")
        .bind(song_id)
        .bind(crate::consts::now_secs())
        .execute(pool)
        .await?;
    Ok(())
}

/// Evicts the least-recently-used playback blobs until their total size fits
/// `max_bytes`. Download tiers are never touched.
///
/// True LRU: the oldest entries go first, whatever their size, until the
/// remaining set fits. The previous newest-first greedy pass could delete a
/// newer large blob and keep older small ones, so recency — not size — must
/// decide eviction (N-09). `last_used` is an INTEGER column and every writer
/// binds integers, so the ordering compares like values.
pub async fn enforce_playback_cap(pool: &SqlitePool, max_bytes: u64) -> Result<usize> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT song_id, file, size FROM media WHERE kind = 'playback'
         ORDER BY last_used ASC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    let mut total = 0u64;
    for (_, _, size) in &rows {
        total = total.saturating_add((*size).max(0) as u64);
    }
    if total <= max_bytes {
        return Ok(0);
    }
    let dir = tier_dir(KIND_PLAYBACK)?;
    let mut removed = 0usize;
    for (song_id, file, size) in rows {
        if total <= max_bytes {
            break;
        }
        remove_blob_in(&dir, &file);
        crate::cache::delete_by(pool, "DELETE FROM media WHERE song_id = ?1", &song_id).await?;
        total = total.saturating_sub(size.max(0) as u64);
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn media_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations_media")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[test]
    fn sanitize_extension_strips_traversal_and_limits_length() {
        assert_eq!(sanitize_extension(".flac"), "flac");
        assert_eq!(sanitize_extension("...MP3"), "mp3");
        assert_eq!(sanitize_extension("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_extension("../"), "bin");
        assert_eq!(sanitize_extension(""), "bin");
        assert_eq!(sanitize_extension("..."), "bin");
        assert_eq!(sanitize_extension("A123456789012345"), "a12345678901");
    }

    #[tokio::test]
    async fn store_in_dir_sanitizes_traversal_suffix() {
        if let Some(dir_os) = std::env::var_os("MUSIKA_TEST_MEDIA_SUBPROC_DIR") {
            let blob_dir = PathBuf::from(dir_os);
            let outside_canary = blob_dir.parent().unwrap().join("outside.txt");
            let sub_pool = media_pool().await;
            let row = Stored {
                song_id: "s-evil".to_string(),
                kind: KIND_PERMANENT.to_string(),
                file: "../outside.txt".to_string(),
            };
            remove_blob(&row);
            assert!(
                outside_canary.is_file(),
                "remove_blob must reject traversal"
            );

            record(
                &sub_pool,
                "s1",
                KIND_PERMANENT,
                "../outside.txt",
                11,
                "original",
                "fp",
                None,
            )
            .await
            .unwrap();
            let part2 = blob_dir.join("download2.part");
            std::fs::write(&part2, b"new-bytes").unwrap();
            _ = store_in_dir(
                &sub_pool,
                &blob_dir,
                "s1",
                &part2,
                9,
                KIND_PERMANENT,
                "original",
                "fp2",
                "flac",
                None,
            )
            .await
            .unwrap();
            assert!(
                outside_canary.is_file(),
                "store_in_dir old.file cleanup must reject traversal"
            );
            return;
        }

        let base = std::env::temp_dir().join(format!("musika-media-s05-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&base);
        let blob_dir = base.join("blobs");
        std::fs::create_dir_all(&blob_dir).unwrap();

        let outside_canary = base.join("outside.txt");
        std::fs::write(&outside_canary, b"keep-outside").unwrap();

        let pool = media_pool().await;
        let part = blob_dir.join("download.part");
        std::fs::write(&part, b"audio-bytes").unwrap();

        let stored = store_in_dir(
            &pool,
            &blob_dir,
            "s1",
            &part,
            11,
            KIND_PERMANENT,
            "original",
            "fp",
            "../../outside.txt",
            None,
        )
        .await
        .unwrap();

        assert_eq!(stored.parent(), Some(blob_dir.as_path()));
        let expected_name = format!("{}.outsidetxt", crate::digest::md5_hex(b"s1").unwrap());
        assert_eq!(stored, blob_dir.join(&expected_name));
        assert!(stored.is_file());
        assert_eq!(
            std::fs::read(&outside_canary).unwrap(),
            b"keep-outside",
            "outside canary must never be overwritten or removed"
        );

        // `safe_blob_path` and `remove_blob_in` must reject traversal/non-normal filenames.
        for bad in [
            "../outside.txt",
            "sub/file.flac",
            "/etc/passwd",
            ".",
            "..",
            "",
            "./local.flac",
        ] {
            assert!(
                safe_blob_path(&blob_dir, bad).is_none(),
                "should reject {bad}"
            );
            remove_blob_in(&blob_dir, bad);
        }
        assert!(outside_canary.is_file());

        // Also test real `remove_blob` and `store_in_dir` old.file cleanup in an
        // isolated subprocess with `MUSIKA_MEDIA_DIR` pointing at `blob_dir`.
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .env("MUSIKA_MEDIA_DIR", &blob_dir)
            .env("MUSIKA_TEST_MEDIA_SUBPROC_DIR", &blob_dir)
            .args([
                "--exact",
                "media::tests::store_in_dir_sanitizes_traversal_suffix",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "media child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(outside_canary.is_file());

        _ = std::fs::remove_dir_all(&base);
    }
}
