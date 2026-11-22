use super::common::*;
use crate::cache::*;
use sqlx::SqlitePool;

async fn pool() -> SqlitePool {
    crate::test::memory_pool(None).await
}

#[tokio::test]
async fn removes_only_rows_missing_from_the_owner_set() {
    let derived = pool().await;
    sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
        .execute(&derived)
        .await
        .unwrap();
    for id in ["keep", "gone", "also-gone"] {
        sqlx::query("INSERT INTO cache (id) VALUES (?1)")
            .bind(id)
            .execute(&derived)
            .await
            .unwrap();
    }

    let owners: HashSet<String> = ["keep".to_string()].into_iter().collect();
    assert_eq!(
        purge_missing(&derived, "cache", "id", &owners)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        purge_missing(&derived, "cache", "id", &owners)
            .await
            .unwrap(),
        0
    );
}

/// The owner-pool variant asks only about the derived ids; an empty cache
/// must not query the owner table at all (it can be absent).
#[tokio::test]
async fn purge_missing_against_checks_only_the_derived_ids() {
    let derived = pool().await;
    sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
        .execute(&derived)
        .await
        .unwrap();
    for id in ["keep", "gone", "also-gone"] {
        sqlx::query("INSERT INTO cache (id) VALUES (?1)")
            .bind(id)
            .execute(&derived)
            .await
            .unwrap();
    }
    let owners = pool().await;
    sqlx::query("CREATE TABLE owners (id TEXT PRIMARY KEY)")
        .execute(&owners)
        .await
        .unwrap();
    sqlx::query("INSERT INTO owners (id) VALUES ('keep')")
        .execute(&owners)
        .await
        .unwrap();

    assert_eq!(
        purge_missing_against(&derived, "cache", "id", &owners, "owners", "id")
            .await
            .unwrap(),
        2
    );

    let empty = pool().await;
    sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
        .execute(&empty)
        .await
        .unwrap();
    let no_owner_table = pool().await;
    assert_eq!(
        purge_missing_against(&empty, "cache", "id", &no_owner_table, "owners", "id")
            .await
            .unwrap(),
        0
    );
}

/// FIX-10: a replace that cannot rename (the destination is a directory)
/// leaves no `*.tmp` behind.
#[test]
fn a_failed_replace_removes_the_temp_file() {
    let dir = std::env::temp_dir().join(format!("musika-atomic-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("cover.jpg")).expect("blocking dir");
    assert!(write_atomic(&dir, "cover.jpg", b"bytes").is_err());
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .expect("temp dir")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files must be cleaned up");
    _ = std::fs::remove_dir_all(dir);
}

/// N-06: `delete_in` binds `cache::IN_BATCH` chunks; a single `IN (...)` for a
/// large queue snapshot could exceed SQLite's parameter limit.
#[tokio::test]
async fn delete_in_chunks_past_the_bind_limit() {
    let pool = pool().await;
    sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO cache (id) VALUES ('keep')")
        .execute(&pool)
        .await
        .unwrap();
    let count = crate::cache::IN_BATCH * 2 + 7;
    let seed = format!(
        "WITH RECURSIVE c(x) AS (
             VALUES(0) UNION ALL SELECT x + 1 FROM c WHERE x + 1 < {}
         ) INSERT INTO cache (id) SELECT 'id' || x FROM c;",
        count - 1
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(seed))
        .execute(&pool)
        .await
        .unwrap();

    let ids: Vec<String> = (0..count).map(|i| format!("id{i}")).collect();
    let mut conn = pool.acquire().await.unwrap();
    delete_in(&mut conn, "DELETE FROM cache WHERE id IN (", &ids)
        .await
        .unwrap();
    // An empty list is a no-op, not an `IN ()` syntax error.
    delete_in(&mut conn, "DELETE FROM cache WHERE id IN (", &[])
        .await
        .unwrap();
    drop(conn);

    let left: Vec<String> = sqlx::query_scalar("SELECT id FROM cache")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["keep"]);
}
