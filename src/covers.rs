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
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use anyhow::{Context, Result};
use openssl::hash::MessageDigest;
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::http_client::SubsonicClient;

/// File-name prefix for the provider's original art. Older cache entries used
/// the server-downscaled 512 px variant; their missing prefix forces one raw
/// refetch on the next pass.
const RAW_PREFIX: &str = "raw-";

/// FIX-2: in-flight registries — a repeated page reload/scroll while a cover
/// is still missing (or its thumbnail is not generated yet) must not start
/// the same background work again. Claimed ids are held only inside the
/// spawned task's life; the lock is taken briefly, never across an await.
static ONGOING_FETCHES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static ONGOING_THUMBS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn claim_fetch(id: &str) -> bool {
    claim(&ONGOING_FETCHES, id)
}

fn release_fetch(id: String) {
    release(&ONGOING_FETCHES, id);
}

fn claim_thumb(name: &str) -> bool {
    claim(&ONGOING_THUMBS, name)
}

fn release_thumb(name: String) {
    release(&ONGOING_THUMBS, name);
}

fn claim(set: &Mutex<HashSet<String>>, key: &str) -> bool {
    set.lock_unguarded().insert(key.to_string())
}

fn release(set: &Mutex<HashSet<String>>, key: String) {
    set.lock_unguarded().remove(&key);
}

/// Mutex helper that never panics on a poisoned lock (a claim registry
/// carries no invariant worth aborting for).
trait LockUnguarded {
    fn lock_unguarded(&self) -> MutexGuard<'_, HashSet<String>>;
}

impl LockUnguarded for Mutex<HashSet<String>> {
    fn lock_unguarded(&self) -> MutexGuard<'_, HashSet<String>> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Longest edge of the thumbnail kept beside each cover; small surfaces load
/// this instead of the full art (keep in sync with `CoverImage.thumbEdge`).
const THUMB_EDGE: u32 = 320;
/// JPEG quality for thumbnails; invisible at the painted row sizes.
const THUMB_QUALITY: u8 = 82;
/// Bounded worker count for lazy thumbnail generation.
const THUMB_CONCURRENCY: usize = 4;
/// Blobs above this are thumbnailed one at a time. A compressed cover can
/// decode to an order of magnitude more RGBA than its file size, so four
/// large originals in parallel (a 6000 px cover is ~144 MB each) would spike
/// the sync's cover pass; small art keeps the parallel path.
const THUMB_LARGE_BYTES: u64 = 2 * 1024 * 1024;

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
    /// Same bytes moved to the canonical raw file name (legacy cache entry).
    pub upgraded: usize,
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
/// `forced` cover ids skip the conditional request and are downloaded again:
/// the album update that produced them must replace the cached art, even if
/// the server would answer `304` to a revalidation.
///
/// Must only be called at the end of a *successful* library sync.
pub async fn refresh(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    referenced: &BTreeSet<String>,
    forced: &[String],
) -> Result<CoverReport> {
    refresh_inner(pool, client, referenced, forced, true).await
}

/// Fetches specific covers without purging the rest: the on-demand path for
/// a surface that needs art the sync has not cached yet.
pub(crate) async fn fetch_ids(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    ids: &[String],
) -> Result<CoverReport> {
    let ids: BTreeSet<String> = ids.iter().filter(|id| !id.is_empty()).cloned().collect();
    if ids.is_empty() {
        return Ok(CoverReport::default());
    }
    refresh_inner(pool, client, &ids, &[], false).await
}

async fn refresh_inner(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    referenced: &BTreeSet<String>,
    forced: &[String],
    purge_unreferenced: bool,
) -> Result<CoverReport> {
    let dir = crate::db::covers_dir()?;
    crate::db::ensure_dir(&dir)?;

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
    let forced: Arc<HashSet<String>> = Arc::new(forced.iter().cloned().collect());
    let mut report = CoverReport::default();
    let mut thumb_jobs: Vec<(String, PathBuf)> = Vec::new();

    // MEMORY: process each completed fetch before starting the next one. The
    // old `fanout::map_bounded` collected every `CoverArt` (body bytes
    // included) first and only then stored them, so a first sync held every
    // downloaded cover in RAM at once — hundreds of MB on a big library. A
    // chunked `JoinSet` keeps at most `FETCH_CONCURRENCY` bodies alive.
    let client = Arc::clone(client);
    for chunk in ids.chunks(crate::consts::FETCH_CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for id in chunk {
            let id = id.clone();
            let client = Arc::clone(&client);
            let known = Arc::clone(&known);
            let forced = Arc::clone(&forced);
            set.spawn(async move {
                let cached = known.get(&id);
                let needs_raw = cached.is_some_and(|row| !is_raw_file(&row.file));
                let (etag, last_modified) = if forced.contains(&id) || needs_raw {
                    (None, None)
                } else {
                    cached
                        .map(|row| (row.etag.clone(), row.last_modified.clone()))
                        .unwrap_or((None, None))
                };
                // No `size`: keep the provider's original art for the large
                // surfaces (fullscreen, album page); small ones use the
                // thumbnail.
                let result = client
                    .get_cover_art(&id, None, etag.as_deref(), last_modified.as_deref())
                    .await;
                (id, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((id, result)) => {
                    apply_fetched(
                        pool,
                        &dir,
                        known.as_ref(),
                        &id,
                        result,
                        &mut report,
                        &mut thumb_jobs,
                    )
                    .await?;
                }
                Err(err) => eprintln!("covers: fetch task failed: {err}"),
            }
        }
    }

    // Purge entries the library no longer references. The on-demand path
    // fetches a subset and must leave every other cached cover alone.
    if purge_unreferenced {
        for (id, row) in known.iter() {
            if !referenced.contains(id) {
                let _ = std::fs::remove_file(dir.join(&row.file));
                let _ = std::fs::remove_file(dir.join(thumb_name(&row.file)));
                crate::cache::delete_by(pool, "DELETE FROM covers WHERE id = ?1", id).await?;
                report.removed += 1;
            }
        }
    }

    let _ = ensure_thumbs(&dir, thumb_jobs).await;
    remove_orphans(pool, &dir).await?;

    Ok(report)
}

/// Stores one completed fetch: revalidation touch, content-addressed write,
/// URI-field upsert and thumbnail job. Split out of [`refresh_inner`] so the
/// network chunk and the disk/database step stay one short result apart.
#[allow(clippy::too_many_arguments)]
async fn apply_fetched(
    pool: &SqlitePool,
    dir: &Path,
    known: &HashMap<String, CoverRow>,
    id: &str,
    result: Result<crate::http_client::CoverArt>,
    report: &mut CoverReport,
    thumb_jobs: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    match result {
        Ok(art) if art.not_modified => {
            touch(pool, id).await?;
            report.unchanged += 1;
        }
        Ok(art) => {
            let Some((bytes, extension, content_type)) =
                store_art(art.bytes, art.content_type.as_deref())
            else {
                // Malformed/oversize SVG: never store an undecodable blob,
                // the placeholder path owns it (ARC-39).
                eprintln!("covers: rejected undecodable SVG cover {id}");
                return Ok(());
            };

            let hash = md5_hex(&bytes);
            // Content-suffixed: changed art gets a new file/URL, so the UI
            // can never keep painting the previous image at the old path
            // (the cover id is not guaranteed to change with it). The raw
            // prefix also marks the source resolution.
            let file = cover_file_name(id, &hash, extension);

            if let Some(old) = known.get(id)
                && old.hash.as_deref() == Some(hash.as_str())
            {
                if old.file == file {
                    // The provider revalidated the art and reported the same
                    // bytes: keep the file, but store the date it now carries
                    // so the next conditional request compares against it.
                    upsert(
                        pool,
                        id,
                        &old.file,
                        &hash,
                        content_type.as_deref(),
                        bytes.len() as i64,
                        art.etag.as_deref(),
                        art.last_modified.as_deref(),
                    )
                    .await?;
                    report.unchanged += 1;
                } else {
                    // Same bytes, legacy filename: move the entry to the
                    // canonical raw name now, or the raw upgrade would force a
                    // full download on every pass forever.
                    crate::cache::write_atomic(dir, &file, &bytes)?;
                    thumb_jobs.push((thumb_name(&file), dir.join(&file)));
                    upsert(
                        pool,
                        id,
                        &file,
                        &hash,
                        content_type.as_deref(),
                        bytes.len() as i64,
                        art.etag.as_deref(),
                        art.last_modified.as_deref(),
                    )
                    .await?;
                    let _ = std::fs::remove_file(dir.join(&old.file));
                    let _ = std::fs::remove_file(dir.join(thumb_name(&old.file)));
                    report.upgraded += 1;
                }
                return Ok(());
            }

            crate::cache::write_atomic(dir, &file, &bytes)?;

            upsert(
                pool,
                id,
                &file,
                &hash,
                content_type.as_deref(),
                bytes.len() as i64,
                art.etag.as_deref(),
                art.last_modified.as_deref(),
            )
            .await?;

            // Generate here, off the UI path, so pages find the thumbnail
            // ready when they first resolve the cover URL.
            thumb_jobs.push((thumb_name(&file), dir.join(&file)));

            if let Some(old) = known.get(id)
                && old.file != file
            {
                let _ = std::fs::remove_file(dir.join(&old.file));
                let _ = std::fs::remove_file(dir.join(thumb_name(&old.file)));
            }
            report.downloaded += 1;
        }
        Err(err) => eprintln!("covers: failed to fetch {id}: {err:#}"),
    }
    Ok(())
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

/// Path to the art the palette sampler should decode: the small thumbnail
/// when the cache has one, the original otherwise.
///
/// The sampler downscales to 112 px either way, so decoding the provider's
/// full-resolution art is pure peak cost: a 3000 px JPEG decodes to ~36 MB of
/// RGBA against ~0.4 MB for the 320 px thumbnail, on every track change. The
/// thumbnail is written by the sync and the on-demand path; a cover whose
/// thumbnail has not landed yet still gets a palette from the original.
pub async fn palette_path(pool: &SqlitePool, id: &str) -> Result<Option<std::path::PathBuf>> {
    let file: Option<String> = sqlx::query_scalar("SELECT file FROM covers WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let Some(file) = file else {
        return Ok(None);
    };
    Ok(preferred_art(&crate::db::covers_dir()?, &file))
}

/// Thumbnail-first resolution shared by [`palette_path`] (kept pure so the
/// preference is testable without a database or the config directory).
fn preferred_art(dir: &Path, file: &str) -> Option<PathBuf> {
    let thumb = dir.join(thumb_name(file));
    if thumb.is_file() {
        return Some(thumb);
    }
    let raw = dir.join(file);
    raw.is_file().then_some(raw)
}

/// The content hash stored for a cached cover (used to revalidate colors).
pub async fn hash(pool: &SqlitePool, id: &str) -> Result<Option<String>> {
    let hash: Option<Option<String>> = sqlx::query_scalar("SELECT hash FROM covers WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(hash.flatten())
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
    let kept_thumbs: HashSet<String> = kept.iter().map(|file| thumb_name(file)).collect();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".tmp") || (!kept.contains(&name) && !kept_thumbs.contains(&name)) {
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
    crate::hex::encode(openssl::hash::hash(MessageDigest::md5(), bytes).expect("openssl md5"))
}

/// Canonical content-suffixed cover filename. The raw prefix is what makes the
/// one-shot legacy upgrade terminate: once written, the next pass sees it.
fn cover_file_name(id: &str, hash: &str, extension: &str) -> String {
    format!(
        "{RAW_PREFIX}{}.{}",
        md5_hex(format!("{id}\0{hash}").as_bytes()),
        extension
    )
}

fn is_raw_file(name: &str) -> bool {
    name.starts_with(RAW_PREFIX)
}

/// Derives the thumbnail file name from a cover file name. `CoverImage.qml`
/// computes the same name from the resolved URL, so keep the two in sync.
fn thumb_name(file: &str) -> String {
    let stem = Path::new(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file);
    format!("{stem}.thumb.jpg")
}

fn build_thumb(source: &Path) -> Result<Vec<u8>> {
    let image = image::ImageReader::open(source)
        .with_context(|| format!("failed to open cover {}", source.display()))?
        .with_guessed_format()
        .with_context(|| format!("failed to sniff cover {}", source.display()))?
        .decode()
        .with_context(|| format!("failed to decode cover {}", source.display()))?;

    // Small art is already cheap to decode; copy it rather than re-encode.
    if image.width() <= THUMB_EDGE && image.height() <= THUMB_EDGE {
        return std::fs::read(source)
            .with_context(|| format!("failed to read cover {}", source.display()));
    }

    let thumb = image.resize(
        THUMB_EDGE,
        THUMB_EDGE,
        image::imageops::FilterType::Lanczos3,
    );
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, THUMB_QUALITY)
        .encode_image(&thumb.to_rgb8())
        .with_context(|| format!("failed to encode thumbnail for {}", source.display()))?;
    Ok(bytes)
}

fn write_thumb(source: &Path, dir: &Path, name: &str) -> Result<()> {
    let bytes = build_thumb(source).or_else(|err| {
        // A blob the decoder rejects still needs a file QML can request; the
        // copy surfaces the same failure as the full-size path.
        eprintln!(
            "covers: thumbnail fallback for {}: {err:#}",
            source.display()
        );
        std::fs::read(source).context("failed to read cover for thumbnail fallback")
    })?;
    crate::cache::write_atomic(dir, name, &bytes)?;
    Ok(())
}

/// Generates missing thumbnails on a bounded blocking pool: decoding is CPU
/// work, and the Qt image reader only decodes one file at a time. Returns
/// the thumb names it processed so the caller can release its claims.
///
/// Large blobs are decoded one at a time ([`THUMB_LARGE_BYTES`]); small blobs
/// keep [`THUMB_CONCURRENCY`] workers. The blob size is a cheap stand-in for
/// decoded dimensions and avoids probing every file twice.
async fn ensure_thumbs(dir: &Path, jobs: Vec<(String, PathBuf)>) -> Vec<String> {
    let processed: Vec<String> = jobs.iter().map(|(name, _)| name.clone()).collect();
    let mut large: Vec<(String, PathBuf)> = Vec::new();
    let mut small: Vec<(String, PathBuf)> = Vec::new();
    for job in jobs {
        let is_large = tokio::fs::metadata(&job.1)
            .await
            .map(|meta| meta.len() > THUMB_LARGE_BYTES)
            .unwrap_or(false);
        if is_large {
            large.push(job);
        } else {
            small.push(job);
        }
    }

    for job in large {
        run_thumb_batch(dir, vec![job]).await;
    }
    for chunk in small.chunks(THUMB_CONCURRENCY) {
        run_thumb_batch(dir, chunk.to_vec()).await;
    }
    processed
}

/// Decodes and writes one batch of thumbnails concurrently.
async fn run_thumb_batch(dir: &Path, jobs: Vec<(String, PathBuf)>) {
    let mut set = tokio::task::JoinSet::new();
    for (name, source) in jobs {
        let dir = dir.to_path_buf();
        set.spawn_blocking(move || {
            if let Err(err) = write_thumb(&source, &dir, &name) {
                eprintln!("covers: failed to thumbnail {}: {err:#}", source.display());
            }
        });
    }
    while set.join_next().await.is_some() {}
}

/// Maps only the requested cover ids to `file://` URLs, in bounded batches.
/// Payload loaders use this so a page's cost follows
/// the page, never the whole cover cache.
///
/// Cache first: a missing cover is resolved to the provider URL right away
/// and cached in the background, so library pages never wait on artwork.
/// Offline/no-credentials simply keeps the cache-only result.
pub async fn url_map_of(
    dbs: &crate::db::Dbs,
    ids: &[String],
) -> Result<HashMap<String, String>> {
    let mut map = cached_urls(&dbs.covers, ids).await?;
    let missing: Vec<String> = unique_ids(ids)
        .into_iter()
        .filter(|id| !map.contains_key(*id))
        .map(str::to_string)
        .collect();
    if missing.is_empty() || !crate::qml::online() {
        return Ok(map);
    }

    let Ok(client) = crate::qml::client(dbs).await else {
        return Ok(map);
    };
    for id in &missing {
        if let Ok(url) = client.endpoint_url("getCoverArt", &[("id", id.clone())]) {
            map.insert(id.clone(), url.to_string());
        }
    }

    // Keep the library responsive: the remote URL is already usable, so the
    // cache write is a detached follow-up for the next request. FIX-2: ids
    // already being fetched are skipped — the running job covers them.
    let missing: Vec<String> = missing.into_iter().filter(|id| claim_fetch(id)).collect();
    if missing.is_empty() {
        return Ok(map);
    }
    let pooled = dbs.covers.clone();
    let client = Arc::new(client);
    crate::runtime().spawn(async move {
        let result = fetch_ids(&pooled, &client, &missing).await;
        for id in &missing {
            release_fetch(id.clone());
        }
        if let Err(err) = result {
            eprintln!("covers: on-demand fetch failed: {err:#}");
        }
    });
    Ok(map)
}

fn unique_ids(ids: &[String]) -> Vec<&str> {
    let mut unique: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| !id.is_empty())
        .collect();
    unique.sort_unstable();
    unique.dedup();
    unique
}

async fn cached_urls(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<HashMap<String, String>> {
    let unique = unique_ids(ids);
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let dir = crate::db::covers_dir()?;
    let mut map = HashMap::with_capacity(unique.len());
    let mut thumb_jobs: Vec<(String, PathBuf)> = Vec::new();
    for chunk in unique.chunks(500) {
        let mut builder = sqlx::QueryBuilder::new("SELECT id, file FROM covers WHERE id IN (");
        crate::cache::push_in_list_refs(&mut builder, chunk);
        let rows: Vec<(String, String)> = builder.build_query_as().fetch_all(pool).await?;
        for (id, file) in rows {
            let path = dir.join(&file);
            if path.is_file() {
                let name = thumb_name(&file);
                if !dir.join(&name).is_file() {
                    thumb_jobs.push((name, path.clone()));
                }
                if let Ok(url) = reqwest::Url::from_file_path(&path) {
                    map.insert(id, url.to_string());
                }
            }
        }
    }
    // Never make a library page wait on thumbnail decoding: raw covers are
    // large, and `CoverImage` falls back to the full file until the thumb
    // lands on disk. FIX-2: thumbs already being generated are skipped too,
    // so repeated page loads share one job per file.
    let thumb_jobs: Vec<(String, PathBuf)> = thumb_jobs
        .into_iter()
        .filter(|(name, _)| claim_thumb(name))
        .collect();
    if !thumb_jobs.is_empty() {
        let dir = dir.clone();
        crate::runtime().spawn(async move {
            ensure_thumbs(&dir, thumb_jobs)
                .await
                .into_iter()
                .for_each(release_thumb);
        });
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FIX-2: a claimed id blocks its duplicates until the owning task
    /// releases it (test keys use a unique prefix; the registries are
    /// process-global).
    #[test]
    fn a_claimed_id_blocks_until_released() {
        let fetch_id = format!("claim-test-{}", std::process::id());
        let thumb_name = format!("claim-test-thumb-{}.jpg", std::process::id());
        assert!(claim_fetch(&fetch_id));
        assert!(!claim_fetch(&fetch_id), "duplicate claim is skipped");
        release_fetch(fetch_id.clone());
        assert!(claim_fetch(&fetch_id), "released ids claim again");
        release_fetch(fetch_id);

        assert!(claim_thumb(&thumb_name));
        assert!(!claim_thumb(&thumb_name));
        release_thumb(thumb_name.clone());
        assert!(claim_thumb(&thumb_name));
        release_thumb(thumb_name);
    }

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
    fn raw_file_name_ends_the_upgrade_task() {
        let file = cover_file_name("cov", "hash", "jpg");
        assert!(is_raw_file(&file));
        assert!(file.ends_with(".jpg"));
        // A second pass resolves the same canonical name, so the legacy
        // upgrade no longer forces an unconditional fetch.
        assert_eq!(file, cover_file_name("cov", "hash", "jpg"));
        assert!(!is_raw_file("legacy-cover.jpg"));
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

    fn temp_thumb_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("musika-thumb-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn thumb_name_replaces_only_the_extension() {
        assert_eq!(thumb_name("abc123.jpg"), "abc123.thumb.jpg");
        assert_eq!(thumb_name("abc123"), "abc123.thumb.jpg");
        assert_eq!(thumb_name("a.b.webp"), "a.b.thumb.jpg");
    }

    /// The single-result storage step (the chunked fetch's landing pad): a
    /// new blob is written content-addressed, recorded and queued for its
    /// thumbnail; a `304` only touches the row.
    #[tokio::test]
    async fn apply_fetched_stores_new_art_and_handles_revalidation() {
        use crate::http_client::CoverArt;

        let pool = crate::sqlite::memory_pool(sqlx::migrate!("./migrations_media")).await;
        let dir = temp_thumb_dir();
        let known = HashMap::new();
        let mut report = CoverReport::default();
        let mut jobs = Vec::new();

        let art = CoverArt {
            not_modified: false,
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            content_type: Some("image/png".to_string()),
            bytes: b"\x89PNG\r\n\x1a\nfake".to_vec(),
        };
        apply_fetched(&pool, &dir, &known, "cov1", Ok(art), &mut report, &mut jobs)
            .await
            .expect("store cover");

        assert_eq!(report.downloaded, 1);
        assert_eq!(jobs.len(), 1);
        let (file, hash, size): (String, String, i64) =
            sqlx::query_as("SELECT file, hash, size FROM covers WHERE id = 'cov1'")
                .fetch_one(&pool)
                .await
                .expect("cover row");
        assert!(file.ends_with(".png"), "content type picks the extension");
        assert_eq!(hash.len(), 32);
        assert_eq!(size, 12);
        assert!(dir.join(&file).is_file());
        assert_eq!(jobs[0].1, dir.join(&file));

        let revalidated = CoverArt {
            not_modified: true,
            etag: None,
            last_modified: None,
            content_type: None,
            bytes: Vec::new(),
        };
        apply_fetched(
            &pool,
            &dir,
            &known,
            "cov1",
            Ok(revalidated),
            &mut report,
            &mut jobs,
        )
        .await
        .expect("touch cover");
        assert_eq!(report.unchanged, 1);
        assert_eq!(jobs.len(), 1, "a 304 never queues a thumbnail");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The palette decodes the 320 px thumbnail when it exists (the whole
    /// point of the thumbnail) and falls back to the original while it does
    /// not (a just-fetched cover, offline).
    #[test]
    fn palette_art_prefers_the_thumbnail_then_the_original() {
        let dir = temp_thumb_dir();
        let raw = dir.join("raw-abc.jpg");
        std::fs::write(&raw, b"raw").expect("write raw");
        assert_eq!(
            preferred_art(&dir, "raw-abc.jpg").as_deref(),
            Some(raw.as_path()),
            "no thumbnail yet: the full cover is the only decodable art"
        );

        let thumb = dir.join("raw-abc.thumb.jpg");
        std::fs::write(&thumb, b"thumb").expect("write thumb");
        assert_eq!(
            preferred_art(&dir, "raw-abc.jpg").as_deref(),
            Some(thumb.as_path()),
            "a present thumbnail must win"
        );

        std::fs::remove_file(&raw).expect("remove raw");
        assert_eq!(
            preferred_art(&dir, "raw-abc.jpg").as_deref(),
            Some(thumb.as_path()),
            "a missing original must not hide an existing thumbnail"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thumb_is_bounded_and_written() {
        let dir = temp_thumb_dir();
        let source = dir.join("cover.png");
        image::RgbaImage::from_pixel(640, 480, image::Rgba([10, 20, 30, 255]))
            .save(&source)
            .expect("write cover");

        let bytes = build_thumb(&source).expect("build thumbnail");
        let thumb = image::load_from_memory(&bytes).expect("thumbnail decodes");
        assert_eq!((thumb.width(), thumb.height()), (THUMB_EDGE, 240));

        let name = thumb_name("cover.png");
        write_thumb(&source, &dir, &name).expect("write thumbnail");
        assert!(dir.join(&name).is_file());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thumb_falls_back_to_the_original_blob() {
        let dir = temp_thumb_dir();
        let source = dir.join("broken.img");
        std::fs::write(&source, b"not an image").expect("write blob");

        let name = thumb_name("broken.img");
        write_thumb(&source, &dir, &name).expect("fallback copy");
        assert_eq!(
            std::fs::read(dir.join(&name)).expect("read thumb"),
            b"not an image"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The size split still processes every job: small art decodes in the
    /// parallel path and an oversized blob goes through the single slot
    /// (falling back to a copy when it is not decodable).
    #[tokio::test]
    async fn ensure_thumbs_covers_both_size_buckets() {
        use image::{Rgba, RgbaImage};

        let dir = temp_thumb_dir();
        let small = dir.join("small.png");
        RgbaImage::from_pixel(64, 64, Rgba([10, 20, 30, 255]))
            .save(&small)
            .expect("small cover");
        let large = dir.join("large.img");
        std::fs::write(&large, vec![0u8; THUMB_LARGE_BYTES as usize + 1]).expect("large blob");

        let processed = ensure_thumbs(
            &dir,
            vec![
                (thumb_name("small.png"), small),
                (thumb_name("large.img"), large),
            ],
        )
        .await;

        assert_eq!(processed.len(), 2);
        assert!(dir.join("small.thumb.jpg").is_file());
        assert!(dir.join("large.thumb.jpg").is_file());
        assert_eq!(
            std::fs::read(dir.join("large.thumb.jpg"))
                .expect("fallback thumb")
                .len(),
            THUMB_LARGE_BYTES as usize + 1
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
