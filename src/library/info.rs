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

//! Cached album notes and artist biographies (PATH-8/PATH-9).

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

use super::sync::upsert_meta;

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
