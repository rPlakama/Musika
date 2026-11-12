use super::common::*;
use crate::mpris::*;
use zbus::zvariant::{ObjectPath, Value};

#[test]
fn track_paths_are_valid_object_paths() {
    assert_eq!(
        track_path("3PlpTsrmYRDfmuo3fe3PEJ"),
        "/org/musika/track/3PlpTsrmYRDfmuo3fe3PEJ"
    );
    assert_eq!(track_path("a-b.c"), "/org/musika/track/a_b_c");
    assert_eq!(track_path(""), "/org/mpris/MediaPlayer2/TrackList/NoTrack");
    assert!(ObjectPath::try_from(track_path("a-b.c")).is_ok());
}

#[test]
fn metadata_carries_the_expected_roles() {
    let state = Snapshot {
        title: "Idioteque".to_string(),
        artist: "Radiohead".to_string(),
        album: "Kid A".to_string(),
        track_id: "abc123".to_string(),
        duration_us: 297_000_000,
        art_url: "file:///tmp/a.jpg".to_string(),
        ..Snapshot::default()
    };
    let map = metadata(&state);
    assert!(map.contains_key("mpris:trackid"));
    assert_eq!(map["mpris:length"], owned(Value::from(297_000_000i64)));
    assert_eq!(map["xesam:title"], owned(Value::from("Idioteque")));
    assert_eq!(
        map["xesam:artist"],
        owned(Value::from(vec!["Radiohead".to_string()]))
    );
    assert!(map.contains_key("mpris:artUrl"));
}

#[test]
fn idle_state_exposes_the_no_track_placeholder() {
    let map = metadata(&Snapshot::default());
    assert_eq!(
        map["mpris:trackid"],
        owned(Value::from(
            ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack").unwrap()
        ))
    );
    assert!(!map.contains_key("mpris:length"));
}

#[test]
fn signal_key_ignores_position_only_updates() {
    let mut state = Snapshot {
        title: "Idioteque".to_string(),
        ..Snapshot::default()
    };
    let before = state.signal_key();
    state.position_us = 42_000_000;
    assert_eq!(before, state.signal_key());

    state.volume = 0.5;
    assert_ne!(before, state.signal_key());
}

/// FIX-8: a flood beyond the capacity drops commands instead of growing
/// the queue or blocking the bus thread.
#[test]
fn a_full_command_queue_drops_instead_of_blocking() {
    let (tx, mut rx) = mpsc::channel(2);
    queue_command(&tx, "mpris_play".to_string());
    queue_command(&tx, "track next".to_string());
    queue_command(&tx, "track previous".to_string());
    assert_eq!(rx.try_recv().as_deref(), Ok("mpris_play"));
    assert_eq!(rx.try_recv().as_deref(), Ok("track next"));
    assert!(rx.try_recv().is_err(), "the excess command was dropped");
}

/// Regression: MPRIS next/previous used to submit "mpris_track next", which is
/// not a command (two-word commands cannot use the single-word `mpris_*` alias
/// vocabulary), so the dispatcher dropped them and the controls did nothing.
#[test]
fn track_controls_submit_canonical_commands() {
    use crate::ipc_command::{canonical_string, parse};

    assert!(parse("mpris_track next").is_err());
    for (action, canonical) in [("next", "track next"), ("previous", "track previous")] {
        let line = track_command(action);
        assert_eq!(line, canonical);
        let command = parse(&line).expect("canonical track command parses");
        assert_eq!(canonical_string(&command), canonical);
    }
}
