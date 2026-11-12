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
//! is pure URL construction. The network is only touched later by the native decoder.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::http_client::SubsonicClient;

/// CACHE-1 local-first: a cached blob is handed to the native decoder as a `file://` URL,
/// otherwise the stream URL is used. The network is still only touched by
/// native decoder; "prefer server online" is CACHE-6.
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

/// Build the `player::Track` rows every queue consumer shares.
///
/// The old path streamed a bespoke 16-field JSON shape and every consumer
/// decoded it straight back into this struct; the queue events carry
/// `Vec<Track>` themselves, so the mapping happens once, here.
pub(crate) fn build_tracks(
    client: &SubsonicClient,
    songs: &[crate::library::SongRow],
    covers: &HashMap<String, String>,
    media: &HashMap<String, PathBuf>,
) -> anyhow::Result<Vec<crate::player::Track>> {
    let mut tracks = Vec::with_capacity(songs.len());
    for song in songs {
        tracks.push(crate::player::Track {
            id: song.id.clone(),
            title: song.title.clone(),
            artist: song.artist.clone().unwrap_or_default(),
            album: song.album.clone().unwrap_or_default(),
            album_id: song.album_id.clone().unwrap_or_default(),
            favorite: song.starred.is_some(),
            duration: song.duration.unwrap_or(0) as f64,
            url: track_url(client, song, media)?,
            cover_art: song.cover_art.clone().unwrap_or_default(),
            cover_url: crate::covers::url_for(song.cover_art.as_deref(), covers),
            cover_remote: song
                .cover_art
                .as_deref()
                .map(|id| crate::covers::provider_cover_url(client, id))
                .unwrap_or_default(),
        });
    }
    Ok(tracks)
}

/// Resolve the cached/stream URL map for `songs` and build their queue rows.
pub(crate) async fn tracks(
    dbs: &crate::db::Dbs,
    songs: Vec<crate::library::SongRow>,
) -> anyhow::Result<Vec<crate::player::Track>> {
    let client = crate::session::client(dbs).await?;
    let covers = crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
    let song_ids: Vec<String> = songs.iter().map(|song| song.id.clone()).collect();
    let media = crate::media::path_map_of(&dbs.media, &song_ids)
        .await
        .unwrap_or_default();
    build_tracks(&client, &songs, &covers, &media)
}

/// CACHE-9: while offline, only downloaded songs are playable — drop the
/// rest from a queue that is about to be built.
pub async fn album_tracks(
    dbs: &crate::db::Dbs,
    album_id: &str,
    offline: bool,
) -> anyhow::Result<Vec<crate::player::Track>> {
    let mut songs = crate::library::album_songs(&dbs.library, album_id).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks(dbs, songs).await
}

/// The playlist queue: playlist order preserved, duplicate tracks kept,
/// missing/offline rows skipped, and the clicked row translated to its
/// playable queue index.
pub async fn playlist_tracks(
    dbs: &crate::db::Dbs,
    playlist_id: &str,
    offline: bool,
    start_row: i64,
) -> anyhow::Result<(Vec<crate::player::Track>, i64)> {
    #[derive(sqlx::FromRow)]
    struct PlaylistSong {
        position: i64,
        song_id: Option<String>,
        title: String,
        artist: Option<String>,
        album: Option<String>,
        duration: Option<i64>,
        cover_art: Option<String>,
    }
    let rows = sqlx::query_as::<_, PlaylistSong>(
        "SELECT position, song_id, title, artist, album, duration, cover_art
         FROM playlist_song WHERE playlist_id = ?1 ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(&dbs.playlist)
    .await?;
    let unique: Vec<String> = rows.iter().filter_map(|row| row.song_id.clone()).collect();
    let library = crate::library::songs_by_ids(&dbs.library, &unique).await?;
    let by_id: std::collections::HashMap<String, crate::library::SongRow> = library
        .into_iter()
        .map(|song| (song.id.clone(), song))
        .collect();
    let filter = crate::offline::load(dbs, offline).await?;
    let mut songs = Vec::new();
    let mut start = 0i64;
    for row in &rows {
        let Some(id) = row.song_id.as_ref() else {
            continue;
        };
        if filter
            .as_ref()
            .is_some_and(|filter| !filter.contains_song(id))
        {
            continue;
        }
        let song = by_id
            .get(id)
            .cloned()
            .unwrap_or_else(|| crate::library::SongRow {
                id: id.clone(),
                title: row.title.clone(),
                artist: row.artist.clone(),
                album: row.album.clone(),
                album_id: None,
                track: Some(row.position as i32 + 1),
                duration: row.duration.map(|value| value as i32),
                suffix: None,
                cover_art: row.cover_art.clone(),
                starred: None,
            });
        if row.position < start_row.max(0) {
            start += 1;
        }
        songs.push(song);
    }
    Ok((tracks(dbs, songs).await?, start))
}

/// The artist's shuffle queue: the artist's albums (the albums table's
/// mapping) Fisher–Yates shuffled, each expanding to its tracklist — one
/// album after another until the array ends. The queue holds the songs of
/// that order, header first.
pub async fn artist_shuffle_tracks(
    dbs: &crate::db::Dbs,
    artist_id: &str,
    offline: bool,
) -> anyhow::Result<Vec<crate::player::Track>> {
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
    tracks(dbs, songs).await
}

/// Shuffle one album's tracklist for an explicit "Shuffle Album" action.
pub async fn album_shuffle_tracks(
    dbs: &crate::db::Dbs,
    album_id: &str,
    offline: bool,
) -> anyhow::Result<Vec<crate::player::Track>> {
    let mut songs = crate::library::album_songs(&dbs.library, album_id).await?;
    if songs.is_empty() {
        return Err(anyhow::anyhow!("this album has no tracks to shuffle"));
    }
    shuffle_songs(&mut songs, crate::consts::now_nanos());
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks(dbs, songs).await
}

pub(crate) fn shuffle_songs(songs: &mut [crate::library::SongRow], seed: u128) {
    shuffle_slice(songs, seed);
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
/// source order is not preserved, nor is the previous cycle's. `seed` is the
/// clock; the caller passes `now()`.
pub fn shuffle_units(units: &mut [String], seed: u128) {
    shuffle_slice(units, seed);
}

/// [`shuffle_units`] for any row type: the queue itself is just an array, so
/// the in-memory queue can be permuted without a database round-trip (and
/// without dropping rows that no longer resolve, e.g. a radio queue).
pub fn shuffle_slice<T>(items: &mut [T], seed: u128) {
    let mut rng = SplitMix64((seed as u64) ^ ((seed >> 64) as u64));
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
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
pub async fn library_queue(
    dbs: &crate::db::Dbs,
    song_id: &str,
    order: &str,
    offline: bool,
) -> anyhow::Result<(Vec<crate::player::Track>, usize)> {
    let mut songs = crate::library::all_songs(&dbs.library, order).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    let start = library_start(&songs, song_id)?;
    Ok((tracks(dbs, songs).await?, start))
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

/// Typed queue rows for the ARC-43 "Add to Queue" overlay (offline-filtered
/// like every other queue).
pub async fn quick_tracks(
    dbs: &crate::db::Dbs,
    kind: &str,
    id: &str,
    offline: bool,
) -> anyhow::Result<Vec<crate::player::Track>> {
    let mut songs = quick_songs(dbs, kind, id).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks(dbs, songs).await
}

/// Batch twin of [`quick_tracks`] for a visual selection (VI-4).
pub async fn quick_tracks_many(
    dbs: &crate::db::Dbs,
    kind: &str,
    ids: &[String],
    offline: bool,
) -> anyhow::Result<Vec<crate::player::Track>> {
    let mut songs = quick_songs_many(dbs, kind, ids).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    tracks(dbs, songs).await
}

/// Restored playback session: the queue rows, their start index and the
/// remembered playhead (S2/S3 resume, P-6). The consumers used to decode a
/// `{album, queue, start, position}` JSON envelope back into exactly this.
pub struct Resume {
    pub tracks: Vec<crate::player::Track>,
    pub start: usize,
    pub position: f64,
}

/// S2/S3 resume (P-6): validates the remembered ids against the local index
/// and returns the restored queue, already pointing at the remembered track,
/// plus the saved playhead. The S3 queue snapshot wins; pre-S3 rows fall back
/// to the song's album queue. Unknown ids are dropped silently, exactly like
/// a first launch, and an unavailable stream client degrades to an empty
/// queue instead of failing the login.
pub async fn resume(
    dbs: &crate::db::Dbs,
    status: &crate::status::SessionStatus,
    offline: bool,
) -> anyhow::Result<Resume> {
    let mut queue = Vec::new();
    let mut start = 0usize;
    // S3: the queue snapshot (exact order, manual edits included) wins over
    // the S2 song -> album rebuild. Stale ids are dropped and the index
    // follows the remembered current song.
    let snapshot: Vec<String> = serde_json::from_str(&status.queue).unwrap_or_default();
    if !snapshot.is_empty() {
        let mut songs = crate::library::songs_by_ids(&dbs.library, &snapshot).await?;
        crate::offline::retain_songs(dbs, offline, &mut songs).await?;
        if !songs.is_empty() {
            let index = snapshot_start(&snapshot, &songs, status.queue_index);
            // No credentials yet (or a stream URL failure) degrades to an
            // empty queue: resume must never fail the login.
            match tracks(dbs, songs).await {
                Ok(built) => {
                    queue = built;
                    start = index.max(0) as usize;
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
            match tracks(dbs, songs).await {
                Ok(built) => {
                    queue = built;
                    start = index.max(0) as usize;
                }
                Err(err) => {
                    eprintln!("status: could not rebuild the resumed queue: {err:#}");
                }
            }
        }
    }
    Ok(Resume {
        tracks: queue,
        start,
        position: status.position,
    })
}
