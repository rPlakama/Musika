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
use sqlx::sqlite::{SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous};

/// The two physical databases, with the logical pools kept as aliases.
#[derive(Clone)]
pub struct Dbs {
    pub credentials: SqlitePool,
    pub library: SqlitePool,
    /// Single-connection writer used by the background sync; its sqlx worker
    /// is demoted so a bulk rebuild cannot starve the GUI.
    pub sync: SqlitePool,
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

/// Per-database durability: the physical library file also holds credentials and
/// user-authored playlists/status, so it stays FULL; media is a cache plus a
/// rebuildable download ledger and can use WAL NORMAL.
#[derive(Clone, Copy)]
enum Durability {
    Full,
    Normal,
}

impl Durability {
    fn synchronous(self) -> SqliteSynchronous {
        match self {
            Self::Full => SqliteSynchronous::Full,
            Self::Normal => SqliteSynchronous::Normal,
        }
    }
}

/// Opens every database and applies its migrations.
///
/// ARC-RS-4.9: 6 connections total (3 library + 2 media + 1 search) instead
/// of 10 (5+3+2); `search` keeps its own pool so a slow query never blocks
/// the writer, but `sync` merges into `library`.
pub async fn init() -> Result<Dbs> {
    let library = open_migrated(
        library_path()?,
        3,
        sqlx::migrate!("./migrations_library"),
        false,
        Durability::Full,
    )
    .await?;
    let media = open_migrated(
        media_path()?,
        2,
        sqlx::migrate!("./migrations_media"),
        false,
        Durability::Normal,
    )
    .await?;
    let sync = open_sync_pool(library_path()?).await?;
    absorb_legacy(&library, &media).await?;
    // Search gets its own connection over the same library file: a slow or
    // cancelled query can never occupy the writer's pool (the search box
    // stays responsive while a sync rebuilds the FTS table).
    let search = open_pool(library_path()?, 1, Durability::Normal).await?;

    Ok(Dbs {
        credentials: library.clone(),
        library: library.clone(),
        sync,
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

/// Warms the live catalog files on a plain OS thread; startup and login never
/// wait for it. `posix_fadvise` would avoid the read, but libc is not a direct
/// dependency, so this uses a buffered pass (the page cache is the goal).
///
/// Only the two physical databases are live now: `search`, `covers`, `lyrics`
/// and `download` are aliases of these pools, while their old files are read
/// once by [`absorb_legacy`] and then removed.
pub fn warm_startup() {
    std::thread::spawn(|| {
        for candidate in warm_startup_paths() {
            let _ = warm_path(&candidate);
        }
    });
}

fn warm_startup_paths() -> Vec<PathBuf> {
    warm_targets([library_path(), media_path()].into_iter().flatten())
}

fn warm_targets(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    for path in paths {
        targets.push(path.clone());
        targets.push(PathBuf::from(format!("{}-wal", path.display())));
    }
    targets
}

fn warm_path(path: &Path) -> std::io::Result<()> {
    match warm_file(path) {
        // A `-wal` only exists while writes are pending; its absence is the
        // normal case, not a warm failure.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            crate::debug_log!("db: warm {} failed: {err}", path.display());
            Err(err)
        }
        Ok(()) => Ok(()),
    }
}

fn warm_file(path: &Path) -> std::io::Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 1024 * 1024];
    while file.read(&mut buffer)? > 0 {}
    Ok(())
}

/// Bounds the WAL after a successful bulk write. Only call this from a
/// completed sync, never from a cancelled or failed one.
pub async fn checkpoint_truncate(pool: &SqlitePool) -> Result<()> {
    sqlx::raw_sql("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await?;
    Ok(())
}

/// One-time import of the old per-domain databases (backup, not required).
///
/// Each physical database keeps its own flag: a crash between the two halves
/// must not make the library flag skip the media import forever.
async fn absorb_legacy(library: &SqlitePool, media: &SqlitePool) -> Result<()> {
    absorb_library_legacy(library).await?;
    absorb_media_legacy(media).await?;
    Ok(())
}

async fn absorb_library_legacy(library: &SqlitePool) -> Result<()> {
    if meta_flag(library).await? {
        return Ok(());
    }
    let legacy = [
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
    absorb_files(library, &legacy, "meta").await
}

async fn absorb_media_legacy(media: &SqlitePool) -> Result<()> {
    if media_meta_flag(media).await? {
        return Ok(());
    }
    let legacy = [
        (covers_path()?, &["covers"][..]),
        (lyrics_path()?, &["lyrics", "lyric_line", "lyrics_meta"][..]),
        (download_path()?, &["download"][..]),
    ];
    absorb_files(media, &legacy, "media_meta").await
}

async fn absorb_files(
    pool: &SqlitePool,
    legacy: &[(PathBuf, &[&'static str])],
    flag_table: &str,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    for (path, tables) in legacy {
        import_file(&mut conn, path, tables).await?;
    }
    let sql = format!(
        "INSERT INTO {flag_table} (key, value) VALUES ('legacy_absorbed', '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&mut *conn)
        .await?;
    drop(conn);
    for (path, _) in legacy {
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

async fn meta_flag(pool: &SqlitePool) -> Result<bool> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'legacy_absorbed'")
            .fetch_optional(pool)
            .await?;
    Ok(value.is_some())
}

async fn media_meta_flag(pool: &SqlitePool) -> Result<bool> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM media_meta WHERE key = 'legacy_absorbed'")
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
    for dir in [media_dir(), playback_dir()].into_iter().flatten() {
        remove_dir(&dir)?;
    }
    Ok(())
}

/// Drops every user table in one database (virtual/FTS tables first, so their
/// shadow tables fall with them; FK checks are off while DROP implicitly
/// deletes rows).
#[cfg(test)]
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

async fn open_sync_pool(path: PathBuf) -> Result<SqlitePool> {
    let before = sqlite_workers();
    let pool = open_pool(path, 1, Durability::Full).await?;
    // Force the worker to exist, then demote only newly created sqlx workers.
    let _ = pool.acquire().await?;
    demote_new_sqlite_workers(&before);
    Ok(pool)
}

#[cfg(target_os = "linux")]
fn sqlite_workers() -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return out;
    };
    for entry in entries.flatten() {
        if let Ok(tid) = entry.file_name().to_string_lossy().parse::<u32>() {
            out.insert(tid);
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn sqlite_workers() -> std::collections::HashSet<u32> {
    std::collections::HashSet::new()
}

#[cfg(target_os = "linux")]
fn demote_new_sqlite_workers(before: &std::collections::HashSet<u32>) {
    #[repr(C)]
    struct SchedParam {
        priority: std::ffi::c_int,
    }
    unsafe extern "C" {
        fn sched_setscheduler(
            pid: std::ffi::c_int,
            policy: std::ffi::c_int,
            param: *const SchedParam,
        ) -> std::ffi::c_int;
        fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
    }
    const SCHED_IDLE: std::ffi::c_int = 5;
    const IOPRIO_WHO_PROCESS: std::ffi::c_int = 1;
    const IOPRIO_CLASS_IDLE: std::ffi::c_int = 3;
    for entry in std::fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(tid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if before.contains(&tid) {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        if !comm.trim().starts_with("sqlx-sqlite-wor") {
            continue;
        }
        let param = SchedParam { priority: 0 };
        let sched = unsafe { sched_setscheduler(tid as std::ffi::c_int, SCHED_IDLE, &param) };
        let io = unsafe {
            syscall(
                251,
                IOPRIO_WHO_PROCESS,
                tid as std::ffi::c_int,
                IOPRIO_CLASS_IDLE << 13,
            ) as std::ffi::c_int
        };
        if sched != 0 || io != 0 {
            crate::debug_log!("db: could not demote sync worker {tid}: sched={sched}, io={io}");
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn demote_new_sqlite_workers(_before: &std::collections::HashSet<u32>) {}

async fn open_migrated(
    path: PathBuf,
    max_connections: u32,
    mut migrations: sqlx::migrate::Migrator,
    ignore_missing: bool,
    durability: Durability,
) -> Result<SqlitePool> {
    let pool = open_pool(path, max_connections, durability).await?;
    if ignore_missing {
        migrations.set_ignore_missing(true);
    }
    migrations.run(&pool).await?;
    Ok(pool)
}

/// Opens a pool with the shared connection options (no migrations).
async fn open_pool(
    path: PathBuf,
    max_connections: u32,
    durability: Durability,
) -> Result<SqlitePool> {
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
        // ARC-RS-4.9: cache_size 8 MiB -> 2 MiB. With 3 pools (3 library +
        // 2 media + 1 search = 6 connections, was 10) the worst RSS ceiling
        // drops 80 MiB -> ~12 MiB. The mmap window still shares hot pages.
        .pragma("cache_size", "-2048")
        .pragma("mmap_size", "268435456")
        .pragma("temp_store", "MEMORY")
        .pragma("journal_size_limit", "16777216")
        .synchronous(durability.synchronous())
        .optimize_on_close(true, 400);

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

#[cfg(test)]
mod options_tests {
    use super::*;

    #[test]
    fn durability_maps_to_synchronous_pragma() {
        let dir = std::env::temp_dir().join(format!("musika-db-sync-{}", std::process::id()));
        ensure_dir(&dir).expect("temp dir");
        let runtime = crate::app::runtime();
        for (durability, expected) in [(Durability::Full, 2), (Durability::Normal, 1)] {
            let path = dir.join(format!("{expected}.db"));
            let pool = runtime
                .block_on(open_pool(path, 1, durability))
                .expect("pool");
            let value: i64 = runtime
                .block_on(sqlx::query_scalar("PRAGMA synchronous").fetch_one(&pool))
                .expect("pragma");
            assert_eq!(value, expected);
            runtime.block_on(pool.close());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warm_file_tolerates_missing_path() {
        let started = std::time::Instant::now();
        assert!(warm_file(Path::new("/nonexistent/musika-warm.db")).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn warm_targets_cover_each_database_and_its_wal() {
        let dir = std::env::temp_dir().join(format!("musika-db-warm-{}", std::process::id()));
        let library = dir.join("library.db");
        let media = dir.join("media.db");
        assert_eq!(
            warm_targets([library.clone(), media.clone()]),
            vec![
                library.clone(),
                dir.join("library.db-wal"),
                media.clone(),
                dir.join("media.db-wal"),
            ]
        );
        let expected = warm_targets([
            library_path().expect("library path"),
            media_path().expect("media path"),
        ]);
        assert_eq!(warm_startup_paths(), expected);
        let startup = warm_startup_paths();
        assert!(!startup.iter().any(|path| {
            matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("search.db" | "covers.db" | "search.db-wal" | "covers.db-wal")
            )
        }));
    }

    #[test]
    fn warm_path_treats_not_found_as_success() {
        assert!(warm_path(Path::new("/nonexistent/musika-warm-wal")).is_ok());
        let dir = std::env::temp_dir().join(format!("musika-db-warm-dir-{}", std::process::id()));
        ensure_dir(&dir).expect("temp dir");
        assert!(warm_path(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_legacy_db_removes_the_file_and_sidecars() {
        let dir =
            std::env::temp_dir().join(format!("musika-db-legacy-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ensure_dir(&dir).expect("temp dir");
        let path = dir.join("covers.db");
        for suffix in ["", "-wal", "-shm"] {
            std::fs::write(
                PathBuf::from(format!("{}{suffix}", path.display())),
                b"legacy",
            )
            .expect("legacy file");
        }
        remove_legacy_db(&path);
        for suffix in ["", "-wal", "-shm"] {
            let file = PathBuf::from(format!("{}{suffix}", path.display()));
            assert!(!file.exists(), "{} was left behind", file.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn absorb_files_imports_then_removes_the_legacy_file() {
        let dir =
            std::env::temp_dir().join(format!("musika-db-legacy-absorb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ensure_dir(&dir).expect("temp dir");
        let library = open_migrated(
            dir.join("library.db"),
            1,
            sqlx::migrate!("./migrations_library"),
            false,
            Durability::Full,
        )
        .await
        .expect("library");
        let legacy = dir.join("credentials.db");
        let legacy_pool = open_pool(legacy.clone(), 1, Durability::Full)
            .await
            .expect("legacy");
        sqlx::raw_sql(
            "CREATE TABLE users (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 username TEXT NOT NULL,
                 password TEXT NOT NULL,
                 serverURL TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 server_type TEXT NOT NULL
             );
             INSERT INTO users (username, password, serverURL, created_at, server_type)
                 VALUES ('me', 'secret', 'https://music.example', 'now', 'navidrome');",
        )
        .execute(&legacy_pool)
        .await
        .expect("legacy rows");
        legacy_pool.close().await;

        absorb_files(&library, &[(legacy.clone(), &["users"][..])], "meta")
            .await
            .expect("absorb");

        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&library)
            .await
            .expect("count");
        assert_eq!(users, 1);
        assert!(meta_flag(&library).await.expect("flag"));
        assert!(!legacy.exists(), "the absorbed legacy file must be removed");
        library.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod sync_bench {
    use super::*;

    /// On-disk write/FTS bench. Run explicitly:
    /// `cargo test db::sync_bench -- --ignored --nocapture`
    #[test]
    #[ignore = "sqlite write bench, run explicitly"]
    fn bench_sync_and_fts_by_synchronous_mode() {
        let runtime = crate::app::runtime();
        let dir = std::env::temp_dir().join(format!("musika-db-bench-{}", std::process::id()));
        ensure_dir(&dir).expect("temp dir");
        for durability in [Durability::Full, Durability::Normal] {
            let path = dir.join(match durability {
                Durability::Full => "full.db",
                Durability::Normal => "normal.db",
            });
            let _ = std::fs::remove_file(&path);
            let started = std::time::Instant::now();
            runtime.block_on(async {
                let pool = open_migrated(
                    path.clone(),
                    1,
                    sqlx::migrate!("./migrations_library"),
                    false,
                    durability,
                )
                .await
                .expect("pool");
                let mut tx = pool.begin().await.expect("tx");
                for index in 0..50_000 {
                    sqlx::query("INSERT INTO songs (id, title) VALUES (?1, ?2)")
                        .bind(format!("s{index}"))
                        .bind(format!("Song {index}"))
                        .execute(&mut *tx)
                        .await
                        .expect("song");
                }
                tx.commit().await.expect("commit");
                let write = started.elapsed();
                let fts = std::time::Instant::now();
                sqlx::query(
                    "INSERT INTO song_search (title, artist, album, duration, song_id, play_count)
                     SELECT title, '', '', 0, id, 0 FROM songs",
                )
                .execute(&pool)
                .await
                .expect("fts");
                println!(
                    "[bench] sync {:?}: write {:?}, fts {:?}",
                    durability.as_str(),
                    write,
                    fts.elapsed()
                );
                pool.close().await;
            });
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
#[cfg(test)]
impl Durability {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "FULL",
            Self::Normal => "NORMAL",
        }
    }
}
