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

mod rows;
mod search;

pub(crate) use rows::*;
pub use search::search_json;
#[cfg(test)]
pub(crate) use search::{merge_json_rows, search_value_index};

pub async fn load_albums_json(
    dbs: &crate::db::Dbs,
    artist: &str,
    order: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let filter = (!artist.is_empty()).then_some(artist);
    let mut albums = crate::library::albums(&dbs.library, filter, order).await?;
    let offline = crate::offline::load(dbs, offline).await?;
    if let Some(offline) = &offline {
        albums.retain(|album| offline.contains_album(&album.id));
    }
    let covers =
        crate::covers::url_map_for(dbs, albums.iter().map(|album| &album.cover_art)).await?;
    // Stream the sequence straight into the payload string: the old
    // `Vec<Value>` built one object/map per row (with its own key and value
    // allocations) before serializing it a second time.
    Ok(serde_json::to_string(&AlbumSeq {
        albums: &albums,
        covers: &covers,
        offline: offline.as_deref(),
    })?)
}

pub async fn load_songs_page_json(
    dbs: &crate::db::Dbs,
    offset: i64,
    limit: i64,
    order: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = crate::library::songs_range(&dbs.library, offset, limit + 1, order).await?;
    let has_more = songs.len() as i64 > limit;
    songs.truncate(limit.max(0) as usize);
    let returned = songs.len() as i64;
    if let Some(filter) = crate::offline::load(dbs, offline).await? {
        songs.retain(|song| filter.contains_song(&song.id));
    }
    let covers = crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
    Ok(serde_json::to_string(&SongsPage {
        offset,
        next: offset + returned,
        has_more,
        songs: &songs,
        covers: &covers,
    })?)
}

pub async fn load_album_songs_json(
    dbs: &crate::db::Dbs,
    album_id: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = crate::library::album_songs(&dbs.library, album_id).await?;
    if let Some(filter) = crate::offline::load(dbs, offline).await? {
        songs.retain(|song| filter.contains_song(&song.id));
    }
    let covers = crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
    song_rows_json(&songs, &covers)
}

pub async fn load_artists_json(
    dbs: &crate::db::Dbs,
    order: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut artists = crate::library::artists(&dbs.library, order).await?;
    let filter = crate::offline::load(dbs, offline).await?;
    if let Some(filter) = &filter {
        artists.retain(|artist| filter.contains_artist(&artist.id));
    }
    let covers =
        crate::covers::url_map_for(dbs, artists.iter().map(|artist| &artist.cover_art)).await?;
    // UI-56: the artist favourite is the server flag ORed with the local
    // `artist_user` overlay (ARC-42), so both surfaces agree.
    let user_starred: std::collections::HashSet<String> =
        crate::artist_user::starred_ids(&dbs.artist_user)
            .await?
            .into_iter()
            .collect();
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
    Ok(serde_json::to_string(&ArtistSeq {
        artists: &artists,
        covers: &covers,
        offline: filter.as_deref(),
        user_starred: &user_starred,
        user_ratings: &user_ratings,
    })?)
}

/// One artist (library.db) with cover URL and its albums, for ArtistEntry.
pub async fn load_artist_json(
    dbs: &crate::db::Dbs,
    id: &str,
    order: &str,
    offline: bool,
) -> anyhow::Result<Option<String>> {
    let Some(artist) = crate::library::artist(&dbs.library, id).await? else {
        return Ok(None);
    };
    let mut albums = crate::library::artist_albums(&dbs.library, id, order).await?;
    let filter = crate::offline::load(dbs, offline).await?;
    if let Some(filter) = &filter {
        albums.retain(|album| filter.contains_album(&album.id));
    }
    let covers = crate::covers::url_map_for(
        dbs,
        std::iter::once(&artist.cover_art).chain(albums.iter().map(|album| &album.cover_art)),
    )
    .await?;
    let user_starred: std::collections::HashSet<String> =
        crate::artist_user::starred_ids(&dbs.artist_user)
            .await?
            .into_iter()
            .collect();
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
    let album_count = filter
        .as_ref()
        .and_then(|filter| filter.artist_count(&artist.id))
        .unwrap_or(artist.album_count);
    let page = ArtistPage {
        artist: &artist,
        cover: crate::covers::url_for_ref(artist.cover_art.as_deref(), &covers),
        album_count,
        starred: artist.starred.is_some() || user_starred.contains(&artist.id),
        rating: user_ratings.get(&artist.id).copied().unwrap_or(0),
        albums: AlbumSeq {
            albums: &albums,
            covers: &covers,
            offline: filter.as_deref(),
        },
    };
    Ok(Some(serde_json::to_string(&page)?))
}

pub async fn album_notes(dbs: &crate::db::Dbs, album_id: &str) -> String {
    let Ok(client) = crate::session::client(dbs).await else {
        return String::new();
    };
    match client.get_album_info2(album_id).await {
        Ok(info) => info.notes.unwrap_or_default(),
        Err(err) => {
            eprintln!("album: could not fetch info for {album_id}: {err:#}");
            String::new()
        }
    }
}
