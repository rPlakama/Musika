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

//! Search result assembly: Nucleo/FTS merge over typed rows.
//!
//! All three engines (Nucleo index, FTS, hybrid) answer with the same
//! [`SearchResults`]. The hybrid merge composes the typed columns instead of
//! serializing each engine's payload only to parse it back for the merge.

use std::collections::HashSet;

use crate::browse::rows::{format_duration, song_subtitle_from_parts};

/// One artist hit, decorated with the live library state (cover URLs, the
/// local favourite overlay and rating).
#[derive(Debug, Clone, Default)]
pub struct SearchArtist {
    pub id: String,
    pub name: String,
    pub album_count: i64,
    pub cover: String,
    pub cover_remote: String,
    pub starred: bool,
    pub rating: i64,
}

impl SearchArtist {
    /// The palette's subtitle for an artist row: the album count.
    pub fn meta(&self) -> String {
        format!(
            "{} album{}",
            self.album_count,
            if self.album_count == 1 { "" } else { "s" }
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub year: i64,
    pub song_count: i64,
    pub duration: i64,
    pub cover: String,
    pub cover_remote: String,
    pub starred: bool,
    pub rating: i64,
}

impl SearchAlbum {
    /// The palette's meta for an album row: the track count.
    pub fn meta(&self) -> String {
        format!("{} tracks", self.song_count)
    }
}

/// One song hit. The optional fields mirror the library's nullable columns
/// (a server-only or malformed row may lack an album), so the palette text
/// keeps its exact fallbacks.
#[derive(Debug, Clone, Default)]
pub struct SearchSong {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub track: Option<i32>,
    pub duration: Option<i32>,
    pub cover: String,
    pub cover_remote: String,
    pub starred: bool,
    pub play_count: i64,
}

impl SearchSong {
    /// The palette's subtitle: `artist · album` with empty parts dropped.
    pub fn subtitle(&self) -> String {
        song_subtitle_from_parts(self.artist.as_deref(), self.album.as_deref())
    }

    /// The palette's meta: the formatted duration.
    pub fn meta(&self) -> String {
        format_duration(self.duration)
    }
}

/// The unified search result: one typed column per scope plus the surface
/// flags the palette keys off (`most_played` for the empty query, the single
/// album context that turns the Songs column into its tracklist).
#[derive(Debug, Clone, Default)]
pub struct SearchResults {
    pub most_played: bool,
    pub album_context: Option<String>,
    pub artists: Vec<SearchArtist>,
    pub albums: Vec<SearchAlbum>,
    pub songs: Vec<SearchSong>,
    pub playlists: Vec<crate::playlists::PlaylistHit>,
}

impl SearchResults {
    /// Merge a sparse secondary engine into this result (primary order wins,
    /// rows are deduped by id and the visible cap is kept).
    fn merge(&mut self, secondary: SearchResults, limit: usize) {
        merge_hits(&mut self.artists, secondary.artists, limit, |row| &row.id);
        merge_hits(&mut self.albums, secondary.albums, limit, |row| &row.id);
        merge_hits(&mut self.songs, secondary.songs, limit, |row| &row.id);
    }
}

/// Append `secondary` rows to `primary`, keeping the primary order and the
/// visible cap. Rows are identified by their library id; the secondary vector
/// is consumed, never cloned.
pub(crate) fn merge_hits<T>(
    primary: &mut Vec<T>,
    secondary: Vec<T>,
    limit: usize,
    id: impl Fn(&T) -> &String,
) {
    let mut seen: HashSet<String> = primary.iter().map(|row| id(row).clone()).collect();
    for row in secondary {
        if primary.len() >= limit {
            break;
        }
        if !seen.insert(id(&row).clone()) {
            continue;
        }
        primary.push(row);
    }
    primary.truncate(limit);
}

/// The unified search: the engine dispatch with the FTS fallback.
pub async fn search(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<SearchResults> {
    let engine = crate::search::engine();
    // A library past the catalog budget already knows FTS is the answer; do
    // not build a Nucleo result only to discard it.
    if engine != crate::search::Engine::Fts && crate::search::index::over_budget() {
        return search_results_fts(dbs, query, offline, limit).await;
    }
    match engine {
        crate::search::Engine::Hybrid => search_results_hybrid(dbs, query, offline, limit).await,
        crate::search::Engine::Nucleo => {
            match search_results_index(dbs, query, offline, limit).await {
                Ok(results) => Ok(results),
                Err(err) => {
                    // The in-memory catalog is a cache: a build failure must
                    // not take the palette down while FTS is still available.
                    eprintln!("search: nucleo engine failed, using FTS: {err:#}");
                    search_results_fts(dbs, query, offline, limit).await
                }
            }
        }
        crate::search::Engine::Fts => search_results_fts(dbs, query, offline, limit).await,
    }
}

/// Nucleo answers first; FTS fills any column that came back short (or steps
/// in wholesale when the in-memory catalog fails). The merge is only paid for
/// sparse queries — a broad hit set never touches SQLite.
async fn search_results_hybrid(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<SearchResults> {
    let mut nucleo = match search_results_index(dbs, query, offline, limit).await {
        Ok(results) => results,
        Err(err) => {
            eprintln!("search: nucleo engine failed, using FTS: {err:#}");
            return search_results_fts(dbs, query, offline, limit).await;
        }
    };
    // The first search may have discovered the over-budget verdict while this
    // request was in flight; `search_results_index` already returned the FTS
    // payload then, so do not merge FTS with itself.
    if crate::search::index::over_budget() {
        return Ok(nucleo);
    }
    // The empty-query surface and S-8's single-album tracklist already own
    // their result set; do not let FTS mix another album in.
    if nucleo.most_played || nucleo.album_context.is_some() {
        return Ok(nucleo);
    }
    // The FTS fill is for sparse song sets (a rare/typo query, or an offline
    // catalog); a broad hit set already fills the Songs column, and the
    // artist/album lists are cheap Nucleo lookups.
    if nucleo.songs.len() >= limit {
        return Ok(nucleo);
    }

    let fts = match search_results_fts(dbs, query, offline, limit).await {
        Ok(results) => results,
        Err(err) => {
            eprintln!("search: FTS fill failed: {err:#}");
            return Ok(nucleo);
        }
    };
    nucleo.merge(fts, limit);
    Ok(nucleo)
}

/// A search hit before the live-library decoration pass.
struct SeedSong {
    id: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_id: Option<String>,
    track: Option<i32>,
    duration: Option<i32>,
    cover_art: Option<String>,
    play_count: i64,
}

impl SeedSong {
    fn from_index(hit: &crate::search::index::SongHit) -> Self {
        Self {
            id: hit.id.clone(),
            title: hit.title.clone(),
            artist: hit.artist.clone(),
            album: hit.album.clone(),
            album_id: hit.album_id.clone(),
            track: hit.track,
            duration: hit.duration,
            cover_art: hit.cover_art.clone(),
            play_count: hit.play_count,
        }
    }
}

async fn search_results_fts(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<SearchResults> {
    let filter = crate::offline::load(dbs, offline).await?;
    let limit = limit.max(1);
    // Offline hits are filtered down to the downloaded catalog, so over-fetch
    // before truncating back to the rendered rows.
    let fetch = if filter.is_some() { limit * 3 } else { limit };
    // S-9: an empty query is the "Most played" surface, not three dead
    // columns. Punctuation is ignored for matching (`(hello` == `hello`), but
    // the raw query still reaches the payload consumers.
    let cleaned = crate::search::clean_query(query);
    let most_played = cleaned.is_empty();
    let (artists, albums, mut songs) = if most_played {
        // The unified search's empty-query surface browses every scope, so
        // artists and albums are full name-ordered lists (songs stay Most
        // played, S-9).
        let (artists, albums, songs) = tokio::try_join!(
            crate::library::artists(&dbs.library, "Name"),
            crate::library::albums(&dbs.library, None, "Artist"),
            crate::search::most_played(&dbs.search, fetch as i64),
        )?;
        (artists, albums, songs)
    } else {
        let ((artists, albums), songs) = tokio::try_join!(
            crate::library::search_artists_albums(&dbs.library, &cleaned, fetch as i64),
            crate::search::search(&dbs.search, &cleaned, fetch as i64),
        )?;
        (artists, albums, songs)
    };
    let (mut artists, mut albums) = match &filter {
        Some(filter) => (
            artists
                .into_iter()
                .filter(|artist| filter.contains_artist(&artist.id))
                .collect(),
            albums
                .into_iter()
                .filter(|album| filter.contains_album(&album.id))
                .collect(),
        ),
        None => (artists, albums),
    };
    artists.truncate(limit);
    albums.truncate(limit);
    if let Some(filter) = &filter {
        songs.retain(|song| filter.contains_song(&song.id));
    }
    songs.truncate(limit);

    let song_ids: Vec<String> = songs.iter().map(|song| song.id.clone()).collect();
    let (song_covers, song_albums) = tokio::try_join!(
        crate::library::song_cover_ids(&dbs.library, &song_ids),
        crate::library::song_album_ids(&dbs.library, &song_ids),
    )?;
    let songs: Vec<SeedSong> = songs
        .into_iter()
        .map(|song| {
            let cover_art = song_covers.get(&song.id).cloned();
            let album_id = song_albums.get(&song.id).cloned();
            SeedSong {
                id: song.id,
                title: song.title,
                artist: song.artist,
                album: song.album,
                album_id,
                track: None,
                duration: song.duration,
                cover_art,
                play_count: song.play_count.unwrap_or(0),
            }
        })
        .collect();

    decorate(
        dbs,
        query,
        most_played,
        None,
        artists.into_iter().map(std::sync::Arc::new).collect(),
        albums.into_iter().map(std::sync::Arc::new).collect(),
        songs,
        limit,
    )
    .await
}

pub(crate) async fn search_results_index(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<SearchResults> {
    let filter = crate::offline::load(dbs, offline).await?;
    let limit = limit.max(1);
    let fetch = if filter.is_some() { limit * 3 } else { limit };
    let cleaned = crate::search::clean_query(query);
    let most_played = cleaned.is_empty();
    let Some(index) = crate::search::index::ensure(&dbs.library).await? else {
        // Over budget: the catalog declined to build, FTS owns the query.
        return search_results_fts(dbs, query, offline, limit).await;
    };
    let (mut artists, mut albums, mut songs) = if most_played {
        // The unified search's empty-query surface browses every scope:
        // artists and albums are full name-ordered lists, songs stay Most
        // played (S-9).
        let (artists, albums) = tokio::try_join!(
            crate::library::artists(&dbs.library, "Name"),
            crate::library::albums(&dbs.library, None, "Artist"),
        )?;
        let artists = artists.into_iter().map(std::sync::Arc::new).collect();
        let albums = albums.into_iter().map(std::sync::Arc::new).collect();
        (
            artists,
            albums,
            index
                .most_played(fetch)
                .iter()
                .map(|hit| SeedSong::from_index(hit))
                .collect::<Vec<_>>(),
        )
    } else {
        // Nucleo's tick blocks for the worker; keep it off the async workers.
        let index = std::sync::Arc::clone(&index);
        let cleaned = cleaned.clone();
        let results = tokio::task::spawn_blocking(move || index.search(&cleaned, fetch)).await?;
        (
            results.artists,
            results.albums,
            results.songs.iter().map(|hit| SeedSong::from_index(hit)).collect(),
        )
    };
    if let Some(filter) = &filter {
        artists.retain(|artist| filter.contains_artist(&artist.id));
        albums.retain(|album| filter.contains_album(&album.id));
        songs.retain(|song| filter.contains_song(&song.id));
    }
    artists.truncate(limit);
    albums.truncate(limit);
    // S-8: one album match turns the Songs column into that album's
    // tracklist. The index already keeps the tracks sorted by disc/track.
    let album_context = (!most_played && albums.len() == 1).then(|| albums[0].id.clone());
    if let Some(album_id) = &album_context {
        songs = index
            .album_tracks(album_id, fetch)
            .iter()
            .map(|hit| SeedSong::from_index(hit))
            .collect();
        if let Some(filter) = &filter {
            songs.retain(|song| filter.contains_song(&song.id));
        }
    }
    songs.truncate(limit);

    decorate(
        dbs,
        query,
        most_played,
        album_context,
        artists,
        albums,
        songs,
        limit,
    )
    .await
}

/// Decorate the engine hits with the live library state that the index's
/// sync-time snapshot cannot know: cover URLs, the favourite overlay and
/// ratings, and the playlist column.
#[expect(
    clippy::too_many_arguments,
    reason = "parameters are the independent search/query overlays; a struct would add a second vocabulary"
)]
async fn decorate(
    dbs: &crate::db::Dbs,
    query: &str,
    most_played: bool,
    album_context: Option<String>,
    artists: Vec<std::sync::Arc<crate::library::ArtistRow>>,
    albums: Vec<std::sync::Arc<crate::library::AlbumRow>>,
    songs: Vec<SeedSong>,
    limit: usize,
) -> anyhow::Result<SearchResults> {
    let playlists = crate::playlists::search_rows(dbs, query, limit as i64).await?;
    let mut cover_ids: Vec<String> = albums
        .iter()
        .filter_map(|album| album.cover_art.clone())
        .collect();
    cover_ids.extend(artists.iter().filter_map(|artist| artist.cover_art.clone()));
    cover_ids.extend(songs.iter().filter_map(|song| song.cover_art.clone()));
    let covers = crate::covers::url_map_of(dbs, &cover_ids);
    let remotes = crate::covers::remote_map_of(dbs, &cover_ids);
    // UI-56: hearts/ratings are live library state, not the index's
    // sync-time snapshot; read them for just this page.
    let song_ids: Vec<String> = songs.iter().map(|song| song.id.clone()).collect();
    let song_starred = crate::library::song_starred_ids(&dbs.library, &song_ids);
    let album_ids: Vec<String> = albums.iter().map(|album| album.id.clone()).collect();
    let album_states = crate::library::album_states(&dbs.library, &album_ids);
    let user_starred = crate::artist_user::starred_ids(&dbs.artist_user);
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user);
    let (covers, remotes, song_starred, album_states, user_starred, user_ratings) = tokio::try_join!(
        covers,
        remotes,
        song_starred,
        album_states,
        user_starred,
        user_ratings
    )?;
    let user_starred: HashSet<String> = user_starred.into_iter().collect();

    let artist_rows = artists
        .iter()
        .map(|artist| SearchArtist {
            id: artist.id.clone(),
            name: artist.name.clone(),
            album_count: artist.album_count,
            cover: crate::covers::url_for(artist.cover_art.as_deref(), &covers),
            cover_remote: artist
                .cover_art
                .as_deref()
                .and_then(|id| remotes.get(id))
                .cloned()
                .unwrap_or_default(),
            starred: artist.starred.is_some() || user_starred.contains(&artist.id),
            rating: user_ratings.get(&artist.id).copied().unwrap_or(0),
        })
        .collect();
    let album_rows = albums
        .iter()
        .map(|album| {
            let (starred, rating) = album_states
                .get(&album.id)
                .map(|(starred, rating)| (*starred, *rating))
                .unwrap_or_else(|| (album.starred.is_some(), album.rating));
            SearchAlbum {
                id: album.id.clone(),
                title: album.title.clone(),
                artist: album.artist.clone().unwrap_or_default(),
                year: album.year.map(i64::from).unwrap_or(0),
                song_count: album.song_count,
                duration: album.duration,
                cover: crate::covers::url_for(album.cover_art.as_deref(), &covers),
                cover_remote: album
                    .cover_art
                    .as_deref()
                    .and_then(|id| remotes.get(id))
                    .cloned()
                    .unwrap_or_default(),
                starred,
                rating,
            }
        })
        .collect();
    let song_rows = songs
        .into_iter()
        .map(|song| SearchSong {
            cover: crate::covers::url_for(song.cover_art.as_deref(), &covers),
            cover_remote: song
                .cover_art
                .as_deref()
                .and_then(|id| remotes.get(id))
                .cloned()
                .unwrap_or_default(),
            starred: song_starred.contains(&song.id),
            id: song.id,
            title: song.title,
            artist: song.artist,
            album: song.album,
            album_id: song.album_id,
            track: song.track,
            duration: song.duration,
            play_count: song.play_count,
        })
        .collect();

    Ok(SearchResults {
        most_played,
        album_context,
        artists: artist_rows,
        albums: album_rows,
        songs: song_rows,
        playlists,
    })
}

