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
