//! Long-lived decode and preload workers.
//!
//! One worker owns the current [`Decoder`]; a second persistent worker opens
//! the next track ahead of the boundary. No thread is created per track and
//! the old `malloc_trim`/reaper churn is gone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::decode::Decoder;
use super::ring::{RATE, RING_FRAMES};
use super::sink::Sink;
use super::status::{Shared, Status};

pub(crate) enum Command {
    Load {
        url: String,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    },
    Pause,
    Resume {
        url: String,
        next_url: Option<String>,
        generation: u64,
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
    paused_request: std::sync::Mutex<Option<(String, Option<String>, u64)>>,
    paused_position: std::sync::Mutex<f64>,
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
            paused_request: std::sync::Mutex::new(None),
            paused_position: std::sync::Mutex::new(0.0),
        })
    }

    pub(crate) fn load(&self, url: String, next_url: Option<String>, paused: bool) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self
            .paused_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((url.clone(), next_url.clone(), generation));
        {
            let mut status = self.shared.lock();
            status.loading = true;
            status.ended = false;
            status.failed = None;
            status.position = 0.0;
            status.duration = 0.0;
            status.stream_size = 0;
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
        if paused {
            *self
                .paused_position
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = position.max(0.0);
            interrupt_current(&self.shared);
            self.sink.reset();
            self.sink.set_paused(true);
            let _ = tx.send(Command::Pause);
        } else {
            self.sink.set_paused(false);
            self.shared.lock().loading = true;
            if let Some((url, next_url, generation)) = self
                .paused_request
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                let position = *self
                    .paused_position
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let _ = tx.send(Command::Resume {
                    url,
                    next_url,
                    generation,
                    position,
                });
            }
        }
    }

    pub(crate) fn seek(&self, seconds: f64) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        self.shared
            .clock_start
            .store(self.sink.played_frames(), Ordering::Release);
        *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = seconds.max(0.0);
        self.shared.lock().loading = true;
        self.sink.flush();
        let _ = tx.send(Command::Seek(seconds.max(0.0)));
    }

    pub(crate) fn stop(&self) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.boundary_active.store(false, Ordering::Release);
        self.shared.decode_done.store(false, Ordering::Release);
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
            // Dropping the receiver would leave a blocked open; send a
            // shutdown sentinel by dropping the sender and waking the worker
            // via its own receiver close is enough because it only blocks when
            // no request is active.
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
}

fn worker_thread(
    rx: Receiver<Command>,
    preload_rx: Receiver<PreloadResult>,
    preload_tx: Sender<PreloadRequest>,
    shared: Arc<Shared>,
    sink: Arc<Sink>,
) {
    let mut current = Current::default();
    let mut pcm = vec![0i16; 4096 * 2];
    loop {
        // A blocking wait is used only when there is no decoder to service;
        // otherwise commands are polled between every decoded chunk.
        let command = if current.decoder.is_some() {
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
                    continue;
                }
                Command::Pause => {
                    cancel_preload(&mut current);
                    interrupt_stop(&mut current);
                    current.decoder = None;
                    current.stop = None;
                    continue;
                }
                Command::Resume {
                    url,
                    next_url,
                    generation,
                    position,
                } => {
                    start_decoder(
                        &mut current,
                        url,
                        next_url,
                        generation,
                        position,
                        false,
                        &shared,
                        &sink,
                    );
                    continue;
                }
                Command::Seek(seconds) => {
                    cancel_preload(&mut current);
                    if let Some(decoder) = current.decoder.as_mut() {
                        match decoder.seek(seconds) {
                            Ok(()) => {
                                current.written_frames = 0;
                                current.timeline_start = seconds.max(0.0);
                                current.parked_url = None;
                                current.first_pcm = true;
                                shared
                                    .clock_start
                                    .store(sink.played_frames(), Ordering::Release);
                                *shared
                                    .clock_origin
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                    seconds.max(0.0);
                                shared.decode_done.store(false, Ordering::Release);
                                shared.lock().loading = true;
                            }
                            Err(error) => {
                                fail(&shared, error);
                                current.decoder = None;
                            }
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
                    sink.set_paused(paused);
                    continue;
                }
            }
        }

        let Some(decoder) = current.decoder.as_mut() else {
            if let Some(url) = current.parked_url.clone()
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
                fail(&shared, error);
                current.decoder = None;
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
                crate::startup::trace("gapless.next");
                current.url = current.next_url.take().unwrap_or_default();
                current.timeline_start = 0.0;
                current.preload_requested = false;
                current.preload_token = current.preload_token.wrapping_add(1);
                current.preload_stop = None;
                current.preload_started = None;
                continue;
            }
            crate::startup::trace("fetch.done");
            shared.decode_done.store(true, Ordering::Release);
            current.decoder = None;
            continue;
        }

        current.written_frames += frames as u64;
        if current.first_pcm {
            current.first_pcm = false;
            crate::startup::trace("first.pcm.written");
        }
        let epoch = current.epoch;
        sink.write(&pcm[..frames * 2], epoch);
        if sink_ring_closed(&sink) {
            // Load/stop replaced the epoch while the writer was blocked.
            current.decoder = None;
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
    shared.decode_done.store(false, Ordering::Release);
    let stop = Arc::new(AtomicBool::new(false));
    *shared
        .current_stop
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&stop));
    current.stop = Some(stop.clone());
    match Decoder::open(&url, stop) {
        Ok(mut decoder) => {
            if position > 0.0
                && let Err(error) = decoder.seek(position)
            {
                fail(shared, error);
                return;
            }
            crate::startup::trace("fetch.first_chunk");
            current.decoder = Some(decoder);
            let mut status = shared.lock();
            status.loading = true;
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
            drop(status);
            *shared
                .clock_origin
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = position;
            shared
                .clock_start
                .store(sink.played_frames(), Ordering::Release);
        }
        Err(error) => {
            if !matches!(error.as_str(), "cancelled" | "stopped") {
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
}
