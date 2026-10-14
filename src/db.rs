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

//! Database layout for Musika.
//!
//! Eight independent SQLite databases live under `~/.config/musika` (each
//! with its own connection pool, so separate worker threads can write without
//! contending on a single writer lock):
//!
//! - `credentials.db` — encrypted account information (DB1)
//! - `library.db`     — artists / albums / songs metadata (DB2a)
//! - `search.db`      — FTS5 index over song metadata (DB2b)
//! - `covers.db`      — cover-art cache index; blobs sit in `covers/` (DB3)
//! - `lyrics.db`      — lyric cache (DB5)
//! - `colors.db`      — cover-seeded dynamic color cache (DB6)
//! - `statussave.db`  — last session status, restored on login (DB7)
//! - `artist_user.db` — per-artist favourite/rating user state (DB8)

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteJournalMode, SqlitePool, SqlitePoolOptions};

/// The eight databases, opened and migrated at start-up.
#[derive(Clone)]
pub struct Dbs {
    pub credentials: SqlitePool,
    pub library: SqlitePool,
    pub search: SqlitePool,
    pub covers: SqlitePool,
    pub lyrics: SqlitePool,
    pub colors: SqlitePool,
    pub status: SqlitePool,
    pub artist_user: SqlitePool,
}

/// Opens every database and applies its migrations.
pub async fn init() -> Result<Dbs> {
    let credentials = open_file(&credentials_path()?, 5).await?;
    // Two early no-op migrations were removed (ARC-18); ignore them if this
    // database already applied them.
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(&credentials)
        .await?;

    let library = open_file(&library_path()?, 5).await?;
    sqlx::migrate!("./migrations_library").run(&library).await?;

    let search = open_file(&search_path()?, 3).await?;
    sqlx::migrate!("./migrations_search").run(&search).await?;

    let covers = open_file(&covers_path()?, 5).await?;
    sqlx::migrate!("./migrations_covers").run(&covers).await?;

    let lyrics = open_file(&lyrics_path()?, 3).await?;
    sqlx::migrate!("./migrations_lyrics").run(&lyrics).await?;

    let colors = open_file(&colors_path()?, 3).await?;
    sqlx::migrate!("./migrations_colors").run(&colors).await?;

    // Single row, tiny: one connection is plenty.
    let status = open_file(&status_path()?, 1).await?;
    sqlx::migrate!("./migrations_status").run(&status).await?;

    let artist_user = open_file(&artist_user_path()?, 3).await?;
    sqlx::migrate!("./migrations_artist_user")
        .run(&artist_user)
        .await?;

    Ok(Dbs {
        credentials,
        library,
        search,
        covers,
        lyrics,
        colors,
        status,
        artist_user,
    })
}

async fn open_file(path: &Path, max_connections: u32) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;

    Ok(pool)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn credentials_path() -> Result<PathBuf> {
    db_path("credentials.db", "MUSIKA_CREDENTIALS_FILE")
}

pub fn library_path() -> Result<PathBuf> {
    db_path("library.db", "MUSIKA_LIBRARY_FILE")
}

pub fn search_path() -> Result<PathBuf> {
    db_path("search.db", "MUSIKA_SEARCH_FILE")
}

pub fn covers_path() -> Result<PathBuf> {
    db_path("covers.db", "MUSIKA_COVERS_FILE")
}

pub fn lyrics_path() -> Result<PathBuf> {
    db_path("lyrics.db", "MUSIKA_LYRICS_FILE")
}

pub fn colors_path() -> Result<PathBuf> {
    db_path("colors.db", "MUSIKA_COLORS_FILE")
}

pub fn status_path() -> Result<PathBuf> {
    db_path("statussave.db", "MUSIKA_STATUS_FILE")
}

pub fn artist_user_path() -> Result<PathBuf> {
    db_path("artist_user.db", "MUSIKA_ARTIST_USER_FILE")
}

/// Plain-text mirror of the library, for external tools such as `rg`.
pub fn mirror_path() -> Result<PathBuf> {
    db_path("index.txt", "MUSIKA_INDEX_MIRROR")
}

/// Directory holding the cached cover-art blobs.
pub fn covers_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_env("MUSIKA_COVERS_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(config_dir()?.join("covers"))
}

fn db_path(name: &str, override_env: &str) -> Result<PathBuf> {
    if let Some(path) = non_empty_env(override_env) {
        return Ok(PathBuf::from(path));
    }
    Ok(config_dir()?.join(name))
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_env("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(dir).join("musika"));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config").join("musika"))
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}
