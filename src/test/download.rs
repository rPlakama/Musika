use super::common::*;
use crate::download::*;

async fn memory_pool() -> SqlitePool {
    crate::test::memory_pool(sqlx::migrate!("./migrations_download")).await
}

#[tokio::test]
async fn runs_the_state_machine_and_never_downgrades_done() {
    let pool = memory_pool().await;
    assert!(get(&pool, "s1").await.unwrap().is_none());

    mark_queued(&pool, "s1", Some("r1")).await.unwrap();
    let row = get(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(row.state, QUEUED);
    assert_eq!(row.rule_id.as_deref(), Some("r1"));

    set_state(&pool, "s1", ACTIVE, None).await.unwrap();
    set_state(&pool, "s1", ERROR, Some("network down"))
        .await
        .unwrap();
    let row = get(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(row.state, ERROR);
    assert_eq!(row.error.as_deref(), Some("network down"));

    // Re-selecting a failed song retries it (clears the error)...
    mark_queued(&pool, "s1", None).await.unwrap();
    let row = get(&pool, "s1").await.unwrap().unwrap();
    assert_eq!(row.state, QUEUED);
    assert_eq!(row.error, None);

    // ...but a finished row stays finished.
    set_state(&pool, "s1", DONE, None).await.unwrap();
    mark_queued(&pool, "s1", None).await.unwrap();
    assert_eq!(get(&pool, "s1").await.unwrap().unwrap().state, DONE);
}

#[tokio::test]
async fn done_ids_lists_only_finished_songs() {
    let pool = memory_pool().await;
    mark_queued(&pool, "s1", None).await.unwrap();
    set_state(&pool, "s2", DONE, None).await.unwrap();
    set_state(&pool, "s3", ERROR, Some("x")).await.unwrap();
    set_state(&pool, "s1", DONE, None).await.unwrap();

    let mut ids = done_ids(&pool).await.unwrap();
    ids.sort();
    assert_eq!(ids, ["s1", "s2"]);
}

#[tokio::test]
async fn counts_and_unmark() {
    let pool = memory_pool().await;
    mark_queued(&pool, "s1", Some("r1")).await.unwrap();
    mark_queued(&pool, "s2", Some("r1")).await.unwrap();
    set_state(&pool, "s2", DONE, None).await.unwrap();
    set_state(&pool, "s3", ERROR, Some("x")).await.unwrap();

    let snapshot = counts(&pool).await.unwrap();
    assert_eq!(
        snapshot,
        Counts {
            queued: 1,
            active: 0,
            done: 1,
            error: 1,
            total: 3
        }
    );

    // The rule's songs are readable before the marks go: the removal
    // path uses them to catch blobs stored without the rule id.
    let mut rule_songs = rule_song_ids(&pool, "r1").await.unwrap();
    rule_songs.sort();
    assert_eq!(rule_songs, ["s1", "s2"]);
    assert!(rule_song_ids(&pool, "nope").await.unwrap().is_empty());

    // Rule removal drops its pending *and* finished rows (the blobs go
    // with the rule); explicit downloads have no rule id and survive.
    assert_eq!(unmark_rule(&pool, "r1").await.unwrap(), 2);
    assert_eq!(counts(&pool).await.unwrap().total, 1);

    unmark(&pool, "s3").await.unwrap();
    assert_eq!(counts(&pool).await.unwrap().total, 0);
}

#[tokio::test]
async fn window_sets_split_done_from_in_flight() {
    let pool = memory_pool().await;
    mark_queued(&pool, "a", None).await.unwrap();
    set_state(&pool, "a", ACTIVE, None).await.unwrap();
    mark_queued(&pool, "b", None).await.unwrap();
    set_state(&pool, "b", DONE, None).await.unwrap();
    mark_queued(&pool, "c", None).await.unwrap();
    set_state(&pool, "c", ERROR, Some("nope")).await.unwrap();

    let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    assert_eq!(done_ids_of(&pool, &ids).await.unwrap(), vec!["b"]);
    assert_eq!(pending_ids_of(&pool, &ids).await.unwrap(), vec!["a"]);
    assert!(pending_ids_of(&pool, &[]).await.unwrap().is_empty());
}
