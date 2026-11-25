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
use crate::player::{Normalization, Repeat, State, Track};

/// How long `Client::connect` waits for the freshly spawned daemon to bind.
const CONNECT_STEP: Duration = Duration::from_millis(50);
const CONNECT_ATTEMPTS: usize = 100;

/// Reconnect backoff floor/ceiling after a transport error or a `Bye`.
const RECONNECT_MIN_MS: u64 = 50;
const RECONNECT_MAX_MS: u64 = 1_000;
/// Retry cadence for the "core is not up yet" startup surface.
const STARTUP_RETRY_MS: u64 = 1_000;

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
    Event(Box<Event>),
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
    /// Wake handle for the live writer thread; `None` while disconnected.
    writer: Option<SyncSender<()>>,
    /// Latest coalesced volume frame (`PlayerOp::SetVolume`).
    pending_volume: Option<Frame>,
    /// Latest coalesced absolute seek frame (`PlayerOp::Seek`).
    pending_seek: Option<Frame>,
    /// Accumulated relative seek request id and delta (`PlayerOp::SeekRelative`).
    pending_seek_relative: Option<(u32, f64)>,
    /// Frames that must not be dropped (transport, requests, pongs) kept while
    /// the transport is down or waiting to drain.
    overflow: VecDeque<Frame>,
}

enum OutboundItem {
    Volume(Frame),
    Seek(Frame),
    SeekRelative { req: u32, delta: f64, frame: Frame },
    Overflow(Frame),
}

impl OutboundItem {
    fn frame(&self) -> &Frame {
        match self {
            Self::Volume(frame)
            | Self::Seek(frame)
            | Self::SeekRelative { frame, .. }
            | Self::Overflow(frame) => frame,
        }
    }
}

/// Wakes the writer thread without blocking the caller.
fn wake_writer(writer: &mut Option<SyncSender<()>>) {
    let Some(sender) = writer.take() else {
        return;
    };
    match sender.try_send(()) {
        Ok(()) | Err(WriteTrySendError::Full(())) => {
            *writer = Some(sender);
        }
        Err(WriteTrySendError::Disconnected(())) => {}
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                pending_volume: None,
                pending_seek: None,
                pending_seek_relative: None,
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

    /// Enqueues one ordered non-coalescible frame.
    fn send_frame(&self, frame: Frame) {
        let mut out = lock(&self.outbound);
        if out.overflow.len() >= OVERFLOW_FRAMES {
            crate::debug_log!("ipc: outbound overflow is full; dropping the oldest frame");
            out.overflow.pop_front();
        }
        out.overflow.push_back(frame);
        wake_writer(&mut out.writer);
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
            Ok(payload) => self.send_frame(Frame::new(ipc_protocol::KIND_REQUEST_JSON, payload)),
            Err(error) => crate::debug_log!(
                "ipc: could not encode bridge request {method}: {:?}",
                error.code
            ),
        }
        req
    }

    /// Encodes a player-operation frame. A batch that cannot be represented on
    /// the wire (more than `u16::MAX` removal indices, D-09) is logged and
    /// dropped instead of silently truncating.
    fn player_frame(&self, request: &PlayerRequest) -> Option<Frame> {
        match request.encode() {
            Ok(payload) => Some(Frame::new(ipc_protocol::KIND_PLAYER_REQUEST, payload)),
            Err(error) => {
                crate::debug_log!(
                    "ipc: dropped an unencodable player request: {:?} {}",
                    error.code,
                    error.message
                );
                None
            }
        }
    }

    /// Sends one `PlayerOp` and returns its request id.
    ///
    /// - `SetVolume` coalesces only with prior `SetVolume`.
    /// - `Seek` coalesces only with prior `Seek`.
    /// - `SeekRelative` accumulates (`delta_a + delta_b`) across pending taps.
    fn send_player(&self, op: PlayerOp) -> u32 {
        let req = self.next_req();
        match op {
            PlayerOp::SetVolume(_) => {
                let request = PlayerRequest { req, op };
                if let Some(frame) = self.player_frame(&request) {
                    let mut out = lock(&self.outbound);
                    out.pending_volume = Some(frame);
                    wake_writer(&mut out.writer);
                }
            }
            PlayerOp::Seek(_) => {
                let request = PlayerRequest { req, op };
                if let Some(frame) = self.player_frame(&request) {
                    let mut out = lock(&self.outbound);
                    out.pending_seek = Some(frame);
                    wake_writer(&mut out.writer);
                }
            }
            PlayerOp::SeekRelative(delta) => {
                let mut out = lock(&self.outbound);
                let total = out
                    .pending_seek_relative
                    .map_or(delta, |(_, prev)| prev + delta);
                out.pending_seek_relative = Some((req, total));
                wake_writer(&mut out.writer);
            }
            PlayerOp::PlayPause
            | PlayerOp::Play
            | PlayerOp::Pause
            | PlayerOp::Stop
            | PlayerOp::Next
            | PlayerOp::Previous
            | PlayerOp::VolumeUp
            | PlayerOp::VolumeDown
            | PlayerOp::ToggleMute
            | PlayerOp::SetMuted(_)
            | PlayerOp::Jump(_)
            | PlayerOp::Remove(_)
            | PlayerOp::RemoveMany(_)
            | PlayerOp::Move { .. }
            | PlayerOp::MoveBlock { .. }
            | PlayerOp::CycleRepeat
            | PlayerOp::ShuffleQueue
            | PlayerOp::ShuffleQueueOrder
            | PlayerOp::SetTrackFavorite { .. }
            | PlayerOp::CycleNormalization
            | PlayerOp::SetNormalization(_)
            | PlayerOp::Resync => {
                let request = PlayerRequest { req, op };
                if let Some(frame) = self.player_frame(&request) {
                    self.send_frame(frame);
                }
            }
        }
        req
    }

    fn send_pong(&self, heartbeat: Heartbeat) {
        let pong = Heartbeat {
            nonce: heartbeat.nonce,
            monotonic_ns: crate::linux::monotonic_ns(),
        };
        self.send_frame(Frame::new(ipc_protocol::KIND_PONG, pong.encode()));
    }

    /// Structural updates are never dropped: the reader thread may block, the
    /// GPUI thread never does.
    fn push_structural(&self, update: UiUpdate) {
        match self.updates.try_send(update) {
            Ok(()) => {}
            Err(UpdateTrySendError::Full(update)) => {
                _ = self.updates.blocking_send(update);
            }
            Err(UpdateTrySendError::Closed(_)) => {}
        }
    }

    /// Coalescible updates (state / queue rows) are dropped when the UI is
    /// behind, then repaired: a lost row chunk asks for a resync, a lost state
    /// leaves a `seq` gap that also asks for one.
    fn push_coalescible(&self, update: UiUpdate) {
        if let Err(UpdateTrySendError::Full(update)) = self.updates.try_send(update)
            && matches!(update, UiUpdate::QueueRows(_))
        {
            self.send_player(PlayerOp::Resync);
        }
    }

    /// Installs the writer thread for a fresh socket, keeping queued frames so
    /// requests issued while disconnected or pre-handshake are delivered after
    /// `Welcome`.
    fn install_writer(self: &Arc<Self>, socket: &UnixStream) -> io::Result<()> {
        let write_half = socket.try_clone()?;
        let (sender, receiver) = sync_channel::<()>(1);
        {
            let mut out = lock(&self.outbound);
            out.writer = Some(sender);
        }
        let inner = Arc::clone(self);
        #[expect(
            clippy::expect_used,
            reason = "without the writer thread the UI cannot talk to the core at all; failure is a fatal startup invariant"
        )]
        let _writer = std::thread::Builder::new()
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
        out.overflow
            .retain(|frame| frame.kind != ipc_protocol::KIND_PONG);
    }

    fn pop_outbound_item(&self) -> Option<OutboundItem> {
        if !self.connected.load(Ordering::Acquire) {
            return None;
        }
        let mut out = lock(&self.outbound);
        if let Some(frame) = out.pending_volume.take() {
            return Some(OutboundItem::Volume(frame));
        }
        if let Some(frame) = out.pending_seek.take() {
            return Some(OutboundItem::Seek(frame));
        }
        if let Some((req, delta)) = out.pending_seek_relative.take() {
            let request = PlayerRequest {
                req,
                op: PlayerOp::SeekRelative(delta),
            };
            let frame = self.player_frame(&request)?;
            return Some(OutboundItem::SeekRelative { req, delta, frame });
        }
        out.overflow.pop_front().map(OutboundItem::Overflow)
    }

    fn requeue_outbound_item(&self, item: OutboundItem) {
        let mut out = lock(&self.outbound);
        match item {
            OutboundItem::Volume(frame) => {
                if out.pending_volume.is_none() {
                    out.pending_volume = Some(frame);
                }
            }
            OutboundItem::Seek(frame) => {
                if out.pending_seek.is_none() {
                    out.pending_seek = Some(frame);
                }
            }
            OutboundItem::SeekRelative { req, delta, .. } => match out.pending_seek_relative {
                Some((newer_req, newer_delta)) => {
                    out.pending_seek_relative = Some((newer_req, delta + newer_delta));
                }
                None => {
                    out.pending_seek_relative = Some((req, delta));
                }
            },
            OutboundItem::Overflow(frame) => {
                if frame.kind != ipc_protocol::KIND_PONG {
                    out.overflow.push_front(frame);
                }
            }
        }
    }

    /// Writes coalesced slots and the overflow once the handshake has completed.
    /// If a socket write fails mid-drain, the unsent frame is re-queued for the
    /// next connection.
    fn drain_outbound(&self, write_half: &UnixStream) -> io::Result<()> {
        while let Some(item) = self.pop_outbound_item() {
            let mut writer = write_half;
            if let Err(error) = ipc_protocol::write_frame_blocking(&mut writer, item.frame()) {
                self.requeue_outbound_item(item);
                return Err(error);
            }
        }
        Ok(())
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
                {
                    let mut out = lock(&self.outbound);
                    wake_writer(&mut out.writer);
                }
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
                ByeReason::Quit | ByeReason::Shutdown | ByeReason::Replaced => {
                    self.stopping.store(true, Ordering::Release);
                    Reaction::Shutdown
                }
                ByeReason::VersionMismatch => {
                    self.stopping.store(true, Ordering::Release);
                    Reaction::Fatal("core protocol version mismatch".to_string())
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
                self.push_structural(UiUpdate::Event(Box::new(event)));
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
        #[expect(
            clippy::expect_used,
            reason = "without the reader/supervisor thread the UI never receives core updates; failure is a fatal startup invariant"
        )]
        let _reader = std::thread::Builder::new()
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

    /// Asks the core to quit. The core broadcasts `Bye { reason: Quit }` before
    /// closing, which the reader maps to [`UiUpdate::Shutdown`].
    pub fn shutdown_core(&self) {
        // One line on the bounded command socket; the write timeout keeps this
        // short enough for the quit path that calls it from the render loop.
        _ = crate::ipc::send_line("quit");
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
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "core daemon unavailable")))
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
        let connected = match socket.take() {
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
            backoff = (backoff * 2).min(RECONNECT_MAX_MS);
            continue;
        }
        if let Err(error) = inner.install_writer(&connected) {
            inner.drop_connection();
            inner.push_structural(UiUpdate::Reconnecting(Some(format!(
                "handshake setup failed: {error}"
            ))));
            backoff = (backoff * 2).min(RECONNECT_MAX_MS);
            continue;
        }
        let control = serve(&inner, &connected);
        let welcomed = inner.connected.load(Ordering::Acquire);
        inner.drop_connection();
        match control {
            Served::Stop => break,
            Served::Closed(reason) => {
                inner.push_structural(UiUpdate::Reconnecting(reason));
                if welcomed {
                    backoff = RECONNECT_MIN_MS;
                } else {
                    backoff = (backoff * 2).min(RECONNECT_MAX_MS);
                }
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
                return Some(socket);
            }
            Err(_) => {
                *backoff = (*backoff * 2).min(RECONNECT_MAX_MS);
            }
        }
    }
}

fn serve(inner: &Arc<Inner>, socket: &UnixStream) -> Served {
    loop {
        let mut reader = socket;
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

fn write_loop(inner: Arc<Inner>, write_half: UnixStream, receiver: WriteReceiver<()>) {
    loop {
        match receiver.recv_timeout(WRITER_POLL) {
            Ok(()) => {
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
        #[expect(
            dead_code,
            reason = "generated methods mirror the shared RPC table: a request the UI does not send yet is still protocol surface"
        )]
        impl RemoteBridge {
            $(
                #[allow(
                    clippy::too_many_arguments,
                    reason = "generated signatures mirror the RPC parameter list"
                )]
                pub fn $name(&self $(, $field: $ty)*) -> u32 {
                    #[allow(
                        unused_mut,
                        reason = "generated methods with no arguments do not need the mutation that others do"
                    )]
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
    normalization: Normalization,
    shuffle: bool,
    loading: bool,
    retrying: bool,
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
            normalization: Normalization::Off,
            shuffle: false,
            loading: false,
            retrying: false,
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

    /// Resets per-session sequence and revision counters on reconnect so frames
    /// from a restarted core daemon (whose counters restart at 1) are accepted.
    pub fn reset_for_reconnect(&mut self) {
        self.last_seq = 0;
        self.queue_rev = 0;
        self.resync = false;
        self.last_error = None;
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
        self.normalization = Normalization::from_code(state.normalization_code());
        self.shuffle = state.shuffle();
        self.volume = state.volume();
        self.muted = state.muted();
        self.loading = state.loading();
        self.retrying = state.retrying();
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
                let was_empty = self.count == 0;
                let at = (at as usize).min(self.count);
                let added: Vec<Track> = rows.into_iter().map(Track::from).collect();
                let added_len = added.len();
                self.ensure_rows();
                for (offset, track) in added.into_iter().enumerate() {
                    self.rows.insert(at + offset, track);
                }
                self.count = self.count.saturating_add(added_len);
                if was_empty {
                    self.index = 0;
                } else if added_len > 0 && at <= self.index {
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

    pub fn normalization(&self) -> Normalization {
        self.normalization
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the GPUI client tests read this projection")
    )]
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

    pub fn is_retrying(&self) -> bool {
        self.retrying
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the GPUI client tests read this projection")
    )]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the GPUI client tests read this projection")
    )]
    pub fn queue_rev(&self) -> u64 {
        self.queue_rev
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only the GPUI client tests read this projection")
    )]
    pub fn last_seq(&self) -> u64 {
        self.last_seq
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
    pub fn cycle_normalization(&self) {
        self.send(PlayerOp::CycleNormalization);
    }
    pub fn set_normalization(&self, mode: Normalization) {
        self.send(PlayerOp::SetNormalization(mode.code()));
    }
    pub fn shuffle_queue(&self) {
        self.send(PlayerOp::ShuffleQueue);
    }
    pub fn shuffle_queue_order(&self) {
        self.send(PlayerOp::ShuffleQueueOrder);
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

    pub fn reset_for_reconnect(&mut self) {
        self.view.reset_for_reconnect();
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

    pub fn state(&self) -> State {
        self.view.state()
    }
    pub fn repeat(&self) -> Repeat {
        self.view.repeat()
    }
    pub fn shuffle(&self) -> bool {
        self.view.shuffle()
    }
    pub fn normalization(&self) -> Normalization {
        self.view.normalization()
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
    pub fn is_retrying(&self) -> bool {
        self.view.is_retrying()
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
    pub fn cycle_normalization(&self) {
        self.client.cycle_normalization();
    }
    pub fn set_normalization(&self, mode: Normalization) {
        self.client.set_normalization(mode);
    }
    pub fn shuffle_queue(&self) {
        self.client.shuffle_queue();
    }
    pub fn shuffle_queue_order(&self) {
        self.client.shuffle_queue_order();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc_protocol::{Bye, StateWire, Welcome};

    fn test_row(id: &str) -> WireTrack {
        WireTrack {
            id: id.to_string(),
            title: format!("Title {id}"),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            album_id: "album-1".to_string(),
            favorite: false,
            duration: 180.0,
            cover_art: "cover-1".to_string(),
            cover_url: String::new(),
            cover_remote: String::new(),
        }
    }

    #[test]
    fn reconnect_resets_session_counters() {
        let mut view = PlaybackView::new();
        view.apply_delta(QueueDelta {
            rev: 12,
            op: QueueOp::Reset { count: 2 },
        });
        view.apply_rows(QueueRows {
            rev: 12,
            start: 0,
            rows: vec![test_row("old-a"), test_row("old-b")],
        });
        view.apply_state(PlaybackState {
            seq: 20,
            index: 1,
            state: StateWire::Playing as u8,
            ..PlaybackState::default()
        });
        assert_eq!(view.last_seq(), 20);
        assert_eq!(view.queue_rev(), 12);

        // Reconnecting to a restarted daemon resets counters so rev=1 / seq=1 apply.
        view.reset_for_reconnect();
        assert_eq!(view.last_seq(), 0);
        assert_eq!(view.queue_rev(), 0);
        assert!(!view.take_resync());

        view.apply_delta(QueueDelta {
            rev: 1,
            op: QueueOp::Reset { count: 1 },
        });
        view.apply_rows(QueueRows {
            rev: 1,
            start: 0,
            rows: vec![test_row("new-a")],
        });
        view.apply_state(PlaybackState {
            seq: 1,
            index: 0,
            state: StateWire::Paused as u8,
            ..PlaybackState::default()
        });
        assert_eq!(view.queue_rev(), 1);
        assert_eq!(view.last_seq(), 1);
        assert_eq!(view.len(), 1);
        assert_eq!(view.current().map(|t| t.id.as_str()), Some("new-a"));
        assert!(!view.take_resync());
    }

    #[test]
    fn insert_into_empty_queue_sets_index_zero() {
        let mut view = PlaybackView::new();
        assert!(view.is_empty());
        assert_eq!(view.index(), 0);

        view.apply_delta(QueueDelta {
            rev: 1,
            op: QueueOp::Insert {
                at: 0,
                rows: vec![test_row("a"), test_row("b"), test_row("c")],
            },
        });
        assert_eq!(view.len(), 3);
        assert_eq!(view.index(), 0);
        assert_eq!(view.current().map(|t| t.id.as_str()), Some("a"));
    }

    #[test]
    fn bye_replaced_stops_reconnect() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let inner = Inner::new(tx);
        let reaction = inner.handle(Message::Bye(Bye {
            reason: ByeReason::Replaced,
        }));
        assert!(matches!(reaction, Reaction::Shutdown));
        assert!(inner.stopping.load(Ordering::Acquire));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn coalescing_preserves_seek_accumulates_relative_and_waits_for_welcome() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let inner = Inner::new(tx);
        let (write_half, mut read_half) = UnixStream::pair().expect("unix pair");
        read_half
            .set_nonblocking(true)
            .expect("nonblocking read_half");

        // Queue commands before Welcome arrives.
        inner.send_player(PlayerOp::Seek(42.5));
        inner.send_player(PlayerOp::SetVolume(80.0));
        inner.send_player(PlayerOp::SetVolume(35.0));
        inner.send_player(PlayerOp::SeekRelative(5.0));
        inner.send_player(PlayerOp::SeekRelative(10.0));
        inner.send_player(PlayerOp::SeekRelative(-2.5));
        inner.send_request("refresh_header", serde_json::json!({}));

        // Before Welcome (`connected == false`), drain_outbound must write nothing.
        inner
            .drain_outbound(&write_half)
            .expect("pre-welcome drain");
        let mut buf = [0u8; 16];
        assert_eq!(
            io::Read::read(&mut read_half, &mut buf)
                .expect_err("no bytes before welcome")
                .kind(),
            io::ErrorKind::WouldBlock
        );

        // Simulate mid-drain write failure after Welcome by closing a broken socket pair.
        let reaction = inner.handle(Message::Welcome(Welcome {
            proto: VERSION,
            core_version: "test".to_string(),
            session: 7,
            features: UI_FEATURES,
        }));
        assert!(matches!(reaction, Reaction::Continue));
        assert!(inner.connected.load(Ordering::Acquire));

        let (broken_write, broken_read) = UnixStream::pair().expect("broken pair");
        drop(broken_read);
        assert!(inner.drain_outbound(&broken_write).is_err());

        // Now drain onto the live socket and verify all coalesced/requeued frames arrive.
        read_half
            .set_nonblocking(false)
            .expect("blocking read_half");
        inner
            .drain_outbound(&write_half)
            .expect("post-welcome drain");
        drop(write_half);

        let mut decoded = Vec::new();
        while let Some(frame) =
            ipc_protocol::read_frame_blocking(&mut read_half).expect("read frame")
        {
            decoded.push(Message::decode(&frame).expect("decode message"));
        }
        assert_eq!(decoded.len(), 4);
        assert!(matches!(
            &decoded[0],
            Message::PlayerRequest(PlayerRequest {
                op: PlayerOp::SetVolume(v),
                ..
            }) if (*v - 35.0).abs() < 1e-6
        ));
        assert!(matches!(
            &decoded[1],
            Message::PlayerRequest(PlayerRequest {
                op: PlayerOp::Seek(s),
                ..
            }) if (*s - 42.5).abs() < 1e-6
        ));
        assert!(matches!(
            &decoded[2],
            Message::PlayerRequest(PlayerRequest {
                op: PlayerOp::SeekRelative(d),
                ..
            }) if (*d - 12.5).abs() < 1e-6
        ));
        assert!(matches!(
            &decoded[3],
            Message::Request(RequestJson { method, .. }) if method == "refresh_header"
        ));
    }
}
