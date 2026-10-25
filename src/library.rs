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

//! `library.db` — artists, albums and songs metadata.
//!
//! The tables are replaced wholesale on every real sync; deletions on the
//! server are therefore reflected locally without any diffing.

#![allow(dead_code)] // Browse queries are prepared before the library UI lands.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::api::{AlbumID3, ArtistID3, Child};

/// A row shaped for the search index rebuild.
#[derive(Debug, Clone, FromRow)]
pub struct SongSearchRow {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<i32>,
    pub play_count: Option<i64>,
}

/// What an API refresh changed, so the derived caches can be invalidated.
#[derive(Debug, Default, Clone)]
pub struct SyncChanges {
    pub albums: Vec<String>,
    pub artists: Vec<String>,
    /// Song ids whose indexed metadata changed or appeared.
    pub songs: Vec<String>,
    /// Cover ids whose album art changed or appeared.
    pub covers: Vec<String>,
}

#[derive(Debug, FromRow, PartialEq, Hash)]
struct AlbumSnapshot {
    id: String,
    artist_id: Option<String>,
    name: String,
    artist: Option<String>,
    year: Option<i32>,
    genre: Option<String>,
    song_count: i64,
    duration: i64,
    cover_art: Option<String>,
}

impl AlbumSnapshot {
    fn from_album(album: &AlbumID3) -> Self {
        Self {
            id: album.id.clone(),
            artist_id: album.artist_id.clone(),
            name: album.name.clone(),
            artist: album.artist.clone(),
            year: album.year,
            genre: album.genre.clone(),
            song_count: i64::from(album.song_count),
            duration: i64::from(album.duration),
            cover_art: album.cover_art.clone(),
        }
    }
}

#[derive(Debug, FromRow, PartialEq, Hash)]
struct ArtistSnapshot {
    id: String,
    name: String,
    album_count: i32,
    cover_art: Option<String>,
    image_url: Option<String>,
}

impl ArtistSnapshot {
    fn from_artist(artist: &ArtistID3) -> Self {
        Self {
            id: artist.id.clone(),
            name: artist.name.clone(),
            album_count: artist.album_count,
            cover_art: artist.cover_art.clone(),
            image_url: artist.artist_image_url.clone(),
        }
    }
}

#[derive(Debug, FromRow, PartialEq, Hash)]
struct SongSnapshot {
    id: String,
    album_id: Option<String>,
    artist_id: Option<String>,
    title: String,
    album: Option<String>,
    artist: Option<String>,
    track: Option<i32>,
    disc_number: Option<i32>,
    year: Option<i32>,
    genre: Option<String>,
    duration: Option<i32>,
    bit_rate: Option<i32>,
    suffix: Option<String>,
    size: Option<i64>,
    path: Option<String>,
    cover_art: Option<String>,
}

impl SongSnapshot {
    fn from_child(song: &Child) -> Self {
        Self {
            id: song.id.clone(),
            album_id: song.album_id.clone(),
            artist_id: song.artist_id.clone(),
            title: song.title.clone(),
            album: song.album.clone(),
            artist: song.artist.clone(),
            track: song.track,
            disc_number: song.disc_number,
            year: song.year,
            genre: song.genre.clone(),
            duration: song.duration,
            bit_rate: song.bit_rate,
            suffix: song.suffix.clone(),
            size: song.size,
            path: song.path.clone(),
            cover_art: song.cover_art.clone(),
        }
    }
}

/// A compact, order-free identity for an unchanged row. The sync diff only
/// needs "same or different", so the old catalog is held as `id -> hash`
/// instead of one full snapshot per row: a 100k-track library drops from
/// tens of MB of strings to a few MB of ids and hashes while the new rows
/// are inserted.
fn snapshot_hash<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::Hasher;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Replaces all library tables in a single transaction and stores the sync
/// bookkeeping values.
///
/// Returns the song/album/artist/cover ids that changed so the caller can
/// refresh the derived caches (covers, lyrics, descriptions) for exactly
/// those rows.
pub async fn replace(
    pool: &SqlitePool,
    artists: &[ArtistID3],
    albums: &[AlbumID3],
    songs: &[Child],
    last_modified: i64,
) -> Result<SyncChanges> {
    let mut tx = pool.begin().await?;

    // MEMORY: hold an `id -> hash` (plus the album cover id, the one field the
    // cover-refresh decision still needs) instead of a full snapshot per row.
    // Albums/artists are few compared with songs, but the same compact shape
    // keeps the diff's peak flat and one row at a time crosses the boundary.
    struct AlbumSeen {
        hash: u64,
        cover_art: Option<String>,
    }
    let mut old_albums: HashMap<String, AlbumSeen> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, AlbumSnapshot>(
            "SELECT id, artist_id, name, artist, year, genre, song_count, duration, cover_art
             FROM albums",
        )
        .fetch(&mut *tx);
        while let Some(album) = rows.try_next().await? {
            let hash = snapshot_hash(&album);
            old_albums.insert(
                album.id.clone(),
                AlbumSeen {
                    hash,
                    cover_art: album.cover_art,
                },
            );
        }
    }
    let mut old_artists: HashMap<String, u64> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, ArtistSnapshot>(
            "SELECT id, name, album_count, cover_art, image_url FROM artists",
        )
        .fetch(&mut *tx);
        while let Some(artist) = rows.try_next().await? {
            old_artists.insert(artist.id.clone(), snapshot_hash(&artist));
        }
    }
    // MEMORY: stream the old songs instead of collecting them. Only the hash
    // survives each row, so the peak stays one row plus the compact map.
    let mut old_songs: HashMap<String, u64> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, SongSnapshot>(
            "SELECT id, album_id, artist_id, title, album, artist, track, disc_number, year,
                    genre, duration, bit_rate, suffix, size, path, cover_art
             FROM songs",
        )
        .fetch(&mut *tx);
        while let Some(song) = rows.try_next().await? {
            old_songs.insert(song.id.clone(), snapshot_hash(&song));
        }
    }

    // MEMORY: resolve the diff before the destructive write, then drop the old
    // catalog. The insert phase is the peak (new rows + SQLite pages), so the
    // old ids/hashes must not ride along through it.
    let mut changes = SyncChanges::default();
    for song in songs {
        match old_songs.get(&song.id) {
            Some(old) if *old == snapshot_hash(&SongSnapshot::from_child(song)) => {}
            _ => changes.songs.push(song.id.clone()),
        }
    }
    for album in albums {
        let seen = old_albums.get(&album.id);
        if seen.is_some_and(|old| old.hash == snapshot_hash(&AlbumSnapshot::from_album(album))) {
            continue;
        }
        changes.albums.push(album.id.clone());
        let cover_changed = seen.is_none_or(|old| old.cover_art != album.cover_art);
        if cover_changed
            && let Some(cover) = album.cover_art.clone().filter(|cover| !cover.is_empty())
        {
            changes.covers.push(cover);
        }
    }
    for artist in artists {
        let changed = old_artists
            .get(&artist.id)
            .is_none_or(|old| *old != snapshot_hash(&ArtistSnapshot::from_artist(artist)));
        if changed {
            changes.artists.push(artist.id.clone());
        }
    }
    drop(old_songs);
    drop(old_albums);
    drop(old_artists);

    sqlx::query("DELETE FROM songs").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM albums").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM artists").execute(&mut *tx).await?;

    crate::cache::insert_many(
        &mut tx,
        "INSERT INTO artists (id, name, album_count, cover_art, image_url, starred) ",
        artists,
        |mut row, artist| {
            row.push_bind(&artist.id)
                .push_bind(&artist.name)
                .push_bind(artist.album_count)
                .push_bind(&artist.cover_art)
                .push_bind(&artist.artist_image_url)
                .push_bind(&artist.starred);
        },
    )
    .await?;

    crate::cache::insert_many(
        &mut tx,
        "INSERT INTO albums
             (id, artist_id, name, artist, year, genre, song_count, duration, cover_art, starred, rating) ",
        albums,
        |mut row, album| {
            row.push_bind(&album.id)
                .push_bind(&album.artist_id)
                .push_bind(&album.name)
                .push_bind(&album.artist)
                .push_bind(album.year)
                .push_bind(&album.genre)
                .push_bind(album.song_count)
                .push_bind(album.duration)
                .push_bind(&album.cover_art)
                .push_bind(&album.starred)
                .push_bind(album.user_rating.unwrap_or(0).clamp(0, 5));
        },
    )
    .await?;

    crate::cache::insert_many(
        &mut tx,
        "INSERT OR REPLACE INTO songs
             (id, album_id, artist_id, title, album, artist, track, disc_number, year,
              genre, duration, bit_rate, suffix, size, path, cover_art, play_count, starred) ",
        songs,
        |mut row, song| {
            row.push_bind(&song.id)
                .push_bind(&song.album_id)
                .push_bind(&song.artist_id)
                .push_bind(&song.title)
                .push_bind(&song.album)
                .push_bind(&song.artist)
                .push_bind(song.track)
                .push_bind(song.disc_number)
                .push_bind(song.year)
                .push_bind(&song.genre)
                .push_bind(song.duration)
                .push_bind(song.bit_rate)
                .push_bind(&song.suffix)
                .push_bind(song.size)
                .push_bind(&song.path)
                .push_bind(&song.cover_art)
                .push_bind(song.play_count)
                .push_bind(&song.starred);
        },
    )
    .await?;

    upsert_meta(&mut *tx, "last_modified", &last_modified.to_string()).await?;
    upsert_meta(
        &mut *tx,
        "last_sync",
        &crate::consts::now_secs().to_string(),
    )
    .await?;

    tx.commit().await?;

    Ok(changes)
}

/// ARC-I-4: the local half of the favourite toggle. The UI reads
/// `starred IS NOT NULL` as "favourited" (UI-56); the exact server timestamp
/// reconciles on the next sync. Songs and albums only — artists live in
/// `artist_user.db` (`artist_user::set_starred`).
pub async fn set_song_starred(pool: &SqlitePool, id: &str, on: bool) -> Result<()> {
    sqlx::query("UPDATE songs SET starred = CASE WHEN ?1 THEN ?2 ELSE NULL END WHERE id = ?3")
        .bind(on)
        .bind(crate::consts::now_secs().to_string())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Song ids carrying a favourite flag — the queue payload's heart truth
/// (the in-memory player track's flag goes stale after `altFavorite`).
pub async fn starred_song_ids(pool: &SqlitePool) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT id FROM songs WHERE starred IS NOT NULL")
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// The album's favourite flag (the fullscreen keybar heart).
pub async fn album_starred(pool: &SqlitePool, id: &str) -> Result<bool> {
    let starred: Option<String> = sqlx::query_scalar("SELECT starred FROM albums WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(starred.is_some())
}

/// UI-60: the album's user rating (0..=5, 0 = unrated).
pub async fn album_rating(pool: &SqlitePool, id: &str) -> Result<i64> {
    let rating: Option<i64> = sqlx::query_scalar("SELECT rating FROM albums WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .flatten();
    Ok(rating.unwrap_or(0).clamp(0, 5))
}

/// Live starred/rating state for a search page's albums (ARC-SEARCH: the
/// in-memory index carries the sync-time snapshot, but a heart or rating
/// write must show on the very next keystroke).
pub async fn album_states(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<HashMap<String, (bool, i64)>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut states = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT id, CASE WHEN starred IS NOT NULL THEN 1 ELSE 0 END AS starred, rating
             FROM albums WHERE id IN (",
        );
        crate::cache::push_in_list(&mut builder, chunk);
        let rows: Vec<(String, i64, i64)> = builder.build_query_as().fetch_all(pool).await?;
        states.extend(
            rows.into_iter()
                .map(|(id, starred, rating)| (id, (starred != 0, rating))),
        );
    }
    Ok(states)
}

/// UI-60: records the album rating locally; the optimistic write is
/// reconciled by the next sync (the server row carries `userRating`).
pub async fn set_album_rating(pool: &SqlitePool, id: &str, rating: u8) -> Result<()> {
    sqlx::query("UPDATE albums SET rating = ?1 WHERE id = ?2")
        .bind(i64::from(rating.min(5)))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Album twin of [`artist_song_ids`] — the container's ledger set.
pub async fn album_song_ids(pool: &SqlitePool, album_id: &str) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT id FROM songs WHERE album_id = ?1")
        .bind(album_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// The current `coverArt` id of many albums in one query (queue refresh).
pub async fn album_covers(
    pool: &SqlitePool,
    album_ids: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    let refs: Vec<&str> = album_ids.iter().map(String::as_str).collect();
    album_covers_refs(pool, &refs).await
}

/// Borrowed twin of [`album_covers`] for callers that already hold `&str`
/// ids (a queue snapshot), avoiding one `String` clone per album.
pub async fn album_covers_refs(
    pool: &SqlitePool,
    album_ids: &[&str],
) -> Result<std::collections::HashMap<String, String>> {
    if album_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut covers = std::collections::HashMap::new();
    for chunk in album_ids.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT id, cover_art FROM albums WHERE cover_art IS NOT NULL AND id IN (",
        );
        crate::cache::push_in_list_refs(&mut builder, chunk);
        let rows: Vec<(String, String)> = builder.build_query_as().fetch_all(pool).await?;
        covers.extend(rows);
    }
    Ok(covers)
}

/// The tracklists of many albums in one query — the mark window's album
/// coverage read (never one roundtrip per album).
pub async fn album_song_ids_of(
    pool: &SqlitePool,
    album_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    if album_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut grouped: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for chunk in album_ids.chunks(crate::cache::IN_BATCH) {
        let mut builder =
            sqlx::QueryBuilder::new("SELECT album_id, id FROM songs WHERE album_id IN (");
        crate::cache::push_in_list(&mut builder, chunk);
        let rows: Vec<(String, String)> = builder.build_query_as().fetch_all(pool).await?;
        for (album_id, song_id) in rows {
            grouped.entry(album_id).or_default().push(song_id);
        }
    }
    Ok(grouped)
}

/// Album twin of [`set_song_starred`].
pub async fn set_album_starred(pool: &SqlitePool, id: &str, on: bool) -> Result<()> {
    sqlx::query("UPDATE albums SET starred = CASE WHEN ?1 THEN ?2 ELSE NULL END WHERE id = ?3")
        .bind(on)
        .bind(crate::consts::now_secs().to_string())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Records that a sync was attempted even when the server reported no changes.
pub async fn touch_last_sync(pool: &SqlitePool) -> Result<()> {
    upsert_meta(pool, "last_sync", &crate::consts::now_secs().to_string()).await
}

pub async fn last_modified(pool: &SqlitePool) -> Result<i64> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'last_modified'")
            .fetch_optional(pool)
            .await?;
    Ok(value.and_then(|value| value.parse().ok()).unwrap_or(0))
}

/// Snapshot shown by the right status bar.
#[derive(Debug, Clone, FromRow)]
pub struct LibraryStatus {
    pub artists: i64,
    pub albums: i64,
    pub songs: i64,
    pub last_sync: Option<String>,
}

/// Row counts plus the time of the last sync, for [`LibraryStatus`].
pub async fn status(pool: &SqlitePool) -> Result<LibraryStatus> {
    let status = sqlx::query_as::<_, LibraryStatus>(
        "SELECT (SELECT COUNT(*) FROM artists) AS artists,
                (SELECT COUNT(*) FROM albums) AS albums,
                (SELECT COUNT(*) FROM songs) AS songs,
                (SELECT value FROM meta WHERE key = 'last_sync') AS last_sync",
    )
    .fetch_one(pool)
    .await?;
    Ok(status)
}

/// Every song id in the library, for derived-cache backfills.
pub async fn song_ids(pool: &SqlitePool) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT id FROM songs")
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// Which of `ids` still exist in `songs`, in 500-id chunks.
///
/// The purge paths use this instead of materializing every library song: a
/// 100k-song catalog answering a 5k-row cache check should cost 5k ids, not
/// 100k.
pub async fn existing_song_ids(
    pool: &SqlitePool,
    ids: &[&str],
) -> Result<std::collections::HashSet<String>> {
    let mut existing = std::collections::HashSet::with_capacity(ids.len());
    for chunk in ids.chunks(crate::cache::IN_BATCH) {
        let mut builder =
            sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT id FROM songs WHERE id IN (");
        crate::cache::push_in_list_refs(&mut builder, chunk);
        let found: Vec<String> = builder.build_query_scalar().fetch_all(pool).await?;
        existing.extend(found);
    }
    Ok(existing)
}

/// Returns the columns needed to rebuild the FTS index.
pub async fn song_search_rows(pool: &SqlitePool) -> Result<Vec<SongSearchRow>> {
    let rows = sqlx::query_as::<_, SongSearchRow>(
        "SELECT id, title, artist, album, duration, play_count FROM songs",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// CACHE-9: from the downloaded song ids, the album id -> downloaded-track
/// count and artist id -> downloaded-album count. The offline mode lists (and
/// counts) only what is actually available. Chunked so a large download set
/// stays under SQLite's bind limit.
pub async fn downloaded_owners(
    pool: &SqlitePool,
    song_ids: &[String],
) -> Result<(HashMap<String, i64>, HashMap<String, i64>)> {
    use sqlx::QueryBuilder;

    let mut album_counts: HashMap<String, i64> = HashMap::new();
    let mut artist_albums: HashMap<String, HashSet<String>> = HashMap::new();

    for chunk in song_ids.chunks(crate::cache::IN_BATCH) {
        let mut query =
            QueryBuilder::new("SELECT album_id, artist_id, COUNT(*) FROM songs WHERE id IN (");
        crate::cache::push_in_list(&mut query, chunk);
        query.push(" GROUP BY album_id, artist_id");
        let rows: Vec<(Option<String>, Option<String>, i64)> =
            query.build_query_as().fetch_all(pool).await?;

        for (album_id, artist_id, count) in rows {
            if let Some(album_id) = album_id {
                *album_counts.entry(album_id.clone()).or_insert(0) += count;
                if let Some(artist_id) = artist_id {
                    artist_albums.entry(artist_id).or_default().insert(album_id);
                }
            } else if let Some(artist_id) = artist_id {
                // A downloaded song without an album still keeps its artist
                // listable (zero albums, one song).
                artist_albums.entry(artist_id).or_default();
            }
        }
    }

    let artist_counts = artist_albums
        .into_iter()
        .map(|(artist, albums)| (artist, albums.len() as i64))
        .collect();

    Ok((album_counts, artist_counts))
}

/// Artists and albums matching `query` by case-insensitive substring, for the
/// floating search (songs come from the FTS index in `search.db`).
pub async fn search_artists_albums(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> Result<(Vec<ArtistRow>, Vec<AlbumRow>)> {
    // Search is prose, not a pattern: ignore punctuation like the FTS side.
    let query = crate::search::clean_query(query);
    if query.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let like = format!("%{escaped}%");

    let artist_sql = format!(
        "{ARTIST_SELECT} WHERE EXISTS (
             SELECT 1 FROM albums WHERE albums.artist_id = artists.id)
         AND name LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT ?2"
    );
    let artists = sqlx::query_as::<_, ArtistRow>(sqlx::AssertSqlSafe(artist_sql))
        .bind(&like)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    let album_sql = format!(
        "{ALBUM_SELECT} WHERE name LIKE ?1 ESCAPE '\\' OR artist LIKE ?1 ESCAPE '\\'{} LIMIT ?2",
        album_order("Artist", true)
    );
    let albums = sqlx::query_as::<_, AlbumRow>(sqlx::AssertSqlSafe(album_sql))
        .bind(&like)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    Ok((artists, albums))
}

/// Maps song ids to their `coverArt` id (album art when the song has none).
pub async fn song_cover_ids(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut builder = sqlx::QueryBuilder::new(
        "SELECT s.id, COALESCE(al.cover_art, s.cover_art) AS cover_art
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id
         WHERE s.id IN (",
    );
    crate::cache::push_in_list(&mut builder, ids);

    let rows: Vec<(String, Option<String>)> = builder.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, cover_art)| cover_art.map(|cover_art| (id, cover_art)))
        .collect())
}

/// Maps song ids to their album ids (ARC-I: the search hits' Go to Artist
/// jump needs the album the search index does not carry).
pub async fn song_album_ids(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut builder = sqlx::QueryBuilder::new("SELECT id, album_id FROM songs WHERE id IN (");
    crate::cache::push_in_list(&mut builder, ids);

    let rows: Vec<(String, Option<String>)> = builder.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, album_id)| album_id.map(|album_id| (id, album_id)))
        .collect())
}

/// The subset of `ids` the server marks as favourites (UI-56): the search
/// index carries no `starred`, so the search payload joins it back in from
/// the library.
pub async fn song_starred_ids(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<std::collections::HashSet<String>> {
    if ids.is_empty() {
        return Ok(std::collections::HashSet::new());
    }

    let mut builder =
        sqlx::QueryBuilder::new("SELECT id FROM songs WHERE starred IS NOT NULL AND id IN (");
    crate::cache::push_in_list(&mut builder, ids);

    let rows: Vec<String> = builder.build_query_scalar().fetch_all(pool).await?;
    Ok(rows.into_iter().collect())
}

/// Every `coverArt` id referenced by the library, used to drive the cover
/// cache and to know which cached images became stale.
///
/// Only album- and artist-level art is cached: songs inherit their album's
/// cover, which keeps the cache bounded (per-song ids exploded it).
pub async fn referenced_cover_ids(pool: &SqlitePool) -> Result<std::collections::BTreeSet<String>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT cover_art FROM artists WHERE cover_art IS NOT NULL
         UNION SELECT cover_art FROM albums WHERE cover_art IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().collect())
}

/// A row for the central album list.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct AlbumRow {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<i32>,
    pub song_count: i64,
    pub cover_art: Option<String>,
    /// Server `starred` timestamp; `Some` = favourited (UI-56).
    pub starred: Option<String>,
    /// UI-60: user rating 0..=5 (0 = unrated).
    pub rating: i64,
}

/// A row for an album's song list.
#[derive(Debug, Clone, FromRow, serde::Serialize, serde::Deserialize)]
pub struct SongRow {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub track: Option<i32>,
    pub duration: Option<i32>,
    /// Provider file suffix (`flac`, `mp3`, …); feeds the album page's
    /// format label.
    pub suffix: Option<String>,
    pub cover_art: Option<String>,
    /// Server `starred` timestamp; `Some` = favourited (UI-56).
    pub starred: Option<String>,
}

/// A row for the artist list.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ArtistRow {
    pub id: String,
    pub name: String,
    pub album_count: i64,
    pub cover_art: Option<String>,
    /// Server `starred` timestamp; the payload ORs the local `artist_user`
    /// overlay on top (UI-56).
    pub starred: Option<String>,
}

/// Orderings offered by the album list and the song list (`Shift-H`/
/// `Shift-L`). The labels map to the Subsonic `getAlbumList2` types they
/// approximate: Artist = alphabeticalByArtist, Name = alphabeticalByName,
/// Year = byYear.
pub const ORDERS: [&str; 3] = ["Artist", "Name", "Year"];

/// Orderings offered by the artist list (`Shift-H`/`Shift-L`): alphabetical,
/// or most albums first.
pub const ARTIST_ORDERS: [&str; 2] = ["Name", "Albums"];

/// Cycles one of the order lists by `delta`; shared so lists never drift.
fn cycle_in(orders: &[&'static str], current: &str, delta: i64) -> &'static str {
    let index = orders
        .iter()
        .position(|order| *order == current)
        .unwrap_or(0) as i64;
    let next = (index + delta).rem_euclid(orders.len() as i64) as usize;
    orders[next]
}

/// Cycles an album/song order label by `delta` (albums and songs share it).
pub fn cycle_order(current: &str, delta: i64) -> &'static str {
    cycle_in(&ORDERS, current, delta)
}

/// Cycles an artist order label by `delta` ([`ARTIST_ORDERS`]).
pub fn cycle_artist_order(current: &str, delta: i64) -> &'static str {
    cycle_in(&ARTIST_ORDERS, current, delta)
}

/// Applies `delta` to `current`, treating empty as the first order.
pub fn effective_order(orders: &[&'static str], current: &str, delta: i64) -> &'static str {
    let current = if current.is_empty() {
        orders[0]
    } else {
        current
    };
    cycle_in(orders, current, delta)
}

/// Albums for the central list, optionally filtered by artist name, ordered
/// by one of [`ORDERS`]. Kept as static SQL (no interpolation) so the
/// allowlist is enforced at the type level.
pub async fn albums(pool: &SqlitePool, artist: Option<&str>, order: &str) -> Result<Vec<AlbumRow>> {
    let filter = if artist.is_some() {
        " WHERE artist = ?1"
    } else {
        ""
    };
    let sql = format!("{ALBUM_SELECT}{filter}{}", album_order(order, true));

    let rows = match artist {
        Some(artist) => {
            sqlx::query_as::<_, AlbumRow>(sqlx::AssertSqlSafe(sql))
                .bind(artist)
                .fetch_all(pool)
                .await?
        }
        None => {
            sqlx::query_as::<_, AlbumRow>(sqlx::AssertSqlSafe(sql))
                .fetch_all(pool)
                .await?
        }
    };
    Ok(rows)
}

/// Albums of one artist by id (ArtistEntry grid), ordered by one of
/// [`ORDERS`]. Static SQL, like [`albums`].
pub async fn artist_albums(
    pool: &SqlitePool,
    artist_id: &str,
    order: &str,
) -> Result<Vec<AlbumRow>> {
    let sql = format!(
        "{ALBUM_SELECT} WHERE artist_id = ?1{}",
        album_order(order, false)
    );
    let rows = sqlx::query_as::<_, AlbumRow>(sqlx::AssertSqlSafe(sql))
        .bind(artist_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// All artists, ordered by one of [`ARTIST_ORDERS`].
pub async fn artists(pool: &SqlitePool, order: &str) -> Result<Vec<ArtistRow>> {
    let order = match order {
        "Albums" => " ORDER BY album_count DESC, name",
        _ => " ORDER BY name",
    };
    let sql = format!(
        "{ARTIST_SELECT} WHERE EXISTS (
             SELECT 1 FROM albums WHERE albums.artist_id = artists.id){order}"
    );
    let rows = sqlx::query_as::<_, ArtistRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

const ALBUM_SELECT: &str =
    "SELECT id, name AS title, artist, year, song_count, cover_art, starred, rating
     FROM albums";

const ARTIST_SELECT: &str = "SELECT id, name, album_count, cover_art, starred FROM artists";

fn album_order(order: &str, by_artist: bool) -> &'static str {
    match (by_artist, order) {
        (_, "Name") => " ORDER BY name",
        (_, "Year") => " ORDER BY year DESC, name",
        (true, _) => " ORDER BY artist, name",
        (false, _) => " ORDER BY year, name",
    }
}

const SONG_SELECT: &str =
    "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration, s.suffix,
            s.starred, COALESCE(al.cover_art, s.cover_art) AS cover_art
     FROM songs s LEFT JOIN albums al ON al.id = s.album_id";

fn song_order(order: &str) -> &'static str {
    match order {
        "Name" => " ORDER BY s.title, s.artist, s.album",
        "Year" => " ORDER BY s.year DESC, s.title, s.artist",
        _ => " ORDER BY s.artist, s.album, s.disc_number, s.track, s.title",
    }
}

/// Every song, ordered by one of [`ORDERS`]. Covers come from the album.
pub async fn all_songs(pool: &SqlitePool, order: &str) -> Result<Vec<SongRow>> {
    let sql = format!("{SONG_SELECT}{}", song_order(order));
    let rows = sqlx::query_as::<_, SongRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Size of the library, without materializing it.
pub async fn song_count(pool: &SqlitePool) -> Result<usize> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM songs")
        .fetch_one(pool)
        .await?;
    Ok(count.max(0) as usize)
}

/// 0-based position of a song in [`all_songs`] order, without materializing
/// the library (same ORDER BY, evaluated by SQLite's ROW_NUMBER).
pub async fn song_ordinal(pool: &SqlitePool, id: &str, order: &str) -> Result<Option<usize>> {
    let sql = format!(
        "SELECT rn - 1 FROM (
             SELECT s.id, ROW_NUMBER() OVER ({}) AS rn
             FROM songs s
         ) WHERE id = ?1",
        song_order(order)
    );
    let ordinal: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(ordinal.map(|value| value.max(0) as usize))
}

/// One page of the [`all_songs`] order, covers resolved from the album like
/// the full list.
pub async fn songs_range(
    pool: &SqlitePool,
    offset: i64,
    limit: i64,
    order: &str,
) -> Result<Vec<SongRow>> {
    let sql = format!("{SONG_SELECT}{} LIMIT ?1 OFFSET ?2", song_order(order));
    let rows = sqlx::query_as::<_, SongRow>(sqlx::AssertSqlSafe(sql))
        .bind(limit.max(0))
        .bind(offset.max(0))
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// A single song by id.
pub async fn song(pool: &SqlitePool, id: &str) -> Result<Option<SongRow>> {
    let sql = format!("{SONG_SELECT} WHERE s.id = ?1");
    let row = sqlx::query_as::<_, SongRow>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Exact song lookup by artist/title (case-insensitive), for clipboard paste.
/// `artist = None` matches any artist.
pub async fn find_song_id(
    pool: &SqlitePool,
    artist: Option<&str>,
    title: &str,
) -> Result<Option<String>> {
    let row: Option<Option<String>> = sqlx::query_scalar(
        "SELECT id FROM songs
         WHERE lower(title) = lower(?1)
           AND (?2 = '' OR lower(COALESCE(artist, '')) = lower(?2))
         LIMIT 1",
    )
    .bind(title)
    .bind(artist.unwrap_or(""))
    .fetch_optional(pool)
    .await?;
    Ok(row.flatten())
}

/// A single album by id.
pub async fn album(pool: &SqlitePool, id: &str) -> Result<Option<AlbumRow>> {
    let sql = format!("{ALBUM_SELECT} WHERE id = ?1");
    let row = sqlx::query_as::<_, AlbumRow>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// The owning artist id of an album (ARC-43 "Go to Artist"); list payloads
/// never need it, so [`AlbumRow`] omits the column.
pub async fn album_artist_id(pool: &SqlitePool, album_id: &str) -> Result<Option<String>> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT artist_id FROM albums WHERE id = ?1")
            .bind(album_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.flatten())
}

/// A single artist by id.
pub async fn artist(pool: &SqlitePool, id: &str) -> Result<Option<ArtistRow>> {
    let sql = format!("{ARTIST_SELECT} WHERE id = ?1");
    let row = sqlx::query_as::<_, ArtistRow>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Songs for an explicit id list, returned in exactly that order and with
/// unknown ids dropped — the P-6 S3 queue snapshot restore path. Unknown ids
/// are stale rows from a resynced server, not an error.
pub async fn songs_by_ids(pool: &SqlitePool, ids: &[String]) -> Result<Vec<SongRow>> {
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    songs_by_ids_refs(pool, &refs).await
}

/// Borrowed twin of [`songs_by_ids`] (queue refresh against a COW snapshot):
/// binding `&str` ids directly avoids a `String` clone per track.
pub async fn songs_by_ids_refs(pool: &SqlitePool, ids: &[&str]) -> Result<Vec<SongRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = sqlx::QueryBuilder::new(format!("{SONG_SELECT} WHERE s.id IN ("));
    crate::cache::push_in_list_refs(&mut builder, ids);

    let rows: Vec<SongRow> = builder.build_query_as().fetch_all(pool).await?;
    let mut by_id: std::collections::HashMap<String, SongRow> =
        rows.into_iter().map(|row| (row.id.clone(), row)).collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(*id)).collect())
}

/// Songs of one album, in disc/track order. Covers come from the album.
pub async fn album_songs(pool: &SqlitePool, album_id: &str) -> Result<Vec<SongRow>> {
    let sql =
        format!("{SONG_SELECT} WHERE s.album_id = ?1 ORDER BY s.disc_number, s.track, s.title");
    let rows = sqlx::query_as::<_, SongRow>(sqlx::AssertSqlSafe(sql))
        .bind(album_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Every song of an artist's albums (ARC-I "Add to Queue" on an artist hit),
/// album-major then disc/track order.
pub async fn artist_songs(pool: &SqlitePool, artist_id: &str) -> Result<Vec<SongRow>> {
    let sql = format!(
        "{SONG_SELECT} WHERE al.artist_id = ?1 ORDER BY al.name, s.disc_number, s.track, s.title"
    );
    let rows = sqlx::query_as::<_, SongRow>(sqlx::AssertSqlSafe(sql))
        .bind(artist_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// `(size, suffix, bit_rate)` for the media fingerprint (CACHE-1/8).
pub async fn song_file_info(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<(Option<i64>, Option<String>, Option<i64>)>> {
    let row = sqlx::query_as("SELECT size, suffix, bit_rate FROM songs WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// The artist's album ids (paint order) — the artist batch reuses the album
/// download per entry, so this is its loop's source.
pub async fn artist_album_ids(pool: &SqlitePool, artist_id: &str) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT id FROM albums WHERE artist_id = ?1 ORDER BY name")
        .bind(artist_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// Every song id of one artist (CACHE-5 rule materialization).
pub async fn artist_song_ids(pool: &SqlitePool, artist_id: &str) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT id FROM songs WHERE artist_id = ?1")
        .bind(artist_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// `(artist_id, album_id)` per song id — the scope inputs for the CACHE-5
/// coverage check (empty map for an empty `ids` list).
pub async fn song_scopes(
    pool: &SqlitePool,
    ids: &[String],
) -> Result<std::collections::HashMap<String, (Option<String>, Option<String>)>> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let mut builder =
        sqlx::QueryBuilder::new("SELECT id, artist_id, album_id FROM songs WHERE id IN (");
    crate::cache::push_in_list(&mut builder, ids);
    let rows: Vec<(String, Option<String>, Option<String>)> =
        builder.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(id, artist, album)| (id, (artist, album)))
        .collect())
}

/// Cached artist biography + external URL (ARC-42, `artist_info` table).
#[derive(Debug, Clone, Default, PartialEq, Eq, sqlx::FromRow, serde::Serialize)]
pub struct ArtistInfoRow {
    pub artist_id: String,
    pub biography: String,
    pub url: String,
    pub fetched_at: String,
}

/// Upserts the cached biography for one artist. library.db owns this (the
/// description is library metadata, unlike the user state in `artist_user.db`).
pub async fn upsert_artist_info(
    pool: &SqlitePool,
    artist_id: &str,
    biography: &str,
    url: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO artist_info (artist_id, biography, url, fetched_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(artist_id) DO UPDATE SET
             biography  = excluded.biography,
             url        = excluded.url,
             fetched_at = excluded.fetched_at",
    )
    .bind(artist_id)
    .bind(biography)
    .bind(url)
    .bind(crate::consts::now_secs().to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// The cached biography for one artist; `None` when never fetched.
pub async fn artist_info(pool: &SqlitePool, artist_id: &str) -> Result<Option<ArtistInfoRow>> {
    let row = sqlx::query_as::<_, ArtistInfoRow>(
        "SELECT artist_id, biography, url, fetched_at
         FROM artist_info WHERE artist_id = ?1",
    )
    .bind(artist_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Drops cached biographies for artists the API just changed, so the next
/// open refetches the description.
pub async fn invalidate_artist_info(pool: &SqlitePool, ids: &[String]) -> Result<()> {
    crate::cache::delete_in(pool, "DELETE FROM artist_info WHERE artist_id IN (", ids).await
}

/// Drops cached biographies fetched before the provider's last library
/// update: the provider has newer metadata, so the old text must not win.
pub async fn invalidate_artist_info_before(pool: &SqlitePool, before: i64) -> Result<u64> {
    let result = sqlx::query("DELETE FROM artist_info WHERE CAST(fetched_at AS INTEGER) < ?1")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Drops cached biographies for artists that left the library. Only called
/// after a fully successful sync (the covers/lyrics/colors rule).
pub async fn purge_stale_artist_info(pool: &SqlitePool) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM artist_info
         WHERE artist_id NOT IN (SELECT id FROM artists)",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Writes the tab-separated `index.txt` mirror, one line per song.
///
/// MEMORY: streamed in ~64 KiB chunks instead of collecting every row and then
/// a second full-size `String`: a 100k-song index used to hold the row set and
/// its rendered text at the same time.
pub async fn write_mirror(pool: &SqlitePool) -> Result<()> {
    use std::fmt::Write as _;
    use tokio::io::AsyncWriteExt as _;

    let path = crate::db::mirror_path()?;
    crate::db::ensure_parent_dir(&path)?;
    let file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = tokio::io::BufWriter::new(file);

    let mut rows = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, COALESCE(artist, ''), COALESCE(album, ''), title
         FROM songs
         ORDER BY artist, album, disc_number, track, title",
    )
    .fetch(pool);
    let mut chunk = String::with_capacity(64 * 1024);
    while let Some((id, artist, album, title)) = rows.try_next().await? {
        // Writing to a String cannot fail; the heap is the only cost.
        let _ = writeln!(chunk, "{artist}\t{album}\t{title}\t{id}");
        if chunk.len() >= 64 * 1024 {
            writer
                .write_all(chunk.as_bytes())
                .await
                .with_context(|| format!("failed to write {}", path.display()))?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        writer
            .write_all(chunk.as_bytes())
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    writer
        .flush()
        .await
        .with_context(|| format!("failed to flush {}", path.display()))?;
    Ok(())
}

async fn upsert_meta<'e, E>(executor: E, key: &str, value: &str) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(executor)
    .await?;
    Ok(())
}

/// PATH-8: one album's cached description and fetch time.
pub async fn album_info(pool: &SqlitePool, album_id: &str) -> Result<Option<(String, i64)>> {
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT notes, fetched_at FROM album_info WHERE album_id = ?1")
            .bind(album_id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

/// PATH-8: stores an album description for the local-first read.
pub async fn upsert_album_info(pool: &SqlitePool, album_id: &str, notes: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO album_info (album_id, notes, fetched_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(album_id) DO UPDATE SET
             notes = excluded.notes,
             fetched_at = excluded.fetched_at",
    )
    .bind(album_id)
    .bind(notes)
    .bind(crate::consts::now_secs())
    .execute(pool)
    .await?;
    Ok(())
}

/// PATH-8: drops cached descriptions for albums the API just changed, so the
/// next open refetches the notes.
pub async fn invalidate_album_info(pool: &SqlitePool, ids: &[String]) -> Result<()> {
    crate::cache::delete_in(pool, "DELETE FROM album_info WHERE album_id IN (", ids).await
}

/// Drops cached album notes fetched before the provider's last library
/// update: the provider has newer metadata, so the old text must not win.
pub async fn invalidate_album_info_before(pool: &SqlitePool, before: i64) -> Result<u64> {
    let result = sqlx::query("DELETE FROM album_info WHERE fetched_at < ?1")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// PATH-8: drops cached descriptions whose album left the library.
pub async fn purge_stale_album_info(pool: &SqlitePool) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM album_info
         WHERE album_id NOT IN (SELECT id FROM albums)",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// PATH-9: is the cover cache due for a revalidation pass?
pub async fn cover_check_due(pool: &SqlitePool, ttl_secs: i64) -> Result<bool> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'covers_checked_at'")
            .fetch_optional(pool)
            .await?;
    let Some(value) = value else {
        return Ok(true);
    };
    let Ok(checked_at) = value.parse::<i64>() else {
        return Ok(true);
    };
    Ok(crate::consts::now_secs() - checked_at >= ttl_secs)
}

/// PATH-9: records a cover revalidation pass.
pub async fn touch_cover_check(pool: &SqlitePool) -> Result<()> {
    upsert_meta(
        pool,
        "covers_checked_at",
        &crate::consts::now_secs().to_string(),
    )
    .await
}
