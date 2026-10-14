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
//! with [`cached_path`]/[`path_map`] and never waits on the network for them.
//! Pruning happens only after a fully successful sync (the covers.rs policy):
//! vanished songs first, then rule-orphans; tier caps are CACHE-3.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result};
use md5::{Digest, Md5};
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
enum Tier {
    /// `~/.config/musika/media/` — permanent and rule blobs.
    Config,
    /// `~/.cache/musika/media/` — playback and rolling blobs.
    Cache,
}

fn tier_for(kind: &str) -> Tier {
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

/// Writes a fetched blob (atomic `.part` → rename) and records it. Returns
/// the final path. A previous blob for the same song is replaced, never
/// orphaned.
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
    let dir = tier_dir(kind)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let file = format!("{}.{}", md5_hex(song_id.as_bytes()), extension);
    let final_path = dir.join(&file);
    let tmp_path = dir.join(format!("{file}.part"));

    std::fs::write(&tmp_path, bytes)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("failed to replace {}", final_path.display()))?;

    let previous = get(pool, song_id).await?;
    record(
        pool,
        song_id,
        kind,
        &file,
        bytes.len() as i64,
        quality,
        fingerprint,
        rule_id,
    )
    .await?;
    if let Some(old) = previous {
        if old.file != file || old.kind != kind {
            if let Ok(old_dir) = tier_dir(&old.kind) {
                let _ = std::fs::remove_file(old_dir.join(&old.file));
            }
        }
    }
    Ok(final_path)
}

/// Absolute path to the cached copy, when the row and the file both exist.
pub async fn cached_path(pool: &SqlitePool, song_id: &str) -> Result<Option<PathBuf>> {
    let Some(row) = get(pool, song_id).await? else {
        return Ok(None);
    };
    let path = tier_dir(&row.kind)?.join(&row.file);
    Ok(path.is_file().then_some(path))
}

/// Batch resolver for queues and lists (the `covers::url_map` pattern).
pub async fn path_map(pool: &SqlitePool) -> Result<HashMap<String, PathBuf>> {
    let rows: Vec<Stored> = sqlx::query_as("SELECT song_id, kind, file FROM media")
        .fetch_all(pool)
        .await?;
    let mut map = HashMap::new();
    for row in rows {
        if let Ok(dir) = tier_dir(&row.kind) {
            let path = dir.join(&row.file);
            if path.is_file() {
                map.insert(row.song_id, path);
            }
        }
    }
    Ok(map)
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
    let known: HashSet<String> = crate::library::song_ids(&dbs.library)
        .await?
        .into_iter()
        .collect();

    let rows: Vec<Stored> = sqlx::query_as("SELECT song_id, kind, file FROM media")
        .fetch_all(&dbs.media)
        .await?;

    // Rule-orphans: the row's rule no longer exists.
    let live_rules: HashSet<String> = sqlx::query_scalar("SELECT id FROM media_rule")
        .fetch_all(&dbs.media)
        .await?
        .into_iter()
        .collect();
    let rule_of: HashMap<String, Option<String>> =
        sqlx::query_as::<_, (String, Option<String>)>("SELECT song_id, rule_id FROM media")
            .fetch_all(&dbs.media)
            .await?
            .into_iter()
            .collect();

    let mut removed = 0;
    for row in rows {
        let vanished = !known.contains(&row.song_id);
        let orphaned = rule_of
            .get(&row.song_id)
            .and_then(|rule| rule.as_ref())
            .is_some_and(|rule| !live_rules.contains(rule));
        if !vanished && !orphaned {
            continue;
        }
        remove_blob(&row);
        sqlx::query("DELETE FROM media WHERE song_id = ?1")
            .bind(&row.song_id)
            .execute(&dbs.media)
            .await?;
        removed += 1;
    }
    Ok(removed)
}

fn md5_hex(bytes: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_covers_every_input() {
        let base = fingerprint(Some(10), Some("mp3"), Some(320), "srv", "original");
        assert_eq!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(320), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(11), Some("mp3"), Some(320), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("ogg"), Some(320), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(192), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(320), "other", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(320), "srv", "mp3@192")
        );
        assert_eq!(
            fingerprint(None, None, None, "srv", "original"),
            "0||0|srv|original"
        );
    }

    #[test]
    fn tiers_split_config_from_cache() {
        assert_eq!(tier_for(KIND_PERMANENT), Tier::Config);
        assert_eq!(tier_for(KIND_RULE), Tier::Config);
        assert_eq!(tier_for(KIND_PLAYBACK), Tier::Cache);
        assert_eq!(tier_for(KIND_ROLLING), Tier::Cache);
    }

    #[tokio::test]
    async fn purge_drops_vanished_songs_and_rule_orphans() {
        use std::str::FromStr;

        let media = async {
            let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();
            sqlx::migrate!("./migrations_media")
                .run(&pool)
                .await
                .unwrap();
            pool
        }
        .await;
        let library = async {
            let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();
            sqlx::migrate!("./migrations_library")
                .run(&pool)
                .await
                .unwrap();
            pool
        }
        .await;

        sqlx::query("INSERT INTO songs (id, title) VALUES ('kept', 'A')")
            .execute(&library)
            .await
            .unwrap();
        sqlx::query("INSERT INTO media_rule (id, kind, ref_id, created_at) VALUES ('r1', 'artist', 'a1', 0)")
            .execute(&media)
            .await
            .unwrap();

        // A live rule-owned blob, a rule-orphan, and a vanished song's blob.
        for (song, rule) in [
            ("kept", Some("r1")),
            ("kept2", Some("dead")),
            ("gone", None),
        ] {
            record(
                &media,
                song,
                KIND_RULE,
                &format!("{song}.mp3"),
                1,
                "original",
                "fp",
                rule,
            )
            .await
            .unwrap();
        }

        let dbs = crate::db::Dbs {
            credentials: library.clone(),
            library: library.clone(),
            search: library.clone(),
            covers: library.clone(),
            lyrics: library.clone(),
            colors: library.clone(),
            status: library.clone(),
            artist_user: library.clone(),
            media: media.clone(),
            download: library.clone(),
        };
        assert_eq!(purge(&dbs).await.unwrap(), 2);

        let left: Vec<String> = sqlx::query_scalar("SELECT song_id FROM media ORDER BY song_id")
            .fetch_all(&media)
            .await
            .unwrap();
        assert_eq!(left, vec!["kept"]);
    }
}
