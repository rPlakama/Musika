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

pub mod index;

/// The active catalog engine (`MUSIKA_SEARCH_ENGINE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Nucleo answers first; FTS only fills the columns Nucleo left short.
    Hybrid,
    /// The in-process fuzzy matcher only.
    Nucleo,
    /// The SQLite FTS5 engine only.
    Fts,
}

/// Hybrid is the default; `nucleo` and `fts` are the opt-outs.
pub fn engine() -> Engine {
    match std::env::var("MUSIKA_SEARCH_ENGINE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "nucleo" => Engine::Nucleo,
        "fts" => Engine::Fts,
        _ => Engine::Hybrid,
    }
}

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

const INSERT_SEARCH: &str =
    "INSERT INTO song_search (title, artist, album, duration, song_id, play_count) ";

/// Binds one page of library rows into `song_search`.
async fn insert_search_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    rows: &[SongSearchRow],
) -> Result<()> {
    crate::cache::insert_many(tx, INSERT_SEARCH, rows, |mut values, row| {
        values
            .push_bind(&row.title)
            .push_bind(row.artist.as_deref().unwrap_or(""))
            .push_bind(row.album.as_deref().unwrap_or(""))
            .push_bind(row.duration.map(|value| value.to_string()))
            .push_bind(&row.id)
            .push_bind(row.play_count.unwrap_or(0));
    })
    .await
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

    insert_search_rows(&mut tx, rows).await?;

    tx.commit().await?;
    Ok(())
}

/// Rebuilds the FTS index straight from `library.db`, one bounded page at a
/// time.
///
/// The sync used to load every indexed row into a `Vec<SongSearchRow>` before
/// opening the search transaction (tens of MB on a large library). Keyset
/// pagination on the primary key keeps the same transaction but holds only one
/// page (plus the insert batch) at once.
pub async fn rebuild_from_library(pool: &SqlitePool) -> Result<()> {
    /// Rows per keyset page; the insert helper batches again at 32.
    const PAGE: i64 = 500;

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM song_search")
        .execute(&mut *tx)
        .await?;

    let mut after = String::new();
    loop {
        let rows: Vec<SongSearchRow> = sqlx::query_as(
            "SELECT id, title, artist, album, duration, play_count
             FROM songs WHERE id > ?1 ORDER BY id LIMIT ?2",
        )
        .bind(&after)
        .bind(PAGE)
        .fetch_all(&mut *tx)
        .await?;
        let Some(last) = rows.last() else {
            break;
        };
        after.clear();
        after.push_str(&last.id);
        insert_search_rows(&mut tx, &rows).await?;
    }

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

/// Strips punctuation from the user's prose so `(hello`, `"kid a"` and
/// `[1979]` narrow to the same words as the bare text. Letters and digits
/// (Unicode-aware) and whitespace survive; every other character becomes a
/// separator and the whitespace runs collapse.
pub fn clean_query(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn fts_query(raw: &str) -> String {
    clean_query(raw)
        .split_whitespace()
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
    async fn rebuild_from_library_pages_every_song() {
        // 600 rows spans more than one 500-row keyset page.
        let pool = crate::sqlite::memory_pool(sqlx::migrate!("./migrations_library")).await;
        sqlx::raw_sql(
            "INSERT INTO songs (id, title, artist, album, duration, play_count)
             SELECT printf('s%04d', x), 'Track ' || x, 'Artist', 'Album', 180, x % 5
             FROM (WITH RECURSIVE c(x) AS (
                       VALUES(1) UNION ALL SELECT x + 1 FROM c WHERE x < 600
                   ) SELECT x FROM c);",
        )
        .execute(&pool)
        .await
        .unwrap();

        rebuild_from_library(&pool).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 600);
        let hits = search(&pool, "599", 10).await.unwrap();
        assert_eq!(hits[0].song_id, "s0599");

        // A second pass replaces the table instead of appending to it.
        rebuild_from_library(&pool).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 600);
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
    fn query_quotes_every_token_and_ignores_punctuation() {
        assert_eq!(fts_query("hello world"), "\"hello\"* \"world\"*");
        assert_eq!(fts_query(""), "");
        assert_eq!(clean_query("(hello) \"kid a\" [1979]"), "hello kid a 1979");
        assert_eq!(fts_query("(hello)"), "\"hello\"*");
    }

    #[tokio::test]
    async fn search_ignores_punctuation_in_the_query() {
        let pool = memory_pool().await;
        rebuild(&pool, &[row("1", "Hello World", 0)]).await.unwrap();

        for query in ["(hello)", "\"world\"", "[hello] world"] {
            let hits = search(&pool, query, 10).await.unwrap();
            assert_eq!(hits.len(), 1, "query {query:?}");
            assert_eq!(hits[0].song_id, "1");
        }
    }

    /// Memory probe: the paged FTS rebuild vs the old
    /// `song_search_rows()` + `rebuild(&rows)` path at 20k songs.
    ///
    /// `cargo test search::tests::bench_fts_rebuild_peak -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_fts_rebuild_peak() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let pool = rt.block_on(crate::sqlite::memory_pool(sqlx::migrate!(
            "./migrations_library"
        )));
        rt.block_on(async {
            sqlx::raw_sql(
                "INSERT INTO songs (id, title, artist, album, duration, play_count)
                 SELECT printf('s%06d', x), 'Track ' || x, 'Artist ' || (x % 200),
                        'Album ' || (x % 2000), 180, x % 5
                 FROM (WITH RECURSIVE c(x) AS (
                           VALUES(1) UNION ALL SELECT x + 1 FROM c WHERE x < 20000
                       ) SELECT x FROM c);",
            )
            .execute(&pool)
            .await
            .expect("seed songs");
        });

        let (_, old_peak) = crate::test_alloc::measure(|| {
            rt.block_on(async {
                let rows = crate::library::song_search_rows(&pool)
                    .await
                    .expect("collect rows");
                rebuild(&pool, &rows).await.expect("rebuild from vec");
            })
        });
        let (_, paged_peak) = crate::test_alloc::measure(|| {
            rt.block_on(rebuild_from_library(&pool))
                .expect("paged rebuild")
        });

        eprintln!(
            "FTS rebuild (20k songs): paged peak {} KiB | Vec + rebuild peak {} KiB",
            paged_peak / 1024,
            old_peak / 1024,
        );
        assert!(
            paged_peak < old_peak,
            "the paged rebuild must not hold the whole row set"
        );
        assert_eq!(rt.block_on(count(&pool)).unwrap(), 20_000);
    }
}
