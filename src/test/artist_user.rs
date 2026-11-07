use super::common::*;
use crate::artist_user::*;

async fn memory_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_artist_user")).await
}

#[tokio::test]
async fn writes_preserve_the_other_field() {
    let pool = memory_pool().await;
    assert!(get(&pool, "a1").await.unwrap().is_none());

    set_starred(&pool, "a1", true).await.unwrap();
    let row = get(&pool, "a1").await.unwrap().unwrap();
    assert!(row.starred);
    assert_eq!(row.rating, 0);

    set_rating(&pool, "a1", 4).await.unwrap();
    let row = get(&pool, "a1").await.unwrap().unwrap();
    assert!(row.starred, "rating must not clear the favourite");
    assert_eq!(row.rating, 4);

    set_starred(&pool, "a1", false).await.unwrap();
    let row = get(&pool, "a1").await.unwrap().unwrap();
    assert!(!row.starred);
    assert_eq!(row.rating, 4, "unstarring must not clear the rating");
}

#[tokio::test]
async fn star_index_lists_only_favourites() {
    let pool = memory_pool().await;
    set_starred(&pool, "a1", true).await.unwrap();
    set_starred(&pool, "a2", false).await.unwrap();
    set_rating(&pool, "a3", 5).await.unwrap();
    assert_eq!(starred_ids(&pool).await.unwrap(), vec!["a1"]);
}

#[tokio::test]
async fn rating_outside_the_domain_is_refused() {
    let pool = memory_pool().await;
    assert!(set_rating(&pool, "a1", 6).await.is_err());
    assert!(get(&pool, "a1").await.unwrap().is_none());
}

#[tokio::test]
async fn ratings_map_and_import_skip_invalid_levels() {
    let pool = memory_pool().await;
    set_rating(&pool, "a1", 3).await.unwrap();
    set_rating(&pool, "a2", 0).await.unwrap();

    set_starred(&pool, "a1", true).await.unwrap();

    let map = ratings(&pool).await.unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("a1"), Some(&3));

    import_ratings(
        &pool,
        &[
            ("a1".to_string(), 5),
            ("a2".to_string(), 2),
            ("a3".to_string(), 9),
        ],
    )
    .await
    .unwrap();
    let row = get(&pool, "a1").await.unwrap().unwrap();
    assert_eq!(row.rating, 5);
    assert!(row.starred, "import must preserve the favourite");
    assert_eq!(get(&pool, "a2").await.unwrap().unwrap().rating, 2);
    assert!(get(&pool, "a3").await.unwrap().is_none());
}

#[tokio::test]
async fn payload_defaults_and_reads() {
    let pool = memory_pool().await;
    let unknown: serde_json::Value = serde_json::from_str(&payload(&pool, "a1").await).unwrap();
    assert_eq!(unknown["starred"], false);
    assert_eq!(unknown["rating"], 0);

    set_starred(&pool, "a1", true).await.unwrap();
    set_rating(&pool, "a1", 3).await.unwrap();
    let known: serde_json::Value = serde_json::from_str(&payload(&pool, "a1").await).unwrap();
    assert_eq!(known["starred"], true);
    assert_eq!(known["rating"], 3);
}
