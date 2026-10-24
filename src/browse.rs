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

use std::collections::{HashMap, HashSet};

use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};

/// CACHE-9: the offline catalog. Only songs whose download finished are
/// listable (and therefore playable) while the server is unreachable; albums
/// and artists survive only through their downloaded songs.
struct OfflineFilter {
    songs: HashSet<String>,
    albums: HashMap<String, i64>,
    artists: HashMap<String, i64>,
}

/// PATH-12: the offline catalog is the same for every payload between
/// ledger changes; a short TTL coalesces paging/search bursts, and download
/// settles clear it eagerly.
static OFFLINE_FILTER: std::sync::OnceLock<
    std::sync::Mutex<Option<(std::time::Instant, std::sync::Arc<OfflineFilter>)>>,
> = std::sync::OnceLock::new();
const OFFLINE_FILTER_TTL: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) fn clear_offline_filter() {
    if let Some(cache) = OFFLINE_FILTER.get() {
        *cache.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

/// `None` while online; the downloaded set while offline.
async fn offline_filter(
    dbs: &crate::db::Dbs,
    offline: bool,
) -> anyhow::Result<Option<std::sync::Arc<OfflineFilter>>> {
    if !offline {
        return Ok(None);
    }
    let cache = OFFLINE_FILTER.get_or_init(|| std::sync::Mutex::new(None));
    {
        let guard = cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((at, filter)) = guard.as_ref()
            && at.elapsed() < OFFLINE_FILTER_TTL
        {
            return Ok(Some(std::sync::Arc::clone(filter)));
        }
    }
    let Some(songs) = crate::download::offline_song_ids(dbs, offline).await? else {
        return Ok(None);
    };
    let ids: Vec<String> = songs.iter().cloned().collect();
    let (albums, artists) = crate::library::downloaded_owners(&dbs.library, &ids).await?;
    let filter = std::sync::Arc::new(OfflineFilter {
        songs,
        albums,
        artists,
    });
    *cache.lock().unwrap_or_else(|error| error.into_inner()) =
        Some((std::time::Instant::now(), std::sync::Arc::clone(&filter)));
    Ok(Some(filter))
}

pub async fn load_albums_json(
    dbs: &crate::db::Dbs,
    artist: &str,
    order: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let filter = (!artist.is_empty()).then_some(artist);
    let mut albums = crate::library::albums(&dbs.library, filter, order).await?;
    let offline = offline_filter(dbs, offline).await?;
    if let Some(offline) = &offline {
        albums.retain(|album| offline.albums.contains_key(&album.id));
    }
    let cover_ids: Vec<String> = albums
        .iter()
        .filter_map(|album| album.cover_art.clone())
        .collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
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
    if let Some(filter) = offline_filter(dbs, offline).await? {
        songs.retain(|song| filter.songs.contains(&song.id));
    }
    let cover_ids: Vec<String> = songs
        .iter()
        .filter_map(|song| song.cover_art.clone())
        .collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
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
    if let Some(filter) = offline_filter(dbs, offline).await? {
        songs.retain(|song| filter.songs.contains(&song.id));
    }
    let cover_ids: Vec<String> = songs
        .iter()
        .filter_map(|song| song.cover_art.clone())
        .collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
    song_rows_json(&songs, &covers)
}

pub async fn load_artists_json(
    dbs: &crate::db::Dbs,
    order: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut artists = crate::library::artists(&dbs.library, order).await?;
    let filter = offline_filter(dbs, offline).await?;
    if let Some(filter) = &filter {
        artists.retain(|artist| filter.artists.contains_key(&artist.id));
    }
    let cover_ids: Vec<String> = artists
        .iter()
        .filter_map(|artist| artist.cover_art.clone())
        .collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
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
    let filter = offline_filter(dbs, offline).await?;
    if let Some(filter) = &filter {
        albums.retain(|album| filter.albums.contains_key(&album.id));
    }
    let cover_ids: Vec<String> = std::iter::once(artist.cover_art.clone())
        .chain(albums.iter().map(|album| album.cover_art.clone()))
        .flatten()
        .collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
    let user_starred: std::collections::HashSet<String> =
        crate::artist_user::starred_ids(&dbs.artist_user)
            .await?
            .into_iter()
            .collect();
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
    let album_count = filter
        .as_ref()
        .and_then(|filter| filter.artists.get(&artist.id).copied())
        .unwrap_or(artist.album_count);
    let page = ArtistPage {
        artist: &artist,
        cover: cover_url_ref(artist.cover_art.as_deref(), &covers),
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

pub(crate) fn artist_json(
    artist: &crate::library::ArtistRow,
    covers: &HashMap<String, String>,
    album_count: Option<i64>,
    starred: bool,
    rating: i64,
) -> serde_json::Value {
    let cover = cover_url(artist.cover_art.as_deref(), covers);
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
    let cover = cover_url(album.cover_art.as_deref(), covers);
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

fn song_json(
    id: &str,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    album_id: Option<&str>,
    track: Option<i32>,
    duration: Option<i32>,
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

/// Borrowed cover URL for a row: the map's value or the empty string.
pub(crate) fn cover_url_ref<'a>(id: Option<&str>, covers: &'a HashMap<String, String>) -> &'a str {
    id.and_then(|id| covers.get(id))
        .map(String::as_str)
        .unwrap_or_default()
}

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
struct AlbumSeq<'a> {
    albums: &'a [crate::library::AlbumRow],
    covers: &'a HashMap<String, String>,
    /// Offline track counts by album id, when the offline filter applies.
    offline: Option<&'a OfflineFilter>,
}

impl Serialize for AlbumSeq<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.albums.len()))?;
        for album in self.albums {
            let song_count = self
                .offline
                .and_then(|filter| filter.albums.get(&album.id).copied())
                .unwrap_or(album.song_count);
            seq.serialize_element(&AlbumElement {
                album,
                cover: cover_url_ref(album.cover_art.as_deref(), self.covers),
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

struct ArtistSeq<'a> {
    artists: &'a [crate::library::ArtistRow],
    covers: &'a HashMap<String, String>,
    offline: Option<&'a OfflineFilter>,
    user_starred: &'a HashSet<String>,
    user_ratings: &'a HashMap<String, i64>,
}

impl Serialize for ArtistSeq<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.artists.len()))?;
        for artist in self.artists {
            let album_count = self
                .offline
                .and_then(|filter| filter.artists.get(&artist.id).copied())
                .unwrap_or(artist.album_count);
            seq.serialize_element(&ArtistElement {
                artist,
                cover: cover_url_ref(artist.cover_art.as_deref(), self.covers),
                album_count,
                starred: artist.starred.is_some() || self.user_starred.contains(&artist.id),
                rating: self.user_ratings.get(&artist.id).copied().unwrap_or(0),
            })?;
        }
        seq.end()
    }
}

/// One artist page: the artist fields plus its `albums` array.
struct ArtistPage<'a> {
    artist: &'a crate::library::ArtistRow,
    cover: &'a str,
    album_count: i64,
    starred: bool,
    rating: i64,
    albums: AlbumSeq<'a>,
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
        let mut state = serializer.serialize_struct("song", 9)?;
        state.serialize_field("id", &self.song.id)?;
        state.serialize_field("title", &self.song.title)?;
        state.serialize_field("artist", &self.song.artist)?;
        state.serialize_field("album", &self.song.album)?;
        state.serialize_field("album_id", &self.song.album_id)?;
        state.serialize_field("track", &self.song.track)?;
        state.serialize_field("duration", &self.song.duration)?;
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
                cover: cover_url_ref(song.cover_art.as_deref(), self.covers),
            })?;
        }
        seq.end()
    }
}

/// The paged Songs payload (`rows` is the streamed SongSeq).
struct SongsPage<'a> {
    offset: i64,
    next: i64,
    has_more: bool,
    songs: &'a [crate::library::SongRow],
    covers: &'a HashMap<String, String>,
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

fn song_rows_json(
    songs: &[crate::library::SongRow],
    covers: &HashMap<String, String>,
) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&SongSeq { songs, covers })?)
}

/// Fetches an album's description text from the server (`getAlbumInfo2`).
pub async fn album_notes(dbs: &crate::db::Dbs, album_id: &str) -> String {
    let Ok(client) = crate::qml::client(dbs).await else {
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

pub(crate) fn cover_url(id: Option<&str>, covers: &HashMap<String, String>) -> String {
    cover_url_ref(id, covers).to_string()
}

/// One song row in the search payload, independent of the engine that found
/// it. The FTS path hydrates the extra fields with joins; the Nucleo index
/// carries them from the catalog build.
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
fn merge_json_rows(
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
    let filter = offline_filter(dbs, offline).await?;
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
        let artists = crate::library::artists(&dbs.library, "Name").await?;
        let albums = crate::library::albums(&dbs.library, None, "Artist").await?;
        (
            artists,
            albums,
            crate::search::most_played(&dbs.search, fetch as i64).await?,
        )
    } else {
        let (artists, albums) =
            crate::library::search_artists_albums(&dbs.library, &cleaned, fetch as i64).await?;
        let songs = crate::search::search(&dbs.search, &cleaned, fetch as i64).await?;
        (artists, albums, songs)
    };
    let (mut artists, mut albums) = match &filter {
        Some(filter) => (
            artists
                .into_iter()
                .filter(|artist| filter.artists.contains_key(&artist.id))
                .collect(),
            albums
                .into_iter()
                .filter(|album| filter.albums.contains_key(&album.id))
                .collect(),
        ),
        None => (artists, albums),
    };
    artists.truncate(limit);
    albums.truncate(limit);
    if let Some(filter) = &filter {
        songs.retain(|song| filter.songs.contains(&song.song_id));
    }
    songs.truncate(limit);

    let song_ids: Vec<String> = songs.iter().map(|song| song.song_id.clone()).collect();
    let song_covers = crate::library::song_cover_ids(&dbs.library, &song_ids).await?;
    let song_albums = crate::library::song_album_ids(&dbs.library, &song_ids).await?;
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

async fn search_value_index(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    let filter = offline_filter(dbs, offline).await?;
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
        let artists = crate::library::artists(&dbs.library, "Name")
            .await?
            .into_iter()
            .map(std::sync::Arc::new)
            .collect();
        let albums = crate::library::albums(&dbs.library, None, "Artist")
            .await?
            .into_iter()
            .map(std::sync::Arc::new)
            .collect();
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
        artists.retain(|artist| filter.artists.contains_key(&artist.id));
        albums.retain(|album| filter.albums.contains_key(&album.id));
        songs.retain(|song| filter.songs.contains(&song.id));
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
            songs.retain(|song| filter.songs.contains(&song.id));
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
                cover_url(song.cover_art.as_deref(), &covers),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn song_row(id: &str, album_id: Option<&str>) -> crate::library::SongRow {
        crate::library::SongRow {
            id: id.to_string(),
            title: id.to_string(),
            artist: Some("Radiohead".to_string()),
            album: Some("Kid A".to_string()),
            album_id: album_id.map(str::to_string),
            track: Some(1),
            duration: Some(200),
            cover_art: None,
            starred: None,
        }
    }

    /// ARC-UI-1: the Songs list maps `album_id` into the row so the Alt
    /// "Go to Artist" jump can resolve the album's artist.
    #[test]
    fn song_rows_carry_the_album_id_for_the_alt_jumps() {
        let rows = vec![song_row("s1", Some("al1")), song_row("s2", None)];
        let json = song_rows_json(&rows, &HashMap::new()).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["album_id"], "al1");
        assert!(parsed[1]["album_id"].is_null());
    }

    /// UI-56: the favourite flag rides the row JSON as a boolean.
    #[test]
    fn song_rows_carry_the_starred_flag() {
        let mut starred = song_row("s1", Some("al1"));
        starred.starred = Some("2026-10-14T00:00:00Z".to_string());
        let rows = vec![starred, song_row("s2", Some("al1"))];
        let json = song_rows_json(&rows, &HashMap::new()).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["starred"], true);
        assert_eq!(parsed[1]["starred"], false);
    }

    /// The streaming serializers must emit exactly the object the old
    /// `Value` builders produce: the Qt side parses this shape, so a field
    /// rename or a null/absent drift would silently blank a list.
    #[test]
    fn streaming_list_rows_match_the_value_payload_shape() {
        let covers = HashMap::new();

        let album = crate::library::AlbumRow {
            id: "al1".to_string(),
            title: "Kid A".to_string(),
            artist: Some("Radiohead".to_string()),
            year: Some(2000),
            song_count: 2,
            cover_art: None,
            starred: None,
            rating: 3,
        };
        let albums = vec![album];
        let streamed = serde_json::to_string(&AlbumSeq {
            albums: &albums,
            covers: &covers,
            offline: None,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([album_json(&albums[0], &covers, None)])
        );

        let artist = crate::library::ArtistRow {
            id: "ar1".to_string(),
            name: "Radiohead".to_string(),
            album_count: 1,
            cover_art: None,
            starred: Some("2026-10-14T00:00:00Z".to_string()),
        };
        let artists = vec![artist];
        let user_starred = HashSet::new();
        let user_ratings = HashMap::new();
        let streamed = serde_json::to_string(&ArtistSeq {
            artists: &artists,
            covers: &covers,
            offline: None,
            user_starred: &user_starred,
            user_ratings: &user_ratings,
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([artist_json(&artists[0], &covers, None, true, 0)])
        );

        let songs = vec![song_row("s1", Some("al1"))];
        let streamed = song_rows_json(&songs, &covers).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
        let expected = song_json(
            &songs[0].id,
            &songs[0].title,
            songs[0].artist.as_deref(),
            songs[0].album.as_deref(),
            songs[0].album_id.as_deref(),
            songs[0].track,
            songs[0].duration,
            cover_url(songs[0].cover_art.as_deref(), &covers),
            false,
        );
        assert_eq!(parsed, serde_json::json!([expected]));

        // The artist page nests the album sequence under `albums`.
        let page = ArtistPage {
            artist: &artists[0],
            cover: "",
            album_count: 1,
            starred: true,
            rating: 0,
            albums: AlbumSeq {
                albums: &albums,
                covers: &covers,
                offline: None,
            },
        };
        let parsed = serde_json::to_value(&page).unwrap();
        assert_eq!(parsed["id"], "ar1");
        assert_eq!(parsed["albums"][0]["id"], "al1");

        // The songs page keeps its paging envelope around the streamed rows.
        let page = SongsPage {
            offset: 10,
            next: 12,
            has_more: true,
            songs: &songs,
            covers: &covers,
        };
        let parsed = serde_json::to_value(&page).unwrap();
        assert_eq!(parsed["offset"], 10);
        assert_eq!(parsed["next"], 12);
        assert_eq!(parsed["has_more"], true);
        assert_eq!(parsed["rows"][0]["id"], "s1");
    }

    /// Memory probe for the streaming serializers: the same 5k-row album list
    /// through the old `Vec<Value>` path and the borrowed `AlbumSeq`.
    ///
    /// `cargo test browse::tests::bench_streamed_list_vs_value_tree -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_streamed_list_vs_value_tree() {
        let albums: Vec<crate::library::AlbumRow> = (0..5_000)
            .map(|index| crate::library::AlbumRow {
                id: format!("al{index}"),
                title: format!("Album number {index}"),
                artist: Some("Radiohead".to_string()),
                year: Some(2000),
                song_count: 12,
                cover_art: Some(format!("cov{index}")),
                starred: None,
                rating: 3,
            })
            .collect();
        let covers: HashMap<String, String> = (0..5_000)
            .map(|index| {
                (
                    format!("cov{index}"),
                    format!("file:///covers/cov{index}.jpg"),
                )
            })
            .collect();

        let (streamed, streamed_peak) = crate::test_alloc::measure(|| {
            serde_json::to_string(&AlbumSeq {
                albums: &albums,
                covers: &covers,
                offline: None,
            })
            .expect("streamed list")
        });
        let (treed, tree_peak) = crate::test_alloc::measure(|| {
            let rows: Vec<serde_json::Value> = albums
                .iter()
                .map(|album| album_json(album, &covers, None))
                .collect();
            serde_json::to_string(&rows).expect("value list")
        });

        eprintln!(
            "album list JSON (5k rows): streamed peak {} KiB | Value-tree peak {} KiB",
            streamed_peak / 1024,
            tree_peak / 1024,
        );
        let streamed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
        let treed: serde_json::Value = serde_json::from_str(&treed).unwrap();
        assert_eq!(
            streamed, treed,
            "the streamed shape must parse to the same JSON value"
        );
        assert!(
            streamed_peak < tree_peak,
            "streaming must not cost more than the intermediate Value tree"
        );
    }

    /// ARC-SEARCH: the Nucleo engine answers from the in-memory catalog and
    /// keeps S-8 (one album match = track order) and S-9 (Most played).
    #[tokio::test]
    async fn nucleo_search_payload_follows_the_one_album_and_most_played() {
        let library = crate::sqlite::memory_pool(sqlx::migrate!("./migrations_library")).await;
        let covers = crate::sqlite::memory_pool(sqlx::migrate!("./migrations_covers")).await;
        let dbs = crate::db::Dbs {
            credentials: library.clone(),
            library: library.clone(),
            search: library.clone(),
            covers,
            media: library.clone(),
            lyrics: library.clone(),
            colors: library.clone(),
            status: library.clone(),
            artist_user: library.clone(),
            download: library.clone(),
            playlist: library.clone(),
        };
        sqlx::raw_sql(
            "INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Radiohead', 1);
             INSERT INTO albums (id, artist_id, name, artist, song_count)
                 VALUES ('al1', 'ar1', 'Kid A', 'Radiohead', 2);
             INSERT INTO songs
                 (id, album_id, title, artist, album, track, disc_number, duration, play_count)
             VALUES
                 ('s1', 'al1', 'Everything In Its Right Place', 'Radiohead', 'Kid A', 1, 1, 240, 0),
                 ('s2', 'al1', 'Idioteque', 'Radiohead', 'Kid A', 2, 1, 300, 5);",
        )
        .execute(&dbs.library)
        .await
        .unwrap();

        let parsed = search_value_index(&dbs, "rdo", false, 20).await.unwrap();
        assert_eq!(parsed["most_played"], false);
        assert_eq!(parsed["album_context"], "al1");
        let ids: Vec<&str> = parsed["songs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|song| song["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["s1", "s2"], "one album match reads in track order");

        let parsed = search_value_index(&dbs, "", false, 20).await.unwrap();
        assert_eq!(parsed["most_played"], true);
        assert!(parsed["album_context"].is_null());
        assert_eq!(parsed["songs"][0]["id"], "s2");
        // The unified search browses every scope on an empty query.
        assert_eq!(parsed["artists"][0]["id"], "ar1");
        assert_eq!(parsed["albums"][0]["id"], "al1");

        crate::search::index::clear();
    }

    #[test]
    fn hybrid_merge_keeps_primary_order_and_dedupes() {
        let mut primary = serde_json::json!([{"id": "a"}, {"id": "b"}]);
        let mut secondary = serde_json::json!([{"id": "b"}, {"id": "c"}, {"id": "d"}]);
        merge_json_rows(&mut primary, &mut secondary, 3);
        let ids: Vec<&str> = primary
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    /// Session-shaped probe: the real list/search loaders against a 20k-song
    /// catalog (2k albums, 200 artists), reporting each payload's peak heap
    /// and size. This is the Rust half of the "live profile" without Qt.
    ///
    /// `cargo test browse::tests::bench_session_loaders -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_session_loaders() {
        use std::time::Instant;

        fn measured(
            rt: &tokio::runtime::Runtime,
            future: impl std::future::Future<Output = anyhow::Result<String>>,
        ) -> (usize, usize, std::time::Duration) {
            let start = Instant::now();
            let (json, peak) = crate::test_alloc::measure(|| rt.block_on(future).expect("loader"));
            (json.len(), peak.max(0) as usize, start.elapsed())
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("probe runtime");
        let library = rt.block_on(crate::sqlite::memory_pool(sqlx::migrate!(
            "./migrations_library"
        )));
        let covers = rt.block_on(crate::sqlite::memory_pool(sqlx::migrate!(
            "./migrations_media"
        )));
        let dbs = crate::db::Dbs {
            credentials: library.clone(),
            library: library.clone(),
            search: library.clone(),
            covers,
            media: library.clone(),
            lyrics: library.clone(),
            colors: library.clone(),
            status: library.clone(),
            artist_user: library.clone(),
            download: library.clone(),
            playlist: library.clone(),
        };
        rt.block_on(async {
            sqlx::raw_sql(
                "WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<200)
                 INSERT INTO artists (id, name, album_count, cover_art)
                 SELECT 'ar'||x, 'Artist '||x, 10, 'acov'||x FROM c;
                 WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<2000)
                 INSERT INTO albums
                     (id, artist_id, name, artist, year, song_count, duration, cover_art)
                 SELECT 'al'||x, 'ar'||(((x-1)/10)+1), 'Album '||x,
                        'Artist '||(((x-1)/10)+1), 2000+(x%20), 10, 3600, 'cov'||x
                 FROM c;
                 WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<20000)
                 INSERT INTO songs
                     (id, album_id, artist_id, title, artist, album, track, disc_number,
                      duration, cover_art, play_count)
                 SELECT 's'||x, 'al'||((x-1)/10+1), 'ar'||(((x-1)/100)+1),
                        'Track '||x, 'Artist '||(((x-1)/100)+1),
                        'Album '||((x-1)/10+1), (x%10)+1, 1, 180+(x%120),
                        'cov'||((x-1)/10+1), x%7
                 FROM c;",
            )
            .execute(&library)
            .await
            .expect("seed session");
        });

        let (albums_len, albums_peak, albums_time) =
            measured(&rt, load_albums_json(&dbs, "", "Artist", false));
        eprintln!(
            "session albums (2k): {} KiB payload, {} KiB peak, {:?}",
            albums_len / 1024,
            albums_peak / 1024,
            albums_time
        );

        let (artists_len, artists_peak, artists_time) =
            measured(&rt, load_artists_json(&dbs, "Name", false));
        eprintln!(
            "session artists (200): {} KiB payload, {} KiB peak, {:?}",
            artists_len / 1024,
            artists_peak / 1024,
            artists_time
        );

        let (songs_len, songs_peak, songs_time) =
            measured(&rt, load_songs_page_json(&dbs, 0, 200, "Title", false));
        eprintln!(
            "session songs page (200): {} KiB payload, {} KiB peak, {:?}",
            songs_len / 1024,
            songs_peak / 1024,
            songs_time
        );

        let artist_page = load_artist_json(&dbs, "ar5", "Artist", false);
        let (page_len, page_peak, page_time) = measured(&rt, async {
            artist_page.await.map(|page| page.unwrap_or_default())
        });
        eprintln!(
            "session artist page (10 albums): {} KiB payload, {} KiB peak, {:?}",
            page_len / 1024,
            page_peak / 1024,
            page_time
        );

        // Warm the catalog once, then measure the typed search payload.
        rt.block_on(search_json(&dbs, "artist", false, 30))
            .expect("warm search");
        let (search_len, search_peak, search_time) =
            measured(&rt, search_json(&dbs, "artist", false, 30));
        eprintln!(
            "session search (30 rows/column): {} KiB payload, {} KiB peak, {:?}",
            search_len / 1024,
            search_peak / 1024,
            search_time
        );

        crate::search::index::clear();
    }
}
