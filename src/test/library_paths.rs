use super::common::*;
use crate::library::*;

#[tokio::test]
async fn album_artist_id_resolves_the_owner() {
    let pool = library_pool().await;
    sqlx::query("INSERT INTO albums (id, name, artist_id) VALUES ('al1', 'Kid A', 'ar1')")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        album_artist_id(&pool, "al1").await.unwrap().as_deref(),
        Some("ar1")
    );
    assert!(album_artist_id(&pool, "missing").await.unwrap().is_none());
}

#[tokio::test]
async fn album_info_cache_and_cover_check_gate_round_trip() {
    let pool = library_pool().await;
    sqlx::query("INSERT INTO albums (id, name) VALUES ('al1', 'Kid A')")
        .execute(&pool)
        .await
        .unwrap();

    assert!(album_info(&pool, "al1").await.unwrap().is_none());
    upsert_album_info(&pool, "al1", "notes").await.unwrap();
    let (notes, _) = album_info(&pool, "al1").await.unwrap().unwrap();
    assert_eq!(notes, "notes");

    invalidate_album_info(&pool, &["al1".to_string()])
        .await
        .unwrap();
    assert!(album_info(&pool, "al1").await.unwrap().is_none());

    upsert_album_info(&pool, "gone", "stale").await.unwrap();
    assert_eq!(purge_stale_album_info(&pool).await.unwrap(), 1);
    assert!(album_info(&pool, "gone").await.unwrap().is_none());

    assert!(cover_check_due(&pool, 3600).await.unwrap());
    touch_cover_check(&pool).await.unwrap();
    assert!(!cover_check_due(&pool, 3600).await.unwrap());
}
