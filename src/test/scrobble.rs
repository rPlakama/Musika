use super::common::*;

#[test]
fn scrobble_threshold_is_half_the_track_capped_at_four_minutes() {
    assert_eq!(crate::scrobble::threshold_secs(0.0), 240.0);
    assert_eq!(crate::scrobble::threshold_secs(60.0), 30.0);
    assert_eq!(crate::scrobble::threshold_secs(480.0), 240.0);
    assert_eq!(crate::scrobble::threshold_secs(1.0), 1.0);
}

#[tokio::test]
async fn offline_submissions_queue_once_and_wait_for_online() {
    let pool = status_pool().await;
    let dbs = dbs_on(&pool);

    crate::scrobble::submit(&dbs, "s1", 1_700_000_000).await;
    crate::scrobble::submit(&dbs, "s1", 1_700_000_000).await;
    assert_eq!(crate::scrobble::pending_count(&pool).await.unwrap(), 1);
    assert_eq!(crate::scrobble::flush(&dbs).await.unwrap(), 0);
    assert_eq!(crate::scrobble::pending_count(&pool).await.unwrap(), 1);
}
