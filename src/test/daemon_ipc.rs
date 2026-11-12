//! Framed daemon socket integration tests (`src/daemon/clients.rs`).
//!
//! These drive the real server over a real Unix socket with no daemon loop:
//! the test plays the daemon's role by consuming `ServerEvent`s and writing
//! replies through the `Outbox`.

use super::common::*;
use crate::daemon::clients::{self, Outbox, ServerEvent};
use crate::ipc_protocol::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

static NEXT: AtomicU64 = AtomicU64::new(1);

struct Harness {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
    events: mpsc::UnboundedReceiver<ServerEvent>,
}

fn harness() -> Harness {
    let dir = std::env::temp_dir().join(format!(
        "musika-daemon-ipc-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("daemon.sock");
    let (tx, rx) = mpsc::unbounded_channel();
    clients::spawn(path.clone(), tx, 4242);
    Harness {
        dir,
        path,
        events: rx,
    }
}

impl Harness {
    async fn connect(&self) -> UnixStream {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match UnixStream::connect(&self.path).await {
                Ok(stream) => return stream,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("daemon socket never came up: {error}"),
            }
        }
    }

    async fn handshake(&mut self) -> (UnixStream, u64, Outbox) {
        let mut stream = self.connect().await;
        let hello = Hello {
            proto: VERSION,
            role: Role::Ui,
            client: "test-ui".to_string(),
            features: UI_FEATURES,
        };
        proto_write(&mut stream, &Frame::new(KIND_HELLO, json_encode(&hello).unwrap())).await;

        let welcome = next_frame(&mut stream).await;
        match Message::decode(&welcome).unwrap() {
            Message::Welcome(welcome) => {
                assert_eq!(welcome.proto, VERSION);
                assert_eq!(welcome.session, 4242);
                assert_eq!(welcome.features, UI_FEATURES);
            }
            other => panic!("expected welcome, got {other:?}"),
        }

        let event = tokio::time::timeout(Duration::from_secs(2), self.events.recv())
            .await
            .expect("connected event")
            .expect("event channel");
        match event {
            ServerEvent::Connected {
                conn,
                role,
                features,
                client,
                outbox,
            } => {
                assert_eq!(role, Role::Ui);
                assert_eq!(features, UI_FEATURES);
                assert_eq!(client, "test-ui");
                (stream, conn, outbox)
            }
            _ => panic!("expected a Connected event"),
        }
    }

    async fn drop_gone(&mut self, conn: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(left, self.events.recv()).await {
                Ok(Some(ServerEvent::Gone { conn: gone })) if gone == conn => return,
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => panic!("no Gone event for client {conn}"),
            }
        }
    }
}

async fn proto_write(stream: &mut UnixStream, frame: &Frame) {
    let bytes = frame.encode().expect("encode frame");
    stream.write_all(&bytes).await.expect("write frame");
}

async fn next_frame(stream: &mut UnixStream) -> Frame {
    tokio::time::timeout(Duration::from_secs(2), read_frame(stream))
        .await
        .expect("frame timeout")
        .expect("frame read")
        .expect("frame eof")
}

async fn read_message(stream: &mut UnixStream) -> Message {
    let frame = next_frame(stream).await;
    Message::decode(&frame).expect("decode message")
}

impl Drop for Harness {
    fn drop(&mut self) {
        _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn handshake_accepts_and_registers_a_ui_client() {
    let mut harness = harness();
    let (_stream, _conn, _outbox) = harness.handshake().await;
}

#[tokio::test]
async fn handshake_rejects_a_protocol_mismatch() {
    let harness = harness();
    let mut stream = harness.connect().await;
    let hello = Hello {
        proto: VERSION + 1,
        role: Role::Ui,
        client: "old-ui".to_string(),
        features: UI_FEATURES,
    };
    proto_write(&mut stream, &Frame::new(KIND_HELLO, json_encode(&hello).unwrap())).await;
    match read_message(&mut stream).await {
        Message::Reject(reject) => assert_eq!(reject.code, ErrorCode::VersionMismatch),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn first_frame_must_be_hello() {
    let harness = harness();
    let mut stream = harness.connect().await;
    let ping = Heartbeat {
        nonce: 1,
        monotonic_ns: 2,
    };
    proto_write(&mut stream, &Frame::new(KIND_PING, ping.encode())).await;
    match read_message(&mut stream).await {
        Message::Reject(reject) => assert_eq!(reject.code, ErrorCode::BadFrame),
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn bridge_requests_are_delivered_and_replies_round_trip() {
    let mut harness = harness();
    let (mut stream, conn, outbox) = harness.handshake().await;
    let request = RequestJson {
        req: 7,
        method: "play_song".to_string(),
        args: json!({ "song_id": "abc", "order": "album" }),
    };
    proto_write(
        &mut stream,
        &Frame::new(KIND_REQUEST_JSON, json_encode(&request).unwrap()),
    )
    .await;

    let event = tokio::time::timeout(Duration::from_secs(2), harness.events.recv())
        .await
        .expect("message")
        .expect("channel");
    match event {
        ServerEvent::Message {
            conn: from,
            message: Message::Request(decoded),
        } => {
            assert_eq!(from, conn);
            assert_eq!(decoded.req, 7);
            assert_eq!(decoded.method, "play_song");
        }
        _ => panic!("expected the request event"),
    }

    let reply = ReplyJson::ok(7);
    outbox.send(&Frame::new(
        KIND_REPLY_JSON,
        json_encode(&reply).unwrap(),
    ));
    match read_message(&mut stream).await {
        Message::Reply(reply) => {
            assert_eq!(reply.req, 7);
            assert!(reply.ok);
        }
        other => panic!("expected a reply, got {other:?}"),
    }
}

#[tokio::test]
async fn latest_frames_coalesce_to_the_newest_state() {
    let mut harness = harness();
    let (mut stream, _conn, outbox) = harness.handshake().await;
    for seq in 1..=500u64 {
        let mut state = PlaybackState {
            seq,
            index: 3,
            ..Default::default()
        };
        state.rate_milli = 1000;
        outbox.send_latest(&Frame::new(KIND_PLAYBACK_STATE, state.encode()));
    }
    // The writer may have sent an intermediate frame; the last one delivered
    // must be the newest.
    let mut last = 0u64;
    for _ in 0..40 {
        match tokio::time::timeout(Duration::from_millis(300), read_frame(&mut stream)).await {
            Ok(Ok(Some(frame))) => {
                if let Ok(Message::PlaybackState(state)) = Message::decode(&frame) {
                    last = state.seq;
                    if last == 500 {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    assert_eq!(last, 500, "the coalesced state slot must win");
}

#[tokio::test]
async fn activate_and_bye_are_typed_frames() {
    let mut harness = harness();
    let (mut stream, conn, outbox) = harness.handshake().await;
    outbox.send(&Frame::new(KIND_ACTIVATE, Vec::new()));
    match read_message(&mut stream).await {
        Message::Activate => {}
        other => panic!("expected activate, got {other:?}"),
    }
    let bye = Bye {
        reason: ByeReason::Quit,
    };
    outbox.send(&Frame::new(KIND_BYE, json_encode(&bye).unwrap()));
    outbox.close();
    match read_message(&mut stream).await {
        Message::Bye(bye) => assert_eq!(bye.reason, ByeReason::Quit),
        other => panic!("expected bye, got {other:?}"),
    }
    harness.drop_gone(conn).await;
}

#[tokio::test]
async fn a_silent_client_is_closed_by_the_heartbeat() {
    let mut harness = harness();
    let (mut stream, _conn, _outbox) = harness.handshake().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = tokio::time::timeout(
            left.min(Duration::from_secs(2)),
            read_frame(&mut stream),
        )
        .await;
        match frame {
            Ok(Ok(Some(frame))) => match Message::decode(&frame) {
                Ok(Message::Ping(_)) => continue,
                Ok(Message::Bye(bye)) => {
                    assert_eq!(bye.reason, ByeReason::Heartbeat);
                    return;
                }
                Ok(_) => continue,
                Err(_) => continue,
            },
            _ => panic!("the heartbeat never closed the silent client"),
        }
    }
}

#[tokio::test]
async fn a_client_that_stops_reading_is_dropped() {
    let mut harness = harness();
    let (_stream, conn, outbox) = harness.handshake().await;
    // Do not read from the socket: fill the kernel buffer, then the bounded
    // outbox. The first failed send means the client must be dropped.
    let payload = vec![0u8; 8 * 1024];
    let frame = Frame::new(KIND_EVENT_JSON, payload);
    let mut failed = false;
    for _ in 0..4096 {
        if !outbox.send(&frame) {
            failed = true;
            break;
        }
    }
    assert!(failed, "the bounded outbox must reject a stalled client");
    drop(_stream);
    outbox.close();
    harness.drop_gone(conn).await;
}
