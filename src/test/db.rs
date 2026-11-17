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
