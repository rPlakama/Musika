use super::common::*;

#[tokio::test]
async fn playback_settings_default_round_trip_and_sanitize() {
    let pool = status_pool().await;
    let defaults = crate::playback_settings::load(&pool).await;
    assert_eq!(defaults.repeat, 0);
    assert_eq!(defaults.normalization, 0);
    assert_eq!(defaults.crossfade_seconds, 5.0);
    assert_eq!(defaults.crossfade_curve, "equal-power");
    assert_eq!(defaults.stream_cache_mib, 2048);
    assert!(defaults.stream_cache);
    assert!(defaults.follow_default);

    let mut changed = defaults.clone();
    changed.repeat = 2;
    changed.shuffle = true;
    changed.crossfade = true;
    changed.crossfade_seconds = 75.0;
    changed.crossfade_curve = "nonsense".to_string();
    changed.normalization = 9;
    changed.stream_max_bitrate = 99_999;
    changed.stream_cache_mib = 1;
    crate::playback_settings::save(&pool, &changed)
        .await
        .unwrap();

    let loaded = crate::playback_settings::load(&pool).await;
    assert_eq!(loaded.repeat, 2);
    assert!(loaded.shuffle);
    assert!(loaded.crossfade);
    // Clamped/sanitized on the way in and out.
    assert_eq!(loaded.crossfade_seconds, 12.0);
    assert_eq!(loaded.crossfade_curve, "equal-power");
    assert_eq!(loaded.normalization, 2);
    assert_eq!(loaded.stream_max_bitrate, 512);
    assert_eq!(loaded.stream_cache_mib, 64);
}

#[tokio::test]
async fn corrupt_playback_settings_fall_back_to_defaults() {
    let pool = status_pool().await;
    sqlx::query("INSERT INTO playback_settings (key, value) VALUES ('playback', 'not-json')")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        crate::playback_settings::load(&pool).await,
        crate::playback_settings::PlaybackSettings::default()
    );
}
