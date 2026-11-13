// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Playback policy: `PlaybackCore` queue/transport semantics.
//!
//! The engine itself needs a live PipeWire session, so only the pure policy is
//! covered here: every test drives the core with explicit monotonic
//! timestamps, which is exactly how the daemon's playback service uses it.

use crate::ipc_protocol::{
    QueueDelta, QueueOp, STATE_FLAG_GAPLESS_ARMED, STATE_FLAG_LOADING, STATE_FLAG_MUTED,
};
use crate::player::core::{CoreCommand, Outcome, PlaybackCore};
use crate::player::engine::{AudioCommand, EngineEvent};
use crate::player::{Normalization, Repeat, State, Track};

const SECOND: u64 = 1_000_000_000;

fn track(id: &str, duration: f64) -> Track {
    Track {
        item: crate::model::ItemMeta {
            id: id.to_string(),
            title: format!("Song {id}"),
            ..crate::model::ItemMeta::default()
        },
        artist: String::new(),
        album: String::new(),
        favorite: false,
        duration,
        url: format!("file:///{id}.flac"),
        cover_art: String::new(),
    }
}

fn tracks(count: usize) -> Vec<Track> {
    (0..count)
        .map(|index| track(&index.to_string(), 100.0))
        .collect()
}

fn ids(core: &PlaybackCore) -> Vec<String> {
    core.queue().iter().map(|track| track.id.clone()).collect()
}

/// Loads `count` tracks and clears the initial `loading` flag, which is what
/// the engine does through `LoadingCleared`/`FirstPcm`.
fn playing_core(count: usize) -> PlaybackCore {
    let mut core = PlaybackCore::new();
    core.load(tracks(count), 0, false);
    core.on_engine(EngineEvent::LoadingCleared, 0);
    core
}

fn only_load(outcome: &Outcome) -> &AudioCommand {
    assert_eq!(outcome.audio.len(), 1, "{:?}", outcome.audio);
    &outcome.audio[0]
}

fn load_url(outcome: &Outcome) -> (&str, Option<&str>, bool) {
    let command = only_load(outcome);
    let AudioCommand::Load {
        url,
        next_url,
        paused,
        ..
    } = command
    else {
        panic!("expected a load, got {command:?}");
    };
    (url.as_str(), next_url.as_deref(), *paused)
}

// --- load ----------------------------------------------------------------

#[test]
fn load_starts_playing_and_resets_repeat() {
    let mut core = PlaybackCore::new();
    core.apply(CoreCommand::CycleRepeat);
    assert_eq!(core.repeat(), Repeat::One);

    let outcome = core.load(tracks(3), 0, false);

    assert_eq!(core.state(), State::Playing);
    assert_eq!(core.index(), 0);
    assert_eq!(core.repeat(), Repeat::Off);
    assert_eq!(core.revision(), 1);
    assert!(outcome.queue_reset);
    assert!(outcome.changed);
    assert_eq!(
        load_url(&outcome),
        ("file:///0.flac", Some("file:///1.flac"), false)
    );
}

#[test]
fn normalization_and_crossfade_commands_reach_the_engine_and_snapshot() {
    let mut core = PlaybackCore::new();
    assert_eq!(core.normalization(), Normalization::Off);
    assert!(!core.crossfade());

    let outcome = core.apply(CoreCommand::CycleNormalization);
    assert_eq!(core.normalization(), Normalization::Track);
    assert!(
        outcome
            .audio
            .contains(&AudioCommand::SetNormalization(Normalization::Track))
    );
    assert!(outcome.changed);
    let snapshot = core.snapshot(1, 0);
    assert_eq!(snapshot.normalization_code(), 1);
    assert!(!snapshot.crossfade());

    let outcome = core.apply(CoreCommand::CycleNormalization);
    assert_eq!(core.normalization(), Normalization::Album);
    assert!(outcome.changed);
    assert_eq!(core.snapshot(2, 0).normalization_code(), 2);

    let outcome = core.apply(CoreCommand::ToggleCrossfade);
    assert!(core.crossfade());
    assert!(
        outcome
            .audio
            .contains(&AudioCommand::SetCrossfade(true))
    );
    let snapshot = core.snapshot(3, 0);
    assert!(snapshot.crossfade());
    assert_eq!(snapshot.normalization_code(), 2);

    let sync = core.audio_sync();
    assert!(sync.contains(&AudioCommand::SetNormalization(Normalization::Album)));
    assert!(sync.contains(&AudioCommand::SetCrossfade(true)));
}

#[test]
fn load_clamps_start_and_arms_the_gapless_row() {
    let mut core = PlaybackCore::new();
    let outcome = core.load(tracks(3), 99, false);
    assert_eq!(core.index(), 2, "start is clamped to the last row");
    assert_eq!(load_url(&outcome), ("file:///2.flac", None, false));
    assert!(
        outcome
            .audio
            .iter()
            .all(|command| !matches!(command, AudioCommand::Seek(_))),
        "a load never seeks"
    );
}

#[test]
fn load_empty_queue_stops() {
    let mut core = playing_core(3);
    let outcome = core.load(Vec::new(), 0, false);
    assert_eq!(core.state(), State::Stopped);
    assert_eq!(ids(&core), Vec::<String>::new());
    assert!(outcome.queue_reset);
    assert_eq!(outcome.audio, vec![AudioCommand::Stop]);
}

#[test]
fn load_paused_keeps_the_transport_paused() {
    let mut core = PlaybackCore::new();
    let outcome = core.load(tracks(2), 1, true);
    assert_eq!(core.state(), State::Paused);
    assert_eq!(load_url(&outcome), ("file:///1.flac", None, true));
    let snapshot = core.snapshot(1, SECOND);
    assert_eq!(snapshot.state, 2);
    // A paused load never starts audio, so the transport must read "Paused"
    // right away instead of spinning: this is the app-open restore path.
    assert_eq!(
        snapshot.flags & STATE_FLAG_LOADING,
        0,
        "a paused load is not loading"
    );
}

#[test]
fn load_playing_reports_loading_until_the_first_pcm() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(1), 0, false);
    assert_eq!(core.state(), State::Playing);
    let snapshot = core.snapshot(1, SECOND);
    assert_ne!(snapshot.flags & STATE_FLAG_LOADING, 0);
}

#[test]
fn gapless_next_follows_the_repeat_mode() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(3), 0, false);
    assert_eq!(core.next_url().as_deref(), Some("file:///1.flac"));

    // repeat One repeats the current row.
    core.apply(CoreCommand::CycleRepeat);
    assert_eq!(core.repeat(), Repeat::One);
    assert_eq!(core.next_url().as_deref(), Some("file:///0.flac"));

    // repeat All wraps at the end, repeat Off does not.
    core.apply(CoreCommand::Jump(2));
    assert_eq!(core.index(), 2);
    assert_eq!(core.next_url(), None);
    core.apply(CoreCommand::CycleRepeat); // One
    core.apply(CoreCommand::CycleRepeat); // All
    assert_eq!(core.repeat(), Repeat::All);
    assert_eq!(core.next_url().as_deref(), Some("file:///0.flac"));
}

// --- transport -----------------------------------------------------------

#[test]
fn play_pause_resume_stop_round_trip() {
    let mut core = playing_core(2);

    let paused = core.apply(CoreCommand::PlayPause);
    assert_eq!(core.state(), State::Paused);
    assert_eq!(paused.audio, vec![AudioCommand::Pause]);

    let resumed = core.apply(CoreCommand::PlayPause);
    assert_eq!(core.state(), State::Playing);
    assert_eq!(resumed.audio, vec![AudioCommand::Resume]);

    let stopped = core.apply(CoreCommand::Stop);
    assert_eq!(core.state(), State::Stopped);
    assert_eq!(stopped.audio, vec![AudioCommand::Stop]);
    assert_eq!(core.queue().len(), 2, "stop keeps the queue rows");

    // A second stop is a no-op (no audio churn).
    assert!(core.apply(CoreCommand::Stop).audio.is_empty());
}

#[test]
fn play_from_stopped_reloads_the_current_row() {
    let mut core = playing_core(2);
    core.apply(CoreCommand::Stop);
    let outcome = core.apply(CoreCommand::Play);
    assert_eq!(core.state(), State::Playing);
    assert_eq!(
        load_url(&outcome),
        ("file:///0.flac", Some("file:///1.flac"), false)
    );
}

#[test]
fn pause_freezes_and_resume_rebases_the_playhead() {
    let mut core = playing_core(1);
    // `playing_core` re-anchors the playhead at `now_ns == 0`.
    let t0 = 0;
    core.snapshot(0, t0);

    let outcome = core.apply_at(CoreCommand::Pause, t0 + 2 * SECOND);
    assert_eq!(outcome.audio, vec![AudioCommand::Pause]);
    assert!((core.position_at(t0 + 2 * SECOND) - 2.0).abs() < 1e-6);
    assert!(
        (core.position_at(t0 + 9 * SECOND) - 2.0).abs() < 1e-6,
        "paused position is frozen"
    );

    let outcome = core.apply_at(CoreCommand::PlayPause, t0 + 9 * SECOND);
    assert_eq!(outcome.audio, vec![AudioCommand::Resume]);
    core.snapshot(1, t0 + 9 * SECOND);
    assert!((core.position_at(t0 + 9 * SECOND) - 2.0).abs() < 1e-6);
    assert!((core.position_at(t0 + 10 * SECOND) - 3.0).abs() < 1e-6);
}

#[test]
fn next_and_previous_load_at_most_once() {
    let mut core = playing_core(5);
    let outcome = core.apply(CoreCommand::Next);
    assert_eq!(core.index(), 1);
    assert_eq!(outcome.audio.len(), 1, "one load per skip");
    assert_eq!(
        load_url(&outcome),
        ("file:///1.flac", Some("file:///2.flac"), false)
    );

    let outcome = core.apply(CoreCommand::Previous);
    assert_eq!(core.index(), 0);
    assert_eq!(outcome.audio.len(), 1);

    // At the head, previous is a no-op.
    assert!(core.apply(CoreCommand::Previous).audio.is_empty());
    assert_eq!(core.index(), 0);
}

#[test]
fn next_at_the_end_stops_without_repeat_all() {
    let mut core = playing_core(2);
    core.apply(CoreCommand::Jump(1));
    let outcome = core.apply(CoreCommand::Next);
    assert_eq!(core.state(), State::Stopped);
    assert_eq!(core.index(), 1, "stop keeps the playhead");
    assert_eq!(outcome.audio, vec![AudioCommand::Stop]);
}

#[test]
fn next_at_the_end_wraps_with_repeat_all() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(2), 1, false);
    core.apply(CoreCommand::CycleRepeat); // One
    core.apply(CoreCommand::CycleRepeat); // All
    assert_eq!(core.repeat(), Repeat::All);
    let outcome = core.apply(CoreCommand::Next);
    assert_eq!(core.index(), 0);
    assert_eq!(outcome.audio.len(), 1, "one load for the wrapped row");
    assert_eq!(core.state(), State::Playing);
}

// --- engine events -------------------------------------------------------

#[test]
fn ended_advances_the_queue() {
    let mut core = playing_core(3);
    let outcome = core.on_engine(EngineEvent::Ended, SECOND);
    assert_eq!(core.index(), 1);
    assert_eq!(
        load_url(&outcome),
        ("file:///1.flac", Some("file:///2.flac"), false)
    );
}

#[test]
fn ended_with_repeat_one_replays_and_repeat_all_wraps() {
    // repeat One replays the same row.
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(2), 1, false);
    core.apply(CoreCommand::CycleRepeat); // One
    assert_eq!(core.repeat(), Repeat::One);
    let outcome = core.on_engine(EngineEvent::Ended, SECOND);
    assert_eq!(core.index(), 1);
    assert_eq!(core.state(), State::Playing);
    let command = only_load(&outcome);
    let AudioCommand::Load {
        url,
        next_url,
        paused,
        ..
    } = command
    else {
        panic!("expected a load, got {command:?}");
    };
    assert_eq!(url, "file:///1.flac");
    // repeat One preloads the same row again.
    assert_eq!(next_url.as_deref(), Some("file:///1.flac"));
    assert!(!paused);

    // repeat All wraps to the head.
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(2), 1, false);
    core.apply(CoreCommand::CycleRepeat);
    core.apply(CoreCommand::CycleRepeat); // All
    let outcome = core.on_engine(EngineEvent::Ended, SECOND);
    assert_eq!(core.index(), 0);
    assert_eq!(core.state(), State::Playing);
    assert_eq!(
        load_url(&outcome),
        ("file:///0.flac", Some("file:///1.flac"), false)
    );

    // repeat Off at the end of the queue stops.
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(2), 1, false);
    assert_eq!(core.repeat(), Repeat::Off);
    let outcome = core.on_engine(EngineEvent::Ended, SECOND);
    assert_eq!(core.state(), State::Stopped);
    assert_eq!(outcome.audio, vec![AudioCommand::Stop]);
}

#[test]
fn failed_advances_and_keeps_the_error() {
    let mut core = playing_core(3);
    let outcome = core.on_engine(
        EngineEvent::Failed {
            message: "decoder: broken".to_string(),
        },
        SECOND,
    );
    assert_eq!(core.index(), 1);
    assert_eq!(outcome.audio.len(), 1);
    let error = core.last_error().expect("error stored");
    assert!(error.contains("decoder: broken"), "{error}");
    assert!(
        error.contains("Song 0"),
        "the old player prefixed the title: {error}"
    );
}

#[test]
fn boundary_advances_into_the_gapless_row() {
    let mut core = playing_core(3);
    let outcome = core.on_engine(
        EngineEvent::Boundary {
            duration: 42.5,
            stream_size: 1_000,
            position: 0.0,
        },
        5 * SECOND,
    );
    assert_eq!(core.index(), 1);
    assert!(
        outcome.audio.is_empty(),
        "the worker already swapped decoders"
    );
    assert_eq!(core.state(), State::Playing);
    assert!((core.duration() - 42.5).abs() < 1e-9);
    assert!(
        (core.position_at(5 * SECOND)).abs() < 1e-9,
        "the new track starts at 0"
    );
    assert!((core.position_at(6 * SECOND) - 1.0).abs() < 1e-9);
}

#[test]
fn boundary_reports_the_crossfade_offset_as_the_new_position() {
    let mut core = playing_core(3);
    let outcome = core.on_engine(
        EngineEvent::Boundary {
            duration: 40.0,
            stream_size: 1_000,
            position: 4.5,
        },
        5 * SECOND,
    );
    assert_eq!(core.index(), 1);
    assert!(outcome.audio.is_empty());
    assert!(
        (core.position_at(5 * SECOND) - 4.5).abs() < 1e-9,
        "the next row begins at the crossfade offset"
    );
    assert!((core.position_at(6 * SECOND) - 5.5).abs() < 1e-9);
}

#[test]
fn boundary_while_paused_keeps_the_transport_paused() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(2), 0, true);
    core.on_engine(
        EngineEvent::Boundary {
            duration: 10.0,
            stream_size: 0,
            position: 0.0,
        },
        SECOND,
    );
    assert_eq!(core.index(), 1);
    assert_eq!(core.state(), State::Paused);
}

#[test]
fn sink_failed_freezes_without_advancing() {
    let mut core = playing_core(3);
    let t0 = SECOND;
    core.snapshot(0, t0);
    let outcome = core.on_engine(
        EngineEvent::SinkFailed {
            message: "PipeWire stream stopped".to_string(),
        },
        t0 + SECOND,
    );
    assert_eq!(core.index(), 0, "a sink failure must not advance the queue");
    assert!(outcome.audio.is_empty());
    assert!(outcome.changed);
    let frozen = core.position_at(t0 + SECOND);
    assert!((core.position_at(t0 + 5 * SECOND) - frozen).abs() < 1e-6);
    assert_eq!(core.last_error(), Some("PipeWire stream stopped"));
}

#[test]
fn loading_cleared_and_loaded_update_the_snapshot() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(2), 0, false);
    assert_ne!(core.snapshot(0, 0).flags & STATE_FLAG_LOADING, 0);

    let outcome = core.on_engine(
        EngineEvent::Loaded {
            duration: 123.0,
            stream_size: 4_000,
            seekable: true,
        },
        SECOND,
    );
    assert!(outcome.changed);
    let cleared = core.on_engine(EngineEvent::LoadingCleared, SECOND);
    assert!(cleared.changed);
    let snapshot = core.snapshot(1, SECOND);
    assert_eq!(snapshot.flags & STATE_FLAG_LOADING, 0);
    assert_eq!(snapshot.duration_ms, 123_000);
}

#[test]
fn first_pcm_clears_loading() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(1), 0, false);
    let outcome = core.on_engine(EngineEvent::FirstPcm, SECOND);
    assert!(outcome.changed);
    assert_eq!(core.snapshot(0, SECOND).flags & STATE_FLAG_LOADING, 0);
}

// --- row mutations -------------------------------------------------------

#[test]
fn removing_the_current_row_plays_the_replacement() {
    let mut core = playing_core(3);
    core.apply(CoreCommand::Jump(1));
    let outcome = core.apply(CoreCommand::Remove(1));
    assert_eq!(ids(&core), vec!["0", "2"]);
    assert_eq!(core.index(), 1);
    assert_eq!(load_url(&outcome), ("file:///2.flac", None, false));
    assert_eq!(
        outcome.delta,
        Some(QueueDelta {
            rev: 2,
            op: QueueOp::Remove { indices: vec![1] },
        })
    );
}

#[test]
fn removing_the_last_row_stops() {
    let mut core = playing_core(1);
    let outcome = core.apply(CoreCommand::Remove(0));
    assert_eq!(core.state(), State::Stopped);
    assert_eq!(core.queue().len(), 0);
    assert_eq!(outcome.audio, vec![AudioCommand::Stop]);
    assert!(outcome.delta.is_some());
}

#[test]
fn removing_above_the_current_row_shifts_the_cursor_without_reloading() {
    let mut core = playing_core(3);
    core.apply(CoreCommand::Jump(2));
    let outcome = core.apply(CoreCommand::Remove(0));
    assert_eq!(core.index(), 1);
    assert_eq!(ids(&core), vec!["1", "2"]);
    assert!(outcome.audio.is_empty(), "the playing row did not change");
    assert_eq!(core.state(), State::Playing);
}

#[test]
fn remove_many_matches_the_old_playhead_rules() {
    let mut core = playing_core(5);
    core.apply(CoreCommand::Jump(3));
    let outcome = core.apply(CoreCommand::RemoveMany(vec![0, 4]));
    assert_eq!(ids(&core), vec!["1", "2", "3"]);
    assert_eq!(core.index(), 2);
    assert!(outcome.audio.is_empty());
    assert_eq!(
        outcome.delta,
        Some(QueueDelta {
            rev: 2,
            op: QueueOp::Remove {
                indices: vec![0, 4]
            },
        })
    );

    // Removing the current row from the same batch reloads the replacement.
    let outcome = core.apply(CoreCommand::RemoveMany(vec![2]));
    assert_eq!(ids(&core), vec!["1", "2"]);
    assert_eq!(core.index(), 1);
    assert_eq!(load_url(&outcome), ("file:///2.flac", None, false));
}

#[test]
fn moving_rows_tracks_the_playhead_without_reloading() {
    let mut core = playing_core(10);
    core.apply(CoreCommand::Jump(5));
    let outcome = core.apply(CoreCommand::Move { from: 0, to: 7 });
    assert_eq!(core.index(), 4);
    assert!(outcome.audio.is_empty());
    assert_eq!(
        outcome.delta,
        Some(QueueDelta {
            rev: 2,
            op: QueueOp::Move { from: 0, to: 7 },
        })
    );

    // Moving the current row itself moves the cursor with it.
    let outcome = core.apply(CoreCommand::Move { from: 4, to: 9 });
    assert_eq!(core.index(), 9);
    assert!(outcome.audio.is_empty());
}

#[test]
fn block_move_preserves_order_and_the_playhead() {
    let mut core = playing_core(6);
    core.apply(CoreCommand::Jump(1));
    let outcome = core.apply(CoreCommand::MoveBlock {
        start: 0,
        len: 3,
        delta: 1,
    });
    let order: Vec<String> = ids(&core);
    assert_eq!(order, vec!["3", "0", "1", "2", "4", "5"]);
    assert_eq!(core.index(), 2, "the playing row moves with its block");
    assert!(outcome.queue_reset, "a multi-row move re-streams the rows");
    assert!(outcome.audio.is_empty());

    // A single-row block move is an ordinary queue Move.
    let outcome = core.apply(CoreCommand::MoveBlock {
        start: 5,
        len: 1,
        delta: -1,
    });
    assert_eq!(
        outcome.delta,
        Some(QueueDelta {
            rev: 3,
            op: QueueOp::Move { from: 5, to: 4 },
        })
    );

    // Boundary moves are no-ops.
    let outcome = core.apply(CoreCommand::MoveBlock {
        start: 0,
        len: 2,
        delta: -1,
    });
    assert!(!outcome.changed);
    assert_eq!(outcome.delta, None);
}

#[test]
fn inserting_above_the_current_row_shifts_the_cursor_without_reloading() {
    let mut core = playing_core(10);
    core.apply(CoreCommand::Jump(5));
    let outcome = core.apply(CoreCommand::InsertMany {
        at: 0,
        tracks: vec![track("a", 1.0), track("b", 1.0)],
    });
    assert_eq!(core.index(), 7);
    assert!(core.state() == State::Playing);
    assert!(outcome.audio.is_empty(), "insert leaves the decoder alone");
    match outcome.delta.as_ref().map(|delta| &delta.op) {
        Some(QueueOp::Insert { at, rows }) => {
            assert_eq!(*at, 0);
            assert_eq!(
                rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
                ["a", "b"]
            );
        }
        other => panic!("expected an insert delta, got {other:?}"),
    }
}

#[test]
fn appending_rows_keeps_the_playhead_and_emits_an_insert() {
    let mut core = playing_core(3);
    let outcome = core.apply(CoreCommand::Append(vec![track("z", 1.0)]));
    assert_eq!(core.index(), 0);
    assert_eq!(core.queue().len(), 4);
    assert!(outcome.audio.is_empty());
    match outcome.delta.as_ref().map(|delta| &delta.op) {
        Some(QueueOp::Insert { at, rows }) => {
            assert_eq!(*at, 3);
            assert_eq!(rows[0].id, "z");
        }
        other => panic!("expected an insert delta, got {other:?}"),
    }
}

#[test]
fn inserting_into_an_empty_queue_selects_the_head() {
    let mut core = PlaybackCore::new();
    let outcome = core.apply(CoreCommand::InsertMany {
        at: 0,
        tracks: tracks(2),
    });
    assert_eq!(core.index(), 0);
    assert_eq!(core.state(), State::Stopped, "insert never autoplays");
    assert!(outcome.audio.is_empty());
}

#[test]
fn large_inserts_fall_back_to_a_row_reset() {
    let mut core = playing_core(1);
    let many: Vec<Track> = (0..=crate::ipc_protocol::QUEUE_INSERT_MAX_ROWS)
        .map(|index| track(&format!("extra-{index}"), 1.0))
        .collect();
    let outcome = core.apply(CoreCommand::Append(many));
    assert!(outcome.queue_reset);
    assert_eq!(outcome.delta, None);
}

#[test]
fn set_track_favorite_patches_the_matching_row() {
    let mut core = playing_core(3);
    let outcome = core.apply(CoreCommand::SetTrackFavorite {
        id: "1".to_string(),
        favorite: true,
    });
    assert!(core.queue()[1].favorite);
    assert_eq!(
        outcome.delta,
        Some(QueueDelta {
            rev: 2,
            op: QueueOp::Patch {
                index: 1,
                favorite: true,
            },
        })
    );

    // An unknown id changes nothing.
    let outcome = core.apply(CoreCommand::SetTrackFavorite {
        id: "nope".to_string(),
        favorite: true,
    });
    assert!(!outcome.changed);
    assert_eq!(outcome.delta, None);
}

#[test]
fn shuffle_permutes_and_restarts_the_queue() {
    let mut core = PlaybackCore::new();
    _ = core.load(tracks(32), 0, false);
    core.apply(CoreCommand::CycleRepeat);
    core.apply(CoreCommand::CycleRepeat); // All
    core.apply(CoreCommand::Jump(17));
    core.apply(CoreCommand::CycleRepeat);
    core.apply(CoreCommand::CycleRepeat); // All again after the jump resets it
    let before = ids(&core);

    let outcome = core.apply(CoreCommand::ShuffleQueue);
    let after = ids(&core);
    let mut sorted_before = before.clone();
    sorted_before.sort();
    let mut sorted_after = after.clone();
    sorted_after.sort();
    assert_eq!(sorted_before, sorted_after, "every row survives once");
    assert_ne!(before, after, "the order really changed (32! odds)");
    assert_eq!(core.index(), 0);
    assert_eq!(core.repeat(), Repeat::Off);
    assert!(outcome.queue_reset);
    assert_eq!(outcome.audio.len(), 1, "the shuffled head is loaded once");
    assert_eq!(core.state(), State::Playing);
}

#[test]
fn shuffle_of_an_empty_queue_is_a_no_op() {
    let mut core = PlaybackCore::new();
    let outcome = core.apply(CoreCommand::ShuffleQueue);
    assert!(!outcome.changed);
    assert!(outcome.audio.is_empty());
}

// --- seek / volume -------------------------------------------------------

#[test]
fn seek_clamps_and_rebases_the_anchor() {
    let mut core = playing_core(1);
    let t0 = SECOND;
    core.snapshot(0, t0);

    let outcome = core.apply_at(CoreCommand::Seek(30.0), t0);
    assert_eq!(outcome.audio, vec![AudioCommand::Seek(30.0)]);
    assert!((core.position_at(t0) - 30.0).abs() < 1e-6);
    core.snapshot(1, t0);
    assert!((core.position_at(t0 + SECOND) - 31.0).abs() < 1e-6);

    let outcome = core.apply_at(CoreCommand::Seek(-10.0), t0);
    assert_eq!(outcome.audio, vec![AudioCommand::Seek(0.0)]);
    assert!((core.position_at(t0)).abs() < 1e-6);
}

#[test]
fn seek_relative_uses_the_interpolated_position() {
    let mut core = playing_core(1);
    // `playing_core` re-anchors the playhead at `now_ns == 0`.
    let t0 = 0;
    core.snapshot(0, t0);
    let outcome = core.apply_at(CoreCommand::SeekRelative(5.0), t0 + 3 * SECOND);
    let command = only_load(&outcome);
    let AudioCommand::Seek(seconds) = command else {
        panic!("expected a seek, got {command:?}");
    };
    assert!((seconds - 8.0).abs() < 1e-6, "{seconds}");
}

#[test]
fn seek_while_stopped_only_moves_the_displayed_clock() {
    let mut core = playing_core(2);
    core.apply(CoreCommand::Stop);
    let outcome = core.apply(CoreCommand::Seek(42.0));
    assert!(
        outcome.audio.is_empty(),
        "a stopped transport has nothing to seek"
    );
    assert!((core.position_at(0) - 42.0).abs() < 1e-6);
    // Playing again starts the row from the beginning, like the old player.
    let outcome = core.apply(CoreCommand::Play);
    assert_eq!(
        load_url(&outcome),
        ("file:///0.flac", Some("file:///1.flac"), false)
    );
    assert!(core.position_at(0).abs() < 1e-6);
}

#[test]
fn seek_is_clamped_to_a_known_duration() {
    let mut core = playing_core(1);
    core.on_engine(
        EngineEvent::Loaded {
            duration: 60.0,
            stream_size: 0,
            seekable: true,
        },
        SECOND,
    );
    core.snapshot(0, SECOND);
    let outcome = core.apply_at(CoreCommand::Seek(1_000.0), SECOND);
    assert_eq!(outcome.audio, vec![AudioCommand::Seek(60.0)]);
}

#[test]
fn volume_is_clamped_to_the_ui_scale() {
    let mut core = playing_core(1);
    let outcome = core.apply(CoreCommand::SetVolume(250.0));
    assert_eq!(core.volume(), 100.0);
    assert_eq!(outcome.audio, vec![AudioCommand::SetVolume(100.0)]);
    assert_eq!(core.snapshot(0, 0).volume_milli, 1000);

    let outcome = core.apply(CoreCommand::SetVolume(-4.0));
    assert_eq!(core.volume(), 0.0);
    assert_eq!(outcome.audio, vec![AudioCommand::SetVolume(0.0)]);

    core.apply(CoreCommand::SetVolume(50.0));
    let outcome = core.apply(CoreCommand::VolumeDown);
    assert_eq!(core.volume(), 45.0);
    assert_eq!(outcome.audio, vec![AudioCommand::SetVolume(45.0)]);
    core.apply(CoreCommand::VolumeUp);
    assert_eq!(core.volume(), 50.0);
}

#[test]
fn mute_is_reflected_in_the_snapshot_flags() {
    let mut core = playing_core(1);
    let outcome = core.apply(CoreCommand::ToggleMute);
    assert_eq!(outcome.audio, vec![AudioCommand::SetMuted(true)]);
    assert!(core.muted());
    assert_ne!(core.snapshot(0, 0).flags & STATE_FLAG_MUTED, 0);
    core.apply(CoreCommand::SetMuted(false));
    assert_eq!(core.snapshot(0, 0).flags & STATE_FLAG_MUTED, 0);
}

// --- snapshot ------------------------------------------------------------

#[test]
fn snapshot_encodes_the_transport() {
    let mut core = PlaybackCore::new();
    core.apply(CoreCommand::SetVolume(73.0));
    // Load paused at the head of two rows: the second row is preloaded.
    _ = core.load(tracks(2), 0, true);
    assert_eq!(core.repeat(), Repeat::Off);
    core.apply(CoreCommand::CycleRepeat); // One
    let now = 7 * SECOND;
    let snapshot = core.snapshot(9, now);
    assert_eq!(snapshot.seq, 9);
    assert_eq!(snapshot.queue_rev, core.revision());
    assert_eq!(snapshot.anchor_ns, now);
    assert_eq!(snapshot.index, 0);
    assert_eq!(snapshot.state, 2);
    assert_eq!(snapshot.repeat, 1);
    assert_eq!(snapshot.volume_milli, 730);
    assert_eq!(snapshot.rate_milli, 1000);
    assert_eq!(snapshot.duration_ms, 100_000);
    assert_eq!(snapshot.position_ms, 0);
    assert_eq!(snapshot.reserved, 0);
    // Loaded paused: no spinner, and the following row is preloaded.
    assert_eq!(snapshot.flags & STATE_FLAG_LOADING, 0);
    assert_ne!(snapshot.flags & STATE_FLAG_GAPLESS_ARMED, 0);
    // Round trip through the wire codec.
    let decoded = crate::ipc_protocol::PlaybackState::decode(&snapshot.encode()).expect("decode");
    assert_eq!(decoded, snapshot);
}

#[test]
fn queue_reset_is_reported_by_load_and_shuffle() {
    let mut core = PlaybackCore::new();
    assert!(core.load(tracks(2), 0, false).queue_reset);
    assert!(core.apply(CoreCommand::ShuffleQueue).queue_reset);
    // Plain transport and row edits do not reset the queue.
    assert!(!core.apply(CoreCommand::Next).queue_reset);
    assert!(!core.apply(CoreCommand::Remove(0)).queue_reset);
}

// --- engine rebuild ------------------------------------------------------

#[test]
fn recover_replays_the_current_row_at_the_last_playhead() {
    let mut core = playing_core(3);
    core.apply(CoreCommand::Jump(1));
    let position = 33.5;
    let outcome = core.apply(CoreCommand::Recover { position });
    assert_eq!(core.state(), State::Playing);
    let command = &outcome.audio[0];
    let AudioCommand::Load {
        url,
        next_url,
        paused,
        generation,
    } = command
    else {
        panic!("expected a load, got {command:?}");
    };
    assert_eq!(url, "file:///1.flac");
    assert_eq!(next_url.as_deref(), Some("file:///2.flac"));
    assert!(!paused);
    assert!(*generation > 0, "a recovery uses a fresh generation");
    assert_eq!(outcome.audio[1], AudioCommand::Seek(position));
    assert!((core.position_at(SECOND) - position).abs() < 1e-6);
    assert!(outcome.changed);
}

#[test]
fn recover_preserves_a_paused_transport() {
    let mut core = playing_core(2);
    core.apply(CoreCommand::Pause);
    let outcome = core.apply(CoreCommand::Recover { position: 12.0 });
    let command = &outcome.audio[0];
    let AudioCommand::Load { paused, .. } = command else {
        panic!("expected a load, got {command:?}");
    };
    assert!(*paused);
    assert_eq!(core.state(), State::Paused);
}

#[test]
fn recover_on_an_idle_core_does_nothing() {
    let mut core = PlaybackCore::new();
    let outcome = core.apply(CoreCommand::Recover { position: 5.0 });
    assert!(outcome.audio.is_empty());
    assert!(!outcome.changed);
}

#[test]
fn every_load_uses_a_new_generation() {
    let mut core = PlaybackCore::new();
    let mut generations = Vec::new();
    for _ in 0..3 {
        let outcome = core.load(tracks(3), 0, false);
        generations.extend(load_generation(&outcome));
        let outcome = core.apply(CoreCommand::Next);
        generations.extend(load_generation(&outcome));
    }
    assert_eq!(generations.len(), 6);
    assert!(
        generations.windows(2).all(|pair| pair[0] < pair[1]),
        "load generations must increase: {generations:?}"
    );
}

fn load_generation(outcome: &Outcome) -> Option<u64> {
    outcome.audio.iter().find_map(|command| {
        let AudioCommand::Load { generation, .. } = command else {
            return None;
        };
        Some(*generation)
    })
}

// --- differential row model ---------------------------------------------

/// A tiny deterministic PRNG (SplitMix64) so the op sequence is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            ((self.next() as u128 * len as u128) >> 64) as usize
        }
    }
}

/// The "simple reference row model": rows, playhead, repeat and revision,
/// written directly from the documented `Player` semantics.
struct Model {
    rows: Vec<String>,
    index: usize,
    repeat: Repeat,
    revision: u64,
    playing: bool,
}

impl Model {
    fn apply(&mut self, op: &Op) {
        match op {
            Op::Jump(index) => {
                if *index < self.rows.len() {
                    self.repeat = Repeat::Off;
                    self.index = *index;
                    self.playing = true;
                }
            }
            Op::Remove(index) => {
                if *index >= self.rows.len() {
                    return;
                }
                let was_current = *index == self.index;
                self.rows.remove(*index);
                self.revision += 1;
                if self.rows.is_empty() {
                    self.index = 0;
                    self.playing = false;
                } else if was_current {
                    self.index = self.index.min(self.rows.len() - 1);
                    self.playing = true;
                } else if *index < self.index {
                    self.index -= 1;
                }
            }
            Op::RemoveMany(indices) => {
                let mut indices: Vec<usize> = indices
                    .iter()
                    .copied()
                    .filter(|index| *index < self.rows.len())
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                if indices.is_empty() {
                    return;
                }
                let old_index = self.index;
                let removed_current = indices.binary_search(&old_index).is_ok();
                let below = indices.partition_point(|index| *index < old_index);
                for index in indices.iter().rev() {
                    self.rows.remove(*index);
                }
                self.revision += 1;
                if self.rows.is_empty() {
                    self.index = 0;
                    self.playing = false;
                } else if removed_current {
                    self.index = old_index.saturating_sub(below).min(self.rows.len() - 1);
                    self.playing = true;
                } else {
                    self.index = old_index - below;
                }
            }
            Op::Move(from, to) => {
                if *from >= self.rows.len() || *to >= self.rows.len() || from == to {
                    return;
                }
                let row = self.rows.remove(*from);
                self.rows.insert(*to, row);
                self.revision += 1;
                if self.index == *from {
                    self.index = *to;
                } else {
                    if *from < self.index {
                        self.index -= 1;
                    }
                    if *to <= self.index {
                        self.index += 1;
                    }
                }
            }
            Op::MoveBlock { start, len, delta } => {
                if *len == 0 || *delta == 0 || *start >= self.rows.len() {
                    return;
                }
                let end = (*start + *len).min(self.rows.len());
                let block_len = end - *start;
                let new_start = *start as i64 + i64::from(*delta);
                if new_start < 0 || new_start as usize + block_len > self.rows.len() {
                    return;
                }
                let new_start = new_start as usize;
                if new_start == *start {
                    return;
                }
                let old_index = self.index;
                let block: Vec<String> = self.rows.drain(*start..end).collect();
                self.rows.splice(new_start..new_start, block);
                self.revision += 1;
                self.index = if old_index >= *start && old_index < end {
                    (old_index as i64 + i64::from(*delta)) as usize
                } else if *delta < 0 && old_index >= new_start && old_index < *start {
                    old_index + block_len
                } else if *delta > 0 && old_index >= end && old_index < new_start + block_len {
                    old_index - block_len
                } else {
                    old_index
                };
            }
            Op::Insert { at, rows } => {
                if rows.is_empty() {
                    return;
                }
                let at = (*at).min(self.rows.len());
                let count = rows.len();
                let was_empty = self.rows.is_empty();
                for (offset, row) in rows.iter().enumerate() {
                    self.rows.insert(at + offset, row.id.clone());
                }
                if was_empty {
                    self.index = 0;
                } else if at <= self.index {
                    self.index += count;
                }
                self.revision += 1;
            }
            Op::Next => {
                if self.rows.is_empty() {
                    return;
                }
                if self.index + 1 < self.rows.len() {
                    self.index += 1;
                    self.playing = true;
                } else if self.repeat == Repeat::All {
                    self.index = 0;
                    self.playing = true;
                } else {
                    self.playing = false;
                }
            }
            Op::Previous => {
                if self.rows.is_empty() {
                    return;
                }
                let index = self.index.saturating_sub(1);
                if index != self.index {
                    self.index = index;
                    self.playing = true;
                }
            }
            Op::PlayPause => {
                if self.rows.is_empty() {
                    return;
                }
                self.playing = !self.playing;
            }
            Op::Stop => self.playing = false,
            Op::CycleRepeat => {
                self.repeat = match self.repeat {
                    Repeat::Off => Repeat::One,
                    Repeat::One => Repeat::All,
                    Repeat::All => Repeat::Off,
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
enum Op {
    Jump(usize),
    Remove(usize),
    RemoveMany(Vec<usize>),
    Move(usize, usize),
    MoveBlock {
        start: usize,
        len: usize,
        delta: i32,
    },
    Insert {
        at: usize,
        rows: Vec<Track>,
    },
    Next,
    Previous,
    PlayPause,
    Stop,
    CycleRepeat,
}

/// Deterministic random operations compared against the reference row model.
#[test]
fn random_row_operations_match_the_reference_model() {
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    let mut core = PlaybackCore::new();
    let mut model = Model {
        rows: Vec::new(),
        index: 0,
        repeat: Repeat::Off,
        revision: 0,
        playing: false,
    };
    let initial: Vec<Track> = tracks(12);
    model.rows = initial.iter().map(|track| track.id.clone()).collect();
    _ = core.load(initial, 0, false);
    model.playing = true;
    model.revision = core.revision();
    core.on_engine(EngineEvent::LoadingCleared, 0);
    model.index = 0;
    model.repeat = Repeat::Off;

    for step in 0..400 {
        let len = model.rows.len();
        let op = match rng.next() % 11 {
            0 => Op::Jump(rng.below(len + 2)),
            1 => Op::Remove(rng.below(len + 2)),
            2 => {
                let count = 1 + rng.below(3);
                let indices = (0..count).map(|_| rng.below(len + 2)).collect();
                Op::RemoveMany(indices)
            }
            3 => Op::Move(rng.below(len + 1), rng.below(len + 1)),
            4 => Op::MoveBlock {
                start: rng.below(len + 1),
                len: rng.below(4),
                delta: (rng.below(5) as i32) - 2,
            },
            5 => Op::Insert {
                at: rng.below(len + 2),
                rows: (0..1 + rng.below(2))
                    .map(|index| track(&format!("ins-{step}-{index}"), 10.0))
                    .collect(),
            },
            6 => Op::Next,
            7 => Op::Previous,
            8 => Op::PlayPause,
            9 => Op::Stop,
            _ => Op::CycleRepeat,
        };
        let command = match &op {
            Op::Jump(index) => CoreCommand::Jump(*index),
            Op::Remove(index) => CoreCommand::Remove(*index),
            Op::RemoveMany(indices) => CoreCommand::RemoveMany(indices.clone()),
            Op::Move(from, to) => CoreCommand::Move {
                from: *from,
                to: *to,
            },
            Op::MoveBlock { start, len, delta } => CoreCommand::MoveBlock {
                start: *start,
                len: *len,
                delta: *delta,
            },
            Op::Insert { at, rows } => CoreCommand::InsertMany {
                at: *at,
                tracks: rows.clone(),
            },
            Op::Next => CoreCommand::Next,
            Op::Previous => CoreCommand::Previous,
            Op::PlayPause => CoreCommand::PlayPause,
            Op::Stop => CoreCommand::Stop,
            Op::CycleRepeat => CoreCommand::CycleRepeat,
        };
        core.apply(command);
        model.apply(&op);

        assert_eq!(
            ids(&core),
            model.rows,
            "rows diverged at step {step}: {op:?}"
        );
        assert_eq!(
            core.revision(),
            model.revision,
            "revision diverged at step {step}"
        );
        assert_eq!(
            core.repeat(),
            model.repeat,
            "repeat diverged at step {step}"
        );
        if !model.rows.is_empty() {
            assert_eq!(
                core.index(),
                model.index,
                "index diverged at step {step}: {op:?}"
            );
            assert!(core.index() < model.rows.len());
        }
    }
}

/// Queue deltas must always describe the row change the service broadcasts.
#[test]
fn row_deltas_match_the_mutated_queue() {
    let mut core = playing_core(8);
    core.apply(CoreCommand::Jump(4));

    let outcome = core.apply(CoreCommand::Remove(2));
    match outcome.delta.map(|delta| delta.op) {
        Some(QueueOp::Remove { indices }) => {
            let mut before = vec!["0", "1", "2", "3", "4", "5", "6", "7"];
            for index in indices.iter().rev() {
                before.remove(*index as usize);
            }
            assert_eq!(ids(&core), before);
        }
        other => panic!("expected a remove delta, got {other:?}"),
    }

    let outcome = core.apply(CoreCommand::InsertMany {
        at: 1,
        tracks: vec![track("x", 1.0)],
    });
    match outcome.delta.map(|delta| delta.op) {
        Some(QueueOp::Insert { at, rows }) => {
            assert_eq!(at, 1);
            assert_eq!(ids(&core)[1], "x");
            assert_eq!(rows.len(), 1);
        }
        other => panic!("expected an insert delta, got {other:?}"),
    }
}

// --- live engine (needs PipeWire, run explicitly with --ignored) ---------

fn wav_bytes_for(frames: u32) -> Vec<u8> {
    let data_len = frames * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&48_000u32.to_le_bytes());
    out.extend_from_slice(&(48_000u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for index in 0..frames {
        let value = (((index as f32 / frames as f32) * 20_000.0) as i16).to_le_bytes();
        out.extend_from_slice(&value);
    }
    out
}

fn wav_url(name: &str, frames: u32) -> (std::path::PathBuf, String) {
    let path = std::env::temp_dir().join(format!(
        "musika-engine-{name}-{}-{}.wav",
        std::process::id(),
        crate::consts::now_nanos()
    ));
    std::fs::write(&path, wav_bytes_for(frames)).expect("write wav");
    let url = format!("file://{}", path.display());
    (path, url)
}

fn drain_events(
    rx: &std::sync::mpsc::Receiver<EngineEvent>,
    until: fn(&EngineEvent) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(event) => {
                if until(&event) {
                    return true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}

#[test]
#[ignore = "requires a running PipeWire session"]
fn engine_crossfade_reports_the_next_track_offset() {
    use crate::player::engine::Engine;
    let (first_path, first_url) = wav_url("crossfade-a", 48_000 * 7);
    let (second_path, second_url) = wav_url("crossfade-b", 48_000 * 10);
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let engine = Engine::new(event_tx).expect("PipeWire engine");
    engine.send(AudioCommand::SetCrossfade(true));
    engine.send(AudioCommand::Load {
        url: first_url,
        next_url: Some(second_url),
        paused: false,
        generation: 1,
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut boundary = None;
    while std::time::Instant::now() < deadline {
        match event_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(EngineEvent::Boundary {
                position,
                duration,
                ..
            }) => {
                boundary = Some((position, duration));
                break;
            }
            Ok(EngineEvent::Failed { message }) => panic!("crossfade failed: {message}"),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let (position, duration) = boundary.expect("crossfade boundary did not arrive");
    assert!(
        position > 1.0,
        "crossfade should start the next row several seconds in, got {position}"
    );
    assert!(
        (duration - 10.0).abs() < 0.5,
        "boundary must describe the next row, got {duration}"
    );
    engine.shutdown();
    _ = std::fs::remove_file(first_path);
    _ = std::fs::remove_file(second_path);
}

#[test]
#[ignore = "requires a running PipeWire session"]
fn engine_pause_resume_keeps_progress_and_shuts_down() {
    use crate::player::engine::Engine;
    let (path, url) = wav_url("pause", 96_000 * 30);
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let engine = Engine::new(event_tx).expect("PipeWire engine");
    engine.send(AudioCommand::Load {
        url,
        next_url: None,
        paused: false,
        generation: 1,
    });
    assert!(
        drain_events(&event_rx, |event| matches!(event, EngineEvent::FirstPcm)),
        "no first PCM"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut played = 0;
    while std::time::Instant::now() < deadline {
        played = engine.clock().played_frames;
        if played > 4_800 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(played > 0, "engine never played");

    engine.send(AudioCommand::Pause);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let paused = engine.clock().played_frames;
    assert!(!engine.clock().loading, "a paused decoder is not loading");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let still = engine.clock().played_frames;
    assert!(
        still.saturating_sub(paused) < 4_800,
        "engine advanced {} frames while paused",
        still.saturating_sub(paused)
    );

    // Seek while paused must be serviced (no deadlock in the decode loop).
    engine.send(AudioCommand::Seek(10.0));
    std::thread::sleep(std::time::Duration::from_millis(100));
    engine.send(AudioCommand::Resume);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut resumed = still;
    while std::time::Instant::now() < deadline {
        resumed = engine.clock().played_frames;
        if resumed > still.saturating_add(4_800) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        resumed > still,
        "resume made no progress ({still} -> {resumed})"
    );

    // Shutdown must join every thread in bounded time.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        engine.shutdown();
        _ = done_tx.send(());
    });
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok(),
        "engine shutdown deadlocked"
    );
    _ = std::fs::remove_file(path);
}

#[test]
#[ignore = "requires a running PipeWire session"]
fn native_can_be_rebuilt_cleanly() {
    use crate::player::pipewire::Native;
    for _ in 0..3 {
        let native = Native::new().expect("PipeWire native");
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(native);
    }
}
