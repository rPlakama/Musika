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

//! Search payload assembly: Nucleo/FTS merge and the unified JSON shape.

use std::collections::HashSet;

use super::rows::{album_json, artist_json, song_json};

struct SearchSong {
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

impl SearchSong {
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

pub async fn search_json(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<String> {
    Ok(search_value(dbs, query, offline, limit).await?.to_string())
}

/// One payload, built as a [`serde_json::Value`] until the bridge boundary.
/// The hybrid merge composes two engines' results, so serializing each one
/// first and parsing it back only to merge would copy the whole payload
/// several times per keystroke.
async fn search_value(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let engine = crate::search::engine();
    // A library past the catalog budget already knows FTS is the answer; do
    // not rebuild/serialize a Nucleo payload only to discard it.
    if engine != crate::search::Engine::Fts && crate::search::index::over_budget() {
        return search_value_fts(dbs, query, offline, limit).await;
    }
    match engine {
        crate::search::Engine::Hybrid => search_value_hybrid(dbs, query, offline, limit).await,
        crate::search::Engine::Nucleo => {
            match search_value_index(dbs, query, offline, limit).await {
                Ok(value) => Ok(value),
                Err(err) => {
                    // The in-memory catalog is a cache: a build failure must
                    // not take the palette down while FTS is still available.
                    eprintln!("search: nucleo engine failed, using FTS: {err:#}");
                    search_value_fts(dbs, query, offline, limit).await
                }
            }
        }
        crate::search::Engine::Fts => search_value_fts(dbs, query, offline, limit).await,
    }
}

/// Nucleo answers first; FTS fills any column that came back short (or steps
/// in wholesale when the in-memory catalog fails). The merge is only paid for
/// sparse queries — a broad hit set never touches SQLite.
async fn search_value_hybrid(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let mut nucleo = match search_value_index(dbs, query, offline, limit).await {
        Ok(value) => value,
        Err(err) => {
            eprintln!("search: nucleo engine failed, using FTS: {err:#}");
            return search_value_fts(dbs, query, offline, limit).await;
        }
    };
    // The first search may have discovered the over-budget verdict while this
    // request was in flight; `search_value_index` already returned the FTS
    // payload then, so do not merge FTS with itself.
    if crate::search::index::over_budget() {
        return Ok(nucleo);
    }
    // The empty-query surface and S-8's single-album tracklist already own
    // their result set; do not let FTS mix another album in.
    if nucleo["most_played"].as_bool() == Some(true) || nucleo["album_context"].is_string() {
        return Ok(nucleo);
    }
    // The FTS fill is for sparse song sets (a rare/typo query, or an offline
    // catalog); a broad hit set already fills the Songs column, and the
    // artist/album lists are cheap Nucleo lookups.
    let sparse_songs = nucleo
        .get("songs")
        .and_then(serde_json::Value::as_array)
        .is_none_or(|rows| rows.len() < limit);
    if !sparse_songs {
        return Ok(nucleo);
    }

    let mut fts = match search_value_fts(dbs, query, offline, limit).await {
        Ok(value) => value,
        Err(err) => {
            eprintln!("search: FTS fill failed: {err:#}");
            return Ok(nucleo);
        }
    };
    for key in ["songs", "artists", "albums"] {
        if let (Some(primary), Some(secondary)) = (nucleo.get_mut(key), fts.get_mut(key)) {
            merge_json_rows(primary, secondary, limit);
        }
    }
    Ok(nucleo)
}

/// Append the FTS-only rows to the Nucleo array, keeping the Nucleo order and
/// the visible cap. Rows are identified by their `id`. The secondary rows are
/// moved, not cloned: the FTS payload is discarded after the merge.
pub(crate) fn merge_json_rows(
    primary: &mut serde_json::Value,
    secondary: &mut serde_json::Value,
    limit: usize,
) {
    let Some(primary_rows) = primary.as_array_mut() else {
        return;
    };
    let mut seen: HashSet<String> = primary_rows.iter().filter_map(json_row_id).collect();
    let Some(secondary_rows) = secondary.as_array_mut() else {
        return;
    };
    for row in std::mem::take(secondary_rows) {
        if primary_rows.len() >= limit {
            break;
        }
        if let Some(id) = json_row_id(&row)
            && !seen.insert(id)
        {
            continue;
        }
        primary_rows.push(row);
    }
    primary_rows.truncate(limit);
}

fn json_row_id(row: &serde_json::Value) -> Option<String> {
    row.get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

async fn search_value_fts(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let filter = crate::offline::load(dbs, offline).await?;
    let limit = limit.max(1);
    // Offline hits are filtered down to the downloaded catalog, so over-fetch
    // before truncating back to the rendered rows.
    let fetch = if filter.is_some() { limit * 3 } else { limit };
    // S-9: an empty query is the "Most played" surface, not three dead
    // columns. Punctuation is ignored for matching (`(hello` == `hello`),
    // but the payload keeps the raw text so the field's stale check works.
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
        songs.retain(|song| filter.contains_song(&song.song_id));
    }
    songs.truncate(limit);

    let song_ids: Vec<String> = songs.iter().map(|song| song.song_id.clone()).collect();
    let (song_covers, song_albums) = tokio::try_join!(
        crate::library::song_cover_ids(&dbs.library, &song_ids),
        crate::library::song_album_ids(&dbs.library, &song_ids),
    )?;
    let songs: Vec<SearchSong> = songs
        .into_iter()
        .map(|song| {
            let cover_art = song_covers.get(&song.song_id).cloned();
            let album_id = song_albums.get(&song.song_id).cloned();
            SearchSong {
                id: song.song_id,
                title: song.title,
                artist: song.artist,
                album: song.album,
                album_id,
                track: None,
                duration: song.duration,
                cover_art,
                play_count: song.play_count,
            }
        })
        .collect();

    search_payload_value(
        dbs,
        query,
        most_played,
        artists.into_iter().map(std::sync::Arc::new).collect(),
        albums.into_iter().map(std::sync::Arc::new).collect(),
        songs,
        None,
    )
    .await
}

pub(crate) async fn search_value_index(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let filter = crate::offline::load(dbs, offline).await?;
    let limit = limit.max(1);
    let fetch = if filter.is_some() { limit * 3 } else { limit };
    let cleaned = crate::search::clean_query(query);
    let most_played = cleaned.is_empty();
    let Some(index) = crate::search::index::ensure(&dbs.library).await? else {
        // Over budget: the catalog declined to build, FTS owns the query.
        return search_value_fts(dbs, query, offline, limit).await;
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
                .map(|song| SearchSong::from_index(song))
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
            results
                .songs
                .iter()
                .map(|song| SearchSong::from_index(song))
                .collect(),
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
            .map(|song| SearchSong::from_index(song))
            .collect();
        if let Some(filter) = &filter {
            songs.retain(|song| filter.contains_song(&song.id));
        }
    }
    songs.truncate(limit);

    search_payload_value(
        dbs,
        query,
        most_played,
        artists,
        albums,
        songs,
        album_context,
    )
    .await
}

async fn search_payload_value(
    dbs: &crate::db::Dbs,
    query: &str,
    most_played: bool,
    artists: Vec<std::sync::Arc<crate::library::ArtistRow>>,
    albums: Vec<std::sync::Arc<crate::library::AlbumRow>>,
    songs: Vec<SearchSong>,
    album_context: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    let mut cover_ids: Vec<String> = albums
        .iter()
        .filter_map(|album| album.cover_art.clone())
        .collect();
    cover_ids.extend(artists.iter().filter_map(|artist| artist.cover_art.clone()));
    cover_ids.extend(songs.iter().filter_map(|song| song.cover_art.clone()));
    let covers = crate::covers::url_map_of(dbs, &cover_ids);
    // UI-56: hearts/ratings are live library state, not the index's
    // sync-time snapshot; read them for just this page.
    let song_ids: Vec<String> = songs.iter().map(|song| song.id.clone()).collect();
    let song_starred = crate::library::song_starred_ids(&dbs.library, &song_ids);
    let album_ids: Vec<String> = albums.iter().map(|album| album.id.clone()).collect();
    let album_states = crate::library::album_states(&dbs.library, &album_ids);
    let user_starred = crate::artist_user::starred_ids(&dbs.artist_user);
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user);
    let (covers, song_starred, album_states, user_starred, user_ratings) = tokio::try_join!(
        covers,
        song_starred,
        album_states,
        user_starred,
        user_ratings
    )?;
    let user_starred: std::collections::HashSet<String> = user_starred.into_iter().collect();

    let artist_rows: Vec<serde_json::Value> = artists
        .iter()
        .map(|artist| {
            let starred = artist.starred.is_some() || user_starred.contains(&artist.id);
            let rating = user_ratings.get(&artist.id).copied().unwrap_or(0);
            artist_json(artist, &covers, None, starred, rating)
        })
        .collect();
    let album_rows: Vec<serde_json::Value> = albums
        .iter()
        .map(|album| {
            let mut value = album_json(album, &covers, None);
            if let Some((starred, rating)) = album_states.get(&album.id) {
                value["starred"] = serde_json::json!(starred);
                value["rating"] = serde_json::json!(rating);
            }
            value
        })
        .collect();
    let song_rows: Vec<serde_json::Value> = songs
        .iter()
        .map(|song| {
            let mut value = song_json(
                &song.id,
                &song.title,
                song.artist.as_deref(),
                song.album.as_deref(),
                song.album_id.as_deref(),
                song.track,
                song.duration,
                None,
                crate::covers::url_for(song.cover_art.as_deref(), &covers),
                song_starred.contains(&song.id),
            );
            value["play_count"] = serde_json::json!(song.play_count);
            value
        })
        .collect();

    Ok(serde_json::json!({
        "query": query,
        "most_played": most_played,
        "album_context": album_context,
        "artists": artist_rows,
        "albums": album_rows,
        "songs": song_rows,
    }))
}
