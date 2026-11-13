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

#![expect(
    dead_code,
    reason = "cover-cache maintenance surface is exercised by tests first; consumers land incrementally"
)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};

use anyhow::{Context, Result};
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

pub(crate) fn claim_fetch(id: &str) -> bool {
    claim(&ONGOING_FETCHES, id)
}

pub(crate) fn release_fetch(id: String) {
    release(&ONGOING_FETCHES, id);
}

pub(crate) fn claim_thumb(name: &str) -> bool {
    claim(&ONGOING_THUMBS, name)
}

pub(crate) fn release_thumb(name: String) {
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
pub(crate) const THUMB_EDGE: u32 = 320;
/// JPEG quality for thumbnails; invisible at the painted row sizes.
const THUMB_QUALITY: u8 = 82;
/// Blobs above this are thumbnailed one at a time. A compressed cover can
/// decode to an order of magnitude more RGBA than its file size, so four
/// large originals in parallel (a 6000 px cover is ~144 MB each) would spike
/// the sync's cover pass; small art keeps the parallel path.
pub(crate) const THUMB_LARGE_BYTES: u64 = 2 * 1024 * 1024;

/// Longest edge of a cached "original". Hero surfaces paint the full file, so
/// ordinary art (typically 1000-2000 px) is stored untouched, but a
/// pathological source (a 7200 px cover decodes to ~198 MB of RGBA on its own)
/// is downscaled once at ingest or by the local migration below. 4096 px keeps
/// more than every display bucket ever asked for while bounding one cover to
/// ~64 MB of RGBA.
pub(crate) const COVER_MAX_EDGE: u32 = 4096;
/// JPEG quality used only when a > [`COVER_MAX_EDGE`] original is re-encoded.
const COVER_CAP_QUALITY: u8 = 92;
/// The migration probes at most this many of the largest cached blobs per pass.
const CAP_MIGRATION_BATCH: usize = 8;
/// Blobs below this cannot plausibly exceed the edge for photographic art;
/// skipping them keeps the migration's per-pass cost negligible.
const CAP_MIGRATION_MIN_BYTES: i64 = 2 * 1024 * 1024;

/// ARC-39: remote SVG art is rasterized at ingest; the longest edge of the
/// stored PNG never exceeds this (the palette and every surface are raster).
pub(crate) const SVG_MAX_EDGE: u32 = 1024;
/// Reject SVG input larger than this before parsing.
pub(crate) const SVG_MAX_INPUT: usize = 4 * 1024 * 1024;
/// Reject rasterized output larger than this (placeholder path instead).
const SVG_MAX_OUTPUT: usize = 8 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct CoverReport {
    pub downloaded: usize,
    /// Same bytes moved to the canonical raw file name (legacy cache entry).
    pub upgraded: usize,
    pub unchanged: usize,
    pub removed: usize,
    /// Cached originals re-encoded down to [`COVER_MAX_EDGE`] by the migration.
    pub capped: usize,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct CoverRow {
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
                _ = std::fs::remove_file(dir.join(&row.file));
                _ = std::fs::remove_file(dir.join(thumb_name(&row.file)));
                crate::cache::delete_by(pool, "DELETE FROM covers WHERE id = ?1", id).await?;
                report.removed += 1;
            }
        }
    }

    // Legacy/upgraded caches can hold a raw cover without its thumbnail (the
    // job list above only covers files changed in this pass). Backfill every
    // referenced cached cover once per full pass, so legacy UI never requests a
    // .thumb.jpg that does not exist.
    if purge_unreferenced {
        let mut queued: HashSet<String> = thumb_jobs.iter().map(|(name, _)| name.clone()).collect();
        for (id, row) in known.iter() {
            if !referenced.contains(id) {
                continue;
            }
            let source = dir.join(&row.file);
            let thumb = thumb_name(&row.file);
            if source.is_file() && !dir.join(&thumb).is_file() && queued.insert(thumb.clone()) {
                thumb_jobs.push((thumb, source));
            }
        }
    }
    _ = ensure_thumbs(&dir, thumb_jobs).await;
    remove_orphans(pool, &dir).await?;
    // Full passes also cap pathological originals already in the cache. This
    // runs after the orphan sweep and keeps the old blob: removing it here
    // could break a page whose payload still names the previous URL. The next
    // sweep collects that orphan.
    if purge_unreferenced {
        report.capped = cap_cached_oversized(pool, &dir, CAP_MIGRATION_BATCH, false).await?;
    }

    Ok(report)
}

/// Stores one completed fetch: revalidation touch, content-addressed write,
/// URI-field upsert and thumbnail job. Split out of [`refresh_inner`] so the
/// network chunk and the disk/database step stay one short result apart.
pub(crate) async fn apply_fetched(
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
            // A provider can serve absurdly large art; cap it once at ingest so
            // every later hero decode (and the on-disk cache) stays bounded.
            let (original, capped) = match tokio::task::spawn_blocking(move || {
                let capped = cap_oversized_art(&bytes, extension);
                ((bytes, extension, content_type), capped)
            })
            .await
            {
                Ok(pair) => pair,
                Err(err) => {
                    eprintln!("covers: art cap task failed for {id}: {err}");
                    return Ok(());
                }
            };
            let (bytes, extension, content_type) =
                match capped {
                    Some((capped, capped_extension)) => (
                        capped,
                        capped_extension,
                        Some(content_type_for(capped_extension).to_string()),
                    ),
                    None => original,
                };

            let hash = crate::digest::md5_hex(&bytes)?;
            // Content-suffixed: changed art gets a new file/URL, so the UI
            // can never keep painting the previous image at the old path
            // (the cover id is not guaranteed to change with it). The raw
            // prefix also marks the source resolution.
            let file = cover_file_name(id, &hash, extension)?;

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
                    _ = std::fs::remove_file(dir.join(&old.file));
                    _ = std::fs::remove_file(dir.join(thumb_name(&old.file)));
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
                _ = std::fs::remove_file(dir.join(&old.file));
                _ = std::fs::remove_file(dir.join(thumb_name(&old.file)));
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
pub(crate) fn preferred_art(dir: &Path, file: &str) -> Option<PathBuf> {
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

#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the cover row; a wrapper struct would restate it"
)]
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
        // The raw blob and its thumbnail are the only files the cache keeps.
        // Legacy `<stem>_<edge>x<edge>.<ext>` display copies are stale
        // derivatives (heroes paint the original now) and go with the
        // unknown/temp blobs.
        if name.ends_with(".tmp") || (!kept.contains(&name) && !kept_thumbs.contains(&name)) {
            _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

pub(crate) fn extension(content_type: Option<&str>) -> &'static str {
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
pub(crate) fn is_svg(bytes: &[u8], content_type: Option<&str>) -> bool {
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
pub(crate) fn rasterize_svg(bytes: &[u8]) -> Option<Vec<u8>> {
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
pub(crate) fn store_art(
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

/// Canonical content-suffixed cover filename. The raw prefix is what makes the
/// one-shot legacy upgrade terminate: once written, the next pass sees it.
pub(crate) fn cover_file_name(id: &str, hash: &str, extension: &str) -> Result<String> {
    Ok(format!(
        "{RAW_PREFIX}{}.{}",
        crate::digest::md5_hex(format!("{id}\0{hash}").as_bytes())?,
        extension
    ))
}

pub(crate) fn is_raw_file(name: &str) -> bool {
    name.starts_with(RAW_PREFIX)
}

/// Derives the thumbnail file name from a cover file name. `CoverImage.legacy`
/// computes the same name from the resolved URL, so keep the two in sync.
pub(crate) fn thumb_name(file: &str) -> String {
    let stem = Path::new(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file);
    format!("{stem}.thumb.jpg")
}

pub(crate) fn build_thumb(source: &Path) -> Result<Vec<u8>> {
    _ = crate::linux::advise_willneed(source);
    let reader = image::ImageReader::open(source)
        .with_context(|| format!("failed to open cover {}", source.display()))?
        .with_guessed_format()
        .with_context(|| format!("failed to sniff cover {}", source.display()))?;

    // ARC-RS-4.10: probe dimensions first, avoid full decode for large art.
    // A 6000x6000 cover decodes to ~144 MB RGBA; thumbnail() samples the
    // 320px output instead of running Lanczos3 over the whole source.
    let (width, height) = reader
        .into_dimensions()
        .with_context(|| format!("failed to probe cover {}", source.display()))?;

    // Small art is already cheap to decode; copy it rather than re-encode.
    if width <= THUMB_EDGE && height <= THUMB_EDGE {
        return std::fs::read(source)
            .with_context(|| format!("failed to read cover {}", source.display()));
    }

    let image = image::ImageReader::open(source)
        .with_context(|| format!("failed to open cover {}", source.display()))?
        .with_guessed_format()
        .with_context(|| format!("failed to sniff cover {}", source.display()))?
        .decode()
        .with_context(|| format!("failed to decode cover {}", source.display()))?;

    let thumb = image.thumbnail(THUMB_EDGE, THUMB_EDGE);
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, THUMB_QUALITY)
        .encode_image(&thumb.to_rgb8())
        .with_context(|| format!("failed to encode thumbnail for {}", source.display()))?;
    Ok(bytes)
}

pub(crate) fn write_thumb(source: &Path, dir: &Path, name: &str) -> Result<()> {
    let bytes = build_thumb(source).or_else(|err| {
        // A blob the decoder rejects still needs a file legacy UI can request; the
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

/// Content type for the two formats [`cap_oversized_art`] can emit.
fn content_type_for(extension: &'static str) -> &'static str {
    if extension == "png" {
        "image/png"
    } else {
        "image/jpeg"
    }
}

/// Re-encodes one image below [`COVER_MAX_EDGE`].
///
/// Returns `None` when the art is already small enough or cannot be decoded
/// (the caller then keeps the original bytes rather than rejecting the art).
/// PNG art keeps its alpha channel as PNG; every other oversized raster is
/// re-encoded to JPEG, which is what a photographic 2000+ px cover should be.
fn cap_oversized_art(bytes: &[u8], extension: &'static str) -> Option<(Vec<u8>, &'static str)> {
    let format = match extension {
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "png" => image::ImageFormat::Png,
        _ => return None,
    };
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if width.max(height) <= COVER_MAX_EDGE {
        return None;
    }
    let decoded = image::load_from_memory_with_format(bytes, format).ok()?;
    let resized = decoded.thumbnail(COVER_MAX_EDGE, COVER_MAX_EDGE);
    let mut output = std::io::Cursor::new(Vec::new());
    if format == image::ImageFormat::Png && decoded.color().has_alpha() {
        resized
            .write_to(&mut output, image::ImageFormat::Png)
            .ok()?;
        Some((output.into_inner(), "png"))
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut output,
            COVER_CAP_QUALITY,
        )
        .encode_image(&resized.to_rgb8())
        .ok()?;
        Some((output.into_inner(), "jpg"))
    }
}

/// Downscales already-cached originals above [`COVER_MAX_EDGE`].
///
/// Runs in the core (daemon startup and full cover passes), never on the UI
/// path, and needs no network: the file is read, capped and re-stored under
/// the canonical content-addressed name, a thumbnail job is queued, and the
/// provider validators are kept so the next sync can still answer `304`.
/// `remove_old` is only safe when no UI can hold the previous URL (startup,
/// before the UI socket exists); a running sync leaves the old blob for the
/// next orphan sweep instead of breaking a page decoded from the old payload.
/// Returns how many blobs were re-encoded.
pub async fn cap_cached_oversized(
    pool: &SqlitePool,
    dir: &Path,
    limit: usize,
    remove_old: bool,
) -> Result<usize> {
    if limit == 0 {
        return Ok(0);
    }
    let rows: Vec<CoverRow> = sqlx::query_as(
        "SELECT id, file, hash, etag, last_modified FROM covers
         WHERE size > ?1 ORDER BY size DESC LIMIT ?2",
    )
    .bind(CAP_MIGRATION_MIN_BYTES)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    let mut capped = 0usize;
    let mut thumb_jobs: Vec<(String, PathBuf)> = Vec::new();
    for row in rows {
        let path = dir.join(&row.file);
        let Ok((width, height)) = image::image_dimensions(&path) else {
            continue;
        };
        if width.max(height) <= COVER_MAX_EDGE {
            continue;
        }
        let extension: &'static str = match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("png") => "png",
            Some(ext) if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") => {
                "jpg"
            }
            _ => continue,
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let capped_art = match tokio::task::spawn_blocking(move || {
            cap_oversized_art(&bytes, extension)
        })
        .await
        {
            Ok(capped) => capped,
            Err(err) => {
                eprintln!("covers: oversized-art task failed for {}: {err}", row.file);
                continue;
            }
        };
        let Some((bytes, extension)) = capped_art else {
            continue;
        };
        let hash = crate::digest::md5_hex(&bytes)?;
        let file = cover_file_name(&row.id, &hash, extension)?;
        crate::cache::write_atomic(dir, &file, &bytes)?;
        thumb_jobs.push((thumb_name(&file), dir.join(&file)));
        upsert(
            pool,
            &row.id,
            &file,
            &hash,
            Some(content_type_for(extension)),
            bytes.len() as i64,
            row.etag.as_deref(),
            row.last_modified.as_deref(),
        )
        .await?;
        if remove_old {
            _ = std::fs::remove_file(&path);
            _ = std::fs::remove_file(dir.join(thumb_name(&row.file)));
        }
        capped += 1;
    }
    if !thumb_jobs.is_empty() {
        _ = ensure_thumbs(dir, thumb_jobs).await;
    }
    Ok(capped)
}

/// Generates missing thumbnails on a bounded blocking pool: decoding is CPU
/// work, and the Qt image reader only decodes one file at a time. Returns
/// the thumb names it processed so the caller can release its claims.
///
/// Large blobs are decoded one at a time ([`THUMB_LARGE_BYTES`]); small blobs
/// keep a quarter-core worker batch. The blob size is a cheap stand-in for
/// decoded dimensions and avoids probing every file twice.
pub(crate) async fn ensure_thumbs(dir: &Path, jobs: Vec<(String, PathBuf)>) -> Vec<String> {
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
    let concurrency = crate::app::worker_limit();
    for chunk in small.chunks(concurrency) {
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

/// The `file://` URL for one cover id, borrowed from the map.
pub(crate) fn url_for_ref<'a>(id: Option<&str>, covers: &'a HashMap<String, String>) -> &'a str {
    id.and_then(|id| covers.get(id))
        .map(String::as_str)
        .unwrap_or_default()
}

/// Owned twin of [`url_for_ref`] for payload builders.
pub(crate) fn url_for(id: Option<&str>, covers: &HashMap<String, String>) -> String {
    url_for_ref(id, covers).to_string()
}

/// Resolves a batch of optional cover ids without the caller re-collecting
/// them: the one place list loaders and queue builders turn rows into URLs.
pub(crate) async fn url_map_for<'a, I>(
    dbs: &crate::db::Dbs,
    ids: I,
) -> Result<HashMap<String, String>>
where
    I: IntoIterator<Item = &'a Option<String>>,
{
    let ids: Vec<String> = ids.into_iter().flatten().cloned().collect();
    url_map_of(dbs, &ids).await
}

/// Maps only the requested cover ids to cover URLs, in bounded batches.
/// Payload loaders use this so a page's cost follows the page, never the whole
/// cover cache.
///
/// Cache first: a missing cover is resolved to the provider's authenticated
/// `getCoverArt` URL right away and cached in the background, so library pages
/// never wait on artwork. The UI can stream the remote URL while the cache is
/// cold. No-credentials keeps the cache-only result.
pub async fn url_map_of(dbs: &crate::db::Dbs, ids: &[String]) -> Result<HashMap<String, String>> {
    let mut map = cached_urls(&dbs.covers, ids).await?;
    let missing: Vec<String> = unique_ids(ids)
        .into_iter()
        .filter(|id| !map.contains_key(*id))
        .map(str::to_string)
        .collect();
    if missing.is_empty() {
        return Ok(map);
    }

    // The provider URL is the streaming fallback whenever saved credentials
    // can build a client. This deliberately does not depend on the UI's
    // `online` latch: a restored session starts with the latch clear until a
    // network request succeeds, but cover art must not stay blank meanwhile.
    let Ok(client) = crate::session::client(dbs).await else {
        return Ok(map);
    };
    for id in &missing {
        let url = row_cover_url(&client, id);
        if !url.is_empty() {
            map.insert(id.clone(), url);
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
    crate::app::runtime().spawn(async move {
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

/// The provider's authenticated `getCoverArt` URL for one cover id.
///
/// Unlike the cache-first mapping in [`url_map_of`], this always names the
/// remote object: the UI keeps it as a fallback for a cached file that fails
/// to decode or disappears after the URL was resolved.
pub(crate) fn provider_cover_url(client: &SubsonicClient, id: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    client
        .endpoint_url("getCoverArt", &[("id", id.to_string())])
        .map(|url| url.to_string())
        .unwrap_or_default()
}

/// The provider URL a *row* should stream when the local cache is cold.
///
/// Rows paint at most ~90 logical pixels, so requesting the provider's
/// row-sized variant avoids downloading and decoding the original (median
/// ~0.7 MB, p90 3-6 MB) just to show a thumbnail. Hero/fallback callers keep
/// using [`provider_cover_url`], which has no `size` parameter.
pub(crate) fn row_cover_url(client: &SubsonicClient, id: &str) -> String {
    client
        .endpoint_url(
            "getCoverArt",
            &[("id", id.to_string()), ("size", "320".to_string())],
        )
        .map(|url| url.to_string())
        .unwrap_or_default()
}

/// Remote fallback URL for a cover id, or empty when no saved session exists.
pub async fn remote_url_for(dbs: &crate::db::Dbs, id: Option<&str>) -> String {
    let Some(id) = id.filter(|id| !id.is_empty()) else {
        return String::new();
    };
    match crate::session::client(dbs).await {
        Ok(client) => provider_cover_url(&client, id),
        Err(_) => String::new(),
    }
}

/// Remote fallback URLs for a batch of ids, sharing one saved session.
pub(crate) async fn remote_map_for<'a, I>(
    dbs: &crate::db::Dbs,
    ids: I,
) -> Result<HashMap<String, String>>
where
    I: IntoIterator<Item = &'a Option<String>>,
{
    let ids: Vec<String> = ids.into_iter().flatten().cloned().collect();
    remote_map_of(dbs, &ids).await
}

/// Remote fallback URLs for the unique non-empty ids, or an empty map when no
/// saved session exists.
pub(crate) async fn remote_map_of(
    dbs: &crate::db::Dbs,
    ids: &[String],
) -> Result<HashMap<String, String>> {
    let unique = unique_ids(ids);
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let Ok(client) = crate::session::client(dbs).await else {
        return Ok(HashMap::new());
    };
    let mut map = HashMap::with_capacity(unique.len());
    for id in unique {
        let url = provider_cover_url(&client, id);
        if !url.is_empty() {
            map.insert(id.to_string(), url);
        }
    }
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

/// One-shot repair for caches whose raw covers predate thumbnail generation:
/// scans the cover index, claims the missing thumb names and generates them on
/// the bounded blocking pool (the same path as the lazy on-demand jobs).
pub(crate) async fn backfill_missing_thumbs(dbs: &crate::db::Dbs) -> usize {
    let dir = match crate::db::covers_dir() {
        Ok(dir) => dir,
        Err(_) => return 0,
    };
    let rows: Vec<(String, String)> = match sqlx::query_as("SELECT id, file FROM covers")
        .fetch_all(&dbs.covers)
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!("covers: thumbnail scan failed: {err:#}");
            return 0;
        }
    };
    let mut jobs: Vec<(String, std::path::PathBuf)> = Vec::new();
    for (_id, file) in rows {
        let source = dir.join(&file);
        let name = thumb_name(&file);
        if source.is_file() && !dir.join(&name).is_file() && claim_thumb(&name) {
            jobs.push((name, source));
        }
    }
    if jobs.is_empty() {
        return 0;
    }
    let processed = ensure_thumbs(&dir, jobs).await;
    let count = processed.len();
    for name in processed {
        release_thumb(name);
    }
    count
}

async fn cached_urls(pool: &SqlitePool, ids: &[String]) -> Result<HashMap<String, String>> {
    let unique = unique_ids(ids);
    if unique.is_empty() {
        return Ok(HashMap::new());
    }
    let dir = crate::db::covers_dir()?;
    let mut map = HashMap::with_capacity(unique.len());
    let mut thumb_jobs: Vec<(String, PathBuf)> = Vec::new();
    for chunk in unique.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new("SELECT id, file FROM covers WHERE id IN (");
        crate::cache::push_in_list(&mut builder, chunk);
        let rows: Vec<(String, String)> = builder.build_query_as().fetch_all(pool).await?;
        for (id, file) in rows {
            let path = dir.join(&file);
            if path.is_file()
                && let Ok(url) = reqwest::Url::from_file_path(&path)
            {
                let name = thumb_name(&file);
                if dir.join(&name).is_file() {
                    map.insert(id, url.to_string());
                } else {
                    // Tell CoverImage this cover has no thumbnail yet: it
                    // uses the full file directly instead of requesting a
                    // missing .thumb.jpg (which Qt logs as an error), while
                    // the job below writes the real thumbnail for next time.
                    map.insert(id, format!("{url}#no-thumb"));
                    thumb_jobs.push((name, path.clone()));
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
        crate::app::runtime().spawn(async move {
            ensure_thumbs(&dir, thumb_jobs)
                .await
                .into_iter()
                .for_each(release_thumb);
        });
    }
    Ok(map)
}

#[cfg(test)]
mod orphan_tests {
    use super::*;
    use std::str::FromStr;

    fn memory_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options);
        crate::app::runtime().block_on(async {
            let pool = pool.await.unwrap();
            sqlx::migrate!("./migrations_covers")
                .run(&pool)
                .await
                .unwrap();
            pool
        })
    }

    #[test]
    fn sweep_keeps_raw_and_thumb_and_removes_stale_derivatives() {
        let dir = std::env::temp_dir().join(format!(
            "musika-covers-sweep-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "raw-abc.jpg",
            "raw-abc.thumb.jpg",
            "raw-abc_1024x1024.jpg",
            "raw-abc_256x256.jpg",
            "stray.jpg",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let pool = memory_pool();
        crate::app::runtime().block_on(async {
            sqlx::query("INSERT INTO covers (id, file, fetched_at) VALUES ('abc', 'raw-abc.jpg', '0')")
                .execute(&pool)
                .await
                .unwrap();
            remove_orphans(&pool, &dir).await.unwrap();
        });
        assert!(dir.join("raw-abc.jpg").exists(), "raw blob removed");
        assert!(dir.join("raw-abc.thumb.jpg").exists(), "thumb removed");
        assert!(
            !dir.join("raw-abc_1024x1024.jpg").exists(),
            "stale display derivative kept"
        );
        assert!(
            !dir.join("raw-abc_256x256.jpg").exists(),
            "stale display derivative kept"
        );
        assert!(!dir.join("stray.jpg").exists(), "unknown blob kept");
        _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    fn jpeg_bytes(width: u32, height: u32, pixel: [u8; 3]) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb(pixel));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .expect("encode fixture");
        bytes.into_inner()
    }

    #[test]
    fn oversized_art_is_capped_and_small_art_is_left_alone() {
        let big = jpeg_bytes(5000, 2500, [10, 20, 30]);
        let (capped, extension) = cap_oversized_art(&big, "jpg").expect("large art capped");
        assert_eq!(extension, "jpg");
        assert!(
            capped.len() < big.len(),
            "re-encoded art should be smaller: {} vs {}",
            capped.len(),
            big.len()
        );
        let decoded = image::load_from_memory(&capped).expect("capped art decodes");
        let (width, height) = (decoded.width(), decoded.height());
        assert_eq!((width, height), (COVER_MAX_EDGE, COVER_MAX_EDGE / 2));

        let small = jpeg_bytes(800, 600, [1, 2, 3]);
        assert!(
            cap_oversized_art(&small, "jpg").is_none(),
            "art within the edge must not be re-encoded"
        );
    }

    #[test]
    fn oversized_png_with_alpha_stays_png() {
        let mut pixels = image::RgbaImage::from_pixel(5000, 5000, image::Rgba([0, 0, 0, 0]));
        pixels.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("encode png fixture");
        let (capped, extension) =
            cap_oversized_art(&bytes.into_inner(), "png").expect("large png capped");
        assert_eq!(extension, "png");
        let decoded = image::load_from_memory(&capped).expect("capped png decodes");
        assert_eq!(decoded.width().max(decoded.height()), COVER_MAX_EDGE);
        assert!(decoded.color().has_alpha());
    }
}
