//! Playback service: one thread owns the queue policy and the audio engine.
//!
//! The daemon never polls playback. It sends typed [`PlaybackInput`]s and
//! receives [`PlaybackOutput`]s; the thread sleeps on its channel and wakes on
//! commands or engine events. A published [`Snapshot`] gives any daemon task an
//! O(1) read of the current transport state and queue (`Arc`), so MPRIS, the
//! status flush and new-UI bootstrap never touch the playback thread.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::ipc_protocol::{ErrorCode, PlaybackState, PlayerOp, StateWire};
use crate::player::core::{CoreCommand, Outcome, PlaybackCore};
use crate::player::engine::{Engine, EngineEvent};

/// How often the playback thread evaluates its clock while active/paused.
const ACTIVE_TICK: Duration = Duration::from_millis(25);
const IDLE_TICK: Duration = Duration::from_millis(250);
/// A playing clock is republished at least this often so UI extrapolation
/// cannot drift for long (this is a correction, not the old 10 Hz position
/// stream: between corrections the UI computes the position locally).
const CLOCK_CORRECTION: Duration = Duration::from_secs(5);
/// Minimum spacing between engine start attempts when PipeWire is unavailable.
const ENGINE_RETRY: Duration = Duration::from_secs(5);

/// Inputs accepted by the playback thread.
pub(crate) enum PlaybackInput {
    /// A UI client's typed transport/queue request.
    Ui { conn: u64, req: u32, op: PlayerOp },
    /// A request from the core itself (text `ipc.sock`, MPRIS, tray).
    Control { op: CoreCommand },
    /// Replace the queue from a bridge event/restore.
    Load {
        tracks: Vec<crate::player::Track>,
        start: usize,
        paused: bool,
        position: Option<f64>,
        volume: Option<f64>,
        muted: Option<bool>,
    },
    /// Re-send the full playback state and queue to every UI client.
    Resync { conn: u64, req: u32 },
    Shutdown,
}

impl std::fmt::Debug for PlaybackInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ui { conn, req, op } => {
                write!(formatter, "Ui {{ conn: {conn}, req: {req}, op: {op:?} }}")
            }
            Self::Control { op } => write!(formatter, "Control {{ op: {op:?} }}"),
            Self::Load { start, paused, .. } => {
                write!(formatter, "Load {{ start: {start}, paused: {paused} }}")
            }
            Self::Resync { conn, req } => {
                write!(formatter, "Resync {{ conn: {conn}, req: {req} }}")
            }
            Self::Shutdown => write!(formatter, "Shutdown"),
        }
    }
}

enum ServiceInput {
    Command(PlaybackInput),
    Engine(EngineEvent),
}

/// Outputs the daemon turns into frames/events.
#[derive(Debug)]
pub(crate) enum PlaybackOutput {
    Ack {
        conn: Option<u64>,
        req: u32,
        ok: bool,
        code: ErrorCode,
        applied_seq: u64,
    },
    State(PlaybackState),
    Delta(crate::ipc_protocol::QueueDelta),
    Reset {
        rev: u64,
        queue: Arc<Vec<crate::player::Track>>,
    },
    Error(String),
    EngineUnavailable(String),
}

/// Read-only projection of the playback thread's state.
#[derive(Default)]
pub(crate) struct Snapshot {
    pub state: PlaybackState,
    pub queue: Arc<Vec<crate::player::Track>>,
    pub queue_rev: u64,
    pub last_error: Option<String>,
    pub engine_available: bool,
}

/// Handle to the playback thread.
pub(crate) struct PlaybackService {
    tx: Sender<ServiceInput>,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
    join: Option<JoinHandle<()>>,
}

impl PlaybackService {
    /// Spawns the playback thread. `outputs` is drained by the daemon loop.
    pub(crate) fn spawn(outputs: UnboundedSender<PlaybackOutput>) -> Self {
        let (tx, rx) = mpsc::channel::<ServiceInput>();
        let engine_tx = tx.clone();
        let snapshot = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let thread_snapshot = Arc::clone(&snapshot);
        #[expect(
            clippy::expect_used,
            reason = "the daemon cannot play audio without its playback thread; a failed spawn is a fatal startup invariant"
        )]
        let join = thread::Builder::new()
            .name("musika-playback".to_string())
            .spawn(move || run(rx, engine_tx, outputs, thread_snapshot))
            .expect("spawn playback thread");
        Self {
            tx,
            snapshot,
            join: Some(join),
        }
    }

    pub(crate) fn send(&self, input: PlaybackInput) -> bool {
        self.tx.send(ServiceInput::Command(input)).is_ok()
    }

    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(
            &self
                .snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Stops the thread. Bounded: the thread exits on `Shutdown` immediately.
    pub(crate) fn shutdown(mut self) {
        _ = self.tx.send(ServiceInput::Command(PlaybackInput::Shutdown));
        if let Some(join) = self.join.take() {
            _ = join.join();
        }
    }
}

/// Cloneable control handle: send inputs and read the snapshot from any thread.
#[derive(Clone)]
pub(crate) struct PlaybackControl {
    tx: Sender<ServiceInput>,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
}

impl PlaybackControl {
    pub(crate) fn send(&self, input: PlaybackInput) -> bool {
        self.tx.send(ServiceInput::Command(input)).is_ok()
    }

    pub(crate) fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(
            &self
                .snapshot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

impl PlaybackService {
    pub(crate) fn control(&self) -> PlaybackControl {
        PlaybackControl {
            tx: self.tx.clone(),
            snapshot: Arc::clone(&self.snapshot),
        }
    }
}

impl Drop for PlaybackService {
    fn drop(&mut self) {
        _ = self.tx.send(ServiceInput::Command(PlaybackInput::Shutdown));
        if let Some(join) = self.join.take() {
            _ = join.join();
        }
    }
}

struct ServiceState {
    core: PlaybackCore,
    engine: Option<Engine>,
    forwarder: Option<JoinHandle<()>>,
    service_tx: Sender<ServiceInput>,
    outputs: UnboundedSender<PlaybackOutput>,
    last_engine_attempt: Option<Instant>,
    last_error: Option<String>,
}

impl ServiceState {
    /// Starts the engine if it is not running. Returns `true` when an engine is
    /// available. Never blocks the daemon: this only runs on the playback
    /// thread (startup warm-up included).
    fn ensure_engine(&mut self) -> bool {
        if self.engine.is_some() {
            return true;
        }
        if self
            .last_engine_attempt
            .is_some_and(|attempt| attempt.elapsed() < ENGINE_RETRY)
        {
            return false;
        }
        self.last_engine_attempt = Some(Instant::now());
        let (event_tx, event_rx) = mpsc::channel::<EngineEvent>();
        match Engine::new(event_tx) {
            Ok(engine) => {
                let service_tx = self.service_tx.clone();
                self.forwarder = match thread::Builder::new()
                    .name("musika-engine-events".to_string())
                    .spawn(move || {
                        for event in event_rx {
                            if service_tx.send(ServiceInput::Engine(event)).is_err() {
                                break;
                            }
                        }
                    }) {
                    Ok(join) => Some(join),
                    Err(error) => {
                        crate::debug_log!("playback: cannot start engine forwarder: {error}");
                        None
                    }
                };
                self.engine = Some(engine);
                crate::debug_log!("playback: engine ready");
                true
            }
            Err(error) => {
                _ = self.outputs.send(PlaybackOutput::EngineUnavailable(error.clone()));
                crate::debug_log!("playback: engine unavailable: {error}");
                false
            }
        }
    }

    fn drop_engine(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        if let Some(forwarder) = self.forwarder.take() {
            _ = forwarder.join();
        }
    }

    fn send_audio(&self, outcome: &Outcome) {
        if let Some(engine) = self.engine.as_ref() {
            for command in outcome.audio.iter().cloned() {
                engine.send(command);
            }
        }
    }

    /// Re-applies volume/mute to a freshly started engine: a new PipeWire sink
    /// begins at unity gain and unmuted, while the model keeps the user's
    /// volume. Called after engine startup and after a `SinkFailed` rebuild.
    fn sync_audio(&self) {
        if let Some(engine) = self.engine.as_ref() {
            for command in self.core.audio_sync() {
                engine.send(command);
            }
        }
    }
}

fn run(
    rx: Receiver<ServiceInput>,
    engine_tx: Sender<ServiceInput>,
    outputs: UnboundedSender<PlaybackOutput>,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
) {
    let core = PlaybackCore::new();
    let mut service = ServiceState {
        core,
        engine: None,
        forwarder: None,
        service_tx: engine_tx,
        outputs,
        last_engine_attempt: None,
        last_error: None,
    };
    let mut published = PlaybackState {
        rate_milli: 1000,
        ..PlaybackState::default()
    };
    let mut published_at = Instant::now();
    let mut next_seq = 1u64;
    // Publish the empty state before warming the engine: a client may attach
    // while PipeWire is still connecting and must never read a zeroed snapshot.
    store_snapshot(&snapshot, &service, &published);

    // Warm the engine at startup (on this thread), exactly like the retired
    // `warm_native()` did, so the first play does not pay the PipeWire
    // connection cost. A failure is reported and retried on demand.
    service.ensure_engine();
    service.sync_audio();
    publish_now(&mut service, &mut published, &mut published_at, &mut next_seq);
    store_snapshot(&snapshot, &service, &published);

    loop {
        let active = published.state() == StateWire::Playing || published.loading();
        let timeout = if active { ACTIVE_TICK } else { IDLE_TICK };
        let input = match rx.recv_timeout(timeout) {
            Ok(input) => Some(input),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let mut force = false;
        let mut queue_reset = false;
        let mut ack: Option<(Option<u64>, u32, bool, ErrorCode)> = None;

        match input {
            None => {}
            Some(ServiceInput::Command(PlaybackInput::Shutdown)) => break,
            Some(ServiceInput::Command(PlaybackInput::Ui { conn, req, op })) => {
                if matches!(op, PlayerOp::Resync) {
                    force = true;
                    queue_reset = true;
                    ack = Some((Some(conn), req, true, ErrorCode::Internal));
                } else if let Some(command) = core_command(&op) {
                    let needs_audio = needs_engine(&op);
                    if needs_audio && !service.ensure_engine() {
                        ack = Some((Some(conn), req, false, ErrorCode::EngineUnavailable));
                    } else {
                        let outcome = service.core.apply(command);
                        apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                        ack = Some((Some(conn), req, true, ErrorCode::Internal));
                    }
                } else {
                    ack = Some((Some(conn), req, false, ErrorCode::InvalidArgs));
                }
            }
            Some(ServiceInput::Command(PlaybackInput::Control { op })) => {
                let needs_audio = matches!(
                    op,
                    CoreCommand::Play
                        | CoreCommand::PlayPause
                        | CoreCommand::Seek(_)
                        | CoreCommand::SeekRelative(_)
                        | CoreCommand::Jump(_)
                        | CoreCommand::Next
                        | CoreCommand::Previous
                        | CoreCommand::ShuffleQueue
                );
                if !needs_audio || service.ensure_engine() {
                    let outcome = service.core.apply(op);
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                } else {
                    _ = service.outputs.send(PlaybackOutput::Error(
                        "the audio engine is unavailable".to_string(),
                    ));
                }
            }
            Some(ServiceInput::Command(PlaybackInput::Load {
                tracks,
                start,
                paused,
                position,
                volume,
                muted,
            })) => {
                // An autoplay load with no usable engine stays paused instead
                // of reporting a silent "playing" transport.
                let paused = paused || !service.ensure_engine();
                if let Some(volume) = volume {
                    let outcome = service.core.apply(CoreCommand::SetVolume(volume));
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                }
                if let Some(muted) = muted {
                    let outcome = service.core.apply(CoreCommand::SetMuted(muted));
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                }
                let outcome = service.core.load(tracks, start, paused);
                apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                if let Some(position) = position
                    && position > 0.0
                {
                    let outcome = service.core.apply(CoreCommand::Seek(position));
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                }
                force = true;
                queue_reset = true;
            }
            Some(ServiceInput::Command(PlaybackInput::Resync { conn, req })) => {
                force = true;
                queue_reset = true;
                ack = Some((Some(conn), req, true, ErrorCode::Internal));
            }
            Some(ServiceInput::Engine(event)) => match event {
                EngineEvent::SinkFailed { message } => {
                    service.last_error = Some(message.clone());
                    _ = service.outputs.send(PlaybackOutput::Error(message));
                    let position = service.core.position_at(crate::linux::monotonic_ns());
                    service.drop_engine();
                    service.last_engine_attempt = None;
                    if service.ensure_engine() {
                        let outcome = service.core.apply(CoreCommand::Recover { position });
                        apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                        service.sync_audio();
                        _ = service.outputs.send(PlaybackOutput::Error(
                            "PipeWire restarted; playback resumed".to_string(),
                        ));
                    }
                    force = true;
                }
                event @ (EngineEvent::FirstPcm
                | EngineEvent::Loaded { .. }
                | EngineEvent::Boundary { .. }
                | EngineEvent::Ended
                | EngineEvent::Failed { .. }
                | EngineEvent::SinkRecovered
                | EngineEvent::LoadingCleared) => {
                    if let EngineEvent::Failed { message } = &event {
                        service.last_error = Some(message.clone());
                        _ = service.outputs.send(PlaybackOutput::Error(message.clone()));
                    }
                    let now = crate::linux::monotonic_ns();
                    let outcome = service.core.on_engine(event, now);
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                }
            },
        }

        if queue_reset {
            _ = service.outputs.send(PlaybackOutput::Reset {
                rev: service.core.revision(),
                queue: service.core.queue(),
            });
        }
        if let Some(state) = publish(
            &mut service,
            &mut published,
            &mut published_at,
            &mut next_seq,
            force,
        ) {
            _ = service.outputs.send(PlaybackOutput::State(state));
        }
        store_snapshot(&snapshot, &service, &published);
        if let Some((conn, req, ok, code)) = ack {
            _ = service.outputs.send(PlaybackOutput::Ack {
                conn,
                req,
                ok,
                code,
                applied_seq: published.seq,
            });
        }
    }

    service.drop_engine();
}

fn apply_outcome(
    service: &ServiceState,
    outcome: &Outcome,
    force: &mut bool,
    queue_reset: &mut bool,
) {
    service.send_audio(outcome);
    if let Some(delta) = outcome.delta.clone() {
        _ = service.outputs.send(PlaybackOutput::Delta(delta));
        *force = true;
    }
    if outcome.changed {
        *force = true;
    }
    if outcome.queue_reset {
        *queue_reset = true;
    }
}

fn publish_now(
    service: &mut ServiceState,
    published: &mut PlaybackState,
    published_at: &mut Instant,
    next_seq: &mut u64,
) {
    let now = crate::linux::monotonic_ns();
    let mut state = service.core.snapshot(*next_seq, now);
    if !service.engine.is_some() {
        state.flags |= crate::ipc_protocol::STATE_FLAG_ENGINE_UNAVAILABLE;
    }
    debug_assert_eq!(state.seq, *next_seq);
    *published = state;
    *published_at = Instant::now();
    *next_seq = next_seq.wrapping_add(1);
}

fn publish(
    service: &mut ServiceState,
    published: &mut PlaybackState,
    published_at: &mut Instant,
    next_seq: &mut u64,
    force: bool,
) -> Option<PlaybackState> {
    let now = crate::linux::monotonic_ns();
    let mut state = service.core.snapshot(*next_seq, now);
    if service.engine.is_none() {
        state.flags |= crate::ipc_protocol::STATE_FLAG_ENGINE_UNAVAILABLE;
    }
    let identity_changed = state.state != published.state
        || state.index != published.index
        || state.repeat != published.repeat
        || state.reserved != published.reserved
        || state.volume_milli != published.volume_milli
        || state.flags != published.flags
        || state.duration_ms != published.duration_ms
        || state.queue_rev != published.queue_rev;
    let playing = state.state() == StateWire::Playing;
    let drift_due = playing && published_at.elapsed() >= CLOCK_CORRECTION;
    if !(force || identity_changed || drift_due || published.seq == 0) {
        return None;
    }
    // The UI extrapolates from the anchor; only corrections are published.
    state.seq = next_seq.wrapping_add(1);
    *next_seq = next_seq.wrapping_add(1);
    *published = state;
    *published_at = Instant::now();
    Some(state)
}

fn store_snapshot(
    snapshot: &Arc<Mutex<Arc<Snapshot>>>,
    service: &ServiceState,
    published: &PlaybackState,
) {
    let next = Arc::new(Snapshot {
        state: *published,
        queue: service.core.queue(),
        queue_rev: service.core.revision(),
        last_error: service.last_error.clone(),
        engine_available: service.engine.is_some(),
    });
    *snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = next;
}

fn core_command(op: &PlayerOp) -> Option<CoreCommand> {
    use PlayerOp as P;
    Some(match op {
        P::PlayPause => CoreCommand::PlayPause,
        P::Play => CoreCommand::Play,
        P::Pause => CoreCommand::Pause,
        P::Stop => CoreCommand::Stop,
        P::Next => CoreCommand::Next,
        P::Previous => CoreCommand::Previous,
        P::SetVolume(volume) => CoreCommand::SetVolume(*volume),
        P::VolumeUp => CoreCommand::VolumeUp,
        P::VolumeDown => CoreCommand::VolumeDown,
        P::ToggleMute => CoreCommand::ToggleMute,
        P::SetMuted(muted) => CoreCommand::SetMuted(*muted),
        P::Seek(seconds) => CoreCommand::Seek(*seconds),
        P::SeekRelative(offset) => CoreCommand::SeekRelative(*offset),
        P::Jump(index) => CoreCommand::Jump(*index as usize),
        P::Remove(index) => CoreCommand::Remove(*index as usize),
        P::RemoveMany(indices) => {
            CoreCommand::RemoveMany(indices.iter().map(|index| *index as usize).collect())
        }
        P::Move { from, to } => CoreCommand::Move {
            from: *from as usize,
            to: *to as usize,
        },
        P::MoveBlock { start, len, delta } => CoreCommand::MoveBlock {
            start: *start as usize,
            len: *len as usize,
            delta: *delta,
        },
        P::CycleRepeat => CoreCommand::CycleRepeat,
        P::CycleNormalization => CoreCommand::CycleNormalization,
        P::ToggleCrossfade => CoreCommand::ToggleCrossfade,
        P::SetNormalization(code) => {
            CoreCommand::SetNormalization(crate::player::Normalization::from_code(*code))
        }
        P::SetCrossfade(enabled) => CoreCommand::SetCrossfade(*enabled),
        P::ShuffleQueue => CoreCommand::ShuffleQueue,
        P::SetTrackFavorite { id, favorite } => CoreCommand::SetTrackFavorite {
            id: id.clone(),
            favorite: *favorite,
        },
        P::Resync => return None,
    })
}

fn needs_engine(op: &PlayerOp) -> bool {
    matches!(
        op,
        PlayerOp::Play
            | PlayerOp::PlayPause
            | PlayerOp::Seek(_)
            | PlayerOp::SeekRelative(_)
            | PlayerOp::Jump(_)
            | PlayerOp::Next
            | PlayerOp::Previous
            | PlayerOp::ShuffleQueue
    )
}

/// `CoreCommand` for a parsed text/MPRIS command, if it is a playback control.
pub(crate) fn command_for_text(command: &crate::ipc_command::IpcCommand) -> Option<CoreCommand> {
    let op = command.player_op()?;
    core_command(&op)
}

/// Builds the ordered TCP-style frames for a full queue reset: `Reset` followed
/// by `QueueRows` chunks. Serialization happens once and the frames are shared
/// by every UI client.
pub(crate) fn reset_frames(
    rev: u64,
    queue: &[crate::player::Track],
) -> Vec<Arc<Vec<u8>>> {
    use crate::ipc_protocol as proto;
    let mut frames = Vec::new();
    let reset = proto::QueueDelta::reset(rev, queue.len());
    if let Ok(bytes) = reset.encode()
        && let Ok(frame) = proto::Frame::new(proto::KIND_QUEUE_DELTA, bytes).encode()
    {
        frames.push(Arc::new(frame));
    }
    for (chunk_index, chunk) in queue.chunks(proto::QUEUE_CHUNK_ROWS).enumerate() {
        let rows = proto::QueueRows {
            rev,
            start: (chunk_index * proto::QUEUE_CHUNK_ROWS) as u32,
            rows: chunk.iter().map(proto::WireTrack::from).collect(),
        };
        let Ok(payload) = rows.encode() else { continue };
        let Ok(frame) = proto::Frame::new(proto::KIND_QUEUE_ROWS, payload).encode() else {
            continue;
        };
        frames.push(Arc::new(frame));
    }
    frames
}
