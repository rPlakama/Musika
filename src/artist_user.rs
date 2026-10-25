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

//! DB8 `artist_user.db`: per-artist user metadata (ARC-42) — the local index
//! of the user's Subsonic `star` / `setRating` state.
//!
//! This is the storage layer only: the backend owns the API call and the
//! optimistic/revert policy, and writes here once the server accepted. Rows
//! survive library syncs (which would revert the server-side `artists.starred`
//! snapshot) until the server is asked again.

use anyhow::{Result, bail};
use sqlx::sqlite::SqlitePool;

/// One artist's user state; `rating` is 0..=5 (0 = unrated).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ArtistUser {
    pub artist_id: String,
    pub starred: bool,
    pub rating: i64,
    pub updated_at: String,
}

async fn upsert_artist_user(
    pool: &SqlitePool,
    artist_id: &str,
    starred: Option<bool>,
    rating: Option<u8>,
) -> Result<()> {
    if let Some(rating) = rating
        && rating > 5
    {
        bail!("artist rating {rating} is outside the 0..=5 domain");
    }
    sqlx::query(
        "INSERT INTO artist_user (artist_id, starred, rating, updated_at)
         VALUES (?1,
                 COALESCE(?2, (SELECT starred FROM artist_user WHERE artist_id = ?1), 0),
                 COALESCE(?3, (SELECT rating FROM artist_user WHERE artist_id = ?1), 0),
                 ?4)
         ON CONFLICT(artist_id) DO UPDATE SET
             starred    = COALESCE(?2, artist_user.starred),
             rating     = COALESCE(?3, artist_user.rating),
             updated_at = excluded.updated_at",
    )
    .bind(artist_id)
    .bind(starred)
    .bind(rating.map(i64::from))
    .bind(crate::consts::now_secs().to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Records the favourite state, preserving any stored rating.
pub async fn set_starred(pool: &SqlitePool, artist_id: &str, starred: bool) -> Result<()> {
    upsert_artist_user(pool, artist_id, Some(starred), None).await
}

/// Records the rating, preserving the stored favourite state. Ratings are the
/// Subsonic 0..=5 domain; anything else is refused at this boundary.
pub async fn set_rating(pool: &SqlitePool, artist_id: &str, rating: u8) -> Result<()> {
    upsert_artist_user(pool, artist_id, None, Some(rating)).await
}

/// One artist's stored state; `None` when the user never touched it.
pub async fn get(pool: &SqlitePool, artist_id: &str) -> Result<Option<ArtistUser>> {
    let row = sqlx::query_as::<_, ArtistUser>(
        "SELECT artist_id, starred, rating, updated_at
         FROM artist_user WHERE artist_id = ?1",
    )
    .bind(artist_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// UI-60: every stored artist rating above zero (id -> 0..=5), for the row
/// payloads' `rating` field.
pub async fn ratings(pool: &SqlitePool) -> Result<std::collections::HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT artist_id, rating FROM artist_user WHERE rating > 0")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// UI-60: imports the server's `userRating` snapshots after a library sync
/// (the server is the authority; a rating made elsewhere lands here).
/// Out-of-domain values are ignored.
pub async fn import_ratings(pool: &SqlitePool, ratings: &[(String, i64)]) -> Result<()> {
    let valid: Vec<(&str, i64)> = ratings
        .iter()
        .filter_map(|(artist_id, rating)| {
            u8::try_from(*rating)
                .ok()
                .filter(|rating| *rating <= 5)
                .map(|rating| (artist_id.as_str(), i64::from(rating)))
        })
        .collect();
    if valid.is_empty() {
        return Ok(());
    }
    let now = crate::consts::now_secs().to_string();
    let mut tx = pool.begin().await?;
    for chunk in valid.chunks(crate::cache::IN_BATCH) {
        let mut query = sqlx::QueryBuilder::new(
            "INSERT INTO artist_user (artist_id, starred, rating, updated_at) ",
        );
        query.push_values(chunk, |mut row, entry| {
            row.push_bind(entry.0)
                .push_bind(0i64)
                .push_bind(entry.1)
                .push_bind(now.as_str());
        });
        query.push(
            " ON CONFLICT(artist_id) DO UPDATE SET
                  rating = excluded.rating,
                  updated_at = excluded.updated_at",
        );
        query.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Every starred artist id, ordered for stable favourites listings.
/// (P-2's favourites view is the consumer; exercised by the tests today.)
#[allow(dead_code)]
pub async fn starred_ids(pool: &SqlitePool) -> Result<Vec<String>> {
    let ids = sqlx::query_scalar(
        "SELECT artist_id FROM artist_user WHERE starred = 1 ORDER BY artist_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// The QML `artist_user` payload for one artist (defaults when unknown).
pub async fn payload(pool: &SqlitePool, artist_id: &str) -> String {
    let (starred, rating) = match get(pool, artist_id).await {
        Ok(Some(row)) => (row.starred, row.rating),
        Ok(None) => (false, 0),
        Err(err) => {
            eprintln!("artist_user: could not read {artist_id}: {err:#}");
            (false, 0)
        }
    };
    serde_json::json!({
        "id": artist_id,
        "starred": starred,
        "rating": rating,
    })
    .to_string()
}
