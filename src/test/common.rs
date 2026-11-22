pub use crate::api::{AlbumID3, ArtistID3, Child, LyricsList};
pub use crate::consts::now_secs;
pub use crate::db::Dbs;
pub(crate) use crate::digest::md5_hex;
pub use crate::library::{AlbumRow, ArtistRow, SongSearchRow};
pub use material_colors::color::Rgb;
pub use material_colors::dynamic_color::{DynamicScheme, Platform, SpecVersion, Variant};
pub use material_colors::hct::Hct;
pub use openssl::hash::MessageDigest;
pub use serde_json::{Value, json};
pub use sqlx::SqlitePool;
pub use std::collections::{HashMap, HashSet};
pub use std::os::unix::net::UnixStream;
pub use std::path::{Path, PathBuf};
pub use std::process::Command;
pub use std::sync::Arc;
pub use std::time::{Duration, Instant};
pub use tokio::sync::mpsc;

/// Serializes tests that use the process-wide offline downloaded-catalog
/// cache (`crate::offline::load`). The cache is intentionally one per process
/// in production, so parallel tests with different databases would otherwise
/// observe each other's filter.
static OFFLINE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) async fn lock_offline_catalog() -> tokio::sync::MutexGuard<'static, ()> {
    OFFLINE_TEST_LOCK.lock().await
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

// A-03: production has exactly two physical schemas — `library.db`
// (`migrations_library`) and `media.db` (`migrations_media`) — and every other
// logical surface is an alias of one of them (see `db::init`). The fixtures
// below use the same schema sets so a test can never pass against columns the
// real database does not have.

/// A migrated in-memory pool for the production library schema.
pub(crate) async fn library_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await
}

/// A migrated in-memory pool for the production media schema.
pub(crate) async fn media_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_media")).await
}

/// Production `artist_user` lives in the library file.
pub(crate) async fn artist_user_pool() -> SqlitePool {
    library_pool().await
}

/// Production `download` lives in the media file.
pub(crate) async fn download_pool() -> SqlitePool {
    media_pool().await
}

/// Production `search` is the library file (a separate pool in `db::init`).
pub(crate) async fn search_pool() -> SqlitePool {
    library_pool().await
}

/// Production status (`session_status`) lives in the library file.
pub(crate) async fn status_pool() -> SqlitePool {
    library_pool().await
}

/// Production playlists live in the library file.
pub(crate) async fn playlist_pool() -> SqlitePool {
    library_pool().await
}

/// Every surface on the same pool (radio/row-window-shaped tests).
pub(crate) fn dbs_on(pool: &SqlitePool) -> Dbs {
    Dbs {
        credentials: pool.clone(),
        library: pool.clone(),
        sync: pool.clone(),
        search: pool.clone(),
        covers: pool.clone(),
        media: pool.clone(),
        lyrics: pool.clone(),
        colors: pool.clone(),
        status: pool.clone(),
        artist_user: pool.clone(),
        download: pool.clone(),
        playlist: pool.clone(),
    }
}

/// Library and media pools split the way the app splits them; every other
/// surface aliases one of the two.
pub(crate) fn dbs_with_media(library: &SqlitePool, media: &SqlitePool) -> Dbs {
    Dbs {
        credentials: library.clone(),
        library: library.clone(),
        sync: library.clone(),
        search: library.clone(),
        covers: media.clone(),
        media: media.clone(),
        lyrics: media.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        download: media.clone(),
        playlist: library.clone(),
    }
}

/// [`dbs_with_media`] with a real download ledger (downloaded-order tests).
pub(crate) fn dbs_with_download(
    library: &SqlitePool,
    media: &SqlitePool,
    download: &SqlitePool,
) -> Dbs {
    Dbs {
        download: download.clone(),
        ..dbs_with_media(library, media)
    }
}

/// Playlist tests: the playlist database is separate from the library, and
/// the remote-create test keeps its own credential pool.
pub(crate) fn dbs_with_credentials(
    credentials: &SqlitePool,
    library: &SqlitePool,
    playlist: &SqlitePool,
) -> Dbs {
    Dbs {
        credentials: credentials.clone(),
        library: library.clone(),
        sync: library.clone(),
        search: library.clone(),
        covers: library.clone(),
        media: library.clone(),
        lyrics: library.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        download: library.clone(),
        playlist: playlist.clone(),
    }
}

/// [`dbs_with_credentials`] with the library doubling as the credentials pool.
pub(crate) fn playlist_dbs(library: &SqlitePool, playlist: &SqlitePool) -> Dbs {
    dbs_with_credentials(library, library, playlist)
}

/// The production shape for multi-surface tests: one library pool and one
/// media pool, with every other [`Dbs`] surface aliased exactly like
/// [`crate::db::init`] does.
pub(crate) async fn isolated_dbs() -> Dbs {
    let library = library_pool().await;
    let media = media_pool().await;
    let mut dbs = dbs_with_media(&library, &media);
    // A-03 follows-up: the playlist surface is still its own empty pool
    // because `recommend`/`queue` fixtures install a playlist-only schema and
    // run `migrations_playlist` on it. Fold it into the library alias once
    // those fixtures move to the production tables.
    dbs.playlist = crate::test::memory_pool(None).await;
    dbs
}
