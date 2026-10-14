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

/// 0-based start row for the Songs-segment queue: the clicked song must exist
/// in the index (a stale id is an error, not a silent head fallback).
fn library_start(songs: &[crate::library::SongRow], song_id: &str) -> anyhow::Result<usize> {
    songs
        .iter()
        .position(|song| song.id == song_id)
        .ok_or_else(|| anyhow::anyhow!("song not found in index"))
}

/// Complete Songs-segment queue: every indexed song in `order`, with `song_id`
/// as the starting row. The whole index (not a forward window) is queued so
/// the tracks before the clicked one stay reachable; the rows are local, so
/// there is nothing to extend lazily.
pub async fn library_queue_payload(
    dbs: &crate::db::Dbs,
    song_id: &str,
    order: &str,
) -> anyhow::Result<(String, usize)> {
    let songs = crate::library::all_songs(&dbs.library, order).await?;
    let start = library_start(&songs, song_id)?;
    Ok((tracks_json(dbs, songs).await?, start))
}

/// Index of `song_id` in its album's track order (0 when it vanished from a
/// stale index): the queue must still start somewhere sensible.
fn queue_start(songs: &[crate::library::SongRow], song_id: &str) -> i64 {
    songs
        .iter()
        .position(|song| song.id == song_id)
        .unwrap_or(0) as i64
}

/// P-6 S3: after stale ids are dropped from a queue snapshot, resume on the
/// remembered track when it survived, else on the first track (the S2
/// "unknown id" rule).
fn snapshot_start(ids: &[String], songs: &[crate::library::SongRow], index: i64) -> i64 {
    let current = ids
        .get(index.max(0) as usize)
        .map(String::as_str)
        .unwrap_or_default();
    queue_start(songs, current)
}

/// Album metadata for UI pages (AlbumEntry header), cover resolved.
fn queue_album_json(
    album: &crate::library::AlbumRow,
    covers: &HashMap<String, String>,
) -> serde_json::Value {
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
    let start = queue_start(&songs, &song.id);
    let queue = tracks_json(dbs, songs).await?;

    let album = match crate::library::album(&dbs.library, album_id).await? {
        Some(album) => {
            let covers = crate::covers::url_map(&dbs.covers).await?;
            queue_album_json(&album, &covers)
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

/// S2/S3 resume payload (P-6): validates the remembered ids against the
/// local index and returns `{album, queue, start, position}` — the album
/// page to reopen (or null) and the restored queue (or ""), already pointing
/// at the remembered track, plus the saved playhead. The S3 queue snapshot
/// wins; pre-S3 rows fall back to the song's album queue. Unknown ids are
/// dropped silently, exactly like a first launch.
pub async fn resume_payload(
    dbs: &crate::db::Dbs,
    status: &crate::status::SessionStatus,
) -> anyhow::Result<String> {
    let covers = crate::covers::url_map(&dbs.covers).await?;

    let album = match status.album_id.as_deref().filter(|id| !id.is_empty()) {
        Some(album_id) => crate::library::album(&dbs.library, album_id)
            .await?
            .map(|album| queue_album_json(&album, &covers)),
        None => None,
    };

    let mut queue = String::new();
    let mut start = 0i64;
    // S3: the queue snapshot (exact order, manual edits included) wins over
    // the S2 song -> album rebuild. Stale ids are dropped and the index
    // follows the remembered current song.
    let snapshot: Vec<String> = serde_json::from_str(&status.queue).unwrap_or_default();
    if !snapshot.is_empty() {
        let songs = crate::library::songs_by_ids(&dbs.library, &snapshot).await?;
        if !songs.is_empty() {
            let index = snapshot_start(&snapshot, &songs, status.queue_index);
            // No credentials yet (or a stream URL failure) degrades to just
            // the album page: resume must never fail the login.
            match tracks_json(dbs, songs).await {
                Ok(json) => {
                    queue = json;
                    start = index;
                }
                Err(err) => {
                    eprintln!("status: could not rebuild the resumed queue: {err:#}");
                }
            }
        }
    } else if let Some(song_id) = status.song_id.as_deref().filter(|id| !id.is_empty()) {
        // S2 fallback for rows without a snapshot (pre-S3 writers).
        if let Some(song) = crate::library::song(&dbs.library, song_id).await? {
            let (songs, index) = match song.album_id.as_deref().filter(|id| !id.is_empty()) {
                Some(album_id) => {
                    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
                    let index = queue_start(&songs, &song.id);
                    (songs, index)
                }
                None => (vec![song], 0),
            };
            match tracks_json(dbs, songs).await {
                Ok(json) => {
                    queue = json;
                    start = index;
                }
                Err(err) => {
                    eprintln!("status: could not rebuild the resumed queue: {err:#}");
                }
            }
        }
    }

    Ok(serde_json::json!({
        "album": album.unwrap_or(serde_json::Value::Null),
        "queue": queue,
        "start": start,
        "position": status.position,
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

    use super::*;

    fn song_row(id: &str) -> crate::library::SongRow {
        crate::library::SongRow {
            id: id.to_string(),
            title: id.to_string(),
            artist: Some("Radiohead".to_string()),
            album: Some("Kid A".to_string()),
            album_id: Some("al1".to_string()),
            track: Some(1),
            duration: Some(200),
            cover_art: None,
        }
    }

    #[test]
    fn queue_start_selects_the_row_or_zero() {
        let rows = vec![song_row("s1"), song_row("s2")];
        assert_eq!(queue_start(&rows, "s2"), 1);
        // A stale id must still produce a playable queue.
        assert_eq!(queue_start(&rows, "gone"), 0);
    }

    #[test]
    fn library_start_requires_the_clicked_row() {
        let rows = vec![song_row("s1"), song_row("s2"), song_row("s3")];
        assert_eq!(library_start(&rows, "s3").unwrap(), 2);
        // The Songs segment never silently retargets a stale id.
        assert!(library_start(&rows, "gone").is_err());
    }

    #[test]
    fn snapshot_start_survives_dropped_ids() {
        let ids = ["s1".to_string(), "gone".to_string(), "s2".to_string()];
        let rows = vec![song_row("s1"), song_row("s2")];
        // The remembered track survived: resume on it.
        assert_eq!(snapshot_start(&ids, &rows, 2), 1);
        // The remembered track itself is gone: fall back to the head.
        assert_eq!(snapshot_start(&ids, &rows, 1), 0);
        // A truncated snapshot lands on the head too.
        assert_eq!(snapshot_start(&ids, &rows, 9), 0);
    }

    async fn memory_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap()
    }

    async fn test_dbs() -> crate::db::Dbs {
        let credentials = memory_pool().await;
        sqlx::migrate!("./migrations")
            .set_ignore_missing(true)
            .run(&credentials)
            .await
            .unwrap();
        let library = memory_pool().await;
        sqlx::migrate!("./migrations_library")
            .run(&library)
            .await
            .unwrap();
        let covers = memory_pool().await;
        sqlx::migrate!("./migrations_covers")
            .run(&covers)
            .await
            .unwrap();
        crate::db::Dbs {
            credentials,
            library,
            search: memory_pool().await,
            covers,
            lyrics: memory_pool().await,
            colors: memory_pool().await,
            status: memory_pool().await,
        }
    }

    #[tokio::test]
    async fn resume_payload_validates_ids_against_the_index() {
        let dbs = test_dbs().await;
        for (id, artist_id, name, artist, year, songs) in [
            ("al1", "ar1", "Kid A", "Radiohead", 2000, 2),
            ("al2", "ar2", "Absolution", "Muse", 2003, 1),
        ] {
            sqlx::query(
                "INSERT INTO albums (id, artist_id, name, artist, year, song_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(id)
            .bind(artist_id)
            .bind(name)
            .bind(artist)
            .bind(year)
            .bind(songs)
            .execute(&dbs.library)
            .await
            .unwrap();
        }
        for (id, album_id, title, track) in
            [("s1", "al1", "Everything", 1), ("s2", "al1", "Kid A", 2)]
        {
            sqlx::query(
                "INSERT INTO songs (id, album_id, artist_id, title, artist, album, track, disc_number)
                 VALUES (?1, ?2, 'ar1', ?3, 'Radiohead', 'Kid A', ?4, 1)",
            )
            .bind(id)
            .bind(album_id)
            .bind(title)
            .bind(track)
            .execute(&dbs.library)
            .await
            .unwrap();
        }

        // Known ids: the album page comes back; without credentials the
        // queue degrades to empty instead of failing the restore.
        let status = crate::status::SessionStatus {
            album_id: Some("al1".to_string()),
            song_id: Some("s2".to_string()),
            position: 12.5,
            ..crate::status::SessionStatus::default()
        };
        let payload = resume_payload(&dbs, &status).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["album"]["id"], "al1");
        assert_eq!(value["album"]["tracks"], 2);
        assert_eq!(value["queue"], "");
        assert_eq!(value["start"], 0);
        assert_eq!(value["position"], 12.5);

        // Unknown ids behave exactly like a first launch.
        let status = crate::status::SessionStatus {
            album_id: Some("gone".to_string()),
            song_id: Some("gone".to_string()),
            ..crate::status::SessionStatus::default()
        };
        let payload = resume_payload(&dbs, &status).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(value["album"].is_null());
        assert_eq!(value["queue"], "");
        assert_eq!(value["start"], 0);
        assert_eq!(value["position"], 0.0);
    }
}
