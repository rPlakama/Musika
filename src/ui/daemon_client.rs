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

//! UI-side client for the core daemon (`ipc_protocol` v2).
//!
//! `musika-ui` never opens the databases, network or audio device. It speaks
//! the framed [`crate::ipc_protocol`] contract over `daemon.sock`:
//!
//! - one reader thread decodes frames and pushes [`UiUpdate`]s into a bounded
//!   tokio channel consumed by the GPUI root;
//! - one writer thread drains a bounded queue of pre-encoded [`Frame`]s, so no
//!   GPUI key/render handler ever blocks on the socket;
//! - the supervisor reconnects with a 50 ms -> 1 s exponential backoff and
//!   re-sends the handshake, after which the core replays state and queue.
//!
//! The core is the single owner of playback and queue state. The UI keeps a
//! read-only [`PlaybackView`] projection (driven by `PlaybackState`,
//! `QueueDelta` and `QueueRows` frames) plus a [`PlaybackClient`] that only
//! emits [`PlayerOp`] requests.

use std::collections::VecDeque;
use std::io;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{
    Receiver as WriteReceiver, RecvTimeoutError, SyncSender, TrySendError as WriteTrySendError,
    sync_channel,
};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc::{Receiver, Sender, error::TrySendError as UpdateTrySendError};

use crate::bridge::Event;
use crate::ipc_protocol::{
    self, ByeReason, ErrorCode, Frame, Heartbeat, Hello, Message, PlaybackState, PlayerOp,
    PlayerReply, PlayerRequest, QueueDelta, QueueOp, QueueRows, ReplyJson, RequestJson, Role,
    UI_FEATURES, VERSION, WireTrack,
};
use crate::player::{Repeat, State, Track};

/// How long `Client::connect` waits for the freshly spawned daemon to bind.
const CONNECT_STEP: Duration = Duration::from_millis(50);
const CONNECT_ATTEMPTS: usize = 100;

/// Reconnect backoff floor/ceiling after a transport error or a `Bye`.
const RECONNECT_MIN_MS: u64 = 50;
const RECONNECT_MAX_MS: u64 = 1_000;
/// Retry cadence for the "core is not up yet" startup surface.
const STARTUP_RETRY_MS: u64 = 1_000;

/// Bounded outbound frame queue (writer thread side).
const WRITER_QUEUE_FRAMES: usize = 64;
/// Bounded inbound update channel (GPUI side).
const UPDATE_QUEUE_FRAMES: usize = 256;
/// Hard cap for non-droppable frames that did not fit the writer queue.
const OVERFLOW_FRAMES: usize = 4_096;
/// Writer poll interval, so a coalesced frame left in the slot is replayed even
/// when the writer queue is idle.
const WRITER_POLL: Duration = Duration::from_millis(50);

/// Same wire body as [`crate::player::Track`] minus the stream URL.
fn empty_track() -> Track {
    Track {
        item: crate::model::ItemMeta::default(),
        artist: String::new(),
        album: String::new(),
        favorite: false,
        duration: 0.0,
        url: String::new(),
        cover_art: String::new(),
    }
}

/// What the reader thread hands to the GPUI root.
#[derive(Debug)]
pub enum UiUpdate {
    /// A cached bridge event (`Header`, auth, `OpenPage`, …).
    Event(Event),
    /// Coalescible transport snapshot.
    State(PlaybackState),
    /// Structural queue change.
    QueueDelta(QueueDelta),
    /// Queue row bodies (coalescible: dropping one asks for a resync).
    QueueRows(QueueRows),
    /// Ack for one `PlayerOp`.
    PlayerReply(PlayerReply),
    /// Ack for one bridge request.
    Reply(ReplyJson),
    /// The user asked to show/raise the window.
    Activate,
    /// The transport dropped; the client is reconnecting.
    Reconnecting(Option<String>),
    /// Handshake completed (initial connection or a reconnect).
    Connected { session: u64 },
    /// Unrecoverable protocol mismatch; the UI must stop and report.
    Fatal(String),
    /// The core asked the UI to close.
    Shutdown,
}

// ---------------------------------------------------------------------------
// Outbound plumbing: one bounded writer queue, one coalescing slot and one
// overflow for frames that must never be dropped by a slow consumer.
// ---------------------------------------------------------------------------

struct Outbound {
    /// Live writer queue; `None` while disconnected.
    writer: Option<SyncSender<Frame>>,
    /// Latest coalescible frame (volume/seek) that did not fit the queue.
    pending: Option<Frame>,
    /// Frames that must not be dropped (transport, requests, pongs) kept while
    /// the writer queue was full or the transport was down.
    overflow: VecDeque<Frame>,
}

/// Pushes one frame without ever blocking the caller. A full queue keeps the
/// frame for the writer thread (coalescible) or parks it in the overflow
/// (critical); a dead writer is dropped so the next connection adopts the work.
fn try_send_frame(writer: &mut Option<SyncSender<Frame>>, frame: Frame) -> Result<(), Frame> {
    let Some(sender) = writer.take() else {
        return Err(frame);
    };
    match sender.try_send(frame) {
        Ok(()) => {
            *writer = Some(sender);
            Ok(())
        }
        Err(WriteTrySendError::Full(frame)) => {
            *writer = Some(sender);
            Err(frame)
        }
        Err(WriteTrySendError::Disconnected(frame)) => Err(frame),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Inner {
    updates: Sender<UiUpdate>,
    outbound: Mutex<Outbound>,
    next_req: AtomicU32,
    session: AtomicU64,
    connected: AtomicBool,
    /// Set when no further reconnect should be attempted (`Shutdown`/`Fatal`).
    stopping: AtomicBool,
}

impl Inner {
    fn new(updates: Sender<UiUpdate>) -> Self {
        Self {
            updates,
            outbound: Mutex::new(Outbound {
                writer: None,
                pending: None,
                overflow: VecDeque::new(),
            }),
            next_req: AtomicU32::new(0),
            session: AtomicU64::new(0),
            connected: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        }
    }

    fn next_req(&self) -> u32 {
        self.next_req.fetch_add(1, Ordering::Relaxed)
    }

    /// Enqueues one frame; `coalescible` marks frames a newer value supersedes.
    fn send_frame(&self, frame: Frame, coalescible: bool) {
        let mut out = lock(&self.outbound);
        if coalescible {
            // A parked value is always the newest one: supersede it instead of
            // letting the writer replay an older target after a newer frame.
            if out.pending.is_some() {
                out.pending = Some(frame);
                return;
            }
            if let Err(frame) = try_send_frame(&mut out.writer, frame) {
                out.pending = Some(frame);
            }
            return;
        }
        // Critical frames keep their order: once one is parked (the queue was
        // full or the transport was down), later ones follow it instead of
        // jumping ahead of it.
        let frame = if out.overflow.is_empty() {
            match try_send_frame(&mut out.writer, frame) {
                Ok(()) => return,
                Err(frame) => frame,
            }
        } else {
            frame
        };
        if out.overflow.len() >= OVERFLOW_FRAMES {
            crate::debug_log!("ipc: outbound overflow is full; dropping the oldest frame");
            out.overflow.pop_front();
        }
        out.overflow.push_back(frame);
    }

    /// Sends one `RequestJson` (bridge surface) and returns its request id.
    fn send_request(&self, method: &str, args: serde_json::Value) -> u32 {
        let req = self.next_req();
        let message = RequestJson {
            req,
            method: method.to_string(),
            args,
        };
        match ipc_protocol::json_encode(&message) {
            Ok(payload) => {
                self.send_frame(Frame::new(ipc_protocol::KIND_REQUEST_JSON, payload), false)
            }
            Err(error) => crate::debug_log!(
                "ipc: could not encode bridge request {method}: {:?}",
                error.code
            ),
        }
        req
    }

    /// Sends one `PlayerOp` and returns its request id. Volume/seek are
    /// coalescible: a backlog keeps only the newest target.
    fn send_player(&self, op: PlayerOp) -> u32 {
        let req = self.next_req();
        let coalescible = matches!(
            op,
            PlayerOp::SetVolume(_) | PlayerOp::Seek(_) | PlayerOp::SeekRelative(_)
        );
        let request = PlayerRequest { req, op };
        self.send_frame(
            Frame::new(ipc_protocol::KIND_PLAYER_REQUEST, request.encode()),
            coalescible,
        );
        req
    }

    fn send_pong(&self, heartbeat: Heartbeat) {
        let pong = Heartbeat {
            nonce: heartbeat.nonce,
            monotonic_ns: crate::linux::monotonic_ns(),
        };
        self.send_frame(Frame::new(ipc_protocol::KIND_PONG, pong.encode()), false);
    }

    /// Structural updates are never dropped: the reader thread may block, the
    /// GPUI thread never does.
    fn push_structural(&self, update: UiUpdate) {
        match self.updates.try_send(update) {
            Ok(()) => {}
            Err(UpdateTrySendError::Full(update)) => {
                let _ = self.updates.blocking_send(update);
            }
            Err(UpdateTrySendError::Closed(_)) => {}
        }
    }

    /// Coalescible updates (state / queue rows) are dropped when the UI is
    /// behind, then repaired: a lost row chunk asks for a resync, a lost state
    /// leaves a `seq` gap that also asks for one.
    fn push_coalescible(&self, update: UiUpdate) {
        if let Err(UpdateTrySendError::Full(update)) = self.updates.try_send(update) {
            if matches!(update, UiUpdate::QueueRows(_)) {
                self.send_player(PlayerOp::Resync);
            }
        }
    }

    /// Installs the writer thread for a fresh socket, keeping the overflow of
    /// critical frames so requests issued while disconnected are delivered.
    fn install_writer(self: &Arc<Self>, socket: &UnixStream) -> io::Result<()> {
        let write_half = socket.try_clone()?;
        let (sender, receiver) = sync_channel::<Frame>(WRITER_QUEUE_FRAMES);
        {
            let mut out = lock(&self.outbound);
            out.writer = Some(sender);
            out.pending = None;
        }
        let inner = Arc::clone(self);
        std::thread::Builder::new()
            .name("musika-ipc-writer".to_string())
            .spawn(move || write_loop(inner, write_half, receiver))
            .expect("spawn ipc writer");
        Ok(())
    }

    fn drop_connection(&self) {
        self.connected.store(false, Ordering::Release);
        self.session.store(0, Ordering::Release);
        let mut out = lock(&self.outbound);
        out.writer = None;
        out.pending = None;
    }

    /// Writes the coalesced slot and the overflow, in that order.
    fn drain_outbound(&self, write_half: &UnixStream) -> io::Result<()> {
        loop {
            let frame = {
                let mut out = lock(&self.outbound);
                match out.pending.take() {
                    Some(frame) => Some(frame),
                    None => out.overflow.pop_front(),
                }
            };
            let Some(frame) = frame else {
                return Ok(());
            };
            let mut writer = write_half;
            ipc_protocol::write_frame_blocking(&mut writer, &frame)?;
        }
    }

    /// Maps one inbound message.
    fn handle(&self, message: Message) -> Reaction {
        match message {
            Message::Welcome(welcome) => {
                if welcome.proto != VERSION {
                    self.stopping.store(true, Ordering::Release);
                    return Reaction::Fatal(format!(
                        "core speaks protocol v{} but musika-ui needs v{}",
                        welcome.proto, VERSION
                    ));
                }
                self.session.store(welcome.session, Ordering::Release);
                self.connected.store(true, Ordering::Release);
                self.push_structural(UiUpdate::Connected {
                    session: welcome.session,
                });
                Reaction::Continue
            }
            Message::Reject(reject) => {
                // A rejected handshake cannot be fixed by retrying.
                self.stopping.store(true, Ordering::Release);
                Reaction::Fatal(format!(
                    "core rejected the UI handshake ({:?}): {}",
                    reject.code, reject.message
                ))
            }
            Message::Bye(bye) => match bye.reason {
                ByeReason::Quit | ByeReason::Shutdown => {
                    self.stopping.store(true, Ordering::Release);
                    Reaction::Shutdown
                }
                ByeReason::VersionMismatch => {
                    self.stopping.store(true, Ordering::Release);
                    Reaction::Fatal("core protocol version mismatch".to_string())
                }
                ByeReason::Replaced => {
                    Reaction::Closed(Some("another UI window replaced this one".to_string()))
                }
                ByeReason::Protocol | ByeReason::SlowConsumer | ByeReason::Heartbeat => {
                    Reaction::Closed(Some(format!(
                        "core closed the connection ({:?})",
                        bye.reason
                    )))
                }
            },
            Message::ProtocolError(error) => Reaction::Closed(Some(format!(
                "protocol error {:?}: {}",
                error.code, error.message
            ))),
            Message::Ping(heartbeat) => {
                self.send_pong(heartbeat);
                Reaction::Continue
            }
            Message::Pong(_) => Reaction::Continue,
            Message::PlaybackState(state) => {
                self.push_coalescible(UiUpdate::State(state));
                Reaction::Continue
            }
            Message::QueueDelta(delta) => {
                self.push_structural(UiUpdate::QueueDelta(delta));
                Reaction::Continue
            }
            Message::QueueRows(rows) => {
                self.push_coalescible(UiUpdate::QueueRows(rows));
                Reaction::Continue
            }
            Message::PlayerReply(reply) => {
                self.push_structural(UiUpdate::PlayerReply(reply));
                Reaction::Continue
            }
            Message::Reply(reply) => {
                self.push_structural(UiUpdate::Reply(reply));
                Reaction::Continue
            }
            Message::Event(event) => {
                self.push_structural(UiUpdate::Event(event));
                Reaction::Continue
            }
            Message::Activate => {
                self.push_structural(UiUpdate::Activate);
                Reaction::Continue
            }
            // Client-to-core messages never arrive here.
            Message::Hello(_) | Message::Request(_) | Message::PlayerRequest(_) => {
                Reaction::Continue
            }
        }
    }
}

/// How the reader reacts to one inbound message.
enum Reaction {
    Continue,
    /// The transport is going away; the supervisor reconnects.
    Closed(Option<String>),
    /// Clean core close: report `UiUpdate::Shutdown` and stop.
    Shutdown,
    /// Unrecoverable protocol mismatch: report `UiUpdate::Fatal` and stop.
    Fatal(String),
}

/// Outcome of one served connection.
enum Served {
    /// The transport died or the core went away; the supervisor reconnects.
    Closed(Option<String>),
    /// No reconnect (`Bye::Quit`/`Shutdown`, rejected handshake).
    Stop,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Cloneable handle to the core connection. Sending never blocks the caller.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// Connects to the core, retrying while a freshly spawned daemon binds its
    /// socket. Returns the client and the update stream consumed by the root.
    pub fn connect() -> io::Result<(Self, Receiver<UiUpdate>)> {
        let socket = dial(CONNECT_ATTEMPTS, CONNECT_STEP)?;
        Ok(Self::spawn(Some(socket), RECONNECT_MIN_MS))
    }

    /// Starts disconnected and keeps retrying in the background (used when the
    /// core is not up yet; the root renders a retry surface until `Connected`).
    pub fn connect_when_available() -> (Self, Receiver<UiUpdate>) {
        Self::spawn(None, STARTUP_RETRY_MS)
    }

    fn spawn(socket: Option<UnixStream>, retry_ms: u64) -> (Self, Receiver<UiUpdate>) {
        let (updates, receiver) = tokio::sync::mpsc::channel(UPDATE_QUEUE_FRAMES);
        let inner = Arc::new(Inner::new(updates));
        let supervisor = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("musika-ipc-reader".to_string())
            .spawn(move || supervise(supervisor, socket, retry_ms))
            .expect("spawn ipc supervisor");
        (Self { inner }, receiver)
    }

    /// Sends one bridge (`RequestJson`) call and returns its request id.
    pub fn send_request(&self, method: &str, args: serde_json::Value) -> u32 {
        self.inner.send_request(method, args)
    }

    /// Sends one transport/queue op and returns its request id.
    pub fn send_player(&self, op: PlayerOp) -> u32 {
        self.inner.send_player(op)
    }

    /// Asks the core to replay the playback state and the queue rows.
    pub fn request_resync(&self) {
        self.inner.send_player(PlayerOp::Resync);
    }

    /// Asks the core to show/raise this window.
    ///
    /// Protocol v2 has no client frame for lifecycle requests yet, so this uses
    /// the core command socket (the same channel the tray/launcher uses, see
    /// `ipc::send_line`); the daemon answers with `KIND_ACTIVATE`, which the
    /// reader maps to [`UiUpdate::Activate`].
    pub fn show(&self) {
        let _ = crate::ipc::send_line("show");
    }

    /// Asks the core to quit. The core broadcasts `Bye { reason: Quit }` before
    /// closing, which the reader maps to [`UiUpdate::Shutdown`].
    pub fn shutdown_core(&self) {
        // One line on the bounded command socket; the write timeout keeps this
        // short enough for the quit path that calls it from the render loop.
        let _ = crate::ipc::send_line("quit");
    }

    /// Random id of the current daemon run (0 while disconnected).
    pub fn session(&self) -> u64 {
        self.inner.session.load(Ordering::Acquire)
    }

    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Acquire)
    }
}

fn dial(attempts: usize, step: Duration) -> io::Result<UnixStream> {
    let path = crate::ipc::daemon_socket_path();
    let mut last_error = None;
    for attempt in 0..attempts {
        match UnixStream::connect(&path) {
            Ok(socket) => return Ok(socket),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < attempts {
                    std::thread::sleep(step);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotConnected, "core daemon unavailable")
    }))
}

fn hello_frame() -> Frame {
    let hello = Hello {
        proto: VERSION,
        role: Role::Ui,
        client: "musika-ui".to_string(),
        features: UI_FEATURES,
    };
    let payload = ipc_protocol::json_encode(&hello).unwrap_or_default();
    Frame::new(ipc_protocol::KIND_HELLO, payload)
}

/// Owns the reader thread: serves one connection, then reconnects with
/// exponential backoff until told to stop.
fn supervise(inner: Arc<Inner>, initial: Option<UnixStream>, retry_ms: u64) {
    let mut socket = initial;
    let mut backoff = retry_ms;
    loop {
        if inner.stopping.load(Ordering::Acquire) {
            break;
        }
        let mut connected = match socket.take() {
            Some(socket) => socket,
            None => match reconnect(&inner, &mut backoff) {
                Some(socket) => socket,
                None => break,
            },
        };
        // The handshake is the first frame on every connection.
        let mut hello_writer = &connected;
        if let Err(error) = ipc_protocol::write_frame_blocking(&mut hello_writer, &hello_frame()) {
            inner.drop_connection();
            inner.push_structural(UiUpdate::Reconnecting(Some(format!(
                "handshake write failed: {error}"
            ))));
            std::thread::sleep(Duration::from_millis(backoff));
            backoff = (backoff * 2).min(RECONNECT_MAX_MS);
            continue;
        }
        if let Err(error) = inner.install_writer(&connected) {
            inner.drop_connection();
            inner.push_structural(UiUpdate::Reconnecting(Some(format!(
                "handshake setup failed: {error}"
            ))));
            std::thread::sleep(Duration::from_millis(backoff));
            backoff = (backoff * 2).min(RECONNECT_MAX_MS);
            continue;
        }
        inner.connected.store(true, Ordering::Release);
        let control = serve(&inner, &mut connected);
        inner.drop_connection();
        match control {
            Served::Stop => break,
            Served::Closed(reason) => {
                inner.push_structural(UiUpdate::Reconnecting(reason));
                backoff = RECONNECT_MIN_MS;
            }
        }
    }
    inner.drop_connection();
}

fn reconnect(inner: &Arc<Inner>, backoff: &mut u64) -> Option<UnixStream> {
    loop {
        if inner.stopping.load(Ordering::Acquire) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(*backoff));
        if inner.stopping.load(Ordering::Acquire) {
            return None;
        }
        match UnixStream::connect(crate::ipc::daemon_socket_path()) {
            Ok(socket) => {
                *backoff = RECONNECT_MIN_MS;
                return Some(socket);
            }
            Err(_) => {
                *backoff = (*backoff * 2).min(RECONNECT_MAX_MS);
            }
        }
    }
}

fn serve(inner: &Arc<Inner>, socket: &mut UnixStream) -> Served {
    loop {
        let mut reader = &*socket;
        let frame = match ipc_protocol::read_frame_blocking(&mut reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => return Served::Closed(Some("core closed the connection".to_string())),
            Err(error) => return Served::Closed(Some(error.to_string())),
        };
        let message = match Message::decode(&frame) {
            Ok(message) => message,
            Err(error) => {
                // A bad payload means the stream is no longer trustworthy.
                return Served::Closed(Some(format!(
                    "protocol error {:?}: {}",
                    error.code, error.message
                )));
            }
        };
        match inner.handle(message) {
            Reaction::Continue => {}
            Reaction::Closed(reason) => return Served::Closed(reason),
            Reaction::Shutdown => {
                inner.push_structural(UiUpdate::Shutdown);
                return Served::Stop;
            }
            Reaction::Fatal(message) => {
                inner.push_structural(UiUpdate::Fatal(message));
                return Served::Stop;
            }
        }
    }
}

fn write_loop(inner: Arc<Inner>, write_half: UnixStream, receiver: WriteReceiver<Frame>) {
    loop {
        match receiver.recv_timeout(WRITER_POLL) {
            Ok(frame) => {
                let mut writer = &write_half;
                if ipc_protocol::write_frame_blocking(&mut writer, &frame).is_err() {
                    break;
                }
                if inner.drain_outbound(&write_half).is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if inner.drain_outbound(&write_half).is_err() {
                    break;
                }
                if inner.stopping.load(Ordering::Acquire) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// RemoteBridge: the bridge surface, one `RequestJson` per call.
// ---------------------------------------------------------------------------

/// Bridge requests for the core (`RequestJson`; results travel as events).
#[derive(Clone)]
pub struct RemoteBridge {
    client: Client,
}

impl RemoteBridge {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn shutdown_core(&self) {
        self.client.shutdown_core();
    }
}

/// Generate the UI's request methods from the shared bridge RPC table: one
/// `RequestJson` frame per call, keyed by the method name.
macro_rules! bridge_client {
    ($( $name:ident ( $( $field:ident : $ty:ty ),* $(,)? ), )*) => {
        impl RemoteBridge {
            $(
                #[allow(clippy::too_many_arguments)]
                pub fn $name(&self $(, $field: $ty)*) -> u32 {
                    #[allow(unused_mut)]
                    let mut args = serde_json::Map::new();
                    $(
                        args.insert(
                            stringify!($field).to_string(),
                            json!($field),
                        );
                    )*
                    self.client
                        .send_request(stringify!($name), serde_json::Value::Object(args))
                }
            )*
        }
    };
}

crate::for_each_bridge_rpc!(bridge_client);

// ---------------------------------------------------------------------------
// PlaybackView: the read-only projection of core playback/queue state.
// ---------------------------------------------------------------------------

/// Read-only mirror of the core's transport and queue.
///
/// Every field is driven by `PlaybackState` / `QueueDelta` / `QueueRows`
/// frames; the UI never mutates it locally. `seq` and `rev` gaps set a resync
/// flag that the root drains with [`Self::take_resync`] and answers with
/// `Client::request_resync`.
pub struct PlaybackView {
    state: State,
    index: usize,
    count: usize,
    queue_rev: u64,
    volume: f64,
    muted: bool,
    repeat: Repeat,
    loading: bool,
    duration: f64,
    anchor: PlaybackState,
    rows: Vec<Track>,
    last_seq: u64,
    last_error: Option<String>,
    snapshot: std::cell::RefCell<Option<Arc<Vec<Track>>>>,
    resync: bool,
}

impl Default for PlaybackView {
    fn default() -> Self {
        Self {
            state: State::Stopped,
            index: 0,
            count: 0,
            queue_rev: 0,
            volume: crate::consts::DEFAULT_VOLUME,
            muted: false,
            repeat: Repeat::Off,
            loading: false,
            duration: 0.0,
            anchor: PlaybackState::default(),
            rows: Vec::new(),
            last_seq: 0,
            last_error: None,
            snapshot: std::cell::RefCell::new(None),
            resync: false,
        }
    }
}

impl PlaybackView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one transport snapshot. The anchor carries the position, so the
    /// UI extrapolates between frames instead of receiving ticks.
    pub fn apply_state(&mut self, state: PlaybackState) {
        // A `seq` jump means at least one state frame was lost (the update
        // channel drops coalescible frames under load): ask for a resync.
        if self.last_seq != 0 && state.seq > self.last_seq.saturating_add(1) {
            self.resync = true;
        }
        self.last_seq = state.seq;
        self.anchor = state;
        self.state = state.state().into();
        self.repeat = state.repeat().into();
        self.volume = state.volume();
        self.muted = state.muted();
        self.loading = state.loading();
        self.duration = state.duration();
        let index = state.index as usize;
        self.index = if self.count > 0 {
            index.min(self.count - 1)
        } else {
            index
        };
        self.snapshot.get_mut().take();
    }

    /// Applies one row-level queue change.
    pub fn apply_delta(&mut self, delta: QueueDelta) {
        if delta.rev < self.queue_rev {
            // Stale: the rows already moved past this revision.
            return;
        }
        if self.queue_rev != 0
            && delta.rev > self.queue_rev.saturating_add(1)
            && !matches!(delta.op, QueueOp::Reset { .. })
        {
            // A revision gap means a structural delta was lost; only the core
            // can restore the rows, so ask for a full resync.
            self.resync = true;
            return;
        }
        self.queue_rev = delta.rev;
        match delta.op {
            QueueOp::Reset { count } => {
                let count = count as usize;
                self.count = count;
                self.rows = vec![empty_track(); count];
                self.index = self.index.min(count.saturating_sub(1));
            }
            QueueOp::Rows { start, rows } => self.fill_rows(start as usize, rows),
            QueueOp::Insert { at, rows } => {
                let at = (at as usize).min(self.count);
                let added: Vec<Track> = rows.into_iter().map(Track::from).collect();
                let added_len = added.len();
                self.ensure_rows();
                for (offset, track) in added.into_iter().enumerate() {
                    self.rows.insert(at + offset, track);
                }
                self.count = self.count.saturating_add(added_len);
                if added_len > 0 && at <= self.index {
                    // Keep pointing at the same row: it shifted right.
                    self.index = self.index.saturating_add(added_len);
                }
                self.index = self.index.min(self.count.saturating_sub(1));
            }
            QueueOp::Remove { indices } => {
                let mut indices: Vec<usize> = indices
                    .into_iter()
                    .map(|index| index as usize)
                    .filter(|index| *index < self.count)
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                if indices.is_empty() {
                    return;
                }
                for index in indices.iter().rev() {
                    if *index < self.rows.len() {
                        self.rows.remove(*index);
                    }
                    if *index < self.index {
                        self.index -= 1;
                    }
                }
                self.count = self.count.saturating_sub(indices.len());
                self.rows.truncate(self.count);
                self.index = self.index.min(self.count.saturating_sub(1));
            }
            QueueOp::Move { from, to } => {
                let from = from as usize;
                let to = to as usize;
                if from >= self.count || to >= self.count {
                    return;
                }
                self.ensure_rows();
                let track = self.rows.remove(from);
                self.rows.insert(to, track);
                if self.index == from {
                    self.index = to;
                } else if from < self.index && to >= self.index {
                    self.index -= 1;
                } else if from > self.index && to <= self.index {
                    self.index += 1;
                }
            }
            QueueOp::Patch { index, favorite } => {
                if let Some(row) = self.rows.get_mut(index as usize) {
                    row.favorite = favorite;
                }
            }
        }
        self.snapshot.get_mut().take();
    }

    /// Applies one chunk of row bodies (`QueueRows`; bounded chunks).
    pub fn apply_rows(&mut self, rows: QueueRows) {
        if self.queue_rev != 0 && rows.rev < self.queue_rev {
            // Older body than the rows we already applied.
            return;
        }
        if rows.rev > self.queue_rev {
            self.queue_rev = rows.rev;
        }
        self.fill_rows(rows.start as usize, rows.rows);
        self.snapshot.get_mut().take();
    }

    /// Records the outcome of one `PlayerOp`. Results travel outside the reply,
    /// so this only tracks rejections.
    pub fn apply_player_reply(&mut self, reply: PlayerReply) {
        if reply.ok {
            self.last_error = None;
            return;
        }
        self.last_error = Some(format!(
            "core rejected a playback request ({:?})",
            reply.code
        ));
        if matches!(
            reply.code,
            ErrorCode::QueueRevMismatch | ErrorCode::SlowConsumer
        ) {
            self.resync = true;
        }
    }

    /// Drains the "ask the core for a fresh snapshot" flag.
    pub fn take_resync(&mut self) -> bool {
        std::mem::take(&mut self.resync)
    }

    // --- reads ------------------------------------------------------------

    pub fn state(&self) -> State {
        self.state
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// Number of queue rows known to the core.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The playing row, `None` while its body is still a placeholder (a
    /// `Reset` lands before the chunked `QueueRows` bodies do).
    pub fn current(&self) -> Option<&Track> {
        let row = self.rows.get(self.index)?;
        (!row.id.is_empty()).then_some(row)
    }

    pub fn queue(&self) -> &[Track] {
        &self.rows
    }

    /// Cached snapshot for render closures; rebuilt only when rows change.
    pub fn queue_snapshot(&self) -> Arc<Vec<Track>> {
        let mut cache = self.snapshot.borrow_mut();
        if let Some(snapshot) = cache.as_ref() {
            return Arc::clone(snapshot);
        }
        let snapshot = Arc::new(self.rows.clone());
        *cache = Some(Arc::clone(&snapshot));
        snapshot
    }

    /// The anchor position as reported by the core (no extrapolation).
    pub fn position(&self) -> f64 {
        self.anchor.position()
    }

    /// The extrapolated playhead for the current frame.
    pub fn position_now(&self) -> f64 {
        self.anchor.position_at(crate::linux::monotonic_ns())
    }

    pub fn duration(&self) -> f64 {
        if self.duration > 0.0 {
            self.duration
        } else {
            self.current().map(|track| track.duration).unwrap_or(0.0)
        }
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn queue_rev(&self) -> u64 {
        self.queue_rev
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn anchor(&self) -> PlaybackState {
        self.anchor
    }

    // --- internals --------------------------------------------------------

    fn ensure_rows(&mut self) {
        if self.rows.len() < self.count {
            self.rows.resize_with(self.count, empty_track);
        } else if self.rows.len() > self.count {
            self.rows.truncate(self.count);
        }
    }

    fn fill_rows(&mut self, start: usize, rows: Vec<WireTrack>) {
        if rows.is_empty() {
            return;
        }
        let end = start.saturating_add(rows.len());
        if end > self.count {
            self.count = end;
        }
        self.ensure_rows();
        for (offset, row) in rows.into_iter().enumerate() {
            if let Some(slot) = self.rows.get_mut(start + offset) {
                *slot = Track::from(row);
            }
        }
    }
}

/// True when `move_block(start, len, delta)` is a no-op or out of bounds; the
/// core still authorises the move, this only keeps the local cursor in sync.
pub fn move_block_allowed(count: usize, start: usize, len: usize, delta: i32) -> bool {
    if len == 0 || delta == 0 || start >= count {
        return false;
    }
    let end = (start + len).min(count);
    let block_len = end - start;
    let new_start = start as i64 + i64::from(delta);
    if new_start < 0 || new_start as usize + block_len > count {
        return false;
    }
    new_start as usize != start
}

// ---------------------------------------------------------------------------
// PlaybackClient: transport/queue ops as `PlayerOp` requests.
// ---------------------------------------------------------------------------

/// Emits transport/queue mutations to the core. Nothing is applied locally;
/// the authoritative result arrives as a `PlaybackState`/`QueueDelta` frame.
#[derive(Clone)]
pub struct PlaybackClient {
    client: Client,
}

impl PlaybackClient {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn send(&self, op: PlayerOp) {
        self.client.send_player(op);
    }

    pub fn toggle(&self) {
        self.send(PlayerOp::PlayPause);
    }
    pub fn pause(&self) {
        self.send(PlayerOp::Pause);
    }
    pub fn resume(&self) {
        self.send(PlayerOp::Play);
    }
    pub fn stop(&self) {
        self.send(PlayerOp::Stop);
    }
    pub fn next(&self) {
        self.send(PlayerOp::Next);
    }
    pub fn previous(&self) {
        self.send(PlayerOp::Previous);
    }
    pub fn set_volume(&self, volume: f64) {
        self.send(PlayerOp::SetVolume(volume.clamp(0.0, 100.0)));
    }
    pub fn volume_up(&self) {
        self.send(PlayerOp::VolumeUp);
    }
    pub fn volume_down(&self) {
        self.send(PlayerOp::VolumeDown);
    }
    pub fn toggle_mute(&self) {
        self.send(PlayerOp::ToggleMute);
    }
    pub fn set_muted(&self, muted: bool) {
        self.send(PlayerOp::SetMuted(muted));
    }
    pub fn seek(&self, seconds: f64) {
        self.send(PlayerOp::Seek(seconds.max(0.0)));
    }
    pub fn seek_relative(&self, offset: f64) {
        self.send(PlayerOp::SeekRelative(offset));
    }
    pub fn jump(&self, index: usize) {
        self.send(PlayerOp::Jump(index as u32));
    }
    pub fn move_item(&self, from: usize, to: usize) {
        self.send(PlayerOp::Move {
            from: from as u32,
            to: to as u32,
        });
    }
    pub fn move_block(&self, start: usize, len: usize, delta: i32) {
        self.send(PlayerOp::MoveBlock {
            start: start as u32,
            len: len as u32,
            delta,
        });
    }
    pub fn remove_many(&self, indices: &[usize]) -> usize {
        let indices: Vec<u32> = indices.iter().map(|index| *index as u32).collect();
        let count = indices.len();
        self.send(PlayerOp::RemoveMany(indices));
        count
    }
    pub fn cycle_repeat(&self) {
        self.send(PlayerOp::CycleRepeat);
    }
    pub fn shuffle_queue(&self) {
        self.send(PlayerOp::ShuffleQueue);
    }
    pub fn set_track_favorite(&self, song_id: &str, favorite: bool) {
        self.send(PlayerOp::SetTrackFavorite {
            id: song_id.to_string(),
            favorite,
        });
    }
}

// ---------------------------------------------------------------------------
// AppPlayer: the root's one player handle (read view + player-op client).
// ---------------------------------------------------------------------------

/// Root-facing player handle: reads come from [`PlaybackView`], mutations go
/// out as [`PlayerOp`] requests through [`PlaybackClient`].
pub struct AppPlayer {
    view: PlaybackView,
    client: PlaybackClient,
}

impl AppPlayer {
    pub fn new(client: Client) -> Self {
        Self {
            view: PlaybackView::new(),
            client: PlaybackClient::new(client),
        }
    }

    pub fn apply_state(&mut self, state: PlaybackState) {
        self.view.apply_state(state);
    }

    pub fn apply_delta(&mut self, delta: QueueDelta) {
        self.view.apply_delta(delta);
    }

    pub fn apply_rows(&mut self, rows: QueueRows) {
        self.view.apply_rows(rows);
    }

    pub fn apply_player_reply(&mut self, reply: PlayerReply) {
        self.view.apply_player_reply(reply);
    }

    pub fn take_resync(&mut self) -> bool {
        self.view.take_resync()
    }

    pub fn view(&self) -> &PlaybackView {
        &self.view
    }

    pub fn state(&self) -> State {
        self.view.state()
    }
    pub fn repeat(&self) -> Repeat {
        self.view.repeat()
    }
    pub fn index(&self) -> usize {
        self.view.index()
    }
    pub fn len(&self) -> usize {
        self.view.len()
    }
    pub fn is_empty(&self) -> bool {
        self.view.is_empty()
    }
    pub fn current(&self) -> Option<&Track> {
        self.view.current()
    }
    pub fn queue(&self) -> &[Track] {
        self.view.queue()
    }
    pub fn queue_snapshot(&self) -> Arc<Vec<Track>> {
        self.view.queue_snapshot()
    }
    pub fn position(&self) -> f64 {
        self.view.position()
    }
    pub fn position_now(&self) -> f64 {
        self.view.position_now()
    }
    pub fn duration(&self) -> f64 {
        self.view.duration()
    }
    pub fn volume(&self) -> f64 {
        self.view.volume()
    }
    pub fn muted(&self) -> bool {
        self.view.muted()
    }
    pub fn is_loading(&self) -> bool {
        self.view.is_loading()
    }
    pub fn last_error(&self) -> Option<&str> {
        self.view.last_error()
    }

    pub fn toggle(&self) {
        self.client.toggle();
    }
    pub fn pause(&self) {
        self.client.pause();
    }
    pub fn resume(&self) {
        self.client.resume();
    }
    pub fn stop(&self) {
        self.client.stop();
    }
    pub fn next(&self) {
        self.client.next();
    }
    pub fn previous(&self) {
        self.client.previous();
    }
    pub fn set_volume(&self, volume: f64) {
        self.client.set_volume(volume);
    }
    pub fn volume_up(&self) {
        self.client.volume_up();
    }
    pub fn volume_down(&self) {
        self.client.volume_down();
    }
    pub fn toggle_mute(&self) {
        self.client.toggle_mute();
    }
    pub fn set_muted(&self, muted: bool) {
        self.client.set_muted(muted);
    }
    pub fn seek(&self, seconds: f64) {
        self.client.seek(seconds);
    }
    /// Relative seek; the core resolves the math authoritatively.
    pub fn seek_relative(&self, offset: f64) {
        self.client.seek_relative(offset);
    }
    pub fn jump(&self, index: usize) {
        self.client.jump(index);
    }
    pub fn move_item(&self, from: usize, to: usize) {
        self.client.move_item(from, to);
    }
    /// Sends a block move when it is possible for the current queue; returns
    /// whether the cursor should follow the move.
    pub fn move_block(&self, start: usize, len: usize, delta: i32) -> bool {
        if !move_block_allowed(self.view.len(), start, len, delta) {
            return false;
        }
        self.client.move_block(start, len, delta);
        true
    }
    pub fn remove_many(&self, indices: &[usize]) -> usize {
        let count = self.view.len();
        let indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| *index < count)
            .collect();
        self.client.remove_many(&indices)
    }
    pub fn cycle_repeat(&self) {
        self.client.cycle_repeat();
    }
    pub fn shuffle_queue(&self) {
        self.client.shuffle_queue();
    }
    pub fn set_track_favorite(&self, song_id: &str, favorite: bool) {
        self.client.set_track_favorite(song_id, favorite);
    }
}
