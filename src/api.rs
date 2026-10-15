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

//! Typed wrappers for the [Subsonic REST API](https://www.subsonic.org/pages/api.jsp)
//! (protocol version 1.16.1), trimmed to the endpoints the tree actually
//! calls (ARC-7): browsing (`getIndexes`/`getArtist`/`getAlbum`, artist and
//! album info), lyrics (`getLyrics`, `getLyricsBySongId`, the extension
//! probe), favourites/ratings (`star`/`unstar`/`setRating`), `ping` and the
//! `stream` URL builder. Everything else was deleted with its response types;
//! the module carries no `allow(dead_code)` and `cargo check` is
//! warning-free.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::http_client::SubsonicClient;

// ---------------------------------------------------------------------------
// Deserialization helpers
// ---------------------------------------------------------------------------

/// Accepts both a single object and an array of objects for list fields.
fn one_or_many<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }

    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    })
}

/// Builds the query-pairs slice, skipping absent (`None`) options.
fn query(pairs: &[(&'static str, Option<String>)]) -> Vec<(&'static str, String)> {
    pairs
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|value| (*key, value.clone())))
        .collect()
}

/// Pulls `key` out of a response body and deserializes it, defaulting to an
/// empty model when the element is absent.
fn extract<T: DeserializeOwned + Default>(body: Value, key: &str) -> Result<T> {
    Ok(extract_opt(body, key)?.unwrap_or_default())
}

fn extract_opt<T: DeserializeOwned + Default>(body: Value, key: &str) -> Result<Option<T>> {
    match body.get(key) {
        Some(value) if !value.is_null() => serde_json::from_value(value.clone())
            .with_context(|| format!("malformed `{key}` in response"))
            .map(Some),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Response models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Indexes {
    #[serde(deserialize_with = "one_or_many")]
    pub shortcut: Vec<Artist>,
    #[serde(deserialize_with = "one_or_many")]
    pub index: Vec<Index>,
    #[serde(deserialize_with = "one_or_many")]
    pub child: Vec<Child>,
    pub last_modified: i64,
    pub ignored_articles: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Index {
    pub name: String,
    #[serde(deserialize_with = "one_or_many")]
    pub artist: Vec<Artist>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub artist_image_url: Option<String>,
    pub starred: Option<String>,
    pub user_rating: Option<i32>,
    pub average_rating: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistID3 {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    pub artist_image_url: Option<String>,
    pub album_count: i32,
    pub starred: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistWithAlbumsID3 {
    #[serde(flatten)]
    pub artist: ArtistID3,
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<AlbumID3>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumID3 {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub cover_art: Option<String>,
    pub song_count: i32,
    pub duration: i32,
    pub play_count: Option<i64>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub year: Option<i32>,
    pub genre: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumWithSongsID3 {
    #[serde(flatten)]
    pub album: AlbumID3,
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

/// The `playlist` element of `getPlaylists`/`getPlaylist` (ARC-44). The
/// dates are the server's ISO-8601 `changed`, used for the staleness check.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaylistID3 {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub song_count: Option<i64>,
    pub duration: Option<i64>,
    pub created: Option<String>,
    pub changed: Option<String>,
    pub cover_art: Option<String>,
}

/// The `playlists` wrapper of `getPlaylists`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlists {
    #[serde(deserialize_with = "one_or_many")]
    pub playlist: Vec<PlaylistID3>,
}

/// `getPlaylist`: the element plus its ordered entries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaylistWithSongs {
    #[serde(flatten)]
    pub playlist: PlaylistID3,
    #[serde(deserialize_with = "one_or_many")]
    pub entry: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Child {
    pub id: String,
    pub parent: Option<String>,
    pub is_dir: bool,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub track: Option<i32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub size: Option<i64>,
    pub content_type: Option<String>,
    pub suffix: Option<String>,
    pub transcoded_content_type: Option<String>,
    pub transcoded_suffix: Option<String>,
    pub duration: Option<i32>,
    pub bit_rate: Option<i32>,
    pub path: Option<String>,
    pub is_video: Option<bool>,
    pub user_rating: Option<i32>,
    pub average_rating: Option<f64>,
    pub play_count: Option<i64>,
    pub disc_number: Option<i32>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
    pub bookmark_position: Option<i64>,
    pub original_width: Option<i32>,
    pub original_height: Option<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Lyrics {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub value: String,
}

/// One entry of the OpenSubsonic `songLyrics` extension
/// (`getLyricsBySongId`): per-song lyrics, optionally time-synced.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StructuredLyrics {
    pub display_artist: Option<String>,
    pub display_title: Option<String>,
    pub lang: Option<String>,
    /// `true` when every `line` carries a `start` timestamp.
    pub synced: bool,
    /// Milliseconds added to every `start` (OpenSubsonic `offset`).
    pub offset: Option<i64>,
    #[serde(deserialize_with = "one_or_many")]
    pub line: Vec<LyricLine>,
}

/// A single lyric line; `start` is present for synced lyrics.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LyricLine {
    pub start: Option<i64>,
    pub value: String,
}

/// Response payload of `getLyricsBySongId`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LyricsList {
    #[serde(deserialize_with = "one_or_many")]
    pub structured_lyrics: Vec<StructuredLyrics>,
}

/// An OpenSubsonic API extension advertised by `getOpenSubsonicExtensions`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OpenSubsonicExtension {
    pub name: String,
    pub versions: Vec<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumInfo {
    pub notes: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    pub small_image_url: Option<String>,
    pub medium_image_url: Option<String>,
    pub large_image_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistInfoBase {
    pub biography: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    pub small_image_url: Option<String>,
    pub medium_image_url: Option<String>,
    pub large_image_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistInfo {
    #[serde(flatten)]
    pub info: ArtistInfoBase,
    #[serde(deserialize_with = "one_or_many")]
    pub similar_artist: Vec<Artist>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistInfo2 {
    #[serde(flatten)]
    pub info: ArtistInfoBase,
    #[serde(deserialize_with = "one_or_many")]
    pub similar_artist: Vec<ArtistID3>,
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

impl SubsonicClient {
    /// `ping` — checks connectivity and credentials; reports what the server
    /// says it is (the envelope's `type`/`serverVersion`).
    pub async fn ping(&self) -> Result<crate::http_client::ServerInfo> {
        let body = self.call::<Value>("ping", &[]).await?;
        Ok(crate::http_client::server_info(&body))
    }

    // -----------------------------------------------------------------------
    // Browsing
    // -----------------------------------------------------------------------

    /// `getIndexes` — artists organized by file structure.
    pub async fn get_indexes(
        &self,
        music_folder_id: Option<&str>,
        if_modified_since: Option<i64>,
    ) -> Result<Indexes> {
        let params = query(&[
            ("musicFolderId", music_folder_id.map(str::to_string)),
            ("ifModifiedSince", if_modified_since.map(|v| v.to_string())),
        ]);
        let body = self.call::<Value>("getIndexes", &params).await?;
        extract(body, "indexes")
    }

    /// `getPlaylists` — the playlists the user may play. Never filter by
    /// `username`: Navidrome ignores it (ARC-44/P-4).
    pub async fn get_playlists(&self) -> Result<Vec<PlaylistID3>> {
        let body = self.call::<Value>("getPlaylists", &[]).await?;
        let lists: Playlists = extract(body, "playlists")?;
        Ok(lists.playlist)
    }

    /// `getPlaylist` — the playlist plus its ordered `entry[]`. Duplicates
    /// and positions are preserved; full Child metadata feeds the page.
    pub async fn get_playlist(&self, id: &str) -> Result<PlaylistWithSongs> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getPlaylist", &params).await?;
        extract(body, "playlist")
    }

    /// `getArtist` — artist details including albums.
    pub async fn get_artist(&self, id: &str) -> Result<ArtistWithAlbumsID3> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getArtist", &params).await?;
        extract(body, "artist")
    }

    /// `getAlbum` — album details including songs.
    pub async fn get_album(&self, id: &str) -> Result<AlbumWithSongsID3> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getAlbum", &params).await?;
        extract(body, "album")
    }

    /// `getArtistInfo` — biography, images and similar artists (last.fm).
    pub async fn get_artist_info(
        &self,
        id: &str,
        count: Option<u32>,
        include_not_present: Option<bool>,
    ) -> Result<ArtistInfo> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("count", count.map(|v| v.to_string())),
            (
                "includeNotPresent",
                include_not_present.map(|v| v.to_string()),
            ),
        ]);
        let body = self.call::<Value>("getArtistInfo", &params).await?;
        extract(body, "artistInfo")
    }

    /// `getArtistInfo2` — like `getArtistInfo`, organized by ID3 tags.
    pub async fn get_artist_info2(
        &self,
        id: &str,
        count: Option<u32>,
        include_not_present: Option<bool>,
    ) -> Result<ArtistInfo2> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("count", count.map(|v| v.to_string())),
            (
                "includeNotPresent",
                include_not_present.map(|v| v.to_string()),
            ),
        ]);
        let body = self.call::<Value>("getArtistInfo2", &params).await?;
        extract(body, "artistInfo2")
    }

    /// `getAlbumInfo2` — like `getAlbumInfo`, organized by ID3 tags.
    pub async fn get_album_info2(&self, id: &str) -> Result<AlbumInfo> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getAlbumInfo2", &params).await?;
        extract(body, "albumInfo")
    }

    // -----------------------------------------------------------------------
    // Media retrieval (URL builders)
    // -----------------------------------------------------------------------

    /// `stream` — URL to stream a media file (transcoded as requested).
    pub fn stream_url(&self, id: &str) -> Result<reqwest::Url> {
        self.endpoint_url("stream", &[("id", id.to_string())])
    }

    /// `getLyrics` — lyrics for a song (empty if not found).
    pub async fn get_lyrics(&self, artist: Option<&str>, title: Option<&str>) -> Result<Lyrics> {
        let params = query(&[
            ("artist", artist.map(str::to_string)),
            ("title", title.map(str::to_string)),
        ]);
        let body = self.call::<Value>("getLyrics", &params).await?;
        extract(body, "lyrics")
    }

    /// `getLyricsBySongId` — OpenSubsonic `songLyrics` extension: per-song and
    /// optionally synced lyrics.
    pub async fn get_lyrics_by_song_id(&self, id: &str) -> Result<LyricsList> {
        let params = [("id", id.to_string())];
        let body = self.call::<Value>("getLyricsBySongId", &params).await?;
        extract(body, "lyricsList")
    }

    /// `getOpenSubsonicExtensions` — the extensions this server advertises, used
    /// to decide whether `getLyricsBySongId` is worth calling.
    pub async fn get_open_subsonic_extensions(&self) -> Result<Vec<OpenSubsonicExtension>> {
        let body = self.call::<Value>("getOpenSubsonicExtensions", &[]).await?;
        extract(body, "openSubsonicExtensions")
    }

    // -----------------------------------------------------------------------
    // Media annotation
    // -----------------------------------------------------------------------

    /// `star` — stars songs, albums and/or artists.
    pub async fn star(&self, ids: &[&str], album_ids: &[&str], artist_ids: &[&str]) -> Result<()> {
        self.call::<Value>("star", &star_params(ids, album_ids, artist_ids))
            .await
            .map(|_| ())
    }

    /// `unstar` — removes stars from songs, albums and/or artists.
    pub async fn unstar(
        &self,
        ids: &[&str],
        album_ids: &[&str],
        artist_ids: &[&str],
    ) -> Result<()> {
        self.call::<Value>("unstar", &star_params(ids, album_ids, artist_ids))
            .await
            .map(|_| ())
    }

    /// `setRating` — sets a rating (1-5, or 0 to remove).
    pub async fn set_rating(&self, id: &str, rating: u8) -> Result<()> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("rating", Some(rating.to_string())),
        ]);
        self.call::<Value>("setRating", &params).await.map(|_| ())
    }
}

fn star_params<'a>(
    ids: &'a [&'a str],
    album_ids: &'a [&'a str],
    artist_ids: &'a [&'a str],
) -> Vec<(&'static str, String)> {
    let mut params: Vec<(&'static str, String)> = Vec::new();
    for id in ids {
        params.push(("id", (*id).to_string()));
    }
    for id in album_ids {
        params.push(("albumId", (*id).to_string()));
    }
    for id in artist_ids {
        params.push(("artistId", (*id).to_string()));
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_object_into_list() {
        let body: Value = serde_json::from_str(
            r#"{"indexes":{"ignoredArticles":"The","lastModified":1,"index":{"name":"A","artist":{"id":"1","name":"AC/DC"}}}}"#,
        )
        .unwrap();

        let indexes: Indexes = extract(body, "indexes").unwrap();
        assert_eq!(indexes.index.len(), 1);
        assert_eq!(indexes.index[0].artist.len(), 1);
        assert_eq!(indexes.index[0].artist[0].name, "AC/DC");
    }

    #[test]
    fn parses_song_type_field_and_missing_fields() {
        let body: Value = serde_json::from_str(
            r#"{"song":{"id":"42","isDir":false,"title":"Hello","artist":"Adele","type":"music"}}"#,
        )
        .unwrap();

        let song: Child = extract(body, "song").unwrap();
        assert_eq!(song.id, "42");
        assert_eq!(song.title, "Hello");
        assert_eq!(song.media_type.as_deref(), Some("music"));
        assert_eq!(song.duration, None);
    }

    #[test]
    fn absent_element_yields_default() {
        let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
        let lyrics: Lyrics = extract(body, "lyrics").unwrap();
        assert!(lyrics.value.is_empty());
    }

    #[test]
    fn album_with_songs_flattens_album_fields() {
        let body: Value = serde_json::from_str(
            r#"{"album":{"id":"a1","name":"Album","songCount":1,"duration":30,"created":"2020-01-01T00:00:00Z","song":[{"id":"s1","isDir":false,"title":"Track"}]}}"#,
        )
        .unwrap();

        let album: AlbumWithSongsID3 = extract(body, "album").unwrap();
        assert_eq!(album.album.name, "Album");
        assert_eq!(album.song.len(), 1);
        assert_eq!(album.song[0].title, "Track");
    }

    #[test]
    fn parses_playlists_and_entries() {
        // A single playlist object becomes a one-element list; fields map.
        let body: Value = serde_json::from_str(
            r#"{"playlists":{"playlist":{"id":"p1","name":"Mix","songCount":2,"duration":90,"owner":"me","public":true,"changed":"2026-01-01T00:00:00Z"}}}"#,
        )
        .unwrap();
        let lists: Playlists = extract(body, "playlists").unwrap();
        assert_eq!(lists.playlist.len(), 1);
        assert_eq!(lists.playlist[0].name, "Mix");
        assert_eq!(lists.playlist[0].song_count, Some(2));
        assert_eq!(lists.playlist[0].public, Some(true));

        // The entries flatten into the playlist and keep their order.
        let body: Value = serde_json::from_str(
            r#"{"playlist":{"id":"p1","name":"Mix","songCount":2,"duration":90,"entry":[{"id":"s1","isDir":false,"title":"A"},{"id":"s2","isDir":false,"title":"B"}]}}"#,
        )
        .unwrap();
        let page: PlaylistWithSongs = extract(body, "playlist").unwrap();
        assert_eq!(page.playlist.name, "Mix");
        assert_eq!(page.entry.len(), 2);
        assert_eq!(page.entry[0].title, "A");

        // An absent element defaults to empty, never errors.
        let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
        let lists: Playlists = extract(body, "playlists").unwrap();
        assert!(lists.playlist.is_empty());
    }

    #[test]
    fn query_skips_none() {
        let params = query(&[("a", Some("1".to_string())), ("b", None)]);
        assert_eq!(params, vec![("a", "1".to_string())]);
    }
}
