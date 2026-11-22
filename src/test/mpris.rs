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

/// D-01: MPRIS `Volume` is `0.0..=1.0` while the core's `mpris_volume:` parser
/// is `0..=100`, so the payload must be scaled exactly once.
/// D-02/D-03: MPRIS `SetShuffle` switches the persistent shuffle *mode*; it must
/// not submit `shuffle queue`, which permutes and restarts the play queue.
#[test]
fn volume_scales_0_to_100_and_shuffle_toggles_mode() {
    use crate::ipc_command::{CommandTarget, IpcCommand, canonical_string, parse};
    use crate::ipc_protocol::PlayerOp;

    assert_eq!(volume_command(0.5).as_deref(), Some("mpris_volume:50.00"));
    assert_eq!(volume_command(1.0).as_deref(), Some("mpris_volume:100.00"));
    assert_eq!(volume_command(0.0).as_deref(), Some("mpris_volume:0.00"));
    assert_eq!(
        volume_command(2.5).as_deref(),
        Some("mpris_volume:100.00"),
        "out-of-range volumes clamp to the MPRIS maximum"
    );
    assert_eq!(volume_command(-0.5).as_deref(), Some("mpris_volume:0.00"));
    assert_eq!(volume_command(f64::NAN), None, "non-finite volume rejected");
    assert_eq!(
        volume_command(f64::INFINITY),
        None,
        "non-finite volume rejected"
    );

    // The parser stores the 0..=100 percentage; there is no second scaling.
    let volume =
        parse(&volume_command(0.5).expect("finite volume")).expect("volume command parses");
    assert_eq!(volume, IpcCommand::SetVolume(50.0));
    assert_eq!(volume.player_op(), Some(PlayerOp::SetVolume(50.0)));
    assert_eq!(canonical_string(&volume), "set_volume 50");

    // MPRIS writes are conditional: a same-state set is not repeated.
    assert_eq!(
        shuffle_command(false, true).as_deref(),
        Some("mpris_shuffle:on")
    );
    assert_eq!(
        shuffle_command(true, false).as_deref(),
        Some("mpris_shuffle:off")
    );
    assert_eq!(shuffle_command(false, false), None);
    assert_eq!(shuffle_command(true, true), None);

    let shuffle =
        parse(&shuffle_command(false, true).expect("shuffle change")).expect("shuffle parses");
    assert_eq!(shuffle, IpcCommand::ShuffleMode { enabled: true });
    assert_eq!(shuffle.target(), CommandTarget::CorePlayback);
    assert_eq!(
        shuffle.player_op(),
        Some(PlayerOp::ShuffleQueue),
        "the persistent mode toggle, not the queue permutation (ShuffleQueueOrder)"
    );
    assert_eq!(canonical_string(&shuffle), "mpris_shuffle:on");
    assert!(parse("mpris_shuffle:maybe").is_err());
    assert!(parse("mpris_shuffle:").is_err());

    // End-to-end through the daemon's text dispatcher: the MPRIS command lands
    // on the persistent mode command, never on the queue permutation.
    assert!(matches!(
        crate::daemon::playback::command_for_text(&shuffle),
        Some(crate::player::core::CoreCommand::ShuffleQueue)
    ));
    let permutation = parse("shuffle queue").expect("shuffle queue parses");
    assert!(matches!(
        crate::daemon::playback::command_for_text(&permutation),
        Some(crate::player::core::CoreCommand::ShuffleQueueOrder(_))
    ));
}

/// D-25(b): the idle snapshot must expose spec-valid MPRIS enum strings, and a
/// malformed publisher value must never leak through the property getters.
#[test]
fn snapshot_default_uses_spec_valid_strings() {
    let state = Snapshot::default();
    assert_eq!(state.playback_status, "Stopped");
    assert_eq!(state.loop_status, "None");
    assert!(!state.shuffle);
    assert_eq!(state.duration_us, 0);
    assert_eq!(state.volume, 0.0);
    assert_eq!(
        track_path(&state.track_id),
        "/org/mpris/MediaPlayer2/TrackList/NoTrack"
    );

    assert_eq!(playback_status("Playing"), "Playing");
    assert_eq!(playback_status("Paused"), "Paused");
    assert_eq!(playback_status(""), "Stopped");
    assert_eq!(playback_status("playing"), "Stopped");
    assert_eq!(loop_status("Track"), "Track");
    assert_eq!(loop_status("Playlist"), "Playlist");
    assert_eq!(loop_status(""), "None");
    assert_eq!(loop_status("Repeat"), "None");
}

/// D-25(c): `SetPosition` must ignore negative positions, positions past the
/// track length and any position when the length is unknown.
#[test]
fn set_position_ignores_negative_and_past_end_positions() {
    assert_eq!(seeks_within_track(0, 0), None, "unknown track length");
    assert_eq!(seeks_within_track(0, 10_000), Some(0));
    assert_eq!(seeks_within_track(5_000, 10_000), Some(5_000));
    assert_eq!(seeks_within_track(10_000, 10_000), Some(10_000));
    assert_eq!(seeks_within_track(10_001, 10_000), None);
    assert_eq!(seeks_within_track(-1, 10_000), None);
}
