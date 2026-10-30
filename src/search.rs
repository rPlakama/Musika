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
use sqlx::AssertSqlSafe;
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

/// The columns a search hit shares with a library search row. The FTS table
/// names its primary key `song_id`, so alias it to the library role.
const SEARCH_COLUMNS: &str = "song_id AS id, title, artist, album,
    CAST(NULLIF(duration, '') AS INTEGER) AS duration,
    COALESCE(play_count, 0) AS play_count";

/// How much one play (from the server's play count) nudges an FTS rank.
/// Rank leads: a strong text match still beats a heavily played weak one.
const PLAY_BOOST: f64 = 0.1;

const INSERT_SEARCH: &str =
    "INSERT INTO song_search (title, artist, album, duration, song_id, play_count) ";

/// Binds one page of library rows into `song_search`.
pub(crate) async fn insert_search_rows(
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
pub async fn search(pool: &SqlitePool, raw: &str, limit: i64) -> Result<Vec<SongSearchRow>> {
    let query = fts_query(raw);
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT {SEARCH_COLUMNS} FROM song_search
         WHERE song_search MATCH ?1
         ORDER BY rank - COALESCE(play_count, 0) * ?3, rank
         LIMIT ?2"
    );
    let hits = sqlx::query_as::<_, SongSearchRow>(AssertSqlSafe(sql))
        .bind(query)
        .bind(limit)
        .bind(PLAY_BOOST)
        .fetch_all(pool)
        .await?;

    Ok(hits)
}

/// The empty-query surface (S-9): the most played songs, tie-broken by title.
/// A library with no play counts still returns its songs in title order.
pub async fn most_played(pool: &SqlitePool, limit: i64) -> Result<Vec<SongSearchRow>> {
    let sql = format!(
        "SELECT {SEARCH_COLUMNS} FROM song_search
         ORDER BY COALESCE(play_count, 0) DESC, title
         LIMIT ?1"
    );
    let hits = sqlx::query_as::<_, SongSearchRow>(AssertSqlSafe(sql))
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

pub(crate) fn fts_query(raw: &str) -> String {
    clean_query(raw)
        .split_whitespace()
        .map(|token| format!("\"{}\"*", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}
