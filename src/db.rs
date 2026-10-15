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
//! Ten independent SQLite databases live under `~/.config/musika` (each
//! with its own connection pool, so separate worker threads can write without
//! contending on a single writer lock):
//!
//! - `credentials.db` — encrypted account information (DB1)
//! - `library.db`     — artists / albums / songs metadata (DB2a)
//! - `search.db`      — FTS5 index over song metadata (DB2b)
//! - `covers.db`      — cover-art cache index; blobs sit in `covers/` (DB3)
//! - `media.db`       — offline audio cache index; blobs in `media/` (DB4)
//! - `lyrics.db`      — lyric cache (DB5)
//! - `colors.db`      — cover-seeded dynamic color cache (DB6)
//! - `statussave.db`  — last session status, restored on login (DB7)
//! - `artist_user.db` — per-artist favourite/rating user state (DB8)
//! - `download.db`    — per-song download intent/progress ledger (DB9)

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteJournalMode, SqlitePool, SqlitePoolOptions};

/// The ten databases, opened and migrated at start-up.
#[derive(Clone)]
pub struct Dbs {
    pub credentials: SqlitePool,
    pub library: SqlitePool,
    pub search: SqlitePool,
    pub covers: SqlitePool,
    pub media: SqlitePool,
    pub lyrics: SqlitePool,
    pub colors: SqlitePool,
    pub status: SqlitePool,
    pub artist_user: SqlitePool,
    pub download: SqlitePool,
}

/// Opens every database and applies its migrations.
pub async fn init() -> Result<Dbs> {
    let dbs = Dbs {
        credentials: open_file(&credentials_path()?, 5).await?,
        library: open_file(&library_path()?, 5).await?,
        search: open_file(&search_path()?, 3).await?,
        covers: open_file(&covers_path()?, 5).await?,
        media: open_file(&media_path()?, 3).await?,
        lyrics: open_file(&lyrics_path()?, 3).await?,
        colors: open_file(&colors_path()?, 3).await?,
        // Single row, tiny: one connection is plenty.
        status: open_file(&status_path()?, 1).await?,
        artist_user: open_file(&artist_user_path()?, 3).await?,
        download: open_file(&download_path()?, 3).await?,
    };
    migrate(&dbs).await?;
    Ok(dbs)
}

/// Applies every pool's embedded migrations (shared by [`init`] and [`wipe`]).
pub async fn migrate(dbs: &Dbs) -> Result<()> {
    // Two early no-op migrations were removed (ARC-18); ignore them if this
    // database already applied them.
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(&dbs.credentials)
        .await?;
    sqlx::migrate!("./migrations_library")
        .run(&dbs.library)
        .await?;
    sqlx::migrate!("./migrations_search")
        .run(&dbs.search)
        .await?;
    sqlx::migrate!("./migrations_covers")
        .run(&dbs.covers)
        .await?;
    sqlx::migrate!("./migrations_media").run(&dbs.media).await?;
    sqlx::migrate!("./migrations_lyrics")
        .run(&dbs.lyrics)
        .await?;
    sqlx::migrate!("./migrations_colors")
        .run(&dbs.colors)
        .await?;
    sqlx::migrate!("./migrations_status")
        .run(&dbs.status)
        .await?;
    sqlx::migrate!("./migrations_artist_user")
        .run(&dbs.artist_user)
        .await?;
    sqlx::migrate!("./migrations_download")
        .run(&dbs.download)
        .await?;
    Ok(())
}

/// Wipes only the library metadata (`library.db`): artists, albums, songs,
/// artist info and the sync bookkeeping. The next sync repopulates it; the
/// derived `index.txt` mirror goes with it.
pub async fn wipe_library(dbs: &Dbs) -> Result<()> {
    wipe_pool(&dbs.library).await?;
    sqlx::migrate!("./migrations_library")
        .run(&dbs.library)
        .await?;
    remove_mirror();
    Ok(())
}

/// Wipes the derived text indexes (`search.db` FTS plus the `index.txt`
/// mirror). Both rebuild from `library.db` on the next sync.
pub async fn wipe_indexes(dbs: &Dbs) -> Result<()> {
    wipe_pool(&dbs.search).await?;
    sqlx::migrate!("./migrations_search")
        .run(&dbs.search)
        .await?;
    remove_mirror();
    Ok(())
}

/// Wipes the cover-art cache: `covers.db`, the blob directory and the
/// cover-derived color cache (`colors.db`), all refetched on demand.
pub async fn wipe_covers(dbs: &Dbs) -> Result<()> {
    wipe_pool(&dbs.covers).await?;
    sqlx::migrate!("./migrations_covers")
        .run(&dbs.covers)
        .await?;
    wipe_pool(&dbs.colors).await?;
    sqlx::migrate!("./migrations_colors")
        .run(&dbs.colors)
        .await?;
    remove_dir(&covers_dir()?)?;
    Ok(())
}

fn remove_mirror() {
    if let Ok(mirror) = mirror_path() {
        let _ = std::fs::remove_file(mirror);
    }
}

fn remove_dir(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", dir.display())),
    }
}

/// Destructive recovery: every database is stripped to an empty schema and
/// re-migrated, and the derived blobs (covers/media) and the `index.txt`
/// mirror are removed. Credentials go too, so the caller must return to the
/// login screen afterwards.
pub async fn wipe(dbs: &Dbs) -> Result<()> {
    for pool in [
        &dbs.credentials,
        &dbs.library,
        &dbs.search,
        &dbs.covers,
        &dbs.media,
        &dbs.lyrics,
        &dbs.colors,
        &dbs.status,
        &dbs.artist_user,
        &dbs.download,
    ] {
        wipe_pool(pool).await?;
    }
    migrate(dbs).await?;

    for dir in [covers_dir(), media_dir(), playback_dir()] {
        if let Ok(dir) = dir {
            remove_dir(&dir)?;
        }
    }
    remove_mirror();
    Ok(())
}

/// Drops every user table in one database (virtual/FTS tables first, so their
/// shadow tables fall with them; FK checks are off while DROP implicitly
/// deletes rows).
async fn wipe_pool(pool: &SqlitePool) -> Result<()> {
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY (sql LIKE 'CREATE VIRTUAL%') DESC, name",
    )
    .fetch_all(&mut *conn)
    .await?;
    for table in tables {
        // The name comes from sqlite_master, never from user input, and is
        // double-quoted with embedded quotes escaped.
        let escaped = table.replace('"', "\"\"");
        let sql = sqlx::AssertSqlSafe(format!("DROP TABLE IF EXISTS \"{escaped}\""));
        sqlx::raw_sql(sql)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("failed to drop {table}"))?;
    }
    Ok(())
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

pub fn media_path() -> Result<PathBuf> {
    db_path("media.db", "MUSIKA_MEDIA_FILE")
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

pub fn download_path() -> Result<PathBuf> {
    db_path("download.db", "MUSIKA_DOWNLOAD_FILE")
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

/// Directory holding permanent/rule media blobs (CACHE-1): config root, never
/// auto-removed below the permanent/rule tiers.
pub fn media_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_env("MUSIKA_MEDIA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(config_dir()?.join("media"))
}

/// Directory holding playback/rolling media blobs (CACHE-1): XDG cache,
/// purgeable without losing user data.
pub fn playback_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_env("MUSIKA_MEDIA_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(cache_dir()?.join("musika").join("media"))
}

fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_env("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cache"))
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[tokio::test]
    async fn wipe_pool_drops_every_table_and_migrations_rebuild_them() {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO artists (id, name) VALUES ('a', 'A')")
            .execute(&pool)
            .await
            .unwrap();

        wipe_pool(&pool).await.unwrap();
        let tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tables, 0);

        // Re-migrating yields the empty schema again (the wipe path's second
        // half): tables exist, rows are gone.
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .unwrap();
        let artists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artists")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(artists, 0);
    }
}
