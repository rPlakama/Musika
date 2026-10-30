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

use serde::ser::{Error as _, Serialize, SerializeSeq, SerializeStruct, Serializer};

use crate::http_client::SubsonicClient;

/// CACHE-1 local-first: a cached blob is handed to mpv as a `file://` URL,
/// otherwise the stream URL is used. The network is still only touched by
/// mpv; "prefer server online" is CACHE-6.
pub(crate) fn track_url(
    client: &SubsonicClient,
    song: &crate::library::SongRow,
    media: &HashMap<String, PathBuf>,
) -> anyhow::Result<String> {
    let url = match media.get(&song.id) {
        Some(path) => reqwest::Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("cached media path is not a valid file URL"))?,
        None => client.stream_url(&song.id)?,
    };
    Ok(url.to_string())
}

/// One `player::Track` row, streamed straight from its `SongRow`.
///
/// The old path built a full `Vec<Track>` (moving every string) and then
/// serialized it, so a library-wide queue held both the row vector and the
/// JSON string. Borrowing the row and only owning the computed stream URL
/// removes that intermediate vector.
struct TrackElement<'a> {
    song: &'a crate::library::SongRow,
    url: &'a str,
    cover_url: &'a str,
}

impl Serialize for TrackElement<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("track", 15)?;
        state.serialize_field("id", &self.song.id)?;
        state.serialize_field("title", &self.song.title)?;
        state.serialize_field("artist", self.song.artist.as_deref().unwrap_or_default())?;
        state.serialize_field("album", self.song.album.as_deref().unwrap_or_default())?;
        state.serialize_field(
            "album_id",
            self.song.album_id.as_deref().unwrap_or_default(),
        )?;
        state.serialize_field("favorite", &self.song.starred.is_some())?;
        state.serialize_field("duration", &(self.song.duration.unwrap_or(0) as f64))?;
        state.serialize_field("url", self.url)?;
        state.serialize_field(
            "cover_art",
            self.song.cover_art.as_deref().unwrap_or_default(),
        )?;
        state.serialize_field("cover_url", self.cover_url)?;
        state.serialize_field("subtitle", self.song.artist.as_deref().unwrap_or_default())?;
        state.serialize_field("meta", &crate::browse::rows::format_duration(self.song.duration))?;
        state.serialize_field("songId", &self.song.id)?;
        state.serialize_field("albumId", self.song.album_id.as_deref().unwrap_or_default())?;
        state.serialize_field("starred", &self.song.starred.is_some())?;
        state.end()
    }
}

pub(crate) struct TrackRows<'a> {
    pub(crate) songs: &'a [crate::library::SongRow],
    pub(crate) client: &'a SubsonicClient,
    pub(crate) covers: &'a HashMap<String, String>,
    pub(crate) media: &'a HashMap<String, PathBuf>,
}

impl Serialize for TrackRows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.songs.len()))?;
        for song in self.songs {
            let url = track_url(self.client, song, self.media).map_err(S::Error::custom)?;
            let cover_url = crate::covers::url_for_ref(song.cover_art.as_deref(), self.covers);
            seq.serialize_element(&TrackElement {
                song,
                url: &url,
                cover_url,
            })?;
        }
        seq.end()
    }
}

/// CACHE-9: while offline, only downloaded songs are playable — drop the
/// rest from a queue that is about to be built.
pub async fn album_tracks_json(
    dbs: &crate::db::Dbs,
    album_id: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = crate::library::album_songs(&dbs.library, album_id).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks_json(dbs, songs).await
}

/// The playlist queue: playlist order preserved, duplicate tracks kept,
/// missing/offline rows skipped, and the clicked row translated to its
/// playable queue index.
pub async fn playlist_tracks_json(
    dbs: &crate::db::Dbs,
    playlist_id: &str,
    offline: bool,
    start_row: i64,
) -> anyhow::Result<(String, i64)> {
    let ids: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT song_id FROM playlist_song WHERE playlist_id = ?1 ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(&dbs.playlist)
    .await?;
    let unique: Vec<String> = ids.iter().flatten().cloned().collect();
    let library = crate::library::songs_by_ids(&dbs.library, &unique).await?;
    let by_id: std::collections::HashMap<String, crate::library::SongRow> =
        library.into_iter().map(|song| (song.id.clone(), song)).collect();
    let filter = crate::offline::load(dbs, offline).await?;
    let mut songs = Vec::new();
    let mut start = 0i64;
    for (position, id) in ids.iter().enumerate() {
        let Some(id) = id.as_ref() else { continue };
        let Some(song) = by_id.get(id) else { continue };
        if filter.as_ref().is_some_and(|filter| !filter.contains_song(id)) {
            continue;
        }
        if position < start_row.max(0) as usize {
            start += 1;
        }
        songs.push(song.clone());
    }
    Ok((tracks_json(dbs, songs).await?, start))
}

/// The artist's shuffle queue: the artist's albums (the albums table's
/// mapping) Fisher–Yates shuffled, each expanding to its tracklist — one
/// album after another until the array ends. The queue holds the songs of
/// that order, header first.
pub async fn artist_shuffle_json(
    dbs: &crate::db::Dbs,
    artist_id: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let mut albums = crate::library::artist_album_ids(&dbs.library, artist_id).await?;
    if albums.is_empty() {
        return Err(anyhow::anyhow!("this artist has no albums to shuffle"));
    }
    let seed = crate::consts::now_nanos();
    shuffle_units(&mut albums, seed);
    let mut songs: Vec<crate::library::SongRow> = Vec::new();
    for album in &albums {
        songs.extend(crate::library::album_songs(&dbs.library, album).await?);
    }
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks_json(dbs, songs).await
}

/// A seeded SplitMix64 — the shuffle's only randomness source. Deterministic
/// for a given seed (tests), statistically independent across clock seeds.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Lemire's multiply-shift: a near-uniform index in `0..len` (the
    /// modulo bias is below `len / 2^64`, irrelevant at these sizes).
    fn below(&mut self, len: usize) -> usize {
        ((self.next() as u128 * len as u128) >> 64) as usize
    }
}

/// A proper array shuffle: Fisher–Yates (Knuth), in place. Every index is
/// drawn exactly once, so the result is an unbiased permutation — the
/// library order is not preserved, nor is the previous cycle's. `seed` is
/// the clock; the caller passes `now()`.
pub fn shuffle_units(units: &mut [String], seed: u128) {
    let mut rng = SplitMix64((seed as u64) ^ ((seed >> 64) as u64));
    for i in (1..units.len()).rev() {
        units.swap(i, rng.below(i + 1));
    }
}

/// The whole shuffle queue: every unit of the shuffled source (album ids
/// when `kind` is "albums", song ids otherwise) resolves in one pass. The
/// queue loads complete — nothing appends mid-session, so the models never
/// republish rows while playing (a rolling window's republish was the
/// queue-follow flicker).
pub async fn shuffle_queue_json(
    dbs: &crate::db::Dbs,
    kind: &str,
    units: &[String],
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = if kind == "albums" {
        let mut songs: Vec<crate::library::SongRow> = Vec::new();
        for album in units {
            songs.extend(crate::library::album_songs(&dbs.library, album).await?);
        }
        songs
    } else {
        // One bulk query, in the shuffled order (`songs_by_ids` preserves the
        // input order), never a query per unit.
        crate::library::songs_by_ids(&dbs.library, units).await?
    };
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks_json(dbs, songs).await
}

/// 0-based start row for the Songs-segment queue: the clicked song must exist
/// in the index (a stale id is an error, not a silent head fallback).
pub(crate) fn library_start(
    songs: &[crate::library::SongRow],
    song_id: &str,
) -> anyhow::Result<usize> {
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
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    let start = library_start(&songs, song_id)?;
    Ok((tracks_json(dbs, songs).await?, start))
}

/// Index of `song_id` in its album's track order (0 when it vanished from a
/// stale index): the queue must still start somewhere sensible.
pub(crate) fn queue_start(songs: &[crate::library::SongRow], song_id: &str) -> i64 {
    songs
        .iter()
        .position(|song| song.id == song_id)
        .unwrap_or(0) as i64
}

/// P-6 S3: after stale ids are dropped from a queue snapshot, resume on the
/// remembered track when it survived, else on the first track (the S2
/// "unknown id" rule).
pub(crate) fn snapshot_start(ids: &[String], songs: &[crate::library::SongRow], index: i64) -> i64 {
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
    crate::browse::album_json(album, covers, None)
}

/// Single pumped event for "play this song inside its album": the serialized
/// queue and the start index. The album page is a separate `open_album`
/// event (`song_album_page_json`) emitted first, so the page is not held back
/// by the queue serialization. `album` stays in the payload as null for shape
/// compatibility.
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
        crate::offline::retain_songs(dbs, offline, &mut songs).await?;
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
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    let start = queue_start(&songs, &song.id);
    let queue = tracks_json(dbs, songs).await?;

    // The album page is emitted first as `open_album`; the queue event
    // carries only the tracks so the page is not held back by the cover
    // lookup. The field stays for payload-shape compatibility.
    Ok(serde_json::json!({
        "queue": queue,
        "start": start,
        "album": serde_json::Value::Null,
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
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks_json(dbs, songs).await
}

/// VI-4: expands one visual selection (same kind, several ids) to rows.
pub(crate) async fn quick_songs_many(
    dbs: &crate::db::Dbs,
    kind: &str,
    ids: &[String],
) -> anyhow::Result<Vec<crate::library::SongRow>> {
    let mut songs = Vec::new();
    for id in ids {
        songs.extend(quick_songs(dbs, kind, id).await?);
    }
    Ok(songs)
}

/// Batch twin of [`quick_tracks_json`] for a visual selection (VI-4): the
/// selected ids are resolved and offline-filtered as one queue append.
pub async fn quick_tracks_json_many(
    dbs: &crate::db::Dbs,
    kind: &str,
    ids: &[String],
    offline: bool,
) -> anyhow::Result<String> {
    let mut songs = quick_songs_many(dbs, kind, ids).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
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
    let cover_ids: Vec<String> = album.cover_art.clone().into_iter().collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
    let mut page = queue_album_json(&album, &covers);
    // The page may open from a song hit or the Go to Album verb; carrying
    // the song id lets AlbumEntry park its cursor on that row.
    page["song_id"] = serde_json::json!(song_id);
    Ok(page.to_string())
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
    let cover_ids: Vec<String> = artist.cover_art.clone().into_iter().collect();
    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
    let cover = crate::covers::url_for(artist.cover_art.as_deref(), &covers);
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
    // CACHE-9: offline, only downloaded content comes back; the album page
    // reopens only when it owns downloads.
    let offline_albums = if offline {
        let songs = crate::download::downloaded_song_ids(dbs).await?;
        let ids: Vec<String> = songs.into_iter().collect();
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
            match crate::library::album(&dbs.library, album_id).await? {
                Some(album) => {
                    let cover_ids: Vec<String> = album.cover_art.clone().into_iter().collect();
                    let covers = crate::covers::url_map_of(dbs, &cover_ids).await?;
                    Some(queue_album_json(&album, &covers))
                }
                None => None,
            }
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
        crate::offline::retain_songs(dbs, offline, &mut songs).await?;
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
            crate::offline::retain_songs(dbs, offline, &mut songs).await?;
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
    let client = crate::session::client(dbs).await?;
    let covers = crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
    let song_ids: Vec<String> = songs.iter().map(|song| song.id.clone()).collect();
    let media = crate::media::path_map_of(&dbs.media, &song_ids)
        .await
        .unwrap_or_default();

    Ok(serde_json::to_string(&TrackRows {
        songs: &songs,
        client: &client,
        covers: &covers,
        media: &media,
    })?)
}
