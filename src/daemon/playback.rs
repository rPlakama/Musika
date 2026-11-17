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
use crate::player::engine::{AudioCommand, Engine, EngineEvent};

/// How often the playback thread evaluates its clock while audible/loading.
///
/// This is not the transport clock: engine events wake the thread through the
/// service channel and the UI extrapolates from the published anchor. The
/// periodic wake only exists so the 5 s drift correction has a deadline, so a
/// coarse cadence is enough. Stopped/paused playback blocks on the channel
/// instead of waking at all.
const ACTIVE_TICK: Duration = Duration::from_millis(250);
/// A playing clock is republished at least this often so UI extrapolation
/// cannot drift for long (this is a correction, not the old 10 Hz position
/// stream: between corrections the UI computes the position locally).
const CLOCK_CORRECTION: Duration = Duration::from_secs(5);
/// Minimum spacing between engine start attempts when PipeWire is unavailable.
const ENGINE_RETRY: Duration = Duration::from_secs(5);
/// Transparent retries for a retryable stream failure (network / 5xx) before
/// the transport stops. The delays grow so a flapping server is not hammered.
const RETRY_ATTEMPTS: u32 = 3;
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
/// Sleep timer's final fade before pausing.
const SLEEP_FADE: Duration = Duration::from_secs(5);

pub(crate) fn retry_delay(attempt: u32) -> Duration {
    RETRY_DELAYS[(attempt as usize).min(RETRY_DELAYS.len() - 1)]
}

/// One decision in the transparent-retry budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    /// Reopen after `attempt` (0-based) previous retries, waiting `delay`.
    Schedule { attempt: u32, delay: Duration },
    /// The budget is spent; stop and report.
    Exhausted,
}

/// `previous_attempts` is the number of retries already consumed for this
/// failure burst (0 for the first failure).
pub(crate) fn retry_decision(previous_attempts: Option<u32>) -> RetryDecision {
    let attempt = previous_attempts.map_or(0, |attempts| attempts + 1);
    if attempt >= RETRY_ATTEMPTS {
        RetryDecision::Exhausted
    } else {
        RetryDecision::Schedule {
            attempt,
            delay: retry_delay(attempt),
        }
    }
}

/// Volume curve for the sleep timer's final fade.
pub(crate) fn original_fade_volume(original: f64, elapsed: f64, total: f64) -> f64 {
    if total <= 0.0 {
        return original;
    }
    (original * (1.0 - elapsed / total).clamp(0.0, 1.0)).clamp(0.0, 100.0)
}

/// Transport/queue actions that make a transparent retry meaningless. Mixing
/// changes (volume, mute, normalization, crossfade) must not cancel one.
pub(crate) fn cancels_retry(command: &CoreCommand) -> bool {
    matches!(
        command,
        CoreCommand::Play
            | CoreCommand::PlayPause
            | CoreCommand::Pause
            | CoreCommand::Stop
            | CoreCommand::Seek(_)
            | CoreCommand::SeekRelative(_)
            | CoreCommand::Jump(_)
            | CoreCommand::Next
            | CoreCommand::Previous
            | CoreCommand::ShuffleQueue
            | CoreCommand::Remove(_)
            | CoreCommand::RemoveMany(_)
            | CoreCommand::Move { .. }
            | CoreCommand::MoveBlock { .. }
    )
}

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
    /// Connectivity verdict from the daemon's health probe.
    SetOffline(bool),
    /// Persisted playback preferences loaded at startup.
    SetSettings(Box<crate::playback_settings::PlaybackSettings>),
    /// Live output-device choice from the UI/command line.
    SetOutputDevice {
        device: Option<String>,
        follow_default: bool,
    },
    /// One live stream-quality/cache change.
    ApplyStreamChange(crate::playback_settings::StreamChange),
    /// Set the Material palette contrast offset (-1..=1).
    SetColorContrast(f64),
    /// Start (`Some(minutes)`) or cancel (`None`) the sleep timer.
    SetSleepTimer(Option<u64>),
    /// Enable/disable scrobbling.
    SetScrobble(bool),
    /// Cycle `eqs/*.{txt,eq}` -> OFF -> first.
    CycleEq,
    /// Change one crossfade field (length/curve/album-only).
    SetCrossfadeConfig(crate::playback_settings::CrossfadeChange),
    /// Change exclusive / bit-perfect output.
    SetOutputMode {
        exclusive: Option<bool>,
        bit_perfect: Option<bool>,
    },
    /// Select the backend/ALSA device for the next engine start.
    SetOutputBackend {
        backend: Option<crate::playback_settings::OutputBackend>,
        device: Option<String>,
    },
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
            Self::SetOffline(offline) => write!(formatter, "SetOffline({offline})"),
            Self::SetSettings(_) => write!(formatter, "SetSettings"),
            Self::SetOutputDevice {
                device,
                follow_default,
            } => write!(
                formatter,
                "SetOutputDevice {{ device: {device:?}, follow_default: {follow_default} }}"
            ),
            Self::ApplyStreamChange(change) => {
                write!(formatter, "ApplyStreamChange({change:?})")
            }
            Self::SetColorContrast(contrast) => {
                write!(formatter, "SetColorContrast({contrast:?})")
            }
            Self::SetSleepTimer(minutes) => write!(formatter, "SetSleepTimer({minutes:?})"),
            Self::SetScrobble(enabled) => write!(formatter, "SetScrobble({enabled})"),
            Self::CycleEq => write!(formatter, "CycleEq"),
            Self::SetCrossfadeConfig(change) => {
                write!(formatter, "SetCrossfadeConfig({change:?})")
            }
            Self::SetOutputMode {
                exclusive,
                bit_perfect,
            } => write!(
                formatter,
                "SetOutputMode {{ exclusive: {exclusive:?}, bit_perfect: {bit_perfect:?} }}"
            ),
            Self::SetOutputBackend { backend, device } => {
                write!(formatter, "SetOutputBackend {{ backend: {backend:?}, device: {device:?} }}")
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
    /// A remote row could not be opened because the server is unreachable.
    /// The daemon switches the whole app to the downloaded catalog and shows
    /// one canonical toast.
    ServerUnavailable,
    /// A user command changed one of the persisted playback preferences. The
    /// daemon writes the document back to `status.db`.
    SettingsChanged(Box<crate::playback_settings::PlaybackSettings>),
    /// Active EQ profile name (`None` = bypass).
    EqChanged(Option<String>),
}

/// Read-only projection of the playback thread's state.
#[derive(Default)]
pub(crate) struct Snapshot {
    pub state: PlaybackState,
    pub queue: Arc<Vec<crate::player::Track>>,
    pub queue_rev: u64,
    pub last_error: Option<String>,
    pub engine_available: bool,
    /// A transparent retry is scheduled or in flight.
    pub retrying: bool,
    pub retry_attempt: u32,
    pub sleep_remaining_secs: u64,
    pub metrics: crate::player::PlaybackMetrics,
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

/// One transient failure waiting for (or in) its transparent retry.
struct RetryState {
    /// Retries already scheduled/attempted for this failure burst.
    attempts: u32,
    /// When the next reopen should run; `None` while an attempt is in flight.
    next_at: Option<Instant>,
    position: f64,
}

/// A network failure that exhausted its retries: the daemon's next successful
/// health probe may resume it once.
struct Recovery {
    position: f64,
}

/// Active sleep-timer fade: the service lowers the user volume over
/// [`SLEEP_FADE`] and restores it after pausing.
struct SleepFade {
    started: Instant,
    original_volume: f64,
}

/// Playback-time accounting for one scrobble.
struct ScrobbleState {
    song_id: String,
    accumulated: Duration,
    threshold: Duration,
    last_tick: Instant,
    submitted: bool,
}

struct ServiceState {
    core: PlaybackCore,
    engine: Option<Engine>,
    service_tx: Sender<ServiceInput>,
    outputs: UnboundedSender<PlaybackOutput>,
    last_engine_attempt: Option<Instant>,
    last_error: Option<String>,
    retry: Option<RetryState>,
    recovery: Option<Recovery>,
    offline: bool,
    settings: crate::playback_settings::PlaybackSettings,
    sleep_deadline: Option<Instant>,
    sleep_fade: Option<SleepFade>,
    scrobble: Option<ScrobbleState>,
    /// Consecutive sink rebuilds since the last healthy stream. Bounded so a
    /// missing PipeWire/ALSA target cannot spin forever.
    sink_failures: u32,
    metrics: crate::player::PlaybackMetrics,
}

impl ServiceState {
    /// Device name the stream should target; `None` follows PipeWire's default.
    fn output_target(&self) -> Option<String> {
        if self.settings.follow_default {
            None
        } else {
            self.settings.output_device.clone()
        }
    }

    /// Applies the current output target to the live engine. A later engine
    /// creation applies it in `ensure_engine`.
    fn apply_output_target(&self) {
        if let Some(engine) = self.engine.as_ref() {
            engine.send(AudioCommand::SetOutputTarget(self.output_target()));
            engine.send(AudioCommand::SetExclusive(self.settings.exclusive));
            engine.send(AudioCommand::SetBitPerfect(self.settings.bit_perfect));
        }
    }

    /// Warm the purgeable playback cache after the first PCM of a remote row.
    /// The fetch runs off the playback thread and never blocks audio.
    fn maybe_cache_current_stream(&self) {
        if !self.settings.stream_cache {
            return;
        }
        let queue = self.core.queue();
        let Some(track) = queue.get(self.core.index()) else {
            return;
        };
        if track.id.is_empty() || !crate::player::is_remote_url(&track.url) {
            return;
        }
        let id = track.id.clone();
        let settings = self.settings.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else {
                return;
            };
            if let Err(error) =
                crate::media_actions::cache_stream(&dbs, &id, &settings).await
            {
                crate::debug_log!("stream cache: {id} failed: {error:#}");
            }
        });
    }

    /// Starts the scrobble clock for the row that just produced PCM and sends
    /// the now-playing notification.
    fn start_scrobble(&mut self) {
        let queue = self.core.queue();
        let Some(track) = queue.get(self.core.index()) else {
            self.scrobble = None;
            return;
        };
        let song_id = track.id.clone();
        if song_id.is_empty() || !self.settings.scrobble {
            self.scrobble = None;
            return;
        }
        let threshold =
            Duration::from_secs_f64(crate::scrobble::threshold_secs(self.core.duration()));
        self.scrobble = Some(ScrobbleState {
            song_id: song_id.clone(),
            accumulated: Duration::ZERO,
            threshold,
            last_tick: Instant::now(),
            submitted: false,
        });
        crate::app::runtime().spawn(async move {
            if let Some(dbs) = crate::app::dbs() {
                crate::scrobble::now_playing(&dbs, &song_id).await;
            }
        });
    }

    /// Accumulates actual playing time (paused time is excluded) and submits
    /// one completed play at the threshold.
    fn tick_scrobble(&mut self) {
        let Some(current) = self
            .core
            .queue()
            .get(self.core.index())
            .map(|track| track.id.clone())
        else {
            self.scrobble = None;
            return;
        };
        let Some(state) = self.scrobble.as_mut() else {
            return;
        };
        if state.song_id != current
            || !self.settings.scrobble
            || self.core.state() == crate::player::State::Stopped
        {
            self.scrobble = None;
            return;
        }
        let now = Instant::now();
        if self.core.state() == crate::player::State::Playing {
            state.accumulated += now.saturating_duration_since(state.last_tick);
        }
        state.last_tick = now;
        if state.submitted || state.accumulated < state.threshold {
            return;
        }
        state.submitted = true;
        let song_id = state.song_id.clone();
        let played_at = crate::consts::now_secs();
        crate::app::runtime().spawn(async move {
            if let Some(dbs) = crate::app::dbs() {
                crate::scrobble::submit(&dbs, &song_id, played_at).await;
            }
        });
    }

    /// Loads `settings.eq_profile` from disk and installs it into the engine.
    /// A missing/invalid file clears the selection instead of keeping a stale
    /// profile.
    fn apply_eq_from_settings(&mut self) {
        let name = self.settings.eq_profile.clone();
        let profile = match name.as_deref() {
            Some(name) => match crate::eq::load_profile(name) {
                Ok(profile) => Some(std::sync::Arc::new(profile)),
                Err(error) => {
                    crate::debug_log!("eq: cannot load profile '{name}': {error:#}");
                    None
                }
            },
            None => None,
        };
        self.settings.eq_profile = profile.as_ref().map(|profile| profile.name.clone());
        if let Some(engine) = self.engine.as_ref() {
            engine.send(AudioCommand::SetEq(profile.clone()));
        }
        _ = self.outputs.send(PlaybackOutput::EqChanged(
            profile.as_ref().map(|profile| profile.name.clone()),
        ));
    }

    fn cycle_eq(&mut self) {
        let profiles = crate::eq::list_profiles();
        crate::debug_log!(
            "eq: cycle from {:?}, {} profile(s) in {}",
            self.settings.eq_profile,
            profiles.len(),
            crate::eq::profiles_dir()
                .map(|dir| dir.display().to_string())
                .unwrap_or_else(|error| format!("<{error}>"))
        );
        let current = self.settings.eq_profile.clone();
        let position = current
            .as_ref()
            .and_then(|name| profiles.iter().position(|candidate| candidate == name));
        let next = match position {
            Some(index) if index + 1 < profiles.len() => Some(profiles[index + 1].clone()),
            Some(_) => None,
            None => profiles.first().cloned(),
        };
        self.settings.eq_profile = next;
        self.apply_eq_from_settings();
    }

    /// Any explicit transport/queue action supersedes a pending retry or a
    /// post-recovery auto-resume.
    fn cancel_retry(&mut self) {
        self.retry = None;
        self.recovery = None;
    }
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
        // The transport reports straight into the service channel; the old
        // forwarding thread only added a hop between the engine and the
        // playback thread.
        let service_tx = self.service_tx.clone();
        let events: crate::player::engine::EventSink = Arc::new(move |event| {
            service_tx.send(ServiceInput::Engine(event)).is_ok()
        });
        match Engine::new(events) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.apply_output_target();
                self.apply_eq_from_settings();
                crate::startup::trace("engine.ready");
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
        service_tx: engine_tx,
        outputs,
        last_engine_attempt: None,
        last_error: None,
        retry: None,
        recovery: None,
        offline: false,
        settings: crate::playback_settings::PlaybackSettings::default(),
        sleep_deadline: None,
        sleep_fade: None,
        scrobble: None,
        sink_failures: 0,
        metrics: crate::player::PlaybackMetrics::default(),
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
    crate::startup::trace("playback.ready");

    loop {
        let active = published.state() == StateWire::Playing
            || published.loading()
            || service.sleep_deadline.is_some()
            || service.sleep_fade.is_some();
        // While the transport is stopped or paused nothing changes without an
        // input, so block on the channel. While playing keep a coarse tick for
        // the drift correction; engine events arrive directly in the channel.
        let input = if active {
            match rx.recv_timeout(ACTIVE_TICK) {
                Ok(input) => Some(input),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(input) => Some(input),
                Err(_) => break,
            }
        };

        let mut force = false;
        let mut queue_reset = false;
        let mut ack: Option<(Option<u64>, u32, bool, ErrorCode)> = None;

        // The retry timer is serviced here rather than on a separate thread:
        // while a retry is pending the published state is still Playing, so
        // the loop wakes on ACTIVE_TICK.
        if service
            .retry
            .as_ref()
            .and_then(|retry| retry.next_at)
            .is_some_and(|at| at <= Instant::now())
        {
            let position = service.retry.as_ref().map(|retry| retry.position).unwrap_or(0.0);
            if service.ensure_engine() {
                if let Some(retry) = service.retry.as_mut() {
                    retry.next_at = None;
                }
                service.metrics.retries += 1;
                let outcome = service.core.apply(CoreCommand::ResumeAt(position));
                apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
            } else if let Some(retry) = service.retry.as_mut() {
                retry.next_at = Some(Instant::now() + ENGINE_RETRY);
            }
        }

        if let Some(deadline) = service.sleep_deadline
            && Instant::now() >= deadline
        {
            service.sleep_deadline = None;
            if service.core.state() == crate::player::State::Playing {
                service.sleep_fade = Some(SleepFade {
                    started: Instant::now(),
                    original_volume: service.core.volume(),
                });
            }
        }
        if let Some(fade) = service.sleep_fade.as_ref() {
            let elapsed = fade.started.elapsed().as_secs_f64();
            let total = SLEEP_FADE.as_secs_f64();
            if elapsed >= total {
                let original = fade.original_volume;
                service.sleep_fade = None;
                let restore = service.core.apply(CoreCommand::SetVolume(original));
                apply_outcome(&service, &restore, &mut force, &mut queue_reset);
                if service.core.state() == crate::player::State::Playing {
                    let pause = service.core.apply(CoreCommand::Pause);
                    apply_outcome(&service, &pause, &mut force, &mut queue_reset);
                }
                force = true;
            } else {
                let volume = original_fade_volume(fade.original_volume, elapsed, total);
                let step = service.core.apply(CoreCommand::SetVolume(volume));
                apply_outcome(&service, &step, &mut force, &mut queue_reset);
            }
        }

        service.tick_scrobble();

        match input {
            None => {}
            Some(ServiceInput::Command(PlaybackInput::Shutdown)) => break,
            Some(ServiceInput::Command(PlaybackInput::Ui { conn, req, op })) => {
                if matches!(op, PlayerOp::Resync) {
                    force = true;
                    queue_reset = true;
                    ack = Some((Some(conn), req, true, ErrorCode::Internal));
                } else if let Some(command) = core_command(&op) {
                    if cancels_retry(&command) {
                        service.cancel_retry();
                    }
                    let needs_audio = needs_engine(&op);
                    if needs_audio && !service.ensure_engine() {
                        ack = Some((Some(conn), req, false, ErrorCode::EngineUnavailable));
                    } else {
                        let setting_change = is_setting_command(&command);
                        let outcome = service.core.apply(command);
                        apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                        if setting_change {
                            capture_settings(&mut service);
                        }
                        ack = Some((Some(conn), req, true, ErrorCode::Internal));
                    }
                } else {
                    ack = Some((Some(conn), req, false, ErrorCode::InvalidArgs));
                }
            }
            Some(ServiceInput::Command(PlaybackInput::Control { op })) => {
                if cancels_retry(&op) {
                    service.cancel_retry();
                }
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
                    let setting_change = is_setting_command(&op);
                    let outcome = service.core.apply(op);
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                    if setting_change {
                        capture_settings(&mut service);
                    }
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
                service.cancel_retry();
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
            Some(ServiceInput::Command(PlaybackInput::SetSettings(settings))) => {
                service.settings = *settings;
                crate::playback_settings::install(&service.settings);
                let repeat = match service.settings.repeat {
                    1 => crate::player::Repeat::One,
                    2 => crate::player::Repeat::All,
                    _ => crate::player::Repeat::Off,
                };
                let normalization =
                    crate::player::Normalization::from_code(service.settings.normalization);
                let shuffle = service.settings.shuffle;
                let mut outcome = service.core.apply(CoreCommand::SetRepeat(repeat));
                for extra in [
                    service
                        .core
                        .apply(CoreCommand::SetShuffle(shuffle)),
                    service.core.apply(CoreCommand::SetNormalization(normalization)),
                    service
                        .core
                        .apply(CoreCommand::SetCrossfadeConfig(
                            service.settings.crossfade_config(),
                        )),
                ] {
                    outcome.audio.extend(extra.audio);
                    outcome.changed |= extra.changed;
                    outcome.queue_reset |= extra.queue_reset;
                }
                apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                service.apply_output_target();
                service.apply_eq_from_settings();
                _ = service.outputs.send(PlaybackOutput::SettingsChanged(Box::new(
                    service.settings.clone(),
                )));
            }
            Some(ServiceInput::Command(PlaybackInput::SetOutputDevice {
                device,
                follow_default,
            })) => {
                service.settings.output_device = device;
                service.settings.follow_default = follow_default;
                service.apply_output_target();
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::ApplyStreamChange(change))) => {
                change.apply(&mut service.settings);
                crate::playback_settings::install(&service.settings);
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetColorContrast(contrast))) => {
                service.settings.color_contrast = contrast.clamp(-1.0, 1.0);
                crate::playback_settings::install(&service.settings);
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetSleepTimer(minutes))) => {
                match minutes {
                    Some(minutes) => {
                        service.sleep_deadline =
                            Some(Instant::now() + Duration::from_secs(minutes.saturating_mul(60)));
                    }
                    None => {
                        service.sleep_deadline = None;
                        if let Some(fade) = service.sleep_fade.take() {
                            let restore =
                                service.core.apply(CoreCommand::SetVolume(fade.original_volume));
                            apply_outcome(
                                &service,
                                &restore,
                                &mut force,
                                &mut queue_reset,
                            );
                        }
                    }
                }
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetOutputBackend {
                backend,
                device,
            })) => {
                if let Some(backend) = backend {
                    service.settings.output_backend = backend;
                }
                if let Some(device) = device {
                    service.settings.alsa_device = device;
                }
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetOutputMode {
                exclusive,
                bit_perfect,
            })) => {
                if let Some(exclusive) = exclusive {
                    service.settings.exclusive = exclusive;
                }
                if let Some(bit_perfect) = bit_perfect {
                    service.settings.bit_perfect = bit_perfect;
                }
                service.apply_output_target();
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetCrossfadeConfig(change))) => {
                change.apply(&mut service.settings);
                let config = service.settings.crossfade_config();
                let outcome = service
                    .core
                    .apply(CoreCommand::SetCrossfadeConfig(config));
                apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::CycleEq)) => {
                service.cycle_eq();
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetScrobble(enabled))) => {
                service.settings.scrobble = enabled;
                if !enabled {
                    service.scrobble = None;
                }
                capture_settings(&mut service);
                force = true;
            }
            Some(ServiceInput::Command(PlaybackInput::SetOffline(offline))) => {
                let was_offline = service.offline;
                let outcome = service.core.set_offline(offline);
                apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                service.offline = offline;
                if was_offline
                    && !offline
                    && let Some(recovery) = service.recovery.take()
                {
                    // The health probe confirmed the server again: resume the
                    // row the transient failure stopped, once.
                    if service.ensure_engine() {
                        service.metrics.reconnects += 1;
                        let outcome = service
                            .core
                            .apply(CoreCommand::ResumeAt(recovery.position));
                        apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                        force = true;
                    } else {
                        service.recovery = Some(recovery);
                    }
                }
            }
            Some(ServiceInput::Command(PlaybackInput::Resync { conn, req })) => {
                force = true;
                queue_reset = true;
                ack = Some((Some(conn), req, true, ErrorCode::Internal));
            }
            Some(ServiceInput::Engine(event)) => match event {
                EngineEvent::SinkFailed { message } => {
                    service.cancel_retry();
                    service.metrics.sink_rebuilds += 1;
                    service.sink_failures += 1;
                    service.last_error = Some(message.clone());
                    _ = service.outputs.send(PlaybackOutput::Error(message));
                    service.drop_engine();
                    if service.sink_failures > 3 {
                        service.sink_failures = 0;
                        service.last_engine_attempt = None;
                        let outcome = service.core.apply(CoreCommand::Stop);
                        apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                        _ = service.outputs.send(PlaybackOutput::Error(
                            "Audio output unavailable; playback paused".to_string(),
                        ));
                    } else {
                        service.last_engine_attempt = None;
                        if service.ensure_engine() {
                            let position =
                                service.core.position_at(crate::linux::monotonic_ns());
                            let outcome = service.core.apply(CoreCommand::Recover { position });
                            apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                            service.sync_audio();
                            _ = service.outputs.send(PlaybackOutput::Error(
                                "PipeWire restarted; playback resumed".to_string(),
                            ));
                        }
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
                    let healthy = matches!(
                        event,
                        EngineEvent::FirstPcm | EngineEvent::Loaded { .. }
                    );
                    let first_pcm = matches!(event, EngineEvent::FirstPcm);
                    if healthy || matches!(event, EngineEvent::SinkRecovered) {
                        service.sink_failures = 0;
                    }
                    let failure = if let EngineEvent::Failed { error } = &event {
                        Some(error.clone())
                    } else {
                        None
                    };
                    let had_retry = if healthy || failure.is_some() {
                        service.retry.take()
                    } else {
                        None
                    };
                    let now = crate::linux::monotonic_ns();
                    let outcome = service.core.on_engine(event, now);
                    if healthy {
                        // The reopen succeeded; a stale retry is over and the
                        // status error line can clear.
                        if had_retry.is_some() {
                            service.last_error = None;
                            force = true;
                        }
                    } else if let Some(error) = failure {
                        if outcome.deferred_failure {
                            // Paused restore: report on the play attempt.
                            service.retry = None;
                            service.last_error = Some(error.message.clone());
                        } else if error.retryable() {
                            let position = outcome
                                .failure_position
                                .or_else(|| had_retry.as_ref().map(|retry| retry.position))
                                .unwrap_or_else(|| service.core.position_at(now));
                            let previous_attempts =
                                had_retry.as_ref().map(|retry| retry.attempts);
                            if retry_decision(previous_attempts) == RetryDecision::Exhausted {
                                // Exhausted: stop, report once, and leave a
                                // recovery marker for the next good probe.
                                let stop = service.core.apply(CoreCommand::Stop);
                                apply_outcome(
                                    &service,
                                    &stop,
                                    &mut force,
                                    &mut queue_reset,
                                );
                                service.retry = None;
                                service.recovery = error
                                    .is_network()
                                    .then_some(Recovery { position });
                                service.last_error = Some(if error.is_network() {
                                    crate::player::SERVER_UNAVAILABLE.to_string()
                                } else {
                                    error.message.clone()
                                });
                                if error.is_network() {
                                    _ = service
                                        .outputs
                                        .send(PlaybackOutput::ServerUnavailable);
                                } else {
                                    _ = service
                                        .outputs
                                        .send(PlaybackOutput::Error(error.message));
                                }
                                force = true;
                            } else if let RetryDecision::Schedule { attempt, delay } =
                                retry_decision(previous_attempts)
                            {
                                service.retry = Some(RetryState {
                                    attempts: attempt,
                                    next_at: Some(Instant::now() + delay),
                                    position,
                                });
                                service.last_error = Some(error.message);
                            }
                        } else {
                            // A permanent failure: the core already advanced;
                            // report it and forget any stale recovery.
                            service.retry = None;
                            service.recovery = None;
                            service.last_error = Some(error.message.clone());
                            _ = service.outputs.send(PlaybackOutput::Error(error.message));
                        }
                    } else if outcome.server_unavailable {
                        service.last_error =
                            Some(crate::player::SERVER_UNAVAILABLE.to_string());
                    }
                    apply_outcome(&service, &outcome, &mut force, &mut queue_reset);
                    if first_pcm {
                        service.maybe_cache_current_stream();
                        service.start_scrobble();
                    }
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

/// Commands whose success changes a persisted preference.
fn is_setting_command(command: &CoreCommand) -> bool {
    matches!(
        command,
        CoreCommand::CycleRepeat
            | CoreCommand::SetRepeat(_)
            | CoreCommand::CycleNormalization
            | CoreCommand::SetNormalization(_)
            | CoreCommand::ToggleCrossfade
            | CoreCommand::SetCrossfade(_)
            | CoreCommand::SetCrossfadeConfig(_)
    )
}

/// Copies the live transport preferences into the settings document and tells
/// the daemon to persist it. Only explicit user commands call this: a queue
/// load resets repeat by design and must not overwrite the stored preference.
fn capture_settings(service: &mut ServiceState) {
    service.settings.repeat = service.core.repeat().code();
    service.settings.normalization = service.core.normalization().code();
    service.settings.crossfade = service.core.crossfade();
    let crossfade = service.core.crossfade_config();
    service.settings.crossfade_seconds = crossfade.seconds;
    service.settings.crossfade_curve = if crossfade.equal_power {
        "equal-power".to_string()
    } else {
        "linear".to_string()
    };
    service.settings.crossfade_album_only = crossfade.album_only;
    service.settings.shuffle = service.core.shuffle();
    _ = service
        .outputs
        .send(PlaybackOutput::SettingsChanged(Box::new(
            service.settings.clone(),
        )));
}

fn apply_outcome(
    service: &ServiceState,
    outcome: &Outcome,
    force: &mut bool,
    queue_reset: &mut bool,
) {
    service.send_audio(outcome);
    if outcome.server_unavailable {
        _ = service.outputs.send(PlaybackOutput::ServerUnavailable);
    }
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

fn sample_engine_metrics(service: &mut ServiceState) {
    if let Some(engine) = service.engine.as_ref() {
        let clock = engine.clock();
        service.metrics.buffered_frames = clock.buffered_frames;
        service.metrics.underruns = clock.underruns;
    }
}

fn publish_now(
    service: &mut ServiceState,
    published: &mut PlaybackState,
    published_at: &mut Instant,
    next_seq: &mut u64,
) {
    sample_engine_metrics(service);
    let now = crate::linux::monotonic_ns();
    let mut state = service.core.snapshot(*next_seq, now);
    if !service.engine.is_some() {
        state.flags |= crate::ipc_protocol::STATE_FLAG_ENGINE_UNAVAILABLE;
    }
    if service.retry.is_some() {
        state.flags |= crate::ipc_protocol::STATE_FLAG_RETRYING;
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
    sample_engine_metrics(service);
    let now = crate::linux::monotonic_ns();
    let mut state = service.core.snapshot(*next_seq, now);
    if service.engine.is_none() {
        state.flags |= crate::ipc_protocol::STATE_FLAG_ENGINE_UNAVAILABLE;
    }
    if service.retry.is_some() {
        state.flags |= crate::ipc_protocol::STATE_FLAG_RETRYING;
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
        retrying: service.retry.is_some(),
        retry_attempt: service
            .retry
            .as_ref()
            .map_or(0, |retry| retry.attempts + 1),
        sleep_remaining_secs: service
            .sleep_deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()).as_secs())
            .or_else(|| {
                service.sleep_fade.as_ref().map(|fade| {
                    SLEEP_FADE
                        .saturating_sub(fade.started.elapsed())
                        .as_secs()
                })
            })
            .unwrap_or(0),
        metrics: service.metrics,
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
