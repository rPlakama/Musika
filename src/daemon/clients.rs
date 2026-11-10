//! Framed core/UI socket: handshake, per-client outbox, heartbeat and registry
//! events.
//!
//! Every accepted connection gets a reader task and a writer task. The writer
//! drains ordered frames from a bounded queue and a single "latest state" slot,
//! so a playback-state burst collapses to the newest frame while queue deltas,
//! replies and bridge events are never silently dropped. A client that stops
//! draining is closed (`ByeReason::SlowConsumer`) and resyncs on reconnect.
//!
//! The daemon itself only sees [`ServerEvent`]s; it never touches a socket.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, mpsc};
use tokio::time::timeout;

use crate::ipc::same_uid;
use crate::ipc_protocol::{
    self as proto, Bye, ByeReason, ErrorCode, Frame, Message, ProtocolError, Reject, Role,
    Welcome, VERSION,
};

/// Ordered frames a client may have pending before it is considered stalled.
const OUTBOX_CAPACITY: usize = 128;
/// Heartbeat cadence and how many missed intervals close the connection.
#[cfg(not(test))]
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
#[cfg(test)]
const HEARTBEAT_EVERY: Duration = Duration::from_millis(25);
#[cfg(not(test))]
const HEARTBEAT_MISSES: u32 = 3;
#[cfg(test)]
const HEARTBEAT_MISSES: u32 = 2;
/// Handshake deadline: a client that connects and says nothing is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Events the daemon consumes from the socket layer.
pub(crate) enum ServerEvent {
    Connected {
        conn: u64,
        role: Role,
        features: u32,
        client: String,
        outbox: Outbox,
    },
    Message {
        conn: u64,
        message: Message,
    },
    Gone {
        conn: u64,
    },
}

struct OutboxInner {
    tx: mpsc::Sender<Arc<Vec<u8>>>,
    latest: Mutex<Option<Arc<Vec<u8>>>>,
    notify: Notify,
    closed: AtomicBool,
}

/// Cloneable handle used by the daemon to write to one client.
#[derive(Clone)]
pub(crate) struct Outbox {
    inner: Arc<OutboxInner>,
}

impl Outbox {
    /// Queues one ordered frame. `false` means the client is not draining and
    /// the caller should close it.
    pub(crate) fn send(&self, frame: &Frame) -> bool {
        let Ok(bytes) = frame.encode() else {
            return false;
        };
        let bytes = Arc::new(bytes);
        match self.inner.tx.try_send(bytes) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.close();
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Queues already-encoded frame bytes (a queue reset is serialized once and
    /// shared by every UI client).
    pub(crate) fn send_bytes(&self, bytes: Arc<Vec<u8>>) -> bool {
        match self.inner.tx.try_send(bytes) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.close();
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Replaces the pending "latest state" frame. Used for playback snapshots
    /// and clock corrections, which are worthless once superseded.
    pub(crate) fn send_latest(&self, frame: &Frame) -> bool {
        let Ok(bytes) = frame.encode() else {
            return false;
        };
        let bytes = Arc::new(bytes);
        let mut latest = self
            .inner
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *latest = Some(bytes);
        self.inner.notify.notify_one();
        true
    }

    /// Closes the client after its already-queued frames are flushed.
    pub(crate) fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.notify.notify_one();
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }
}

async fn write_loop(
    inner: Arc<OutboxInner>,
    mut rx: mpsc::Receiver<Arc<Vec<u8>>>,
    mut writer: tokio::net::unix::OwnedWriteHalf,
) {
    // Give a client a bounded amount of time to accept the backlog so a
    // half-dead peer cannot park the writer task forever.
    let mut stall_deadline: Option<tokio::time::Instant> = None;
    loop {
        let pending = {
            let mut latest = inner
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            latest.take()
        };
        if let Some(bytes) = pending {
            if writer.write_all(&bytes).await.is_err() {
                break;
            }
            stall_deadline = None;
            continue;
        }
        if inner.closed.load(Ordering::Acquire) {
            // Flush anything already queued, then stop.
            while let Ok(bytes) = rx.try_recv() {
                if writer.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            break;
        }
        let notified = inner.notify.notified();
        tokio::pin!(notified);
        let deadline = *stall_deadline
            .get_or_insert_with(|| tokio::time::Instant::now() + Duration::from_secs(10));
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(bytes) => {
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                    stall_deadline = None;
                }
                None => break,
            },
            _ = &mut notified => continue,
            _ = tokio::time::sleep_until(deadline) => break,
        }
    }
    let _ = writer.shutdown().await;
}

/// Starts the framed server. Returns immediately; connections are handled by
/// spawned tasks and reported through `events`.
pub(crate) fn spawn(
    path: PathBuf,
    events: mpsc::UnboundedSender<ServerEvent>,
    session: u64,
) {
    tokio::spawn(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(
                parent,
                std::fs::Permissions::from_mode(0o700),
            )
            .await;
        }
        let _ = tokio::fs::remove_file(&path).await;
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("daemon: cannot bind {}: {error}", path.display());
                return;
            }
        };
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let next_conn = Arc::new(AtomicU64::new(1));
        loop {
            let Ok((stream, _addr)) = listener.accept().await else {
                break;
            };
            if !same_uid(&stream) {
                eprintln!("daemon: refused a control connection from another user");
                continue;
            }
            let conn = next_conn.fetch_add(1, Ordering::Relaxed);
            let events = events.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_connection(conn, stream, events.clone(), session).await {
                    crate::debug_log!("daemon: connection {conn} ended: {error}");
                }
                let _ = events.send(ServerEvent::Gone { conn });
            });
        }
    });
}

fn json_frame(kind: u8, value: &impl serde::Serialize) -> std::io::Result<Frame> {
    proto::json_encode(value)
        .map(|payload| Frame::new(kind, payload))
        .map_err(|error| error.io())
}

async fn handle_connection(
    conn: u64,
    stream: UnixStream,
    events: mpsc::UnboundedSender<ServerEvent>,
    session: u64,
) -> std::io::Result<()> {
    let (mut reader, writer) = stream.into_split();

    let hello = match timeout(HANDSHAKE_TIMEOUT, proto::read_frame(&mut reader)).await {
        Err(_) => {
            let bye = Bye {
                reason: ByeReason::Heartbeat,
            };
            if let Ok(frame) = json_frame(proto::KIND_BYE, &bye) {
                send_one_shot(writer, &frame).await;
            }
            return Ok(());
        }
        Ok(Err(error)) => return Err(error),
        Ok(Ok(None)) => return Ok(()),
        Ok(Ok(Some(frame))) => match Message::decode(&frame) {
            Ok(Message::Hello(hello)) => hello,
            _ => {
                let reject = Reject {
                    code: ErrorCode::BadFrame,
                    message: "the first frame must be Hello".to_string(),
                };
                let frame = json_frame(proto::KIND_REJECT, &reject)?;
                send_one_shot(writer, &frame).await;
                return Ok(());
            }
        },
    };
    if hello.proto != VERSION {
        let reject = Reject {
            code: ErrorCode::VersionMismatch,
            message: format!(
                "core protocol {VERSION}, client protocol {} — rebuild musika-ui with this musika",
                hello.proto
            ),
        };
        let frame = json_frame(proto::KIND_REJECT, &reject)?;
        send_one_shot(writer, &frame).await;
        return Ok(());
    }

    let (tx, rx) = mpsc::channel::<Arc<Vec<u8>>>(OUTBOX_CAPACITY);
    let inner = Arc::new(OutboxInner {
        tx,
        latest: Mutex::new(None),
        notify: Notify::new(),
        closed: AtomicBool::new(false),
    });
    let outbox = Outbox {
        inner: Arc::clone(&inner),
    };
    tokio::spawn(write_loop(Arc::clone(&inner), rx, writer));

    let features = hello.features & (proto::UI_FEATURES | proto::CONTROL_FEATURES);
    let welcome = Welcome {
        proto: VERSION,
        core_version: env!("CARGO_PKG_VERSION").to_string(),
        session,
        features,
    };
    if !outbox.send(&json_frame(proto::KIND_WELCOME, &welcome)?) {
        return Ok(());
    }

    if events
        .send(ServerEvent::Connected {
            conn,
            role: hello.role,
            features,
            client: hello.client,
            outbox: outbox.clone(),
        })
        .is_err()
    {
        return Ok(());
    }

    let mut misses = 0u32;
    loop {
        match timeout(HEARTBEAT_EVERY, proto::read_frame(&mut reader)).await {
            Err(_) => {
                misses += 1;
                if misses > HEARTBEAT_MISSES {
                    if let Ok(frame) = json_frame(
                        proto::KIND_BYE,
                        &Bye {
                            reason: ByeReason::Heartbeat,
                        },
                    ) {
                        outbox.send(&frame);
                    }
                    break;
                }
                let ping = crate::ipc_protocol::Heartbeat {
                    nonce: misses as u64,
                    monotonic_ns: crate::linux::monotonic_ns(),
                };
                outbox.send(&Frame::new(proto::KIND_PING, ping.encode()));
            }
            Ok(Err(error)) => {
                crate::debug_log!("daemon: client {conn} read failed: {error}");
                break;
            }
            Ok(Ok(None)) => break,
            Ok(Ok(Some(frame))) => {
                misses = 0;
                match Message::decode(&frame) {
                    Ok(Message::Ping(ping)) => {
                        outbox.send(&Frame::new(proto::KIND_PONG, ping.encode()));
                    }
                    Ok(Message::Pong(_)) => {}
                    Ok(Message::Hello(_)) => {
                        if let Ok(frame) = json_frame(
                        proto::KIND_REJECT,
                        &Reject {
                            code: ErrorCode::BadFrame,
                            message: "unexpected second Hello".to_string(),
                        },
                    ) {
                        outbox.send(&frame);
                    }
                    break;
                    }
                    Ok(message) => {
                        if events.send(ServerEvent::Message { conn, message }).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let protocol_error = ProtocolError {
                            code: error.code,
                            message: error.message,
                        };
                        if let Ok(frame) = json_frame(proto::KIND_PROTOCOL_ERROR, &protocol_error) {
                            outbox.send(&frame);
                        }
                        if let Ok(frame) = json_frame(
                            proto::KIND_BYE,
                            &Bye {
                                reason: ByeReason::Protocol,
                            },
                        ) {
                            outbox.send(&frame);
                        }
                        break;
                    }
                }
            }
        }
    }

    outbox.close();
    Ok(())
}

async fn send_one_shot(mut writer: tokio::net::unix::OwnedWriteHalf, frame: &Frame) {
    if let Ok(bytes) = frame.encode() {
        let _ = writer.write_all(&bytes).await;
        let _ = writer.flush().await;
    }
    let _ = writer.shutdown().await;
}
