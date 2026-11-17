//! Pure playback policy: queue, transport state and command/event reduction.
//!
//! Nothing in this module starts a thread, touches audio or depends on tokio:
//! every method takes the time it needs (`now_ns`) so the policy is
//! deterministic and testable. The daemon's playback service owns one
//! [`PlaybackCore`], forwards the [`AudioCommand`]s it returns to the event
//! driven [`crate::player::engine::Engine`] and funnels the engine's
//! [`EngineEvent`]s back through [`PlaybackCore::on_engine`].
//!
//! Position is an anchor: `(position, anchor_ns)` plus a `rate_milli` of 1000.
//! Readers extrapolate instead of receiving a position tick, so the service only
//! publishes a state when something actually changes.

use std::sync::Arc;

use crate::ipc_protocol::{
    PlaybackState, QueueDelta, QueueOp, STATE_FLAG_GAPLESS_ARMED, STATE_FLAG_LOADING,
    STATE_FLAG_MUTED, WireTrack,
};
use crate::player::{CrossfadeConfig, Normalization, Repeat, State, Track};

use super::engine::{AudioCommand, EngineEvent};

/// Volume step for [`CoreCommand::VolumeUp`]/[`CoreCommand::VolumeDown`]; the
/// UI and the text commands have always used 5 points.
const VOLUME_STEP: f64 = 5.0;

/// Nanoseconds per second, as `f64` for position interpolation.
const NANOS_PER_SECOND: f64 = 1_000_000_000.0;

#[derive(Debug, Clone)]
pub(crate) enum CoreCommand {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    SetVolume(f64),
    VolumeUp,
    VolumeDown,
    ToggleMute,
    SetMuted(bool),
    Seek(f64),
    SeekRelative(f64),
    Jump(usize),
    Remove(usize),
    RemoveMany(Vec<usize>),
    Move {
        from: usize,
        to: usize,
    },
    MoveBlock {
        start: usize,
        len: usize,
        delta: i32,
    },
    CycleRepeat,
    SetRepeat(Repeat),
    /// Toggle the persistent shuffle mode.
    ShuffleQueue,
    /// Set the shuffle mode explicitly (MPRIS/settings).
    SetShuffle(bool),
    CycleNormalization,
    ToggleCrossfade,
    SetNormalization(Normalization),
    SetCrossfade(bool),
    SetCrossfadeConfig(CrossfadeConfig),
    SetTrackFavorite {
        id: String,
        favorite: bool,
    },
    /// Appends rows at the end of the queue (bridge "Add to Queue").
    Append(Vec<Track>),
    /// Inserts cloned rows at `at` (clipboard paste).
    InsertMany {
        at: usize,
        tracks: Vec<Track>,
    },
    /// The engine was rebuilt after a failure: reload the current row at
    /// `position` and keep the transport state.
    Recover {
        position: f64,
    },
    /// Retry the current row at `position` as a live playback attempt (the
    /// service's transparent network retry / post-recovery resume).
    ResumeAt(f64),
}

#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) audio: Vec<AudioCommand>,
    /// Row mutation for the clients (`None` when the change is not expressible
    /// as one queue op; then `queue_reset` re-streams the rows instead).
    pub(crate) delta: Option<QueueDelta>,
    /// Transport/identity state changed: the service republishes a snapshot.
    pub(crate) changed: bool,
    /// Re-stream every row (metadata/cover refresh, shuffle, block reorder).
    pub(crate) queue_reset: bool,
    /// A remote row could not be played because the server is unreachable;
    /// the service reports the one canonical toast and the daemon flips to
    /// the downloaded catalog.
    pub(crate) server_unavailable: bool,
    /// A paused restore pre-opened the stream and failed. The verdict is kept
    /// for the next play attempt instead of toasting before the user asked to
    /// play anything.
    pub(crate) deferred_failure: bool,
    /// The structured failure behind `changed`, for the service's retry and
    /// toast policy.
    pub(crate) failure: Option<crate::player::PlaybackError>,
    /// The transport position at the failure, used to reopen at the same
    /// point. `None` when the outcome is not a failure.
    pub(crate) failure_position: Option<f64>,
}

pub(crate) struct PlaybackCore {
    queue: Arc<Vec<Track>>,
    index: usize,
    state: State,
    repeat: Repeat,
    normalization: Normalization,
    crossfade: CrossfadeConfig,
    volume: f64,
    muted: bool,
    /// Row revision; bumped by every queue-row mutation.
    revision: u64,
    /// Duration reported by the engine for the current track (0 = unknown).
    duration: f64,
    stream_size: u64,
    seekable: bool,
    loading: bool,
    failed: bool,
    /// The current remote row opened (the decoder reported `Loaded`). A
    /// failure before that point is an unreachable server, not a codec error
    /// in the middle of a playing stream.
    loaded_current: bool,
    /// Offline: `file://` rows are playable, remote rows are refused before
    /// the engine can stall on an HTTP connect.
    offline: bool,
    last_error: Option<String>,
    last_failure: Option<crate::player::PlaybackError>,
    /// A retryable failure is waiting for the service's retry timer; the row
    /// stays selected and the transport advertises the retrying flag.
    retrying: bool,
    /// Persistent shuffle mode: the queue keeps its user-visible order, but
    /// next/gapless follow `shuffle_plan`.
    shuffle: bool,
    /// Upcoming row indices in shuffle order (current excluded).
    shuffle_plan: Vec<usize>,
    /// Actual play history for Previous (bounded).
    history: Vec<usize>,
    /// The row the engine was told to predecode after the current one.
    gapless_next: Option<usize>,
    /// Displayed position at `anchor_ns`.
    anchor_position: f64,
    anchor_ns: u64,
    /// The anchor has to move to the next observed `now_ns` (a load, seek or
    /// resume just happened and had no clock of its own).
    rebase: bool,
    /// Last monotonic time observed through `snapshot`/`on_engine`/`apply_at`.
    now_ns: u64,
    /// Monotonic load generation handed to the worker.
    generation: u64,
}

impl PlaybackCore {
    pub(crate) fn new() -> Self {
        Self {
            queue: Arc::new(Vec::new()),
            index: 0,
            state: State::Stopped,
            repeat: Repeat::Off,
            normalization: Normalization::Off,
            crossfade: CrossfadeConfig::default(),
            volume: crate::consts::DEFAULT_VOLUME,
            muted: false,
            revision: 0,
            duration: 0.0,
            stream_size: 0,
            seekable: false,
            loading: false,
            failed: false,
            loaded_current: false,
            offline: false,
            last_error: None,
            last_failure: None,
            retrying: false,
            shuffle: false,
            shuffle_plan: Vec::new(),
            history: Vec::new(),
            gapless_next: None,
            anchor_position: 0.0,
            anchor_ns: 0,
            rebase: false,
            now_ns: 0,
            generation: 0,
        }
    }

    // --- reads ------------------------------------------------------------

    pub(crate) fn queue(&self) -> Arc<Vec<Track>> {
        Arc::clone(&self.queue)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn state(&self) -> State {
        self.state
    }

    pub(crate) fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub(crate) fn normalization(&self) -> Normalization {
        self.normalization
    }

    pub(crate) fn crossfade(&self) -> bool {
        self.crossfade.enabled
    }

    pub(crate) fn crossfade_config(&self) -> CrossfadeConfig {
        self.crossfade
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn volume(&self) -> f64 {
        self.volume
    }

    pub(crate) fn muted(&self) -> bool {
        self.muted
    }

    pub(crate) fn duration(&self) -> f64 {
        if self.duration > 0.0 {
            self.duration
        } else {
            self.queue
                .get(self.index)
                .map(|track| track.duration)
                .unwrap_or(0.0)
        }
    }

    /// URL of the row the gapless rules pick next (`repeat One` repeats the
    /// current row, `repeat All` wraps at the end).
    pub(crate) fn next_url(&self) -> Option<String> {
        self.gapless_index()
            .and_then(|index| self.queue.get(index))
            .map(|track| track.url.clone())
    }

    /// Last track/engine error (diagnostics and UI toasts).
    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn offline(&self) -> bool {
        self.offline
    }

    pub(crate) fn last_failure(&self) -> Option<&crate::player::PlaybackError> {
        self.last_failure.as_ref()
    }

    pub(crate) fn retrying(&self) -> bool {
        self.retrying
    }

    pub(crate) fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Applies a connectivity verdict. This only governs *future* loads: a
    /// stream that is already open and playing is left alone (it may well
    /// finish from the read-ahead buffer).
    pub(crate) fn set_offline(&mut self, offline: bool) -> Outcome {
        let mut outcome = Outcome::default();
        if self.offline != offline {
            self.offline = offline;
            outcome.changed = true;
        }
        outcome
    }

    pub(crate) fn position_at(&self, now_ns: u64) -> f64 {
        let base = self.anchor_position.max(0.0);
        // A load or seek has not been re-anchored to a real clock yet; the
        // playhead is still at its target, and a paused/stopped transport never
        // advances.
        if self.state != State::Playing || self.loading || self.rebase {
            return self.clamp_position(base);
        }
        let elapsed = now_ns.saturating_sub(self.anchor_ns) as f64 / NANOS_PER_SECOND;
        self.clamp_position(base + elapsed)
    }

    /// Audio commands that re-apply the current volume/mute to a freshly
    /// created engine (used after `EngineEvent::SinkFailed` recovery).
    pub(crate) fn audio_sync(&self) -> Vec<AudioCommand> {
        let mut commands = vec![
            AudioCommand::SetVolume(self.volume),
            AudioCommand::SetMuted(self.muted),
            AudioCommand::SetNormalization(self.normalization),
            AudioCommand::SetCrossfadeConfig(self.crossfade),
        ];
        if self.crossfade.enabled {
            commands.push(AudioCommand::SetCrossfadeAllowed(self.crossfade_allowed()));
        }
        commands
    }

    /// Full playback snapshot; `seq` is the service's frame sequence.
    pub(crate) fn snapshot(&mut self, seq: u64, now_ns: u64) -> PlaybackState {
        self.observe(now_ns);
        let position = self.position_at(now_ns);
        let duration = self.duration();
        let mut flags = 0;
        if self.muted {
            flags |= STATE_FLAG_MUTED;
        }
        if self.loading {
            flags |= STATE_FLAG_LOADING;
        }
        if self.gapless_next.is_some() {
            flags |= STATE_FLAG_GAPLESS_ARMED;
        }
        if self.retrying {
            flags |= crate::ipc_protocol::STATE_FLAG_RETRYING;
        }
        if self.shuffle {
            flags |= crate::ipc_protocol::STATE_FLAG_SHUFFLE;
        }
        PlaybackState {
            seq,
            queue_rev: self.revision,
            // The service adds `STATE_FLAG_ENGINE_UNAVAILABLE`: only it knows
            // whether an engine exists.
            anchor_ns: now_ns,
            index: self.index.min(u32::MAX as usize) as u32,
            duration_ms: seconds_to_ms(duration),
            position_ms: seconds_to_ms(position),
            flags,
            volume_milli: (self.volume.clamp(0.0, 100.0) * 10.0).round() as u16,
            state: self.state_byte(),
            repeat: self.repeat.code() as u8,
            rate_milli: 1000,
            reserved: u16::from(self.normalization.code())
                | if self.crossfade.enabled {
                    crate::ipc_protocol::RESERVED_CROSSFADE
                } else {
                    0
                },
        }
    }

    // --- commands ---------------------------------------------------------

    pub(crate) fn apply(&mut self, command: CoreCommand) -> Outcome {
        let now_ns = self.now_ns;
        self.apply_at(command, now_ns)
    }

    /// [`Self::apply`] with an explicit clock, for tests and for callers that
    /// already read `CLOCK_MONOTONIC`.
    pub(crate) fn apply_at(&mut self, command: CoreCommand, now_ns: u64) -> Outcome {
        self.observe(now_ns);
        // Every user/core command supersedes a pending retry.
        self.retrying = false;
        let mut outcome = Outcome::default();
        match command {
            CoreCommand::PlayPause => self.play_pause(&mut outcome),
            CoreCommand::Play => self.play(&mut outcome),
            CoreCommand::Pause => self.pause(&mut outcome),
            CoreCommand::Stop => self.stop(&mut outcome),
            CoreCommand::Next => self.advance_by(1, &mut outcome),
            CoreCommand::Previous => self.retreat_by(1, &mut outcome),
            CoreCommand::SetVolume(volume) => self.set_volume(volume, &mut outcome),
            CoreCommand::VolumeUp => self.set_volume(self.volume + VOLUME_STEP, &mut outcome),
            CoreCommand::VolumeDown => self.set_volume(self.volume - VOLUME_STEP, &mut outcome),
            CoreCommand::ToggleMute => self.set_muted(!self.muted, &mut outcome),
            CoreCommand::SetMuted(muted) => self.set_muted(muted, &mut outcome),
            CoreCommand::Seek(seconds) => self.seek(seconds, &mut outcome),
            CoreCommand::SeekRelative(offset) => {
                let position = self.position_at(self.now_ns) + offset;
                self.seek(position, &mut outcome);
            }
            CoreCommand::Jump(index) => self.jump(index, &mut outcome),
            CoreCommand::Remove(index) => self.remove(index, &mut outcome),
            CoreCommand::RemoveMany(indices) => self.remove_many(&indices, &mut outcome),
            CoreCommand::Move { from, to } => self.move_item(from, to, &mut outcome),
            CoreCommand::MoveBlock { start, len, delta } => {
                self.move_block(start, len, delta, &mut outcome)
            }
            CoreCommand::CycleNormalization => {
                let next = self.normalization.cycle();
                self.set_normalization(next, &mut outcome);
            }
            CoreCommand::ToggleCrossfade => {
                let next = !self.crossfade.enabled;
                self.set_crossfade(next, &mut outcome);
            }
            CoreCommand::SetNormalization(mode) => self.set_normalization(mode, &mut outcome),
            CoreCommand::SetCrossfade(enabled) => self.set_crossfade(enabled, &mut outcome),
            CoreCommand::SetCrossfadeConfig(config) => {
                self.set_crossfade_config(config, &mut outcome)
            }
            CoreCommand::CycleRepeat => {
                self.repeat = self.repeat.cycle();
                outcome.changed = true;
            }
            CoreCommand::SetRepeat(repeat) => {
                if self.repeat != repeat {
                    self.repeat = repeat;
                    outcome.changed = true;
                }
            }
            CoreCommand::ShuffleQueue => self.set_shuffle(!self.shuffle, &mut outcome),
            CoreCommand::SetShuffle(enabled) => self.set_shuffle(enabled, &mut outcome),
            CoreCommand::SetTrackFavorite { id, favorite } => {
                self.set_track_favorite(&id, favorite, &mut outcome)
            }
            CoreCommand::Append(tracks) => {
                let at = self.queue.len();
                self.insert_rows(at, tracks, &mut outcome);
            }
            CoreCommand::InsertMany { at, tracks } => self.insert_rows(at, tracks, &mut outcome),
            CoreCommand::Recover { position } => self.recover(position, &mut outcome),
            CoreCommand::ResumeAt(position) => self.resume_at(position, &mut outcome),
        }
        outcome
    }

    /// Replaces the queue and starts at `start`.
    pub(crate) fn load(&mut self, tracks: Vec<Track>, start: usize, paused: bool) -> Outcome {
        let mut outcome = Outcome::default();
        self.repeat = Repeat::Off;
        self.queue = Arc::new(tracks);
        self.index = start.min(self.queue.len().saturating_sub(1));
        self.revision += 1;
        self.history.clear();
        self.shuffle_plan.clear();
        self.duration = 0.0;
        self.stream_size = 0;
        self.seekable = false;
        self.failed = false;
        self.anchor_position = 0.0;
        self.rebase = true;
        outcome.queue_reset = true;
        outcome.changed = true;
        if self.queue.is_empty() {
            self.state = State::Stopped;
            self.loading = false;
            self.gapless_next = None;
            outcome.audio.push(AudioCommand::Stop);
            return outcome;
        }
        if self.shuffle {
            self.rebuild_shuffle_plan();
        }
        self.start_current(paused, &mut outcome);
        outcome
    }

    // --- engine events ----------------------------------------------------

    pub(crate) fn on_engine(&mut self, event: EngineEvent, now_ns: u64) -> Outcome {
        self.observe(now_ns);
        let mut outcome = Outcome::default();
        match event {
            EngineEvent::FirstPcm => {
                // Playback actually started: the spinner can go away even if
                // the buffer heuristic has not fired yet.
                self.clear_loading(now_ns, &mut outcome);
            }
            EngineEvent::Loaded {
                duration,
                stream_size,
                seekable,
            } => {
                let duration = duration.max(0.0);
                // A parked decoder reopens and seeks to the decoded boundary
                // while the stream plays; it reports the same track again. Only
                // a real change (a different track or a cleared failure) is
                // worth republishing.
                let changed = (duration - self.duration).abs() > f64::EPSILON
                    || stream_size != self.stream_size
                    || seekable != self.seekable
                    || self.failed;
                self.duration = duration;
                self.stream_size = stream_size;
                self.seekable = seekable;
                self.failed = false;
                self.loaded_current = true;
                outcome.changed = changed;
            }
            EngineEvent::Boundary {
                duration,
                stream_size,
                position,
            } => self.boundary(duration, stream_size, position, now_ns, &mut outcome),
            EngineEvent::Ended => self.advance_after_error(&mut outcome),
            EngineEvent::Failed { error } => {
                let failure_position = self.position_at(now_ns);
                outcome.failure_position = Some(failure_position);
                let title = self
                    .queue
                    .get(self.index)
                    .map(|track| track.title.clone())
                    .unwrap_or_default();
                self.last_error = Some(if title.is_empty() {
                    error.message.clone()
                } else {
                    format!("{title}: {}", error.message)
                });
                self.failed = true;
                self.last_failure = Some(error.clone());
                outcome.failure = Some(error.clone());

                let remote = self
                    .queue
                    .get(self.index)
                    .is_some_and(|track| crate::player::is_remote_url(&track.url));
                if remote && !self.loaded_current && self.state == State::Paused {
                    // A restored queue pre-opens its row while paused. That
                    // failure is not a play attempt: keep the paused row and
                    // let the next Play report it. The retry budget is not
                    // touched by a pre-open.
                    outcome.deferred_failure = true;
                    return outcome;
                }

                if error.retryable() {
                    // Keep the row/index and let the service reopen the
                    // stream at the stored position. No advance, no toast yet.
                    self.retrying = true;
                    self.loading = false;
                    outcome.changed = true;
                    return outcome;
                }
                if error.is_auth() {
                    // Credentials are wrong for every row: advancing would
                    // toast once per track and drain the queue.
                    self.stop(&mut outcome);
                    return outcome;
                }
                self.advance_after_error(&mut outcome);
            }
            EngineEvent::SinkFailed { message } => {
                // The PipeWire stream died: freeze the playhead and do NOT
                // advance. The service drops and rebuilds the engine, then
                // sends `CoreCommand::Recover`.
                self.anchor_position = self.position_at(now_ns);
                self.anchor_ns = now_ns;
                self.rebase = true;
                self.loading = false;
                self.last_error = Some(message);
                outcome.changed = true;
            }
            EngineEvent::SinkRecovered => {
                // A fresh engine reported a live stream. Recovery itself is
                // driven by `CoreCommand::Recover` (which replays the row);
                // here we only clear the failure flag.
                if self.failed {
                    self.failed = false;
                    outcome.changed = true;
                }
            }
            EngineEvent::LoadingCleared => self.clear_loading(now_ns, &mut outcome),
        }
        outcome
    }

    // --- transport helpers ------------------------------------------------

    fn play_pause(&mut self, outcome: &mut Outcome) {
        match self.state {
            State::Playing => self.pause(outcome),
            State::Paused => self.resume(outcome),
            State::Stopped => {
                if !self.queue.is_empty() {
                    self.start_current(false, outcome);
                }
            }
        }
    }

    fn play(&mut self, outcome: &mut Outcome) {
        match self.state {
            State::Paused => self.resume(outcome),
            State::Stopped => {
                if !self.queue.is_empty() {
                    self.start_current(false, outcome);
                }
            }
            State::Playing => {}
        }
    }

    fn pause(&mut self, outcome: &mut Outcome) {
        if self.state != State::Playing {
            return;
        }
        self.anchor_position = self.position_at(self.now_ns);
        self.state = State::Paused;
        self.rebase = true;
        outcome.audio.push(AudioCommand::Pause);
        outcome.changed = true;
    }

    fn resume(&mut self, outcome: &mut Outcome) {
        match self.state {
            State::Paused => {
                if self.offline_blocks_current() {
                    self.refuse_offline(outcome);
                    return;
                }
                self.state = State::Playing;
                // The anchor was frozen at pause; re-base it to the clock
                // without moving the displayed position.
                self.rebase = true;
                outcome.audio.push(AudioCommand::Resume);
                outcome.changed = true;
            }
            State::Stopped => {
                if !self.queue.is_empty() {
                    self.start_current(false, outcome);
                }
            }
            State::Playing => {}
        }
    }

    /// The selected row needs the server while the transport is offline.
    fn offline_blocks_current(&self) -> bool {
        self.offline
            && self
                .queue
                .get(self.index)
                .is_some_and(|track| crate::player::is_remote_url(&track.url))
    }

    /// Stops with the canonical server-unavailable verdict.
    fn refuse_offline(&mut self, outcome: &mut Outcome) {
        self.failed = true;
        self.last_error = Some(crate::player::SERVER_UNAVAILABLE.to_string());
        self.stop(outcome);
        outcome.server_unavailable = true;
        outcome.changed = true;
    }

    fn stop(&mut self, outcome: &mut Outcome) {
        let was_active = self.state != State::Stopped || self.loading;
        self.state = State::Stopped;
        self.loading = false;
        self.loaded_current = false;
        self.retrying = false;
        self.gapless_next = None;
        self.anchor_position = 0.0;
        self.rebase = true;
        if was_active {
            outcome.audio.push(AudioCommand::Stop);
            outcome.changed = true;
        }
    }

    fn clear_loading(&mut self, now_ns: u64, outcome: &mut Outcome) {
        if !self.loading {
            return;
        }
        self.loading = false;
        // Start the playhead from *now*, not from the load, so a slow stream
        // does not show a position that was never audible.
        self.anchor_ns = now_ns;
        self.rebase = false;
        outcome.changed = true;
    }

    fn seek(&mut self, seconds: f64, outcome: &mut Outcome) {
        let position = seconds.max(0.0);
        self.anchor_position = position;
        self.rebase = true;
        if self.duration > 0.0 {
            self.anchor_position = position.min(self.duration);
        }
        // A stopped transport has no decoder to seek (and must not be started
        // by a seek); only the displayed clock moves until the next play.
        if self.state != State::Stopped {
            outcome.audio.push(AudioCommand::Seek(self.anchor_position));
        }
        outcome.changed = true;
    }

    fn jump(&mut self, index: usize, outcome: &mut Outcome) {
        if index >= self.queue.len() {
            return;
        }
        self.repeat = Repeat::Off;
        self.history_push(self.index);
        if self.shuffle {
            self.shuffle_plan.retain(|candidate| *candidate != index);
        }
        self.index = index;
        outcome.changed = true;
        self.start_current(false, outcome);
    }

    fn set_normalization(&mut self, mode: Normalization, outcome: &mut Outcome) {
        let changed = mode != self.normalization;
        self.normalization = mode;
        outcome.audio.push(AudioCommand::SetNormalization(mode));
        if changed {
            outcome.changed = true;
        }
    }

    fn set_crossfade(&mut self, enabled: bool, outcome: &mut Outcome) {
        let changed = enabled != self.crossfade.enabled;
        self.crossfade.enabled = enabled;
        outcome
            .audio
            .push(AudioCommand::SetCrossfadeConfig(self.crossfade));
        if self.crossfade.enabled {
            outcome
                .audio
                .push(AudioCommand::SetCrossfadeAllowed(self.crossfade_allowed()));
        }
        if changed {
            outcome.changed = true;
        }
    }

    fn set_crossfade_config(&mut self, config: CrossfadeConfig, outcome: &mut Outcome) {
        let config = config.sanitized();
        let changed = config != self.crossfade;
        self.crossfade = config;
        outcome.audio.push(AudioCommand::SetCrossfadeConfig(config));
        if self.crossfade.enabled {
            outcome
                .audio
                .push(AudioCommand::SetCrossfadeAllowed(self.crossfade_allowed()));
        }
        if changed {
            outcome.changed = true;
        }
    }

    fn set_volume(&mut self, volume: f64, outcome: &mut Outcome) {
        let volume = volume.clamp(0.0, 100.0);
        let changed = (volume - self.volume).abs() > f64::EPSILON;
        self.volume = volume;
        outcome.audio.push(AudioCommand::SetVolume(volume));
        if changed {
            outcome.changed = true;
        }
    }

    fn set_muted(&mut self, muted: bool, outcome: &mut Outcome) {
        let changed = muted != self.muted;
        self.muted = muted;
        outcome.audio.push(AudioCommand::SetMuted(muted));
        if changed {
            outcome.changed = true;
        }
    }

    /// Move forward by `steps` rows, loading at most once (old `advance_by`).
    /// Bounded actual-play history: the stack Previous walks.
    fn history_push(&mut self, index: usize) {
        if self.history.last().copied() == Some(index) {
            return;
        }
        self.history.push(index);
        let cap = 128;
        if self.history.len() > cap {
            let drain = self.history.len() - cap;
            self.history.drain(..drain);
        }
    }

    /// Rebuilds the upcoming shuffle indices around the current row. Structural
    /// queue edits call this because their indices shift; ordinary play does
    /// not (the plan is consumed one row at a time).
    fn rebuild_shuffle_plan(&mut self) {
        self.shuffle_plan.clear();
        if !self.shuffle || self.queue.len() <= 1 {
            return;
        }
        self.shuffle_plan = (0..self.queue.len())
            .filter(|index| *index != self.index)
            .collect();
        crate::queue::shuffle_slice(
            &mut self.shuffle_plan,
            crate::consts::now_nanos(),
        );
    }

    /// Structural edits shift row indices; rebuild the plan and drop stale
    /// history rather than walking removed rows.
    fn shuffle_after_edit(&mut self) {
        self.history.clear();
        self.rebuild_shuffle_plan();
    }

    /// The next row for `from`, consuming one shuffle-plan slot. `None` means
    /// the queue is exhausted under the current repeat mode.
    fn advance_index(&mut self, from: usize) -> Option<usize> {
        if self.shuffle {
            if self.shuffle_plan.is_empty() && self.repeat == Repeat::All {
                self.rebuild_shuffle_plan();
            }
            return if self.shuffle_plan.is_empty() {
                None
            } else {
                Some(self.shuffle_plan.remove(0))
            };
        }
        if from + 1 < self.queue.len() {
            Some(from + 1)
        } else if self.repeat == Repeat::All {
            Some(0)
        } else {
            None
        }
    }

    fn advance_by(&mut self, steps: usize, outcome: &mut Outcome) {        if self.queue.is_empty() || steps == 0 {
            return;
        }
        let mut index = self.index;
        for _ in 0..steps {
            let Some(next) = self.advance_index(index) else {
                self.stop(outcome);
                return;
            };
            if next != index {
                self.history_push(index);
                index = next;
            }
        }
        if index != self.index {
            self.index = index;
            self.start_current(false, outcome);
        }
    }

    /// Move backward through actual play history, falling back to the linear
    /// previous row when the history is empty.
    fn retreat_by(&mut self, steps: usize, outcome: &mut Outcome) {
        if self.queue.is_empty() || steps == 0 {
            return;
        }
        let mut index = self.index;
        if self.shuffle {
            for _ in 0..steps {
                index = self
                    .history
                    .pop()
                    .unwrap_or_else(|| index.saturating_sub(1));
            }
        } else {
            index = index.saturating_sub(steps);
        }
        if index != self.index && index < self.queue.len() {
            self.index = index;
            if self.shuffle {
                self.shuffle_plan.retain(|candidate| *candidate != index);
            }
            self.start_current(false, outcome);
        }
    }

    /// `repeat One` replays, otherwise advance through the active order
    /// (linear or shuffle), wrap with `repeat All`, else stop.
    fn advance_after_error(&mut self, outcome: &mut Outcome) {
        if self.queue.is_empty() {
            self.stop(outcome);
            return;
        }
        if self.repeat == Repeat::One {
            self.start_current(false, outcome);
            return;
        }
        if let Some(next) = self.advance_index(self.index) {
            self.history_push(self.index);
            self.index = next;
            self.start_current(false, outcome);
        } else {
            self.stop(outcome);
        }
    }

    fn boundary(
        &mut self,
        duration: f64,
        stream_size: u64,
        position: f64,
        now_ns: u64,
        outcome: &mut Outcome,
    ) {
        let Some(next) = self.gapless_next.take() else {
            return;
        };
        if next >= self.queue.len() {
            return;
        }
        self.history_push(self.index);
        self.index = next;
        if self.shuffle {
            self.shuffle_plan.retain(|candidate| *candidate != next);
        }
        self.gapless_next = self.gapless_index();
        if self.crossfade.enabled {
            outcome
                .audio
                .push(AudioCommand::SetCrossfadeAllowed(self.crossfade_allowed()));
        }
        self.duration = duration.max(0.0);
        self.stream_size = stream_size;
        self.loading = false;
        // The worker already swapped decoders: the next track starts at 0 for
        // a gapless handoff, or at the crossfade offset when the next track's
        // head was mixed into the old track's tail.
        self.anchor_position = position.max(0.0);
        self.anchor_ns = now_ns;
        self.rebase = false;
        outcome.changed = true;
    }

    /// Crossfade is only legal when the next row exists and, for album-only,
    /// shares the current non-empty album.
    fn crossfade_allowed(&self) -> bool {
        if !self.crossfade.enabled {
            return false;
        }
        let Some(next) = self.gapless_next else {
            return false;
        };
        if !self.crossfade.album_only {
            return true;
        }
        let Some(current) = self.queue.get(self.index) else {
            return false;
        };
        let Some(next) = self.queue.get(next) else {
            return false;
        };
        !current.album_id.is_empty() && current.album_id == next.album_id
    }

    /// Loads the current row into the engine (old `Player::rebuild`).
    fn start_current(&mut self, paused: bool, outcome: &mut Outcome) {
        let Some(track) = self.queue.get(self.index).cloned() else {
            self.state = State::Stopped;
            return;
        };
        // Offline rows are local-only. Refusing the load here (instead of
        // letting FFmpeg wait on an HTTP connect) is what makes a stale remote
        // queue row fail gracefully and immediately.
        if !paused && self.offline_blocks_current() {
            self.refuse_offline(outcome);
            return;
        }
        self.retrying = false;
        self.duration = 0.0;
        self.stream_size = 0;
        self.seekable = false;
        self.loaded_current = false;
        // A paused load is not going to start audio, so it must not claim the
        // loading state: on app open the restored queue loads paused and the
        // transport should read "Paused" immediately, not spin until the
        // decoder reports ready (it may never, e.g. offline).
        self.loading = !paused;
        self.failed = false;
        self.gapless_next = self.gapless_index();
        let next_url = self
            .gapless_next
            .and_then(|index| self.queue.get(index))
            .map(|next| next.url.clone());
        self.generation = self.generation.wrapping_add(1);
        let crossfade_allowed = self.crossfade_allowed();
        outcome.audio.push(AudioCommand::Load {
            url: track.url,
            next_url,
            paused,
            generation: self.generation,
        });
        if self.crossfade.enabled {
            outcome
                .audio
                .push(AudioCommand::SetCrossfadeAllowed(crossfade_allowed));
        }
        self.state = if paused {
            State::Paused
        } else {
            State::Playing
        };
        self.anchor_position = 0.0;
        self.anchor_ns = self.now_ns;
        self.rebase = true;
        outcome.changed = true;
    }

    /// Reloads the current row at `position` without changing the transport.
    fn recover(&mut self, position: f64, outcome: &mut Outcome) {
        if self.queue.is_empty() || self.state == State::Stopped {
            return;
        }
        let paused = self.state != State::Playing;
        self.load_current_at(position, paused, outcome);
    }

    /// Retry path: reload the current row at `position` and play it.
    fn resume_at(&mut self, position: f64, outcome: &mut Outcome) {
        if self.queue.is_empty() {
            return;
        }
        self.state = State::Playing;
        self.load_current_at(position, false, outcome);
    }

    /// Shared engine reload at an explicit position. `paused` preserves the
    /// sink-recovery behaviour; the retry path always asks for audio.
    fn load_current_at(&mut self, position: f64, paused: bool, outcome: &mut Outcome) {
        let Some(track) = self.queue.get(self.index).cloned() else {
            return;
        };
        if self.offline_blocks_current() {
            self.refuse_offline(outcome);
            return;
        }
        let position = position.max(0.0);
        self.retrying = false;
        self.duration = 0.0;
        self.stream_size = 0;
        self.seekable = false;
        self.loaded_current = false;
        self.loading = !paused;
        self.failed = false;
        self.gapless_next = self.gapless_index();
        let next_url = self
            .gapless_next
            .and_then(|index| self.queue.get(index))
            .map(|next| next.url.clone());
        self.generation = self.generation.wrapping_add(1);
        let crossfade_allowed = self.crossfade_allowed();
        outcome.audio.push(AudioCommand::Load {
            url: track.url,
            next_url,
            paused,
            generation: self.generation,
        });
        if self.crossfade.enabled {
            outcome
                .audio
                .push(AudioCommand::SetCrossfadeAllowed(crossfade_allowed));
        }
        self.state = if paused {
            State::Paused
        } else {
            State::Playing
        };
        self.anchor_position = position;
        self.anchor_ns = self.now_ns;
        self.rebase = true;
        if position > 0.0 {
            outcome.audio.push(AudioCommand::Seek(position));
        }
        outcome.changed = true;
    }

    // --- queue rows -------------------------------------------------------

    fn remove(&mut self, index: usize, outcome: &mut Outcome) {
        if index >= self.queue.len() {
            return;
        }
        let was_current = index == self.index;
        Arc::make_mut(&mut self.queue).remove(index);
        self.revision += 1;
        outcome.changed = true;
        outcome.delta = Some(QueueDelta {
            rev: self.revision,
            op: QueueOp::Remove {
                indices: vec![index.min(u32::MAX as usize) as u32],
            },
        });
        if self.queue.is_empty() {
            self.stop(outcome);
            return;
        }
        if was_current {
            if self.index >= self.queue.len() {
                self.index = self.queue.len() - 1;
            }
        } else if index < self.index {
            self.index -= 1;
        }
        self.shuffle_after_edit();
        if was_current {
            self.start_current(false, outcome);
            return;
        }
        self.gapless_next = self.gapless_index();
    }

    fn remove_many(&mut self, indices: &[usize], outcome: &mut Outcome) {
        let mut indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&index| index < self.queue.len())
            .collect();
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return;
        }
        let old_index = self.index;
        let removed_current = indices.binary_search(&old_index).is_ok();
        let below = indices.partition_point(|&index| index < old_index);
        for index in indices.iter().rev() {
            Arc::make_mut(&mut self.queue).remove(*index);
        }
        self.revision += 1;
        outcome.changed = true;
        outcome.delta = Some(QueueDelta {
            rev: self.revision,
            op: QueueOp::Remove {
                indices: indices
                    .iter()
                    .map(|index| (*index).min(u32::MAX as usize) as u32)
                    .collect(),
            },
        });
        if self.queue.is_empty() {
            self.stop(outcome);
            return;
        }
        self.index = if removed_current {
            old_index.saturating_sub(below).min(self.queue.len() - 1)
        } else {
            old_index - below
        };
        self.shuffle_after_edit();
        if removed_current {
            self.start_current(false, outcome);
        } else {
            self.gapless_next = self.gapless_index();
        }
    }

    fn move_item(&mut self, from: usize, to: usize, outcome: &mut Outcome) {
        if from >= self.queue.len() || to >= self.queue.len() || from == to {
            return;
        }
        let queue = Arc::make_mut(&mut self.queue);
        let track = queue.remove(from);
        queue.insert(to, track);
        self.revision += 1;
        if self.index == from {
            self.index = to;
        } else {
            if from < self.index {
                self.index -= 1;
            }
            if to <= self.index {
                self.index += 1;
            }
        }
        outcome.changed = true;
        outcome.delta = Some(QueueDelta {
            rev: self.revision,
            op: QueueOp::Move {
                from: from.min(u32::MAX as usize) as u32,
                to: to.min(u32::MAX as usize) as u32,
            },
        });
        self.shuffle_after_edit();
        self.gapless_next = self.gapless_index();
    }

    fn move_block(&mut self, start: usize, len: usize, delta: i32, outcome: &mut Outcome) {
        if len == 0 || delta == 0 || start >= self.queue.len() {
            return;
        }
        let end = (start + len).min(self.queue.len());
        let block_len = end - start;
        let new_start = start as i64 + i64::from(delta);
        if new_start < 0 || new_start as usize + block_len > self.queue.len() {
            return;
        }
        let new_start = new_start as usize;
        if new_start == start {
            return;
        }
        let old_index = self.index;
        let block: Vec<Track> = Arc::make_mut(&mut self.queue).drain(start..end).collect();
        Arc::make_mut(&mut self.queue).splice(new_start..new_start, block);
        self.revision += 1;

        self.index = if old_index >= start && old_index < end {
            (old_index as i64 + i64::from(delta)) as usize
        } else if delta < 0 && old_index >= new_start && old_index < start {
            old_index + block_len
        } else if delta > 0 && old_index >= end && old_index < new_start + block_len {
            old_index - block_len
        } else {
            old_index
        };
        outcome.changed = true;
        if block_len == 1 {
            outcome.delta = Some(QueueDelta {
                rev: self.revision,
                op: QueueOp::Move {
                    from: start.min(u32::MAX as usize) as u32,
                    to: new_start.min(u32::MAX as usize) as u32,
                },
            });
        } else {
            // A multi-row block move has no single queue op: the clients
            // re-stream the rows instead.
            outcome.queue_reset = true;
        }
        self.shuffle_after_edit();
        self.gapless_next = self.gapless_index();
    }

    fn insert_rows(&mut self, at: usize, tracks: Vec<Track>, outcome: &mut Outcome) {
        if tracks.is_empty() {
            return;
        }
        let at = at.min(self.queue.len());
        let count = tracks.len();
        let was_empty = self.queue.is_empty();
        Arc::make_mut(&mut self.queue).splice(at..at, tracks);
        if was_empty {
            // Old `insert_many` shifted the cursor past the end on an empty
            // queue; keep the head selected instead.
            self.index = 0;
        } else if at <= self.index {
            self.index += count;
        }
        self.revision += 1;
        outcome.changed = true;
        self.shuffle_after_edit();
        if count <= crate::ipc_protocol::QUEUE_INSERT_MAX_ROWS {
            let rows: Vec<WireTrack> = self
                .queue
                .iter()
                .skip(at)
                .take(count)
                .map(WireTrack::from)
                .collect();
            outcome.delta = Some(QueueDelta {
                rev: self.revision,
                op: QueueOp::Insert {
                    at: at.min(u32::MAX as usize) as u32,
                    rows,
                },
            });
        } else {
            outcome.queue_reset = true;
        }
        self.gapless_next = self.gapless_index();
    }

    fn set_shuffle(&mut self, enabled: bool, outcome: &mut Outcome) {
        if self.shuffle == enabled {
            return;
        }
        self.shuffle = enabled;
        self.shuffle_plan.clear();
        if enabled {
            self.rebuild_shuffle_plan();
            // A single-row queue is still a valid (trivial) shuffle mode; the
            // flag is user state, not a promise that the next row differs.
        }
        outcome.changed = true;
    }

    fn set_track_favorite(&mut self, id: &str, favorite: bool, outcome: &mut Outcome) {
        let mut matches = Vec::new();
        for (index, track) in Arc::make_mut(&mut self.queue).iter_mut().enumerate() {
            if track.id == id {
                track.favorite = favorite;
                matches.push(index);
            }
        }
        if matches.is_empty() {
            return;
        }
        self.revision += 1;
        outcome.changed = true;
        if matches.len() == 1 {
            outcome.delta = Some(QueueDelta {
                rev: self.revision,
                op: QueueOp::Patch {
                    index: matches[0].min(u32::MAX as usize) as u32,
                    favorite,
                },
            });
        } else {
            // Several rows share the id: patch them all via a row re-stream.
            outcome.queue_reset = true;
        }
    }

    // --- internals --------------------------------------------------------

    fn observe(&mut self, now_ns: u64) {
        self.now_ns = now_ns;
        if self.rebase {
            self.anchor_ns = now_ns;
            self.rebase = false;
        }
    }

    fn clamp_position(&self, position: f64) -> f64 {
        let duration = self.duration();
        if duration > 0.0 {
            position.min(duration)
        } else {
            position
        }
    }

    /// Row the gapless rules pick after the current one.
    fn gapless_index(&self) -> Option<usize> {
        if self.queue.is_empty() {
            return None;
        }
        if self.repeat == Repeat::One {
            Some(self.index)
        } else if self.shuffle {
            self.shuffle_plan.first().copied()
        } else if self.index + 1 < self.queue.len() {
            Some(self.index + 1)
        } else if self.repeat == Repeat::All {
            Some(0)
        } else {
            None
        }
    }

    fn state_byte(&self) -> u8 {
        match self.state {
            State::Stopped => 0,
            State::Playing => 1,
            State::Paused => 2,
        }
    }
}

fn seconds_to_ms(seconds: f64) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    let millis = (seconds * 1000.0).round();
    if millis >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        millis as u32
    }
}
