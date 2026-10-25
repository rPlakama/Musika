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

//! Streaming row serializers for the list/search payloads.

use std::collections::{HashMap, HashSet};

use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};

pub(crate) fn artist_json(
    artist: &crate::library::ArtistRow,
    covers: &HashMap<String, String>,
    album_count: Option<i64>,
    starred: bool,
    rating: i64,
) -> serde_json::Value {
    let cover = crate::covers::url_for(artist.cover_art.as_deref(), covers);
    serde_json::json!({
        "id": artist.id,
        "name": artist.name,
        "album_count": album_count.unwrap_or(artist.album_count),
        "cover": cover,
        // UI-56: server starred OR the local `artist_user` overlay.
        "starred": starred,
        // UI-60: the local `artist_user` rating (0 = unrated).
        "rating": rating,
    })
}

pub(crate) fn album_json(
    album: &crate::library::AlbumRow,
    covers: &HashMap<String, String>,
    song_count: Option<i64>,
) -> serde_json::Value {
    let cover = crate::covers::url_for(album.cover_art.as_deref(), covers);
    let count = song_count.unwrap_or(album.song_count);
    serde_json::json!({
        "id": album.id,
        "title": album.title,
        "artist": album.artist,
        "year": album.year,
        "song_count": count,
        "tracks": count,
        "cover": cover,
        "starred": album.starred.is_some(),
        // UI-60: user rating 0..=5 (0 = unrated).
        "rating": album.rating,
    })
}

pub(crate) fn song_json(
    id: &str,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    album_id: Option<&str>,
    track: Option<i32>,
    duration: Option<i32>,
    suffix: Option<&str>,
    cover: String,
    starred: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "artist": artist,
        "album": album,
        "album_id": album_id,
        "track": track,
        "duration": duration,
        "suffix": suffix,
        "cover": cover,
        "starred": starred,
    })
}

// ---------------------------------------------------------------------------
// Streaming list serializers
// ---------------------------------------------------------------------------
//
// The album/artist/song list loaders used to build a `Vec<serde_json::Value>`
// and serialize it afterwards. Every row then existed as a tree of maps,
// boxed keys and cloned strings before the final string was written. These
// borrow-based `Serialize` views emit the same JSON straight into the output
// buffer, so a list page costs the row vector + the final string.

struct AlbumElement<'a> {
    album: &'a crate::library::AlbumRow,
    cover: &'a str,
    song_count: i64,
}

impl Serialize for AlbumElement<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("album", 9)?;
        state.serialize_field("id", &self.album.id)?;
        state.serialize_field("title", &self.album.title)?;
        state.serialize_field("artist", &self.album.artist)?;
        state.serialize_field("year", &self.album.year)?;
        state.serialize_field("song_count", &self.song_count)?;
        state.serialize_field("tracks", &self.song_count)?;
        state.serialize_field("cover", self.cover)?;
        state.serialize_field("starred", &self.album.starred.is_some())?;
        state.serialize_field("rating", &self.album.rating)?;
        state.end()
    }
}

/// An album list (or an artist page's `albums` array) streamed row by row.
pub(crate) struct AlbumSeq<'a> {
    pub(crate) albums: &'a [crate::library::AlbumRow],
    pub(crate) covers: &'a HashMap<String, String>,
    /// Offline track counts by album id, when the offline filter applies.
    pub(crate) offline: Option<&'a crate::offline::Filter>,
}

impl Serialize for AlbumSeq<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.albums.len()))?;
        for album in self.albums {
            let song_count = self
                .offline
                .and_then(|filter| filter.album_count(&album.id))
                .unwrap_or(album.song_count);
            seq.serialize_element(&AlbumElement {
                album,
                cover: crate::covers::url_for_ref(album.cover_art.as_deref(), self.covers),
                song_count,
            })?;
        }
        seq.end()
    }
}

struct ArtistElement<'a> {
    artist: &'a crate::library::ArtistRow,
    cover: &'a str,
    album_count: i64,
    starred: bool,
    rating: i64,
}

impl Serialize for ArtistElement<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("artist", 6)?;
        state.serialize_field("id", &self.artist.id)?;
        state.serialize_field("name", &self.artist.name)?;
        state.serialize_field("album_count", &self.album_count)?;
        state.serialize_field("cover", self.cover)?;
        state.serialize_field("starred", &self.starred)?;
        state.serialize_field("rating", &self.rating)?;
        state.end()
    }
}

pub(crate) struct ArtistSeq<'a> {
    pub(crate) artists: &'a [crate::library::ArtistRow],
    pub(crate) covers: &'a HashMap<String, String>,
    pub(crate) offline: Option<&'a crate::offline::Filter>,
    pub(crate) user_starred: &'a HashSet<String>,
    pub(crate) user_ratings: &'a HashMap<String, i64>,
}

impl Serialize for ArtistSeq<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.artists.len()))?;
        for artist in self.artists {
            let album_count = self
                .offline
                .and_then(|filter| filter.artist_count(&artist.id))
                .unwrap_or(artist.album_count);
            seq.serialize_element(&ArtistElement {
                artist,
                cover: crate::covers::url_for_ref(artist.cover_art.as_deref(), self.covers),
                album_count,
                starred: artist.starred.is_some() || self.user_starred.contains(&artist.id),
                rating: self.user_ratings.get(&artist.id).copied().unwrap_or(0),
            })?;
        }
        seq.end()
    }
}

/// One artist page: the artist fields plus its `albums` array.
pub(crate) struct ArtistPage<'a> {
    pub(crate) artist: &'a crate::library::ArtistRow,
    pub(crate) cover: &'a str,
    pub(crate) album_count: i64,
    pub(crate) starred: bool,
    pub(crate) rating: i64,
    pub(crate) albums: AlbumSeq<'a>,
}

impl Serialize for ArtistPage<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("artist_page", 7)?;
        state.serialize_field("id", &self.artist.id)?;
        state.serialize_field("name", &self.artist.name)?;
        state.serialize_field("album_count", &self.album_count)?;
        state.serialize_field("cover", self.cover)?;
        state.serialize_field("starred", &self.starred)?;
        state.serialize_field("rating", &self.rating)?;
        state.serialize_field("albums", &self.albums)?;
        state.end()
    }
}

struct SongElement<'a> {
    song: &'a crate::library::SongRow,
    cover: &'a str,
}

impl Serialize for SongElement<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("song", 10)?;
        state.serialize_field("id", &self.song.id)?;
        state.serialize_field("title", &self.song.title)?;
        state.serialize_field("artist", &self.song.artist)?;
        state.serialize_field("album", &self.song.album)?;
        state.serialize_field("album_id", &self.song.album_id)?;
        state.serialize_field("track", &self.song.track)?;
        state.serialize_field("duration", &self.song.duration)?;
        state.serialize_field("suffix", &self.song.suffix)?;
        state.serialize_field("cover", self.cover)?;
        state.serialize_field("starred", &self.song.starred.is_some())?;
        state.end()
    }
}

struct SongSeq<'a> {
    songs: &'a [crate::library::SongRow],
    covers: &'a HashMap<String, String>,
}

impl Serialize for SongSeq<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.songs.len()))?;
        for song in self.songs {
            seq.serialize_element(&SongElement {
                song,
                cover: crate::covers::url_for_ref(song.cover_art.as_deref(), self.covers),
            })?;
        }
        seq.end()
    }
}

/// The paged Songs payload (`rows` is the streamed SongSeq).
pub(crate) struct SongsPage<'a> {
    pub(crate) offset: i64,
    pub(crate) next: i64,
    pub(crate) has_more: bool,
    pub(crate) songs: &'a [crate::library::SongRow],
    pub(crate) covers: &'a HashMap<String, String>,
}

impl Serialize for SongsPage<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("songs_page", 4)?;
        state.serialize_field("offset", &self.offset)?;
        state.serialize_field("next", &self.next)?;
        state.serialize_field("has_more", &self.has_more)?;
        state.serialize_field(
            "rows",
            &SongSeq {
                songs: self.songs,
                covers: self.covers,
            },
        )?;
        state.end()
    }
}

pub(crate) fn song_rows_json(
    songs: &[crate::library::SongRow],
    covers: &HashMap<String, String>,
) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&SongSeq { songs, covers })?)
}
