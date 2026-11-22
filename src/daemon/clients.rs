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
    self as proto, Bye, ByeReason, ErrorCode, Frame, Message, ProtocolError, Reject, Role, VERSION,
    Welcome,
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
/// A single socket write that cannot complete within this budget means the
/// peer stopped draining: drop the connection instead of parking the writer
/// task forever. A healthy but idle client never trips it because no frame is
/// written for it in the first place.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait after a transient listener error before accepting again.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);
/// Upper bound of the exponential accept backoff (`EMFILE` storms).
const ACCEPT_BACKOFF_MAX: Duration = Duration::from_secs(1);

/// Events the daemon consumes from the socket layer.
///
/// The `Message` variant embeds the whole wire [`Message`] enum (320 B) because
/// the daemon consumes these one at a time; boxing it would only move the cost
/// to a heap allocation per frame and churn every dispatch site.
#[expect(
    clippy::large_enum_variant,
    reason = "the wire Message payload is consumed one frame at a time; boxing it would only add an allocation per frame"
)]
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
    /// A farewell frame that bypasses the bounded queue so a slow consumer is
    /// still told why it is being closed.
    farewell: Mutex<Option<Arc<Vec<u8>>>>,
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
        match frame.encode() {
            Ok(bytes) => self.try_push(Arc::new(bytes)),
            Err(_) => false,
        }
    }

    /// Queues already-encoded frame bytes (a queue reset is serialized once and
    /// shared by every UI client).
    pub(crate) fn send_bytes(&self, bytes: Arc<Vec<u8>>) -> bool {
        self.try_push(bytes)
    }

    /// Ordered send that waits for outbox capacity. Used by the queue
    /// bootstrap, whose frame count grows with the queue length: a
    /// multi-thousand-track queue must stream with backpressure instead of
    /// overflowing the bounded outbox and closing a healthy client.
    pub(crate) async fn send_await(&self, frame: &Frame) -> bool {
        match frame.encode() {
            Ok(bytes) => self.send_bytes_await(Arc::new(bytes)).await,
            Err(_) => false,
        }
    }

    /// Already-encoded variant of [`Self::send_await`].
    pub(crate) async fn send_bytes_await(&self, bytes: Arc<Vec<u8>>) -> bool {
        if self.is_closed() {
            return false;
        }
        self.inner.tx.send(bytes).await.is_ok()
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

    pub(crate) fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Best-effort frame for traffic that may be dropped under backpressure.
    /// A full queue while the writer is busy (e.g. during a large queue
    /// bootstrap) must not be mistaken for a slow consumer; only a closed
    /// client reports `false`.
    pub(crate) fn send_best_effort(&self, frame: &Frame) -> bool {
        let Ok(bytes) = frame.encode() else {
            return false;
        };
        match self.inner.tx.try_send(Arc::new(bytes)) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// The bounded queue can no longer take a frame: remember a best-effort
    /// `Bye{SlowConsumer}` (the writer drains that slot before the backlog)
    /// and close. Returns `false` so every broadcast caller marks the client
    /// dead.
    fn try_push(&self, bytes: Arc<Vec<u8>>) -> bool {
        match self.inner.tx.try_send(bytes) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.farewell(ByeReason::SlowConsumer);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    /// Enqueues the farewell frame the writer must send before closing. Kept
    /// outside the bounded channel: the whole point is that the queue is full.
    fn farewell(&self, reason: ByeReason) {
        if let Ok(frame) = json_frame(proto::KIND_BYE, &Bye { reason })
            && let Ok(bytes) = frame.encode()
        {
            let mut farewell = self
                .inner
                .farewell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if farewell.is_none() {
                *farewell = Some(Arc::new(bytes));
            }
        }
        self.close();
    }
}

/// Writes one frame with a bounded budget: a peer that stops reading can only
/// park the writer for [`WRITE_TIMEOUT`], never forever.
async fn write_frame(writer: &mut tokio::net::unix::OwnedWriteHalf, bytes: &[u8]) -> bool {
    matches!(
        timeout(WRITE_TIMEOUT, writer.write_all(bytes)).await,
        Ok(Ok(()))
    )
}

async fn write_loop(
    inner: Arc<OutboxInner>,
    mut rx: mpsc::Receiver<Arc<Vec<u8>>>,
    mut writer: tokio::net::unix::OwnedWriteHalf,
) {
    loop {
        // A farewell (slow consumer) is the only frame worth sending once the
        // peer has overrun its outbox: skip the backlog and tell it why.
        let farewell = {
            let mut slot = inner
                .farewell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slot.take()
        };
        if let Some(bytes) = farewell {
            _ = write_frame(&mut writer, &bytes).await;
            break;
        }
        let pending = {
            let mut latest = inner
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            latest.take()
        };
        if let Some(bytes) = pending {
            if !write_frame(&mut writer, &bytes).await {
                break;
            }
            continue;
        }
        if inner.closed.load(Ordering::Acquire) {
            // Flush anything already queued, then stop. A write that times out
            // means the peer is gone, so there is nothing left to flush.
            while let Ok(bytes) = rx.try_recv() {
                if !write_frame(&mut writer, &bytes).await {
                    break;
                }
            }
            break;
        }
        let notified = inner.notify.notified();
        tokio::pin!(notified);
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(bytes) => {
                    if !write_frame(&mut writer, &bytes).await {
                        break;
                    }
                }
                None => break,
            },
            _ = &mut notified => continue,
        }
    }
    _ = writer.shutdown().await;
}

/// Starts the framed server. Returns immediately; connections are handled by
/// spawned tasks and reported through `events`.
pub(crate) fn spawn(path: PathBuf, events: mpsc::UnboundedSender<ServerEvent>, session: u64) {
    tokio::spawn(async move {
        // Only a Musika-owned parent directory is created/restricted; a
        // user-provided override path is never chmodded (S-14).
        crate::ipc::prepare_socket_parent(&path);
        _ = tokio::fs::remove_file(&path).await;
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("daemon: cannot bind {}: {error}", path.display());
                return;
            }
        };
        crate::startup::trace("daemon.socket.bound");
        {
            use std::os::unix::fs::PermissionsExt;
            _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let next_conn = Arc::new(AtomicU64::new(1));
        let mut backoff = ACCEPT_BACKOFF;
        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(accepted) => {
                    backoff = ACCEPT_BACKOFF;
                    accepted
                }
                Err(error) => {
                    // A listener error is never fatal for the daemon: EINTR
                    // retries immediately, resource exhaustion (EMFILE/ENFILE)
                    // backs off instead of hot-looping, and the control socket
                    // stays up rather than silently disappearing.
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    crate::debug_log!("daemon: accept failed: {error}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(ACCEPT_BACKOFF_MAX);
                    continue;
                }
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
                _ = events.send(ServerEvent::Gone { conn });
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
        farewell: Mutex::new(None),
        notify: Notify::new(),
        closed: AtomicBool::new(false),
    });
    let outbox = Outbox {
        inner: Arc::clone(&inner),
    };
    tokio::spawn(write_loop(Arc::clone(&inner), rx, writer));

    // A client that predates the feature bits (or omits them) keeps working
    // with its role's default surface; an explicit mask is honored, clamped to
    // what its role can actually consume.
    let features = if hello.features == 0 {
        hello.role.default_features()
    } else {
        hello.features & hello.role.default_features()
    };
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
        // The writer closes the outbox when the peer overruns it; stop reading
        // (and stop forwarding work for a client that will never see it)
        // instead of lingering until the heartbeat timeout.
        if outbox.is_closed() {
            break;
        }
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
                // Heartbeat probes are disposable: a full outbox (e.g. during
                // the queue bootstrap) drops the ping instead of closing a
                // client that is merely busy reading the initial stream.
                if !outbox.send_best_effort(&Frame::new(proto::KIND_PING, ping.encode())) {
                    break;
                }
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
                        if !outbox.send(&Frame::new(proto::KIND_PONG, ping.encode())) {
                            break;
                        }
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
        _ = timeout(WRITE_TIMEOUT, writer.write_all(&bytes)).await;
        _ = timeout(WRITE_TIMEOUT, writer.flush()).await;
    }
    _ = writer.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    struct Harness {
        dir: PathBuf,
        events: mpsc::UnboundedReceiver<ServerEvent>,
    }

    fn harness() -> Harness {
        let dir = std::env::temp_dir().join(format!(
            "musika-clients-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let (events, rx) = mpsc::unbounded_channel();
        spawn(dir.join("daemon.sock"), events, 9001);
        Harness { dir, events: rx }
    }

    impl Harness {
        async fn connect(&self) -> UnixStream {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            loop {
                match UnixStream::connect(self.dir.join("daemon.sock")).await {
                    Ok(stream) => return stream,
                    Err(_) if tokio::time::Instant::now() < deadline => {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Err(error) => panic!("daemon socket never came up: {error}"),
                }
            }
        }

        #[expect(
            clippy::needless_pass_by_ref_mut,
            reason = "the awaited read_frame future advances the stream; clippy cannot see the mutation through the async generic"
        )]
        async fn next_frame(&mut self, stream: &mut UnixStream) -> Frame {
            // Reborrow explicitly: the helper itself never mutates `stream`.
            tokio::time::timeout(Duration::from_secs(2), proto::read_frame(&mut *stream))
                .await
                .expect("frame timeout")
                .expect("frame read")
                .expect("frame eof")
        }

        async fn handshake(&mut self) -> (UnixStream, u64, Outbox) {
            let mut stream = self.connect().await;
            let hello = crate::ipc_protocol::Hello {
                proto: VERSION,
                role: Role::Ui,
                client: "clients-test".to_string(),
                features: proto::UI_FEATURES,
            };
            let bytes = Frame::new(
                proto::KIND_HELLO,
                proto::json_encode(&hello).expect("hello"),
            )
            .encode()
            .expect("hello frame");
            stream.write_all(&bytes).await.expect("hellO write");

            let welcome = self.next_frame(&mut stream).await;
            let Message::Welcome(welcome) = Message::decode(&welcome).expect("welcome decode")
            else {
                panic!("expected a welcome frame");
            };
            assert_eq!(welcome.proto, VERSION);
            assert_eq!(welcome.session, 9001);

            let event = tokio::time::timeout(Duration::from_secs(2), self.events.recv())
                .await
                .expect("connected event")
                .expect("event channel");
            let ServerEvent::Connected { conn, outbox, .. } = event else {
                panic!("expected a Connected event");
            };
            (stream, conn, outbox)
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

    impl Drop for Harness {
        fn drop(&mut self) {
            _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// D-04/D-05 regression: a healthy idle client that only answers the
    /// heartbeat must never be dropped, and a client that stops draining its
    /// outbox must be told `ByeReason::SlowConsumer` before it is closed.
    #[tokio::test(start_paused = true)]
    async fn idle_client_not_killed_and_slow_consumer_gets_bye() {
        let mut harness = harness();

        // Idle phase: the client answers the wire heartbeat but the core has
        // no outbound work for it. The paused clock lets 12 s of writer-idle
        // time pass instantly, which is exactly what the old inverted stall
        // deadline treated as a dead peer.
        let (mut idle, idle_conn, idle_outbox) = harness.handshake().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
        let pong = Frame::new(
            proto::KIND_PONG,
            crate::ipc_protocol::Heartbeat {
                nonce: 0,
                monotonic_ns: crate::linux::monotonic_ns(),
            }
            .encode(),
        )
        .encode()
        .expect("pong frame");
        while tokio::time::Instant::now() < deadline {
            idle.write_all(&pong)
                .await
                .expect("an idle client must stay connected");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            !idle_outbox.is_closed(),
            "an idle client must not be closed by the writer"
        );
        drop(idle);
        harness.drop_gone(idle_conn).await;

        // Slow-consumer phase: never read the socket, overrun the bounded
        // outbox, then drain the kernel buffer until the farewell arrives.
        let (mut slow, slow_conn, slow_outbox) = harness.handshake().await;
        let payload = vec![0u8; 8 * 1024];
        let frame = Frame::new(proto::KIND_EVENT_JSON, payload);
        let mut overflowed = false;
        for _ in 0..4096 {
            if !slow_outbox.send(&frame) {
                overflowed = true;
                break;
            }
        }
        assert!(
            overflowed,
            "the bounded outbox must reject a stalled client"
        );

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let frame = tokio::time::timeout(left, proto::read_frame(&mut slow))
                .await
                .expect("the farewell never arrived")
                .expect("frame read")
                .expect("frame eof before the farewell");
            match Message::decode(&frame) {
                Ok(Message::Bye(bye)) => {
                    assert_eq!(bye.reason, ByeReason::SlowConsumer);
                    break;
                }
                // The writer may have flushed (part of) the backlog it was
                // already writing before the outbox overflowed: the filler
                // frames are not decodable messages, so skip everything until
                // the farewell.
                _ => continue,
            }
        }
        harness.drop_gone(slow_conn).await;
    }
}
