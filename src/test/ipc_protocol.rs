//! Wire protocol v2 codec tests (`src/ipc_protocol.rs`).

use super::common::*;
use crate::ipc_protocol::*;
use std::io::Cursor;

fn wire_track(id: &str, favorite: bool) -> WireTrack {
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

#[test]
fn frame_round_trips_through_a_stream() {
    let frames = vec![
        Frame::new(KIND_PING, Heartbeat { nonce: 7, monotonic_ns: 9 }.encode()),
        Frame::new(KIND_REQUEST_JSON, b"{}".to_vec()),
        Frame::with_flags(
            KIND_PLAYER_REPLY,
            FLAG_REPLY,
            PlayerReply {
                req: 4,
                ok: true,
                code: ErrorCode::Internal,
                applied_seq: 42,
            }
            .encode(),
        ),
    ];
    let mut buf = Vec::new();
    for frame in &frames {
        write_frame_blocking(&mut buf, frame).expect("write frame");
    }

    let mut cursor = Cursor::new(buf);
    for expected in &frames {
        let frame = read_frame_blocking(&mut cursor).expect("read frame").unwrap();
        assert_eq!(&frame, expected);
    }
    assert!(read_frame_blocking(&mut cursor).expect("eof").is_none());
}

#[test]
fn oversize_frame_is_rejected_at_encode_and_header_parse() {
    let frame = Frame::new(KIND_EVENT_JSON, vec![0u8; MAX_FRAME_BYTES + 1]);
    assert_eq!(frame.encode().unwrap_err().code, ErrorCode::TooLarge);

    let mut header = [0u8; FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(&((MAX_FRAME_BYTES + 1) as u32).to_le_bytes());
    assert_eq!(
        Frame::parse_header(&header).unwrap_err().code,
        ErrorCode::TooLarge
    );
}

#[test]
fn truncated_frame_reports_eof_instead_of_a_bad_message() {
    let frame = Frame::new(KIND_REQUEST_JSON, b"hello".to_vec());
    let mut bytes = frame.encode().unwrap();
    bytes.truncate(bytes.len() - 1);
    let mut cursor = Cursor::new(bytes);
    let error = read_frame_blocking(&mut cursor).expect_err("short payload must fail");
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn unknown_frame_kind_is_a_typed_error() {
    let frame = Frame::new(0x7f, Vec::new());
    let error = Message::decode(&frame).expect_err("unknown kind");
    assert_eq!(error.code, ErrorCode::UnknownKind);
}

#[test]
fn playback_state_is_exactly_48_bytes_and_round_trips() {
    let state = PlaybackState {
        seq: 12,
        queue_rev: 34,
        anchor_ns: 56,
        index: 7,
        duration_ms: 210_500,
        position_ms: 61_250,
        flags: STATE_FLAG_MUTED | STATE_FLAG_LOADING,
        volume_milli: 700,
        state: 1,
        repeat: 2,
        rate_milli: 1000,
        reserved: 0,
    };
    let bytes = state.encode();
    assert_eq!(bytes.len(), PlaybackState::ENCODED_BYTES);
    assert_eq!(PlaybackState::decode(&bytes).unwrap(), state);

    assert_eq!(
        PlaybackState::decode(&bytes[..bytes.len() - 1])
            .unwrap_err()
            .code,
        ErrorCode::BadPayload
    );
    assert!(state.muted() && state.loading());
    assert_eq!(state.volume(), 70.0);
}

#[test]
fn position_extrapolates_only_while_playing() {
    let mut state = PlaybackState {
        seq: 1,
        state: 1,
        anchor_ns: 10_000_000_000,
        position_ms: 1_000,
        duration_ms: 60_000,
        rate_milli: 1000,
        ..Default::default()
    };
    let two_seconds_later = state.anchor_ns + 2_000_000_000;
    assert!((state.position_at(two_seconds_later) - 3.0).abs() < 1e-9);
    // A clock that jumps backwards saturates instead of returning a negative
    // position.
    assert!((state.position_at(0) - 1.0).abs() < 1e-9);
    // The playhead never passes the known duration.
    assert!((state.position_at(state.anchor_ns + 600_000_000_000) - 60.0).abs() < 1e-9);

    state.state = 2; // paused freezes the anchor position
    assert!((state.position_at(two_seconds_later) - 1.0).abs() < 1e-9);

    // Unknown duration streams still extrapolate without a clamp.
    state.state = 1;
    state.duration_ms = 0;
    assert!(state.position_at(state.anchor_ns + 600_000_000_000) > 60.0);
}

#[test]
fn queue_deltas_round_trip_every_op() {
    let deltas = vec![
        QueueDelta::reset(3, 900),
        QueueDelta {
            rev: 4,
            op: QueueOp::Rows {
                start: 0,
                rows: vec![wire_track("a", false), wire_track("b", true)],
            },
        },
        QueueDelta {
            rev: 5,
            op: QueueOp::Insert {
                at: 1,
                rows: vec![wire_track("c", false)],
            },
        },
        QueueDelta {
            rev: 6,
            op: QueueOp::Remove {
                indices: vec![1, 4, 9],
            },
        },
        QueueDelta {
            rev: 7,
            op: QueueOp::Move { from: 2, to: 8 },
        },
        QueueDelta {
            rev: 8,
            op: QueueOp::Patch {
                index: 3,
                favorite: true,
            },
        },
    ];
    for delta in deltas {
        let bytes = delta.encode().expect("encode delta");
        assert_eq!(QueueDelta::decode(&bytes).unwrap(), delta);
    }
}

#[test]
fn queue_insert_is_capped() {
    let delta = QueueDelta {
        rev: 1,
        op: QueueOp::Insert {
            at: 0,
            rows: (0..QUEUE_INSERT_MAX_ROWS + 1)
                .map(|index| wire_track(&index.to_string(), false))
                .collect(),
        },
    };
    assert_eq!(delta.encode().unwrap_err().code, ErrorCode::TooLarge);
}

#[test]
fn queue_delta_payload_rejects_garbage() {
    let error = QueueDelta::decode(&[0xff; 4]).expect_err("short delta");
    assert_eq!(error.code, ErrorCode::BadPayload);

    let mut bytes = QueueDelta::reset(1, 2).encode().unwrap();
    bytes[8] = 0x7f; // unknown op tag
    assert_eq!(
        QueueDelta::decode(&bytes).unwrap_err().code,
        ErrorCode::BadPayload
    );
}

#[test]
fn queue_rows_round_trip_with_revision() {
    let rows = QueueRows {
        rev: 11,
        start: QUEUE_CHUNK_ROWS as u32,
        rows: vec![wire_track("x", false)],
    };
    let bytes = rows.encode().unwrap();
    assert_eq!(QueueRows::decode(&bytes).unwrap(), rows);
    assert!(QueueRows::decode(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn every_player_op_round_trips() {
    let ops = vec![
        PlayerOp::PlayPause,
        PlayerOp::Play,
        PlayerOp::Pause,
        PlayerOp::Stop,
        PlayerOp::Next,
        PlayerOp::Previous,
        PlayerOp::SetVolume(42.5),
        PlayerOp::VolumeUp,
        PlayerOp::VolumeDown,
        PlayerOp::ToggleMute,
        PlayerOp::SetMuted(true),
        PlayerOp::Seek(12.25),
        PlayerOp::SeekRelative(-5.0),
        PlayerOp::Jump(9),
        PlayerOp::Remove(3),
        PlayerOp::RemoveMany(vec![1, 2, 7]),
        PlayerOp::Move { from: 1, to: 4 },
        PlayerOp::MoveBlock {
            start: 2,
            len: 3,
            delta: -1,
        },
        PlayerOp::CycleRepeat,
        PlayerOp::ShuffleQueue,
        PlayerOp::SetTrackFavorite {
            id: "song-1".to_string(),
            favorite: true,
        },
        PlayerOp::Resync,
    ];
    for (index, op) in ops.into_iter().enumerate() {
        let request = PlayerRequest {
            req: index as u32,
            op,
        };
        let bytes = request.encode();
        assert_eq!(PlayerRequest::decode(&bytes).unwrap(), request);
    }
}

#[test]
fn player_request_and_reply_frames_round_trip() {
    let request = PlayerRequest {
        req: 99,
        op: PlayerOp::Seek(30.0),
    };
    let frame = Frame::new(KIND_PLAYER_REQUEST, request.encode());
    let message = Message::decode(&frame).unwrap();
    let Message::PlayerRequest(decoded) = message else {
        panic!("expected a player request, got {message:?}");
    };
    assert_eq!(decoded, request);

    let reply = PlayerReply {
        req: 99,
        ok: false,
        code: ErrorCode::EngineUnavailable,
        applied_seq: 3,
    };
    let frame = Frame::with_flags(KIND_PLAYER_REPLY, FLAG_REPLY | FLAG_ERROR, reply.encode());
    let message = Message::decode(&frame).unwrap();
    let Message::PlayerReply(decoded) = message else {
        panic!("expected a player reply, got {message:?}");
    };
    assert_eq!(decoded, reply);
}

#[test]
fn wire_track_never_carries_a_stream_url() {
    let track = crate::player::Track {
        item: crate::model::ItemMeta {
            id: "song-1".to_string(),
            title: "Title".to_string(),
            album_id: "album".to_string(),
            cover_url: "file:///tmp/cover".to_string(),
            cover_remote: String::new(),
        },
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        favorite: true,
        duration: 10.0,
        url: "https://user:secret@example.invalid/stream?token=abc".to_string(),
        cover_art: "cover".to_string(),
    };
    let wire = WireTrack::from(&track);
    let json = serde_json::to_string(&wire).unwrap();
    assert!(!json.contains("secret"), "the stream URL must stay in the core");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(
        value.get("url").is_none(),
        "WireTrack must not have a stream URL field"
    );

    let back = crate::player::Track::from(wire);
    assert_eq!(back.id, track.id);
    assert_eq!(back.favorite, track.favorite);
    assert!(back.url.is_empty());
}

#[test]
fn activate_frame_is_empty_and_typed() {
    let message = Message::decode(&Frame::new(KIND_ACTIVATE, Vec::new())).unwrap();
    let Message::Activate = message else {
        panic!("expected activate, got {message:?}");
    };
    assert_eq!(
        Message::decode(&Frame::new(KIND_ACTIVATE, vec![1])).unwrap_err().code,
        ErrorCode::BadPayload
    );
}

#[test]
fn heartbeat_rejects_a_short_payload() {
    let heartbeat = Heartbeat {
        nonce: 5,
        monotonic_ns: 123,
    };
    assert_eq!(Heartbeat::decode(&heartbeat.encode()).unwrap(), heartbeat);
    assert_eq!(
        Heartbeat::decode(&[0; 8]).unwrap_err().code,
        ErrorCode::BadPayload
    );
}

#[test]
fn handshake_frames_round_trip() {
    let hello = Hello {
        proto: VERSION,
        role: Role::Ui,
        client: "musika-ui".to_string(),
        features: UI_FEATURES,
    };
    let message = Message::decode(&Frame::new(KIND_HELLO, json_encode(&hello).unwrap())).unwrap();
    let Message::Hello(decoded) = message else {
        panic!("expected hello, got {message:?}");
    };
    assert_eq!(decoded.proto, VERSION);
    assert_eq!(decoded.role, Role::Ui);
    assert_eq!(decoded.features, UI_FEATURES);

    let bye = Bye {
        reason: ByeReason::Quit,
    };
    let message = Message::decode(&Frame::new(KIND_BYE, json_encode(&bye).unwrap())).unwrap();
    let Message::Bye(decoded) = message else {
        panic!("expected bye, got {message:?}");
    };
    assert_eq!(decoded.reason, ByeReason::Quit);

    let welcome = Welcome {
        proto: VERSION,
        core_version: "0.6.8".to_string(),
        session: 7,
        features: CONTROL_FEATURES,
    };
    let message =
        Message::decode(&Frame::new(KIND_WELCOME, json_encode(&welcome).unwrap())).unwrap();
    let Message::Welcome(decoded) = message else {
        panic!("expected welcome, got {message:?}");
    };
    assert_eq!(decoded.session, 7);
}

#[test]
fn error_codes_are_stable_across_binary_boundaries() {
    for code in [
        ErrorCode::BadFrame,
        ErrorCode::TooLarge,
        ErrorCode::UnknownKind,
        ErrorCode::BadPayload,
        ErrorCode::VersionMismatch,
        ErrorCode::UnknownRequest,
        ErrorCode::InvalidArgs,
        ErrorCode::NoUiAttached,
        ErrorCode::EngineUnavailable,
        ErrorCode::QueueRevMismatch,
        ErrorCode::DuplicateRequest,
        ErrorCode::SlowConsumer,
        ErrorCode::Internal,
    ] {
        assert_eq!(ErrorCode::from_u8(code.as_u8()), code);
    }
    // Unknown codes degrade to Internal rather than failing a reply.
    assert_eq!(ErrorCode::from_u8(200), ErrorCode::Internal);
}

#[test]
fn bridge_events_travel_as_json_frames() {
    let event = crate::bridge::Event::Error("boom".to_string());
    let frame = event_frame(&event).unwrap();
    assert_eq!(frame.kind, KIND_EVENT_JSON);
    let message = Message::decode(&frame).unwrap();
    let Message::Event(crate::bridge::Event::Error(text)) = message else {
        panic!("expected an error event, got {message:?}");
    };
    assert_eq!(text, "boom");
}

#[test]
fn request_and_reply_json_round_trip() {
    let request = RequestJson {
        req: 3,
        method: "play_song".to_string(),
        args: json!({ "song_id": "abc", "order": "album" }),
    };
    let message = Message::decode(&Frame::new(
        KIND_REQUEST_JSON,
        json_encode(&request).unwrap(),
    ))
    .unwrap();
    let Message::Request(decoded) = message else {
        panic!("expected request, got {message:?}");
    };
    assert_eq!(decoded.req, 3);
    assert_eq!(decoded.method, "play_song");
    assert_eq!(decoded.args["song_id"], "abc");

    let reply = ReplyJson::err(3, ErrorCode::InvalidArgs, "song_id is required");
    let message =
        Message::decode(&Frame::new(KIND_REPLY_JSON, json_encode(&reply).unwrap())).unwrap();
    let Message::Reply(decoded) = message else {
        panic!("expected reply, got {message:?}");
    };
    assert!(!decoded.ok);
    assert_eq!(decoded.code, Some(ErrorCode::InvalidArgs));
    assert_eq!(decoded.message.as_deref(), Some("song_id is required"));
    assert!(ReplyJson::ok(1).ok);
}
