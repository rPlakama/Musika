//! Library sync orchestration.
//!
//! Fetches the library from the Subsonic API and fans it out to the three
//! content databases:
//!
//!   library.db  ← metadata (artists/albums/songs)
//!   search.db   ← FTS5 rebuild, right after the library write
//!   covers.db   ← cover cache, only after the sync has otherwise succeeded
//!
//! The sync is incremental at the top level: `getIndexes` is asked for changes
//! since the last `lastModified`, and an empty answer is a no-op.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::task::JoinSet;

use crate::api::{AlbumID3, AlbumWithSongsID3, ArtistID3, ArtistWithAlbumsID3, Child};
use crate::covers::CoverReport;
use crate::db::Dbs;
use crate::http_client::SubsonicClient;

const FETCH_CONCURRENCY: usize = 8;

/// Outcome of a [`sync_library`] run.
#[derive(Debug, Default, Clone, Copy)]
#[allow(dead_code)]
pub struct SyncReport {
    /// `false` when the server reported no changes and nothing was fetched.
    pub updated: bool,
    pub artists: usize,
    pub albums: usize,
    pub songs: usize,
    pub covers: CoverReport,
}

/// Coarse progress update emitted while a sync runs.
#[derive(Debug, Clone)]
pub struct SyncProgress {
    pub message: String,
    pub done: usize,
    pub total: usize,
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
    progress(SyncProgress {
        message: "Checking for changes…".to_string(),
        done: 0,
        total: 100,
    });

    let last_modified = crate::library::last_modified(&dbs.library).await?;

    let indexes = client
        .get_indexes(None, (last_modified > 0).then_some(last_modified))
        .await
        .context("failed to fetch the library index")?;

    if indexes.index.is_empty() && indexes.shortcut.is_empty() {
        crate::library::touch_last_sync(&dbs.library).await?;
        progress(SyncProgress {
            message: "Up to date".to_string(),
            done: 100,
            total: 100,
        });
        // The library is unchanged, but the cover cache may still need topping
        // up (first run interrupted, or covers that previously failed).
        let covers = refresh_covers(dbs, &client).await;
        return Ok(SyncReport {
            updated: false,
            covers,
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

    progress(SyncProgress {
        message: format!("Fetching {} artists…", artist_ids.len()),
        done: 10,
        total: 100,
    });
    let fetched_artists = fetch_artists(&client, &artist_ids).await;

    let mut album_ids: Vec<String> = fetched_artists
        .iter()
        .flat_map(|artist| artist.album.iter())
        .map(|album| album.id.clone())
        .collect();
    album_ids.sort();
    album_ids.dedup();

    progress(SyncProgress {
        message: format!("Fetching {} albums…", album_ids.len()),
        done: 40,
        total: 100,
    });
    let fetched_albums = fetch_albums(&client, &album_ids).await;

    let artists: Vec<ArtistID3> = fetched_artists
        .into_iter()
        .map(|entry| entry.artist)
        .collect();
    let mut albums: Vec<AlbumID3> = Vec::with_capacity(fetched_albums.len());
    let mut songs: Vec<Child> = Vec::new();
    for entry in &fetched_albums {
        albums.push(entry.album.clone());
        songs.extend(entry.song.iter().cloned());
    }

    // 1. library metadata
    progress(SyncProgress {
        message: "Indexing library…".to_string(),
        done: 65,
        total: 100,
    });
    crate::library::replace(&dbs.library, &artists, &albums, &songs, indexes.last_modified).await?;

    // 2. search index
    let rows = crate::library::song_search_rows(&dbs.library).await?;
    crate::search::rebuild(&dbs.search, &rows).await?;

    // 3. plain-text mirror for external tools (best effort)
    if let Err(err) = crate::library::write_mirror(&dbs.library).await {
        eprintln!("index: could not write the search mirror: {err:#}");
    }

    // 4. cover cache, only now that the sync has otherwise succeeded
    progress(SyncProgress {
        message: "Caching covers…".to_string(),
        done: 80,
        total: 100,
    });
    let covers = refresh_covers(dbs, &client).await;

    // 5. lyric cache: drop entries for songs that left the library
    if let Err(err) = crate::lyrics::purge(dbs).await {
        eprintln!("index: lyric cache purge failed: {err:#}");
    }

    progress(SyncProgress {
        message: "Done".to_string(),
        done: 100,
        total: 100,
    });

    Ok(SyncReport {
        updated: true,
        artists: artists.len(),
        albums: albums.len(),
        songs: songs.len(),
        covers,
    })
}

/// Revalidates/downloads referenced covers and purges the rest.
///
/// Failures are logged and swallowed: the library and search databases are
/// already consistent, and a cover cache problem is not fatal.
async fn refresh_covers(dbs: &Dbs, client: &Arc<SubsonicClient>) -> CoverReport {
    let referenced = match crate::library::referenced_cover_ids(&dbs.library).await {
        Ok(referenced) => referenced,
        Err(err) => {
            eprintln!("index: could not determine referenced covers: {err:#}");
            return CoverReport::default();
        }
    };

    match crate::covers::refresh(&dbs.covers, client, &referenced).await {
        Ok(report) => report,
        Err(err) => {
            eprintln!("index: cover sync failed: {err:#}");
            CoverReport::default()
        }
    }
}

async fn fetch_artists(client: &Arc<SubsonicClient>, ids: &[String]) -> Vec<ArtistWithAlbumsID3> {
    let mut out = Vec::new();
    for chunk in ids.chunks(FETCH_CONCURRENCY) {
        let mut set = JoinSet::new();
        for id in chunk {
            let client = Arc::clone(client);
            let id = id.clone();
            set.spawn(async move {
                let result = client.get_artist(&id).await;
                (id, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((_id, Ok(artist))) => out.push(artist),
                Ok((id, Err(err))) => eprintln!("index: failed to fetch artist {id}: {err:#}"),
                Err(err) => eprintln!("index: an artist fetch task failed: {err}"),
            }
        }
    }
    out
}

async fn fetch_albums(client: &Arc<SubsonicClient>, ids: &[String]) -> Vec<AlbumWithSongsID3> {
    let mut out = Vec::new();
    for chunk in ids.chunks(FETCH_CONCURRENCY) {
        let mut set = JoinSet::new();
        for id in chunk {
            let client = Arc::clone(client);
            let id = id.clone();
            set.spawn(async move {
                let result = client.get_album(&id).await;
                (id, result)
            });
        }
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((_id, Ok(album))) => out.push(album),
                Ok((id, Err(err))) => eprintln!("index: failed to fetch album {id}: {err:#}"),
                Err(err) => eprintln!("index: an album fetch task failed: {err}"),
            }
        }
    }
    out
}
