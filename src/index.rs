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

//! Library sync orchestration.
//!
//! Fetches the library from the Subsonic API and fans it out to the three
//! content databases:
//!
//!   library.db  ← metadata (artists/albums/songs)
//!   search.db   ← FTS5 rebuild, right after the library write
//!   covers.db   ← cover cache, run as a secondary task after the sync returns
//!
//! The sync is incremental at the top level: `getIndexes` is asked for changes
//! since the last `lastModified`, and an empty answer is a no-op.

use std::sync::Arc;

use anyhow::{Context, Result};

use crate::api::{AlbumID3, AlbumWithSongsID3, ArtistID3, ArtistWithAlbumsID3, Child};
use crate::covers::CoverReport;
use crate::db::Dbs;
use crate::http_client::SubsonicClient;

/// Outcome of a [`sync_library`] run.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub struct SyncReport {
    /// `false` when the server reported no changes and nothing was fetched.
    pub updated: bool,
    pub artists: usize,
    pub albums: usize,
    pub songs: usize,
    /// The caller starts the asynchronous cover pass after the sync returns.
    pub covers_due: bool,
    /// Covers whose album art changed, forced past the conditional request.
    pub forced_covers: Vec<String>,
    /// Lyrics fetched by the bounded sync backfill.
    pub lyrics: usize,
    /// ARC-44: server playlists refreshed by this run.
    pub playlists: usize,
}

/// Coarse progress update emitted while a sync runs.
#[derive(Debug, Clone)]
pub struct SyncProgress {
    pub message: String,
    pub done: usize,
    pub total: usize,
}

fn report(progress: &mut impl FnMut(SyncProgress), message: impl Into<String>, done: usize) {
    progress(SyncProgress {
        message: message.into(),
        done,
        total: 100,
    });
}

/// `getIndexes.lastModified` is milliseconds on Subsonic/Navidrome, but the
/// cache `fetched_at` columns are seconds. Normalise before comparing.
pub(crate) fn provider_secs(ts: i64) -> i64 {
    if ts > 1_000_000_000_000 {
        ts / 1000
    } else {
        ts
    }
}

async fn best_effort<T>(
    label: &str,
    future: impl std::future::Future<Output = Result<T>>,
) -> Option<T> {
    match future.await {
        Ok(value) => Some(value),
        Err(err) => {
            eprintln!("index: {label}: {err:#}");
            None
        }
    }
}

/// Fetches the library and refreshes the local databases.
///
/// `progress` is called at each stage so the UI can display a status/progress.
pub async fn sync_library<F>(
    dbs: &Dbs,
    client: Arc<SubsonicClient>,
    mut progress: F,
) -> Result<SyncReport>
where
    F: FnMut(SyncProgress) + Send,
{
    report(&mut progress, "Checking for changes…", 0);

    let last_modified = crate::library::last_modified(&dbs.library).await?;

    let indexes = client
        .get_indexes(None, (last_modified > 0).then_some(last_modified))
        .await
        .context("failed to fetch the library index")?;

    if indexes.index.is_empty() && indexes.shortcut.is_empty() {
        crate::library::touch_last_sync(&dbs.library).await?;
        report(&mut progress, "Up to date", 100);
        // S-9: the search schema migration recreates the FTS table, so an
        // unchanged library can still have an empty index. Rebuild it here
        // rather than waiting for the next library change.
        if crate::search::count(&dbs.search).await.unwrap_or(0) == 0 {
            if let Err(err) = rebuild_search(dbs).await {
                eprintln!("search: could not rebuild the index: {err:#}");
            }
        }
        // The library is unchanged, but the derived caches may still need
        // topping up (first run interrupted, or entries that previously
        // failed): the caller starts the cover pass asynchronously after this
        // returns; the bounded lyric backfill stays in the sync.
        let covers_due =
            crate::library::cover_check_due(&dbs.library, crate::consts::COVER_CHECK_TTL_SECS)
                .await
                .unwrap_or(true);
        let lyrics = refresh_lyrics(dbs, &client, &mut progress).await;
        return Ok(SyncReport {
            updated: false,
            covers_due,
            lyrics,
            ..SyncReport::default()
        });
    }

    let mut artist_ids: Vec<String> = indexes
        .index
        .iter()
        .flat_map(|index| index.artist.iter())
        .chain(indexes.shortcut.iter())
        .map(|artist| artist.id.clone())
        .collect();
    artist_ids.sort();
    artist_ids.dedup();

    report(
        &mut progress,
        format!("Fetching {} artists…", artist_ids.len()),
        10,
    );
    // ARC-25: abort before any destructive step when a fetch was skipped.
    let fetched_artists = fetch_artists(&client, &artist_ids).await?;

    let mut album_ids: Vec<String> = fetched_artists
        .iter()
        .flat_map(|artist| artist.album.iter())
        .map(|album| album.id.clone())
        .collect();
    album_ids.sort();
    album_ids.dedup();

    // MEMORY: take the artist rows now, before fetching albums. The artist
    // entries carry their album metadata; keeping that Vec alive alongside the
    // album entries (which repeat it) and the flattened `albums` copy held the
    // same rows in memory up to three times during a sync.
    let artists: Vec<ArtistID3> = fetched_artists
        .into_iter()
        .map(|entry| entry.artist)
        .collect();

    report(
        &mut progress,
        format!("Fetching {} albums…", album_ids.len()),
        40,
    );
    let fetched_albums = fetch_albums(&client, &album_ids).await?;

    // MEMORY: consume the album entries instead of cloning their album and
    // song vectors into a second full copy of the catalog.
    let mut albums: Vec<AlbumID3> = Vec::with_capacity(fetched_albums.len());
    let mut songs: Vec<Child> = Vec::new();
    for entry in fetched_albums {
        albums.push(entry.album);
        songs.extend(entry.song);
    }

    // 1. library metadata
    report(&mut progress, "Indexing library…", 65);
    let changes = crate::library::replace(
        &dbs.library,
        &artists,
        &albums,
        &songs,
        indexes.last_modified,
    )
    .await?;

    // The provider date moved: any description or lyric document cached
    // before that update is older than the provider's copy. Drop it so the
    // next read fetches the current text instead of serving the stale cache.
    let provider_secs = provider_secs(indexes.last_modified);
    if provider_secs > 0 {
        best_effort(
            "album info invalidation failed",
            crate::library::invalidate_album_info_before(&dbs.library, provider_secs),
        )
        .await;
        best_effort(
            "artist info invalidation failed",
            crate::library::invalidate_artist_info_before(&dbs.library, provider_secs),
        )
        .await;
        best_effort(
            "lyric invalidation failed",
            crate::lyrics::invalidate_before(&dbs.lyrics, provider_secs),
        )
        .await;
    }

    // The API also changed rows that may carry a cache fetched after the
    // provider date: drop those too, so changed rows always refetch.
    if !changes.albums.is_empty() {
        best_effort(
            "album info invalidation failed",
            crate::library::invalidate_album_info(&dbs.library, &changes.albums),
        )
        .await;
        best_effort("lyric invalidation failed", async {
            let grouped = crate::library::album_song_ids_of(&dbs.library, &changes.albums).await?;
            let ids: Vec<String> = grouped.into_values().flatten().collect();
            crate::lyrics::invalidate_songs(&dbs.lyrics, &ids).await
        })
        .await;
    }
    if !changes.songs.is_empty() {
        best_effort(
            "lyric invalidation failed",
            crate::lyrics::invalidate_songs(&dbs.lyrics, &changes.songs),
        )
        .await;
    }
    if !changes.artists.is_empty() {
        best_effort(
            "artist info invalidation failed",
            crate::library::invalidate_artist_info(&dbs.library, &changes.artists),
        )
        .await;
    }

    // 1b. UI-60: the server's artist `userRating` snapshots seed the local
    // `artist_user` overlay (a rating made elsewhere lands here).
    let artist_ratings: Vec<(String, i64)> = indexes
        .index
        .iter()
        .flat_map(|index| index.artist.iter())
        .chain(indexes.shortcut.iter())
        .filter_map(|artist| {
            artist
                .user_rating
                .map(|rating| (artist.id.clone(), i64::from(rating)))
        })
        .collect();
    best_effort(
        "artist rating import failed",
        crate::artist_user::import_ratings(&dbs.artist_user, &artist_ratings),
    )
    .await;

    // 2. search index
    rebuild_search(dbs).await?;

    // 3. plain-text mirror for external tools (best effort)
    best_effort(
        "could not write the search mirror",
        crate::library::write_mirror(&dbs.library),
    )
    .await;

    // 4. the cover cache is no longer part of the sync proper: it is a
    // secondary task the caller starts once this library pass has succeeded.

    // 5. lyric cache: drop entries for songs that left the library
    best_effort("lyric cache purge failed", crate::lyrics::purge(dbs)).await;

    // 5b. artist biography + album notes caches: drop rows that left the
    // library
    best_effort(
        "artist info purge failed",
        crate::library::purge_stale_artist_info(&dbs.library),
    )
    .await;
    best_effort(
        "album info purge failed",
        crate::library::purge_stale_album_info(&dbs.library),
    )
    .await;

    // 5c. media cache: drop blobs for vanished songs and dead rules
    best_effort("media cache purge failed", crate::media::purge(dbs)).await;

    // 5c'. media cache: rule-owned blobs no longer inside a selected
    // artist/album container (CACHE-5 coverage check)
    best_effort(
        "media coverage prune failed",
        crate::media::prune_uncovered(dbs),
    )
    .await;

    // 5d. download ledger: drop rows for songs that left the library
    best_effort("download ledger purge failed", async {
        crate::cache::purge_missing_against(
            &dbs.download,
            "download",
            "song_id",
            &dbs.library,
            "songs",
            "id",
        )
        .await?;
        Ok(())
    })
    .await;

    // 6. bounded lyric backfill (the sync button's lyric pass)
    let lyrics = refresh_lyrics(dbs, &client, &mut progress).await;

    // 7. color cache: drop schemes whose cover vanished from covers.db
    best_effort("color cache purge failed", crate::colors::purge(dbs)).await;

    // 8. ARC-44: the Subsonic playlist list rides the library sync; the
    // contents stay lazy, and a failure here never fails the sync.
    let playlists = best_effort(
        "could not refresh playlists",
        crate::playlists::refresh_subsonic(dbs, &client),
    )
    .await
    .unwrap_or(0);

    report(&mut progress, "Done", 100);

    Ok(SyncReport {
        updated: true,
        artists: artists.len(),
        albums: albums.len(),
        songs: songs.len(),
        covers_due: true,
        forced_covers: changes.covers,
        lyrics,
        playlists,
    })
}

/// Rebuilds the derived search caches from the current library rows: the
/// FTS5 mirror (kept as the fallback engine) and, when ARC-SEARCH is opted
/// in, the in-memory Nucleo catalog.
async fn rebuild_search(dbs: &Dbs) -> Result<()> {
    // Paged rebuild: the old path collected every search row into a Vec
    // before writing the FTS table (FIX-26).
    crate::search::rebuild_from_library(&dbs.search).await?;
    if crate::search::index::enabled()
        && let Err(err) = crate::search::index::rebuild(&dbs.library).await
    {
        eprintln!("search: could not rebuild the in-memory index: {err:#}");
    }
    Ok(())
}

/// Sync-time lyric pass: fetches songs without lyrics or past their miss TTL,
/// capped by [`crate::lyrics::SYNC_BACKFILL_LIMIT`]. Best effort.
async fn refresh_lyrics<F>(dbs: &Dbs, client: &Arc<SubsonicClient>, progress: &mut F) -> usize
where
    F: FnMut(SyncProgress),
{
    report(progress, "Fetching lyrics…", 85);
    best_effort(
        "lyric backfill failed",
        crate::lyrics::backfill(dbs, client, crate::lyrics::SYNC_BACKFILL_LIMIT),
    )
    .await
    .map_or(0, |report| report.fetched)
}

/// Revalidates/downloads referenced covers and purges the rest.
///
/// Runs as the sync's secondary, asynchronous task: failures are logged and
/// swallowed, because the library and search databases are already
/// consistent and a cover cache problem is not fatal.
/// FIX-7: one cover pass may run at a time — the sync returns before its
/// detached cover task, so a back-to-back sync would otherwise start a
/// second pass while the first is still revalidating/rewriting rows. An
/// overlapping pass just waits for the lock (the next TTL/sync would repeat
/// the check anyway).
static COVER_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn cache_covers(
    dbs: &Dbs,
    client: &Arc<SubsonicClient>,
    forced: &[String],
) -> CoverReport {
    let pass = COVER_PASS.lock().await;
    let result = cache_covers_locked(dbs, client, forced).await;
    drop(pass);
    result
}

async fn cache_covers_locked(
    dbs: &Dbs,
    client: &Arc<SubsonicClient>,
    forced: &[String],
) -> CoverReport {
    let referenced = match crate::library::referenced_cover_ids(&dbs.library).await {
        Ok(referenced) => referenced,
        Err(err) => {
            eprintln!("index: could not determine referenced covers: {err:#}");
            return CoverReport::default();
        }
    };

    match crate::covers::refresh(&dbs.covers, client, &referenced, forced).await {
        Ok(report) => {
            let _ = crate::library::touch_cover_check(&dbs.library).await;
            report
        }
        Err(err) => {
            eprintln!("index: cover sync failed: {err:#}");
            CoverReport::default()
        }
    }
}

/// Fails the sync when any fetch was skipped, so the destructive phase below
/// (wholesale replace + cover/lyric/color purges) never runs on a partial
/// library (ARC-25).
pub(crate) fn ensure_complete(failed: &[String], total: usize) -> Result<()> {
    if failed.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "{} of {} fetches failed (first: {}); nothing was changed",
        failed.len(),
        total,
        failed[0]
    );
}

async fn fetch_artists(
    client: &Arc<SubsonicClient>,
    ids: &[String],
) -> Result<Vec<ArtistWithAlbumsID3>> {
    let client = Arc::clone(client);
    let results = crate::fanout::map_bounded(ids.to_vec(), move |id| {
        let client = Arc::clone(&client);
        async move { client.get_artist(&id).await }
    })
    .await;

    let mut out = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (id, result) in results {
        match result {
            Ok(artist) => out.push(artist),
            Err(err) => {
                eprintln!("index: failed to fetch artist {id}: {err:#}");
                failed.push(id);
            }
        }
    }
    ensure_complete(&failed, ids.len())?;
    Ok(out)
}

async fn fetch_albums(
    client: &Arc<SubsonicClient>,
    ids: &[String],
) -> Result<Vec<AlbumWithSongsID3>> {
    let client = Arc::clone(client);
    let results = crate::fanout::map_bounded(ids.to_vec(), move |id| {
        let client = Arc::clone(&client);
        async move { client.get_album(&id).await }
    })
    .await;

    let mut out = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (id, result) in results {
        match result {
            Ok(album) => out.push(album),
            Err(err) => {
                eprintln!("index: failed to fetch album {id}: {err:#}");
                failed.push(id);
            }
        }
    }
    ensure_complete(&failed, ids.len())?;
    Ok(out)
}
