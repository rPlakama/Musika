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

use anyhow::{Context, Result};
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
}

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

    for artist in artists {
        sqlx::query(
            "INSERT INTO artists (id, name, album_count, cover_art, image_url, starred)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&artist.id)
        .bind(&artist.name)
        .bind(artist.album_count)
        .bind(&artist.cover_art)
        .bind(&artist.artist_image_url)
        .bind(&artist.starred)
        .execute(&mut *tx)
        .await?;
    }

    for album in albums {
        sqlx::query(
            "INSERT INTO albums
                 (id, artist_id, name, artist, year, genre, song_count, duration, cover_art, starred)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(&album.id)
        .bind(&album.artist_id)
        .bind(&album.name)
        .bind(&album.artist)
        .bind(album.year)
        .bind(&album.genre)
        .bind(album.song_count)
        .bind(album.duration)
        .bind(&album.cover_art)
        .bind(&album.starred)
        .execute(&mut *tx)
        .await?;
    }

    for song in songs {
        sqlx::query(
            "INSERT OR REPLACE INTO songs
                 (id, album_id, artist_id, title, album, artist, track, disc_number, year,
                  genre, duration, bit_rate, suffix, size, path, cover_art, play_count, starred)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        )
        .bind(&song.id)
        .bind(&song.album_id)
        .bind(&song.artist_id)
        .bind(&song.title)
        .bind(&song.album)
        .bind(&song.artist)
        .bind(song.track)
        .bind(song.disc_number)
        .bind(song.year)
        .bind(&song.genre)
        .bind(song.duration)
        .bind(song.bit_rate)
        .bind(&song.suffix)
        .bind(song.size)
        .bind(&song.path)
        .bind(&song.cover_art)
        .bind(song.play_count)
        .bind(&song.starred)
        .execute(&mut *tx)
        .await?;
    }

    set_meta(&mut tx, "last_modified", &last_modified.to_string()).await?;
    set_meta(&mut tx, "last_sync", &unix_now().to_string()).await?;

    tx.commit().await?;
    Ok(())
}

/// Records that a sync was attempted even when the server reported no changes.
pub async fn touch_last_sync(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES ('last_sync', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(unix_now().to_string())
    .execute(pool)
    .await?;
    Ok(())
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

/// Returns the columns needed to rebuild the FTS index.
pub async fn song_search_rows(pool: &SqlitePool) -> Result<Vec<SongSearchRow>> {
    let rows =
        sqlx::query_as::<_, SongSearchRow>("SELECT id, title, artist, album, duration FROM songs")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// Artists and albums matching `query` by case-insensitive substring, for the
/// floating search (songs come from the FTS index in `search.db`).
pub async fn search_artists_albums(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> Result<(Vec<ArtistRow>, Vec<AlbumRow>)> {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let like = format!("%{escaped}%");

    let artists = sqlx::query_as::<_, ArtistRow>(
        "SELECT id, name, album_count, cover_art
         FROM artists
         WHERE name LIKE ?1 ESCAPE '\\'
         ORDER BY name
         LIMIT ?2",
    )
    .bind(&like)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let albums = sqlx::query_as::<_, AlbumRow>(
        "SELECT id, name AS title, artist, year, song_count, cover_art
         FROM albums
         WHERE name LIKE ?1 ESCAPE '\\' OR artist LIKE ?1 ESCAPE '\\'
         ORDER BY artist, name
         LIMIT ?2",
    )
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
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id.clone());
    }
    separated.push_unseparated(")");

    let rows: Vec<(String, Option<String>)> = builder.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, cover_art)| cover_art.map(|cover_art| (id, cover_art)))
        .collect())
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
}

/// A row for the artist list.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct ArtistRow {
    pub id: String,
    pub name: String,
    pub album_count: i64,
    pub cover_art: Option<String>,
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

/// Albums for the central list, optionally filtered by artist name, ordered
/// by one of [`ORDERS`]. Kept as static SQL (no interpolation) so the
/// allowlist is enforced at the type level.
pub async fn albums(pool: &SqlitePool, artist: Option<&str>, order: &str) -> Result<Vec<AlbumRow>> {
    let sql: &'static str = match (artist.is_some(), order) {
        (true, "Name") => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums WHERE artist = ?1 ORDER BY name"
        }
        (true, "Year") => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums WHERE artist = ?1 ORDER BY year DESC, name"
        }
        (true, _) => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums WHERE artist = ?1 ORDER BY artist, name"
        }
        (false, "Name") => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums ORDER BY name"
        }
        (false, "Year") => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums ORDER BY year DESC, name"
        }
        (false, _) => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums ORDER BY artist, name"
        }
    };

    let rows = match artist {
        Some(artist) => {
            sqlx::query_as::<_, AlbumRow>(sql)
                .bind(artist)
                .fetch_all(pool)
                .await?
        }
        None => sqlx::query_as::<_, AlbumRow>(sql).fetch_all(pool).await?,
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
    let sql: &'static str = match order {
        "Name" => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums WHERE artist_id = ?1 ORDER BY name"
        }
        "Year" => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums WHERE artist_id = ?1 ORDER BY year DESC, name"
        }
        _ => {
            "SELECT id, name AS title, artist, year, song_count, cover_art
             FROM albums WHERE artist_id = ?1 ORDER BY year, name"
        }
    };
    let rows = sqlx::query_as::<_, AlbumRow>(sql)
        .bind(artist_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// All artists, ordered by one of [`ARTIST_ORDERS`].
pub async fn artists(pool: &SqlitePool, order: &str) -> Result<Vec<ArtistRow>> {
    let sql: &'static str = match order {
        "Albums" => {
            "SELECT id, name, album_count, cover_art FROM artists
             ORDER BY album_count DESC, name"
        }
        _ => "SELECT id, name, album_count, cover_art FROM artists ORDER BY name",
    };
    let rows = sqlx::query_as::<_, ArtistRow>(sql).fetch_all(pool).await?;
    Ok(rows)
}

/// Every song, ordered by one of [`ORDERS`]. Covers come from the album.
pub async fn all_songs(pool: &SqlitePool, order: &str) -> Result<Vec<SongRow>> {
    let sql: &'static str = match order {
        "Name" => {
            "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id
             ORDER BY s.title, s.artist, s.album"
        }
        "Year" => {
            "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id
             ORDER BY s.year DESC, s.title, s.artist"
        }
        _ => {
            "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id
             ORDER BY s.artist, s.album, s.disc_number, s.track, s.title"
        }
    };
    let rows = sqlx::query_as::<_, SongRow>(sql).fetch_all(pool).await?;
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
    let sql: &'static str = match order {
        "Name" => {
            "SELECT rn - 1 FROM (
                 SELECT id, ROW_NUMBER() OVER (ORDER BY title, artist, album) AS rn
                 FROM songs
             ) WHERE id = ?1"
        }
        "Year" => {
            "SELECT rn - 1 FROM (
                 SELECT id, ROW_NUMBER() OVER (ORDER BY year DESC, title, artist) AS rn
                 FROM songs
             ) WHERE id = ?1"
        }
        _ => {
            "SELECT rn - 1 FROM (
                 SELECT id, ROW_NUMBER() OVER (
                            ORDER BY artist, album, disc_number, track, title) AS rn
                 FROM songs
             ) WHERE id = ?1"
        }
    };
    let ordinal: Option<i64> = sqlx::query_scalar(sql)
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
    let sql: &'static str = match order {
        "Name" => {
            "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id
             ORDER BY s.title, s.artist, s.album
             LIMIT ?1 OFFSET ?2"
        }
        "Year" => {
            "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id
             ORDER BY s.year DESC, s.title, s.artist
             LIMIT ?1 OFFSET ?2"
        }
        _ => {
            "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                    COALESCE(al.cover_art, s.cover_art) AS cover_art
             FROM songs s LEFT JOIN albums al ON al.id = s.album_id
             ORDER BY s.artist, s.album, s.disc_number, s.track, s.title
             LIMIT ?1 OFFSET ?2"
        }
    };
    let rows = sqlx::query_as::<_, SongRow>(sql)
        .bind(limit.max(0))
        .bind(offset.max(0))
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// A single song by id.
pub async fn song(pool: &SqlitePool, id: &str) -> Result<Option<SongRow>> {
    let row = sqlx::query_as::<_, SongRow>(
        "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                COALESCE(al.cover_art, s.cover_art) AS cover_art
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id
         WHERE s.id = ?1",
    )
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
    let row = sqlx::query_as::<_, AlbumRow>(
        "SELECT id, name AS title, artist, year, song_count, cover_art
         FROM albums
         WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// A single artist by id.
pub async fn artist(pool: &SqlitePool, id: &str) -> Result<Option<ArtistRow>> {
    let row = sqlx::query_as::<_, ArtistRow>(
        "SELECT id, name, album_count, cover_art FROM artists WHERE id = ?1",
    )
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

    let mut builder = sqlx::QueryBuilder::new(
        "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                COALESCE(al.cover_art, s.cover_art) AS cover_art
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id
         WHERE s.id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id.clone());
    }
    separated.push_unseparated(")");

    let rows: Vec<SongRow> = builder.build_query_as().fetch_all(pool).await?;
    let mut by_id: std::collections::HashMap<String, SongRow> =
        rows.into_iter().map(|row| (row.id.clone(), row)).collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// Songs of one album, in disc/track order. Covers come from the album.
pub async fn album_songs(pool: &SqlitePool, album_id: &str) -> Result<Vec<SongRow>> {
    let rows = sqlx::query_as::<_, SongRow>(
        "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                COALESCE(al.cover_art, s.cover_art) AS cover_art
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id
         WHERE s.album_id = ?1
         ORDER BY s.disc_number, s.track, s.title",
    )
    .bind(album_id)
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

/// Every song id of one artist (CACHE-5 rule materialization).
pub async fn artist_song_ids(pool: &SqlitePool, artist_id: &str) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar("SELECT id FROM songs WHERE artist_id = ?1")
        .bind(artist_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

async fn set_meta(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    async fn memory_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .unwrap();
        pool
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
}
