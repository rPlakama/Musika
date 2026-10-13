//! `search.db` — FTS5 index and querying.
//!
//! Separate from `library.db` so searches run on their own connection pool and
//! never wait on the library writer. It stores its own copy of the indexed
//! columns, because SQLite cannot join across databases.

use anyhow::Result;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;

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

    sqlx::query("DELETE FROM song_search").execute(&mut *tx).await?;

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

// ---------------------------------------------------------------------------
// Floating palette (FTS5 over `search.db`)
// ---------------------------------------------------------------------------

/// A palette hit: the song id plus the display label.
#[derive(Debug, Clone, Serialize)]
pub struct PaletteHit {
    pub id: String,
    pub label: String,
}

/// Palette search: runs the FTS5 query and formats each hit as
/// `title - artist - album`, skipping fields the server left empty.
pub async fn search_palette(pool: &SqlitePool, raw: &str, limit: i64) -> Result<Vec<PaletteHit>> {
    let hits = search(pool, raw, limit).await?;
    Ok(hits
        .into_iter()
        .map(|hit| {
            let label = palette_label(&hit);
            PaletteHit {
                id: hit.song_id,
                label,
            }
        })
        .collect())
}

fn palette_label(hit: &SearchHit) -> String {
    [Some(hit.title.as_str()), hit.artist.as_deref(), hit.album.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" - ")
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
        sqlx::migrate!("./migrations_search").run(&pool).await.unwrap();
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

    #[tokio::test]
    async fn palette_matches_prefixes_and_labels_hits() {
        let pool = memory_pool().await;

        let rows = vec![
            SongSearchRow {
                id: "1".to_string(),
                title: "Weird Fishes".to_string(),
                artist: Some("Radiohead".to_string()),
                album: Some("In Rainbows".to_string()),
                duration: Some(300),
            },
            SongSearchRow {
                id: "2".to_string(),
                title: "Everything In Its Right Place".to_string(),
                artist: None,
                album: None,
                duration: None,
            },
        ];
        rebuild(&pool, &rows).await.unwrap();

        let hits = search_palette(&pool, "weird fis", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "1");
        assert_eq!(hits[0].label, "Weird Fishes - Radiohead - In Rainbows");

        let hits = search_palette(&pool, "everything", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "2");
        assert_eq!(hits[0].label, "Everything In Its Right Place");

        assert!(search_palette(&pool, "   ", 10).await.unwrap().is_empty());
    }
}
