//! `library.db` — artists, albums and songs metadata.
//!
//! The tables are replaced wholesale on every real sync; deletions on the
//! server are therefore reflected locally without any diffing.

#![allow(dead_code)] // Browse queries are prepared before the library UI lands.

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;

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
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = 'last_modified'")
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

/// Returns the columns needed to rebuild the FTS index.
pub async fn song_search_rows(pool: &SqlitePool) -> Result<Vec<SongSearchRow>> {
    let rows = sqlx::query_as::<_, SongSearchRow>(
        "SELECT id, title, artist, album, duration FROM songs",
    )
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
    let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
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

    let rows: Vec<(String, Option<String>)> =
        builder.build_query_as().fetch_all(pool).await?;
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

/// Albums for the central list, optionally filtered by artist name.
pub async fn albums(pool: &SqlitePool, artist: Option<&str>) -> Result<Vec<AlbumRow>> {
    let rows = match artist {
        Some(artist) => {
            sqlx::query_as::<_, AlbumRow>(
                "SELECT id, name AS title, artist, year, song_count, cover_art
                 FROM albums
                 WHERE artist = ?1
                 ORDER BY name",
            )
            .bind(artist)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, AlbumRow>(
                "SELECT id, name AS title, artist, year, song_count, cover_art
                 FROM albums
                 ORDER BY artist, name",
            )
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

/// All artists, ordered by name.
pub async fn artists(pool: &SqlitePool) -> Result<Vec<ArtistRow>> {
    let rows = sqlx::query_as::<_, ArtistRow>(
        "SELECT id, name, album_count, cover_art FROM artists ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Every song, ordered by artist/album/track. Covers come from the album.
pub async fn all_songs(pool: &SqlitePool) -> Result<Vec<SongRow>> {
    let rows = sqlx::query_as::<_, SongRow>(
        "SELECT s.id, s.title, s.artist, s.album, s.album_id, s.track, s.duration,
                COALESCE(al.cover_art, s.cover_art) AS cover_art
         FROM songs s LEFT JOIN albums al ON al.id = s.album_id
         ORDER BY s.artist, s.album, s.disc_number, s.track, s.title",
    )
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
    std::fs::write(&path, text)
        .with_context(|| format!("failed to write {}", path.display()))?;
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
