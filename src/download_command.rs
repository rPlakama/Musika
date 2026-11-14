//! Scoped download/removal command backend.
//!
//! The textual `download` command describes a scope by name; this module
//! resolves that description to library ids and then either hands every song
//! to the media downloader or removes the local marks/blobs. Removal is
//! device-local by design: it never calls a server delete endpoint.

use anyhow::{Context, Result, anyhow, bail};

use crate::db::Dbs;
use crate::ipc_command::{CurrentTrack, DownloadMode, DownloadRequest, DownloadScope};

/// Name resolution only needs the best few search hits, not a full page.
const RESOLVE_LIMIT: usize = 50;

/// The concrete library ids a request resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedIds {
    Track(String),
    Album {
        album_id: String,
        song_ids: Vec<String>,
    },
    Artist {
        artist_id: String,
        song_ids: Vec<String>,
    },
}

impl ResolvedIds {
    fn song_ids(&self) -> &[String] {
        match self {
            Self::Track(id) => std::slice::from_ref(id),
            Self::Album { song_ids, .. } | Self::Artist { song_ids, .. } => song_ids,
        }
    }
}

/// One candidate from a search payload or a library list.
struct Candidate {
    id: String,
    label: String,
}

/// Resolve and execute one download/removal request.
pub(crate) async fn execute(
    dbs: &Dbs,
    request: &DownloadRequest,
    current: Option<&CurrentTrack>,
) -> Result<()> {
    let resolved = resolve_ids(dbs, request, current).await?;
    if request.remove {
        remove(dbs, resolved).await
    } else {
        download(dbs, resolved).await
    }
}

/// Resolve a request to the library ids it targets.
///
/// Exposed to the crate so the resolver can be tested without performing any
/// download or removal work.
pub(crate) async fn resolve_ids(
    dbs: &Dbs,
    request: &DownloadRequest,
    current: Option<&CurrentTrack>,
) -> Result<ResolvedIds> {
    match &request.mode {
        DownloadMode::Current => {
            let current = current.ok_or_else(|| anyhow!("No current track is playing."))?;
            if current.id.trim().is_empty() {
                bail!("No current track is playing.");
            }
            Ok(ResolvedIds::Track(current.id.clone()))
        }
        DownloadMode::Direct { scope, query } => resolve_direct(dbs, *scope, query).await,
        DownloadMode::Path {
            artist,
            album,
            track,
        } => {
            if track.is_some() && album.is_none() {
                bail!("A track needs an album name.");
            }
            resolve_path(dbs, artist, album.as_deref(), track.as_deref()).await
        }
    }
}

/// Direct scope: ask the unified search for names, then match the requested
/// phrase within the returned scope.
async fn resolve_direct(dbs: &Dbs, scope: DownloadScope, query: &str) -> Result<ResolvedIds> {
    // Resolution is always against the local catalog: resolving a download
    // target must see the full library even while playback is offline.
    let results = crate::browse::search(dbs, query, false, RESOLVE_LIMIT)
        .await
        .context("Could not search the library")?;
    let id = search_match_id(&results, scope, query).ok_or_else(|| no_match(scope, query))?;
    resolve_id(dbs, scope, id).await
}

/// Path scope: artist by phrase, then album within that artist, then track
/// within that album.
async fn resolve_path(
    dbs: &Dbs,
    artist: &str,
    album: Option<&str>,
    track: Option<&str>,
) -> Result<ResolvedIds> {
    let (artists, _) =
        crate::library::search_artists_albums(&dbs.library, artist, RESOLVE_LIMIT as i64).await?;
    let artists: Vec<Candidate> = artists
        .into_iter()
        .map(|artist| Candidate {
            id: artist.id,
            label: artist.name,
        })
        .collect();
    let artist_id = best_match(&artists, artist, false)
        .map(|artist| artist.id.clone())
        .ok_or_else(|| no_match(DownloadScope::Artist, artist))?;

    let Some(album) = album else {
        let song_ids = crate::library::artist_song_ids(&dbs.library, &artist_id).await?;
        return Ok(ResolvedIds::Artist {
            artist_id,
            song_ids,
        });
    };

    let albums = crate::library::artist_albums(&dbs.library, &artist_id, "Name").await?;
    let albums: Vec<Candidate> = albums
        .into_iter()
        .map(|album| Candidate {
            id: album.id,
            label: album.title,
        })
        .collect();
    let album_id = best_match(&albums, album, false)
        .map(|album| album.id.clone())
        .ok_or_else(|| no_match(DownloadScope::Album, album))?;

    let Some(track) = track else {
        let song_ids = crate::library::album_song_ids(&dbs.library, &album_id).await?;
        return Ok(ResolvedIds::Album { album_id, song_ids });
    };

    let songs = crate::library::album_songs(&dbs.library, &album_id).await?;
    let songs: Vec<Candidate> = songs
        .into_iter()
        .map(|song| Candidate {
            id: song.id,
            label: song.title,
        })
        .collect();
    let track_id = best_match(&songs, track, false)
        .map(|song| song.id.clone())
        .ok_or_else(|| no_match(DownloadScope::Track, track))?;
    Ok(ResolvedIds::Track(track_id))
}

/// Resolve a name-or-id argument for commands that need one scope id
/// (`shuffle album`, `radio song`, …). An exact library id wins; otherwise the
/// same search engine the Search overlay uses resolves the phrase.
pub(crate) async fn resolve_named_id(
    dbs: &Dbs,
    scope: DownloadScope,
    value: &str,
) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("An empty name cannot be resolved.");
    }
    let is_id = match scope {
        DownloadScope::Artist => crate::library::artist(&dbs.library, value).await?.is_some(),
        DownloadScope::Album => crate::library::album(&dbs.library, value).await?.is_some(),
        DownloadScope::Track => crate::library::song(&dbs.library, value).await?.is_some(),
    };
    if is_id {
        return Ok(value.to_string());
    }
    let results = crate::browse::search(dbs, value, false, RESOLVE_LIMIT)
        .await
        .context("Could not search the library")?;
    search_match_id(&results, scope, value).ok_or_else(|| no_match(scope, value))
}

/// Turn one already-matched scope id into its concrete download/removal set.
async fn resolve_id(dbs: &Dbs, scope: DownloadScope, id: String) -> Result<ResolvedIds> {
    match scope {
        DownloadScope::Artist => {
            let song_ids = crate::library::artist_song_ids(&dbs.library, &id).await?;
            Ok(ResolvedIds::Artist {
                artist_id: id,
                song_ids,
            })
        }
        DownloadScope::Album => {
            let song_ids = crate::library::album_song_ids(&dbs.library, &id).await?;
            Ok(ResolvedIds::Album {
                album_id: id,
                song_ids,
            })
        }
        DownloadScope::Track => Ok(ResolvedIds::Track(id)),
    }
}

/// Stable best match: exact (case-insensitive), then prefix, then contains.
/// Ties keep the earlier candidate.
///
/// `fallback_first` is for search hits: a fuzzy engine can return a relevant
/// row with no textual match, so its first row stays the deterministic
/// fallback. Local lists (an artist's albums, an album's tracks) are complete,
/// so an absent name is a real no-match instead of an arbitrary first row.
fn best_match<'a>(
    candidates: &'a [Candidate],
    query: &str,
    fallback_first: bool,
) -> Option<&'a Candidate> {
    let query = query.trim().to_lowercase();
    if query.is_empty() && !fallback_first {
        return None;
    }
    let mut best: Option<(u8, &'a Candidate)> = None;
    for candidate in candidates {
        let label = candidate.label.to_lowercase();
        let rank = if label == query {
            0
        } else if label.starts_with(&query) {
            1
        } else if label.contains(&query) {
            2
        } else {
            continue;
        };
        if best.is_none_or(|(best_rank, _)| rank < best_rank) {
            best = Some((rank, candidate));
        }
    }
    best.map(|(_, candidate)| candidate).or_else(|| {
        // A search hit can be relevant without a textual match (fuzzy
        // ranking, typo tolerance); the engine's first row stays the
        // deterministic fallback.
        if fallback_first {
            candidates.first()
        } else {
            None
        }
    })
}

/// Pick the id of the best matching row in one search-payload column.
pub(crate) fn search_match_id(
    results: &crate::browse::search::SearchResults,
    scope: DownloadScope,
    query: &str,
) -> Option<String> {
    let candidates: Vec<Candidate> = match scope {
        DownloadScope::Artist => results
            .artists
            .iter()
            .map(|artist| Candidate {
                id: artist.id.clone(),
                label: artist.name.clone(),
            })
            .collect(),
        DownloadScope::Album => results
            .albums
            .iter()
            .map(|album| Candidate {
                id: album.id.clone(),
                label: album.title.clone(),
            })
            .collect(),
        DownloadScope::Track => results
            .songs
            .iter()
            .map(|song| Candidate {
                id: song.id.clone(),
                label: song.title.clone(),
            })
            .collect(),
    };
    best_match(&candidates, query, true).map(|candidate| candidate.id.clone())
}

fn no_match(scope: DownloadScope, query: &str) -> anyhow::Error {
    match scope {
        DownloadScope::Artist => anyhow!("No artist matched “{query}”"),
        DownloadScope::Album => anyhow!("No album matched “{query}”"),
        DownloadScope::Track => anyhow!("No track matched “{query}”"),
    }
}

/// Download every resolved song. All songs are attempted even after a
/// failure; the first error is reported with a success/failure summary.
async fn download(dbs: &Dbs, resolved: ResolvedIds) -> Result<()> {
    let song_ids = resolved.song_ids();
    if song_ids.is_empty() {
        bail!("No songs matched the download request.");
    }
    let total = song_ids.len();
    let mut failures = 0usize;
    let mut first_failure: Option<(String, anyhow::Error)> = None;
    for song_id in song_ids {
        if let Err(err) = crate::media_actions::download_one(dbs, song_id).await {
            failures += 1;
            if first_failure.is_none() {
                first_failure = Some((song_id.clone(), err));
            }
        }
    }
    let Some((song_id, err)) = first_failure else {
        return Ok(());
    };

    let succeeded = total - failures;
    if succeeded == 0 {
        bail!("No songs were downloaded ({failures}/{total} failed); {song_id}: {err:#}");
    }
    bail!("Downloaded {succeeded} of {total} songs; {song_id}: {err:#}");
}

/// Remove only the local side of a request: marks and cached media. Server
/// library entries are never deleted.
async fn remove(dbs: &Dbs, resolved: ResolvedIds) -> Result<()> {
    match resolved {
        ResolvedIds::Track(id) => remove_songs(dbs, std::slice::from_ref(&id)).await,
        ResolvedIds::Album { album_id, song_ids } => {
            crate::media_actions::album_container_off(dbs, &album_id).await;
            remove_songs(dbs, &song_ids).await
        }
        ResolvedIds::Artist {
            artist_id,
            song_ids,
        } => {
            let album_ids = crate::library::artist_album_ids(&dbs.library, &artist_id).await?;
            for album_id in &album_ids {
                crate::media_actions::album_container_off(dbs, album_id).await;
            }
            remove_songs(dbs, &song_ids).await
        }
    }
}

/// Drop every song's ledger mark and its config-tier blobs. Every operation
/// is attempted; the first error is returned after the batch finished.
async fn remove_songs(dbs: &Dbs, song_ids: &[String]) -> Result<()> {
    let mut first_error = None;
    for song_id in song_ids {
        if let Err(err) = crate::download::unmark(&dbs.download, song_id).await
            && first_error.is_none()
        {
            first_error = Some(err);
        }
    }
    if let Err(err) = crate::media::remove_songs_media(&dbs.media, song_ids).await
        && first_error.is_none()
    {
        first_error = Some(err);
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Resolve one free-form playlist-add target (`"Pink Floyd"`, `"Model Man"`)
/// to song ids. The search payload is authoritative for names; a fuzzy hit
/// still provides a deterministic fallback when no row matches textually.
pub(crate) async fn resolve_target_song_ids(dbs: &Dbs, target: &str) -> Result<Vec<String>> {
    let results = crate::browse::search(dbs, target, false, RESOLVE_LIMIT)
        .await
        .context("Could not search the library")?;
    let mut candidates: Vec<(u8, u8, String, u8)> = Vec::new();
    // kind priority for equal textual rank: song, album, artist.
    let query = target.trim().to_lowercase();
    if query.is_empty() {
        bail!("An empty playlist target cannot be resolved.");
    }
    let mut add = |id: &str, label: &str, kind: u8, index: usize| {
        if id.is_empty() {
            return;
        }
        let label = label.to_lowercase();
        let rank = if label == query {
            0
        } else if label.starts_with(&query) {
            1
        } else if label.contains(&query) {
            2
        } else {
            3
        };
        candidates.push((rank, kind, id.to_string(), index as u8));
    };
    for (index, song) in results.songs.iter().enumerate() {
        add(&song.id, &song.title, 0, index);
    }
    for (index, album) in results.albums.iter().enumerate() {
        add(&album.id, &album.title, 1, index);
    }
    for (index, artist) in results.artists.iter().enumerate() {
        add(&artist.id, &artist.name, 2, index);
    }
    let best_rank = candidates.iter().map(|(rank, ..)| *rank).min();
    candidates.sort_by_key(|(rank, kind, _, index)| {
        // Exact/prefix/contains hits prefer the most specific row; a wholly
        // fuzzy hit falls back to artists first, matching the search payload.
        let kind_priority = if *rank == 3 { 2 - *kind } else { *kind };
        (*rank, kind_priority, *index)
    });
    let Some((rank, kind, id, _)) = candidates.first().cloned() else {
        bail!("No playlist target matched “{target}”");
    };
    debug_assert_eq!(Some(rank), best_rank);
    if rank == 3 {
        // Search fuzzy hits still count; use the first row's kind below.
        eprintln!("download: playlist target {target:?} used a fuzzy search hit");
    }
    match kind {
        0 => Ok(vec![id]),
        1 => crate::library::album_song_ids(&dbs.library, &id).await,
        _ => crate::library::artist_song_ids(&dbs.library, &id).await,
    }
}
