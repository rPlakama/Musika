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
    assert!(defaults.scrobble);

    let mut changed = defaults.clone();
    changed.repeat = 2;
    changed.shuffle = true;
    changed.crossfade = true;
    changed.crossfade_seconds = 75.0;
    changed.crossfade_curve = "nonsense".to_string();
    changed.normalization = 9;
    changed.stream_max_bitrate = 99_999;
    changed.stream_cache_mib = 1;
    changed.output_device = Some("sink.one".to_string());
    changed.eq_profile = Some("Rock".to_string());
    changed.exclusive = true;
    changed.bit_perfect = true;
    changed.output_backend = crate::playback_settings::OutputBackend::Alsa;
    changed.alsa_device = "hw:0,0".to_string();
    changed.follow_default = false;
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
    assert_eq!(loaded.output_device.as_deref(), Some("sink.one"));
    assert_eq!(loaded.eq_profile.as_deref(), Some("Rock"));
    assert!(loaded.exclusive);
    assert!(loaded.bit_perfect);
    assert_eq!(
        loaded.output_backend,
        crate::playback_settings::OutputBackend::Alsa
    );
    assert_eq!(loaded.alsa_device, "hw:0,0");
    assert!(!loaded.follow_default);
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

#[test]
fn stream_changes_apply_one_field_and_keys_match_the_request() {
    use crate::playback_settings::{PlaybackSettings, StreamChange};

    let mut settings = PlaybackSettings::default();
    assert!(settings.stream_is_original());
    assert_eq!(settings.stream_quality_key(), "original");

    StreamChange::Quality(128).apply(&mut settings);
    StreamChange::Format(Some("OPUS".to_string())).apply(&mut settings);
    assert!(!settings.stream_is_original());
    assert_eq!(settings.stream_format.as_deref(), Some("opus"));
    assert_eq!(settings.stream_quality_key(), "stream:128:opus");

    StreamChange::Format(None).apply(&mut settings);
    StreamChange::Quality(0).apply(&mut settings);
    assert!(settings.stream_is_original());

    StreamChange::Cache(false).apply(&mut settings);
    assert!(!settings.stream_cache);
    StreamChange::CacheSize(1).apply(&mut settings);
    assert_eq!(settings.stream_cache_mib, 64);
    StreamChange::FixedRate(Some(96_000)).apply(&mut settings);
    assert!(settings.fixed_pipeline_rate);
    assert_eq!(settings.fixed_pipeline_hz, 96_000);
    StreamChange::FixedRate(None).apply(&mut settings);
    assert!(!settings.fixed_pipeline_rate);
}
