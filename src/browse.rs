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
    let covers = crate::covers::url_map_of(&dbs.covers, &cover_ids).await?;
    let rows: Vec<serde_json::Value> = albums
        .iter()
        .map(|album| {
            album_json(
                album,
                &covers,
                offline
                    .as_ref()
                    .and_then(|filter| filter.albums.get(&album.id).copied()),
            )
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
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
    let covers = crate::covers::url_map_of(&dbs.covers, &cover_ids).await?;
    let rows: serde_json::Value = serde_json::from_str(&song_rows_json(&songs, &covers)?)
        .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
    Ok(serde_json::json!({
        "offset": offset,
        "next": offset + returned,
        "has_more": has_more,
        "rows": rows,
    })
    .to_string())
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
    let covers = crate::covers::url_map_of(&dbs.covers, &cover_ids).await?;
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
    let covers = crate::covers::url_map_of(&dbs.covers, &cover_ids).await?;
    // UI-56: the artist favourite is the server flag ORed with the local
    // `artist_user` overlay (ARC-42), so both surfaces agree.
    let user_starred: std::collections::HashSet<String> =
        crate::artist_user::starred_ids(&dbs.artist_user)
            .await?
            .into_iter()
            .collect();
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
    let rows: Vec<serde_json::Value> = artists
        .iter()
        .map(|artist| {
            artist_json(
                artist,
                &covers,
                filter
                    .as_ref()
                    .and_then(|filter| filter.artists.get(&artist.id).copied()),
                artist.starred.is_some() || user_starred.contains(&artist.id),
                user_ratings.get(&artist.id).copied().unwrap_or(0),
            )
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
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
    let covers = crate::covers::url_map_of(&dbs.covers, &cover_ids).await?;
    let user_starred: std::collections::HashSet<String> =
        crate::artist_user::starred_ids(&dbs.artist_user)
            .await?
            .into_iter()
            .collect();
    let user_ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
    let mut value = artist_json(
        &artist,
        &covers,
        filter
            .as_ref()
            .and_then(|filter| filter.artists.get(&artist.id).copied()),
        artist.starred.is_some() || user_starred.contains(&artist.id),
        user_ratings.get(&artist.id).copied().unwrap_or(0),
    );
    value["albums"] = serde_json::Value::Array(
        albums
            .iter()
            .map(|album| {
                album_json(
                    album,
                    &covers,
                    filter
                        .as_ref()
                        .and_then(|filter| filter.albums.get(&album.id).copied()),
                )
            })
            .collect(),
    );
    Ok(Some(value.to_string()))
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

fn song_rows_json(
    songs: &[crate::library::SongRow],
    covers: &HashMap<String, String>,
) -> anyhow::Result<String> {
    let rows: Vec<serde_json::Value> = songs
        .iter()
        .map(|song| {
            song_json(
                &song.id,
                &song.title,
                song.artist.as_deref(),
                song.album.as_deref(),
                song.album_id.as_deref(),
                song.track,
                song.duration,
                cover_url(song.cover_art.as_deref(), covers),
                song.starred.is_some(),
            )
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
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
    id.and_then(|id| covers.get(id))
        .cloned()
        .unwrap_or_default()
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
) -> anyhow::Result<String> {
    match crate::search::engine() {
        crate::search::Engine::Hybrid => search_json_hybrid(dbs, query, offline).await,
        crate::search::Engine::Nucleo => {
            match search_json_index(dbs, query, offline).await {
                Ok(json) => Ok(json),
                Err(err) => {
                    // The in-memory catalog is a cache: a build failure must
                    // not take the palette down while FTS is still available.
                    eprintln!("search: nucleo engine failed, using FTS: {err:#}");
                    search_json_fts(dbs, query, offline).await
                }
            }
        }
        crate::search::Engine::Fts => search_json_fts(dbs, query, offline).await,
    }
}

/// Nucleo answers first; FTS fills any column that came back short (or steps
/// in wholesale when the in-memory catalog fails). The merge is only paid for
/// sparse queries — a broad hit set never touches SQLite.
async fn search_json_hybrid(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let nucleo_json = match search_json_index(dbs, query, offline).await {
        Ok(json) => json,
        Err(err) => {
            eprintln!("search: nucleo engine failed, using FTS: {err:#}");
            return search_json_fts(dbs, query, offline).await;
        }
    };
    let Ok(nucleo) = serde_json::from_str::<serde_json::Value>(&nucleo_json) else {
        return Ok(nucleo_json);
    };
    // The empty-query surface and S-8's single-album tracklist already own
    // their result set; do not let FTS mix another album in.
    if nucleo["most_played"].as_bool() == Some(true) || nucleo["album_context"].is_string() {
        return Ok(nucleo_json);
    }
    const VISIBLE: usize = 20;
    // The FTS fill is for sparse song sets (a rare/typo query, or an offline
    // catalog); a broad hit set already fills the Songs column, and the
    // artist/album lists are cheap Nucleo lookups.
    let sparse_songs = nucleo
        .get("songs")
        .and_then(serde_json::Value::as_array)
        .is_none_or(|rows| rows.len() < VISIBLE);
    if !sparse_songs {
        return Ok(nucleo_json);
    }

    let fts_json = match search_json_fts(dbs, query, offline).await {
        Ok(json) => json,
        Err(err) => {
            eprintln!("search: FTS fill failed: {err:#}");
            return Ok(nucleo_json);
        }
    };
    let Ok(fts) = serde_json::from_str::<serde_json::Value>(&fts_json) else {
        return Ok(nucleo_json);
    };
    let mut merged = nucleo;
    for key in ["songs", "artists", "albums"] {
        if let (Some(primary), Some(secondary)) = (merged.get_mut(key), fts.get(key)) {
            merge_json_rows(primary, secondary, VISIBLE);
        }
    }
    Ok(merged.to_string())
}

/// Append the FTS-only rows to the Nucleo array, keeping the Nucleo order and
/// the visible cap. Rows are identified by their `id`.
fn merge_json_rows(primary: &mut serde_json::Value, secondary: &serde_json::Value, limit: usize) {
    let Some(primary_rows) = primary.as_array_mut() else {
        return;
    };
    let mut seen: HashSet<String> = primary_rows.iter().filter_map(json_row_id).collect();
    if let Some(secondary_rows) = secondary.as_array() {
        for row in secondary_rows {
            if primary_rows.len() >= limit {
                break;
            }
            if let Some(id) = json_row_id(row)
                && !seen.insert(id)
            {
                continue;
            }
            primary_rows.push(row.clone());
        }
    }
    primary_rows.truncate(limit);
}

fn json_row_id(row: &serde_json::Value) -> Option<String> {
    row.get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

async fn search_json_fts(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let filter = offline_filter(dbs, offline).await?;
    // Offline hits are filtered down to the downloaded catalog, so over-fetch
    // before truncating back to the visible 20.
    let limit = if filter.is_some() { 60 } else { 20 };
    // S-9: an empty query is the "Most played" surface, not three dead
    // columns. Punctuation is ignored for matching (`(hello` == `hello`),
    // but the payload keeps the raw text so the field's stale check works.
    let cleaned = crate::search::clean_query(query);
    let most_played = cleaned.is_empty();
    let (artists, albums, mut songs) = if most_played {
        (
            Vec::new(),
            Vec::new(),
            crate::search::most_played(&dbs.search, limit).await?,
        )
    } else {
        let (artists, albums) =
            crate::library::search_artists_albums(&dbs.library, &cleaned, limit).await?;
        let songs = crate::search::search(&dbs.search, &cleaned, limit).await?;
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
    artists.truncate(20);
    albums.truncate(20);
    if let Some(filter) = &filter {
        songs.retain(|song| filter.songs.contains(&song.song_id));
    }
    songs.truncate(20);

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

    search_payload(
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

async fn search_json_index(
    dbs: &crate::db::Dbs,
    query: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let filter = offline_filter(dbs, offline).await?;
    let limit = if filter.is_some() { 60 } else { 20 };
    let cleaned = crate::search::clean_query(query);
    let most_played = cleaned.is_empty();
    let index = crate::search::index::ensure(&dbs.library).await?;
    let (mut artists, mut albums, mut songs) = if most_played {
        (
            Vec::new(),
            Vec::new(),
            index
                .most_played(limit)
                .iter()
                .map(|song| SearchSong::from_index(song))
                .collect::<Vec<_>>(),
        )
    } else {
        // Nucleo's tick blocks for the worker; keep it off the async workers.
        let index = std::sync::Arc::clone(&index);
        let cleaned = cleaned.clone();
        let results = tokio::task::spawn_blocking(move || index.search(&cleaned, limit)).await?;
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
    artists.truncate(20);
    albums.truncate(20);
    // S-8: one album match turns the Songs column into that album's
    // tracklist. The index already keeps the tracks sorted by disc/track.
    let album_context = (!most_played && albums.len() == 1).then(|| albums[0].id.clone());
    if let Some(album_id) = &album_context {
        songs = index
            .album_tracks(album_id, limit)
            .iter()
            .map(|song| SearchSong::from_index(song))
            .collect();
        if let Some(filter) = &filter {
            songs.retain(|song| filter.songs.contains(&song.id));
        }
    }
    songs.truncate(20);

    search_payload(
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

async fn search_payload(
    dbs: &crate::db::Dbs,
    query: &str,
    most_played: bool,
    artists: Vec<std::sync::Arc<crate::library::ArtistRow>>,
    albums: Vec<std::sync::Arc<crate::library::AlbumRow>>,
    songs: Vec<SearchSong>,
    album_context: Option<String>,
) -> anyhow::Result<String> {
    let mut cover_ids: Vec<String> = albums
        .iter()
        .filter_map(|album| album.cover_art.clone())
        .collect();
    cover_ids.extend(artists.iter().filter_map(|artist| artist.cover_art.clone()));
    cover_ids.extend(songs.iter().filter_map(|song| song.cover_art.clone()));
    let covers = crate::covers::url_map_of(&dbs.covers, &cover_ids);
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
    })
    .to_string())
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
             INSERT INTO albums (id, name, artist, song_count)
                 VALUES ('al1', 'Kid A', 'Radiohead', 2);
             INSERT INTO songs
                 (id, album_id, title, artist, album, track, disc_number, duration, play_count)
             VALUES
                 ('s1', 'al1', 'Everything In Its Right Place', 'Radiohead', 'Kid A', 1, 1, 240, 0),
                 ('s2', 'al1', 'Idioteque', 'Radiohead', 'Kid A', 2, 1, 300, 5);",
        )
        .execute(&dbs.library)
        .await
        .unwrap();

        let json = search_json_index(&dbs, "rdo", false).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["most_played"], false);
        assert_eq!(parsed["album_context"], "al1");
        let ids: Vec<&str> = parsed["songs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|song| song["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["s1", "s2"], "one album match reads in track order");

        let json = search_json_index(&dbs, "", false).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["most_played"], true);
        assert!(parsed["album_context"].is_null());
        assert_eq!(parsed["songs"][0]["id"], "s2");

        crate::search::index::clear();
    }

    #[test]
    fn hybrid_merge_keeps_primary_order_and_dedupes() {
        let mut primary = serde_json::json!([{"id": "a"}, {"id": "b"}]);
        let secondary = serde_json::json!([{"id": "b"}, {"id": "c"}, {"id": "d"}]);
        merge_json_rows(&mut primary, &secondary, 3);
        let ids: Vec<&str> = primary
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
