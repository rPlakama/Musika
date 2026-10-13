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

//! `search.db` — FTS5 index and querying.
//!
//! Separate from `library.db` so searches run on their own connection pool and
//! never wait on the library writer. It stores its own copy of the indexed
//! columns, because SQLite cannot join across databases.

use anyhow::Result;
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::library::SongSearchRow;

/// A full-text search result.
#[derive(Debug, Clone, FromRow)]
pub struct SearchHit {
    pub song_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[allow(dead_code)]
    pub duration: Option<i32>,
}

/// Rebuilds the whole FTS index from library rows.
///
/// There is no cross-database transaction, so this runs right after a
/// successful library write; if it fails, search is simply stale until the
/// next sync (the library itself stays consistent).
pub async fn rebuild(pool: &SqlitePool, rows: &[SongSearchRow]) -> Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM song_search")
        .execute(&mut *tx)
        .await?;

    for row in rows {
        sqlx::query(
            "INSERT INTO song_search (title, artist, album, duration, song_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&row.title)
        .bind(row.artist.as_deref().unwrap_or(""))
        .bind(row.album.as_deref().unwrap_or(""))
        .bind(row.duration.map(|value| value.to_string()))
        .bind(&row.id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Full-text search over song title, artist and album.
///
/// Input is tokenised on whitespace; each token is quoted and treated as a
/// prefix, so punctuation and FTS5 operators typed by the user are literal and
/// `radio` matches `Radiohead` while the palette is being typed.
pub async fn search(pool: &SqlitePool, raw: &str, limit: i64) -> Result<Vec<SearchHit>> {
    let query = fts_query(raw);
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let hits = sqlx::query_as::<_, SearchHit>(
        "SELECT song_id, title, artist, album,
                CAST(NULLIF(duration, '') AS INTEGER) AS duration
         FROM song_search
         WHERE song_search MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(hits)
}

fn fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn memory_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations_search")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn search_matches_song_metadata() {
        let pool = memory_pool().await;

        let rows = vec![SongSearchRow {
            id: "1".to_string(),
            title: "Hello World".to_string(),
            artist: Some("The Testers".to_string()),
            album: Some("First".to_string()),
            duration: Some(200),
        }];
        rebuild(&pool, &rows).await.unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].song_id, "1");
        assert_eq!(hits[0].artist.as_deref(), Some("The Testers"));
        assert_eq!(hits[0].duration, Some(200));

        assert!(search(&pool, "missing", 10).await.unwrap().is_empty());
        assert!(search(&pool, "   ", 10).await.unwrap().is_empty());
    }

    #[test]
    fn query_quotes_every_token() {
        assert_eq!(fts_query("hello world"), "\"hello\"* \"world\"*");
        assert_eq!(fts_query(""), "");
    }
}
