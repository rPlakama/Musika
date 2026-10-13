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

//! Cover-art cache.
//!
//! Image blobs are written next to the databases (see [`crate::db::covers_dir`])
//! and indexed in `covers.db`. Cache entries are only ever removed after a
//! fully successful sync pass:
//!
//! - a cached cover is revalidated with a conditional request; a `304` keeps it,
//! - a changed cover is downloaded to a temporary file and atomically renamed
//!   over the old one (readers never see a partial image),
//! - entries no longer referenced by the library are purged last.
//!
//! If the server provides no validators, the downloaded bytes are compared by
//! hash so an unchanged image is not rewritten.

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use md5::{Digest, Md5};
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;
use tokio::task::JoinSet;

use crate::http_client::SubsonicClient;

/// Cover size requested from the server (large enough for the mainscreen).
const COVER_SIZE: u32 = 512;
const FETCH_CONCURRENCY: usize = 8;

#[derive(Debug, Default, Clone, Copy)]
pub struct CoverReport {
    pub downloaded: usize,
    pub unchanged: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, FromRow)]
struct CoverRow {
    id: String,
    file: String,
    hash: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// Revalidates/downloads every referenced cover and purges the rest.
///
/// Must only be called at the end of a *successful* library sync.
pub async fn refresh(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    referenced: &BTreeSet<String>,
) -> Result<CoverReport> {
    let dir = crate::db::covers_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    let existing: Vec<CoverRow> =
        sqlx::query_as("SELECT id, file, hash, etag, last_modified FROM covers")
            .fetch_all(pool)
            .await?;
    let known: HashMap<String, CoverRow> = existing
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();

    let ids: Vec<String> = referenced.iter().cloned().collect();
    let mut report = CoverReport::default();

    for chunk in ids.chunks(FETCH_CONCURRENCY) {
        let mut set = JoinSet::new();
        for id in chunk {
            let client = Arc::clone(client);
            let id = id.clone();
            let (etag, last_modified) = known
                .get(&id)
                .map(|row| (row.etag.clone(), row.last_modified.clone()))
                .unwrap_or((None, None));
            set.spawn(async move {
                let result = client
                    .get_cover_art(
                        &id,
                        Some(COVER_SIZE),
                        etag.as_deref(),
                        last_modified.as_deref(),
                    )
                    .await;
                (id, result)
            });
        }

        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((id, Ok(art))) if art.not_modified => {
                    touch(pool, &id).await?;
                    report.unchanged += 1;
                }
                Ok((id, Ok(art))) => {
                    let hash = md5_hex(&art.bytes);
                    if known.get(&id).and_then(|row| row.hash.as_deref()) == Some(hash.as_str()) {
                        touch(pool, &id).await?;
                        report.unchanged += 1;
                        continue;
                    }

                    let extension = extension(art.content_type.as_deref());
                    let file = format!("{}.{}", md5_hex(id.as_bytes()), extension);
                    let final_path = dir.join(&file);
                    let tmp_path = dir.join(format!("{file}.tmp"));

                    std::fs::write(&tmp_path, &art.bytes)
                        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
                    std::fs::rename(&tmp_path, &final_path)
                        .with_context(|| format!("failed to replace {}", final_path.display()))?;

                    upsert(
                        pool,
                        &id,
                        &file,
                        &hash,
                        art.content_type.as_deref(),
                        art.bytes.len() as i64,
                        art.etag.as_deref(),
                        art.last_modified.as_deref(),
                    )
                    .await?;

                    if let Some(old) = known.get(&id) {
                        if old.file != file {
                            let _ = std::fs::remove_file(dir.join(&old.file));
                        }
                    }
                    report.downloaded += 1;
                }
                Ok((id, Err(err))) => eprintln!("covers: failed to fetch {id}: {err:#}"),
                Err(err) => eprintln!("covers: a fetch task failed: {err}"),
            }
        }
    }

    // Purge entries the library no longer references.
    for (id, row) in &known {
        if !referenced.contains(id) {
            let _ = std::fs::remove_file(dir.join(&row.file));
            sqlx::query("DELETE FROM covers WHERE id = ?1")
                .bind(id)
                .execute(pool)
                .await?;
            report.removed += 1;
        }
    }

    remove_orphans(pool, &dir).await?;

    Ok(report)
}

/// Absolute path to a cached cover, if present.
pub async fn cached_path(pool: &SqlitePool, id: &str) -> Result<Option<std::path::PathBuf>> {
    let file: Option<String> = sqlx::query_scalar("SELECT file FROM covers WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let Some(file) = file else {
        return Ok(None);
    };
    let path = crate::db::covers_dir()?.join(file);
    Ok(path.is_file().then_some(path))
}

/// The content hash stored for a cached cover (used to revalidate colors).
pub async fn hash(pool: &SqlitePool, id: &str) -> Result<Option<String>> {
    let hash: Option<Option<String>> = sqlx::query_scalar("SELECT hash FROM covers WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(hash.flatten())
}

/// Maps every cached cover id to its `file://` URL. Used to resolve art while
/// loading lists off the UI thread.
pub async fn url_map(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, file FROM covers")
        .fetch_all(pool)
        .await?;
    let dir = crate::db::covers_dir()?;
    let mut map = HashMap::new();
    for (id, file) in rows {
        if let Ok(url) = reqwest::Url::from_file_path(dir.join(&file)) {
            map.insert(id, url.to_string());
        }
    }
    Ok(map)
}

async fn touch(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("UPDATE covers SET fetched_at = ?2 WHERE id = ?1")
        .bind(id)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upsert(
    pool: &SqlitePool,
    id: &str,
    file: &str,
    hash: &str,
    content_type: Option<&str>,
    size: i64,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO covers (id, file, hash, content_type, size, etag, last_modified, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
             file = excluded.file,
             hash = excluded.hash,
             content_type = excluded.content_type,
             size = excluded.size,
             etag = excluded.etag,
             last_modified = excluded.last_modified,
             fetched_at = excluded.fetched_at",
    )
    .bind(id)
    .bind(file)
    .bind(hash)
    .bind(content_type)
    .bind(size)
    .bind(etag)
    .bind(last_modified)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes temp leftovers and any blob the index does not know about.
async fn remove_orphans(pool: &SqlitePool, dir: &std::path::Path) -> Result<()> {
    let kept: Vec<String> = sqlx::query_scalar("SELECT file FROM covers")
        .fetch_all(pool)
        .await?;
    let kept: HashSet<String> = kept.into_iter().collect();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".tmp") || !kept.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

fn extension(content_type: Option<&str>) -> &'static str {
    match content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
    {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "img",
    }
}

fn md5_hex(bytes: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_from_content_type() {
        assert_eq!(extension(Some("image/jpeg")), "jpg");
        assert_eq!(extension(Some("image/png; charset=binary")), "png");
        assert_eq!(extension(Some("image/webp")), "webp");
        assert_eq!(extension(None), "img");
        assert_eq!(extension(Some("application/octet-stream")), "img");
    }

    #[test]
    fn md5_is_stable_and_distinct() {
        assert_eq!(md5_hex(b"same"), md5_hex(b"same"));
        assert_ne!(md5_hex(b"a"), md5_hex(b"b"));
    }
}
