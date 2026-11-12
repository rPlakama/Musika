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

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A migrated in-memory pool for one schema.
pub(crate) async fn library_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await
}

pub(crate) async fn media_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_media")).await
}

pub(crate) async fn artist_user_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_artist_user")).await
}

pub(crate) async fn download_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_download")).await
}

pub(crate) async fn search_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_search")).await
}

pub(crate) async fn status_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_status")).await
}

pub(crate) async fn playlist_pool() -> SqlitePool {
    crate::test::memory_pool(None).await
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

/// Queue tests: isolated per-schema databases (credentials migrate separately
/// and tolerate missing tables).
pub(crate) async fn isolated_dbs() -> Dbs {
    let mut credential_migrations = sqlx::migrate!("./migrations");
    credential_migrations.set_ignore_missing(true);
    let credentials = crate::test::memory_pool(credential_migrations).await;
    let library = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    let covers = crate::test::memory_pool(sqlx::migrate!("./migrations_covers")).await;
    let download = crate::test::memory_pool(sqlx::migrate!("./migrations_download")).await;
    Dbs {
        credentials,
        sync: library.clone(),
        library,
        search: crate::test::memory_pool(None).await,
        covers,
        media: crate::test::memory_pool(None).await,
        lyrics: crate::test::memory_pool(None).await,
        colors: crate::test::memory_pool(None).await,
        status: crate::test::memory_pool(None).await,
        artist_user: crate::test::memory_pool(None).await,
        download,
        playlist: crate::test::memory_pool(None).await,
    }
}
