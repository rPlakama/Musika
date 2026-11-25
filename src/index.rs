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
//! The top level always fetches the full artist/album manifest (Navidrome's
//! `ifModifiedSince` answers are not trustworthy for library moves); only the
//! albums whose manifest hash changed are re-fetched, and a periodic/manual
//! deep pass re-reads every album's songs.

use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::api::{AlbumID3, AlbumWithSongsID3, ArtistID3, ArtistWithAlbumsID3, Child};
use crate::covers::CoverReport;
use crate::db::Dbs;
use crate::http_client::SubsonicClient;

/// Outcome of a [`sync_library`] run.
#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    /// The caller starts the asynchronous cover pass after the sync returns.
    pub covers_due: bool,
    /// Whether this pass exercised the deep reconciler, either because the
    /// caller asked for it or because the periodic full-sync deadline or a
    /// low manifest overlap forced it. The cover pass follows this, not the
    /// requested [`crate::bridge::SyncMode`], so an automatic incremental pass
    /// that is internally deep still warms playlist/album art.
    pub deep: bool,
    /// Covers whose album art changed, forced past the conditional request.
    pub forced_covers: Vec<String>,
}

impl SyncReport {
    /// A deep pass that found covers worth revalidating: the caller should run
    /// the cover cache pass even when the request was `Incremental`.
    pub fn wants_cover_pass(&self) -> bool {
        self.deep && self.covers_due
    }
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
    deep: bool,
    mut progress: F,
) -> Result<SyncReport>
where
    F: FnMut(SyncProgress) + Send,
{
    report(&mut progress, "Checking for changes…", 0);
    let before = crate::library::counts(&dbs.library)
        .await
        .unwrap_or((0, 0, 0));

    // Always fetch the full artist manifest. `getIndexes(ifModifiedSince)` is
    // not reliable for library changes: Navidrome's master may answer with a
    // stale empty index (song moves never seen) or with only the changed
    // artists (a destructive full replace would then wipe the rest). The
    // manifest is cheap compared with songs and lets the reconciler below
    // decide what to fetch.
    let indexes = client
        .get_indexes(None, None)
        .await
        .context("failed to fetch the library index")?;

    let mut artist_ids: Vec<String> = indexes
        .index
        .iter()
        .flat_map(|index| index.artist.iter())
        .chain(indexes.shortcut.iter())
        .map(|artist| artist.id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    artist_ids.sort();
    artist_ids.dedup();

    report(
        &mut progress,
        format!("Fetching {} artists…", artist_ids.len()),
        10,
    );
    let fetched_artists = fetch_artists(&client, &artist_ids).await?;
    let mut artists: Vec<ArtistID3> = Vec::with_capacity(fetched_artists.len());
    let mut albums: Vec<AlbumID3> = Vec::new();
    for entry in fetched_artists {
        if entry.artist.id.is_empty() {
            continue;
        }
        artists.push(entry.artist);
        for album in entry.album {
            if !album.id.is_empty() {
                albums.push(album);
            }
        }
    }
    // An album can be listed by more than one artist; keep one deterministic row.
    let mut seen_albums: std::collections::HashSet<String> = std::collections::HashSet::new();
    albums.retain(|album| seen_albums.insert(album.id.clone()));

    // Safety: an empty/partial manifest is far more likely a broken upstream
    // answer than a library the user emptied. Never wipe a populated local
    // library from a suspicious shallow manifest, even when `full_sync_due` or
    // low manifest overlap would escalate `deep`; only an explicit manual
    // refresh (`deep == true` from the caller) overrides this guard.
    let manual_deep = deep;
    let old_artists = before.2.max(0) as usize;
    if old_artists > 0 && !manual_deep {
        if artists.is_empty() {
            bail!("server returned an empty artist index; keeping the local library");
        }
        if artists.len() * 2 < old_artists {
            bail!(
                "server returned only {}/{} artists; keeping the local library (manual refresh to force)",
                artists.len(),
                old_artists
            );
        }
    }

    // Shallow passes re-fetch only albums whose stored hash changed (a song
    // move changes the album's count/duration/cover); a deep pass re-fetches
    // every album's songs so silent metadata edits are caught too.
    let mut deep = deep || crate::library::full_sync_due(&dbs.library).await;
    let manifest = crate::library::album_manifest(&dbs.library).await?;
    // A server rescan/re-key (moved files) can replace most album ids at once.
    // A shallow pass would then append the new rows next to the dead ones, so
    // treat low manifest overlap as a full rescan and run the deep reconciler.
    if !deep && !manifest.is_empty() {
        let overlap = albums
            .iter()
            .filter(|album| manifest.contains_key(&album.id))
            .count();
        if overlap * 2 < manifest.len() {
            deep = true;
        }
    }
    let mut fetch_ids: Vec<String> = Vec::new();
    for album in &albums {
        let changed = manifest
            .get(&album.id)
            .is_none_or(|seen| seen.hash != crate::library::album_hash(album));
        if deep || changed {
            fetch_ids.push(album.id.clone());
        }
    }

    report(
        &mut progress,
        format!("Fetching {} albums…", fetch_ids.len()),
        45,
    );
    let fetched_albums = fetch_albums(&client, &fetch_ids).await?;
    let mut songs_by_album: std::collections::HashMap<String, Vec<Child>> =
        std::collections::HashMap::new();
    for entry in fetched_albums {
        let album_id = entry.album.id.clone();
        if album_id.is_empty() {
            continue;
        }
        let songs: Vec<Child> = entry
            .song
            .into_iter()
            .filter(|song| !song.id.is_empty())
            .collect();
        songs_by_album.insert(album_id, songs);
    }

    report(&mut progress, "Indexing library…", 75);
    let changes = crate::library::apply(
        &dbs.sync,
        &artists,
        &albums,
        &songs_by_album,
        indexes.last_modified,
        deep,
    )
    .await?;

    // Cached radio orders point at song ids from the previous catalog.
    crate::radio::clear_cache();

    // The provider date moved: drop cached descriptions/lyrics older than the
    // provider copy so the next read fetches the current text.
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

    // UI-60: the server's artist `userRating` snapshots seed the local
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
        .filter(|(id, _)| !id.is_empty())
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

    // 4. the cover cache is a secondary task the caller starts after success.

    // 5. lyric cache: drop entries for songs that left the library
    best_effort("lyric cache purge failed", crate::lyrics::purge(dbs)).await;

    // 5b. artist biography + album notes caches: drop rows that left the library
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
    _ = refresh_lyrics(dbs, &client, &mut progress).await;

    // 7. color cache: drop schemes whose cover vanished from covers.db
    best_effort("color cache purge failed", crate::colors::purge(dbs)).await;

    // 8. ARC-44: the Subsonic playlist list rides the library sync.
    _ = best_effort(
        "could not refresh playlists",
        crate::playlists::refresh_subsonic(dbs, &client),
    )
    .await;

    let after = crate::library::counts(&dbs.library)
        .await
        .unwrap_or((0, 0, 0));
    let updated = changes.any() || before != after;
    let covers_due = updated
        || deep
        || crate::library::cover_check_due(&dbs.library, crate::consts::COVER_CHECK_TTL_SECS)
            .await
            .unwrap_or(true);

    report(&mut progress, "Done", 100);
    Ok(SyncReport {
        covers_due,
        deep,
        forced_covers: changes.covers,
    })
}

/// Rebuilds the derived search caches from the current library rows: the
/// FTS5 mirror (kept as the fallback engine) and, when ARC-SEARCH is opted
/// in, the in-memory Nucleo catalog.
async fn rebuild_search(dbs: &Dbs) -> Result<()> {
    // Paged rebuild: the old path collected every search row into a Vec
    // before writing the FTS table (FIX-26).
    crate::search::rebuild_from_library(&dbs.sync).await?;
    // Only refresh the in-memory Nucleo catalog if a search session already
    // materialized it; when running headless/cold, keep it unloaded until the
    // next `search::index::ensure` so sync does not allocate ~30 MiB unused.
    if crate::search::index::enabled()
        && crate::search::index::is_loaded()
        && let Err(err) = crate::search::index::rebuild(&dbs.sync).await
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
    crate::linux::malloc_trim();
    result
}

async fn cache_covers_locked(
    dbs: &Dbs,
    client: &Arc<SubsonicClient>,
    forced: &[String],
) -> CoverReport {
    let mut referenced = match crate::library::referenced_cover_ids(&dbs.library).await {
        Ok(referenced) => referenced,
        Err(err) => {
            eprintln!("index: could not determine referenced covers: {err:#}");
            return CoverReport::default();
        }
    };
    match crate::playlists::referenced_cover_ids(&dbs.playlist).await {
        Ok(playlist_covers) => referenced.extend(playlist_covers),
        Err(err) => eprintln!("index: could not determine playlist covers: {err:#}"),
    }

    match crate::covers::refresh(&dbs.covers, client, &referenced, forced).await {
        Ok(report) => {
            _ = crate::library::touch_cover_check(&dbs.library).await;
            // Artist portraits are signed external URLs, not `getCoverArt`
            // ids; cache them as a detached follow-up so the sync does not wait
            // on 100+ extra image downloads.
            if let Ok(artists) = crate::library::artist_image_urls(&dbs.library).await {
                let pool = dbs.covers.clone();
                let client = Arc::clone(client);
                crate::app::runtime().spawn(async move {
                    if let Err(err) =
                        crate::covers::refresh_artist_images(&pool, &client, &artists).await
                    {
                        eprintln!("index: artist image cache failed: {err:#}");
                    }
                });
            }
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

/// Chunked fetch: processes `ids` in `FETCH_CONCURRENCY` batches,
/// flattening each batch into `out` instead of retaining the whole
/// `Vec<(id, Result<T>)>` (ARC-RS-4.8). Only one batch of `T` + `out`
/// is live at a time; peak is `batch + out` not `2x catalog`.
async fn fetch_chunked<T, F, Fut>(
    client: &Arc<SubsonicClient>,
    ids: &[String],
    fetch: F,
) -> Result<Vec<T>>
where
    T: Send + 'static,
    F: Fn(Arc<SubsonicClient>, String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
{
    let mut out = Vec::with_capacity(ids.len());
    let mut failed: Vec<String> = Vec::new();
    crate::fanout::for_each_bounded(
        ids,
        {
            let fetch = fetch.clone();
            let client = Arc::clone(client);
            move |id| {
                let fetch = fetch.clone();
                let client = Arc::clone(&client);
                async move { fetch(client, id).await }
            }
        },
        |id, result| match result {
            Ok(value) => out.push(value),
            Err(err) => {
                eprintln!("index: fetch failed for {id}: {err:#}");
                failed.push(id);
            }
        },
    )
    .await;
    ensure_complete(&failed, ids.len())?;
    Ok(out)
}

async fn fetch_artists(
    client: &Arc<SubsonicClient>,
    ids: &[String],
) -> Result<Vec<ArtistWithAlbumsID3>> {
    let client = Arc::clone(client);
    fetch_chunked(
        &client,
        ids,
        move |c, id| async move { c.get_artist(&id).await },
    )
    .await
}

async fn fetch_albums(
    client: &Arc<SubsonicClient>,
    ids: &[String],
) -> Result<Vec<AlbumWithSongsID3>> {
    let client = Arc::clone(client);
    fetch_chunked(
        &client,
        ids,
        move |c, id| async move { c.get_album(&id).await },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn test_dbs() -> Dbs {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .unwrap();
        let media = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations_media")
            .run(&media)
            .await
            .unwrap();
        Dbs {
            credentials: pool.clone(),
            library: pool.clone(),
            sync: pool.clone(),
            search: pool.clone(),
            covers: media.clone(),
            media: media.clone(),
            lyrics: media.clone(),
            colors: pool.clone(),
            status: pool.clone(),
            artist_user: pool.clone(),
            download: media,
            playlist: pool,
        }
    }

    async fn seed_four_artists(dbs: &Dbs) {
        let artists: Vec<ArtistID3> = (1..=4)
            .map(|i| ArtistID3 {
                id: format!("ar{i}"),
                name: format!("Artist {i}"),
                album_count: 1,
                ..ArtistID3::default()
            })
            .collect();
        let albums: Vec<AlbumID3> = (1..=4)
            .map(|i| AlbumID3 {
                id: format!("al{i}"),
                name: format!("Album {i}"),
                artist_id: Some(format!("ar{i}")),
                artist: Some(format!("Artist {i}")),
                song_count: 1,
                duration: 120,
                ..AlbumID3::default()
            })
            .collect();
        let mut songs_by_album: HashMap<String, Vec<Child>> = HashMap::new();
        for i in 1..=4 {
            songs_by_album.insert(
                format!("al{i}"),
                vec![Child {
                    id: format!("s{i}"),
                    album_id: Some(format!("al{i}")),
                    artist_id: Some(format!("ar{i}")),
                    title: format!("Song {i}"),
                    album: Some(format!("Album {i}")),
                    artist: Some(format!("Artist {i}")),
                    track: Some(1),
                    duration: Some(120),
                    ..Child::default()
                }],
            );
        }
        crate::library::apply(&dbs.sync, &artists, &albums, &songs_by_album, 100, true)
            .await
            .unwrap();
        // Force `full_sync_due` to return `true` so an automatic sync (`deep = false`)
        // would escalate `deep = true` if the safety guard ran too late.
        sqlx::query("DELETE FROM meta WHERE key = 'full_sync_at'")
            .execute(&dbs.library)
            .await
            .unwrap();
        assert!(crate::library::full_sync_due(&dbs.library).await);
    }

    async fn spawn_mock_subsonic(
        indexes_body: String,
        artist_body: String,
        album_body: String,
        album_hits: Arc<AtomicUsize>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let Ok(read) = socket.read(&mut buf).await else {
                    break;
                };
                let req = String::from_utf8_lossy(&buf[..read]);
                let path_line = req.lines().next().unwrap_or("");
                let body = if path_line.contains("/rest/getIndexes.view") {
                    indexes_body.clone()
                } else if path_line.contains("/rest/getArtist.view") {
                    artist_body.clone()
                } else if path_line.contains("/rest/getAlbum.view") {
                    album_hits.fetch_add(1, Ordering::SeqCst);
                    album_body.clone()
                } else {
                    r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#.to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                _ = socket.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn empty_and_shrunk_artist_index_rejected_even_when_full_sync_due() {
        let artist_json = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","artist":{"id":"ar1","name":"Artist 1","albumCount":1,"album":[{"id":"al1","name":"Album 1","artistId":"ar1","artist":"Artist 1","songCount":1,"duration":120}]}}}"#.to_string();
        let album_json = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"al1","name":"Album 1","artistId":"ar1","artist":"Artist 1","songCount":1,"duration":120,"song":[{"id":"s1","albumId":"al1","artistId":"ar1","title":"Song 1","album":"Album 1","artist":"Artist 1","track":1,"duration":120}]}}}"#.to_string();
        let shrunk_indexes = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","indexes":{"lastModified":200,"index":[{"name":"A","artist":[{"id":"ar1","name":"Artist 1","albumCount":1}]}],"shortcut":[]}}}"#.to_string();

        if std::env::var_os("MUSIKA_TEST_INDEX_MANUAL_DEEP").is_some() {
            let dbs = test_dbs().await;
            seed_four_artists(&dbs).await;
            let hits = Arc::new(AtomicUsize::new(0));
            let (url, server) =
                spawn_mock_subsonic(shrunk_indexes, artist_json, album_json, Arc::clone(&hits))
                    .await;
            let client = Arc::new(SubsonicClient::new(&url, "alice", "secret"));
            let report = sync_library(&dbs, client, true, |_| {}).await.unwrap();
            assert!(report.deep);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            assert_eq!(
                crate::library::counts(&dbs.library).await.unwrap(),
                (1, 1, 1)
            );
            server.abort();
            return;
        }

        let dbs = test_dbs().await;
        seed_four_artists(&dbs).await;
        assert_eq!(
            crate::library::counts(&dbs.library).await.unwrap(),
            (4, 4, 4)
        );

        // 1. Empty artist index rejected before fetch_albums even though full_sync_due is true.
        let empty_hits = Arc::new(AtomicUsize::new(0));
        let empty_indexes = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","indexes":{"lastModified":200,"index":[],"shortcut":[]}}}"#.to_string();
        let (empty_url, empty_server) = spawn_mock_subsonic(
            empty_indexes,
            artist_json.clone(),
            album_json.clone(),
            Arc::clone(&empty_hits),
        )
        .await;
        let client = Arc::new(SubsonicClient::new(&empty_url, "alice", "secret"));
        let err = sync_library(&dbs, client, false, |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("empty artist index"));
        assert_eq!(empty_hits.load(Ordering::SeqCst), 0);
        assert_eq!(
            crate::library::counts(&dbs.library).await.unwrap(),
            (4, 4, 4)
        );
        empty_server.abort();

        // 2. Shrunk artist index (1 of 4 artists) rejected before fetch_albums even though full_sync_due is true.
        let shrunk_hits = Arc::new(AtomicUsize::new(0));
        let (shrunk_url, shrunk_server) = spawn_mock_subsonic(
            shrunk_indexes,
            artist_json,
            album_json,
            Arc::clone(&shrunk_hits),
        )
        .await;
        let client = Arc::new(SubsonicClient::new(&shrunk_url, "alice", "secret"));
        let err = sync_library(&dbs, client, false, |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("1/4 artists"));
        assert_eq!(
            shrunk_hits.load(Ordering::SeqCst),
            0,
            "fetch_albums must not run on a rejected shrunk manifest"
        );
        assert_eq!(
            crate::library::counts(&dbs.library).await.unwrap(),
            (4, 4, 4)
        );
        shrunk_server.abort();

        // 3. Explicit manual deep refresh (`deep = true`) overrides the guard.
        // Run in an isolated subprocess so `sync_library`'s search/mirror side effects
        // never mutate the parent test process's global `search::index` or user config.
        let base = std::env::temp_dir().join(format!("musika-index-s04-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&base);
        let cfg = base.join("cfg");
        let cache = base.join("cache");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("XDG_CACHE_HOME", &cache)
            .env("MUSIKA_TEST_INDEX_MANUAL_DEEP", "1")
            .args([
                "--exact",
                "index::tests::empty_and_shrunk_artist_index_rejected_even_when_full_sync_due",
            ])
            .output()
            .unwrap();
        _ = std::fs::remove_dir_all(&base);
        assert!(
            output.status.success(),
            "manual deep child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
