//! Long-lived decode and preload workers.
//!
//! One worker owns the current [`Decoder`]; a second persistent worker opens
//! the next track ahead of the boundary. No thread is created per track and
//! the old `malloc_trim`/reaper churn is gone.
//!
//! Pause keeps a seekable decoder alive: the loop stops reading/writing while
//! `current.paused` is set (so a queued `Seek`/`Resume` is serviced
//! immediately instead of waiting for the PCM ring to drain) and the ring keeps
//! its buffered audio, so resume continues from the same sample without
//! reopening or re-seeking. A pause longer than [`PAUSE_REOPEN_AFTER`] falls
//! back to interrupt + reopen because an idle HTTP connection is not reliable.
//! Non-seekable inputs always interrupt on pause and reopen at the paused
//! position on resume.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::decode::{Decoder, MAX_PIPELINE_RATE};
use super::ring;
use super::sink::PcmSink;
use super::status::{Shared, Status};
use crate::player::Normalization;

/// A pause longer than this reopens the stream instead of resuming the decoder
/// in memory: a long-idle HTTP response is not reliable.
const PAUSE_REOPEN_AFTER: Duration = Duration::from_secs(30);

/// True when a decoder has been idle long enough that its HTTP connection can
/// no longer be trusted; resume must reopen instead of reading it.
fn pause_is_stale(paused_at: Option<Instant>, now: Instant) -> bool {
    paused_at.is_some_and(|at| now.saturating_duration_since(at) > PAUSE_REOPEN_AFTER)
}

/// Optional equal-length crossfade between the outgoing and incoming track,
/// when enabled from the UI. The fade starts when the decoded position is this
/// close to the end; the next decoder is preloaded at the current pipeline rate
/// so both streams can be mixed without a graph rebuild.
/// Crossfade length from the shared config, clamped defensively.
fn crossfade_secs(shared: &Shared) -> f64 {
    f64::from(shared.crossfade_millis.load(Ordering::Acquire)) / 1000.0
}

/// Target temporal size of one decode read: 20 ms (`rate / 50` frames). Keeping
/// the chunk time-constant (not frame-constant) keeps seek/stop responses fast
/// at high rates and the PCM scratch buffer bounded.
const CHUNK_DIVISOR: u32 = 50;
/// Lower clamp for very low-rate sources (still only a few milliseconds).
const MIN_CHUNK_FRAMES: usize = 256;
/// Largest chunk: 20 ms at [`MAX_PIPELINE_RATE`]. The worker's PCM scratch
/// buffer is allocated for this size once.
const MAX_CHUNK_FRAMES: usize = MAX_PIPELINE_RATE as usize / CHUNK_DIVISOR as usize;

/// Frames to read per decoder call at the current pipeline rate.
fn chunk_frames(rate: u32) -> usize {
    ((rate / CHUNK_DIVISOR) as usize).clamp(MIN_CHUNK_FRAMES, MAX_CHUNK_FRAMES)
}

/// Minimum PCM queued before releasing playback on a remote Load, Seek or
/// reopen. Absorbs initial HTTP/TLS/FLAC burst latency without immediately
/// starving after the first 20 ms chunk.
const PREBUFFER_REMOTE_SECS: f64 = 1.0;
/// Minimum PCM queued before resuming after a mid-track ring underrun. Longer
/// than the initial prebuffer so a slow connection does not stutter repeatedly.
const REBUFFER_SECS: f64 = 2.0;

fn buffer_target_frames(seconds: f64, rate: u32, usable_limit: u64) -> u64 {
    let wanted = (seconds * f64::from(rate.max(1))).round().max(1.0) as u64;
    let cap = (usable_limit / 2).max(1);
    wanted.clamp(1, cap)
}

fn set_decoder_live(shared: &Shared, live: bool) {
    shared.decoder_live.store(live, Ordering::Release);
    shared.lock().decoder_live = live;
}

pub(crate) enum Command {
    Load {
        url: String,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
        fade_out: bool,
    },
    SetNextUrl(Option<String>),
    Pause,
    Resume {
        position: f64,
    },
    Seek(f64),
    Stop {
        fade_out: bool,
    },
    Shutdown,
}

struct PreloadRequest {
    token: u64,
    url: String,
    stop: Arc<AtomicBool>,
    /// Decode at this pipeline rate when crossfade is enabled, so the head of
    /// the next track can be mixed into the current track's tail.
    target_rate: Option<u32>,
}

struct PreloadResult {
    token: u64,
    result: Result<Decoder, String>,
}

pub(crate) struct Worker {
    tx: Option<Sender<Command>>,
    join: Option<JoinHandle<()>>,
    preload_tx: Option<Sender<PreloadRequest>>,
    preload_join: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
    sink: Arc<dyn PcmSink>,
    paused_position: std::sync::Mutex<f64>,
    paused: AtomicBool,
}

impl Worker {
    pub(crate) fn new(shared: Arc<Shared>, sink: Arc<dyn PcmSink>) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (preload_tx, preload_rx) = mpsc::channel::<PreloadRequest>();
        let (preload_result_tx, preload_result_rx) = mpsc::channel::<PreloadResult>();
        let preload_join = thread::Builder::new()
            .name("musika-preload".to_string())
            .spawn(move || preload_thread(preload_rx, preload_result_tx))
            .map_err(|error| format!("cannot start preload worker: {error}"))?;
        let worker_shared = Arc::clone(&shared);
        let worker_sink = Arc::clone(&sink);
        let worker_preload = preload_tx.clone();
        let join = thread::Builder::new()
            .name("musika-decode".to_string())
            .spawn(move || {
                worker_thread(
                    rx,
                    preload_result_rx,
                    worker_preload,
                    worker_shared,
                    worker_sink,
                )
            })
            .map_err(|error| format!("cannot start decode worker: {error}"))?;
        Ok(Self {
            tx: Some(tx),
            join: Some(join),
            preload_tx: Some(preload_tx),
            preload_join: Some(preload_join),
            shared,
            sink,
            paused_position: std::sync::Mutex::new(0.0),
            paused: AtomicBool::new(false),
        })
    }

    /// Loads `url`, preloading `next_url` ahead of the gapless boundary.
    ///
    /// `generation` lets the playback core reject superseded opens: the worker
    /// keeps the largest generation it has seen and drops any `Load` command
    /// that does not match it. A value that is not newer than the internal
    /// counter (legacy callers pass 0) falls back to that counter.
    pub(crate) fn load(
        &self,
        url: String,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    ) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        let generation = {
            let current = self.shared.generation.load(Ordering::Acquire);
            if generation > current {
                self.shared.generation.store(generation, Ordering::Release);
                generation
            } else {
                self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1
            }
        };
        let was_playing = !self.paused.load(Ordering::Acquire);
        self.paused.store(paused, Ordering::Release);
        *self
            .paused_position
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
        self.shared.decoder_live.store(false, Ordering::Release);
        {
            let mut status = self.shared.lock();
            status.loading = true;
            status.ended = false;
            status.failed = None;
            status.position = 0.0;
            status.duration = 0.0;
            status.stream_size = 0;
            status.seekable = false;
            status.decoder_live = false;
            status.loaded = false;
        }
        self.shared
            .clock_start
            .store(self.sink.played_frames(), Ordering::Release);
        *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
        self.shared.boundary_active.store(false, Ordering::Release);
        self.shared.decode_done.store(false, Ordering::Release);
        self.shared.first_pcm.store(false, Ordering::Release);
        let fade_out = was_playing && !paused;
        if fade_out {
            self.sink.request_fade_out();
        }
        interrupt_current(&self.shared);
        self.sink.close_writes();
        _ = tx.send(Command::Load {
            url,
            next_url,
            paused,
            generation,
            fade_out,
        });
    }

    pub(crate) fn set_next_url(&self, next_url: Option<String>) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        _ = tx.send(Command::SetNextUrl(next_url));
    }

    pub(crate) fn set_paused(&self, paused: bool, position: f64) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        self.paused.store(paused, Ordering::Release);
        if paused {
            *self
                .paused_position
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = position.max(0.0);
            // Freeze the sink immediately: the RT callback stops consuming and
            // fills silence, so audio stops on the next period.
            self.sink.set_paused(true);
            // Only a non-seekable decoder needs to be interrupted right here;
            // it can be blocked in a network read that would otherwise keep the
            // pause from being observed. A seekable decoder is kept alive and
            // the worker loop parks itself on the `Pause` command.
            let seekable = self.shared.lock().seekable;
            if !seekable {
                interrupt_current(&self.shared);
                self.sink.reset();
            }
            _ = tx.send(Command::Pause);
        } else {
            // The worker unpauses the sink once the audio is ready: a live
            // decoder can resume immediately, but a long-idle resume must reset
            // the ring and reopen the stream first (otherwise the stale buffered
            // tail plays and is then replayed from the pause point).
            let position = *self
                .paused_position
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            _ = tx.send(Command::Resume { position });
        }
    }

    pub(crate) fn seek(&self, seconds: f64) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        let seconds = seconds.max(0.0);
        *self
            .paused_position
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = seconds;
        self.shared
            .clock_start
            .store(self.sink.played_frames(), Ordering::Release);
        *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = seconds;
        let paused = self.paused.load(Ordering::Acquire);
        // A seek while paused must not leave the engine reporting `loading`
        // forever: nothing decodes until the worker is resumed.
        self.shared.lock().loading = !paused;
        if !paused {
            self.sink.request_fade_out();
        }
        _ = tx.send(Command::Seek(seconds));
    }

    pub(crate) fn stop(&self) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        let was_playing = !self.paused.load(Ordering::Acquire);
        self.paused.store(true, Ordering::Release);
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.boundary_active.store(false, Ordering::Release);
        self.shared.decode_done.store(false, Ordering::Release);
        self.shared.first_pcm.store(false, Ordering::Release);
        interrupt_current(&self.shared);
        {
            let mut status = self.shared.lock();
            *status = Status::default();
        }
        self.shared.clock_start.store(0, Ordering::Release);
        *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
        if was_playing {
            self.sink.request_fade_out();
            self.sink.close_writes();
        } else {
            self.sink.reset();
            self.sink.close_writes();
            self.sink.set_paused(true);
        }
        _ = tx.send(Command::Stop {
            fade_out: was_playing,
        });
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        interrupt_current(&self.shared);
        self.sink.reset();
        self.sink.close_writes();
        if let Some(tx) = self.tx.take() {
            _ = tx.send(Command::Shutdown);
        }
        if let Some(preload_tx) = self.preload_tx.take() {
            // Dropping the sender lets the preload thread observe the closed
            // channel and exit; a blocked open is interrupted by `stop`.
            drop(preload_tx);
        }
        if let Some(join) = self.join.take() {
            _ = join.join();
        }
        if let Some(join) = self.preload_join.take() {
            _ = join.join();
        }
    }
}

fn preload_thread(rx: Receiver<PreloadRequest>, results: Sender<PreloadResult>) {
    while let Ok(request) = rx.recv() {
        if request.stop.load(Ordering::Acquire) {
            _ = results.send(PreloadResult {
                token: request.token,
                result: Err("preload cancelled".to_string()),
            });
            continue;
        }
        let result = Decoder::open_with_output_rate(
            &request.url,
            Arc::clone(&request.stop),
            request.target_rate,
        );
        if results
            .send(PreloadResult {
                token: request.token,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

#[derive(Default)]
struct Current {
    decoder: Option<Decoder>,
    url: String,
    epoch: u64,
    generation: u64,
    written_frames: u64,
    timeline_start: f64,
    /// Pipeline rate (Hz) currently configured in the sink. PCM written for
    /// this decoder is at this rate.
    rate: u32,
    next_url: Option<String>,
    preload_requested: bool,
    preload_token: u64,
    /// Pipeline rate the current preload request targets (`None` = native).
    preload_target_rate: Option<u32>,
    preload: Option<Decoder>,
    /// Active crossfade: the preloaded decoder is being mixed with the tail of
    /// the current one.
    crossfade: Option<CrossfadeState>,
    /// A preloaded decoder whose rate differs from [`Current::rate`]. The
    /// gapless swap waits for the ring to drain before the format is switched,
    /// because queued frames still belong to the old rate.
    pending_handoff: Option<Decoder>,
    preload_stop: Option<Arc<AtomicBool>>,
    preload_started: Option<Instant>,
    first_pcm: bool,
    /// True when the active `ring.buffering` hold was armed for an initial
    /// remote load/seek/reopen (uses [`PREBUFFER_REMOTE_SECS`]); `false` uses
    /// [`REBUFFER_SECS`] for mid-track underrun recovery.
    initial_buffer: bool,
    stop: Option<Arc<AtomicBool>>,
    /// EQ chain for the current decoder; rebuilt when the profile or rate
    /// changes.
    eq: Option<crate::eq::EqRuntime>,
    eq_epoch: u64,
    eq_rate: u32,
    /// The decoder is kept open but not read: pause with a seekable input.
    paused: bool,
    paused_at: Option<Instant>,
    /// The transport was stopped: a stale seek/resume must not reopen audio.
    stopped: bool,
}

struct CrossfadeState {
    next: Decoder,
    next_stop: Option<Arc<AtomicBool>>,
    /// EQ state for the incoming decoder (independent filter history).
    next_eq: Option<crate::eq::EqRuntime>,
    eq_epoch: u64,
    equal_power: bool,
    /// Frames of the crossfade window mixed so far (advances with the outgoing
    /// track so a short or failed incoming track still fades out smoothly).
    played_frames: u64,
    /// Frames of `next` consumed by the fade so far (used for the handoff
    /// timeline offset).
    next_frames: u64,
    /// Total frames of the fade (the remaining duration when it started).
    total_frames: u64,
    /// Reading the next decoder failed; keep mixing silence and let the
    /// boundary/advance path handle the error.
    next_failed: bool,
}

/// Whether the decode loop has time-based work that a command cannot wake it
/// for. Keep this in step with the bottom of the loop: pending rate handoff,
/// preload and crossfade all advance on their own.
fn needs_timed_wait(current: &Current) -> bool {
    // A paused decoder produces no PCM and is woken by Resume/Seek, so it
    // blocks too; only work that advances without a command keeps polling.
    current.pending_handoff.is_some()
        || current.preload.is_some()
        || current.preload_requested
        || current.crossfade.is_some()
}

fn worker_thread(
    rx: Receiver<Command>,
    preload_rx: Receiver<PreloadResult>,
    preload_tx: Sender<PreloadRequest>,
    shared: Arc<Shared>,
    sink: Arc<dyn PcmSink>,
) {
    let mut current = Current {
        // Nothing is loaded yet: a stale seek/resume before the first load must
        // not reopen the (empty) URL.
        stopped: true,
        rate: sink.ring().rate(),
        ..Current::default()
    };
    let mut pcm = vec![0.0f32; MAX_CHUNK_FRAMES * ring::CHANNELS];
    let mut next_pcm = vec![0.0f32; MAX_CHUNK_FRAMES * ring::CHANNELS];
    let mut pending_command = None;
    loop {
        // A blocking wait is used whenever the loop has nothing to decode:
        // no decoder, or a paused one. Paused never produces PCM, so a queued
        // seek/resume is serviced immediately instead of waiting for the ring
        // to drain.
        let command = if let Some(command) = pending_command.take() {
            Some(command)
        } else if current.decoder.is_some() && !current.paused {
            match rx.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => break,
            }
        } else if needs_timed_wait(&current) {
            // A parked HTTP decoder, a pending rate handoff or an in-flight
            // preload makes time (not just a command) the thing to wait on.
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            // Nothing loaded and nothing parked: block until the next command.
            match rx.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };

        if let Some(command) = command {
            match command {
                Command::Shutdown => {
                    cancel_preload(&mut current);
                    interrupt_stop(&mut current);
                    break;
                }
                Command::Stop { fade_out } => {
                    cancel_preload(&mut current);
                    interrupt_stop(&mut current);
                    if fade_out {
                        sink.request_fade_out();
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    sink.reset();
                    sink.close_writes();
                    sink.ring().set_buffering(false);
                    sink.set_paused(true);
                    current.decoder = None;
                    current.stop = None;
                    current.paused = false;
                    current.paused_at = None;
                    current.initial_buffer = false;
                    current.stopped = true;
                    set_decoder_live(&shared, false);
                    continue;
                }
                Command::SetNextUrl(next_url) => {
                    if current.next_url != next_url {
                        current.next_url = next_url;
                        if current.crossfade.is_none() {
                            cancel_preload(&mut current);
                            current.preload = None;
                            current.preload_requested = false;
                            current.pending_handoff = None;
                            if !current.stopped && current.decoder.is_some() {
                                seed_preload(&mut current, &shared, &preload_tx);
                            }
                        }
                    }
                    continue;
                }
                Command::Pause => {
                    cancel_preload(&mut current);
                    // A live seekable decoder is kept: the ring keeps its
                    // buffered audio and resume continues from it.
                    let keep = current.decoder.as_ref().is_some_and(Decoder::is_seekable);
                    if keep {
                        current.paused = true;
                        current.paused_at = Some(Instant::now());
                        shared.lock().loading = false;
                    } else {
                        interrupt_stop(&mut current);
                        current.decoder = None;
                        current.stop = None;
                        current.paused = false;
                        current.paused_at = None;
                        current.initial_buffer = false;
                        sink.reset();
                        sink.ring().set_buffering(false);
                        shared.decoder_live.store(false, Ordering::Release);
                        let mut status = shared.lock();
                        status.loading = false;
                        status.decoder_live = false;
                        status.seekable = false;
                    }
                    continue;
                }
                Command::Resume { position } => {
                    let resume_at = position.max(0.0);
                    let long_idle = pause_is_stale(current.paused_at, Instant::now());
                    current.paused = false;
                    current.paused_at = None;
                    if current.decoder.is_some() {
                        if long_idle {
                            // The stream sat idle longer than an HTTP response
                            // can be trusted: reopen and seek to the playhead.
                            // The sink stays paused until the ring is reset and
                            // the decoder is ready; otherwise the pre-pause
                            // buffered tail is played and then replayed from the
                            // pause point (audible jump + backward clock jump).
                            let (url, next_url, generation) = (
                                current.url.clone(),
                                current.next_url.clone(),
                                current.generation,
                            );
                            cancel_preload(&mut current);
                            interrupt_stop(&mut current);
                            current.decoder = None;
                            sink.reset();
                            rebase_clock(&shared, sink.as_ref(), resume_at);
                            let prebuffer = crate::player::is_remote_url(&url);
                            current.initial_buffer = prebuffer;
                            sink.ring().set_buffering(prebuffer);
                            // Show the spinner while the new stream opens.
                            shared.lock().loading = true;
                            shared.signal();
                            start_decoder(
                                &mut current,
                                url,
                                next_url,
                                generation,
                                resume_at,
                                false,
                                true,
                                &shared,
                                sink.as_ref(),
                            );
                            // If the reopen failed the failure path/retry will
                            // unpause; playing a stale tail here would be worse.
                            if current.decoder.is_some() {
                                sink.set_paused(false);
                            }
                        } else {
                            // Live decoder: keep it, do not re-seek. If the
                            // user paused mid-rebuffer, keep holding until the
                            // ring refills instead of playing empty silence.
                            if !sink.ring().buffering() {
                                shared.lock().loading = false;
                            }
                            sink.set_paused(false);
                        }
                        continue;
                    }
                    if current.stopped {
                        continue;
                    }
                    // Interrupted at pause (non-seekable): reopen at the paused
                    // position; a stream that cannot seek restarts from 0 and
                    // reports the position reset through the engine.
                    let (url, next_url, generation) = (
                        current.url.clone(),
                        current.next_url.clone(),
                        current.generation,
                    );
                    let prebuffer = crate::player::is_remote_url(&url);
                    current.initial_buffer = prebuffer;
                    sink.ring().set_buffering(prebuffer);
                    shared.lock().loading = true;
                    shared.signal();
                    start_decoder(
                        &mut current,
                        url,
                        next_url,
                        generation,
                        resume_at,
                        false,
                        true,
                        &shared,
                        sink.as_ref(),
                    );
                    if current.decoder.is_some() {
                        sink.set_paused(false);
                    }
                    continue;
                }
                Command::Seek(mut seconds) => {
                    // A newer skip supersedes queued older skips. Keep
                    // command ordering by stopping at the first non-seek;
                    // that command is handled before any later seek.
                    while let Ok(command) = rx.try_recv() {
                        match command {
                            Command::Seek(newer) => seconds = newer,
                            Command::Load { .. }
                            | Command::SetNextUrl(_)
                            | Command::Pause
                            | Command::Resume { .. }
                            | Command::Stop { .. }
                            | Command::Shutdown => {
                                pending_command = Some(command);
                                break;
                            }
                        }
                    }
                    let seconds = seconds.max(0.0);
                    // A live non-seekable stream (transcoded radio/HTTP) cannot
                    // jump. Dropping the decoder here made every UI seek a
                    // retryable Seek failure (reopen+seek x3, then stop); ignore
                    // the seek and snap the displayed position back to the audio
                    // that is actually playing instead.
                    if let Some(decoder) = current.decoder.as_ref()
                        && !decoder.is_seekable()
                    {
                        let actual = shared
                            .clock_origin
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .max(0.0)
                            + (sink
                                .played_frames()
                                .saturating_sub(shared.clock_start.load(Ordering::Acquire)))
                                as f64
                                / f64::from(current.rate.max(1));
                        shared.request_position_reset(actual);
                        shared.lock().loading = false;
                        if current.paused {
                            current.paused_at = Some(Instant::now());
                        }
                        continue;
                    }
                    if !current.paused {
                        // Let the old audio fade out before the seek discards
                        // the queue; the new position fades in below.
                        sink.request_fade_out();
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    sink.flush();
                    let prebuffer = !current.paused && crate::player::is_remote_url(&current.url);
                    current.initial_buffer = prebuffer;
                    sink.ring().set_buffering(prebuffer);
                    if prebuffer {
                        shared.signal();
                    }
                    *shared
                        .clock_origin
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = seconds;
                    shared
                        .clock_start
                        .store(sink.played_frames(), Ordering::Release);
                    // A seek cuts the queued old audio; fade the new position's
                    // PCM in instead of starting it with a hard discontinuity.
                    // Remote streams defer the fade-in until prebuffering finishes.
                    if !current.paused && !prebuffer {
                        sink.request_fade_in();
                    }
                    cancel_preload(&mut current);
                    if let Some(decoder) = current.decoder.as_mut() {
                        match decoder.seek(seconds) {
                            Ok(()) => {
                                current.written_frames = 0;
                                current.timeline_start = seconds;
                                shared.decode_done.store(false, Ordering::Release);
                                let paused = current.paused;
                                shared.lock().loading = !paused;
                                if paused {
                                    // The decoder is repositioned; the HTTP
                                    // connection still ages from now.
                                    current.paused_at = Some(Instant::now());
                                }
                            }
                            Err(error) => {
                                fail_at(&shared, crate::player::ErrorStage::Seek, error);
                                current.decoder = None;
                                current.initial_buffer = false;
                                sink.ring().set_buffering(false);
                                set_decoder_live(&shared, false);
                            }
                        }
                    } else if !current.stopped {
                        // No live decoder (interrupted by a pause): reopen at
                        // the seek target so the position is honoured.
                        let (url, next_url, generation) = (
                            current.url.clone(),
                            current.next_url.clone(),
                            current.generation,
                        );
                        start_decoder(
                            &mut current,
                            url,
                            next_url,
                            generation,
                            seconds,
                            false,
                            true,
                            &shared,
                            sink.as_ref(),
                        );
                        if current.paused {
                            shared.lock().loading = false;
                            current.paused_at = Some(Instant::now());
                        }
                    }
                    continue;
                }
                Command::Load {
                    url,
                    next_url,
                    paused,
                    generation,
                    fade_out,
                } => {
                    // A burst of skips can enqueue several loads while a
                    // previous HTTP open is being interrupted. Only the
                    // newest generation should reach Navidrome/FFmpeg.
                    if generation != shared.generation.load(Ordering::Acquire) {
                        continue;
                    }
                    if fade_out {
                        // A manual track change cuts the old stream; fade it
                        // out before the reset instead of clicking.
                        sink.request_fade_out();
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    sink.reset();
                    // Apply the pause/active state before opening: a paused
                    // restore then leaves the PipeWire node suspended for the
                    // whole (possibly slow) network open instead of burning
                    // graph wakeups until `start_decoder` returns.
                    current.paused = paused;
                    // Remember when the row was opened paused: a restored row
                    // can sit paused for hours, and its HTTP connection must be
                    // treated like any other long pause on Play.
                    current.paused_at = paused.then(Instant::now);
                    // Hold remote streams in buffering state until the ring has
                    // accumulated `PREBUFFER_REMOTE_SECS` of decoded PCM so
                    // audio never starts dry and immediately starves.
                    let prebuffer = !paused && crate::player::is_remote_url(&url);
                    current.initial_buffer = prebuffer;
                    sink.ring().set_buffering(prebuffer);
                    sink.set_paused(paused);
                    if prebuffer {
                        shared.signal();
                    }
                    shared
                        .clock_start
                        .store(sink.played_frames(), Ordering::Release);
                    start_decoder(
                        &mut current,
                        url,
                        next_url,
                        generation,
                        0.0,
                        false,
                        false,
                        &shared,
                        sink.as_ref(),
                    );
                    continue;
                }
            }
        }

        // Paused with a live decoder: wait for the next command instead of
        // reading PCM. Reaching this branch without a command means the decode
        // loop has nothing to do.
        if current.paused {
            continue;
        }

        // Crossfade toggled on after the next decoder was seeded at its native
        // rate: re-request it at the current pipeline rate so it can be mixed
        // into this track's tail.
        if current.crossfade.is_none() && shared.crossfade.load(Ordering::Acquire) {
            let mismatched_ready = current
                .preload
                .as_ref()
                .is_some_and(|decoder| decoder.sample_rate() != current.rate);
            let mismatched_pending =
                current.preload_requested && current.preload_target_rate != Some(current.rate);
            if mismatched_ready || mismatched_pending {
                cancel_preload(&mut current);
            }
        }
        if current.crossfade.is_none()
            && shared.crossfade.load(Ordering::Acquire)
            && current.preload.is_none()
            && current.preload_requested
            && let Some(remaining) = crossfade_remaining(&current)
            && remaining <= crossfade_secs(&shared)
        {
            // A local file can be decoded far faster than the preload thread
            // gets scheduled; wait at the fade threshold so a short local
            // track still crossfades instead of falling back to a hard cut.
            if let Ok(result) = preload_rx.recv_timeout(Duration::from_millis(300)) {
                apply_preload(&mut current, result);
            }
        }
        if current.crossfade.is_none() {
            maybe_start_crossfade(&mut current, &shared);
        }
        if current.crossfade.is_some() {
            run_crossfade_chunk(
                &mut current,
                &shared,
                sink.as_ref(),
                &mut pcm,
                &mut next_pcm,
            );
            continue;
        }

        let Some(decoder) = current.decoder.as_mut() else {
            // A rate change at a gapless boundary is not gapless: the frames
            // still queued in the ring were produced at the old rate, so wait
            // for playback to drain them before renegotiating the stream and
            // swapping the decoder in.
            if let Some(next) = current.pending_handoff.take() {
                if sink.ring().buffered_frames() > 0 {
                    current.pending_handoff = Some(next);
                    continue;
                }
                let rate = next.sample_rate();
                if rate != current.rate {
                    match sink.set_format(rate) {
                        Ok(()) => current.rate = rate,
                        Err(error) => {
                            fail_at(&shared, crate::player::ErrorStage::Sink, error);
                            set_decoder_live(&shared, false);
                            continue;
                        }
                    }
                }
                let next_stop = current.preload_stop.take();
                handoff_to_decoder(&mut current, next, next_stop, 0.0, &shared, sink.as_ref());
                continue;
            }
            continue;
        };

        let frames = match decoder.read(&mut pcm, chunk_frames(current.rate)) {
            Ok(frames) => frames,
            Err(error) => {
                let superseded = current.generation != shared.generation.load(Ordering::Acquire)
                    || current
                        .stop
                        .as_ref()
                        .is_some_and(|stop| stop.load(Ordering::Acquire));
                current.decoder = None;
                current.stop = None;
                current.initial_buffer = false;
                sink.ring().set_buffering(false);
                set_decoder_live(&shared, false);
                if !superseded {
                    fail_at(&shared, crate::player::ErrorStage::Read, error);
                }
                continue;
            }
        };
        if frames == 0 {
            // A short track or near-EOF seek may finish decoding before the
            // ring reaches the prebuffer/rebuffer watermark: release whatever
            // was decoded so the tail plays out instead of stalling forever.
            _ = release_buffer_if_ready(&mut current, &shared, sink.as_ref(), true);
            // A very fast local decode can reach EOF before the preload
            // worker's result has been polled. Give that in-flight open a
            // short grace period so short tracks still hand off gaplessly.
            if current.preload.is_none()
                && current.preload_requested
                && let Ok(result) = preload_rx.recv_timeout(Duration::from_millis(300))
            {
                apply_preload(&mut current, result);
            }
            if let Some(next) = current.preload.take() {
                let next_rate = next.sample_rate();
                if next_rate != current.rate {
                    // Switching the graph rate in the middle of the stream
                    // would replay the queued old-rate frames at the wrong
                    // speed, so the swap waits for the ring to drain. A small
                    // audible gap at the rate change is accepted; same-rate
                    // handoffs stay gapless.
                    current.pending_handoff = Some(next);
                    current.decoder = None;
                    current.stop = None;
                    set_decoder_live(&shared, false);
                    continue;
                }
                let next_stop = current.preload_stop.take();
                handoff_to_decoder(&mut current, next, next_stop, 0.0, &shared, sink.as_ref());
                continue;
            }
            crate::startup::trace("fetch.done");
            shared.decode_done.store(true, Ordering::Release);
            set_decoder_live(&shared, false);
            shared.signal();
            current.decoder = None;
            continue;
        }

        let mode = Normalization::from_code(shared.normalization.load(Ordering::Acquire) as u8);
        if !shared.bit_perfect.load(Ordering::Acquire)
            && mode.enabled()
            && let Some(decoder) = current.decoder.as_ref()
            && let Some(gain) = normalization_gain_with_peak(
                mode,
                decoder.replaygain_track_gain_db(),
                decoder.replaygain_album_gain_db(),
                decoder.replaygain_track_peak(),
                decoder.replaygain_album_peak(),
            )
        {
            apply_normalization(&mut pcm[..frames * ring::CHANNELS], gain);
        }

        refresh_eq(&mut current, &shared);
        if let Some(eq) = current.eq.as_mut() {
            eq.process(&mut pcm[..frames * ring::CHANNELS], frames);
        }

        current.written_frames += frames as u64;
        let epoch = current.epoch;
        sink.write(&pcm[..frames * 2], epoch);
        if sink_ring_closed(sink.as_ref()) {
            // Load/stop replaced the epoch while the writer was blocked.
            current.decoder = None;
            current.initial_buffer = false;
            set_decoder_live(&shared, false);
            cancel_preload(&mut current);
            continue;
        }
        if release_buffer_if_ready(&mut current, &shared, sink.as_ref(), false) && current.first_pcm
        {
            current.first_pcm = false;
            shared.first_pcm.store(true, Ordering::Release);
            shared.signal();
            crate::startup::trace("first.pcm.written");
        }

        seed_preload(&mut current, &shared, &preload_tx);
        drain_preload(&mut current, &preload_rx);
        if sink.ring().paused()
            && current
                .preload_started
                .is_some_and(|started| started.elapsed() > Duration::from_secs(600))
        {
            cancel_preload(&mut current);
            current.next_url = None;
        }
    }

    cancel_preload(&mut current);
}

/// What to do with a decoder that was opened at a requested position.
///
/// Non-seekable streams (live/transcoded HTTP) cannot jump; restarting from 0
/// and reporting the position reset is the only way a pause/seek can return to
/// playback instead of entering the (always-retryable) Seek failure loop.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SeekPlan {
    /// Start at this (possibly zero) position without calling `seek`.
    Start(f64),
    /// Call `Decoder::seek` with this position.
    Seek(f64),
    /// The stream cannot seek: restart at 0 and emit a position reset.
    RestartFromZero,
}

fn seek_plan(position: f64, seekable: bool) -> SeekPlan {
    let position = position.max(0.0);
    if position <= 0.0 {
        SeekPlan::Start(position)
    } else if seekable {
        SeekPlan::Seek(position)
    } else {
        SeekPlan::RestartFromZero
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the decoder start is driven by the worker's current state plus the load parameters; grouping them into a struct would be a throwaway used by five call sites"
)]
fn start_decoder(
    current: &mut Current,
    url: String,
    next_url: Option<String>,
    generation: u64,
    position: f64,
    preserve_preload: bool,
    is_reopen: bool,
    shared: &Shared,
    sink: &dyn PcmSink,
) {
    if !preserve_preload {
        cancel_preload(current);
        current.preload = None;
        current.preload_requested = false;
        current.preload_token = current.preload_token.wrapping_add(1);
        current.preload_started = None;
        current.next_url = next_url.clone();
    }
    interrupt_stop(current);
    current.decoder = None;
    current.url = url.clone();
    current.epoch = sink.epoch();
    current.generation = generation;
    current.written_frames = 0;
    current.timeline_start = 0.0;
    current.pending_handoff = None;
    if !preserve_preload {
        current.next_url = next_url;
    }
    if !is_reopen {
        current.first_pcm = true;
    }
    current.stopped = false;
    current.eq = None;
    current.eq_epoch = 0;
    current.eq_rate = 0;
    shared.decode_done.store(false, Ordering::Release);
    let stop = Arc::new(AtomicBool::new(false));
    *shared
        .current_stop
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&stop));
    current.stop = Some(stop.clone());
    match Decoder::open(&url, Arc::clone(&stop)) {
        Ok(mut decoder) => {
            // A live/transcoded HTTP stream may not support seeking at all. A
            // failed seek used to kill the decoder and enter the retry loop
            // (`ErrorStage::Seek` is always retryable), so a pause+play on such
            // a stream never came back. Detect it up front: restart from 0 and
            // tell the core to re-anchor the displayed position.
            let seekable = decoder.is_seekable();
            let start_position = match seek_plan(position, seekable) {
                SeekPlan::Start(position) => position,
                SeekPlan::Seek(position) => {
                    if let Err(error) = decoder.seek(position) {
                        current.initial_buffer = false;
                        sink.ring().set_buffering(false);
                        if generation == shared.generation.load(Ordering::Acquire)
                            && !stop.load(Ordering::Acquire)
                        {
                            fail_at(shared, crate::player::ErrorStage::Seek, error);
                        }
                        set_decoder_live(shared, false);
                        return;
                    }
                    position
                }
                SeekPlan::RestartFromZero => {
                    shared.request_position_reset(0.0);
                    0.0
                }
            };
            current.timeline_start = start_position;
            crate::startup::trace("fetch.first_chunk");
            // Adopt the track's native rate before any PCM is written for it:
            // the sink rebuilds its PipeWire stream to F32LE/stereo/<rate>.
            let rate = decoder.sample_rate();
            if rate != current.rate {
                match sink.set_format(rate) {
                    Ok(()) => current.rate = rate,
                    Err(error) => {
                        current.initial_buffer = false;
                        sink.ring().set_buffering(false);
                        set_decoder_live(shared, false);
                        if generation == shared.generation.load(Ordering::Acquire)
                            && !stop.load(Ordering::Acquire)
                        {
                            fail_at(shared, crate::player::ErrorStage::Sink, error);
                        }
                        return;
                    }
                }
            }
            current.decoder = Some(decoder);
            shared.decoder_live.store(true, Ordering::Release);
            {
                let mut status = shared.lock();
                if !is_reopen {
                    status.loading = !current.paused;
                    status.loaded = true;
                }
                status.duration = current
                    .decoder
                    .as_ref()
                    .map(Decoder::duration)
                    .unwrap_or(0.0);
                status.stream_size = current
                    .decoder
                    .as_ref()
                    .map(|d| d.stream_size().max(0) as u64)
                    .unwrap_or(0);
                status.seekable = seekable;
                status.decoder_live = true;
            }
            if !is_reopen || start_position != position {
                rebase_clock(shared, sink, start_position);
            }
            shared.signal();
        }
        Err(error) => {
            current.initial_buffer = false;
            sink.ring().set_buffering(false);
            set_decoder_live(shared, false);
            if generation == shared.generation.load(Ordering::Acquire)
                && !stop.load(Ordering::Acquire)
            {
                fail_at(shared, crate::player::ErrorStage::Open, error);
            }
        }
    }
}

fn crossfade_remaining(current: &Current) -> Option<f64> {
    let decoder = current.decoder.as_ref()?;
    let duration = decoder.duration();
    if duration <= 0.0 {
        return None;
    }
    let decoded = current.timeline_start + current.written_frames as f64 / f64::from(current.rate);
    Some(duration - decoded)
}

/// Starts an equal-length crossfade when the decoded playhead is within
/// configured crossfade seconds of the current track's end and the preloaded decoder
/// runs at the same pipeline rate.
fn maybe_start_crossfade(current: &mut Current, shared: &Shared) {
    if !shared.crossfade.load(Ordering::Acquire) || current.crossfade.is_some() || current.paused {
        return;
    }
    let Some(decoder) = current.decoder.as_ref() else {
        return;
    };
    let duration = decoder.duration();
    if duration <= 0.0 {
        return;
    }
    let decoded = current.timeline_start + current.written_frames as f64 / f64::from(current.rate);
    let remaining = duration - decoded;
    if shared.bit_perfect.load(Ordering::Acquire)
        || remaining <= 0.0
        || remaining > crossfade_secs(shared)
        || current.preload.is_none()
        || !shared.crossfade_allowed.load(Ordering::Acquire)
    {
        return;
    }
    // Repeat-one preloads the same row; mixing its head into its own tail would
    // skip the first seconds of every repeat, so leave that to gapless.
    if current.next_url.as_deref() == Some(current.url.as_str()) {
        return;
    }
    let some = current.preload.as_ref().map(Decoder::sample_rate);
    if some != Some(current.rate) {
        // A preload opened at its native rate (crossfade toggled after the
        // seed) cannot be mixed into the current stream; this transition
        // falls back to the normal gapless/rate-change path.
        return;
    }
    let Some(next) = current.preload.take() else {
        return;
    };
    let total_frames = (remaining * f64::from(current.rate)).round().max(1.0) as u64;
    let (eq_epoch, eq_profile) = shared.eq_snapshot();
    let next_eq =
        eq_profile.map(|profile| crate::eq::EqRuntime::new(profile, current.rate, ring::CHANNELS));
    current.crossfade = Some(CrossfadeState {
        next,
        next_stop: current.preload_stop.take(),
        next_eq,
        eq_epoch,
        equal_power: shared.crossfade_equal_power.load(Ordering::Acquire),
        played_frames: 0,
        next_frames: 0,
        total_frames,
        next_failed: false,
    });
    current.preload_requested = false;
    current.preload_started = None;
}

/// Recompiles the current decoder's EQ chain when the profile epoch or the
/// pipeline rate changes. The lock is only taken on a real change; the steady
/// state costs one atomic load per chunk.
fn refresh_eq(current: &mut Current, shared: &Shared) {
    if shared.bit_perfect.load(Ordering::Acquire) {
        current.eq = None;
        current.eq_epoch = shared.eq_epoch.load(Ordering::Acquire);
        current.eq_rate = current.rate;
        return;
    }
    let epoch = shared.eq_epoch.load(Ordering::Acquire);
    if current.eq_epoch == epoch && current.eq_rate == current.rate {
        return;
    }
    let profile = shared
        .eq
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    current.eq =
        profile.map(|profile| crate::eq::EqRuntime::new(profile, current.rate, ring::CHANNELS));
    current.eq_epoch = epoch;
    current.eq_rate = current.rate;
}

/// Complementary-gain mix for one PCM block, interpolating per frame across the
/// chunk. Frames past `next_frames` (or any frame when `next_failed`)
/// contribute silence, so a short next track cannot underflow the buffer.
#[expect(
    clippy::too_many_arguments,
    reason = "per-frame crossfade mixing takes the two sample buffers, frame counts, and progress envelope parameters directly"
)]
fn mix_crossfade(
    pcm: &mut [f32],
    next: &[f32],
    frames: usize,
    next_frames: usize,
    next_failed: bool,
    played_frames: u64,
    total_frames: u64,
    equal_power: bool,
) {
    let frames = frames.min(pcm.len() / ring::CHANNELS);
    let total = total_frames.max(1) as f64;
    for frame in 0..frames {
        let progress =
            (played_frames.saturating_add(frame as u64) as f64 / total).clamp(0.0, 1.0) as f32;
        let (gain_out, gain_in) = crate::player::crossfade_gains(progress, equal_power);
        let next_available = !next_failed && frame < next_frames;
        for channel in 0..ring::CHANNELS {
            let index = frame * ring::CHANNELS + channel;
            let current_sample = pcm[index] * gain_out;
            let next_sample = if next_available {
                next[index] * gain_in
            } else {
                0.0
            };
            pcm[index] = current_sample + next_sample;
        }
    }
}

/// Mixes one chunk of the crossfade: current-track tail plus next-track head
/// at complementary linear gains. At the current track's EOF the preloaded
/// decoder becomes live and the boundary carries the already-played offset.
fn run_crossfade_chunk(
    current: &mut Current,
    shared: &Shared,
    sink: &dyn PcmSink,
    pcm: &mut [f32],
    next_pcm: &mut [f32],
) {
    let chunk = chunk_frames(current.rate);
    let cur_frames = {
        let Some(decoder) = current.decoder.as_mut() else {
            current.crossfade = None;
            return;
        };
        match decoder.read(pcm, chunk) {
            Ok(frames) => frames,
            Err(error) => {
                let superseded = current.generation != shared.generation.load(Ordering::Acquire)
                    || current
                        .stop
                        .as_ref()
                        .is_some_and(|stop| stop.load(Ordering::Acquire));
                current.decoder = None;
                current.stop = None;
                current.crossfade = None;
                current.initial_buffer = false;
                sink.ring().set_buffering(false);
                set_decoder_live(shared, false);
                if !superseded {
                    fail_at(shared, crate::player::ErrorStage::Read, error);
                }
                return;
            }
        }
    };
    if cur_frames == 0 {
        // The old track ended: swap decoders and arm the boundary with the
        // number of frames the next track already played under the fade.
        let Some(crossfade) = current.crossfade.take() else {
            return;
        };
        current.eq = crossfade.next_eq;
        current.eq_epoch = crossfade.eq_epoch;
        current.eq_rate = current.rate;
        let offset = crossfade.next_frames as f64 / f64::from(current.rate);
        handoff_to_decoder(
            current,
            crossfade.next,
            crossfade.next_stop,
            offset,
            shared,
            sink,
        );
        return;
    }

    let next_frames = {
        let Some(crossfade) = current.crossfade.as_mut() else {
            return;
        };
        if crossfade.next_failed {
            0
        } else {
            match crossfade.next.read(next_pcm, cur_frames) {
                Ok(frames) => frames,
                Err(error) => {
                    crate::debug_log!("crossfade: next decoder failed: {error}");
                    crossfade.next_failed = true;
                    0
                }
            }
        }
    };

    let mode = Normalization::from_code(shared.normalization.load(Ordering::Acquire) as u8);
    if !shared.bit_perfect.load(Ordering::Acquire)
        && mode.enabled()
        && let Some(decoder) = current.decoder.as_ref()
        && let Some(gain) = normalization_gain_with_peak(
            mode,
            decoder.replaygain_track_gain_db(),
            decoder.replaygain_album_gain_db(),
            decoder.replaygain_track_peak(),
            decoder.replaygain_album_peak(),
        )
    {
        apply_normalization(&mut pcm[..cur_frames * ring::CHANNELS], gain);
    }
    if !shared.bit_perfect.load(Ordering::Acquire) && mode.enabled() && next_frames > 0 {
        let Some(crossfade) = current.crossfade.as_ref() else {
            return;
        };
        if let Some(gain) = normalization_gain_with_peak(
            mode,
            crossfade.next.replaygain_track_gain_db(),
            crossfade.next.replaygain_album_gain_db(),
            crossfade.next.replaygain_track_peak(),
            crossfade.next.replaygain_album_peak(),
        ) {
            apply_normalization(&mut next_pcm[..next_frames * ring::CHANNELS], gain);
        }
    }

    refresh_eq(current, shared);
    if let Some(eq) = current.eq.as_mut() {
        eq.process(&mut pcm[..cur_frames * ring::CHANNELS], cur_frames);
    }
    if let Some(crossfade) = current.crossfade.as_mut() {
        if shared.bit_perfect.load(Ordering::Acquire) {
            crossfade.next_eq = None;
            crossfade.eq_epoch = shared.eq_epoch.load(Ordering::Acquire);
        } else if next_frames > 0 {
            let epoch = shared.eq_epoch.load(Ordering::Acquire);
            if crossfade.eq_epoch != epoch {
                let profile = shared
                    .eq
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                crossfade.next_eq = profile.map(|profile| {
                    crate::eq::EqRuntime::new(profile, current.rate, ring::CHANNELS)
                });
                crossfade.eq_epoch = epoch;
            }
            if let Some(eq) = crossfade.next_eq.as_mut() {
                eq.process(&mut next_pcm[..next_frames * ring::CHANNELS], next_frames);
            }
        }
    }
    let (played, total, next_failed, equal_power) = {
        let Some(crossfade) = current.crossfade.as_ref() else {
            return;
        };
        (
            crossfade.played_frames,
            crossfade.total_frames,
            crossfade.next_failed,
            crossfade.equal_power,
        )
    };
    mix_crossfade(
        pcm,
        next_pcm,
        cur_frames,
        next_frames,
        next_failed,
        played,
        total,
        equal_power,
    );
    if let Some(crossfade) = current.crossfade.as_mut() {
        crossfade.played_frames = crossfade.played_frames.saturating_add(cur_frames as u64);
        crossfade.next_frames = crossfade.next_frames.saturating_add(next_frames as u64);
    }

    current.written_frames += cur_frames as u64;
    let epoch = current.epoch;
    sink.write(&pcm[..cur_frames * ring::CHANNELS], epoch);
    if sink_ring_closed(sink) {
        current.decoder = None;
        current.crossfade = None;
        current.initial_buffer = false;
        set_decoder_live(shared, false);
        cancel_preload(current);
        return;
    }
    if release_buffer_if_ready(current, shared, sink, false) && current.first_pcm {
        current.first_pcm = false;
        shared.first_pcm.store(true, Ordering::Release);
        shared.signal();
        crate::startup::trace("first.pcm.written");
    }
}

/// Releases the ring's buffering hold once enough decoded PCM is queued (or
/// when EOF arrives before the watermark). Returns `true` when playback is
/// actively flowing (not held in buffering).
fn release_buffer_if_ready(
    current: &mut Current,
    shared: &Shared,
    sink: &dyn PcmSink,
    at_eof: bool,
) -> bool {
    if !sink.ring().buffering() {
        return true;
    }
    let buffered = sink.ring().buffered_frames();
    let target = if current.initial_buffer {
        buffer_target_frames(
            PREBUFFER_REMOTE_SECS,
            current.rate,
            sink.ring().usable_frames(),
        )
    } else {
        buffer_target_frames(REBUFFER_SECS, current.rate, sink.ring().usable_frames())
    };
    if !at_eof && buffered < target {
        return false;
    }
    if buffered > 0 {
        sink.request_fade_in();
    }
    current.initial_buffer = false;
    sink.ring().set_buffering(false);
    shared.lock().loading = false;
    if current.first_pcm {
        current.first_pcm = false;
        shared.first_pcm.store(true, Ordering::Release);
        crate::startup::trace("first.pcm.written");
    }
    shared.signal();
    true
}

/// Installs a preloaded decoder as the current one and arms the gapless
/// boundary at the current end of the ring. The caller has already made the
/// pipeline rate match the decoder's own rate.
fn handoff_to_decoder(
    current: &mut Current,
    next: Decoder,
    next_stop: Option<Arc<AtomicBool>>,
    start_position: f64,
    shared: &Shared,
    sink: &dyn PcmSink,
) {
    let duration = next.duration();
    let stream_size = next.stream_size().max(0) as u64;
    let seekable = next.is_seekable();
    current.decoder = Some(next);
    current.epoch = sink.epoch();
    current.written_frames = 0;
    current.first_pcm = false;
    let boundary = sink_ring_written(sink);
    shared.boundary_frame.store(boundary, Ordering::Release);
    shared
        .boundary_epoch
        .store(current.epoch, Ordering::Release);
    *shared
        .boundary_duration
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = duration;
    *shared
        .boundary_origin
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = start_position.max(0.0);
    shared
        .boundary_stream_size
        .store(stream_size, Ordering::Release);
    shared.boundary_active.store(true, Ordering::Release);
    shared.decode_done.store(false, Ordering::Release);
    shared.decoder_live.store(true, Ordering::Release);
    {
        let mut status = shared.lock();
        status.duration = duration;
        status.stream_size = stream_size;
        status.seekable = seekable;
        status.loaded = true;
        status.decoder_live = true;
    }
    crate::startup::trace("gapless.next");
    current.url = current.next_url.take().unwrap_or_default();
    current.timeline_start = start_position.max(0.0);
    current.preload_requested = false;
    current.preload_token = current.preload_token.wrapping_add(1);
    current.preload_started = None;
    // The preloaded decoder was opened with the preload interrupt flag; it is
    // now the live decoder, so adopt that flag for load/seek/stop cancels.
    let stop = next_stop;
    current.stop = stop.clone();
    *shared
        .current_stop
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = stop;
    shared.signal();
}

fn seed_preload(current: &mut Current, shared: &Shared, preload_tx: &Sender<PreloadRequest>) {
    let Some(url) = current.next_url.clone() else {
        return;
    };
    if current.crossfade.is_some()
        || current.preload_requested
        || current.preload.is_some()
        || current.pending_handoff.is_some()
    {
        return;
    }
    let status = shared.lock();
    let duration = status.duration;
    drop(status);
    let written_seconds =
        current.timeline_start + current.written_frames as f64 / f64::from(current.rate.max(1));
    let ready = if duration > 0.0 {
        duration - written_seconds <= 30.0
    } else {
        !current.first_pcm
    };
    if !ready {
        return;
    }
    let token = current.preload_token.wrapping_add(1);
    let stop = Arc::new(AtomicBool::new(false));
    current.preload_requested = true;
    current.preload_token = token;
    current.preload_stop = Some(Arc::clone(&stop));
    current.preload_started = Some(Instant::now());
    let target_rate = shared
        .crossfade
        .load(Ordering::Acquire)
        .then_some(current.rate);
    current.preload_target_rate = target_rate;
    _ = preload_tx.send(PreloadRequest {
        token,
        url,
        stop,
        target_rate,
    });
}

fn drain_preload(current: &mut Current, rx: &Receiver<PreloadResult>) {
    while let Ok(result) = rx.try_recv() {
        apply_preload(current, result);
    }
}

fn apply_preload(current: &mut Current, result: PreloadResult) {
    if result.token != current.preload_token {
        return;
    }
    current.preload_requested = false;
    current.preload_started = None;
    match result.result {
        Ok(decoder) => current.preload = Some(decoder),
        Err(error) => {
            current.preload = None;
            current.next_url = None;
            current.preload_stop = None;
            crate::debug_log!("preload failed: {error}");
        }
    }
}

fn sink_ring_written(sink: &dyn PcmSink) -> u64 {
    sink.ring().written_frames()
}

fn sink_ring_closed(sink: &dyn PcmSink) -> bool {
    sink.ring().closed()
}

fn cancel_preload(current: &mut Current) {
    if let Some(stop) = current.preload_stop.take() {
        stop.store(true, Ordering::Release);
    }
    if let Some(crossfade) = current.crossfade.take()
        && let Some(stop) = crossfade.next_stop
    {
        stop.store(true, Ordering::Release);
    }
    current.preload = None;
    current.pending_handoff = None;
    current.preload_requested = false;
    current.preload_target_rate = None;
    current.preload_token = current.preload_token.wrapping_add(1);
    current.preload_started = None;
}

fn interrupt_current(shared: &Shared) {
    if let Some(stop) = shared
        .current_stop
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        stop.store(true, Ordering::Release);
    }
}

/// Re-anchors the hardware clock so `played_frames` deltas after a ring reset
/// describe the new audio timeline (not the discarded one).
fn rebase_clock(shared: &Shared, sink: &dyn PcmSink, position: f64) {
    *shared
        .clock_origin
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = position.max(0.0);
    shared
        .clock_start
        .store(sink.played_frames(), Ordering::Release);
}

fn interrupt_stop(current: &mut Current) {
    if let Some(stop) = current.stop.take() {
        stop.store(true, Ordering::Release);
    }
}

fn fail_at(shared: &Shared, stage: crate::player::ErrorStage, error: String) {
    fail(shared, crate::player::PlaybackError::classify(stage, error));
}

fn fail(shared: &Shared, error: crate::player::PlaybackError) {
    let mut status = shared.lock();
    status.loading = false;
    status.failed = Some(error);
    drop(status);
    shared.signal();
}

/// Linear gain for a ReplayGain mode. Album prefers the album tag; track
/// prefers the track tag and falls back to the other one. Off/unavailable/
/// malformed gains return `None` (unity).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the ReplayGain tests call the peak-less wrapper"
    )
)]
fn normalization_gain(
    mode: Normalization,
    track_db: Option<f64>,
    album_db: Option<f64>,
) -> Option<f64> {
    normalization_gain_with_peak(mode, track_db, album_db, None, None)
}

/// Linear gain for a ReplayGain mode, capped by `1.0 / peak` when peak metadata
/// is available so positive makeup gain cannot push samples above full scale.
fn normalization_gain_with_peak(
    mode: Normalization,
    track_db: Option<f64>,
    album_db: Option<f64>,
    track_peak: Option<f64>,
    album_peak: Option<f64>,
) -> Option<f64> {
    let (db, peak) = match mode {
        Normalization::Off => return None,
        Normalization::Track => match track_db {
            Some(db) => (db, track_peak.or(album_peak)),
            None => (album_db?, album_peak.or(track_peak)),
        },
        Normalization::Album => match album_db {
            Some(db) => (db, album_peak.or(track_peak)),
            None => (track_db?, track_peak.or(album_peak)),
        },
    };
    if !db.is_finite() {
        return None;
    }
    // ReplayGain tags are usually negative; cap positive makeup to +12 dB so
    // a bad tag cannot blast the output.
    let db = db.min(12.0);
    let mut gain = 10f64.powf(db / 20.0);
    if !gain.is_finite() {
        return None;
    }
    if let Some(peak) = peak
        && peak.is_finite()
        && peak > 0.0
        && gain * peak > 1.0
    {
        gain = 1.0 / peak;
    }
    Some(gain)
}

/// Applies a linear normalization gain to interleaved F32 stereo, clamping to
/// `[-1.0, 1.0]` instead of wrapping or overflowing full scale.
fn apply_normalization(pcm: &mut [f32], gain: f64) {
    if !gain.is_finite() || (gain - 1.0).abs() < 1e-6 {
        return;
    }
    let gain = gain as f32;
    for sample in pcm {
        *sample = (*sample * gain).clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::pipewire::sink::Sink;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn crossfade_mix_keeps_the_old_track_at_the_start_and_new_at_the_end() {
        let mut pcm = [0.25f32; 2 * 4];
        let next = [0.5f32; 2 * 4];
        // Across a 4-frame linear crossfade from frame 0 to frame 3 (`total_frames = 3`):
        // frame 0 is 100% old (0.25), frame 3 is 100% new (0.5), and intermediate
        // frames are smoothly interpolated per frame.
        mix_crossfade(&mut pcm, &next, 4, 4, false, 0, 3, false);
        assert!((pcm[0] - 0.25).abs() < 1e-6);
        assert!((pcm[1] - 0.25).abs() < 1e-6);
        assert!(pcm[2] > 0.25 && pcm[2] < 0.5);
        assert!(pcm[4] > pcm[2] && pcm[4] < 0.5);
        assert!((pcm[6] - 0.5).abs() < 1e-6);
        assert!((pcm[7] - 0.5).abs() < 1e-6);

        // At completion (`played_frames == total_frames`), all frames use the new track.
        let mut pcm = [0.25f32; 2 * 4];
        mix_crossfade(&mut pcm, &next, 4, 4, false, 100, 100, false);
        assert_eq!(pcm, [0.5; 8]);
    }

    #[test]
    fn crossfade_mix_pads_a_short_or_failed_next_stream() {
        let mut pcm = [0.25f32; 2 * 4];
        let next = [0.5f32; 2 * 2];
        // Midpoint (`played_frames = 500_000, total_frames = 1_000_000`, linear):
        // first 2 frames mix both tracks (~0.375), trailing 2 frames fade only the
        // old track (~0.125) because `next` reached EOF at 2 frames.
        mix_crossfade(&mut pcm, &next, 4, 2, false, 500_000, 1_000_000, false);
        assert!((pcm[0] - 0.375).abs() < 1e-4);
        assert!((pcm[2] - 0.375).abs() < 1e-4);
        assert!((pcm[4] - 0.125).abs() < 1e-4, "past EOF only the old tail");
        assert!((pcm[6] - 0.125).abs() < 1e-4, "past EOF only the old tail");

        // A failed next stream contributes silence while the outgoing track still
        // fades out across the chunk (`played_frames = 0, total_frames = 3`).
        let mut pcm = [0.25f32; 2 * 4];
        mix_crossfade(&mut pcm, &next, 4, 2, true, 0, 3, false);
        assert!((pcm[0] - 0.25).abs() < 1e-6);
        assert!(pcm[2] < 0.25 && pcm[2] > 0.0);
        assert!(
            (pcm[6] - 0.0).abs() < 1e-6,
            "outgoing track fades to silence"
        );
    }

    #[test]
    fn replaygain_modes_choose_the_right_tag_and_clamp() {
        assert_eq!(
            normalization_gain(Normalization::Off, Some(-6.0), Some(-8.0)),
            None
        );
        let track = normalization_gain(Normalization::Track, Some(-6.0), Some(-8.0)).unwrap();
        assert!((track - 10f64.powf(-6.0 / 20.0)).abs() < 1e-9);
        let album = normalization_gain(Normalization::Track, None, Some(-8.0)).unwrap();
        assert!((album - 10f64.powf(-8.0 / 20.0)).abs() < 1e-9);
        // Album mode prefers album, and falls back to the track tag.
        assert_eq!(
            normalization_gain(Normalization::Album, None, Some(-8.0)).unwrap(),
            10f64.powf(-8.0 / 20.0)
        );
        assert!(normalization_gain(Normalization::Album, Some(-6.0), None).is_some());
        // Positive makeup is capped at +12 dB; garbage tags are ignored.
        assert!(normalization_gain(Normalization::Track, Some(60.0), None).unwrap() < 4.0);
        assert_eq!(
            normalization_gain(Normalization::Track, Some(f64::NAN), None),
            None
        );
    }

    #[test]
    fn normalization_scales_and_clips_samples() {
        let mut pcm = [0.1f32, -0.1, 0.9, -0.9];
        apply_normalization(&mut pcm, 2.0);
        assert_eq!(pcm, [0.2, -0.2, 1.0, -1.0]);
        let mut unharmed = [0.5f32; 4];
        apply_normalization(&mut unharmed, 1.0);
        assert_eq!(unharmed, [0.5; 4]);
    }

    #[test]
    fn normalization_respects_peak_and_avoids_clipping() {
        // +6 dB (~1.995x) on a track with peak 0.8 would clip at ~1.596, so
        // peak limiting caps the linear gain to 1.0 / 0.8 = 1.25.
        let gain =
            normalization_gain_with_peak(Normalization::Track, Some(6.0), None, Some(0.8), None)
                .unwrap();
        assert!((gain - 1.25).abs() < 1e-9, "unexpected capped gain: {gain}");

        // When gain * peak <= 1.0, the dB gain is applied untouched.
        let untouched =
            normalization_gain_with_peak(Normalization::Track, Some(-6.0), None, Some(0.95), None)
                .unwrap();
        assert!((untouched - 10f64.powf(-6.0 / 20.0)).abs() < 1e-9);

        let mut pcm = [0.8f32, -0.8, 0.4, -0.4];
        apply_normalization(&mut pcm, gain);
        assert_eq!(pcm, [1.0, -1.0, 0.5, -0.5]);
    }

    #[test]
    fn seed_preload_includes_timeline_start() {
        let shared = Shared::new();
        shared.lock().duration = 200.0;
        let (preload_tx, preload_rx) = mpsc::channel();

        // Near the start of a 200 s track (`timeline_start = 0.0`), 480 frames
        // at 48 kHz (0.01 s) is nowhere near the 30 s preload threshold.
        let mut current = Current {
            rate: 48_000,
            timeline_start: 0.0,
            written_frames: 480,
            next_url: Some("file:///tmp/next.wav".to_string()),
            ..Current::default()
        };
        seed_preload(&mut current, &shared, &preload_tx);
        assert!(!current.preload_requested);
        assert!(preload_rx.try_recv().is_err());

        // After seeking to 180.0 s on a 200.0 s track, only 20 s remain (<= 30 s),
        // so `seed_preload` triggers immediately even when `written_frames` is small.
        current.timeline_start = 180.0;
        seed_preload(&mut current, &shared, &preload_tx);
        assert!(current.preload_requested);
        let request = preload_rx
            .try_recv()
            .expect("preload request should be sent when timeline_start is within 30s of end");
        assert_eq!(request.url, "file:///tmp/next.wav");
    }

    struct WavFile {
        path: PathBuf,
    }

    impl WavFile {
        fn url(&self) -> String {
            format!("file://{}", self.path.display())
        }
    }

    impl Drop for WavFile {
        fn drop(&mut self) {
            if let Some(dir) = self.path.parent() {
                _ = fs::remove_dir_all(dir);
            }
        }
    }

    fn write_wav(path: &Path, sample_rate: u32, seconds: f64) {
        let frames = (f64::from(sample_rate) * seconds) as usize;
        let data_len = u32::try_from(frames * 4).expect("test WAV too large");
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // linear PCM
        bytes.extend_from_slice(&2u16.to_le_bytes()); // stereo
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(44 + data_len as usize, 0);
        fs::write(path, bytes).expect("write test WAV");
    }

    fn wav_fixture(sample_rate: u32, seconds: f64) -> WavFile {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "musika-worker-{}-{}-{}",
            std::process::id(),
            serial,
            nanos
        ));
        fs::create_dir_all(&dir).expect("create test temp dir");
        let path = dir.join("fixture.wav");
        write_wav(&path, sample_rate, seconds);
        WavFile { path }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if predicate() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        predicate()
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn rate_change_handoff_drains_then_renegotiates() {
        let first = wav_fixture(44_100, 0.4);
        let second = wav_fixture(48_000, 0.4);
        let shared = Arc::new(Shared::new());
        let sink = Arc::new(Sink::new(Arc::clone(&shared)).expect("PipeWire sink"));
        let worker = Worker::new(Arc::clone(&shared), sink.clone() as Arc<dyn PcmSink>)
            .expect("decode worker");
        assert_eq!(sink.rate(), ring::DEFAULT_RATE);
        worker.load(first.url(), Some(second.url()), false, 1);

        // First track: the worker adopts 44.1 kHz and renegotiates before PCM.
        assert!(
            wait_until(|| sink.rate() == 44_100, Duration::from_secs(15)),
            "first track did not adopt its native rate"
        );
        let failed = shared.lock().failed.clone();
        assert!(failed.is_none(), "first load failed: {failed:?}");

        // Second track: the queued 44.1 kHz frames are drained first, then the
        // stream is rebuilt at the new rate and the gapless boundary armed. The
        // ring rate flips before the worker arms the boundary (the sink applies
        // the format first), so wait for the boundary, not just the rate.
        assert!(
            wait_until(
                || shared.boundary_active.load(Ordering::Acquire),
                Duration::from_secs(15)
            ),
            "second track never armed the gapless boundary (rate {})",
            sink.rate()
        );
        assert_eq!(
            sink.rate(),
            48_000,
            "the stream was not renegotiated to the second track's rate"
        );
        let failed = shared.lock().failed.clone();
        assert!(failed.is_none(), "rate-change handoff failed: {failed:?}");

        let boundary = shared.boundary_frame.load(Ordering::Acquire);
        assert!(
            wait_until(|| sink.read_frames() >= boundary, Duration::from_secs(10)),
            "playback never reached the armed boundary ({boundary})"
        );
        assert!(
            !sink.failed(),
            "sink reported a stream error during the handoff"
        );
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn same_rate_handoff_reuses_the_stream() {
        let first = wav_fixture(44_100, 0.4);
        let second = wav_fixture(44_100, 0.4);
        let shared = Arc::new(Shared::new());
        let sink = Arc::new(Sink::new(Arc::clone(&shared)).expect("PipeWire sink"));
        let worker = Worker::new(Arc::clone(&shared), sink.clone() as Arc<dyn PcmSink>)
            .expect("decode worker");
        worker.load(first.url(), Some(second.url()), false, 1);

        assert!(
            wait_until(|| sink.rate() == 44_100, Duration::from_secs(15)),
            "first track did not adopt its native rate"
        );
        assert!(
            wait_until(
                || shared.boundary_active.load(Ordering::Acquire),
                Duration::from_secs(15)
            ),
            "same-rate track never armed the gapless boundary"
        );
        // A same-rate handoff must not touch the stream format.
        assert_eq!(
            sink.rate(),
            44_100,
            "same-rate handoff renegotiated the stream"
        );
        let failed = shared.lock().failed.clone();
        assert!(failed.is_none(), "same-rate handoff failed: {failed:?}");
        let boundary = shared.boundary_frame.load(Ordering::Acquire);
        assert!(
            wait_until(|| sink.read_frames() >= boundary, Duration::from_secs(10)),
            "playback never reached the same-rate boundary"
        );
        assert!(
            !sink.failed(),
            "sink reported a stream error during the handoff"
        );
    }

    #[test]
    fn idle_transport_blocks_instead_of_polling() {
        // A freshly started or stopped transport has no decoder and nothing
        // pending, so the worker may sleep on its command channel.
        let current = Current::default();
        assert!(!needs_timed_wait(&current));

        // A paused decoder is woken by Resume/Seek, not by time.
        let current = Current {
            paused: true,
            ..Current::default()
        };
        assert!(!needs_timed_wait(&current));

        // In-flight preload work advances on its own even without a command.
        let current = Current {
            preload_requested: true,
            ..Current::default()
        };
        assert!(needs_timed_wait(&current));
    }

    #[test]
    fn pause_staleness_uses_the_reopen_threshold() {
        let now = Instant::now();
        assert!(!pause_is_stale(None, now));
        assert!(!pause_is_stale(Some(now - Duration::from_secs(5)), now));
        assert!(!pause_is_stale(Some(now - PAUSE_REOPEN_AFTER), now));
        assert!(pause_is_stale(
            Some(now - PAUSE_REOPEN_AFTER - Duration::from_secs(1)),
            now
        ));
    }

    #[test]
    fn seek_plan_restarts_non_seekable_streams_instead_of_failing() {
        // A cold/fresh decoder starts at 0 without a seek call.
        assert_eq!(seek_plan(0.0, true), SeekPlan::Start(0.0));
        assert_eq!(seek_plan(0.0, false), SeekPlan::Start(0.0));
        // Seekable streams seek as requested.
        assert_eq!(seek_plan(42.0, true), SeekPlan::Seek(42.0));
        // Live/transcoded streams cannot jump: restart instead of failing the
        // always-retryable Seek stage (the old pause/resume stuck path).
        assert_eq!(seek_plan(42.0, false), SeekPlan::RestartFromZero);
        // Negative targets clamp to the start.
        assert_eq!(seek_plan(-3.0, true), SeekPlan::Start(0.0));
    }

    #[test]
    fn buffer_watermark_holds_until_target_or_eof() {
        struct StubSink {
            ring: Arc<ring::Ring>,
            fade_ins: std::sync::atomic::AtomicUsize,
        }
        impl PcmSink for StubSink {
            fn ring(&self) -> &Arc<ring::Ring> {
                &self.ring
            }
            fn epoch(&self) -> u64 {
                self.ring.epoch()
            }
            fn rate(&self) -> u32 {
                self.ring.rate()
            }
            fn set_format(&self, _rate: u32) -> Result<(), String> {
                Ok(())
            }
            fn set_target(&self, _target: Option<String>) -> Result<(), String> {
                Ok(())
            }
            fn set_exclusive(&self, _exclusive: bool) -> Result<(), String> {
                Ok(())
            }
            fn write(&self, pcm: &[f32], epoch: u64) {
                self.ring.write(pcm, epoch);
            }
            fn set_paused(&self, paused: bool) {
                self.ring.set_paused(paused);
            }
            fn request_fade_in(&self) {
                self.fade_ins.fetch_add(1, Ordering::Relaxed);
                self.ring.request_fade(true);
            }
            fn request_fade_out(&self) {
                self.ring.request_fade(false);
            }
            fn set_muted(&self, muted: bool) {
                self.ring.set_muted(muted);
            }
            fn set_volume(&self, volume: f64) {
                self.ring.set_volume(volume);
            }
            fn flush(&self) {
                self.ring.flush();
            }
            fn reset(&self) {
                self.ring.reset();
            }
            fn close_writes(&self) {
                self.ring.close_writes();
            }
            fn played_frames(&self) -> u64 {
                self.ring.played_frames()
            }
            fn read_frames(&self) -> u64 {
                self.ring.read_frames()
            }
            fn buffered_frames(&self) -> u64 {
                self.ring.buffered_frames()
            }
            fn underruns(&self) -> u64 {
                self.ring.underruns()
            }
            fn failed(&self) -> bool {
                self.ring.failed()
            }
        }

        let ring = Arc::new(ring::Ring::new().expect("ring"));
        let sink = StubSink {
            ring: Arc::clone(&ring),
            fade_ins: std::sync::atomic::AtomicUsize::new(0),
        };
        let shared = Shared::new();
        let rate = ring.rate();
        let epoch = ring.epoch();

        // 1) Initial remote prebuffer holds until PREBUFFER_REMOTE_SECS (1.0 s).
        let mut current = Current {
            rate,
            first_pcm: true,
            initial_buffer: true,
            ..Current::default()
        };
        ring.set_buffering(true);
        shared.lock().loading = true;
        let half_sec = vec![0.1f32; (rate as usize / 2) * ring::CHANNELS];
        ring.write(&half_sec, epoch);
        assert!(!release_buffer_if_ready(
            &mut current,
            &shared,
            &sink,
            false
        ));
        assert!(ring.buffering());
        assert!(!shared.first_pcm.load(Ordering::Acquire));

        ring.write(&half_sec, epoch);
        assert!(release_buffer_if_ready(&mut current, &shared, &sink, false));
        assert!(!ring.buffering());
        assert!(!current.initial_buffer);
        assert!(shared.first_pcm.load(Ordering::Acquire));
        assert!(!shared.lock().loading);
        assert_eq!(sink.fade_ins.load(Ordering::Relaxed), 1);

        // 2) Mid-track underrun rebuffer holds until REBUFFER_SECS (2.0 s).
        ring.flush();
        let mut drain = vec![0.0f32; 16 * ring::CHANNELS];
        _ = ring.read_into(&mut drain, 0);
        let epoch = ring.epoch();
        ring.set_buffering(true);
        let one_sec = vec![0.1f32; (rate as usize) * ring::CHANNELS];
        ring.write(&one_sec, epoch);
        assert!(
            !release_buffer_if_ready(&mut current, &shared, &sink, false),
            "mid-track rebuffer must wait past 1.0 s for REBUFFER_SECS (2.0 s)"
        );
        ring.write(&one_sec, epoch);
        assert!(release_buffer_if_ready(&mut current, &shared, &sink, false));
        assert!(!ring.buffering());

        // 3) EOF releases a short tail even below the watermark.
        ring.flush();
        _ = ring.read_into(&mut drain, 0);
        let epoch = ring.epoch();
        ring.set_buffering(true);
        let tiny = vec![0.1f32; 256 * ring::CHANNELS];
        ring.write(&tiny, epoch);
        assert!(release_buffer_if_ready(&mut current, &shared, &sink, true));
        assert!(!ring.buffering());
    }
}
