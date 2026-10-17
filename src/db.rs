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
    pub playlist: SqlitePool,
}

/// Opens every database and applies its migrations.
pub async fn init() -> Result<Dbs> {
    Ok(Dbs {
        credentials: open_migrated(credentials_path()?, 5, sqlx::migrate!("./migrations"), true)
            .await?,
        library: open_migrated(
            library_path()?,
            5,
            sqlx::migrate!("./migrations_library"),
            false,
        )
        .await?,
        search: open_migrated(
            search_path()?,
            3,
            sqlx::migrate!("./migrations_search"),
            false,
        )
        .await?,
        covers: open_migrated(
            covers_path()?,
            5,
            sqlx::migrate!("./migrations_covers"),
            false,
        )
        .await?,
        media: open_migrated(
            media_path()?,
            3,
            sqlx::migrate!("./migrations_media"),
            false,
        )
        .await?,
        lyrics: open_migrated(
            lyrics_path()?,
            3,
            sqlx::migrate!("./migrations_lyrics"),
            false,
        )
        .await?,
        colors: open_migrated(
            colors_path()?,
            3,
            sqlx::migrate!("./migrations_colors"),
            false,
        )
        .await?,
        // Single row, tiny: one connection is plenty.
        status: open_migrated(
            status_path()?,
            1,
            sqlx::migrate!("./migrations_status"),
            false,
        )
        .await?,
        artist_user: open_migrated(
            artist_user_path()?,
            3,
            sqlx::migrate!("./migrations_artist_user"),
            false,
        )
        .await?,
        download: open_migrated(
            download_path()?,
            3,
            sqlx::migrate!("./migrations_download"),
            false,
        )
        .await?,
        playlist: open_migrated(
            playlist_path()?,
            3,
            sqlx::migrate!("./migrations_playlist"),
            false,
        )
        .await?,
    })
}

/// Applies every pool's embedded migrations (shared by [`init`] and [`wipe`]).
#[allow(dead_code)]
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
    sqlx::migrate!("./migrations_playlist")
        .run(&dbs.playlist)
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

/// The full reset: the pools close, **every entry under the config
/// directory** goes (databases, WAL/SHM sidecars, covers, media, the
/// `index.txt` mirror — the running IPC socket stays), the cache-tier media
/// (`playback_dir`, outside the config tree) goes with it, and the caller
/// re-initialises the databases with fresh files and schemas. An old or
/// foreign schema cannot survive this.
pub async fn wipe_files(dbs: &Dbs) -> Result<()> {
    // Close first: an open pool keeps the deleted inodes alive and the next
    // login would write into unlinked files.
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
        &dbs.playlist,
    ] {
        pool.clone().close().await;
    }
    let dir = config_dir()?;
    let socket = crate::ipc::socket_path();
    let entries = std::fs::read_dir(&dir)
        .with_context(|| format!("failed to list {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path == socket {
            continue;
        }
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(err) = result {
            eprintln!("wipe: could not remove {}: {err}", path.display());
        }
    }
    // Media is explicitly wiped wherever it lives: the config-tier dir is a
    // config-tree entry above (env overrides can point it elsewhere), and
    // the playback/rolling tier sits under the XDG cache, outside the config
    // dir — "wipe everything" leaves no media behind anywhere.
    for dir in [media_dir(), playback_dir()] {
        if let Ok(dir) = dir {
            remove_dir(&dir)?;
        }
    }
    Ok(())
}

/// Destructive recovery: every database is stripped to an empty schema and
/// re-migrated, and the derived blobs (covers/media) and the `index.txt`
/// mirror are removed. Credentials go too, so the caller must return to the
/// login screen afterwards.
#[allow(dead_code)]
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

async fn open_migrated(
    path: PathBuf,
    max_connections: u32,
    mut migrations: sqlx::migrate::Migrator,
    ignore_missing: bool,
) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;

    if ignore_missing {
        migrations.set_ignore_missing(true);
    }
    migrations.run(&pool).await?;

    Ok(pool)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

macro_rules! db_path {
    ($($func:ident: $file:literal, $env:literal);* $(;)?) => {
        $(
            pub fn $func() -> Result<PathBuf> {
                dir_from_env($env, || Ok(config_dir()?.join($file)))
            }
        )*
    };
}

db_path! {
    credentials_path: "credentials.db", "MUSIKA_CREDENTIALS_FILE";
    library_path: "library.db", "MUSIKA_LIBRARY_FILE";
    search_path: "search.db", "MUSIKA_SEARCH_FILE";
    covers_path: "covers.db", "MUSIKA_COVERS_FILE";
    media_path: "media.db", "MUSIKA_MEDIA_FILE";
    lyrics_path: "lyrics.db", "MUSIKA_LYRICS_FILE";
    colors_path: "colors.db", "MUSIKA_COLORS_FILE";
    status_path: "statussave.db", "MUSIKA_STATUS_FILE";
    artist_user_path: "artist_user.db", "MUSIKA_ARTIST_USER_FILE";
    download_path: "download.db", "MUSIKA_DOWNLOAD_FILE";
    playlist_path: "playlist.db", "MUSIKA_PLAYLIST_FILE";
    mirror_path: "index.txt", "MUSIKA_INDEX_MIRROR";
}

/// Directory holding the cached cover-art blobs.
pub fn covers_dir() -> Result<PathBuf> {
    dir_from_env("MUSIKA_COVERS_DIR", || Ok(config_dir()?.join("covers")))
}

/// Directory holding permanent/rule media blobs (CACHE-1): config root, never
/// auto-removed below the permanent/rule tiers.
pub fn media_dir() -> Result<PathBuf> {
    dir_from_env("MUSIKA_MEDIA_DIR", || Ok(config_dir()?.join("media")))
}

/// Directory holding playback/rolling media blobs (CACHE-1): XDG cache,
/// purgeable without losing user data.
pub fn playback_dir() -> Result<PathBuf> {
    dir_from_env("MUSIKA_MEDIA_CACHE_DIR", || {
        Ok(cache_dir()?.join("musika").join("media"))
    })
}

fn cache_dir() -> Result<PathBuf> {
    dir_from_env("XDG_CACHE_HOME", || Ok(home_dir()?.join(".cache")))
}

fn dir_from_env(env: &str, fallback: impl FnOnce() -> Result<PathBuf>) -> Result<PathBuf> {
    match non_empty_env(env) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => fallback(),
    }
}

fn home_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var("HOME").context("HOME is not set")?,
    ))
}

pub fn config_dir() -> Result<PathBuf> {
    dir_from_env("XDG_CONFIG_HOME", || {
        Ok(home_dir()?.join(".config").join("musika"))
    })
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
