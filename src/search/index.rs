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

//! ARC-SEARCH: the in-process Nucleo catalog index.
//!
//! Songs are matched by a `nucleo` worker (fuzzy, smartcase, Unicode
//! normalization, incremental on query extensions); artists and albums are
//! small enough to score directly with the same matcher. The index is fed
//! from `library.db`, rebuilt after a sync, and is a disposable cache: the
//! FTS5 tables stay in place as the fallback engine.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::Result;
use futures_util::TryStreamExt;
use nucleo::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Nucleo, Utf32Str, Utf32String};
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::library::{AlbumRow, ArtistRow};

/// How many of the best text matches are re-scored for the play-count boost.
/// Nucleo orders by text score; the boost only needs to reorder near-ties,
/// and this keeps the re-sort bounded on a very common query.
const MATCH_SCAN: usize = 512;

/// One Nucleo config for the index and the standalone matchers. Prefix
/// preference makes the first typed character compete on word starts
/// instead of being treated as an arbitrary fuzzy scatter.
fn matcher_config() -> Config {
    let mut config = Config::DEFAULT;
    config.prefer_prefix = true;
    config
}
/// S-9 usage boost: score units added per play (capped).
const PLAY_BOOST: i64 = 2;
const PLAY_CAP: i64 = 500;

/// A song row as the index stores it. Everything the search payload needs is
/// carried here, so a typed query never has to join back to `library.db`
/// (only the cover URL map remains).
#[derive(Debug, Clone, FromRow)]
pub struct SongHit {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<i32>,
    pub play_count: i64,
    pub album_id: Option<String>,
    pub cover_art: Option<String>,
    pub track: Option<i32>,
    pub disc_number: Option<i32>,
}

impl SongHit {
    /// The single fuzzy haystack for this song: title, artist, album. Title
    /// first so its word-boundary bonuses lead the ranking.
    fn haystack(&self) -> String {
        let mut text = String::with_capacity(
            self.title.len()
                + self.artist.as_deref().map_or(0, str::len)
                + self.album.as_deref().map_or(0, str::len)
                + 2,
        );
        text.push_str(&self.title);
        if let Some(artist) = self.artist.as_deref().filter(|value| !value.is_empty()) {
            text.push(' ');
            text.push_str(artist);
        }
        if let Some(album) = self.album.as_deref().filter(|value| !value.is_empty()) {
            text.push(' ');
            text.push_str(album);
        }
        text
    }
}

/// An artist/album row plus the pre-built haystack, so a search never
/// allocates one per item per keystroke.
struct Indexed<T> {
    row: Arc<T>,
    haystack: String,
}

struct SongIndex {
    nucleo: Nucleo<Arc<SongHit>>,
    last_query: String,
}

/// The catalog snapshot one search request clones.
pub struct SearchResults {
    pub songs: Vec<Arc<SongHit>>,
    pub artists: Vec<Arc<ArtistRow>>,
    pub albums: Vec<Arc<AlbumRow>>,
}

/// The in-memory catalog. Rebuilt as a whole after a library sync; search
/// requests clone the `Arc` and never see a half-built index.
pub struct SearchIndex {
    songs: std::sync::Mutex<SongIndex>,
    artists: Vec<Indexed<ArtistRow>>,
    albums: Vec<Indexed<AlbumRow>>,
    most_played: Vec<Arc<SongHit>>,
    by_album: HashMap<String, Vec<Arc<SongHit>>>,
}

impl SearchIndex {
    /// Builds from already-shared hits. The sync path streams rows straight
    /// into `Arc<SongHit>`s, so it never allocates the intermediate
    /// `Vec<SongHit>` buffer (≈17 MB at 100k tracks) on top of the catalog.
    pub(crate) fn from_shared(
        songs: Vec<Arc<SongHit>>,
        artists: Vec<ArtistRow>,
        albums: Vec<AlbumRow>,
    ) -> Self {
        let nucleo = Nucleo::new(matcher_config(), Arc::new(|| {}), Some(4), 1);
        {
            let injector = nucleo.injector();
            for song in &songs {
                injector.push(Arc::clone(song), |song, columns| {
                    columns[0] = Utf32String::from(song.haystack());
                });
            }
        }

        let mut most_played = songs.clone();
        most_played.sort_by(|a, b| {
            b.play_count
                .cmp(&a.play_count)
                .then_with(|| a.title.cmp(&b.title))
        });

        let mut by_album: HashMap<String, Vec<Arc<SongHit>>> = HashMap::new();
        for song in &songs {
            if let Some(album_id) = &song.album_id {
                by_album
                    .entry(album_id.clone())
                    .or_default()
                    .push(Arc::clone(song));
            }
        }
        for tracks in by_album.values_mut() {
            tracks.sort_by(|a, b| {
                a.disc_number
                    .unwrap_or(0)
                    .cmp(&b.disc_number.unwrap_or(0))
                    .then_with(|| a.track.unwrap_or(0).cmp(&b.track.unwrap_or(0)))
                    .then_with(|| a.title.cmp(&b.title))
            });
        }

        let artists = artists
            .into_iter()
            .map(|row| {
                let haystack = row.name.clone();
                Indexed {
                    row: Arc::new(row),
                    haystack,
                }
            })
            .collect();
        let albums = albums
            .into_iter()
            .map(|row| {
                let haystack = match row.artist.as_deref().filter(|value| !value.is_empty()) {
                    Some(artist) => format!("{} {}", row.title, artist),
                    None => row.title.clone(),
                };
                Indexed {
                    row: Arc::new(row),
                    haystack,
                }
            })
            .collect();

        Self {
            songs: std::sync::Mutex::new(SongIndex {
                nucleo,
                last_query: String::new(),
            }),
            artists,
            albums,
            most_played,
            by_album,
        }
    }

    /// Fuzzy-search the catalog. `raw` is already punctuation-cleaned by the
    /// caller; an empty query returns no typed hits (the caller paints the
    /// Most-played surface instead).
    pub fn search(&self, raw: &str, limit: usize) -> SearchResults {
        // Keep direct callers safe from Nucleo pattern syntax: punctuation is
        // a separator, so unmatched quotes/operators degrade to a literal
        // fuzzy query instead of silently matching nothing.
        let raw = crate::search::clean_query(raw);
        let pattern = Pattern::new(
            &raw,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        if pattern.atoms.is_empty() || limit == 0 {
            return SearchResults {
                songs: Vec::new(),
                artists: Vec::new(),
                albums: Vec::new(),
            };
        }

        let songs = self.search_songs(&pattern, &raw, limit);
        let mut matcher = Matcher::new(matcher_config());
        let artists = match_indexed(&pattern, &self.artists, &mut matcher, limit);
        let albums = match_indexed(&pattern, &self.albums, &mut matcher, limit);
        SearchResults {
            songs,
            artists,
            albums,
        }
    }

    fn search_songs(&self, pattern: &Pattern, raw: &str, limit: usize) -> Vec<Arc<SongHit>> {
        let mut guard = self.songs.lock().unwrap_or_else(|error| error.into_inner());
        // Do not reuse the previous ordering when the query grows: an item
        // that was already on screen must be allowed to move up as the new
        // query refines the score. Reparse from scratch each keystroke.
        guard
            .nucleo
            .pattern
            .reparse(0, raw, CaseMatching::Smart, Normalization::Smart, false);
        guard.last_query.clear();
        guard.last_query.push_str(raw);
        loop {
            let status = guard.nucleo.tick(10);
            if !status.running {
                break;
            }
        }

        let snapshot = guard.nucleo.snapshot();
        let matched = snapshot.matched_item_count() as usize;
        if matched == 0 {
            return Vec::new();
        }
        // Re-score only the best text matches; the play boost (S-9) then
        // reorders that short list without sorting the whole snapshot.
        let scan = matched.min(MATCH_SCAN.max(limit.saturating_mul(8)));
        let mut matcher = Matcher::new(matcher_config());
        let mut scored: Vec<(i64, u32, Arc<SongHit>)> = Vec::with_capacity(scan);
        for item in snapshot.matched_items(0..scan as u32) {
            let Some(text_score) = pattern.score(item.matcher_columns[0].slice(..), &mut matcher)
            else {
                continue;
            };
            let plays = item.data.play_count.clamp(0, PLAY_CAP);
            scored.push((
                i64::from(text_score) + plays * PLAY_BOOST,
                text_score,
                Arc::clone(item.data),
            ));
        }
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.title.cmp(&b.2.title))
        });
        scored.truncate(limit);
        scored.into_iter().map(|(_, _, song)| song).collect()
    }

    /// The pre-sorted empty-query surface: no scan, no database read.
    pub fn most_played(&self, limit: usize) -> Vec<Arc<SongHit>> {
        self.most_played.iter().take(limit).cloned().collect()
    }

    /// S-8: an album's tracks in disc/track order.
    pub fn album_tracks(&self, album_id: &str, limit: usize) -> Vec<Arc<SongHit>> {
        self.by_album
            .get(album_id)
            .map(|tracks| tracks.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }
}

fn match_indexed<T>(
    pattern: &Pattern,
    items: &[Indexed<T>],
    matcher: &mut Matcher,
    limit: usize,
) -> Vec<Arc<T>> {
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, &Indexed<T>)> = Vec::new();
    for item in items {
        let haystack = Utf32Str::new(&item.haystack, &mut buf);
        if let Some(score) = pattern.score(haystack, matcher) {
            scored.push((score, item));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.haystack.cmp(&b.1.haystack)));
    scored.truncate(limit);
    scored
        .into_iter()
        .map(|(_, item)| Arc::clone(&item.row))
        .collect()
}

// ---------------------------------------------------------------------------
// Process-wide index
// ---------------------------------------------------------------------------

fn slot() -> &'static RwLock<Option<Arc<SearchIndex>>> {
    static SLOT: std::sync::OnceLock<RwLock<Option<Arc<SearchIndex>>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Bumped by every clear; a build that started before a wipe must not write
/// its catalog back into the fresh slot.
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Set when the library is too large for the in-memory catalog's budget. The
/// FTS5 tables stay authoritative and `ensure` returns `None`, so a huge
/// library trades fuzzy matching for a bounded resident set instead of
/// growing without limit. Re-evaluated on every rebuild (a wipe or a smaller
/// library lifts it).
static OVER_BUDGET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Default catalog budget (MiB). The estimate below is ~0.4 KiB per song, so
/// 64 MiB keeps roughly 170k ASCII-heavy tracks fuzzy-matchable;
/// `MUSIKA_SEARCH_INDEX_BUDGET_MIB=0` disables the cap for very large
/// libraries that prefer the catalog.
const DEFAULT_BUDGET_MIB: usize = 64;

/// Fixed per-song cost of the catalog: `ArcInner<SongHit>` (192 B), the
/// boxcar entry holding the `Arc` plus its `Utf32String` column (~32 B) and
/// the two secondary index slots (~16 B), with a little slack. Calibrated
/// against `bench_100k_songs` (256 + 3×text lands within ~10% of the
/// measured retained bytes).
pub(crate) const SONG_FIXED_COST: usize = 256;
/// Text factor: every byte is stored once in the `SongHit` `String` and once
/// more in Nucleo's haystack — 1 byte/char for ASCII, up to 4 bytes for
/// non-ASCII (UTF-32). Three is the conservative middle, so a non-Latin
/// library does not silently overshoot the budget by much.
pub(crate) const HAYSTACK_FACTOR: usize = 3;

/// The in-memory catalog is built for the hybrid and Nucleo engines;
/// `MUSIKA_SEARCH_ENGINE=fts` skips it. `search_json` falls back to FTS
/// whenever the in-memory build or query fails.
pub fn enabled() -> bool {
    matches!(
        crate::search::engine(),
        crate::search::Engine::Hybrid | crate::search::Engine::Nucleo
    )
}

/// Whether the last build declined the catalog for its memory budget. The
/// search path skips the attempt entirely while this holds; the next
/// [`rebuild`]/[`clear`] re-evaluates.
pub fn over_budget() -> bool {
    OVER_BUDGET.load(Ordering::Relaxed)
}

/// The catalog budget in bytes (`None` = unlimited).
fn budget_bytes() -> Option<usize> {
    let mib = std::env::var("MUSIKA_SEARCH_INDEX_BUDGET_MIB")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_BUDGET_MIB);
    (mib > 0).then_some(mib.saturating_mul(1024 * 1024))
}

/// Estimates the catalog's resident cost from `library.db` without loading
/// any row into Rust: one aggregate scan, then the fixed and text terms.
pub(crate) async fn catalog_cost(pool: &SqlitePool) -> Result<usize> {
    let (songs, text_bytes): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),
                COALESCE(SUM(
                    length(CAST(s.id AS BLOB))
                    + length(CAST(s.title AS BLOB))
                    + COALESCE(length(CAST(s.artist AS BLOB)), 0)
                    + COALESCE(length(CAST(s.album AS BLOB)), 0)
                    + COALESCE(length(CAST(s.album_id AS BLOB)), 0)
                    + COALESCE(length(CAST(COALESCE(al.cover_art, s.cover_art) AS BLOB)), 0)
                ), 0)
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id",
    )
    .fetch_one(pool)
    .await?;
    let songs = usize::try_from(songs).unwrap_or(usize::MAX);
    let text = usize::try_from(text_bytes).unwrap_or(usize::MAX);
    Ok(songs
        .saturating_mul(SONG_FIXED_COST)
        .saturating_add(text.saturating_mul(HAYSTACK_FACTOR)))
}

/// Drops the in-memory catalog (wipe/logout). The next search rebuilds it.
pub fn clear() {
    EPOCH.fetch_add(1, Ordering::Relaxed);
    OVER_BUDGET.store(false, Ordering::Relaxed);
    *slot().write().unwrap_or_else(|error| error.into_inner()) = None;
}

/// FIX-6: serialises the catalog build — concurrent first searches (or a
/// first search racing a post-sync rebuild) wait for the running build
/// instead of reading the library and building a second catalog in parallel.
static BUILD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The current index, built lazily from `library.db` on first use. `None`
/// means the library exceeded the catalog budget: the caller must use FTS.
/// FIX-6: concurrent builds wait for the in-flight one (the epoch guard
/// already handles wipe races).
pub async fn ensure(pool: &SqlitePool) -> Result<Option<Arc<SearchIndex>>> {
    if OVER_BUDGET.load(Ordering::Relaxed) {
        return Ok(None);
    }
    if let Some(index) = slot()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        return Ok(Some(index));
    }
    let build = BUILD.lock().await;
    // Double-check: the wait may already have paid for the build.
    if let Some(index) = slot()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        return Ok(Some(index));
    }
    if OVER_BUDGET.load(Ordering::Relaxed) {
        return Ok(None);
    }
    rebuild_locked(pool).await?;
    drop(build);
    Ok(slot()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone())
}

/// Rebuilds the whole catalog from `library.db` (after a sync or on first
/// use). Serialized with [`ensure`] through [`BUILD`]. The Nucleo injection
/// runs on a blocking worker.
pub async fn rebuild(pool: &SqlitePool) -> Result<()> {
    let build = BUILD.lock().await;
    let result = rebuild_locked(pool).await;
    drop(build);
    result
}

async fn rebuild_locked(pool: &SqlitePool) -> Result<()> {
    let epoch = EPOCH.load(Ordering::Relaxed);
    // A previous over-budget verdict must not stick forever: every rebuild
    // re-measures (the library may have shrunk, or the budget changed).
    OVER_BUDGET.store(false, Ordering::Relaxed);
    if let Some(budget) = budget_bytes() {
        let estimate = catalog_cost(pool).await?;
        if estimate > budget {
            OVER_BUDGET.store(true, Ordering::Relaxed);
            // Drop any older catalog: the point of the budget is a smaller
            // resident set, not keeping the previous generation alive.
            *slot().write().unwrap_or_else(|error| error.into_inner()) = None;
            eprintln!(
                "search: catalog estimate {} MiB exceeds the {} MiB budget; \
                 FTS answers instead (MUSIKA_SEARCH_INDEX_BUDGET_MIB=0 disables the cap)",
                estimate / (1024 * 1024),
                budget / (1024 * 1024),
            );
            return Ok(());
        }
    }

    // MEMORY: wrap each row in its `Arc` as it arrives instead of collecting
    // into a `Vec<SongHit>` and converting afterwards; `from_shared` then
    // injects the same Arcs, so the build never holds both forms.
    let mut songs: Vec<Arc<SongHit>> = Vec::new();
    {
        let mut rows = sqlx::query_as::<_, SongHit>(
            "SELECT s.id, s.title, s.artist, s.album, s.duration,
                    COALESCE(s.play_count, 0) AS play_count,
                    s.album_id,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art,
                    s.track, s.disc_number
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id",
        )
        .fetch(pool);
        while let Some(song) = rows.try_next().await? {
            songs.push(Arc::new(song));
        }
    }
    let artists: Vec<ArtistRow> = sqlx::query_as(
        "SELECT id, name, album_count, cover_art, starred FROM artists
             WHERE EXISTS (SELECT 1 FROM albums WHERE albums.artist_id = artists.id)",
    )
    .fetch_all(pool)
    .await?;
    let albums: Vec<AlbumRow> = sqlx::query_as(
        "SELECT id, name AS title, artist, year, song_count, cover_art, starred, rating
         FROM albums",
    )
    .fetch_all(pool)
    .await?;

    let index =
        tokio::task::spawn_blocking(move || SearchIndex::from_shared(songs, artists, albums))
            .await?;
    if EPOCH.load(Ordering::Relaxed) != epoch {
        // A wipe landed while this build was running; drop the stale catalog.
        return Ok(());
    }
    *slot().write().unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(index));
    Ok(())
}
