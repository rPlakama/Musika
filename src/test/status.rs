use super::common::*;
use crate::status::*;

async fn memory_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_status")).await
}

#[tokio::test]
async fn round_trips_and_last_write_wins() {
    let pool = memory_pool().await;
    assert!(load(&pool).await.unwrap().is_none());

    let first = SessionStatus {
        server_url: "https://music.example".to_string(),
        username: "demo".to_string(),
        album_id: Some("al1".to_string()),
        song_id: Some("s1".to_string()),
        page_kind: "album".to_string(),
        page_id: "al1".to_string(),
        queue: r#"["s1","s2","s3"]"#.to_string(),
        queue_index: 1,
        position: 61.5,
        volume: 42.0,
        muted: true,
        updated_at: 100,
    };
    save(&pool, &first).await.unwrap();
    assert_eq!(load(&pool).await.unwrap().as_ref(), Some(&first));

    // The single row is replaced, never duplicated.
    let second = SessionStatus {
        album_id: None,
        song_id: None,
        queue: String::new(),
        queue_index: 0,
        position: 0.0,
        volume: 70.0,
        muted: false,
        updated_at: 200,
        ..first.clone()
    };
    save(&pool, &second).await.unwrap();
    let loaded = load(&pool).await.unwrap().unwrap();
    assert_eq!(loaded, second);
    assert_eq!(loaded.album_id, None);
    assert_eq!(loaded.queue, "");
    assert_eq!(loaded.position, 0.0);
    assert_eq!(loaded.updated_at, 200);
}
