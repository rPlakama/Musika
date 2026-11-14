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
use super::sink::Sink;
use super::status::{Shared, Status};
use crate::player::Normalization;

/// A pause longer than this reopens the stream instead of resuming the decoder
/// in memory: a long-idle HTTP response is not reliable.
const PAUSE_REOPEN_AFTER: Duration = Duration::from_secs(30);

/// Optional equal-length crossfade between the outgoing and incoming track,
/// when enabled from the UI. The fade starts when the decoded position is this
/// close to the end; the next decoder is preloaded at the current pipeline rate
/// so both streams can be mixed without a graph rebuild.
const CROSSFADE_SECS: f64 = 5.0;

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

pub(crate) enum Command {
    Load {
        url: String,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    },
    Pause,
    Resume {
        position: f64,
    },
    Seek(f64),
    Stop,
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
    sink: Arc<Sink>,
    paused_position: std::sync::Mutex<f64>,
    paused: AtomicBool,
}

impl Worker {
    pub(crate) fn new(shared: Arc<Shared>, sink: Arc<Sink>) -> Result<Self, String> {
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
        if was_playing && !paused {
            // A manual track change cuts the old stream; fade it out before the
            // reset instead of clicking. The decoded position is already stale
            // for the new row, so this only smooths the transition.
            self.sink.request_fade_out();
            std::thread::sleep(Duration::from_millis(20));
        }
        interrupt_current(&self.shared);
        self.sink.reset();
        self.sink.set_paused(paused);
        _ = tx.send(Command::Load {
            url,
            next_url,
            paused,
            generation,
        });
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
            self.sink.set_paused(false);
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
        // A seek while paused must not leave the engine reporting `loading`
        // forever: nothing decodes until the worker is resumed.
        self.shared.lock().loading = !self.paused.load(Ordering::Acquire);
        if !self.paused.load(Ordering::Acquire) {
            // Let the old audio fade out before the seek discards the queue;
            // the new position fades in below. This is deliberately short and
            // runs on the decode-worker thread, never the RT thread.
            self.sink.request_fade_out();
            std::thread::sleep(Duration::from_millis(25));
        }
        self.sink.flush();
        // A seek cuts the queued old audio; fade the new position's PCM in
        // instead of starting it with a hard discontinuity.
        if !self.paused.load(Ordering::Acquire) {
            self.sink.request_fade_in();
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
            std::thread::sleep(Duration::from_millis(20));
        }
        self.sink.reset();
        self.sink.close_writes();
        self.sink.set_paused(true);
        _ = tx.send(Command::Stop);
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
    parked_url: Option<String>,
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
    stop: Option<Arc<AtomicBool>>,
    /// The decoder is kept open but not read: pause with a seekable input.
    paused: bool,
    paused_at: Option<Instant>,
    /// The transport was stopped: a stale seek/resume must not reopen audio.
    stopped: bool,
}

struct CrossfadeState {
    next: Decoder,
    next_stop: Option<Arc<AtomicBool>>,
    /// Frames of `next` consumed by the fade so far.
    next_frames: u64,
    /// Total frames of the fade (the remaining duration when it started).
    total_frames: u64,
    /// Reading the next decoder failed; keep mixing silence and let the
    /// boundary/advance path handle the error.
    next_failed: bool,
}

/// Whether the decode loop has time-based work that a command cannot wake it
/// for. Keep this in step with the bottom of the loop: parked HTTP reopen,
/// pending rate handoff, preload and crossfade all advance on their own.
fn needs_timed_wait(current: &Current) -> bool {
    // A paused decoder produces no PCM and is woken by Resume/Seek, so it
    // blocks too; only work that advances without a command keeps polling.
    current.parked_url.is_some()
        || current.pending_handoff.is_some()
        || current.preload.is_some()
        || current.preload_requested
        || current.crossfade.is_some()
}

fn worker_thread(
    rx: Receiver<Command>,
    preload_rx: Receiver<PreloadResult>,
    preload_tx: Sender<PreloadRequest>,
    shared: Arc<Shared>,
    sink: Arc<Sink>,
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
                Command::Stop => {
                    cancel_preload(&mut current);
                    interrupt_stop(&mut current);
                    current.decoder = None;
                    current.stop = None;
                    current.parked_url = None;
                    current.paused = false;
                    current.paused_at = None;
                    current.stopped = true;
                    shared.lock().decoder_live = false;
                    continue;
                }
                Command::Pause => {
                    cancel_preload(&mut current);
                    // A parked or live seekable decoder is kept: the ring keeps
                    // its buffered audio and resume continues from it.
                    let keep = current.parked_url.is_some()
                        || current.decoder.as_ref().is_some_and(Decoder::is_seekable);
                    if keep {
                        current.paused = true;
                        current.paused_at = Some(Instant::now());
                        shared.lock().loading = false;
                    } else {
                        interrupt_stop(&mut current);
                        current.decoder = None;
                        current.stop = None;
                        current.parked_url = None;
                        current.paused = false;
                        current.paused_at = None;
                        sink.reset();
                        let mut status = shared.lock();
                        status.loading = false;
                        status.decoder_live = false;
                        status.seekable = false;
                    }
                    continue;
                }
                Command::Resume { position } => {
                    let resume_at = position.max(0.0);
                    let long_idle = current
                        .paused_at
                        .is_some_and(|at| at.elapsed() > PAUSE_REOPEN_AFTER);
                    current.paused = false;
                    current.paused_at = None;
                    if current.decoder.is_some() {
                        if long_idle {
                            // The stream sat idle longer than an HTTP response
                            // can be trusted: reopen and seek to the playhead.
                            let (url, next_url, generation) = (
                                current.url.clone(),
                                current.next_url.clone(),
                                current.generation,
                            );
                            cancel_preload(&mut current);
                            interrupt_stop(&mut current);
                            current.decoder = None;
                            current.parked_url = None;
                            sink.reset();
                            start_decoder(
                                &mut current,
                                url,
                                next_url,
                                generation,
                                resume_at,
                                false,
                                &shared,
                                &sink,
                            );
                        } else {
                            // Live decoder: keep it, do not re-seek.
                            shared.lock().loading = false;
                        }
                        continue;
                    }
                    if current.parked_url.is_some() {
                        // The parked reopen path resumes automatically as soon
                        // as the ring drains; it seeks to the decoded boundary.
                        shared.lock().loading = false;
                        continue;
                    }
                    if current.stopped {
                        continue;
                    }
                    // Interrupted at pause (non-seekable): reopen at the
                    // paused position.
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
                        resume_at,
                        false,
                        &shared,
                        &sink,
                    );
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
                            | Command::Pause
                            | Command::Resume { .. }
                            | Command::Stop
                            | Command::Shutdown => {
                                pending_command = Some(command);
                                break;
                            }
                        }
                    }
                    let seconds = seconds.max(0.0);
                    cancel_preload(&mut current);
                    if let Some(decoder) = current.decoder.as_mut() {
                        match decoder.seek(seconds) {
                            Ok(()) => {
                                current.written_frames = 0;
                                current.timeline_start = seconds;
                                current.parked_url = None;
                                current.first_pcm = true;
                                // The request path already published the
                                // newest clock target. Do not write this
                                // command's older target here: a later seek
                                // may have arrived while decoder.seek ran.
                                shared.decode_done.store(false, Ordering::Release);
                                let paused = current.paused;
                                shared.lock().loading = !paused;
                            }
                            Err(error) => {
                                fail(&shared, error);
                                current.decoder = None;
                                shared.lock().decoder_live = false;
                            }
                        }
                    } else if !current.stopped {
                        // No live decoder (parked or interrupted by a pause):
                        // reopen at the seek target so the position is honoured.
                        current.parked_url = None;
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
                            &shared,
                            &sink,
                        );
                        if current.paused {
                            shared.lock().loading = false;
                        }
                    }
                    continue;
                }
                Command::Load {
                    url,
                    next_url,
                    paused,
                    generation,
                } => {
                    // A burst of skips can enqueue several loads while a
                    // previous HTTP open is being interrupted. Only the
                    // newest generation should reach Navidrome/FFmpeg.
                    if generation != shared.generation.load(Ordering::Acquire) {
                        continue;
                    }
                    // Apply the pause/active state before opening: a paused
                    // restore then leaves the PipeWire node suspended for the
                    // whole (possibly slow) network open instead of burning
                    // graph wakeups until `start_decoder` returns.
                    current.paused = paused;
                    current.paused_at = None;
                    sink.set_paused(paused);
                    start_decoder(
                        &mut current,
                        url,
                        next_url,
                        generation,
                        0.0,
                        false,
                        &shared,
                        &sink,
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
            let mismatched_pending = current.preload_requested
                && current.preload_target_rate != Some(current.rate);
            if mismatched_ready || mismatched_pending {
                cancel_preload(&mut current);
            }
        }
        if current.crossfade.is_none()
            && shared.crossfade.load(Ordering::Acquire)
            && current.preload.is_none()
            && current.preload_requested
            && let Some(remaining) = crossfade_remaining(&current)
            && remaining <= CROSSFADE_SECS
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
            run_crossfade_chunk(&mut current, &shared, &sink, &mut pcm, &mut next_pcm);
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
                            fail(&shared, error);
                            shared.lock().decoder_live = false;
                            continue;
                        }
                    }
                }
                let next_stop = current.preload_stop.take();
                handoff_to_decoder(&mut current, next, next_stop, 0.0, &shared, &sink);
                continue;
            }
            if let Some(url) = current.parked_url.clone()
                && !sink.ring().paused()
                && sink.ring().buffered_frames() <= current.rate as u64 * 2
            {
                let target = current.timeline_start
                    + current.written_frames as f64 / f64::from(current.rate);
                let next_url = current.next_url.clone();
                let generation = current.generation;
                start_decoder(
                    &mut current,
                    url,
                    next_url,
                    generation,
                    target,
                    true,
                    &shared,
                    &sink,
                );
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
                shared.lock().decoder_live = false;
                if !superseded {
                    fail(&shared, error);
                }
                continue;
            }
        };
        if frames == 0 {
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
                    shared.lock().decoder_live = false;
                    continue;
                }
                let next_stop = current.preload_stop.take();
                handoff_to_decoder(&mut current, next, next_stop, 0.0, &shared, &sink);
                continue;
            }
            crate::startup::trace("fetch.done");
            shared.decode_done.store(true, Ordering::Release);
            shared.lock().decoder_live = false;
            shared.signal();
            current.decoder = None;
            continue;
        }

        let mode = Normalization::from_code(
            shared.normalization.load(Ordering::Acquire) as u8,
        );
        if mode.enabled()
            && let Some(decoder) = current.decoder.as_ref()
            && let Some(gain) = normalization_gain(
                mode,
                decoder.replaygain_track_gain_db(),
                decoder.replaygain_album_gain_db(),
            )
        {
            apply_normalization(&mut pcm[..frames * ring::CHANNELS], gain);
        }

        current.written_frames += frames as u64;
        if current.first_pcm {
            current.first_pcm = false;
            shared.first_pcm.store(true, Ordering::Release);
            shared.signal();
            crate::startup::trace("first.pcm.written");
        }
        let epoch = current.epoch;
        sink.write(&pcm[..frames * 2], epoch);
        if sink_ring_closed(&sink) {
            // Load/stop replaced the epoch while the writer was blocked.
            current.decoder = None;
            shared.lock().decoder_live = false;
            cancel_preload(&mut current);
            continue;
        }

        // The ring is the playback buffer. Once it is nearly full, close the
        // current AVIO/HTTP response and let playback consume the buffered
        // PCM. The threshold is 7/8 of the rate-scaled usable capacity (~9.5 s
        // of audio at any pipeline rate). Reopen before it runs low, seeking to
        // the decoded boundary.
        if current.crossfade.is_none()
            && sink.ring().buffered_frames() >= sink.ring().usable_frames() * 7 / 8
            && current.decoder.as_ref().is_some_and(Decoder::is_seekable)
        {
            current.parked_url = Some(current.url.clone());
            current.decoder = None;
            current.stop = None;
            shared.lock().decoder_live = false;
            continue;
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
    shared: &Shared,
    sink: &Sink,
) {
    if !preserve_preload {
        cancel_preload(current);
        current.preload = None;
        current.preload_requested = false;
        current.preload_token = 0;
        current.preload_started = None;
        current.next_url = next_url.clone();
    }
    interrupt_stop(current);
    current.decoder = None;
    current.url = url.clone();
    current.epoch = sink.epoch();
    current.generation = generation;
    current.written_frames = 0;
    current.timeline_start = position;
    current.parked_url = None;
    current.pending_handoff = None;
    if !preserve_preload {
        current.next_url = next_url;
    }
    current.first_pcm = true;
    current.stopped = false;
    shared.decode_done.store(false, Ordering::Release);
    let stop = Arc::new(AtomicBool::new(false));
    *shared
        .current_stop
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&stop));
    current.stop = Some(stop.clone());
    match Decoder::open(&url, Arc::clone(&stop)) {
        Ok(mut decoder) => {
            if position > 0.0
                && let Err(error) = decoder.seek(position)
            {
                if generation == shared.generation.load(Ordering::Acquire)
                    && !stop.load(Ordering::Acquire)
                {
                    fail(shared, error);
                }
                shared.lock().decoder_live = false;
                return;
            }
            crate::startup::trace("fetch.first_chunk");
            // Adopt the track's native rate before any PCM is written for it:
            // the sink rebuilds its PipeWire stream to F32LE/stereo/<rate>.
            let rate = decoder.sample_rate();
            if rate != current.rate {
                match sink.set_format(rate) {
                    Ok(()) => current.rate = rate,
                    Err(error) => {
                        shared.lock().decoder_live = false;
                        if generation == shared.generation.load(Ordering::Acquire)
                            && !stop.load(Ordering::Acquire)
                        {
                            fail(shared, error);
                        }
                        return;
                    }
                }
            }
            let seekable = decoder.is_seekable();
            current.decoder = Some(decoder);
            {
                let mut status = shared.lock();
                status.loading = !current.paused;
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
                status.loaded = true;
            }
            *shared
                .clock_origin
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = position;
            shared
                .clock_start
                .store(sink.played_frames(), Ordering::Release);
            shared.signal();
        }
        Err(error) => {
            shared.lock().decoder_live = false;
            if generation == shared.generation.load(Ordering::Acquire)
                && !stop.load(Ordering::Acquire)
            {
                fail(shared, error);
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
/// [`CROSSFADE_SECS`] of the current track's end and the preloaded decoder
/// runs at the same pipeline rate.
fn maybe_start_crossfade(current: &mut Current, shared: &Shared) {
    if !shared.crossfade.load(Ordering::Acquire)
        || current.crossfade.is_some()
        || current.paused
    {
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
    if remaining <= 0.0 || remaining > CROSSFADE_SECS || current.preload.is_none() {
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
    current.crossfade = Some(CrossfadeState {
        next,
        next_stop: current.preload_stop.take(),
        next_frames: 0,
        total_frames,
        next_failed: false,
    });
    current.preload_requested = false;
    current.preload_started = None;
}

/// Complementary-gain mix for one PCM block. Frames past `next_frames` (or
/// any frame when `next_failed`) contribute silence, so a short next track
/// cannot underflow the buffer.
fn mix_crossfade(
    pcm: &mut [f32],
    next: &[f32],
    frames: usize,
    next_frames: usize,
    next_failed: bool,
    gain_out: f32,
    gain_in: f32,
) {
    let frames = frames.min(pcm.len() / ring::CHANNELS);
    for frame in 0..frames {
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
    sink: &Sink,
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
                shared.lock().decoder_live = false;
                if !superseded {
                    fail(shared, error);
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
    if mode.enabled()
        && let Some(decoder) = current.decoder.as_ref()
        && let Some(gain) = normalization_gain(
            mode,
            decoder.replaygain_track_gain_db(),
            decoder.replaygain_album_gain_db(),
        )
    {
        apply_normalization(&mut pcm[..cur_frames * ring::CHANNELS], gain);
    }
    if mode.enabled() && next_frames > 0 {
        let Some(crossfade) = current.crossfade.as_ref() else {
            return;
        };
        if let Some(gain) = normalization_gain(
            mode,
            crossfade.next.replaygain_track_gain_db(),
            crossfade.next.replaygain_album_gain_db(),
        ) {
            apply_normalization(&mut next_pcm[..next_frames * ring::CHANNELS], gain);
        }
    }

    let (played, total, next_failed) = {
        let Some(crossfade) = current.crossfade.as_ref() else {
            return;
        };
        (
            crossfade.next_frames,
            crossfade.total_frames,
            crossfade.next_failed,
        )
    };
    let progress = (played as f64 / total.max(1) as f64).clamp(0.0, 1.0) as f32;
    mix_crossfade(
        pcm,
        next_pcm,
        cur_frames,
        next_frames,
        next_failed,
        1.0 - progress,
        progress,
    );
    if let Some(crossfade) = current.crossfade.as_mut() {
        crossfade.next_frames = crossfade.next_frames.saturating_add(next_frames as u64);
    }

    current.written_frames += cur_frames as u64;
    if current.first_pcm {
        current.first_pcm = false;
        shared.first_pcm.store(true, Ordering::Release);
        shared.signal();
        crate::startup::trace("first.pcm.written");
    }
    let epoch = current.epoch;
    sink.write(&pcm[..cur_frames * ring::CHANNELS], epoch);
    if sink_ring_closed(sink) {
        current.decoder = None;
        current.crossfade = None;
        shared.lock().decoder_live = false;
        cancel_preload(current);
    }
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
    sink: &Sink,
) {
    let duration = next.duration();
    let stream_size = next.stream_size().max(0) as u64;
    let seekable = next.is_seekable();
    current.decoder = Some(next);
    current.written_frames = 0;
    current.first_pcm = true;
    let boundary = sink_ring_written(sink);
    shared.boundary_frame.store(boundary, Ordering::Release);
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
    let written_seconds = current.written_frames as f64 / f64::from(current.rate);
    let ready = if duration > 0.0 {
        duration - written_seconds <= 30.0
    } else {
        !current.first_pcm
    };
    if !ready {
        return;
    }
    let token = current.generation;
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

fn sink_ring_written(sink: &Sink) -> u64 {
    sink.ring().written_frames()
}

fn sink_ring_closed(sink: &Sink) -> bool {
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

fn interrupt_stop(current: &mut Current) {
    if let Some(stop) = current.stop.take() {
        stop.store(true, Ordering::Release);
    }
}

fn fail(shared: &Shared, message: String) {
    let mut status = shared.lock();
    status.loading = false;
    status.failed = Some(message);
    drop(status);
    shared.signal();
}

/// Linear gain for a ReplayGain mode. Album prefers the album tag; track
/// prefers the track tag and falls back to the other one. Off/unavailable/
/// malformed gains return `None` (unity).
fn normalization_gain(
    mode: Normalization,
    track_db: Option<f64>,
    album_db: Option<f64>,
) -> Option<f64> {
    let db = match mode {
        Normalization::Off => return None,
        Normalization::Track => track_db.or(album_db),
        Normalization::Album => album_db.or(track_db),
    }?;
    if !db.is_finite() {
        return None;
    }
    // ReplayGain tags are usually negative; cap positive makeup to +12 dB so
    // a bad tag cannot blast the output.
    let db = db.min(12.0);
    (10f64.powf(db / 20.0)).is_finite().then(|| 10f64.powf(db / 20.0))
}

/// Applies a linear normalization gain to interleaved F32 stereo.
/// instead of wrapping.
fn apply_normalization(pcm: &mut [f32], gain: f64) {
    if !gain.is_finite() || (gain - 1.0).abs() < 1e-6 {
        return;
    }
    let gain = gain as f32;
    for sample in pcm {
        *sample *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU64;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn crossfade_mix_keeps_the_old_track_at_the_start_and_new_at_the_end() {
        let mut pcm = [0.25f32; 2 * 4];
        let next = [0.5f32; 2 * 4];
        mix_crossfade(&mut pcm, &next, 4, 4, false, 1.0, 0.0);
        assert_eq!(pcm, [0.25; 8]);
        let mut pcm = [0.25f32; 2 * 4];
        mix_crossfade(&mut pcm, &next, 4, 4, false, 0.0, 1.0);
        assert_eq!(pcm, [0.5; 8]);
        let mut pcm = [0.25f32; 2 * 4];
        mix_crossfade(&mut pcm, &next, 4, 4, false, 0.5, 0.5);
        assert_eq!(pcm, [0.375; 8]);
    }

    #[test]
    fn crossfade_mix_pads_a_short_or_failed_next_stream() {
        let mut pcm = [0.25f32; 2 * 4];
        let next = [0.5f32; 2 * 2];
        mix_crossfade(&mut pcm, &next, 4, 2, false, 0.5, 0.5);
        assert_eq!(&pcm[..4], &[0.375; 4]);
        assert_eq!(&pcm[4..], &[0.125; 4], "past EOF only the old tail");
        let mut pcm = [0.25f32; 2 * 4];
        mix_crossfade(&mut pcm, &next, 4, 2, true, 1.0, 1.0);
        assert_eq!(pcm, [0.25; 8], "a failed next stream is silent");
    }

    #[test]
    fn replaygain_modes_choose_the_right_tag_and_clamp() {
        assert_eq!(normalization_gain(Normalization::Off, Some(-6.0), Some(-8.0)), None);
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
        assert_eq!(normalization_gain(Normalization::Track, Some(f64::NAN), None), None);
    }

    #[test]
    fn normalization_scales_and_clips_samples() {
        let mut pcm = [0.1f32, -0.1, 0.9, -0.9];
        apply_normalization(&mut pcm, 2.0);
        assert_eq!(pcm, [0.2, -0.2, 1.8, -1.8]);
        let mut unharmed = [0.5f32; 4];
        apply_normalization(&mut unharmed, 1.0);
        assert_eq!(unharmed, [0.5; 4]);
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
        let worker = Worker::new(Arc::clone(&shared), Arc::clone(&sink)).expect("decode worker");
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
            wait_until(|| sink.played_frames() >= boundary, Duration::from_secs(10)),
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
        let worker = Worker::new(Arc::clone(&shared), Arc::clone(&sink)).expect("decode worker");
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
            wait_until(|| sink.played_frames() >= boundary, Duration::from_secs(10)),
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
        // parked, so the worker may sleep on its command channel.
        let current = Current::default();
        assert!(!needs_timed_wait(&current));

        // A paused decoder is woken by Resume/Seek, not by time.
        let current = Current {
            paused: true,
            ..Current::default()
        };
        assert!(!needs_timed_wait(&current));

        // Every field below advances on its own even without a command.
        let current = Current {
            parked_url: Some("https://example.invalid/track".to_string()),
            ..Current::default()
        };
        assert!(needs_timed_wait(&current));

        let current = Current {
            preload_requested: true,
            ..Current::default()
        };
        assert!(needs_timed_wait(&current));
    }
}
