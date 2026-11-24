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
/// Artist portraits live in the same cover store under this prefix; the sync
/// cover pass must not purge them as unreferenced `getCoverArt` ids.
const ARTIST_IMAGE_PREFIX: &str = "artist-image:";

/// FIX-2/N-10: in-flight registries — a repeated page reload/scroll while a
/// cover is still missing (or its thumbnail is not generated yet) must not
/// start the same background work again. A claim is owned by an RAII guard
/// held for the claiming task's life, so a panic or cancellation releases it
/// instead of leaking the id forever. The lock is taken briefly, never across
/// an await.
static ONGOING_FETCHES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static ONGOING_THUMBS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Owned claim on one registry key; the key is released on drop (N-10).
pub(crate) struct Claim {
    set: &'static Mutex<HashSet<String>>,
    key: String,
}

impl Claim {
    pub(crate) fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.set.lock_unguarded().remove(&self.key);
    }
}

/// Claims `id` for a cover fetch, or `None` when another task owns it.
pub(crate) fn claim_fetch(id: &str) -> Option<Claim> {
    claim(&ONGOING_FETCHES, id)
}

/// Claims `name` for a thumbnail job, or `None` when another task owns it.
pub(crate) fn claim_thumb(name: &str) -> Option<Claim> {
    claim(&ONGOING_THUMBS, name)
}

fn claim(set: &'static Mutex<HashSet<String>>, key: &str) -> Option<Claim> {
    if !set.lock_unguarded().insert(key.to_string()) {
        return None;
    }
    Some(Claim {
        set,
        key: key.to_string(),
    })
}

/// Whether a thumbnail job is currently in flight for `name`.
fn thumb_in_flight(name: &str) -> bool {
    ONGOING_THUMBS.lock_unguarded().contains(name)
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

/// Fetches specific cover ids unconditionally (no ETag/Last-Modified),
/// replacing the cached image. Playlist covers are regenerated server-side
/// while their ids stay stable, so a normal conditional fetch would keep the
/// old thumbnail forever.
pub(crate) async fn fetch_ids_forced(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    ids: &[String],
) -> Result<CoverReport> {
    let ids: BTreeSet<String> = ids.iter().filter(|id| !id.is_empty()).cloned().collect();
    if ids.is_empty() {
        return Ok(CoverReport::default());
    }
    let forced: Vec<String> = ids.iter().cloned().collect();
    refresh_inner(pool, client, &ids, &forced, false).await
}

/// The cached rows [`refresh_inner`] needs to decide what to fetch.
///
/// A full pass needs the whole table (it purges and thumbnails every cached
/// entry). The on-demand path only reads the requested ids: loading the entire
/// `covers` table just to look up a page's covers was an N+1-style full scan
/// (N-20), and the `IN (...)` binds are chunked so a large page cannot exceed
/// SQLite's parameter limit.
pub(crate) async fn known_covers(
    pool: &SqlitePool,
    referenced: &BTreeSet<String>,
    purge_unreferenced: bool,
) -> Result<HashMap<String, CoverRow>> {
    if purge_unreferenced {
        let existing: Vec<CoverRow> =
            sqlx::query_as("SELECT id, file, hash, etag, last_modified FROM covers")
                .fetch_all(pool)
                .await?;
        return Ok(existing
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect());
    }
    let ids: Vec<&String> = referenced.iter().collect();
    let mut known = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT id, file, hash, etag, last_modified FROM covers WHERE id IN (",
        );
        crate::cache::push_in_list(&mut builder, chunk);
        let rows: Vec<CoverRow> = builder.build_query_as().fetch_all(pool).await?;
        known.extend(rows.into_iter().map(|row| (row.id.clone(), row)));
    }
    Ok(known)
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

    let known = Arc::new(known_covers(pool, referenced, purge_unreferenced).await?);

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
            if !referenced.contains(id) && !id.starts_with(ARTIST_IMAGE_PREFIX) {
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
            if !referenced.contains(id) && !id.starts_with(ARTIST_IMAGE_PREFIX) {
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
    // Only a full (single-cover-pass) refresh may sweep the cache directory:
    // the on-demand path would race another page's freshly written blob or
    // temporary file (N-04). The orphan sweep also skips recent temp files.
    if purge_unreferenced {
        remove_orphans(pool, &dir).await?;
        // Full passes also cap pathological originals already in the cache.
        // This runs after the orphan sweep and keeps the old blob: removing it
        // here could break a page whose payload still names the previous URL.
        // The next sweep collects that orphan.
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
            update_validators(pool, id, art.etag.as_deref(), art.last_modified.as_deref()).await?;
            report.unchanged += 1;
        }
        Ok(art) => {
            let Some((bytes, extension, content_type)) =
                store_art(art.bytes, art.content_type.as_deref())
            else {
                // Undecodable/oversize art: never store a blob the renderer
                // cannot decode; the placeholder path owns it (ARC-39, N-01).
                eprintln!("covers: rejected undecodable cover {id}");
                return Ok(());
            };
            // A provider can serve absurdly large art; cap it once at ingest so
            // every later hero decode (and the on-disk cache) stays bounded.
            let (original, capped) = match tokio::task::spawn_blocking(move || {
                let capped = cap_oversized_art(&bytes);
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
            let (bytes, extension, content_type) = match capped {
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

/// Records a revalidation: refreshes the validators the `304` carried and the
/// check time, keeping the cached copy untouched (N-21).
async fn update_validators(
    pool: &SqlitePool,
    id: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE covers SET fetched_at = ?2, etag = COALESCE(?3, etag),
                           last_modified = COALESCE(?4, last_modified)
         WHERE id = ?1",
    )
    .bind(id)
    .bind(crate::consts::now_secs().to_string())
    .bind(etag)
    .bind(last_modified)
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
        "INSERT INTO covers (id, file, hash, content_type, size, etag, last_modified, fetched_at, cap_checked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
             file = excluded.file,
             hash = excluded.hash,
             content_type = excluded.content_type,
             size = excluded.size,
             etag = excluded.etag,
             last_modified = excluded.last_modified,
             fetched_at = excluded.fetched_at,
             cap_checked = excluded.cap_checked",
    )
    .bind(id)
    .bind(file)
    .bind(hash)
    .bind(content_type)
    .bind(size)
    .bind(etag)
    .bind(last_modified)
    .bind(crate::consts::now_secs().to_string())
    .bind(i64::from(COVER_MAX_EDGE))
    .execute(pool)
    .await?;
    Ok(())
}

/// Temp/uncommitted files younger than this are never swept: an in-flight
/// fetch may have written its temp file or renamed blob but not yet committed
/// the row, and a page may already be painting it (N-04).
const ORPHAN_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Deletes temp leftovers and any blob the index does not know about.
///
/// Only the single full cover pass calls this (`purge_unreferenced`); the
/// on-demand and artist-image follow-ups must leave the directory alone while
/// another pass may be writing into it (N-04). The directory scan runs on the
/// blocking pool (N-18).
async fn remove_orphans(pool: &SqlitePool, dir: &std::path::Path) -> Result<()> {
    let kept: Vec<String> = sqlx::query_scalar("SELECT file FROM covers")
        .fetch_all(pool)
        .await?;
    let kept: HashSet<String> = kept.into_iter().collect();
    let kept_thumbs: HashSet<String> = kept.iter().map(|file| thumb_name(file)).collect();
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || sweep_orphans(&dir, &kept, &kept_thumbs))
        .await
        .context("cover orphan sweep task failed")?;
    Ok(())
}

fn sweep_orphans(dir: &Path, kept: &HashSet<String>, kept_thumbs: &HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // The raw blob and its thumbnail are the only files the cache keeps.
        // A claimed thumbnail is still being generated; a recent file may be a
        // fetch that has not recorded its row yet. Never delete those (N-04).
        if kept.contains(&name) || kept_thumbs.contains(&name) || thumb_in_flight(&name) {
            continue;
        }
        let path = entry.path();
        if is_recent(&path) {
            continue;
        }
        // Legacy `<stem>_<edge>x<edge>.<ext>` display copies are stale
        // derivatives (heroes paint the original now) and go with the
        // unknown/temp blobs.
        _ = std::fs::remove_file(path);
    }
}

fn is_recent(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < ORPHAN_MIN_AGE)
}

/// Maps a sniffed raster format to the cache's extension and MIME type. Only
/// formats enabled in this build are accepted; every other header format is
/// not cache art (N-01).
fn raster_kind(format: image::ImageFormat) -> Option<(&'static str, &'static str)> {
    // `ImageFormat` is `#[non_exhaustive]`, so it is matched through its
    // extension table rather than an enum match with a wildcard arm.
    let extension = *format.extensions_str().first()?;
    match extension {
        "jpg" | "png" | "webp" | "gif" => Some((extension, format.to_mime_type())),
        _ => None,
    }
}

/// Sniffs and header-validates a raster image: the magic bytes must name a
/// format this build decodes and the header must report non-zero dimensions.
///
/// An error envelope (JSON/XML/HTML) that arrived with an `image/*` content
/// type fails here (N-01), and a content type that disagrees with the bytes is
/// ignored in favour of the sniffed format (N-12).
pub(crate) fn sniff_raster(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.is_empty() {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    let kind = raster_kind(format)?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some(kind)
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
/// (ARC-39); raster payloads must carry a real image header, and the bytes win
/// over a mismatched `Content-Type` (N-01, N-12). `None` means the body is not
/// cache art at all and must not reach a file or a row.
pub(crate) fn store_art(
    bytes: Vec<u8>,
    content_type: Option<&str>,
) -> Option<(Vec<u8>, &'static str, Option<String>)> {
    if is_svg(&bytes, content_type) {
        let png = rasterize_svg(&bytes)?;
        return Some((png, "png", Some("image/png".to_string())));
    }

    let (extension, sniffed_type) = sniff_raster(&bytes)?;
    Some((bytes, extension, Some(sniffed_type.to_string())))
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
/// Returns `None` when the art is already small enough or is not a raster
/// format this build decodes (the caller then keeps the original bytes rather
/// than rejecting the art). The format is sniffed from the bytes, so a wrong
/// file extension or `Content-Type`, and WebP originals, are handled instead
/// of skipped (N-12). Art with an alpha channel keeps it as PNG; every other
/// oversized raster becomes JPEG, which is what a photographic cover is.
fn cap_oversized_art(bytes: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let format = image::guess_format(bytes).ok()?;
    raster_kind(format)?;
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
    if decoded.color().has_alpha() {
        resized
            .write_to(&mut output, image::ImageFormat::Png)
            .ok()?;
        Some((output.into_inner(), "png"))
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, COVER_CAP_QUALITY)
            .encode_image(&resized.to_rgb8())
            .ok()?;
        Some((output.into_inner(), "jpg"))
    }
}

/// One candidate row for [`cap_cached_oversized`].
#[derive(Debug, sqlx::FromRow)]
struct CapRow {
    id: String,
    file: String,
    etag: Option<String>,
    last_modified: Option<String>,
    size: i64,
}

/// Outcome of probing a cached cover's header dimensions against
/// [`COVER_MAX_EDGE`] without reading the full file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeOutcome {
    /// Header dimensions are within `COVER_MAX_EDGE`, or the format is not a
    /// supported raster format that [`cap_oversized_art`] can re-encode.
    WithinLimit,
    /// Longest edge exceeds `COVER_MAX_EDGE`; full decode + re-encode needed.
    Oversized,
    /// The file could not be opened or read; leave unchecked for a later pass.
    IoError,
}

/// Reads at most the image header bytes of `path` to determine its pixel
/// dimensions.
///
/// Unlike `std::fs::read` + `image::ImageReader::into_dimensions` (whose JPEG
/// backend buffers the whole stream into memory), this parses PNG, GIF, WebP
/// and JPEG headers directly over a small buffered reader and uses
/// `Seek::seek_relative` to skip large JPEG APP/EXIF/ICC segments without
/// reading them. Falls back to `image::ImageReader` only when a recognized
/// raster format has an unusual header layout.
fn probe_file_dimensions(path: &Path) -> std::io::Result<Option<(u32, u32)>> {
    use std::io::BufReader;

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(8 * 1024, file);
    let mut head = [0u8; 30];
    let n = read_prefix(&mut reader, &mut head)?;
    if n < 2 {
        return Ok(None);
    }
    let slice = &head[..n];

    if let Some(dims) = parse_png_dimensions(slice)
        .or_else(|| parse_gif_dimensions(slice))
        .or_else(|| parse_webp_dimensions(slice))
    {
        return Ok(Some(dims));
    }

    if slice[0] == 0xFF
        && slice[1] == 0xD8
        && let Some(dims) = parse_jpeg_dimensions(&mut reader, n)?
    {
        return Ok(Some(dims));
    }

    // Only pay for the generic `image` fallback when the magic bytes actually
    // belong to a raster format this build decodes (e.g. a rare DNL JPEG or
    // non-standard WebP chunk order). Non-raster blobs return `None` right away.
    let is_supported_raster = image::guess_format(slice)
        .ok()
        .and_then(raster_kind)
        .is_some();
    if !is_supported_raster {
        return Ok(None);
    }
    Ok(image::ImageReader::open(path)
        .ok()
        .and_then(|reader| reader.with_guessed_format().ok())
        .and_then(|reader| reader.into_dimensions().ok()))
}

fn read_prefix<R: std::io::Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

fn parse_png_dimensions(head: &[u8]) -> Option<(u32, u32)> {
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if head.len() < 24 || head[..8] != PNG_MAGIC || &head[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    (width > 0 && height > 0).then_some((width, height))
}

fn parse_gif_dimensions(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 10 || (&head[..6] != b"GIF87a" && &head[..6] != b"GIF89a") {
        return None;
    }
    let width = u32::from(u16::from_le_bytes([head[6], head[7]]));
    let height = u32::from(u16::from_le_bytes([head[8], head[9]]));
    (width > 0 && height > 0).then_some((width, height))
}

fn parse_webp_dimensions(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 30 || &head[..4] != b"RIFF" || &head[8..12] != b"WEBP" {
        return None;
    }
    let (width, height) = match &head[12..16] {
        b"VP8 " => {
            if head[23..26] != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([head[26], head[27]]) & 0x3FFF);
            let height = u32::from(u16::from_le_bytes([head[28], head[29]]) & 0x3FFF);
            (width, height)
        }
        b"VP8L" => {
            if head[20] != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes([head[21], head[22], head[23], head[24]]);
            let width = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            (width, height)
        }
        b"VP8X" => {
            let width =
                (u32::from(head[24]) | (u32::from(head[25]) << 8) | (u32::from(head[26]) << 16))
                    + 1;
            let height =
                (u32::from(head[27]) | (u32::from(head[28]) << 8) | (u32::from(head[29]) << 16))
                    + 1;
            (width, height)
        }
        _ => return None,
    };
    (width > 0 && height > 0).then_some((width, height))
}

/// Walks JPEG markers starting right after SOI (`0xFF 0xD8`), seeking past
/// non-SOF segments so large EXIF/ICC/XMP payloads are never read into memory.
fn parse_jpeg_dimensions(
    reader: &mut std::io::BufReader<std::fs::File>,
    already_read: usize,
) -> std::io::Result<Option<(u32, u32)>> {
    use std::io::Read;

    // Rewind to byte 2 (immediately after `0xFF 0xD8`) without dropping the
    // 8 KiB buffer that `read_prefix` already populated.
    reader.seek_relative(2 - already_read as i64)?;

    const MAX_SEGMENTS: usize = 512;
    let mut byte = [0u8; 1];
    for _ in 0..MAX_SEGMENTS {
        if reader.read_exact(&mut byte).is_err() {
            return Ok(None);
        }
        if byte[0] != 0xFF {
            return Ok(None);
        }
        // Skip any `0xFF` fill bytes before the marker code.
        let marker = loop {
            if reader.read_exact(&mut byte).is_err() {
                return Ok(None);
            }
            if byte[0] != 0xFF {
                break byte[0];
            }
        };
        match marker {
            // Standalone markers with no length field.
            0x00 | 0x01 | 0xD0..=0xD8 => continue,
            // EOI or Start of Scan: no SOF found before compressed image data.
            0xD9 | 0xDA => return Ok(None),
            _ => {}
        }
        let mut len_bytes = [0u8; 2];
        if reader.read_exact(&mut len_bytes).is_err() {
            return Ok(None);
        }
        let seg_len = u16::from_be_bytes(len_bytes);
        if seg_len < 2 {
            return Ok(None);
        }
        let is_sof = matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        );
        if is_sof {
            if seg_len < 7 {
                return Ok(None);
            }
            let mut sof = [0u8; 5];
            if reader.read_exact(&mut sof).is_err() {
                return Ok(None);
            }
            let height = u32::from(u16::from_be_bytes([sof[1], sof[2]]));
            let width = u32::from(u16::from_be_bytes([sof[3], sof[4]]));
            // `height == 0` signals a deferred DNL marker; let the fallback
            // decoder handle it.
            return Ok((width > 0 && height > 0).then_some((width, height)));
        }
        if reader.seek_relative(i64::from(seg_len - 2)).is_err() {
            return Ok(None);
        }
    }
    Ok(None)
}

fn probe_cap_candidate(path: &Path, max_edge: u32) -> ProbeOutcome {
    match probe_file_dimensions(path) {
        Ok(Some((width, height))) if width.max(height) > max_edge => ProbeOutcome::Oversized,
        Ok(_) => ProbeOutcome::WithinLimit,
        Err(_) => ProbeOutcome::IoError,
    }
}

async fn mark_cap_checked(pool: &SqlitePool, ids: &[String], edge: u32) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for chunk in ids.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new("UPDATE covers SET cap_checked = ");
        builder.push_bind(i64::from(edge));
        builder.push(" WHERE id IN (");
        crate::cache::push_in_list(&mut builder, chunk);
        builder.build().execute(tx.as_mut()).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Downscales already-cached originals above [`COVER_MAX_EDGE`].
///
/// Runs in the core (daemon startup and full cover passes), never on the UI
/// path, and needs no network: unchecked candidates above
/// [`CAP_MIGRATION_MIN_BYTES`] have only their image headers probed first,
/// verified in-edge rows are marked `cap_checked = COVER_MAX_EDGE` so future
/// launches skip them in O(1), and only genuinely oversized files are read in
/// full, capped and re-stored under the canonical content-addressed name.
/// `remove_old` is only safe when no UI can hold the previous URL (startup,
/// before the UI socket exists); a running sync leaves the old blob for the
/// next orphan sweep instead of breaking a page decoded from the old payload.
///
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
    const CANDIDATE_BATCH: usize = 32;
    let mut capped = 0usize;
    let mut thumb_jobs: Vec<(String, PathBuf)> = Vec::new();
    let max_edge = COVER_MAX_EDGE;
    let max_edge_i64 = i64::from(max_edge);
    // Keyset cursor over `(size DESC, id ASC)` restricted to rows not yet
    // verified at `<= max_edge`. Preserves the N-12 anti-starvation walk while
    // keeping steady-state startup to a single zero-row index lookup.
    let mut cursor: Option<(i64, String)> = None;
    'migration: loop {
        let rows: Vec<CapRow> = match &cursor {
            None => {
                sqlx::query_as(
                    "SELECT id, file, etag, last_modified, size FROM covers
                     WHERE size > ?1 AND (cap_checked = 0 OR cap_checked > ?2)
                     ORDER BY size DESC, id ASC LIMIT ?3",
                )
                .bind(CAP_MIGRATION_MIN_BYTES)
                .bind(max_edge_i64)
                .bind(CANDIDATE_BATCH as i64)
                .fetch_all(pool)
                .await?
            }
            Some((size, id)) => {
                sqlx::query_as(
                    "SELECT id, file, etag, last_modified, size FROM covers
                     WHERE size > ?1 AND (cap_checked = 0 OR cap_checked > ?2)
                       AND (size < ?3 OR (size = ?3 AND id > ?4))
                     ORDER BY size DESC, id ASC LIMIT ?5",
                )
                .bind(CAP_MIGRATION_MIN_BYTES)
                .bind(max_edge_i64)
                .bind(*size)
                .bind(id)
                .bind(CANDIDATE_BATCH as i64)
                .fetch_all(pool)
                .await?
            }
        };
        if rows.is_empty() {
            break;
        }
        // Probe all headers in this batch on one blocking task (N-18), reading
        // only a few bytes per file instead of loading multi-MiB blobs.
        let probe_paths: Vec<PathBuf> = rows.iter().map(|row| dir.join(&row.file)).collect();
        let outcomes = match tokio::task::spawn_blocking(move || {
            probe_paths
                .iter()
                .map(|path| probe_cap_candidate(path, max_edge))
                .collect::<Vec<_>>()
        })
        .await
        {
            Ok(outcomes) => outcomes,
            Err(err) => {
                eprintln!("covers: oversized-art header probe task failed: {err}");
                break;
            }
        };

        let mut checked_ids: Vec<String> = Vec::new();
        let mut hit_limit = false;
        for (row, outcome) in rows.into_iter().zip(outcomes) {
            cursor = Some((row.size, row.id.clone()));
            match outcome {
                ProbeOutcome::WithinLimit => {
                    checked_ids.push(row.id);
                    continue;
                }
                ProbeOutcome::IoError => continue,
                ProbeOutcome::Oversized => {}
            }
            if capped >= limit {
                hit_limit = true;
                break;
            }
            // Only genuinely oversized blobs are read in full and re-encoded.
            let path = dir.join(&row.file);
            let capped_art = match tokio::task::spawn_blocking(move || cap_cached_file(&path)).await
            {
                Ok(capped) => capped,
                Err(err) => {
                    eprintln!("covers: oversized-art task failed for {}: {err}", row.file);
                    continue;
                }
            };
            let Some((bytes, extension)) = capped_art else {
                // Header exceeded the edge, but full decode/re-encode declined
                // (e.g. corrupt payload); mark checked so we do not re-read it
                // on every startup.
                checked_ids.push(row.id);
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
                _ = std::fs::remove_file(dir.join(&row.file));
                _ = std::fs::remove_file(dir.join(thumb_name(&row.file)));
            }
            capped += 1;
        }
        mark_cap_checked(pool, &checked_ids, max_edge).await?;
        if hit_limit || capped >= limit {
            break 'migration;
        }
    }
    if !thumb_jobs.is_empty() {
        _ = ensure_thumbs(dir, thumb_jobs).await;
    }
    Ok(capped)
}

/// Blocking half of the migration: probes one cached blob and re-encodes it
/// when its longest edge exceeds [`COVER_MAX_EDGE`].
fn cap_cached_file(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    let bytes = std::fs::read(path).ok()?;
    cap_oversized_art(&bytes)
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
    // cache write is a detached follow-up for the next request. FIX-2/N-10:
    // ids already being fetched are skipped — the running job covers them,
    // and its RAII claim releases the ids even if the task is cancelled.
    let mut claims: Vec<Claim> = Vec::new();
    for id in missing {
        if let Some(claim) = claim_fetch(&id) {
            claims.push(claim);
        }
    }
    if claims.is_empty() {
        return Ok(map);
    }
    let missing: Vec<String> = claims.iter().map(|claim| claim.key().to_string()).collect();
    let pooled = dbs.covers.clone();
    let client = Arc::new(client);
    crate::app::runtime().spawn(async move {
        let result = fetch_ids(&pooled, &client, &missing).await;
        drop(claims);
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
    let mut claims: Vec<Claim> = Vec::new();
    for (_id, file) in rows {
        let source = dir.join(&file);
        let name = thumb_name(&file);
        if source.is_file()
            && !dir.join(&name).is_file()
            && let Some(claim) = claim_thumb(&name)
        {
            claims.push(claim);
            jobs.push((name, source));
        }
    }
    if jobs.is_empty() {
        return 0;
    }
    let processed = ensure_thumbs(&dir, jobs).await;
    drop(claims);
    processed.len()
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
    // lands on disk. FIX-2/N-10: thumbs already being generated are skipped
    // too (their claim covers the job), so repeated page loads share one job
    // per file and a cancelled task releases its claims.
    let mut claims: Vec<Claim> = Vec::new();
    let mut claimed_jobs: Vec<(String, PathBuf)> = Vec::new();
    for (name, path) in thumb_jobs {
        if let Some(claim) = claim_thumb(&name) {
            claims.push(claim);
            claimed_jobs.push((name, path));
        }
    }
    if !claimed_jobs.is_empty() {
        let dir = dir.clone();
        crate::app::runtime().spawn(async move {
            ensure_thumbs(&dir, claimed_jobs).await;
            drop(claims);
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

    /// Backdates one file so the sweep treats it as abandoned (the recent-file
    /// guard is what keeps in-flight writes alive, N-04).
    fn backdate(path: &std::path::Path) {
        let old =
            std::time::SystemTime::now() - ORPHAN_MIN_AGE - std::time::Duration::from_secs(60);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open fixture");
        file.set_modified(old).expect("backdate fixture");
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
            "abandoned.tmp",
            "abandoned.part",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
            backdate(&dir.join(name));
        }
        // A temp file from a fetch that is still running must never be swept.
        std::fs::write(dir.join("in-flight.tmp"), b"x").unwrap();
        // An unknown blob written moments ago may be a fetch whose row has not
        // landed yet: also kept.
        std::fs::write(dir.join("fresh-unknown.jpg"), b"x").unwrap();

        let pool = memory_pool();
        crate::app::runtime().block_on(async {
            sqlx::query(
                "INSERT INTO covers (id, file, fetched_at) VALUES ('abc', 'raw-abc.jpg', '0')",
            )
            .execute(&pool)
            .await
            .unwrap();
            remove_orphans(&pool, &dir).await.unwrap();
        });
        assert!(dir.join("raw-abc.jpg").exists(), "raw blob removed");
        assert!(dir.join("raw-abc.thumb.jpg").exists(), "thumb removed");
        for gone in [
            "raw-abc_1024x1024.jpg",
            "raw-abc_256x256.jpg",
            "stray.jpg",
            "abandoned.tmp",
            "abandoned.part",
        ] {
            assert!(!dir.join(gone).exists(), "stale file kept: {gone}");
        }
        assert!(
            dir.join("in-flight.tmp").exists(),
            "a recent temp file must survive the sweep"
        );
        assert!(
            dir.join("fresh-unknown.jpg").exists(),
            "a recently written unknown blob must survive the sweep"
        );
        _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod cap_tests {
    use super::*;
    use std::str::FromStr;

    fn memory_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options);
        crate::app::runtime().block_on(async {
            let pool = pool.await.unwrap();
            sqlx::migrate!("./migrations_media")
                .run(&pool)
                .await
                .unwrap();
            pool
        })
    }

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
        let (capped, extension) = cap_oversized_art(&big).expect("large art capped");
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
            cap_oversized_art(&small).is_none(),
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
        let (capped, extension) = cap_oversized_art(&bytes.into_inner()).expect("large png capped");
        assert_eq!(extension, "png");
        let decoded = image::load_from_memory(&capped).expect("capped png decodes");
        assert_eq!(decoded.width().max(decoded.height()), COVER_MAX_EDGE);
        assert!(decoded.color().has_alpha());
    }

    #[test]
    fn oversized_webp_is_capped_to_jpeg() {
        let image = image::RgbImage::from_pixel(5000, 2500, image::Rgb([9, 21, 33]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::WebP)
            .expect("encode webp fixture");
        let (capped, extension) =
            cap_oversized_art(&bytes.into_inner()).expect("large webp capped");
        assert_eq!(extension, "jpg", "opaque WebP is re-encoded to JPEG");
        let decoded = image::load_from_memory(&capped).expect("capped art decodes");
        assert_eq!(decoded.width().max(decoded.height()), COVER_MAX_EDGE);
    }

    async fn async_memory_pool() -> SqlitePool {
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

    /// N-12: the migration used to fetch only `LIMIT` candidates by descending
    /// size and stop, so it starved behind the largest blobs whenever those
    /// were already within the edge. It must walk past them to the genuinely
    /// oversized rows.
    #[tokio::test]
    async fn migration_reaches_oversized_rows_below_the_top_candidates() {
        let dir = std::env::temp_dir().join(format!(
            "musika-covers-migration-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        fn solid_jpeg(width: u32, height: u32) -> Vec<u8> {
            let image = image::RgbImage::from_pixel(width, height, image::Rgb([7, 11, 13]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut bytes, image::ImageFormat::Jpeg)
                .expect("encode fixture");
            bytes.into_inner()
        }

        std::fs::write(dir.join("raw-in-edge.jpg"), solid_jpeg(4000, 4000)).unwrap();
        std::fs::write(dir.join("raw-oversized.jpg"), solid_jpeg(5000, 2500)).unwrap();

        let pool = async_memory_pool().await;
        // The row with the largest recorded size is already within the edge;
        // the oversized row sorts below it.
        for (id, file, size) in [
            ("in-edge", "raw-in-edge.jpg", 9_000_000_i64),
            ("oversized", "raw-oversized.jpg", 3_000_000_i64),
        ] {
            sqlx::query(
                "INSERT INTO covers (id, file, hash, content_type, size, fetched_at)
                 VALUES (?1, ?2, 'h', 'image/jpeg', ?3, '0')",
            )
            .bind(id)
            .bind(file)
            .bind(size)
            .execute(&pool)
            .await
            .unwrap();
        }
        let capped = cap_cached_oversized(&pool, &dir, 1, true)
            .await
            .expect("migration");
        assert_eq!(
            capped, 1,
            "the oversized row below the largest candidate must be capped"
        );
        let file: String = sqlx::query_scalar("SELECT file FROM covers WHERE id = 'oversized'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(file, "raw-oversized.jpg", "the blob is content-addressed");
        assert!(dir.join(&file).is_file(), "capped blob written");

        // Both the in-edge row and the newly capped row must be persisted as
        // verified at `COVER_MAX_EDGE` so subsequent startups skip them.
        for id in ["in-edge", "oversized"] {
            let checked: i64 = sqlx::query_scalar("SELECT cap_checked FROM covers WHERE id = ?1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                checked,
                i64::from(COVER_MAX_EDGE),
                "row {id} must be marked cap_checked"
            );
        }

        // Proves a subsequent startup skips already-checked rows without
        // touching the filesystem: replacing `raw-in-edge.jpg` on disk with an
        // oversized image must not trigger a re-read/re-encode.
        std::fs::write(dir.join("raw-in-edge.jpg"), solid_jpeg(5000, 2500)).unwrap();
        let second = cap_cached_oversized(&pool, &dir, 16, true)
            .await
            .expect("second migration pass");
        assert_eq!(second, 0, "already-checked rows must not be re-probed");

        // If `COVER_MAX_EDGE` is lowered in the future (simulated by a row
        // checked at a higher threshold), it must be re-checked automatically.
        sqlx::query("UPDATE covers SET cap_checked = ?1 WHERE id = 'in-edge'")
            .bind(i64::from(COVER_MAX_EDGE) + 1024)
            .execute(&pool)
            .await
            .unwrap();
        let rechecked = cap_cached_oversized(&pool, &dir, 16, true)
            .await
            .expect("recheck migration pass");
        assert_eq!(
            rechecked, 1,
            "rows checked at a higher threshold must be re-evaluated"
        );
        let updated_checked: i64 =
            sqlx::query_scalar("SELECT cap_checked FROM covers WHERE id = 'in-edge'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(updated_checked, i64::from(COVER_MAX_EDGE));

        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_file_dimensions_handles_png_jpeg_with_exif_webp_gif_and_non_raster() {
        let dir = std::env::temp_dir().join(format!(
            "musika-covers-probe-{}-{}",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let png_path = dir.join("sample.png");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            321,
            654,
            image::Rgb([1, 2, 3]),
        ))
        .save_with_format(&png_path, image::ImageFormat::Png)
        .unwrap();
        assert_eq!(probe_file_dimensions(&png_path).unwrap(), Some((321, 654)));

        // Inject a 60 KiB APP1 segment right after SOI to exercise seeking
        // past large EXIF/ICC metadata without reading it all.
        let mut raw_jpg = jpeg_bytes(777, 444, [4, 5, 6]);
        let mut padded_jpg = Vec::with_capacity(raw_jpg.len() + 60_004);
        padded_jpg.extend_from_slice(&raw_jpg[..2]);
        padded_jpg.extend_from_slice(&[0xFF, 0xE1]);
        let seg_len: u16 = 60_002;
        padded_jpg.extend_from_slice(&seg_len.to_be_bytes());
        padded_jpg.resize(padded_jpg.len() + 60_000, 0x42);
        padded_jpg.extend_from_slice(&raw_jpg[2..]);
        raw_jpg.clear();
        let jpg_path = dir.join("padded.jpg");
        std::fs::write(&jpg_path, &padded_jpg).unwrap();
        assert_eq!(probe_file_dimensions(&jpg_path).unwrap(), Some((777, 444)));

        let webp_path = dir.join("sample.webp");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            412,
            234,
            image::Rgb([7, 8, 9]),
        ))
        .save_with_format(&webp_path, image::ImageFormat::WebP)
        .unwrap();
        assert_eq!(probe_file_dimensions(&webp_path).unwrap(), Some((412, 234)));

        let gif_path = dir.join("sample.gif");
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            123,
            89,
            image::Rgb([10, 11, 12]),
        ))
        .save_with_format(&gif_path, image::ImageFormat::Gif)
        .unwrap();
        assert_eq!(probe_file_dimensions(&gif_path).unwrap(), Some((123, 89)));

        // Avoid a `.svg` literal in the filename so `scripts/check-icons.sh`
        // does not treat this test fixture as a missing UI icon asset.
        let svg_path = dir.join("sample.xml");
        std::fs::write(&svg_path, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        assert_eq!(probe_file_dimensions(&svg_path).unwrap(), None);

        let missing_path = dir.join("does-not-exist.jpg");
        assert!(probe_file_dimensions(&missing_path).is_err());

        _ = std::fs::remove_dir_all(&dir);
    }
}

/// Cache key for an artist image. Kept distinct from Subsonic `coverArt` ids
/// so an artist id cannot collide with an album cover row.
pub(crate) fn artist_image_key(artist_id: &str) -> String {
    format!("{ARTIST_IMAGE_PREFIX}{artist_id}")
}

fn artist_id_from_key(key: &str) -> Option<&str> {
    key.strip_prefix(ARTIST_IMAGE_PREFIX)
}

/// Maps artist ids to locally cached image URLs, fetching any missing image
/// in the background. The returned map contains only cached entries; callers
/// use the artist's `image_url` as the remote fallback.
pub async fn artist_image_urls(
    dbs: &crate::db::Dbs,
    artists: &[(String, String)],
) -> Result<HashMap<String, String>> {
    let entries: Vec<(String, String)> = artists
        .iter()
        .filter(|(id, url)| !id.is_empty() && !url.is_empty())
        .map(|(id, url)| (artist_image_key(id), url.clone()))
        .collect();
    if entries.is_empty() {
        return Ok(HashMap::new());
    }
    let keys: Vec<String> = entries.iter().map(|(key, _)| key.clone()).collect();
    let cached = cached_urls(&dbs.covers, &keys).await?;
    let mut map = HashMap::new();
    let mut missing = Vec::new();
    let mut claims: Vec<Claim> = Vec::new();
    for (key, url) in entries {
        if let Some(local) = cached.get(&key) {
            if let Some(artist_id) = artist_id_from_key(&key) {
                map.insert(artist_id.to_string(), local.clone());
            }
        } else if let Some(claim) = claim_fetch(&key) {
            claims.push(claim);
            missing.push((key, url));
        }
    }
    if missing.is_empty() {
        return Ok(map);
    }
    let Ok(client) = crate::session::client(dbs).await else {
        return Ok(map);
    };
    let pooled = dbs.covers.clone();
    let client = Arc::new(client);
    crate::app::runtime().spawn(async move {
        // The on-demand path fetches a subset: never sweep the directory from
        // here, another pass may be writing into it (N-04).
        let result = refresh_external_inner(&pooled, &client, &missing, false).await;
        drop(claims);
        if let Err(err) = result {
            eprintln!("covers: artist-image fetch failed: {err:#}");
        }
    });
    Ok(map)
}

/// Background pass over every artist image URL after a successful sync.
pub(crate) async fn refresh_artist_images(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    artists: &[(String, String)],
) -> Result<CoverReport> {
    let items: Vec<(String, String)> = artists
        .iter()
        .filter(|(id, url)| !id.is_empty() && !url.is_empty())
        .map(|(id, url)| (artist_image_key(id), url.clone()))
        .collect();
    // This follow-up runs inside the full cover pass, after a successful
    // library sync, so it may also sweep the cache directory (N-04).
    refresh_external_inner(pool, client, &items, true).await
}

async fn refresh_external_inner(
    pool: &SqlitePool,
    client: &Arc<SubsonicClient>,
    items: &[(String, String)],
    sweep: bool,
) -> Result<CoverReport> {
    let dir = crate::db::covers_dir()?;
    crate::db::ensure_dir(&dir)?;
    let existing: Vec<CoverRow> =
        sqlx::query_as("SELECT id, file, hash, etag, last_modified FROM covers")
            .fetch_all(pool)
            .await?;
    let known: Arc<HashMap<String, CoverRow>> = Arc::new(
        existing
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect(),
    );
    let known_for_apply = Arc::clone(&known);
    let mut report = CoverReport::default();
    let mut thumb_jobs = Vec::new();
    for chunk in items.chunks(crate::consts::FETCH_CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for (key, url) in chunk {
            let key = key.clone();
            let url = url.clone();
            let client = Arc::clone(client);
            let known = Arc::clone(&known);
            set.spawn(async move {
                let cached = known.get(&key);
                let etag = cached.and_then(|row| row.etag.clone());
                let last_modified = cached.and_then(|row| row.last_modified.clone());
                let Ok(url) = reqwest::Url::parse(&url) else {
                    return (key, Err(anyhow::anyhow!("invalid artist image URL: {url}")));
                };
                let result = client
                    .get_external_image(url, etag.as_deref(), last_modified.as_deref())
                    .await;
                (key, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((key, result)) => {
                    apply_fetched(
                        pool,
                        &dir,
                        known_for_apply.as_ref(),
                        &key,
                        result,
                        &mut report,
                        &mut thumb_jobs,
                    )
                    .await?;
                }
                Err(err) => eprintln!("covers: artist-image task failed: {err}"),
            }
        }
    }
    _ = ensure_thumbs(&dir, thumb_jobs).await;
    if sweep {
        remove_orphans(pool, &dir).await?;
    }
    Ok(report)
}
