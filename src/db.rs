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
//! Two SQLite databases live under `~/.config/musika`, each with its own
//! connection pool:
//!
//! - `library.db` — account, metadata and library-side state: credentials,
//!   artists / albums / songs, FTS search, artist user state, session status,
//!   cover colors and playlists.
//! - `media.db`   — blob-cache indexes and media state: covers, lyrics,
//!   offline media and the download ledger.
//!
//! `Dbs` keeps the old logical names as aliases of those two pools, so the
//! modules stay independent without opening a pool per table.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteJournalMode, SqlitePool, SqlitePoolOptions};

/// The two physical databases, with the logical pools kept as aliases.
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
    let library = open_migrated(
        library_path()?,
        5,
        sqlx::migrate!("./migrations_library"),
        false,
    )
    .await?;
    let media = open_migrated(
        media_path()?,
        3,
        sqlx::migrate!("./migrations_media"),
        false,
    )
    .await?;
    absorb_legacy(&library, &media).await?;
    // Search gets its own connections over the same library file: a slow or
    // cancelled query can never occupy the writer's pool (the search box
    // stays responsive while a sync rebuilds the FTS table).
    let search = open_pool(library_path()?, 2).await?;

    Ok(Dbs {
        credentials: library.clone(),
        library: library.clone(),
        search,
        covers: media.clone(),
        media: media.clone(),
        lyrics: media.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        download: media.clone(),
        playlist: library,
    })
}

/// Applies both embedded migrations (shared by [`init`] and [`wipe`]).
#[allow(dead_code)]
pub async fn migrate(dbs: &Dbs) -> Result<()> {
    sqlx::migrate!("./migrations_library")
        .run(&dbs.library)
        .await?;
    sqlx::migrate!("./migrations_media").run(&dbs.media).await?;
    Ok(())
}

/// One-time import of the old per-domain databases (backup, not required).
async fn absorb_legacy(library: &SqlitePool, media: &SqlitePool) -> Result<()> {
    if meta_flag(library, "legacy_absorbed").await? {
        return Ok(());
    }

    let mut conn = library.acquire().await?;
    let library_legacy = [
        (credentials_path()?, &["users"][..]),
        (search_path()?, &["song_search"][..]),
        (artist_user_path()?, &["artist_user"][..]),
        (status_path()?, &["session_status"][..]),
        (
            colors_path()?,
            &["cover_color", "colors_meta", "color_seed"][..],
        ),
        (
            playlist_path()?,
            &["playlist", "playlist_song", "playlist_meta"][..],
        ),
    ];
    for (path, tables) in &library_legacy {
        import_file(&mut conn, path, tables).await?;
    }
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES ('legacy_absorbed', '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&mut *conn)
    .await?;
    drop(conn);
    for (path, _) in &library_legacy {
        remove_legacy_db(path);
    }

    let mut conn = media.acquire().await?;
    let media_legacy = [
        (covers_path()?, &["covers"][..]),
        (lyrics_path()?, &["lyrics", "lyric_line", "lyrics_meta"][..]),
        (download_path()?, &["download"][..]),
    ];
    for (path, tables) in &media_legacy {
        import_file(&mut conn, path, tables).await?;
    }
    sqlx::query(
        "INSERT INTO media_meta (key, value) VALUES ('legacy_absorbed', '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .execute(&mut *conn)
    .await?;
    drop(conn);
    for (path, _) in &media_legacy {
        remove_legacy_db(path);
    }
    Ok(())
}

fn remove_legacy_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = PathBuf::from(format!("{}{suffix}", path.display()));
        let _ = std::fs::remove_file(candidate);
    }
}

async fn meta_flag(pool: &SqlitePool, key: &str) -> Result<bool> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value.is_some())
}

async fn import_file(
    conn: &mut sqlx::SqliteConnection,
    path: &Path,
    tables: &[&'static str],
) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    sqlx::query("ATTACH DATABASE ?1 AS legacy")
        .bind(path.to_string_lossy().to_string())
        .execute(&mut *conn)
        .await?;
    for table in tables {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT name FROM legacy.sqlite_master WHERE type IN ('table', 'view') AND name = ?1",
        )
        .bind(table)
        .fetch_optional(&mut *conn)
        .await?;
        if exists.is_some() {
            let sql = if *table == "song_search" {
                "INSERT OR IGNORE INTO main.song_search
                     (rowid, title, artist, album, duration, song_id, play_count)
                 SELECT rowid, title, artist, album, duration, song_id, play_count
                 FROM legacy.song_search"
                    .to_string()
            } else {
                format!("INSERT OR IGNORE INTO main.{table} SELECT * FROM legacy.{table}")
            };
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(&mut *conn)
                .await?;
        }
    }
    sqlx::query("DETACH DATABASE legacy")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn delete_tables(pool: &SqlitePool, tables: &[&'static str]) -> Result<()> {
    for table in tables {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Wipes only the library metadata: artists, albums, songs, artist/album
/// descriptions and the sync bookkeeping. The next sync repopulates it; the
/// derived cover, lyric and color caches and the `index.txt` mirror go with
/// it.
pub async fn wipe_library(dbs: &Dbs) -> Result<()> {
    delete_tables(
        &dbs.library,
        &[
            "songs",
            "albums",
            "artists",
            "artist_info",
            "album_info",
            "meta",
        ],
    )
    .await?;
    delete_tables(&dbs.lyrics, &["lyric_line", "lyrics"]).await?;
    wipe_covers(dbs).await?;
    crate::search::index::clear();
    remove_mirror();
    Ok(())
}

/// Wipes the derived full-text index plus the `index.txt` mirror. Both
/// rebuild from the library on the next sync.
pub async fn wipe_indexes(dbs: &Dbs) -> Result<()> {
    delete_tables(&dbs.search, &["song_search"]).await?;
    crate::search::index::clear();
    remove_mirror();
    Ok(())
}

/// Wipes the cover-art cache: its index, the blob directory and the
/// cover-derived color cache, all refetched on demand.
pub async fn wipe_covers(dbs: &Dbs) -> Result<()> {
    delete_tables(&dbs.covers, &["covers"]).await?;
    delete_tables(&dbs.library, &["cover_color", "color_seed", "colors_meta"]).await?;
    // The next sync must refill the cache even when the library is unchanged
    // (otherwise the 24 h revalidation gate would keep it empty).
    sqlx::query("DELETE FROM meta WHERE key = 'covers_checked_at'")
        .execute(&dbs.library)
        .await?;
    remove_dir(&covers_dir()?)?;
    Ok(())
}

async fn wipe_and_migrate(pool: &SqlitePool, migrations: sqlx::migrate::Migrator) -> Result<()> {
    wipe_pool(pool).await?;
    migrations.run(pool).await?;
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
    crate::search::index::clear();
    // Close first: an open pool keeps the deleted inodes alive and the next
    // login would write into unlinked files.
    for pool in [&dbs.library, &dbs.media] {
        pool.clone().close().await;
    }
    let dir = config_dir()?;
    let socket = crate::ipc::socket_path();
    let entries =
        std::fs::read_dir(&dir).with_context(|| format!("failed to list {}", dir.display()))?;
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
    wipe_and_migrate(&dbs.library, sqlx::migrate!("./migrations_library")).await?;
    wipe_and_migrate(&dbs.media, sqlx::migrate!("./migrations_media")).await?;
    crate::search::index::clear();

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
pub(crate) async fn wipe_pool(pool: &SqlitePool) -> Result<()> {
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

pub fn config_socket(name: &str) -> PathBuf {
    config_dir()
        .map(|dir| dir.join(name))
        .unwrap_or_else(|_| PathBuf::from(format!("/tmp/musika-{name}")))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    Ok(())
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    Ok(())
}

async fn open_migrated(
    path: PathBuf,
    max_connections: u32,
    mut migrations: sqlx::migrate::Migrator,
    ignore_missing: bool,
) -> Result<SqlitePool> {
    let pool = open_pool(path, max_connections).await?;
    if ignore_missing {
        migrations.set_ignore_missing(true);
    }
    migrations.run(&pool).await?;
    Ok(pool)
}

/// Opens a pool with the shared connection options (no migrations).
async fn open_pool(path: PathBuf, max_connections: u32) -> Result<SqlitePool> {
    ensure_parent_dir(&path)?;

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
        // PATH: the catalogs are small enough to stay resident; a warmed
        // page cache and memory-backed sorts keep search/gather reads off
        // the disk after the first touch. `mmap_size` is virtual address
        // space, not committed RAM.
        //
        // MEMORY: `cache_size` is per *connection*, and `init` opens ten of
        // them (5 library + 3 media + 2 search). The old -32768 was therefore
        // a ~320 MiB ceiling for a library that fits in a fraction of that;
        // -8192 keeps a comfortable per-connection cache while capping the
        // guaranteed worst case near 80 MiB. The file-backed mmap window
        // still shares hot pages between connections.
        .pragma("cache_size", "-8192")
        .pragma("mmap_size", "268435456")
        .pragma("temp_store", "MEMORY");

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;

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
    scroll_path: "scroll.json", "MUSIKA_SCROLL_FILE";
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
