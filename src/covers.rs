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

use crate::http_client::SubsonicClient;

/// Cover size requested from the server (large enough for the mainscreen).
const COVER_SIZE: u32 = 512;

/// ARC-39: remote SVG art is rasterized at ingest; the longest edge of the
/// stored PNG never exceeds this (the palette and every surface are raster).
const SVG_MAX_EDGE: u32 = 1024;
/// Reject SVG input larger than this before parsing.
const SVG_MAX_INPUT: usize = 4 * 1024 * 1024;
/// Reject rasterized output larger than this (placeholder path instead).
const SVG_MAX_OUTPUT: usize = 8 * 1024 * 1024;

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
    let known = Arc::new(
        existing
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect::<HashMap<String, CoverRow>>(),
    );

    let ids: Vec<String> = referenced.iter().cloned().collect();
    let mut report = CoverReport::default();

    let client = Arc::clone(client);
    let known_for_fetch = Arc::clone(&known);
    let results = crate::fanout::map_bounded(ids, move |id| {
        let client = Arc::clone(&client);
        let known = Arc::clone(&known_for_fetch);
        async move {
            let (etag, last_modified) = known
                .get(&id)
                .map(|row| (row.etag.clone(), row.last_modified.clone()))
                .unwrap_or((None, None));
            client
                .get_cover_art(
                    &id,
                    Some(COVER_SIZE),
                    etag.as_deref(),
                    last_modified.as_deref(),
                )
                .await
        }
    })
    .await;

    for (id, result) in results {
        match result {
            Ok(art) if art.not_modified => {
                touch(pool, &id).await?;
                report.unchanged += 1;
            }
            Ok(art) => {
                let Some((bytes, extension, content_type)) =
                    store_art(art.bytes, art.content_type.as_deref())
                else {
                    // Malformed/oversize SVG: never store an undecodable
                    // blob, the placeholder path owns it (ARC-39).
                    eprintln!("covers: rejected undecodable SVG cover {id}");
                    continue;
                };

                let hash = md5_hex(&bytes);
                if known.get(&id).and_then(|row| row.hash.as_deref()) == Some(hash.as_str()) {
                    touch(pool, &id).await?;
                    report.unchanged += 1;
                    continue;
                }

                let file = format!("{}.{}", md5_hex(id.as_bytes()), extension);
                let final_path = dir.join(&file);
                let tmp_path = dir.join(format!("{file}.tmp"));

                std::fs::write(&tmp_path, &bytes)
                    .with_context(|| format!("failed to write {}", tmp_path.display()))?;
                std::fs::rename(&tmp_path, &final_path)
                    .with_context(|| format!("failed to replace {}", final_path.display()))?;

                upsert(
                    pool,
                    &id,
                    &file,
                    &hash,
                    content_type.as_deref(),
                    bytes.len() as i64,
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
            Err(err) => eprintln!("covers: failed to fetch {id}: {err:#}"),
        }
    }

    // Purge entries the library no longer references.
    for (id, row) in known.iter() {
        if !referenced.contains(id) {
            let _ = std::fs::remove_file(dir.join(&row.file));
            crate::cache::delete_by(pool, "DELETE FROM covers WHERE id = ?1", id).await?;
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
        .bind(crate::consts::now_secs().to_string())
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
    .bind(crate::consts::now_secs().to_string())
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

/// ARC-39: true when fetched art is an SVG, by content type or by sniffing the
/// head (`<svg`, or an XML declaration followed by one). Raster-only consumers
/// (`image`, the palette sampler, `CoverImage`) must never see these bytes.
fn is_svg(bytes: &[u8], content_type: Option<&str>) -> bool {
    let ctype = content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if ctype.eq_ignore_ascii_case("image/svg+xml") {
        return true;
    }

    let head = &bytes[..bytes.len().min(512)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

/// Rasterizes SVG bytes to a PNG bounded by [`SVG_MAX_EDGE`] and
/// [`SVG_MAX_OUTPUT`]; `None` for malformed input or an output over the caps
/// (the caller falls back to the placeholder path).
fn rasterize_svg(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > SVG_MAX_INPUT {
        return None;
    }

    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let longest = size.width().max(size.height());
    if !longest.is_finite() || longest <= 0.0 {
        return None;
    }

    let scale = (SVG_MAX_EDGE as f32 / longest).min(1.0);
    let width = ((size.width() * scale).ceil().max(1.0)) as u32;
    let height = ((size.height() * scale).ceil().max(1.0)) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let png = pixmap.encode_png().ok()?;
    (png.len() <= SVG_MAX_OUTPUT).then_some(png)
}

/// Prepares fetched art for storage: SVG payloads become bounded PNGs here
/// (ARC-39); everything else passes through with its content type/extension.
/// `None` means the art is undecodable and must not reach the cache.
fn store_art(
    bytes: Vec<u8>,
    content_type: Option<&str>,
) -> Option<(Vec<u8>, &'static str, Option<String>)> {
    if !is_svg(&bytes, content_type) {
        return Some((
            bytes,
            extension(content_type),
            content_type.map(str::to_string),
        ));
    }

    let png = rasterize_svg(&bytes)?;
    Some((png, "png", Some("image/png".to_string())))
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

    #[test]
    fn svg_detection_by_content_type_and_sniff() {
        assert!(is_svg(b"whatever", Some("image/svg+xml; charset=utf-8")));
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", None));
        assert!(is_svg(b"\n\t<svg/>", None));
        assert!(is_svg(b"\xef\xbb\xbf<svg/>", None));
        assert!(is_svg(
            b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            None
        ));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n", Some("image/png")));
        assert!(!is_svg(b"<?xml version=\"1.0\"?><root/>", None));
    }

    #[test]
    fn svg_is_rasterized_to_a_bounded_png() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2048" height="2048"><rect width="2048" height="2048" fill="red"/></svg>"#;
        let png = rasterize_svg(svg).expect("a valid SVG rasterizes");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let img = image::load_from_memory(&png).expect("stored bytes decode as an image");
        assert_eq!((img.width(), img.height()), (SVG_MAX_EDGE, SVG_MAX_EDGE));
        assert_eq!(img.to_rgba8().get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn small_svg_keeps_its_size() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="lime"/></svg>"#;
        let png = rasterize_svg(svg).expect("a valid SVG rasterizes");
        let img = image::load_from_memory(&png).expect("stored bytes decode as an image");
        assert_eq!((img.width(), img.height()), (64, 32));
    }

    #[test]
    fn malformed_and_oversize_svg_are_rejected() {
        assert!(store_art(b"<svg".to_vec(), Some("image/svg+xml")).is_none());
        let oversize = vec![b' '; SVG_MAX_INPUT + 1];
        assert!(store_art(oversize, Some("image/svg+xml")).is_none());
    }

    #[test]
    fn raster_decoded_art_passes_through() {
        let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
        let (bytes, extension, content_type) =
            store_art(png.clone(), Some("image/png")).expect("raster art passes through");
        assert_eq!(bytes, png);
        assert_eq!(extension, "png");
        assert_eq!(content_type.as_deref(), Some("image/png"));
    }
}
