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
use std::sync::{OnceLock, RwLock};

/// Active account data root. All databases, caches and user playlists resolve
/// under it; `accounts.json`, the encryption key and the IPC socket stay in the
/// global config dir.
static DATA_ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

pub fn set_data_root(path: PathBuf) {
    let slot = DATA_ROOT.get_or_init(|| RwLock::new(None));
    *slot.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

pub fn data_root() -> Result<PathBuf> {
    if let Some(slot) = DATA_ROOT.get()
        && let Some(path) = slot.read().unwrap_or_else(|e| e.into_inner()).clone()
    {
        return Ok(path);
    }
    config_dir()
}

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
/// ARC-RS-4.9: at most 7 connections (3 library + 2 media + 1 sync + 1
/// search) instead of 10 (5+3+2); `search` stays a separate pool so a slow
/// query never blocks the writer, and it is opened lazily. Extra burst
/// connections are reaped after 60 s idle, so a settled daemon keeps one
/// connection per eager pool.
pub async fn init() -> Result<Dbs> {
    let started = std::time::Instant::now();
    let library = open_migrated(
        library_path()?,
        3,
        sqlx::migrate!("./migrations_library"),
        false,
        Durability::Full,
    )
    .await?;
    crate::startup::trace_elapsed("db.library", started.elapsed());
    let started = std::time::Instant::now();
    let media = open_migrated(
        media_path()?,
        2,
        sqlx::migrate!("./migrations_media"),
        false,
        Durability::Normal,
    )
    .await?;
    crate::startup::trace_elapsed("db.media", started.elapsed());
    let started = std::time::Instant::now();
    let sync = open_sync_pool(library_path()?).await?;
    crate::startup::trace_elapsed("db.sync", started.elapsed());
    let started = std::time::Instant::now();
    absorb_legacy(&library, &media).await?;
    crate::startup::trace_elapsed("db.legacy", started.elapsed());
    // Search gets its own connection over the same library file: a slow or
    // cancelled query can never occupy the writer's pool (the search box
    // stays responsive while a sync rebuilds the FTS table). It is opened
    // lazily: the daemon may never take the FTS fallback, and an unused
    // connection still costs a worker thread and a page cache.
    let search = open_lazy_pool(library_path()?, 1, Durability::Normal)?;

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
        // Best-effort: the page cache is the goal, never a correctness
        // dependency, and this I/O must not compete with audio or the UI.
        crate::linux::demote_current();
        for candidate in warm_startup_paths() {
            _ = warm_path(&candidate);
        }
        crate::startup::trace("db.warm.done");
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
    // `readahead(2)`/`posix_fadvise(WILLNEED)` fill the page cache without a
    // userspace copy. The old loop read the whole database through a 1 MiB
    // buffer, which duplicated the kernel work for no extra warmth.
    crate::linux::prefetch_file(path)
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
        _ = std::fs::remove_file(candidate);
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

pub(crate) async fn import_file(
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
    let import_result: Result<()> = async {
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
                } else if *table == "covers" {
                    "INSERT OR IGNORE INTO main.covers
                         (id, file, hash, content_type, size, etag, last_modified, fetched_at)
                     SELECT id, file, hash, content_type, size, etag, last_modified, fetched_at
                     FROM legacy.covers"
                        .to_string()
                } else {
                    format!("INSERT OR IGNORE INTO main.{table} SELECT * FROM legacy.{table}")
                };
                sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                    .execute(&mut *conn)
                    .await?;
            }
        }
        Ok(())
    }
    .await;
    let detach_result = sqlx::query("DETACH DATABASE legacy")
        .execute(&mut *conn)
        .await;
    import_result?;
    detach_result?;
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

/// Wipes cached lyric documents and lines. Local `.lrc`/`.txt` overrides and
/// the capability flag in `lyrics_meta` are user/server state and stay.
pub async fn wipe_lyrics(dbs: &Dbs) -> Result<()> {
    delete_tables(&dbs.lyrics, &["lyric_line", "lyrics"]).await
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
        _ = std::fs::remove_file(mirror);
    }
}

fn remove_dir(dir: &Path) -> Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to remove {}", dir.display())),
    }
}

/// The full reset: the pools close, **Musika-owned data files and directories**
/// under `data_root()` go (databases, WAL/SHM sidecars, covers, media,
/// playback, playlists, `index.txt`), while global account/config state
/// (`accounts.json`, `accounts/`, `key`, `eqs/`, sockets, and any non-Musika
/// sibling entries) is never touched. Cache-tier media (`media_dir`,
/// `playback_dir`, `covers_dir`) goes too, and the caller re-initialises
/// fresh files and schemas.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the full database wipe is exercised only by the wipe tests"
    )
)]
pub async fn wipe_files(dbs: &Dbs) -> Result<()> {
    crate::search::index::clear();
    // Close first: an open pool keeps the deleted inodes alive and the next
    // login would write into unlinked files.
    for pool in [&dbs.library, &dbs.sync, &dbs.search, &dbs.media] {
        pool.clone().close().await;
    }
    let dir = data_root()?;
    let extra_dirs: Vec<PathBuf> = [covers_dir(), media_dir(), playback_dir()]
        .into_iter()
        .flatten()
        .collect();
    wipe_allowlisted_in(&dir, &extra_dirs)
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the full database wipe is exercised only by the wipe tests"
    )
)]
const WIPE_DB_FILES: &[&str] = &[
    "library.db",
    "media.db",
    "credentials.db",
    "search.db",
    "covers.db",
    "lyrics.db",
    "colors.db",
    "statussave.db",
    "artist_user.db",
    "download.db",
    "playlist.db",
];

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the full database wipe is exercised only by the wipe tests"
    )
)]
const WIPE_PLAIN_FILES: &[&str] = &["index.txt"];
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the full database wipe is exercised only by the wipe tests"
    )
)]
const WIPE_DIRS: &[&str] = &["covers", "media", "playback", "playlists"];

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the full database wipe is exercised only by the wipe tests"
    )
)]
fn remove_file_best_effort(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => eprintln!("wipe: could not remove {}: {err}", path.display()),
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the full database wipe is exercised only by the wipe tests"
    )
)]
fn wipe_allowlisted_in(dir: &Path, extra_dirs: &[PathBuf]) -> Result<()> {
    for db_name in WIPE_DB_FILES {
        for suffix in ["", "-wal", "-shm"] {
            remove_file_best_effort(&dir.join(format!("{db_name}{suffix}")));
        }
    }
    for file_name in WIPE_PLAIN_FILES {
        remove_file_best_effort(&dir.join(file_name));
    }
    for dir_name in WIPE_DIRS {
        remove_dir(&dir.join(dir_name))?;
    }
    // Media is explicitly wiped wherever it lives: env overrides can point
    // covers/media/playback outside `data_root()`.
    for extra in extra_dirs {
        remove_dir(extra)?;
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
    ensure_parent_dir(&path)?;
    // One long-lived demoted writer: it must never be reaped and replaced by a
    // normal-priority worker in the middle of a sync.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect_with(connect_options(path, Durability::Full))
        .await?;
    // Force the worker to exist, then demote only newly created sqlx workers.
    _ = pool.acquire().await?;
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
        if !crate::linux::set_idle_priority(tid) {
            crate::debug_log!("db: could not demote sync worker {tid}");
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
///
/// Eager pools keep one warm connection and reap any extra burst connections
/// after a minute: startup/migrations/page loads briefly need two or three
/// connections, but an idle daemon should not keep their threads and page
/// caches alive for sqlx's default ten minutes.
async fn open_pool(
    path: PathBuf,
    max_connections: u32,
    durability: Durability,
) -> Result<SqlitePool> {
    ensure_parent_dir(&path)?;
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .min_connections(1)
        .idle_timeout(Some(Duration::from_secs(60)))
        .connect_with(connect_options(path, durability))
        .await?;
    Ok(pool)
}

/// A pool that does not open a connection until its first query.
fn open_lazy_pool(
    path: PathBuf,
    max_connections: u32,
    durability: Durability,
) -> Result<SqlitePool> {
    ensure_parent_dir(&path)?;
    Ok(SqlitePoolOptions::new()
        .max_connections(max_connections)
        .min_connections(0)
        .idle_timeout(Some(Duration::from_secs(60)))
        .connect_lazy_with(connect_options(path, durability)))
}

/// The shared SQLite connection options; the pool policy differs per role.
fn connect_options(path: PathBuf, durability: Durability) -> sqlx::sqlite::SqliteConnectOptions {
    sqlx::sqlite::SqliteConnectOptions::new()
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
        .statement_cache_capacity(32)
        .synchronous(durability.synchronous())
        .optimize_on_close(true, 400)
}

/// Asks SQLite to release as much heap memory (lookaside + page cache) as
/// possible back to the allocator after bulk migrations, syncs, or wipes.
pub(crate) async fn shrink_memory(pool: &SqlitePool) {
    if let Err(err) = sqlx::query("PRAGMA shrink_memory").execute(pool).await {
        crate::debug_log!("db: shrink_memory failed: {err:#}");
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

macro_rules! db_path {
    ($($func:ident: $file:literal, $env:literal);* $(;)?) => {
        $(
            pub fn $func() -> Result<PathBuf> {
                dir_from_env($env, || Ok(data_root()?.join($file)))
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

/// Path to the persisted scroll-memory document (GPUI front end only).
#[cfg_attr(
    not(feature = "gpuui"),
    expect(
        dead_code,
        reason = "the scroll-memory document is only read by the GPUI front end"
    )
)]
pub fn scroll_path() -> Result<PathBuf> {
    dir_from_env(
        "MUSIKA_SCROLL_FILE",
        || Ok(data_root()?.join("scroll.json")),
    )
}

/// Directory holding the cached cover-art blobs.
pub fn covers_dir() -> Result<PathBuf> {
    dir_from_env("MUSIKA_COVERS_DIR", || Ok(data_root()?.join("covers")))
}

/// Directory holding permanent/rule media blobs (CACHE-1): config root, never
/// auto-removed below the permanent/rule tiers.
pub fn media_dir() -> Result<PathBuf> {
    dir_from_env("MUSIKA_MEDIA_DIR", || Ok(data_root()?.join("media")))
}

/// Directory holding playback/rolling media blobs (CACHE-1): XDG cache,
/// purgeable without losing user data.
pub fn playback_dir() -> Result<PathBuf> {
    dir_from_env("MUSIKA_MEDIA_CACHE_DIR", || {
        Ok(cache_dir()?.join("playback"))
    })
}

fn cache_dir() -> Result<PathBuf> {
    match non_empty_env("XDG_CACHE_HOME") {
        Some(dir) => Ok(PathBuf::from(dir).join("musika")),
        None => Ok(home_dir()?.join(".cache").join("musika")),
    }
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
    match non_empty_env("XDG_CONFIG_HOME") {
        Some(dir) => Ok(PathBuf::from(dir).join("musika")),
        None => Ok(home_dir()?.join(".config").join("musika")),
    }
}

const LEGACY_XDG_FILES: &[&str] = &["accounts.json", "library.db", "media.db", "key"];

fn legacy_xdg_config_warning(xdg_config_home: &Path) -> Option<String> {
    let target = xdg_config_home.join("musika");
    if target.exists() {
        return None;
    }
    let found: Vec<&'static str> = LEGACY_XDG_FILES
        .iter()
        .copied()
        .filter(|name| xdg_config_home.join(name).exists())
        .collect();
    if found.is_empty() {
        return None;
    }
    Some(format!(
        "musika: found legacy config files directly in XDG_CONFIG_HOME ({}): {}; Musika now uses {} and will not move files automatically",
        xdg_config_home.display(),
        found.join(", "),
        target.display()
    ))
}

/// Warns once on startup if `XDG_CONFIG_HOME` is non-empty,
/// `$XDG_CONFIG_HOME/musika` does not exist, and legacy Musika files exist
/// directly in `$XDG_CONFIG_HOME`. Warn-only: never moves files.
pub fn warn_legacy_xdg_config_once() {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        if let Some(xdg) = non_empty_env("XDG_CONFIG_HOME")
            && let Some(warning) = legacy_xdg_config_warning(Path::new(&xdg))
        {
            eprintln!("{warning}");
        }
    });
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
        _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn lazy_pool_stays_unopened_until_first_acquire() {
        let dir = std::env::temp_dir().join(format!("musika-db-lazy-{}", std::process::id()));
        ensure_dir(&dir).expect("temp dir");
        let path = dir.join("lazy.db");
        let pool = open_lazy_pool(path, 1, Durability::Normal).expect("lazy pool");
        assert_eq!(
            pool.size(),
            0,
            "a lazy pool must not open a connection at construction"
        );
        drop(pool.acquire().await.expect("first acquire"));
        assert_eq!(
            pool.size(),
            1,
            "the first acquire opens exactly one connection"
        );
        pool.close().await;
        _ = std::fs::remove_dir_all(&dir);
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
                library,
                dir.join("library.db-wal"),
                media,
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
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_legacy_db_removes_the_file_and_sidecars() {
        let dir =
            std::env::temp_dir().join(format!("musika-db-legacy-remove-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
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
        _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn absorb_files_imports_then_removes_the_legacy_file() {
        let dir =
            std::env::temp_dir().join(format!("musika-db-legacy-absorb-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
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
        _ = std::fs::remove_dir_all(&dir);
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
            _ = std::fs::remove_file(&path);
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
        _ = std::fs::remove_dir_all(&dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_appends_musika_under_xdg_config_home() {
        if std::env::var_os("MUSIKA_TEST_XDG_SET").is_some() {
            let cfg_home = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
            let cache_home = PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap());
            assert_eq!(config_dir().unwrap(), cfg_home.join("musika"));
            assert_eq!(cache_dir().unwrap(), cache_home.join("musika"));
            assert_eq!(
                playback_dir().unwrap(),
                cache_home.join("musika").join("playback")
            );
            return;
        }
        if std::env::var_os("MUSIKA_TEST_XDG_EMPTY").is_some() {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap());
            assert_eq!(config_dir().unwrap(), home.join(".config").join("musika"));
            assert_eq!(cache_dir().unwrap(), home.join(".cache").join("musika"));
            assert_eq!(
                playback_dir().unwrap(),
                home.join(".cache").join("musika").join("playback")
            );
            return;
        }

        let base = std::env::temp_dir().join(format!("musika-xdg-test-{}", std::process::id()));
        let cfg_home = base.join("cfg");
        let cache_home = base.join("cache");
        let home = base.join("home");

        let exe = std::env::current_exe().unwrap();
        for (mode, xdg_cfg, xdg_cache) in [
            (
                "MUSIKA_TEST_XDG_SET",
                cfg_home.to_string_lossy().into_owned(),
                cache_home.to_string_lossy().into_owned(),
            ),
            ("MUSIKA_TEST_XDG_EMPTY", String::new(), String::new()),
        ] {
            let output = std::process::Command::new(&exe)
                .env("XDG_CONFIG_HOME", &xdg_cfg)
                .env("XDG_CACHE_HOME", &xdg_cache)
                .env_remove("MUSIKA_MEDIA_CACHE_DIR")
                .env("HOME", &home)
                .env(mode, "1")
                .args([
                    "--exact",
                    "db::tests::config_dir_appends_musika_under_xdg_config_home",
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child ({mode}) failed: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn warn_legacy_xdg_config_once_detects_without_moving() {
        if std::env::var_os("MUSIKA_TEST_LEGACY_XDG_CHILD").is_some() {
            warn_legacy_xdg_config_once();
            return;
        }

        let base = std::env::temp_dir().join(format!("musika-xdg-legacy-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&base);
        let xdg = base.join("xdg_cfg");
        ensure_dir(&xdg).unwrap();

        // Empty XDG_CONFIG_HOME: no warning.
        assert!(legacy_xdg_config_warning(&xdg).is_none());

        // Legacy files directly under XDG_CONFIG_HOME and no `musika/` subdir.
        for file in ["accounts.json", "library.db", "media.db", "key"] {
            std::fs::write(xdg.join(file), format!("legacy-{file}")).unwrap();
        }
        let warning = legacy_xdg_config_warning(&xdg).expect("should warn for legacy files");
        assert!(warning.contains("accounts.json"));
        assert!(warning.contains("musika"));

        // Calling warn_legacy_xdg_config_once in a clean subprocess emits the warning
        // on stderr and leaves all legacy files in place without auto-moving them.
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .env("XDG_CONFIG_HOME", &xdg)
            .env("MUSIKA_TEST_LEGACY_XDG_CHILD", "1")
            .args([
                "--exact",
                "db::tests::warn_legacy_xdg_config_once_detects_without_moving",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("found legacy config files directly in XDG_CONFIG_HOME"),
            "unexpected stderr: {stderr}"
        );
        assert!(
            !xdg.join("musika").exists(),
            "warn-only must not create or move into musika/"
        );
        for file in ["accounts.json", "library.db", "media.db", "key"] {
            assert_eq!(
                std::fs::read_to_string(xdg.join(file)).unwrap(),
                format!("legacy-{file}")
            );
        }

        // Once `musika/` exists, legacy_xdg_config_warning stays quiet.
        ensure_dir(&xdg.join("musika")).unwrap();
        assert!(legacy_xdg_config_warning(&xdg).is_none());

        _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn wipe_files_preserves_non_musika_siblings_and_eqs() {
        if std::env::var_os("MUSIKA_TEST_WIPE_CHILD").is_some() {
            let musika_root = config_dir().unwrap();
            let runtime = crate::app::runtime();
            runtime.block_on(async {
                let dbs = init().await.unwrap();

                // Musika-owned wipe targets (legacy DBs + sidecars, index.txt, media/cache dirs).
                for legacy in [
                    "credentials.db",
                    "credentials.db-wal",
                    "credentials.db-shm",
                    "search.db",
                    "index.txt",
                ] {
                    std::fs::write(musika_root.join(legacy), b"wipe-me").unwrap();
                }
                for dir in ["covers", "media", "playback", "playlists"] {
                    ensure_dir(&musika_root.join(dir)).unwrap();
                    std::fs::write(musika_root.join(dir).join("item"), b"wipe-me").unwrap();
                }

                // Protected global files, EQ profiles, sockets, and unknown siblings.
                std::fs::write(musika_root.join("accounts.json"), b"{\"active\":\"a1\"}").unwrap();
                ensure_dir(&musika_root.join("accounts").join("a1")).unwrap();
                std::fs::write(
                    musika_root.join("accounts").join("a1").join("library.db"),
                    b"other-account",
                )
                .unwrap();
                std::fs::write(musika_root.join("key"), b"secret-key").unwrap();
                ensure_dir(&musika_root.join("eqs")).unwrap();
                std::fs::write(musika_root.join("eqs").join("flat.eq"), b"Preamp: 0\n").unwrap();
                std::fs::write(musika_root.join("ipc.sock"), b"sock").unwrap();
                std::fs::write(musika_root.join("daemon.sock"), b"sock").unwrap();
                std::fs::write(musika_root.join("custom_notes.txt"), b"keep").unwrap();
                std::fs::write(musika_root.join("unrelated.db"), b"keep-db").unwrap();
                ensure_dir(&musika_root.join("sibling_dir")).unwrap();
                std::fs::write(musika_root.join("sibling_dir").join("data"), b"keep").unwrap();

                wipe_files(&dbs).await.unwrap();

                assert!(dbs.library.is_closed(), "library pool must be closed");
                assert!(dbs.sync.is_closed(), "sync pool must be closed");
                assert!(dbs.search.is_closed(), "search pool must be closed");
                assert!(dbs.media.is_closed(), "media pool must be closed");

                for removed in [
                    "library.db",
                    "library.db-wal",
                    "library.db-shm",
                    "media.db",
                    "media.db-wal",
                    "media.db-shm",
                    "credentials.db",
                    "credentials.db-wal",
                    "credentials.db-shm",
                    "search.db",
                    "index.txt",
                    "covers",
                    "media",
                    "playback",
                    "playlists",
                ] {
                    assert!(
                        !musika_root.join(removed).exists(),
                        "Musika target {removed} should have been wiped"
                    );
                }

                assert_eq!(
                    std::fs::read(musika_root.join("accounts.json")).unwrap(),
                    b"{\"active\":\"a1\"}"
                );
                assert_eq!(
                    std::fs::read(musika_root.join("accounts").join("a1").join("library.db"))
                        .unwrap(),
                    b"other-account"
                );
                assert_eq!(
                    std::fs::read(musika_root.join("key")).unwrap(),
                    b"secret-key"
                );
                assert_eq!(
                    std::fs::read(musika_root.join("eqs").join("flat.eq")).unwrap(),
                    b"Preamp: 0\n"
                );
                assert!(musika_root.join("ipc.sock").exists());
                assert!(musika_root.join("daemon.sock").exists());
                assert_eq!(
                    std::fs::read(musika_root.join("custom_notes.txt")).unwrap(),
                    b"keep"
                );
                assert_eq!(
                    std::fs::read(musika_root.join("unrelated.db")).unwrap(),
                    b"keep-db"
                );
                assert_eq!(
                    std::fs::read(musika_root.join("sibling_dir").join("data")).unwrap(),
                    b"keep"
                );
            });
            return;
        }

        let base = std::env::temp_dir().join(format!("musika-wipe-test-{}", std::process::id()));
        let xdg_cfg = base.join("cfg");
        let xdg_cache = base.join("cache");
        _ = std::fs::remove_dir_all(&base);
        ensure_dir(&xdg_cfg).unwrap();
        ensure_dir(&xdg_cache).unwrap();
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .env("XDG_CONFIG_HOME", &xdg_cfg)
            .env("XDG_CACHE_HOME", &xdg_cache)
            .env_remove("MUSIKA_LIBRARY_FILE")
            .env_remove("MUSIKA_MEDIA_FILE")
            .env_remove("MUSIKA_MEDIA_DIR")
            .env_remove("MUSIKA_MEDIA_CACHE_DIR")
            .env_remove("MUSIKA_COVERS_DIR")
            .env("MUSIKA_TEST_WIPE_CHILD", "1")
            .args([
                "--exact",
                "db::tests::wipe_files_preserves_non_musika_siblings_and_eqs",
            ])
            .output()
            .unwrap();
        _ = std::fs::remove_dir_all(&base);
        assert!(
            output.status.success(),
            "wipe child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
