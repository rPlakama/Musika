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

//! Playback queue construction for the Qt bridge (ARC-17): library rows ->
//! serialized `player::Track` queues. List/search payloads live in
//! [`crate::browse`].
//!
//! Every queue is built from the local index only — `library.db` rows,
//! `covers.db` URL map and the locally decrypted credentials; `stream_url`
//! is pure URL construction. The network is only touched later by mpv.

use std::collections::HashMap;

use crate::http_client::SubsonicClient;

/// Songs-segment lazy queue: load this many tracks up front, then extend by
/// at most [`LIBRARY_QUEUE_CHUNK`] once [`LIBRARY_QUEUE_AHEAD`] remain (i.e.
/// playback reaches track 190 of a 200-track window; each later extension
/// pre-loads no more than 5 songs forward). The window is cheap (ids/rows
/// from the local index), so it is sized to absorb skip bursts.
pub const LIBRARY_QUEUE_INITIAL: usize = 200;
pub const LIBRARY_QUEUE_CHUNK: usize = 5;
pub const LIBRARY_QUEUE_AHEAD: usize = 10;

/// Lazy-extension cursor for the Songs-segment queue (owned by `Backend`).
pub struct LibraryQueue {
    /// Absolute offset in `library::all_songs` of the next chunk.
    pub next: usize,
    pub total: usize,
    /// Order label the queue was built with; extensions must keep it.
    pub order: String,
    /// A chunk fetch is in flight; don't stack a second one.
    pub loading: bool,
}

/// Builds a playable [`crate::player::Track`] from an indexed song.
fn build_track(
    client: &SubsonicClient,
    song: crate::library::SongRow,
    covers: &HashMap<String, String>,
) -> anyhow::Result<crate::player::Track> {
    let url = client.stream_url(&song.id)?;
    let cover_art = song.cover_art.unwrap_or_default();
    let cover_url = covers.get(&cover_art).cloned().unwrap_or_default();
    Ok(crate::player::Track {
        id: song.id,
        title: song.title,
        artist: song.artist.unwrap_or_default(),
        album: song.album.unwrap_or_default(),
        duration: song.duration.unwrap_or(0) as f64,
        url: url.to_string(),
        cover_art,
        cover_url,
    })
}

pub async fn album_tracks_json(dbs: &crate::db::Dbs, album_id: &str) -> anyhow::Result<String> {
    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
    tracks_json(dbs, songs).await
}

/// First Songs-segment window (200 tracks from `song_id`) plus the extension
/// cursor. The ordinal and the page come straight from the indexed rows
/// (`ROW_NUMBER` / `LIMIT OFFSET`), so no full library is materialized and
/// nothing is fetched from the server here.
pub async fn library_queue_payload(
    dbs: &crate::db::Dbs,
    song_id: &str,
    order: &str,
) -> anyhow::Result<(String, usize, usize)> {
    let Some(start) = crate::library::song_ordinal(&dbs.library, song_id, order).await? else {
        return Err(anyhow::anyhow!("song not found in index"));
    };
    let total = crate::library::song_count(&dbs.library).await?;
    let songs = crate::library::songs_range(
        &dbs.library,
        start as i64,
        LIBRARY_QUEUE_INITIAL as i64,
        order,
    )
    .await?;
    let next = start + songs.len();
    Ok((tracks_json(dbs, songs).await?, next, total))
}

/// Next lazy window from `offset` in the `all_songs` order (local index,
/// `LIMIT`/`OFFSET` — same rows and order the DB already holds).
pub async fn library_chunk_json(
    dbs: &crate::db::Dbs,
    offset: usize,
    order: &str,
) -> anyhow::Result<(String, usize, usize)> {
    let total = crate::library::song_count(&dbs.library).await?;
    let songs = crate::library::songs_range(
        &dbs.library,
        offset as i64,
        LIBRARY_QUEUE_CHUNK as i64,
        order,
    )
    .await?;
    let next = offset + songs.len();
    Ok((tracks_json(dbs, songs).await?, next, total))
}

/// Single pumped event for "play this song inside its album": the serialized
/// queue, the start index and the album metadata (or null).
pub async fn song_album_payload(dbs: &crate::db::Dbs, song_id: &str) -> anyhow::Result<String> {
    let Some(song) = crate::library::song(&dbs.library, song_id).await? else {
        return Err(anyhow::anyhow!("song not found in index"));
    };
    let Some(album_id) = song.album_id.as_deref().filter(|id| !id.is_empty()) else {
        let queue = tracks_json(dbs, vec![song]).await?;
        return Ok(serde_json::json!({
            "queue": queue,
            "start": 0,
            "album": serde_json::Value::Null,
        })
        .to_string());
    };

    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
    let start = songs.iter().position(|s| s.id == song.id).unwrap_or(0) as i64;
    let queue = tracks_json(dbs, songs).await?;

    let album = match crate::library::album(&dbs.library, album_id).await? {
        Some(album) => {
            let covers = crate::covers::url_map(&dbs.covers).await?;
            let cover = album
                .cover_art
                .as_deref()
                .and_then(|id| covers.get(id))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "id": album.id,
                "title": album.title,
                "artist": album.artist,
                "year": album.year,
                "tracks": album.song_count,
                "cover": cover,
            })
        }
        None => serde_json::Value::Null,
    };

    Ok(serde_json::json!({
        "queue": queue,
        "start": start,
        "album": album,
    })
    .to_string())
}

async fn tracks_json(
    dbs: &crate::db::Dbs,
    songs: Vec<crate::library::SongRow>,
) -> anyhow::Result<String> {
    let credentials = crate::sqlite::load_credentials(&dbs.credentials).await?;
    let Some(credentials) = credentials else {
        return Err(anyhow::anyhow!("no saved credentials to stream with"));
    };
    let client = SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    );
    let covers = crate::covers::url_map(&dbs.covers).await?;

    let mut tracks = Vec::with_capacity(songs.len());
    for song in songs {
        tracks.push(build_track(&client, song, &covers)?);
    }
    Ok(serde_json::to_string(&tracks)?)
}
