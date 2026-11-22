use super::common::*;
use crate::db::*;

#[test]
fn library_migrations_include_the_combined_schema() {
    let migrations = sqlx::migrate!("./migrations_library");
    assert!(
        migrations
            .iter()
            .any(|migration| migration.version == 20261017000001)
    );
}

#[tokio::test]
async fn production_library_migrations_include_playback_status_tables() {
    let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    for table in ["playback_settings", "pending_scrobble"] {
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(found, 1, "production library schema is missing {table}");
    }
}

/// A-03: every shared fixture must be migrated from the production schema
/// sets only, exactly like `db::init` migrates the two real database files.
/// Comparing the applied migration versions pins the fixtures to the same
/// schema list, so a test can no longer pass against columns the real database
/// does not have.
#[tokio::test]
async fn test_pool_helpers_use_the_production_migrations() {
    fn versions(migrations: &sqlx::migrate::Migrator) -> Vec<i64> {
        migrations
            .iter()
            .map(|migration| migration.version)
            .collect()
    }
    async fn assert_applied(pool: &sqlx::SqlitePool, label: &str, expected: &[i64]) {
        let applied: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
                .fetch_all(pool)
                .await
                .unwrap();
        assert_eq!(
            applied, expected,
            "{label} is not migrated from the production schema set"
        );
    }

    let library_versions = versions(&sqlx::migrate!("./migrations_library"));
    let media_versions = versions(&sqlx::migrate!("./migrations_media"));

    for (label, pool) in [
        ("library_pool", library_pool().await),
        ("artist_user_pool", artist_user_pool().await),
        ("search_pool", search_pool().await),
        ("status_pool", status_pool().await),
        ("playlist_pool", playlist_pool().await),
    ] {
        assert_applied(&pool, label, &library_versions).await;
    }
    for (label, pool) in [
        ("media_pool", media_pool().await),
        ("download_pool", download_pool().await),
    ] {
        assert_applied(&pool, label, &media_versions).await;
    }

    // The multi-surface fixture aliases the same two schema sets, so every
    // non-playlist surface must report the matching production migrations.
    let dbs = isolated_dbs().await;
    for (label, pool) in [
        ("isolated_dbs.library", &dbs.library),
        ("isolated_dbs.credentials", &dbs.credentials),
        ("isolated_dbs.sync", &dbs.sync),
        ("isolated_dbs.search", &dbs.search),
        ("isolated_dbs.colors", &dbs.colors),
        ("isolated_dbs.status", &dbs.status),
        ("isolated_dbs.artist_user", &dbs.artist_user),
    ] {
        assert_applied(pool, label, &library_versions).await;
    }
    for (label, pool) in [
        ("isolated_dbs.covers", &dbs.covers),
        ("isolated_dbs.media", &dbs.media),
        ("isolated_dbs.lyrics", &dbs.lyrics),
        ("isolated_dbs.download", &dbs.download),
    ] {
        assert_applied(pool, label, &media_versions).await;
    }
    // `isolated_dbs.playlist` is still an empty pool: `recommend`/`queue`
    // fixtures install a playlist-only schema and run their own migration.
    // The A-03 hygiene pass folds it into the library alias; see
    // `test/common.rs`.
}

#[tokio::test]
async fn wipe_pool_drops_every_table_and_migrations_rebuild_them() {
    let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
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

/// Applied migrations are immutable: SQLx stores a SHA-384 of the file, and
/// editing one — even a comment — makes every existing database fail with
/// "previously applied but has been modified". This test encodes the hashes of
/// migrations that have already shipped so a future edit is caught here, not
/// on a user's real database.
#[test]
fn shipped_migrations_are_immutable() {
    use openssl::hash::MessageDigest;

    const SHIPPED: &[(&str, &str)] = &[
        (
            "20261117000001_playback_settings.sql",
            "4fcfbb24984ebab9a7f133e2e1ec041acd34863430b39de753c0a39feb5076ddffd4b8fa66e9a9e4102adc49c2762ecc",
        ),
        (
            "20261117000002_pending_scrobble.sql",
            "c666dcd5b3ab97f481a7ce50bfdf57523f6af5eda07038296349d0ff755de021e4e5e1f781dd0467cfdbe84d80801561",
        ),
    ];
    for (file, expected) in SHIPPED {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations_library")
            .join(file);
        let bytes = std::fs::read(&path).unwrap();
        let digest = openssl::hash::hash(MessageDigest::sha384(), &bytes).unwrap();
        assert_eq!(
            crate::hex::encode(digest),
            *expected,
            "shipped migration {file} was edited; add a new migration instead"
        );
    }
}

#[tokio::test]
async fn import_file_detaches_legacy_database_on_query_error() {
    use std::str::FromStr;

    let dir = std::env::temp_dir().join(format!("musika-db-detach-err-{}", std::process::id()));
    _ = std::fs::remove_dir_all(&dir);
    ensure_dir(&dir).unwrap();

    // The main connection must be file-backed: SQLite turns ATTACH into a
    // private in-memory database when the connection itself was opened with
    // SQLITE_OPEN_MEMORY (`sqlite::memory:`), which would silently turn the
    // import into a no-op instead of exercising the error path.
    let main_path = dir.join("main.db");
    let main_options =
        sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite://{}", main_path.display()))
            .unwrap()
            .create_if_missing(true);
    let main_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(main_options)
        .await
        .unwrap();
    sqlx::migrate!("./migrations_library")
        .run(&main_pool)
        .await
        .unwrap();

    let bad_legacy_path = dir.join("bad_legacy.db");
    {
        let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
            "sqlite://{}",
            bad_legacy_path.display()
        ))
        .unwrap()
        .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE users (c1 TEXT, c2 TEXT, c3 TEXT, c4 TEXT, c5 TEXT, c6 TEXT, c7 TEXT, c8 TEXT);
             INSERT INTO users VALUES ('1', '2', '3', '4', '5', '6', '7', '8');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    let mut conn = main_pool.acquire().await.unwrap();
    let err = import_file(&mut conn, &bad_legacy_path, &["users"]).await;
    assert!(err.is_err(), "mismatched legacy schema must return Err");

    // Verify `legacy` was detached despite the error: attaching again or inspecting
    // attached databases must not see `legacy`.
    let attached: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
        .fetch_all(&mut *conn)
        .await
        .unwrap();
    assert!(
        !attached.iter().any(|(_, name, _)| name == "legacy"),
        "legacy database must be detached after import_file error: {attached:?}"
    );

    _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn playback_dir_defaults_to_xdg_cache_home_musika_playback() {
    if std::env::var_os("MUSIKA_TEST_PLAYBACK_DIR_CHILD").is_some() {
        let cache_home = std::path::PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap());
        assert_eq!(
            playback_dir().unwrap(),
            cache_home.join("musika").join("playback")
        );
        return;
    }

    let base = std::env::temp_dir().join(format!("musika-playback-dir-{}", std::process::id()));
    let cache_home = base.join("xdg_cache");
    let exe = std::env::current_exe().unwrap();
    let output = std::process::Command::new(&exe)
        .env("XDG_CACHE_HOME", &cache_home)
        .env_remove("MUSIKA_MEDIA_CACHE_DIR")
        .env("MUSIKA_TEST_PLAYBACK_DIR_CHILD", "1")
        .args([
            "--exact",
            "test::db::playback_dir_defaults_to_xdg_cache_home_musika_playback",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "playback_dir child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
