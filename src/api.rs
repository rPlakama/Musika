//! Typed wrappers for the [Subsonic REST API](https://www.subsonic.org/pages/api.jsp)
//! (protocol version 1.16.1).
//!
//! Every method is implemented on [`SubsonicClient`] and takes care of the
//! required and optional parameters. Core response types are modeled as
//! `Deserialize` structs; the long tail returns [`serde_json::Value`].

#![allow(dead_code)] // Full API surface; callers pick what they need.

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

/// Accepts a JSON string or number and normalizes it to `String`.
fn string_or_number<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.map(|value| match value {
        Value::String(text) => text,
        other => other.to_string(),
    }))
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
// Parameter enums
// ---------------------------------------------------------------------------

/// `type` parameter shared by `getAlbumList` / `getAlbumList2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumListType {
    Random,
    Newest,
    Highest,
    Frequent,
    Recent,
    AlphabeticalByName,
    AlphabeticalByArtist,
    Starred,
    ByYear,
    ByGenre,
}

impl AlbumListType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Newest => "newest",
            Self::Highest => "highest",
            Self::Frequent => "frequent",
            Self::Recent => "recent",
            Self::AlphabeticalByName => "alphabeticalByName",
            Self::AlphabeticalByArtist => "alphabeticalByArtist",
            Self::Starred => "starred",
            Self::ByYear => "byYear",
            Self::ByGenre => "byGenre",
        }
    }
}

/// `action` parameter of `jukeboxControl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JukeboxAction {
    Get,
    Status,
    Set,
    Start,
    Stop,
    Skip,
    Add,
    Clear,
    Remove,
    Shuffle,
    SetGain,
}

impl JukeboxAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Status => "status",
            Self::Set => "set",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Skip => "skip",
            Self::Add => "add",
            Self::Clear => "clear",
            Self::Remove => "remove",
            Self::Shuffle => "shuffle",
            Self::SetGain => "setGain",
        }
    }
}

/// Role flags shared by `createUser` / `updateUser`. `None` leaves the flag
/// untouched (only meaningful for `updateUser`).
#[derive(Debug, Clone, Default)]
pub struct UserRoles {
    pub ldap_authenticated: Option<bool>,
    pub admin_role: Option<bool>,
    pub settings_role: Option<bool>,
    pub stream_role: Option<bool>,
    pub jukebox_role: Option<bool>,
    pub download_role: Option<bool>,
    pub upload_role: Option<bool>,
    pub playlist_role: Option<bool>,
    pub cover_art_role: Option<bool>,
    pub comment_role: Option<bool>,
    pub podcast_role: Option<bool>,
    pub share_role: Option<bool>,
    pub video_conversion_role: Option<bool>,
    pub max_bit_rate: Option<u32>,
    pub music_folder_ids: Vec<i32>,
}

impl UserRoles {
    fn append(&self, params: &mut Vec<(&'static str, String)>) {
        for (key, value) in [
            ("ldapAuthenticated", self.ldap_authenticated),
            ("adminRole", self.admin_role),
            ("settingsRole", self.settings_role),
            ("streamRole", self.stream_role),
            ("jukeboxRole", self.jukebox_role),
            ("downloadRole", self.download_role),
            ("uploadRole", self.upload_role),
            ("playlistRole", self.playlist_role),
            ("coverArtRole", self.cover_art_role),
            ("commentRole", self.comment_role),
            ("podcastRole", self.podcast_role),
            ("shareRole", self.share_role),
            ("videoConversionRole", self.video_conversion_role),
        ] {
            if let Some(flag) = value {
                params.push((key, flag.to_string()));
            }
        }
        if let Some(bit_rate) = self.max_bit_rate {
            params.push(("maxBitRate", bit_rate.to_string()));
        }
        for folder in &self.music_folder_ids {
            params.push(("musicFolderId", folder.to_string()));
        }
    }
}

// ---------------------------------------------------------------------------
// Response models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct License {
    pub valid: bool,
    pub email: Option<String>,
    pub license_expires: Option<String>,
    pub trial_expires: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MusicFolders {
    #[serde(deserialize_with = "one_or_many")]
    pub music_folder: Vec<MusicFolder>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MusicFolder {
    pub id: i32,
    pub name: Option<String>,
}

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
pub struct Genres {
    #[serde(deserialize_with = "one_or_many")]
    pub genre: Vec<Genre>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Genre {
    pub value: String,
    pub song_count: i32,
    pub album_count: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArtistsID3 {
    pub ignored_articles: String,
    #[serde(deserialize_with = "one_or_many")]
    pub index: Vec<IndexID3>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IndexID3 {
    pub name: String,
    #[serde(deserialize_with = "one_or_many")]
    pub artist: Vec<ArtistID3>,
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
pub struct Directory {
    pub id: String,
    pub parent: Option<String>,
    pub name: String,
    pub starred: Option<String>,
    pub user_rating: Option<i32>,
    pub average_rating: Option<f64>,
    pub play_count: Option<i64>,
    #[serde(deserialize_with = "one_or_many")]
    pub child: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Videos {
    #[serde(deserialize_with = "one_or_many")]
    pub video: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Captions {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AudioTrack {
    pub id: String,
    pub name: Option<String>,
    pub language_code: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoConversion {
    pub id: String,
    pub bit_rate: Option<i32>,
    pub audio_track_id: Option<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoInfo {
    pub id: String,
    #[serde(deserialize_with = "one_or_many")]
    pub captions: Vec<Captions>,
    #[serde(deserialize_with = "one_or_many")]
    pub audio_track: Vec<AudioTrack>,
    #[serde(deserialize_with = "one_or_many")]
    pub conversion: Vec<VideoConversion>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NowPlayingEntry {
    #[serde(flatten)]
    pub song: Child,
    pub username: String,
    pub minutes_ago: i32,
    pub player_id: i32,
    pub player_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NowPlaying {
    #[serde(deserialize_with = "one_or_many")]
    pub entry: Vec<NowPlayingEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(rename = "match", deserialize_with = "one_or_many")]
    pub match_: Vec<Child>,
    pub offset: i32,
    pub total_hits: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchResult2 {
    #[serde(deserialize_with = "one_or_many")]
    pub artist: Vec<Artist>,
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<Child>,
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchResult3 {
    #[serde(deserialize_with = "one_or_many")]
    pub artist: Vec<ArtistID3>,
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<AlbumID3>,
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub song_count: i32,
    pub duration: i32,
    pub created: Option<String>,
    pub changed: Option<String>,
    pub cover_art: Option<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub allowed_user: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlists {
    #[serde(deserialize_with = "one_or_many")]
    pub playlist: Vec<Playlist>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaylistWithSongs {
    #[serde(flatten)]
    pub playlist: Playlist,
    #[serde(deserialize_with = "one_or_many")]
    pub entry: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JukeboxStatus {
    pub current_index: i32,
    pub playing: bool,
    pub gain: f32,
    pub position: Option<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JukeboxPlaylist {
    #[serde(flatten)]
    pub status: JukeboxStatus,
    #[serde(deserialize_with = "one_or_many")]
    pub entry: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatMessage {
    pub username: String,
    pub time: i64,
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatMessages {
    #[serde(deserialize_with = "one_or_many")]
    pub chat_message: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumList {
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumList2 {
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<AlbumID3>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Songs {
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Lyrics {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PodcastChannel {
    pub id: String,
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub cover_art: Option<String>,
    pub original_image_url: Option<String>,
    pub status: Option<String>,
    pub error_message: Option<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub episode: Vec<PodcastEpisode>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Podcasts {
    #[serde(deserialize_with = "one_or_many")]
    pub channel: Vec<PodcastChannel>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PodcastEpisode {
    #[serde(flatten)]
    pub song: Child,
    pub stream_id: Option<String>,
    pub channel_id: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub publish_date: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewestPodcasts {
    #[serde(deserialize_with = "one_or_many")]
    pub episode: Vec<PodcastEpisode>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InternetRadioStation {
    pub id: String,
    pub name: String,
    pub stream_url: String,
    pub home_page_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InternetRadioStations {
    #[serde(deserialize_with = "one_or_many")]
    pub internet_radio_station: Vec<InternetRadioStation>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Bookmark {
    pub entry: Option<Child>,
    pub position: i64,
    pub username: String,
    pub comment: Option<String>,
    pub created: Option<String>,
    pub changed: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Bookmarks {
    #[serde(deserialize_with = "one_or_many")]
    pub bookmark: Vec<Bookmark>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlayQueue {
    #[serde(deserialize_with = "one_or_many")]
    pub entry: Vec<Child>,
    #[serde(default, deserialize_with = "string_or_number")]
    pub current: Option<String>,
    pub position: Option<i64>,
    pub username: Option<String>,
    pub changed: Option<String>,
    pub changed_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Share {
    pub id: String,
    pub url: String,
    pub description: Option<String>,
    pub username: String,
    pub created: Option<String>,
    pub expires: Option<String>,
    pub last_visited: Option<String>,
    pub visit_count: i32,
    #[serde(deserialize_with = "one_or_many")]
    pub entry: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Shares {
    #[serde(deserialize_with = "one_or_many")]
    pub share: Vec<Share>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Starred {
    #[serde(deserialize_with = "one_or_many")]
    pub artist: Vec<Artist>,
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<Child>,
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Starred2 {
    #[serde(deserialize_with = "one_or_many")]
    pub artist: Vec<ArtistID3>,
    #[serde(deserialize_with = "one_or_many")]
    pub album: Vec<AlbumID3>,
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SimilarSongs {
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SimilarSongs2 {
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TopSongs {
    #[serde(deserialize_with = "one_or_many")]
    pub song: Vec<Child>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanStatus {
    pub scanning: bool,
    pub count: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct User {
    pub username: String,
    pub email: Option<String>,
    pub scrobbling_enabled: bool,
    pub max_bit_rate: Option<i32>,
    pub admin_role: bool,
    pub settings_role: bool,
    pub download_role: bool,
    pub upload_role: bool,
    pub playlist_role: bool,
    pub cover_art_role: bool,
    pub comment_role: bool,
    pub podcast_role: bool,
    pub stream_role: bool,
    pub jukebox_role: bool,
    pub share_role: bool,
    pub video_conversion_role: bool,
    pub avatar_last_changed: Option<String>,
    #[serde(deserialize_with = "one_or_many")]
    pub folder: Vec<i32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Users {
    #[serde(deserialize_with = "one_or_many")]
    pub user: Vec<User>,
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

impl SubsonicClient {
    /// `ping` — checks connectivity and credentials.
    pub async fn ping(&self) -> Result<()> {
        self.call::<Value>("ping", &[]).await.map(|_| ())
    }

    /// `getLicense` — software license details.
    pub async fn get_license(&self) -> Result<License> {
        let body = self.call::<Value>("getLicense", &[]).await?;
        extract(body, "license")
    }

    // -----------------------------------------------------------------------
    // Browsing
    // -----------------------------------------------------------------------

    /// `getMusicFolders` — configured top-level music folders.
    pub async fn get_music_folders(&self) -> Result<MusicFolders> {
        let body = self.call::<Value>("getMusicFolders", &[]).await?;
        extract(body, "musicFolders")
    }

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

    /// `getMusicDirectory` — files in a music directory.
    pub async fn get_music_directory(&self, id: &str) -> Result<Directory> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getMusicDirectory", &params).await?;
        extract(body, "directory")
    }

    /// `getGenres` — all genres.
    pub async fn get_genres(&self) -> Result<Genres> {
        let body = self.call::<Value>("getGenres", &[]).await?;
        extract(body, "genres")
    }

    /// `getArtists` — artists organized by ID3 tags.
    pub async fn get_artists(&self, music_folder_id: Option<&str>) -> Result<ArtistsID3> {
        let params = query(&[("musicFolderId", music_folder_id.map(str::to_string))]);
        let body = self.call::<Value>("getArtists", &params).await?;
        extract(body, "artists")
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

    /// `getSong` — details for a single song.
    pub async fn get_song(&self, id: &str) -> Result<Child> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getSong", &params).await?;
        extract(body, "song")
    }

    /// `getVideos` — all video files.
    pub async fn get_videos(&self) -> Result<Videos> {
        let body = self.call::<Value>("getVideos", &[]).await?;
        extract(body, "videos")
    }

    /// `getVideoInfo` — details for a video.
    pub async fn get_video_info(&self, id: &str) -> Result<VideoInfo> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getVideoInfo", &params).await?;
        extract(body, "videoInfo")
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

    /// `getAlbumInfo` — album notes and images (last.fm).
    pub async fn get_album_info(&self, id: &str) -> Result<AlbumInfo> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getAlbumInfo", &params).await?;
        extract(body, "albumInfo")
    }

    /// `getAlbumInfo2` — like `getAlbumInfo`, organized by ID3 tags.
    pub async fn get_album_info2(&self, id: &str) -> Result<AlbumInfo> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getAlbumInfo2", &params).await?;
        extract(body, "albumInfo")
    }

    /// `getSimilarSongs` — random songs from similar artists (last.fm).
    pub async fn get_similar_songs(&self, id: &str, count: Option<u32>) -> Result<SimilarSongs> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("count", count.map(|v| v.to_string())),
        ]);
        let body = self.call::<Value>("getSimilarSongs", &params).await?;
        extract(body, "similarSongs")
    }

    /// `getSimilarSongs2` — like `getSimilarSongs`, organized by ID3 tags.
    pub async fn get_similar_songs2(&self, id: &str, count: Option<u32>) -> Result<SimilarSongs2> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("count", count.map(|v| v.to_string())),
        ]);
        let body = self.call::<Value>("getSimilarSongs2", &params).await?;
        extract(body, "similarSongs2")
    }

    /// `getTopSongs` — top songs for an artist (last.fm).
    pub async fn get_top_songs(&self, artist: &str, count: Option<u32>) -> Result<TopSongs> {
        let params = query(&[
            ("artist", Some(artist.to_string())),
            ("count", count.map(|v| v.to_string())),
        ]);
        let body = self.call::<Value>("getTopSongs", &params).await?;
        extract(body, "topSongs")
    }

    // -----------------------------------------------------------------------
    // Album / song lists
    // -----------------------------------------------------------------------

    /// `getAlbumList` — album lists organized by file structure.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_album_list(
        &self,
        kind: AlbumListType,
        size: Option<u32>,
        offset: Option<u32>,
        from_year: Option<i32>,
        to_year: Option<i32>,
        genre: Option<&str>,
        music_folder_id: Option<&str>,
    ) -> Result<AlbumList> {
        let body = self
            .call::<Value>(
                "getAlbumList",
                &album_list_params(
                    kind,
                    size,
                    offset,
                    from_year,
                    to_year,
                    genre,
                    music_folder_id,
                ),
            )
            .await?;
        extract(body, "albumList")
    }

    /// `getAlbumList2` — album lists organized by ID3 tags.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_album_list2(
        &self,
        kind: AlbumListType,
        size: Option<u32>,
        offset: Option<u32>,
        from_year: Option<i32>,
        to_year: Option<i32>,
        genre: Option<&str>,
        music_folder_id: Option<&str>,
    ) -> Result<AlbumList2> {
        let body = self
            .call::<Value>(
                "getAlbumList2",
                &album_list_params(
                    kind,
                    size,
                    offset,
                    from_year,
                    to_year,
                    genre,
                    music_folder_id,
                ),
            )
            .await?;
        extract(body, "albumList2")
    }

    /// `getRandomSongs` — random songs matching optional criteria.
    pub async fn get_random_songs(
        &self,
        size: Option<u32>,
        genre: Option<&str>,
        from_year: Option<i32>,
        to_year: Option<i32>,
        music_folder_id: Option<&str>,
    ) -> Result<Songs> {
        let params = query(&[
            ("size", size.map(|v| v.to_string())),
            ("genre", genre.map(str::to_string)),
            ("fromYear", from_year.map(|v| v.to_string())),
            ("toYear", to_year.map(|v| v.to_string())),
            ("musicFolderId", music_folder_id.map(str::to_string)),
        ]);
        let body = self.call::<Value>("getRandomSongs", &params).await?;
        extract(body, "randomSongs")
    }

    /// `getSongsByGenre` — songs in a given genre.
    pub async fn get_songs_by_genre(
        &self,
        genre: &str,
        count: Option<u32>,
        offset: Option<u32>,
        music_folder_id: Option<&str>,
    ) -> Result<Songs> {
        let params = query(&[
            ("genre", Some(genre.to_string())),
            ("count", count.map(|v| v.to_string())),
            ("offset", offset.map(|v| v.to_string())),
            ("musicFolderId", music_folder_id.map(str::to_string)),
        ]);
        let body = self.call::<Value>("getSongsByGenre", &params).await?;
        extract(body, "songsByGenre")
    }

    /// `getNowPlaying` — what every user is currently playing.
    pub async fn get_now_playing(&self) -> Result<NowPlaying> {
        let body = self.call::<Value>("getNowPlaying", &[]).await?;
        extract(body, "nowPlaying")
    }

    /// `getStarred` — starred songs, albums and artists.
    pub async fn get_starred(&self, music_folder_id: Option<&str>) -> Result<Starred> {
        let params = query(&[("musicFolderId", music_folder_id.map(str::to_string))]);
        let body = self.call::<Value>("getStarred", &params).await?;
        extract(body, "starred")
    }

    /// `getStarred2` — like `getStarred`, organized by ID3 tags.
    pub async fn get_starred2(&self, music_folder_id: Option<&str>) -> Result<Starred2> {
        let params = query(&[("musicFolderId", music_folder_id.map(str::to_string))]);
        let body = self.call::<Value>("getStarred2", &params).await?;
        extract(body, "starred2")
    }

    // -----------------------------------------------------------------------
    // Searching
    // -----------------------------------------------------------------------

    /// `search` — deprecated, use [`SubsonicClient::search2`] instead.
    #[allow(clippy::too_many_arguments)]
    pub async fn search(
        &self,
        artist: Option<&str>,
        album: Option<&str>,
        title: Option<&str>,
        any: Option<&str>,
        count: Option<u32>,
        offset: Option<u32>,
        newer_than: Option<i64>,
    ) -> Result<SearchResult> {
        let params = query(&[
            ("artist", artist.map(str::to_string)),
            ("album", album.map(str::to_string)),
            ("title", title.map(str::to_string)),
            ("any", any.map(str::to_string)),
            ("count", count.map(|v| v.to_string())),
            ("offset", offset.map(|v| v.to_string())),
            ("newerThan", newer_than.map(|v| v.to_string())),
        ]);
        let body = self.call::<Value>("search", &params).await?;
        extract(body, "searchResult")
    }

    /// `search2` — albums, artists and songs organized by file structure.
    pub async fn search2(
        &self,
        query_text: &str,
        options: &SearchOptions,
    ) -> Result<SearchResult2> {
        let body = self
            .call::<Value>("search2", &search_params(query_text, options))
            .await?;
        extract(body, "searchResult2")
    }

    /// `search3` — albums, artists and songs organized by ID3 tags.
    pub async fn search3(
        &self,
        query_text: &str,
        options: &SearchOptions,
    ) -> Result<SearchResult3> {
        let body = self
            .call::<Value>("search3", &search_params(query_text, options))
            .await?;
        extract(body, "searchResult3")
    }

    // -----------------------------------------------------------------------
    // Playlists
    // -----------------------------------------------------------------------

    /// `getPlaylists` — all playlists the user may play.
    pub async fn get_playlists(&self, username: Option<&str>) -> Result<Playlists> {
        let params = query(&[("username", username.map(str::to_string))]);
        let body = self.call::<Value>("getPlaylists", &params).await?;
        extract(body, "playlists")
    }

    /// `getPlaylist` — listing of files in a saved playlist.
    pub async fn get_playlist(&self, id: &str) -> Result<PlaylistWithSongs> {
        let params = query(&[("id", Some(id.to_string()))]);
        let body = self.call::<Value>("getPlaylist", &params).await?;
        extract(body, "playlist")
    }

    /// `createPlaylist` — creates or updates a playlist.
    pub async fn create_playlist(
        &self,
        playlist_id: Option<&str>,
        name: Option<&str>,
        song_ids: &[&str],
    ) -> Result<Option<PlaylistWithSongs>> {
        let mut params = query(&[
            ("playlistId", playlist_id.map(str::to_string)),
            ("name", name.map(str::to_string)),
        ]);
        for id in song_ids {
            params.push(("songId", (*id).to_string()));
        }
        let body = self.call::<Value>("createPlaylist", &params).await?;
        extract_opt(body, "playlist")
    }

    /// `updatePlaylist` — updates a playlist owned by the user.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist(
        &self,
        playlist_id: &str,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        song_ids_to_add: &[&str],
        song_indexes_to_remove: &[i32],
    ) -> Result<()> {
        let mut params = query(&[
            ("playlistId", Some(playlist_id.to_string())),
            ("name", name.map(str::to_string)),
            ("comment", comment.map(str::to_string)),
            ("public", public.map(|v| v.to_string())),
        ]);
        for id in song_ids_to_add {
            params.push(("songIdToAdd", (*id).to_string()));
        }
        for index in song_indexes_to_remove {
            params.push(("songIndexToRemove", index.to_string()));
        }
        self.call::<Value>("updatePlaylist", &params)
            .await
            .map(|_| ())
    }

    /// `deletePlaylist` — deletes a saved playlist.
    pub async fn delete_playlist(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("deletePlaylist", &params)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Media retrieval (URL builders)
    // -----------------------------------------------------------------------

    /// `stream` — URL to stream a media file (transcoded as requested).
    pub fn stream_url(&self, id: &str) -> Result<reqwest::Url> {
        self.endpoint_url("stream", &[("id", id.to_string())])
    }

    /// `download` — URL to download the original media file.
    pub fn download_url(&self, id: &str) -> Result<reqwest::Url> {
        self.endpoint_url("download", &[("id", id.to_string())])
    }

    /// `hls` — URL of the HLS playlist. Repeat `bit_rate` for adaptive streaming.
    pub fn hls_url(
        &self,
        id: &str,
        bit_rates: &[&str],
        audio_track: Option<&str>,
    ) -> Result<reqwest::Url> {
        let mut params: Vec<(&str, String)> = vec![("id", id.to_string())];
        for bit_rate in bit_rates {
            params.push(("bitRate", (*bit_rate).to_string()));
        }
        if let Some(track) = audio_track {
            params.push(("audioTrack", track.to_string()));
        }
        self.resource_url("hls.m3u8", &params)
    }

    /// `getCaptions` — URL of a video's subtitles.
    pub fn get_captions_url(&self, id: &str, format: Option<&str>) -> Result<reqwest::Url> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("format", format.map(str::to_string)),
        ]);
        self.endpoint_url("getCaptions", &params)
    }

    /// `getCoverArt` — URL of a cover art image.
    pub fn get_cover_art_url(&self, id: &str, size: Option<u32>) -> Result<reqwest::Url> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("size", size.map(|v| v.to_string())),
        ]);
        self.endpoint_url("getCoverArt", &params)
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

    /// `getAvatar` — URL of a user's avatar image.
    pub fn get_avatar_url(&self, username: &str) -> Result<reqwest::Url> {
        self.endpoint_url("getAvatar", &[("username", username.to_string())])
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

    /// `scrobble` — registers local playback of one or more files.
    pub async fn scrobble(
        &self,
        ids: &[&str],
        times: &[i64],
        submission: Option<bool>,
    ) -> Result<()> {
        let mut params: Vec<(&str, String)> = Vec::new();
        for id in ids {
            params.push(("id", (*id).to_string()));
        }
        for time in times {
            params.push(("time", time.to_string()));
        }
        if let Some(submission) = submission {
            params.push(("submission", submission.to_string()));
        }
        self.call::<Value>("scrobble", &params).await.map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Sharing
    // -----------------------------------------------------------------------

    /// `getShares` — shares the user may manage.
    pub async fn get_shares(&self) -> Result<Shares> {
        let body = self.call::<Value>("getShares", &[]).await?;
        extract(body, "shares")
    }

    /// `createShare` — creates a public share URL.
    pub async fn create_share(
        &self,
        ids: &[&str],
        description: Option<&str>,
        expires: Option<i64>,
    ) -> Result<Shares> {
        let mut params: Vec<(&str, String)> = Vec::new();
        for id in ids {
            params.push(("id", (*id).to_string()));
        }
        if let Some(description) = description {
            params.push(("description", description.to_string()));
        }
        if let Some(expires) = expires {
            params.push(("expires", expires.to_string()));
        }
        let body = self.call::<Value>("createShare", &params).await?;
        extract(body, "shares")
    }

    /// `updateShare` — updates an existing share.
    pub async fn update_share(
        &self,
        id: &str,
        description: Option<&str>,
        expires: Option<i64>,
    ) -> Result<()> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("description", description.map(str::to_string)),
            ("expires", expires.map(|v| v.to_string())),
        ]);
        self.call::<Value>("updateShare", &params).await.map(|_| ())
    }

    /// `deleteShare` — deletes an existing share.
    pub async fn delete_share(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("deleteShare", &params).await.map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Podcast
    // -----------------------------------------------------------------------

    /// `getPodcasts` — subscribed channels and optionally their episodes.
    pub async fn get_podcasts(
        &self,
        include_episodes: Option<bool>,
        id: Option<&str>,
    ) -> Result<Podcasts> {
        let params = query(&[
            ("includeEpisodes", include_episodes.map(|v| v.to_string())),
            ("id", id.map(str::to_string)),
        ]);
        let body = self.call::<Value>("getPodcasts", &params).await?;
        extract(body, "podcasts")
    }

    /// `getNewestPodcasts` — most recently published episodes.
    pub async fn get_newest_podcasts(&self, count: Option<u32>) -> Result<NewestPodcasts> {
        let params = query(&[("count", count.map(|v| v.to_string()))]);
        let body = self.call::<Value>("getNewestPodcasts", &params).await?;
        extract(body, "newestPodcasts")
    }

    /// `refreshPodcasts` — asks the server to check for new episodes.
    pub async fn refresh_podcasts(&self) -> Result<()> {
        self.call::<Value>("refreshPodcasts", &[]).await.map(|_| ())
    }

    /// `createPodcastChannel` — adds a new Podcast channel.
    pub async fn create_podcast_channel(&self, url: &str) -> Result<()> {
        let params = query(&[("url", Some(url.to_string()))]);
        self.call::<Value>("createPodcastChannel", &params)
            .await
            .map(|_| ())
    }

    /// `deletePodcastChannel` — deletes a Podcast channel.
    pub async fn delete_podcast_channel(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("deletePodcastChannel", &params)
            .await
            .map(|_| ())
    }

    /// `deletePodcastEpisode` — deletes a Podcast episode.
    pub async fn delete_podcast_episode(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("deletePodcastEpisode", &params)
            .await
            .map(|_| ())
    }

    /// `downloadPodcastEpisode` — asks the server to download an episode.
    pub async fn download_podcast_episode(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("downloadPodcastEpisode", &params)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Jukebox
    // -----------------------------------------------------------------------

    /// `jukeboxControl` — controls playback on the server's audio hardware.
    pub async fn jukebox_control(
        &self,
        action: JukeboxAction,
        index: Option<i32>,
        offset: Option<i64>,
        ids: &[&str],
        gain: Option<f32>,
    ) -> Result<Value> {
        let mut params: Vec<(&str, String)> = vec![("action", action.as_str().to_string())];
        if let Some(index) = index {
            params.push(("index", index.to_string()));
        }
        if let Some(offset) = offset {
            params.push(("offset", offset.to_string()));
        }
        for id in ids {
            params.push(("id", (*id).to_string()));
        }
        if let Some(gain) = gain {
            params.push(("gain", gain.to_string()));
        }
        self.call::<Value>("jukeboxControl", &params).await
    }

    // -----------------------------------------------------------------------
    // Internet radio
    // -----------------------------------------------------------------------

    /// `getInternetRadioStations` — all internet radio stations.
    pub async fn get_internet_radio_stations(&self) -> Result<InternetRadioStations> {
        let body = self.call::<Value>("getInternetRadioStations", &[]).await?;
        extract(body, "internetRadioStations")
    }

    /// `createInternetRadioStation` — adds a station (admin only).
    pub async fn create_internet_radio_station(
        &self,
        stream_url: &str,
        name: &str,
        homepage_url: Option<&str>,
    ) -> Result<()> {
        let params = query(&[
            ("streamUrl", Some(stream_url.to_string())),
            ("name", Some(name.to_string())),
            ("homepageUrl", homepage_url.map(str::to_string)),
        ]);
        self.call::<Value>("createInternetRadioStation", &params)
            .await
            .map(|_| ())
    }

    /// `updateInternetRadioStation` — updates a station (admin only).
    pub async fn update_internet_radio_station(
        &self,
        id: &str,
        stream_url: &str,
        name: &str,
        homepage_url: Option<&str>,
    ) -> Result<()> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("streamUrl", Some(stream_url.to_string())),
            ("name", Some(name.to_string())),
            ("homepageUrl", homepage_url.map(str::to_string)),
        ]);
        self.call::<Value>("updateInternetRadioStation", &params)
            .await
            .map(|_| ())
    }

    /// `deleteInternetRadioStation` — deletes a station (admin only).
    pub async fn delete_internet_radio_station(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("deleteInternetRadioStation", &params)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Chat
    // -----------------------------------------------------------------------

    /// `getChatMessages` — chat messages, optionally since a timestamp.
    pub async fn get_chat_messages(&self, since: Option<i64>) -> Result<ChatMessages> {
        let params = query(&[("since", since.map(|v| v.to_string()))]);
        let body = self.call::<Value>("getChatMessages", &params).await?;
        extract(body, "chatMessages")
    }

    /// `addChatMessage` — posts a chat message.
    pub async fn add_chat_message(&self, message: &str) -> Result<()> {
        let params = query(&[("message", Some(message.to_string()))]);
        self.call::<Value>("addChatMessage", &params)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // User management
    // -----------------------------------------------------------------------

    /// `getUser` — details for a single user.
    pub async fn get_user(&self, username: &str) -> Result<User> {
        let params = query(&[("username", Some(username.to_string()))]);
        let body = self.call::<Value>("getUser", &params).await?;
        extract(body, "user")
    }

    /// `getUsers` — all users (admin only).
    pub async fn get_users(&self) -> Result<Users> {
        let body = self.call::<Value>("getUsers", &[]).await?;
        extract(body, "users")
    }

    /// `createUser` — creates a user.
    pub async fn create_user(
        &self,
        username: &str,
        password: &str,
        email: &str,
        roles: &UserRoles,
    ) -> Result<()> {
        let mut params = query(&[
            ("username", Some(username.to_string())),
            ("password", Some(password.to_string())),
            ("email", Some(email.to_string())),
        ]);
        roles.append(&mut params);
        self.call::<Value>("createUser", &params).await.map(|_| ())
    }

    /// `updateUser` — updates a user. `None` fields are left unchanged.
    pub async fn update_user(
        &self,
        username: &str,
        password: Option<&str>,
        email: Option<&str>,
        roles: &UserRoles,
    ) -> Result<()> {
        let mut params = query(&[
            ("username", Some(username.to_string())),
            ("password", password.map(str::to_string)),
            ("email", email.map(str::to_string)),
        ]);
        roles.append(&mut params);
        self.call::<Value>("updateUser", &params).await.map(|_| ())
    }

    /// `deleteUser` — deletes a user.
    pub async fn delete_user(&self, username: &str) -> Result<()> {
        let params = query(&[("username", Some(username.to_string()))]);
        self.call::<Value>("deleteUser", &params).await.map(|_| ())
    }

    /// `changePassword` — changes a user's password.
    pub async fn change_password(&self, username: &str, password: &str) -> Result<()> {
        let params = query(&[
            ("username", Some(username.to_string())),
            ("password", Some(password.to_string())),
        ]);
        self.call::<Value>("changePassword", &params)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Bookmarks and play queue
    // -----------------------------------------------------------------------

    /// `getBookmarks` — all bookmarks for the user.
    pub async fn get_bookmarks(&self) -> Result<Bookmarks> {
        let body = self.call::<Value>("getBookmarks", &[]).await?;
        extract(body, "bookmarks")
    }

    /// `createBookmark` — creates or overwrites a bookmark.
    pub async fn create_bookmark(
        &self,
        id: &str,
        position: i64,
        comment: Option<&str>,
    ) -> Result<()> {
        let params = query(&[
            ("id", Some(id.to_string())),
            ("position", Some(position.to_string())),
            ("comment", comment.map(str::to_string)),
        ]);
        self.call::<Value>("createBookmark", &params)
            .await
            .map(|_| ())
    }

    /// `deleteBookmark` — deletes a bookmark.
    pub async fn delete_bookmark(&self, id: &str) -> Result<()> {
        let params = query(&[("id", Some(id.to_string()))]);
        self.call::<Value>("deleteBookmark", &params)
            .await
            .map(|_| ())
    }

    /// `getPlayQueue` — the saved play queue, if any.
    pub async fn get_play_queue(&self) -> Result<Option<PlayQueue>> {
        let body = self.call::<Value>("getPlayQueue", &[]).await?;
        extract_opt(body, "playQueue")
    }

    /// `savePlayQueue` — saves the play queue.
    pub async fn save_play_queue(
        &self,
        ids: &[&str],
        current: Option<&str>,
        position: Option<i64>,
    ) -> Result<()> {
        let mut params: Vec<(&str, String)> = Vec::new();
        for id in ids {
            params.push(("id", (*id).to_string()));
        }
        if let Some(current) = current {
            params.push(("current", current.to_string()));
        }
        if let Some(position) = position {
            params.push(("position", position.to_string()));
        }
        self.call::<Value>("savePlayQueue", &params)
            .await
            .map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Media library scanning
    // -----------------------------------------------------------------------

    /// `getScanStatus` — current scan status.
    pub async fn get_scan_status(&self) -> Result<ScanStatus> {
        let body = self.call::<Value>("getScanStatus", &[]).await?;
        extract(body, "scanStatus")
    }

    /// `startScan` — starts a media library scan.
    pub async fn start_scan(&self) -> Result<ScanStatus> {
        let body = self.call::<Value>("startScan", &[]).await?;
        extract(body, "scanStatus")
    }
}

/// Optional parameters shared by `search2` / `search3`.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    pub artist_count: Option<u32>,
    pub artist_offset: Option<u32>,
    pub album_count: Option<u32>,
    pub album_offset: Option<u32>,
    pub song_count: Option<u32>,
    pub song_offset: Option<u32>,
    pub music_folder_id: Option<String>,
}

fn search_params(query_text: &str, options: &SearchOptions) -> Vec<(&'static str, String)> {
    query(&[
        ("query", Some(query_text.to_string())),
        ("artistCount", options.artist_count.map(|v| v.to_string())),
        ("artistOffset", options.artist_offset.map(|v| v.to_string())),
        ("albumCount", options.album_count.map(|v| v.to_string())),
        ("albumOffset", options.album_offset.map(|v| v.to_string())),
        ("songCount", options.song_count.map(|v| v.to_string())),
        ("songOffset", options.song_offset.map(|v| v.to_string())),
        ("musicFolderId", options.music_folder_id.clone()),
    ])
}

#[allow(clippy::too_many_arguments)]
fn album_list_params(
    kind: AlbumListType,
    size: Option<u32>,
    offset: Option<u32>,
    from_year: Option<i32>,
    to_year: Option<i32>,
    genre: Option<&str>,
    music_folder_id: Option<&str>,
) -> Vec<(&'static str, String)> {
    query(&[
        ("type", Some(kind.as_str().to_string())),
        ("size", size.map(|v| v.to_string())),
        ("offset", offset.map(|v| v.to_string())),
        ("fromYear", from_year.map(|v| v.to_string())),
        ("toYear", to_year.map(|v| v.to_string())),
        ("genre", genre.map(str::to_string)),
        ("musicFolderId", music_folder_id.map(str::to_string)),
    ])
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
            r#"{"artists":{"ignoredArticles":"The","index":{"name":"A","artist":{"id":"1","name":"AC/DC","albumCount":2}}}}"#,
        )
        .unwrap();

        let artists: ArtistsID3 = extract(body, "artists").unwrap();
        assert_eq!(artists.index.len(), 1);
        assert_eq!(artists.index[0].artist.len(), 1);
        assert_eq!(artists.index[0].artist[0].name, "AC/DC");
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
    fn query_skips_none() {
        let params = query(&[("a", Some("1".to_string())), ("b", None)]);
        assert_eq!(params, vec![("a", "1".to_string())]);
    }
}
