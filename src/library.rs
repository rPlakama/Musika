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
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::api::{AlbumID3, ArtistID3, Child};

/// Replaces all library tables in a single transaction and stores the sync
/// bookkeeping values.
pub async fn replace(
    pool: &SqlitePool,
    artists: &[ArtistID3],
    albums: &[AlbumID3],
    songs: &[Child],
    last_modified: i64,
) -> Result<()> {
    let mut tx = pool.begin().await?;

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
    Ok(())
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
    for chunk in ids.chunks(500) {
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
    for chunk in album_ids.chunks(500) {
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

    for chunk in song_ids.chunks(500) {
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

    let artist_sql =
        format!("{ARTIST_SELECT} WHERE name LIKE ?1 ESCAPE '\\' ORDER BY name LIMIT ?2");
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
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct SongRow {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub track: Option<i32>,
    pub duration: Option<i32>,
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
    let sql = format!("{ARTIST_SELECT}{order}");
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
    "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration, s.starred,
            COALESCE(al.cover_art, s.cover_art) AS cover_art
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
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = sqlx::QueryBuilder::new(format!("{SONG_SELECT} WHERE s.id IN ("));
    crate::cache::push_in_list(&mut builder, ids);

    let rows: Vec<SongRow> = builder.build_query_as().fetch_all(pool).await?;
    let mut by_id: std::collections::HashMap<String, SongRow> =
        rows.into_iter().map(|row| (row.id.clone(), row)).collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
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
pub async fn write_mirror(pool: &SqlitePool) -> Result<()> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, COALESCE(artist, ''), COALESCE(album, ''), title
         FROM songs
         ORDER BY artist, album, disc_number, track, title",
    )
    .fetch_all(pool)
    .await?;

    let mut text = String::new();
    for (id, artist, album, title) in rows {
        text.push_str(&format!("{artist}\t{album}\t{title}\t{id}\n"));
    }

    let path = crate::db::mirror_path()?;
    crate::db::ensure_parent_dir(&path)?;
    std::fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_pool() -> SqlitePool {
        crate::sqlite::memory_pool(sqlx::migrate!("./migrations_library")).await
    }

    #[tokio::test]
    async fn replace_round_trips_artists_albums_and_songs() {
        let pool = memory_pool().await;
        let artist = ArtistID3 {
            id: "ar1".into(),
            name: "Radiohead".into(),
            cover_art: Some("cov1".into()),
            artist_image_url: Some("https://img/ar1".into()),
            album_count: 1,
            starred: Some("2026-01-02T03:04:05Z".into()),
        };
        let album = AlbumID3 {
            id: "al1".into(),
            artist_id: Some("ar1".into()),
            name: "Kid A".into(),
            artist: Some("Radiohead".into()),
            song_count: 1,
            ..AlbumID3::default()
        };
        let song = Child {
            id: "s1".into(),
            album_id: Some("al1".into()),
            artist_id: Some("ar1".into()),
            title: "Everything in Its Right Place".into(),
            track: Some(1),
            ..Child::default()
        };

        // Regression: the artists INSERT once listed `starred` twice (7
        // columns for 6 values), failing every sync with SQLite error
        // "6 values for 7 columns".
        replace(&pool, &[artist], &[album], &[song], 42)
            .await
            .unwrap();

        let (name, image, starred): (String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT name, image_url, starred FROM artists WHERE id = 'ar1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(name, "Radiohead");
        assert_eq!(image.as_deref(), Some("https://img/ar1"));
        assert_eq!(starred.as_deref(), Some("2026-01-02T03:04:05Z"));

        let songs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM songs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(songs, 1);
        assert_eq!(last_modified(&pool).await.unwrap(), 42);
    }

    #[tokio::test]
    async fn album_rating_round_trips_and_syncs_the_user_rating() {
        let pool = memory_pool().await;
        let album = AlbumID3 {
            id: "al1".into(),
            name: "Kid A".into(),
            user_rating: Some(4),
            ..AlbumID3::default()
        };
        replace(&pool, &[], &[album], &[], 1).await.unwrap();
        assert_eq!(album_rating(&pool, "al1").await.unwrap(), 4);

        set_album_rating(&pool, "al1", 2).await.unwrap();
        assert_eq!(album_rating(&pool, "al1").await.unwrap(), 2);
        assert_eq!(album_rating(&pool, "missing").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn finds_songs_by_artist_and_title_case_insensitively() {
        let pool = memory_pool().await;
        sqlx::query(
            "INSERT INTO songs (id, title, artist) VALUES ('s1', 'Weird Fishes', 'Radiohead')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let by_both = find_song_id(&pool, Some("radiohead"), "WEIRD fishes")
            .await
            .unwrap();
        assert_eq!(by_both.as_deref(), Some("s1"));

        // No artist in the pasted text: title-only match.
        let by_title = find_song_id(&pool, None, "Weird Fishes").await.unwrap();
        assert_eq!(by_title.as_deref(), Some("s1"));

        // Wrong artist must not match.
        assert!(
            find_song_id(&pool, Some("Muse"), "Weird Fishes")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn order_cycles_both_ways() {
        assert_eq!(cycle_order("Artist", 1), "Name");
        assert_eq!(cycle_order("Name", 1), "Year");
        assert_eq!(cycle_order("Year", 1), "Artist");
        assert_eq!(cycle_order("Artist", -1), "Year");
        assert_eq!(cycle_order("Year", -1), "Name");
        // Unknown values snap back to the first order.
        assert_eq!(cycle_order("", 1), "Name");
    }

    #[test]
    fn artist_order_cycles_both_ways() {
        assert_eq!(cycle_artist_order("Name", 1), "Albums");
        assert_eq!(cycle_artist_order("Albums", 1), "Name");
        assert_eq!(cycle_artist_order("Name", -1), "Albums");
        assert_eq!(cycle_artist_order("", 1), "Albums");
    }

    #[tokio::test]
    async fn artist_orders_switch_between_name_and_album_count() {
        let pool = memory_pool().await;
        for (id, name, count) in [("a1", "Zed", 3), ("a2", "Alpha", 1), ("a3", "Beta", 5)] {
            sqlx::query("INSERT INTO artists (id, name, album_count) VALUES (?1, ?2, ?3)")
                .bind(id)
                .bind(name)
                .bind(count)
                .execute(&pool)
                .await
                .unwrap();
        }
        let by_name: Vec<String> = artists(&pool, "Name")
            .await
            .unwrap()
            .iter()
            .map(|artist| artist.name.clone())
            .collect();
        assert_eq!(by_name, ["Alpha", "Beta", "Zed"]);
        let by_albums: Vec<String> = artists(&pool, "Albums")
            .await
            .unwrap()
            .iter()
            .map(|artist| artist.name.clone())
            .collect();
        assert_eq!(by_albums, ["Beta", "Zed", "Alpha"]);
    }

    #[tokio::test]
    async fn artist_albums_follow_the_id_not_the_name() {
        let pool = memory_pool().await;
        for (id, name) in [("a1", "Radiohead"), ("a2", "Radiohead"), ("a3", "Muse")] {
            sqlx::query("INSERT INTO artists (id, name) VALUES (?1, ?2)")
                .bind(id)
                .bind(name)
                .execute(&pool)
                .await
                .unwrap();
        }
        for (id, artist_id, name, year) in [
            ("al1", "a1", "Kid A", 2000),
            ("al2", "a1", "OK Computer", 1997),
            ("al3", "a2", "Kid A", 2000),
            ("al4", "a3", "Origin of Symmetry", 2001),
        ] {
            sqlx::query(
                "INSERT INTO albums (id, artist_id, name, artist, year)
                 VALUES (?1, ?2, ?3, 'same-name', ?4)",
            )
            .bind(id)
            .bind(artist_id)
            .bind(name)
            .bind(year)
            .execute(&pool)
            .await
            .unwrap();
        }

        // Same display name, different id: the id decides.
        let ids = |albums: Vec<AlbumRow>| -> Vec<String> {
            albums.into_iter().map(|album| album.id).collect()
        };
        assert_eq!(
            ids(artist_albums(&pool, "a1", "Year").await.unwrap()),
            vec!["al1", "al2"]
        );
        assert_eq!(
            ids(artist_albums(&pool, "a2", "Year").await.unwrap()),
            vec!["al3"]
        );
        assert_eq!(
            ids(artist_albums(&pool, "a1", "Name").await.unwrap()),
            vec!["al1", "al2"]
        );
        assert!(
            artist_albums(&pool, "missing", "Name")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn song_pages_follow_all_songs_order_without_materializing_it() {
        let pool = memory_pool().await;
        for (id, artist, album, disc, track, title) in [
            (
                "s1",
                "Radiohead",
                "Kid A",
                1,
                1,
                "Everything In Its Right Place",
            ),
            ("s2", "Radiohead", "Kid A", 1, 2, "Kid A"),
            ("s3", "Muse", "Absolution", 1, 1, "Intro"),
            ("s4", "Muse", "Absolution", 1, 2, "Apocalypse Please"),
            ("s5", "Aurora", "All My Demons", 1, 1, "Runaway"),
        ] {
            sqlx::query(
                "INSERT INTO songs (id, artist, album, disc_number, track, title)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(id)
            .bind(artist)
            .bind(album)
            .bind(disc)
            .bind(track)
            .bind(title)
            .execute(&pool)
            .await
            .unwrap();
        }

        let all = all_songs(&pool, "Artist").await.unwrap();
        assert_eq!(song_count(&pool).await.unwrap(), all.len());
        for (index, song) in all.iter().enumerate() {
            assert_eq!(
                song_ordinal(&pool, &song.id, "Artist").await.unwrap(),
                Some(index)
            );
        }
        assert_eq!(
            song_ordinal(&pool, "missing", "Artist").await.unwrap(),
            None
        );

        // Pages line up with the same order, whatever the offset.
        let page = songs_range(&pool, 1, 2, "Artist").await.unwrap();
        let ids: Vec<&str> = page.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, vec![all[1].id.as_str(), all[2].id.as_str()]);
        assert!(
            songs_range(&pool, 10, 5, "Artist")
                .await
                .unwrap()
                .is_empty()
        );

        // A different label is the same machinery over another ORDER BY.
        let by_name = songs_range(&pool, 0, 2, "Name").await.unwrap();
        let ids: Vec<&str> = by_name.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, vec!["s4", "s1"]);
        assert_eq!(song_ordinal(&pool, "s4", "Name").await.unwrap(), Some(0));
    }

    #[tokio::test]
    async fn artist_info_round_trips_and_purges_with_the_artist() {
        let pool = memory_pool().await;
        sqlx::query("INSERT INTO artists (id, name) VALUES ('a1', 'Pixies')")
            .execute(&pool)
            .await
            .unwrap();

        assert!(artist_info(&pool, "a1").await.unwrap().is_none());

        upsert_artist_info(&pool, "a1", "Boston band.", "https://last.fm/pixies")
            .await
            .unwrap();
        let row = artist_info(&pool, "a1").await.unwrap().unwrap();
        assert_eq!(row.biography, "Boston band.");
        assert_eq!(row.url, "https://last.fm/pixies");
        assert!(!row.fetched_at.is_empty());

        // A refetch replaces the row, never duplicates it.
        upsert_artist_info(&pool, "a1", "Updated bio.", "")
            .await
            .unwrap();
        let row = artist_info(&pool, "a1").await.unwrap().unwrap();
        assert_eq!(row.biography, "Updated bio.");
        assert_eq!(row.url, "");

        // The artist left the library: the cached biography goes with it.
        sqlx::query("DELETE FROM artists WHERE id = 'a1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(purge_stale_artist_info(&pool).await.unwrap(), 1);
        assert!(artist_info(&pool, "a1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn artist_song_ids_and_file_info_feed_the_downloader() {
        let pool = memory_pool().await;
        sqlx::query(
            "INSERT INTO songs (id, title, artist_id, size, suffix, bit_rate)
             VALUES ('s1', 'A', 'a1', 123, 'mp3', 320)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO songs (id, title, artist_id) VALUES ('s2', 'B', 'a1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO songs (id, title, artist_id) VALUES ('s3', 'C', 'a2')")
            .execute(&pool)
            .await
            .unwrap();

        let mut ids = artist_song_ids(&pool, "a1").await.unwrap();
        ids.sort();
        assert_eq!(ids, vec!["s1", "s2"]);

        assert_eq!(
            song_file_info(&pool, "s1").await.unwrap(),
            Some((Some(123), Some("mp3".to_string()), Some(320)))
        );
        assert_eq!(
            song_file_info(&pool, "s2").await.unwrap(),
            Some((None, None, None))
        );
        assert!(song_file_info(&pool, "missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn downloaded_owners_count_only_the_selected_ids() {
        let pool = memory_pool().await;
        for (id, album, artist) in [
            ("s1", "al1", "ar1"),
            ("s2", "al1", "ar1"),
            ("s3", "al2", "ar1"),
            ("s4", "al3", "ar2"),
        ] {
            sqlx::query(
                "INSERT INTO songs (id, title, album_id, artist_id) VALUES (?1, ?1, ?2, ?3)",
            )
            .bind(id)
            .bind(album)
            .bind(artist)
            .execute(&pool)
            .await
            .unwrap();
        }

        let ids = vec!["s1".to_string(), "s3".to_string()];
        let (albums, artists) = downloaded_owners(&pool, &ids).await.unwrap();
        assert_eq!(albums.get("al1"), Some(&1));
        assert_eq!(albums.get("al2"), Some(&1));
        assert_eq!(albums.get("al3"), None);
        // ar1 owns two downloaded albums; ar2 owns none.
        assert_eq!(artists.get("ar1"), Some(&2));
        assert_eq!(artists.get("ar2"), None);

        // An empty selection is an empty catalog, not an error.
        let (albums, artists) = downloaded_owners(&pool, &[]).await.unwrap();
        assert!(albums.is_empty());
        assert!(artists.is_empty());
    }
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

#[cfg(test)]
mod path_tests {
    use super::*;

    async fn memory_pool() -> SqlitePool {
        crate::sqlite::memory_pool(sqlx::migrate!("./migrations_library")).await
    }

    #[tokio::test]
    async fn album_info_cache_and_cover_check_gate_round_trip() {
        let pool = memory_pool().await;
        sqlx::query("INSERT INTO albums (id, name) VALUES ('al1', 'Kid A')")
            .execute(&pool)
            .await
            .unwrap();

        assert!(album_info(&pool, "al1").await.unwrap().is_none());
        upsert_album_info(&pool, "al1", "notes").await.unwrap();
        let (notes, _) = album_info(&pool, "al1").await.unwrap().unwrap();
        assert_eq!(notes, "notes");

        upsert_album_info(&pool, "gone", "stale").await.unwrap();
        assert_eq!(purge_stale_album_info(&pool).await.unwrap(), 1);
        assert!(album_info(&pool, "gone").await.unwrap().is_none());

        assert!(cover_check_due(&pool, 3600).await.unwrap());
        touch_cover_check(&pool).await.unwrap();
        assert!(!cover_check_due(&pool, 3600).await.unwrap());
    }
}
