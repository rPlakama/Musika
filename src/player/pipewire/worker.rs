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

use super::decode::Decoder;
use super::ring::{RATE, RING_FRAMES};
use super::sink::Sink;
use super::status::{Shared, Status};

/// A pause longer than this reopens the stream instead of resuming the decoder
/// in memory: a long-idle HTTP response is not reliable.
const PAUSE_REOPEN_AFTER: Duration = Duration::from_secs(30);

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
        interrupt_current(&self.shared);
        self.sink.reset();
        self.sink.set_paused(paused);
        let _ = tx.send(Command::Load {
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
            let _ = tx.send(Command::Pause);
        } else {
            self.sink.set_paused(false);
            let position = *self
                .paused_position
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = tx.send(Command::Resume { position });
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
        self.sink.flush();
        let _ = tx.send(Command::Seek(seconds));
    }

    pub(crate) fn stop(&self) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
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
        self.sink.reset();
        self.sink.close_writes();
        self.sink.set_paused(true);
        let _ = tx.send(Command::Stop);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        interrupt_current(&self.shared);
        self.sink.reset();
        self.sink.close_writes();
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Command::Shutdown);
        }
        if let Some(preload_tx) = self.preload_tx.take() {
            // Dropping the sender lets the preload thread observe the closed
            // channel and exit; a blocked open is interrupted by `stop`.
            drop(preload_tx);
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        if let Some(join) = self.preload_join.take() {
            let _ = join.join();
        }
    }
}

fn preload_thread(rx: Receiver<PreloadRequest>, results: Sender<PreloadResult>) {
    while let Ok(request) = rx.recv() {
        if request.stop.load(Ordering::Acquire) {
            let _ = results.send(PreloadResult {
                token: request.token,
                result: Err("preload cancelled".to_string()),
            });
            continue;
        }
        let result = Decoder::open(&request.url, Arc::clone(&request.stop));
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
    parked_url: Option<String>,
    next_url: Option<String>,
    preload_requested: bool,
    preload_token: u64,
    preload: Option<Decoder>,
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
        ..Current::default()
    };
    let mut pcm = vec![0i16; 4096 * 2];
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
        } else {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
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
                            command => {
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
                    current.paused = paused;
                    current.paused_at = None;
                    sink.set_paused(paused);
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

        let Some(decoder) = current.decoder.as_mut() else {
            if let Some(url) = current.parked_url.clone()
                && !sink.ring().paused()
                && sink.ring().buffered_frames() <= (RATE as u64 * 2)
            {
                let target = current.timeline_start + current.written_frames as f64 / RATE as f64;
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

        let frames = match decoder.read(&mut pcm, 1024) {
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
                current.decoder = Some(next);
                current.written_frames = 0;
                current.first_pcm = true;
                let duration = current
                    .decoder
                    .as_ref()
                    .map(Decoder::duration)
                    .unwrap_or(0.0);
                let stream_size = current
                    .decoder
                    .as_ref()
                    .map(|decoder| decoder.stream_size().max(0) as u64)
                    .unwrap_or(0);
                let boundary = sink_ring_written(&sink);
                shared.boundary_frame.store(boundary, Ordering::Release);
                *shared
                    .boundary_duration
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = duration;
                shared
                    .boundary_stream_size
                    .store(stream_size, Ordering::Release);
                shared.boundary_active.store(true, Ordering::Release);
                shared.decode_done.store(false, Ordering::Release);
                {
                    let mut status = shared.lock();
                    status.duration = duration;
                    status.stream_size = stream_size;
                    status.seekable = current.decoder.as_ref().is_some_and(Decoder::is_seekable);
                    status.loaded = true;
                    status.decoder_live = true;
                }
                crate::startup::trace("gapless.next");
                current.url = current.next_url.take().unwrap_or_default();
                current.timeline_start = 0.0;
                current.preload_requested = false;
                current.preload_token = current.preload_token.wrapping_add(1);
                current.preload_stop = None;
                current.preload_started = None;
                shared.signal();
                continue;
            }
            crate::startup::trace("fetch.done");
            shared.decode_done.store(true, Ordering::Release);
            shared.lock().decoder_live = false;
            shared.signal();
            current.decoder = None;
            continue;
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
        // PCM. Reopen before it runs low, seeking to the decoded boundary.
        if sink.ring().buffered_frames() >= RING_FRAMES as u64 * 7 / 8
            && current.decoder.as_ref().is_some_and(Decoder::is_seekable)
        {
            current.parked_url = Some(current.url.clone());
            current.decoder = None;
            current.stop = None;
            shared.lock().decoder_live = false;
            continue;
        }

        seed_preload(&mut current, &shared, &sink, &preload_tx);
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

fn seed_preload(
    current: &mut Current,
    shared: &Shared,
    sink: &Sink,
    preload_tx: &Sender<PreloadRequest>,
) {
    let Some(url) = current.next_url.clone() else {
        return;
    };
    if current.preload_requested || current.preload.is_some() {
        return;
    }
    let status = shared.lock();
    let duration = status.duration;
    drop(status);
    let written_seconds = current.written_frames as f64 / RATE as f64;
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
    let _ = preload_tx.send(PreloadRequest { token, url, stop });
    let _ = sink;
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
    current.preload = None;
    current.preload_requested = false;
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
