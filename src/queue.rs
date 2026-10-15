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
use std::path::PathBuf;

use crate::http_client::SubsonicClient;

/// Builds a playable [`crate::player::Track`] from an indexed song.
///
/// CACHE-1 local-first: a cached blob is handed to mpv as a `file://` URL,
/// otherwise the stream URL is used. The network is still only touched by
/// mpv; "prefer server online" is CACHE-6.
fn build_track(
    client: &SubsonicClient,
    song: crate::library::SongRow,
    covers: &HashMap<String, String>,
    media: &HashMap<String, PathBuf>,
) -> anyhow::Result<crate::player::Track> {
    let url = match media.get(&song.id) {
        Some(path) => reqwest::Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("cached media path is not a valid file URL"))?,
        None => client.stream_url(&song.id)?,
    };
    let cover_art = song.cover_art.unwrap_or_default();
    let cover_url = covers.get(&cover_art).cloned().unwrap_or_default();
    Ok(crate::player::Track {
        id: song.id,
        title: song.title,
        artist: song.artist.unwrap_or_default(),
        album: song.album.unwrap_or_default(),
        album_id: song.album_id.unwrap_or_default(),
        favorite: song.starred.is_some(),
        duration: song.duration.unwrap_or(0) as f64,
        url: url.to_string(),
        cover_art,
        cover_url,
    })
}

/// CACHE-9: while offline, only downloaded songs are playable — drop the
/// rest from a queue that is about to be built.
async fn filter_offline(
    dbs: &crate::db::Dbs,
    offline: bool,
    songs: &mut Vec<crate::library::SongRow>,
) -> anyhow::Result<()> {
    if !offline {
        return Ok(());
    }
    let done: std::collections::HashSet<String> = crate::download::done_ids(&dbs.download)
        .await?
        .into_iter()
        .collect();
    songs.retain(|song| done.contains(&song.id));
    Ok(())
}

pub async fn album_tracks_json(
    dbs: &crate::db::Dbs,
    album_id: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = crate::library::album_songs(&dbs.library, album_id).await?;
    filter_offline(dbs, offline, &mut songs).await?;
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
    offline: bool,
) -> anyhow::Result<(String, usize)> {
    let mut songs = crate::library::all_songs(&dbs.library, order).await?;
    filter_offline(dbs, offline, &mut songs).await?;
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
        // UI-56/ARC-I-2: the album page header's favourite mark.
        "starred": album.starred.is_some(),
    })
}

/// Single pumped event for "play this song inside its album": the serialized
/// queue, the start index and the album metadata (or null).
pub async fn song_album_payload(
    dbs: &crate::db::Dbs,
    song_id: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let Some(song) = crate::library::song(&dbs.library, song_id).await? else {
        return Err(anyhow::anyhow!("song not found in index"));
    };
    let Some(album_id) = song.album_id.as_deref().filter(|id| !id.is_empty()) else {
        let mut songs = vec![song];
        filter_offline(dbs, offline, &mut songs).await?;
        if songs.is_empty() {
            return Err(anyhow::anyhow!("song is not downloaded"));
        }
        let queue = tracks_json(dbs, songs).await?;
        return Ok(serde_json::json!({
            "queue": queue,
            "start": 0,
            "album": serde_json::Value::Null,
        })
        .to_string());
    };

    let mut songs = crate::library::album_songs(&dbs.library, album_id).await?;
    filter_offline(dbs, offline, &mut songs).await?;
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

/// Rows for the ARC-43 "Add to Queue" overlay: one song or a whole album.
pub async fn quick_songs(
    dbs: &crate::db::Dbs,
    kind: &str,
    id: &str,
) -> anyhow::Result<Vec<crate::library::SongRow>> {
    match kind {
        "song" => Ok(crate::library::song(&dbs.library, id)
            .await?
            .into_iter()
            .collect()),
        "album" => crate::library::album_songs(&dbs.library, id).await,
        "artist" => crate::library::artist_songs(&dbs.library, id).await,
        other => Err(anyhow::anyhow!("unknown quick-action kind: {other}")),
    }
}

/// Serialized tracks for the ARC-43 "Add to Queue" overlay (the overlay is
/// offline-filtered like every other queue).
pub async fn quick_tracks_json(
    dbs: &crate::db::Dbs,
    kind: &str,
    id: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = quick_songs(dbs, kind, id).await?;
    filter_offline(dbs, offline, &mut songs).await?;
    tracks_json(dbs, songs).await
}

/// Album-page payload for ARC-43 "Go to Album" (navigation only, no queue):
/// the same JSON shape [`queue_album_json`] gives the album page.
pub async fn song_album_page_json(dbs: &crate::db::Dbs, song_id: &str) -> anyhow::Result<String> {
    let song = crate::library::song(&dbs.library, song_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("song not found in index"))?;
    let album_id = song
        .album_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("song has no album"))?;
    let album = crate::library::album(&dbs.library, album_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("album not found in index"))?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    Ok(queue_album_json(&album, &covers).to_string())
}

/// Artist-page payload for ARC-43 "Go to Artist" (navigation only).
pub async fn album_artist_page_json(
    dbs: &crate::db::Dbs,
    album_id: &str,
) -> anyhow::Result<String> {
    let artist_id = crate::library::album_artist_id(&dbs.library, album_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("album has no artist"))?;
    let artist = crate::library::artist(&dbs.library, &artist_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("artist not found in index"))?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    let cover = artist
        .cover_art
        .as_deref()
        .and_then(|id| covers.get(id))
        .cloned()
        .unwrap_or_default();
    Ok(serde_json::json!({
        "id": artist.id,
        "name": artist.name,
        "album_count": artist.album_count,
        "cover": cover,
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
    offline: bool,
) -> anyhow::Result<String> {
    let covers = crate::covers::url_map(&dbs.covers).await?;

    // CACHE-9: offline, only downloaded content comes back; the album page
    // reopens only when it owns downloads.
    let offline_albums = if offline {
        let ids = crate::download::done_ids(&dbs.download).await?;
        Some(
            crate::library::downloaded_owners(&dbs.library, &ids)
                .await?
                .0,
        )
    } else {
        None
    };

    let album = match status.album_id.as_deref().filter(|id| !id.is_empty()) {
        Some(album_id)
            if offline_albums
                .as_ref()
                .map_or(true, |albums| albums.contains_key(album_id)) =>
        {
            crate::library::album(&dbs.library, album_id)
                .await?
                .map(|album| queue_album_json(&album, &covers))
        }
        _ => None,
    };

    let mut queue = String::new();
    let mut start = 0i64;
    // S3: the queue snapshot (exact order, manual edits included) wins over
    // the S2 song -> album rebuild. Stale ids are dropped and the index
    // follows the remembered current song.
    let snapshot: Vec<String> = serde_json::from_str(&status.queue).unwrap_or_default();
    if !snapshot.is_empty() {
        let mut songs = crate::library::songs_by_ids(&dbs.library, &snapshot).await?;
        filter_offline(dbs, offline, &mut songs).await?;
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
            let song_id = song.id.clone();
            let mut songs = match song.album_id.as_deref().filter(|id| !id.is_empty()) {
                Some(album_id) => crate::library::album_songs(&dbs.library, album_id).await?,
                None => vec![song],
            };
            filter_offline(dbs, offline, &mut songs).await?;
            let index = queue_start(&songs, &song_id);
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
    let media = crate::media::path_map(&dbs.media).await.unwrap_or_default();

    let mut tracks = Vec::with_capacity(songs.len());
    for song in songs {
        tracks.push(build_track(&client, song, &covers, &media)?);
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
            starred: None,
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
        let download = memory_pool().await;
        sqlx::migrate!("./migrations_download")
            .run(&download)
            .await
            .unwrap();
        crate::db::Dbs {
            credentials,
            library,
            search: memory_pool().await,
            covers,
            media: memory_pool().await,
            lyrics: memory_pool().await,
            colors: memory_pool().await,
            status: memory_pool().await,
            artist_user: memory_pool().await,
            download,
        }
    }

    #[tokio::test]
    async fn offline_filter_keeps_only_downloaded_songs() {
        let dbs = test_dbs().await;
        crate::download::set_state(&dbs.download, "s1", crate::download::DONE, None)
            .await
            .unwrap();
        crate::download::set_state(&dbs.download, "s2", crate::download::ERROR, Some("x"))
            .await
            .unwrap();

        let mut songs = vec![song_row("s1"), song_row("s2"), song_row("s3")];
        filter_offline(&dbs, true, &mut songs).await.unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].id, "s1");

        // Online keeps everything.
        let mut songs = vec![song_row("s1"), song_row("s2")];
        filter_offline(&dbs, false, &mut songs).await.unwrap();
        assert_eq!(songs.len(), 2);
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
        let payload = resume_payload(&dbs, &status, false).await.unwrap();
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
        let payload = resume_payload(&dbs, &status, false).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(value["album"].is_null());
        assert_eq!(value["queue"], "");
        assert_eq!(value["start"], 0);
        assert_eq!(value["position"], 0.0);
    }

    #[tokio::test]
    async fn quick_actions_resolve_songs_albums_and_pages() {
        let dbs = test_dbs().await;
        for (id, artist_id, name) in [("al1", "ar1", "Kid A"), ("al2", "ar2", "Absolution")] {
            sqlx::query(
                "INSERT INTO albums (id, artist_id, name, artist, year, song_count)
                 VALUES (?1, ?2, ?3, 'Radiohead', 2000, 2)",
            )
            .bind(id)
            .bind(artist_id)
            .bind(name)
            .execute(&dbs.library)
            .await
            .unwrap();
        }
        sqlx::query("INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Radiohead', 2)")
            .execute(&dbs.library)
            .await
            .unwrap();
        for (id, album_id, title, track) in
            [("s1", "al1", "Everything", 1), ("s2", "al1", "Kid A", 2)]
        {
            sqlx::query(
                "INSERT INTO songs (id, album_id, artist_id, title, track)
                 VALUES (?1, ?2, 'ar1', ?3, ?4)",
            )
            .bind(id)
            .bind(album_id)
            .bind(title)
            .bind(track)
            .execute(&dbs.library)
            .await
            .unwrap();
        }

        // "Add to Queue" selection: one song, or the album's rows.
        let songs = quick_songs(&dbs, "song", "s2").await.unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].id, "s2");
        let songs = quick_songs(&dbs, "album", "al1").await.unwrap();
        assert_eq!(songs.len(), 2);
        assert!(quick_songs(&dbs, "bogus", "s2").await.is_err());

        // ARC-I: an artist hit queues every song of the artist's albums.
        let songs = quick_songs(&dbs, "artist", "ar1").await.unwrap();
        assert_eq!(songs.len(), 2);
        assert!(songs
            .iter()
            .all(|song| song.album_id.as_deref() == Some("al1")));
        assert!(quick_songs(&dbs, "artist", "gone")
            .await
            .unwrap()
            .is_empty());

        // ARC-I: the search payload album-id map (the search index carries no
        // album ids; the library lookup fills the Go to Artist jump).
        let albums = crate::library::song_album_ids(
            &dbs.library,
            &["s1".to_string(), "s2".to_string(), "gone".to_string()],
        )
        .await
        .unwrap();
        assert_eq!(albums.get("s1").map(String::as_str), Some("al1"));
        assert!(!albums.contains_key("gone"));

        // Navigation payloads carry the shape the pages consume.
        let json = song_album_page_json(&dbs, "s1").await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], "al1");
        assert_eq!(value["tracks"], 2);
        assert!(song_album_page_json(&dbs, "gone").await.is_err());

        let json = album_artist_page_json(&dbs, "al1").await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], "ar1");
        assert_eq!(value["name"], "Radiohead");
        assert!(album_artist_page_json(&dbs, "al2").await.is_err());
    }
}
