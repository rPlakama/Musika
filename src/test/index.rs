use super::common::*;
use crate::index::*;

#[test]
fn provider_date_normalises_milliseconds() {
    assert_eq!(provider_secs(1_700_000_000), 1_700_000_000);
    assert_eq!(provider_secs(1_700_000_000_123), 1_700_000_000);
    assert_eq!(provider_secs(0), 0);
}

#[test]
fn complete_fetches_pass_and_partial_ones_fail() {
    assert!(ensure_complete(&[], 10).is_ok());

    let err = ensure_complete(&["al-1".to_string()], 10).unwrap_err();
    assert!(err.to_string().contains("1 of 10"));
    assert!(err.to_string().contains("al-1"));

    let err = ensure_complete(&["a".to_string(), "b".to_string()], 3).unwrap_err();
    assert!(err.to_string().contains("2 of 3"));
}

#[test]
fn cover_pass_follows_the_effective_depth() {
    let report = SyncReport {
        deep: true,
        covers_due: true,
        ..SyncReport::default()
    };
    assert!(report.wants_cover_pass());

    let shallow = SyncReport {
        deep: false,
        covers_due: true,
        ..SyncReport::default()
    };
    assert!(!shallow.wants_cover_pass());
}
