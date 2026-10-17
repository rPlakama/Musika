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
    pub play_count: i64,
}

/// How much one play (from the server's play count) nudges an FTS rank.
/// Rank leads: a strong text match still beats a heavily played weak one.
const PLAY_BOOST: f64 = 0.1;

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

    crate::cache::insert_many(
        &mut tx,
        "INSERT INTO song_search (title, artist, album, duration, song_id, play_count) ",
        rows,
        |mut values, row| {
            values
                .push_bind(&row.title)
                .push_bind(row.artist.as_deref().unwrap_or(""))
                .push_bind(row.album.as_deref().unwrap_or(""))
                .push_bind(row.duration.map(|value| value.to_string()))
                .push_bind(&row.id)
                .push_bind(row.play_count.unwrap_or(0));
        },
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Full-text search over song title, artist and album.
///
/// Input is tokenised on whitespace; each token is quoted and treated as a
/// prefix, so punctuation and FTS5 operators typed by the user are literal and
/// `radio` matches `Radiohead` while the palette is being typed. The rank is
/// nudged by the play count (S-9's usage boost) but the text rank still leads.
pub async fn search(pool: &SqlitePool, raw: &str, limit: i64) -> Result<Vec<SearchHit>> {
    let query = fts_query(raw);
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let hits = sqlx::query_as::<_, SearchHit>(
        "SELECT song_id, title, artist, album,
                CAST(NULLIF(duration, '') AS INTEGER) AS duration,
                COALESCE(play_count, 0) AS play_count
         FROM song_search
         WHERE song_search MATCH ?1
         ORDER BY rank - COALESCE(play_count, 0) * ?3, rank
         LIMIT ?2",
    )
    .bind(query)
    .bind(limit)
    .bind(PLAY_BOOST)
    .fetch_all(pool)
    .await?;

    Ok(hits)
}

/// The empty-query surface (S-9): the most played songs, tie-broken by title.
/// A library with no play counts still returns its songs in title order.
pub async fn most_played(pool: &SqlitePool, limit: i64) -> Result<Vec<SearchHit>> {
    let hits = sqlx::query_as::<_, SearchHit>(
        "SELECT song_id, title, artist, album,
                CAST(NULLIF(duration, '') AS INTEGER) AS duration,
                COALESCE(play_count, 0) AS play_count
         FROM song_search
         ORDER BY COALESCE(play_count, 0) DESC, title
         LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(hits)
}

/// The number of indexed rows; zero after the S-9 migration recreated the
/// table, which the sync's up-to-date branch uses to self-heal.
pub async fn count(pool: &SqlitePool) -> Result<i64> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM song_search")
        .fetch_one(pool)
        .await?;
    Ok(count.0)
}

fn fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_pool() -> SqlitePool {
        crate::sqlite::memory_pool(sqlx::migrate!("./migrations_search")).await
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
            play_count: Some(7),
        }];
        rebuild(&pool, &rows).await.unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].song_id, "1");
        assert_eq!(hits[0].artist.as_deref(), Some("The Testers"));
        assert_eq!(hits[0].duration, Some(200));
        assert_eq!(hits[0].play_count, 7);

        assert!(search(&pool, "missing", 10).await.unwrap().is_empty());
        assert!(search(&pool, "   ", 10).await.unwrap().is_empty());
    }

    fn row(id: &str, title: &str, plays: i64) -> SongSearchRow {
        SongSearchRow {
            id: id.to_string(),
            title: title.to_string(),
            artist: None,
            album: None,
            duration: Some(180),
            play_count: Some(plays),
        }
    }

    #[tokio::test]
    async fn most_played_orders_by_count_then_title() {
        let pool = memory_pool().await;
        rebuild(
            &pool,
            &[
                row("1", "Zebra", 0),
                row("2", "Alpha", 9),
                row("3", "Beta", 4),
                row("4", "Another", 9),
            ],
        )
        .await
        .unwrap();

        let hits = most_played(&pool, 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|hit| hit.song_id.as_str()).collect();
        // Two 9-play ties break by title, then the 4, then the 0.
        assert_eq!(ids, ["2", "4", "3", "1"]);
    }

    #[tokio::test]
    async fn play_count_boosts_equal_ranks() {
        let pool = memory_pool().await;
        // Same matched text => equal rank; the play count decides (S-9).
        rebuild(&pool, &[row("quiet", "Hello", 0), row("loud", "Hello", 40)])
            .await
            .unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|hit| hit.song_id.as_str()).collect();
        assert_eq!(ids, ["loud", "quiet"]);
    }

    #[tokio::test]
    async fn text_rank_leads_until_plays_are_large() {
        let pool = memory_pool().await;
        // bm25 magnitudes collapse to ~1e-6 in a two-row corpus, so use a
        // realistic size where the rank gap means something.
        let mut rows = vec![
            row("strong", "Hello Hello Hello Hello Hello", 0),
            row("weak", "Hello There", 3),
        ];
        for i in 0..20 {
            let id = format!("filler{i}");
            let title = format!("Filler {i}");
            rows.push(row(&id, &title, 0));
        }
        rebuild(&pool, &rows).await.unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(
            hits[0].song_id, "strong",
            "a strong match survives a small usage boost"
        );

        // Enough plays still overtake a better text match (the boost's job).
        rows[0].play_count = Some(0);
        rows[1].play_count = Some(60);
        rebuild(&pool, &rows).await.unwrap();
        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(hits[0].song_id, "weak");
    }

    #[test]
    fn query_quotes_every_token() {
        assert_eq!(fts_query("hello world"), "\"hello\"* \"world\"*");
        assert_eq!(fts_query(""), "");
    }
}
