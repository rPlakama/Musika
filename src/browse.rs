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

//! Off-thread list/search loaders for the Qt bridge: `library.db` /
//! `search.db` rows -> JSON payloads with resolved `file://` cover URLs.
//!
//! Split out of `qml.rs` (ARC-17); playback queues live in [`crate::queue`].

use std::collections::HashMap;

use crate::http_client::SubsonicClient;

pub async fn load_albums_json(
    dbs: &crate::db::Dbs,
    artist: &str,
    order: &str,
) -> anyhow::Result<String> {
    let filter = (!artist.is_empty()).then_some(artist);
    let albums = crate::library::albums(&dbs.library, filter, order).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    let rows: Vec<serde_json::Value> = albums
        .iter()
        .map(|album| album_json(album, &covers))
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

pub async fn load_songs_json(dbs: &crate::db::Dbs, order: &str) -> anyhow::Result<String> {
    let songs = crate::library::all_songs(&dbs.library, order).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    song_rows_json(&songs, &covers)
}

pub async fn load_album_songs_json(dbs: &crate::db::Dbs, album_id: &str) -> anyhow::Result<String> {
    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    song_rows_json(&songs, &covers)
}

pub async fn load_artists_json(dbs: &crate::db::Dbs, order: &str) -> anyhow::Result<String> {
    let artists = crate::library::artists(&dbs.library, order).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    let rows: Vec<serde_json::Value> = artists
        .iter()
        .map(|artist| artist_json(artist, &covers))
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

/// One artist (library.db) with cover URL and its albums, for ArtistEntry.
pub async fn load_artist_json(
    dbs: &crate::db::Dbs,
    id: &str,
    order: &str,
) -> anyhow::Result<Option<String>> {
    let Some(artist) = crate::library::artist(&dbs.library, id).await? else {
        return Ok(None);
    };
    let albums = crate::library::artist_albums(&dbs.library, id, order).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    let mut value = artist_json(&artist, &covers);
    value["albums"] = serde_json::Value::Array(
        albums
            .iter()
            .map(|album| album_json(album, &covers))
            .collect(),
    );
    Ok(Some(value.to_string()))
}

fn artist_json(
    artist: &crate::library::ArtistRow,
    covers: &HashMap<String, String>,
) -> serde_json::Value {
    let cover = cover_url(artist.cover_art.as_deref(), covers);
    serde_json::json!({
        "id": artist.id,
        "name": artist.name,
        "album_count": artist.album_count,
        "cover": cover,
    })
}

fn album_json(
    album: &crate::library::AlbumRow,
    covers: &HashMap<String, String>,
) -> serde_json::Value {
    let cover = cover_url(album.cover_art.as_deref(), covers);
    serde_json::json!({
        "id": album.id,
        "title": album.title,
        "artist": album.artist,
        "year": album.year,
        "song_count": album.song_count,
        "cover": cover,
    })
}

fn song_rows_json(
    songs: &[crate::library::SongRow],
    covers: &HashMap<String, String>,
) -> anyhow::Result<String> {
    let rows: Vec<serde_json::Value> = songs
        .iter()
        .map(|song| {
            let cover = cover_url(song.cover_art.as_deref(), covers);
            serde_json::json!({
                "id": song.id,
                "title": song.title,
                "artist": song.artist,
                "album": song.album,
                "track": song.track,
                "duration": song.duration,
                "cover": cover,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

/// Fetches an album's description text from the server (`getAlbumInfo2`).
pub async fn album_notes(dbs: &crate::db::Dbs, album_id: &str) -> String {
    let Ok(Some(credentials)) = crate::sqlite::load_credentials(&dbs.credentials).await else {
        return String::new();
    };
    let client = SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    );
    match client.get_album_info2(album_id).await {
        Ok(info) => info.notes.unwrap_or_default(),
        Err(err) => {
            eprintln!("album: could not fetch info for {album_id}: {err:#}");
            String::new()
        }
    }
}

fn cover_url(id: Option<&str>, covers: &HashMap<String, String>) -> String {
    id.and_then(|id| covers.get(id))
        .cloned()
        .unwrap_or_default()
}

pub async fn search_json(dbs: &crate::db::Dbs, query: &str) -> anyhow::Result<String> {
    // S-9: an empty query is the "Most played" surface, not three dead
    // columns.
    let most_played = query.trim().is_empty();
    let (artists, albums, songs) = if most_played {
        (
            Vec::new(),
            Vec::new(),
            crate::search::most_played(&dbs.search, 20).await?,
        )
    } else {
        let (artists, albums) =
            crate::library::search_artists_albums(&dbs.library, query, 20).await?;
        let songs = crate::search::search(&dbs.search, query, 20).await?;
        (artists, albums, songs)
    };
    let covers = crate::covers::url_map(&dbs.covers).await?;

    let song_ids: Vec<String> = songs.iter().map(|song| song.song_id.clone()).collect();
    let song_covers = crate::library::song_cover_ids(&dbs.library, &song_ids).await?;

    let artist_rows: Vec<serde_json::Value> = artists
        .into_iter()
        .map(|artist| {
            serde_json::json!({
                "id": artist.id,
                "name": artist.name,
                "album_count": artist.album_count,
                "cover": cover_url(artist.cover_art.as_deref(), &covers),
            })
        })
        .collect();
    let album_rows: Vec<serde_json::Value> = albums
        .into_iter()
        .map(|album| {
            serde_json::json!({
                "id": album.id,
                "title": album.title,
                "artist": album.artist,
                "year": album.year,
                "song_count": album.song_count,
                "cover": cover_url(album.cover_art.as_deref(), &covers),
            })
        })
        .collect();
    let song_rows: Vec<serde_json::Value> = songs
        .iter()
        .map(|song| {
            let cover = song_covers
                .get(&song.song_id)
                .and_then(|cover_art| covers.get(cover_art))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "id": song.song_id,
                "title": song.title,
                "artist": song.artist,
                "album": song.album,
                "duration": song.duration,
                "play_count": song.play_count,
                "cover": cover,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "query": query,
        "most_played": most_played,
        "artists": artist_rows,
        "albums": album_rows,
        "songs": song_rows,
    })
    .to_string())
}
