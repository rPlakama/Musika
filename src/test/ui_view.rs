//! UI-side `PlaybackView` projection tests (`src/ui/daemon_client.rs`).
//!
//! The core owns playback and queue state; these cases pin the behaviour the
//! IPC v2 stream contract relies on: chunked `QueueRows` bodies filling a
//! `Reset` placeholder, structural deltas, `seq`/`rev` gap detection and the
//! anchored-position extrapolation.

#[cfg(feature = "gpuui")]
mod gpuui {
    use std::sync::Arc;

    use crate::ipc_protocol::{
        ErrorCode, PlaybackState, PlayerReply, QueueDelta, QueueOp, QueueRows, StateWire, WireTrack,
    };
    use crate::player::State;
    use crate::ui::daemon_client::{PlaybackView, move_block_allowed};

    fn row(id: &str, favorite: bool) -> WireTrack {
        WireTrack {
            id: id.to_string(),
            title: format!("Title {id}"),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            album_id: "album-1".to_string(),
            favorite,
            duration: 210.5,
            cover_art: "cover-1".to_string(),
            cover_url: "file:///tmp/cover.jpg".to_string(),
            cover_remote: "https://example.invalid/cover".to_string(),
        }
    }

    fn reset(rev: u64, count: u32) -> QueueDelta {
        QueueDelta {
            rev,
            op: QueueOp::Reset { count },
        }
    }

    fn rows(rev: u64, start: u32, tracks: Vec<WireTrack>) -> QueueRows {
        QueueRows { rev, start, rows: tracks }
    }

    fn delta(rev: u64, op: QueueOp) -> QueueDelta {
        QueueDelta { rev, op }
    }

    fn state(seq: u64, index: u32, wire: StateWire, position_ms: u32) -> PlaybackState {
        PlaybackState {
            seq,
            index,
            anchor_ns: crate::linux::monotonic_ns(),
            duration_ms: 120_000,
            position_ms,
            volume_milli: 700,
            state: match wire {
                StateWire::Stopped => 0,
                StateWire::Playing => 1,
                StateWire::Paused => 2,
            },
            rate_milli: 1_000,
            ..PlaybackState::default()
        }
    }

    #[test]
    fn reset_then_rows_builds_the_queue() {
        let mut view = PlaybackView::new();
        view.apply_delta(reset(7, 2));
        // A Reset lands before its row bodies: placeholders are safe, not rows.
        assert_eq!(view.len(), 2);
        assert!(view.queue().iter().all(|track| track.id.is_empty()));
        assert!(view.current().is_none());

        view.apply_rows(rows(7, 0, vec![row("a", false), row("b", true)]));
        assert_eq!(view.queue_rev(), 7);
        assert_eq!(view.len(), 2);
        assert_eq!(view.queue()[0].id, "a");
        assert_eq!(view.current().map(|track| track.id.as_str()), Some("a"));
        assert_eq!(view.queue_snapshot().len(), 2);

        // The snapshot is cached until the rows change.
        let first = view.queue_snapshot();
        assert!(Arc::ptr_eq(&first, &view.queue_snapshot()));
        view.apply_delta(delta(
            8,
            QueueOp::Patch {
                index: 0,
                favorite: true,
            },
        ));
        assert!(view.queue()[0].favorite);
        assert!(!Arc::ptr_eq(&first, &view.queue_snapshot()));
        assert!(!view.take_resync());
    }

    #[test]
    fn structural_deltas_follow_the_core() {
        let mut view = PlaybackView::new();
        view.apply_delta(reset(1, 3));
        view.apply_rows(rows(
            1,
            0,
            vec![row("a", false), row("b", false), row("c", false)],
        ));
        view.apply_state(state(1, 1, StateWire::Playing, 0));
        assert_eq!(view.current().map(|track| track.id.as_str()), Some("b"));

        // Inserting before the playing row shifts the index with it.
        view.apply_delta(delta(
            2,
            QueueOp::Insert {
                at: 1,
                rows: vec![row("x", false)],
            },
        ));
        assert_eq!(view.len(), 4);
        assert_eq!(view.index(), 2);
        assert_eq!(view.current().map(|track| track.id.as_str()), Some("b"));
        let ids: Vec<&str> = view.queue().iter().map(|track| track.id.as_str()).collect();
        assert_eq!(ids, ["a", "x", "b", "c"]);

        // Removing a row above the playing one pulls the index back.
        view.apply_delta(delta(3, QueueOp::Remove { indices: vec![0] }));
        assert_eq!(view.len(), 3);
        assert_eq!(view.index(), 1);
        assert_eq!(view.current().map(|track| track.id.as_str()), Some("b"));

        // Moving the playing row keeps the index on it.
        view.apply_delta(delta(4, QueueOp::Move { from: 1, to: 2 }));
        assert_eq!(view.len(), 3);
        assert_eq!(view.index(), 2);
        assert_eq!(view.queue()[2].id, "b");

        assert!(!view.take_resync());
    }

    #[test]
    fn a_full_reset_replaces_every_row() {
        let mut view = PlaybackView::new();
        view.apply_delta(reset(1, 3));
        view.apply_rows(rows(
            1,
            0,
            vec![row("a", false), row("b", false), row("c", false)],
        ));
        view.apply_state(state(1, 2, StateWire::Playing, 0));

        // A reconnect resets the queue; the older bodies must not leak in.
        view.apply_delta(reset(9, 1));
        assert_eq!(view.len(), 1);
        assert_eq!(view.index(), 0);
        assert!(view.current().is_none());
        view.apply_rows(rows(9, 0, vec![row("z", false)]));
        assert_eq!(view.current().map(|track| track.id.as_str()), Some("z"));
        assert!(!view.take_resync());
    }

    #[test]
    fn a_sequence_gap_requests_a_resync() {
        let mut view = PlaybackView::new();
        view.apply_state(state(1, 0, StateWire::Stopped, 0));
        assert!(!view.take_resync());
        // Contiguous sequence: nothing to repair.
        view.apply_state(state(2, 0, StateWire::Paused, 0));
        assert!(!view.take_resync());
        // A dropped state frame leaves a gap.
        view.apply_state(state(4, 0, StateWire::Playing, 0));
        assert!(view.take_resync());
        assert!(!view.take_resync());
        assert_eq!(view.state(), State::Playing);
    }

    #[test]
    fn a_revision_gap_requests_a_resync_and_refuses_the_delta() {
        let mut view = PlaybackView::new();
        view.apply_delta(reset(10, 2));
        view.apply_rows(rows(10, 0, vec![row("a", false), row("b", false)]));
        assert!(!view.take_resync());

        // Structural deltas are never dropped, so a jump means the rows and the
        // core disagree: the delta must not be applied blindly.
        view.apply_delta(delta(
            12,
            QueueOp::Insert {
                at: 0,
                rows: vec![row("x", false)],
            },
        ));
        assert!(view.take_resync());
        assert_eq!(view.len(), 2);
        assert_eq!(view.queue()[0].id, "a");

        // The next contiguous revision still applies.
        view.apply_delta(delta(
            11,
            QueueOp::Insert {
                at: 1,
                rows: vec![row("x", false)],
            },
        ));
        assert_eq!(view.len(), 3);
        assert!(!view.take_resync());
    }

    #[test]
    fn stale_row_bodies_are_ignored() {
        let mut view = PlaybackView::new();
        view.apply_delta(reset(5, 1));
        view.apply_rows(rows(5, 0, vec![row("a", false)]));
        view.apply_rows(rows(4, 0, vec![row("stale", false)]));
        assert_eq!(view.queue()[0].id, "a");
    }

    #[test]
    fn playback_state_drives_transport_and_extrapolates_the_position() {
        let mut view = PlaybackView::new();
        view.apply_delta(reset(1, 2));
        view.apply_rows(rows(1, 0, vec![row("a", false), row("b", false)]));

        view.apply_state(PlaybackState {
            seq: 1,
            index: 1,
            anchor_ns: crate::linux::monotonic_ns(),
            duration_ms: 120_000,
            position_ms: 1_000,
            volume_milli: 550,
            state: 1,
            repeat: 2,
            rate_milli: 1_000,
            flags: crate::ipc_protocol::STATE_FLAG_MUTED | crate::ipc_protocol::STATE_FLAG_LOADING,
            ..PlaybackState::default()
        });
        assert_eq!(view.index(), 1);
        assert_eq!(view.state(), State::Playing);
        assert_eq!(view.repeat(), crate::player::Repeat::All);
        assert_eq!(view.current().map(|track| track.id.as_str()), Some("b"));
        assert!((view.position() - 1.0).abs() < 1e-9);
        let now = view.position_now();
        assert!((1.0..2.0).contains(&now), "position_now {now}");
        assert!((view.duration() - 120.0).abs() < 1e-9);
        assert!((view.volume() - 55.0).abs() < 1e-9);
        assert!(view.muted());
        assert!(view.is_loading());

        // A paused anchor never extrapolates.
        view.apply_state(state(2, 1, StateWire::Paused, 4_000));
        assert!((view.position() - 4.0).abs() < 1e-9);
        assert!((view.position_now() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn player_replies_track_rejections() {
        let mut view = PlaybackView::new();
        view.apply_player_reply(PlayerReply {
            req: 1,
            ok: true,
            code: ErrorCode::Internal,
            applied_seq: 3,
        });
        assert!(view.last_error().is_none());
        assert!(!view.take_resync());

        view.apply_player_reply(PlayerReply {
            req: 2,
            ok: false,
            code: ErrorCode::QueueRevMismatch,
            applied_seq: 3,
        });
        assert!(view.last_error().is_some());
        assert!(view.take_resync());
    }

    #[test]
    fn move_block_bounds_match_the_core_rules() {
        assert!(move_block_allowed(5, 0, 1, 1));
        assert!(move_block_allowed(5, 1, 2, 2));
        assert!(!move_block_allowed(5, 0, 1, 0));
        assert!(!move_block_allowed(5, 0, 0, 1));
        assert!(!move_block_allowed(0, 0, 1, 1));
        assert!(!move_block_allowed(5, 4, 1, 1));
        assert!(!move_block_allowed(5, 1, 2, 3));
        assert!(!move_block_allowed(5, 2, 1, -3));
    }
}
