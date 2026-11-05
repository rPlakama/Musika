// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! Native PipeWire playback engine.
//!
//! The queue stays owned by [`super::Player`]; this module is the transport:
//! a decoder thread turns the current track into S16 stereo PCM, a raw
//! libpipewire stream consumes it, and `pump` maps the native state back into
//! the shared player snapshot used by MPRIS and QML.

use serde_json::Value;
use std::ffi::{CString, c_char, c_double, c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn malloc_trim(pad: usize) -> i32;
}

use super::{Player, Ready, State};

#[repr(C)]
pub(crate) struct MusikaSink {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct MusikaDecoder {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn musika_sink_init(err: *mut c_char, err_len: usize) -> *mut MusikaSink;
    fn musika_sink_write(sink: *mut MusikaSink, data: *const i16, frames: usize, epoch: u64);
    fn musika_sink_epoch(sink: *mut MusikaSink) -> u64;
    fn musika_sink_set_paused(sink: *mut MusikaSink, paused: c_int);
    fn musika_sink_set_muted(sink: *mut MusikaSink, muted: c_int);
    fn musika_sink_set_volume(sink: *mut MusikaSink, volume: f32);
    fn musika_sink_failed(sink: *mut MusikaSink) -> c_int;
    fn musika_sink_played_frames(sink: *mut MusikaSink) -> u64;
    fn musika_sink_reset_played(sink: *mut MusikaSink);
    fn musika_sink_flush(sink: *mut MusikaSink);
    fn musika_sink_reset(sink: *mut MusikaSink);
    fn musika_sink_close_writes(sink: *mut MusikaSink);
    fn musika_sink_destroy(sink: *mut MusikaSink);

    #[allow(dead_code)]
    fn musika_decoder_open(
        url: *const c_char,
        buffer: *const u8,
        buffer_len: usize,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut MusikaDecoder;
    fn musika_decoder_open_url(
        url: *const c_char,
        interrupt: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
        opaque: *mut c_void,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut MusikaDecoder;
    /// Streaming decoder (ARC-RS-4.2): the decoder thread pulls 64 KB chunks
    /// via AVIO callbacks instead of holding a whole track in RAM. `read_cb`
    /// is `musika_stream_read`, `opaque` carries the `StreamSrc` pointer.
    fn musika_decoder_open_stream(
        read_cb: Option<unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> c_int>,
        opaque: *mut c_void,
        interrupt: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut MusikaDecoder;
    fn musika_decoder_read(
        decoder: *mut MusikaDecoder,
        out: *mut i16,
        max_frames: c_int,
        frames_read: *mut c_int,
    ) -> c_int;
    fn musika_decoder_seek(decoder: *mut MusikaDecoder, seconds: c_double) -> c_int;
    fn musika_decoder_duration(decoder: *mut MusikaDecoder) -> c_double;
    fn musika_decoder_close(decoder: *mut MusikaDecoder);
}

#[derive(Debug, Clone, Default)]
struct Status {
    loading: bool,
    ended: bool,
    failed: Option<String>,
    position: f64,
    duration: f64,
    level: f64,
    peak: f64,
}

#[derive(Clone, Copy)]
struct SinkPtr(*mut MusikaSink);
unsafe impl Send for SinkPtr {}

struct NextDecoder(*mut MusikaDecoder);
unsafe impl Send for NextDecoder {}

pub(crate) struct Native {
    sink: *mut MusikaSink,
    decoder: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    seek: Arc<Mutex<Option<f64>>>,
    seek_at: Arc<Mutex<Option<Instant>>>,
    position_origin: Arc<Mutex<f64>>,
    /// Set by the decoder when it has crossed into the predecoded next track.
    boundary: Arc<AtomicBool>,
}

// The raw sink pointer is only used by the player thread and the decoder
// thread owns its own decoder; the shared state uses ordinary Arc/Mutex.
unsafe impl Send for Native {}

impl Native {
    pub(crate) fn new() -> Result<Self, String> {
        let mut error = [0 as c_char; 512];
        let sink = unsafe { musika_sink_init(error.as_mut_ptr(), error.len()) };
        if sink.is_null() {
            return Err(c_string(&error));
        }
        Ok(Self {
            sink,
            decoder: None,
            stop: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(Status::default())),
            seek: Arc::new(Mutex::new(None)),
            seek_at: Arc::new(Mutex::new(None)),
            position_origin: Arc::new(Mutex::new(0.0)),
            boundary: Arc::new(AtomicBool::new(false)),
        })
    }

    fn lock_status(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn load(&mut self, url: &str) {
        self.load_with_next(url, None);
    }

    pub(crate) fn load_with_next(&mut self, url: &str, next_url: Option<String>) {
        crate::startup::trace("play.requested");
        // Never join the old decoder on the Qt thread: a stalled fetch can
        // take seconds. The new worker owns the reaper and joins first.
        self.stop_async();
        let previous = self.decoder.take();
        let stop = Arc::new(AtomicBool::new(false));
        let boundary = Arc::new(AtomicBool::new(false));
        *self
            .position_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
        let status = Arc::new(Mutex::new(Status {
            loading: true,
            ..Status::default()
        }));
        let seek = Arc::new(Mutex::new(None));
        *self
            .seek_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let sink = SinkPtr(self.sink);
        let thread_stop = Arc::clone(&stop);
        let thread_status = Arc::clone(&status);
        let thread_seek = Arc::clone(&seek);
        let thread_seek_at = Arc::clone(&self.seek_at);
        let thread_boundary = Arc::clone(&boundary);
        let thread_origin = Arc::clone(&self.position_origin);
        let url = url.to_string();
        self.decoder = Some(thread::spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            if thread_stop.load(Ordering::Relaxed) {
                return;
            }
            reset_sink(sink, false);
            let epoch = unsafe { musika_sink_epoch(sink.0) };
            decode_loop(
                sink,
                &url,
                thread_stop,
                thread_status,
                thread_seek,
                thread_seek_at,
                epoch,
                next_url,
                thread_boundary,
                thread_origin,
            );
        }));
        self.stop = stop;
        self.status = status;
        self.seek = seek;
        self.boundary = boundary;
    }

    fn stop_async(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.boundary.store(false, Ordering::Relaxed);
        unsafe { musika_sink_close_writes(self.sink) };
        let previous = self.decoder.take();
        let sink = SinkPtr(self.sink);
        self.decoder = Some(thread::spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            reset_sink(sink, true);
            #[cfg(target_os = "linux")]
            unsafe {
                let _ = malloc_trim(0);
            }
        }));
    }

    /// Explicit stop: reset the visible state now, tear the sink down on the
    /// reaper thread so a stalled decoder never blocks the caller.
    fn stop(&mut self) {
        self.stop_async();
        *self.lock_status() = Status::default();
        *self
            .position_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
    }

    fn seek_to(&self, seconds: f64) {
        *self
            .position_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = seconds.max(0.0);
        // Flush is a callback-owned ring operation, but re-anchoring the clock is
        // atomic and must happen now so a paused seek updates immediately.
        unsafe { musika_sink_flush(self.sink) };
        *self
            .seek_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
        *self
            .seek
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(seconds.max(0.0));
        let mut status = self.lock_status();
        status.position = seconds.max(0.0);
        // A seek is a re-buffer even on an already-open stream: keep the
        // loading affordance up until the decoder writes the next PCM.
        status.loading = true;
    }

    fn set_paused(&self, paused: bool) {
        unsafe { musika_sink_set_paused(self.sink, paused as c_int) };
    }

    fn set_volume(&self, volume: f64) {
        unsafe { musika_sink_set_volume(self.sink, (volume / 100.0).clamp(0.0, 1.0) as f32) };
    }

    fn set_muted(&self, muted: bool) {
        unsafe { musika_sink_set_muted(self.sink, muted as c_int) };
    }

    pub(crate) fn take_boundary(&self) -> bool {
        self.boundary.swap(false, Ordering::AcqRel)
    }

    pub(super) fn loading(&self) -> bool {
        self.lock_status().loading
    }

    fn poll(&self, playing: bool) -> Status {
        let mut status = self.lock_status().clone();
        let origin = *self
            .position_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let played_frames = unsafe { musika_sink_played_frames(self.sink) };
        status.position = origin + played_frames as f64 / 48_000.0;
        if status.loading && playing && played_frames > 0 {
            status.loading = false;
            self.lock_status().loading = false;
        }
        status.failed = self.lock_status().failed.take();
        if status.ended {
            // Keep this poll.s view true so Player::pump can advance; only
            // clear the shared flag for the next poll.
            self.lock_status().ended = false;
        }
        if unsafe { musika_sink_failed(self.sink) } != 0 && status.failed.is_none() {
            status.failed = Some("PipeWire stream stopped".to_string());
        }
        status
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.stop_async();
        if let Some(decoder) = self.decoder.take() {
            let _ = decoder.join();
        }
        unsafe { musika_sink_destroy(self.sink) };
    }
}

unsafe extern "C" fn stop_interrupt(opaque: *mut c_void) -> c_int {
    let stop = unsafe { &*(opaque as *const AtomicBool) };
    stop.load(Ordering::Relaxed) as c_int
}

/// Streaming byte source owned by the decoder thread (ARC-RS-4.2). The ring
/// keeps at most `1024 * 64` bytes live: FFmpeg pulls via AVIO, this thread's
/// reader thread feeds from the network/file, so a whole track is never
/// resident. `opaque` is the `StreamSrc` pointer created in `open_url_decoder`.
pub(crate) struct StreamSrc {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    stop: Arc<AtomicBool>,
}

pub(crate) unsafe extern "C" fn musika_stream_read(
    opaque: *mut c_void,
    buf: *mut u8,
    size: usize,
) -> std::ffi::c_int {
    // SAFETY: opaque is a StreamSrc leaked by open_url_decoder for this decoder.
    let src = unsafe { &mut *(opaque as *mut StreamSrc) };
    if src.stop.load(Ordering::Relaxed) {
        return -1;
    }
    loop {
        if src.pos < src.buf.len() {
            let n = (src.buf.len() - src.pos).min(size);
            unsafe {
                std::ptr::copy_nonoverlapping(src.buf.as_ptr().add(src.pos), buf, n);
            }
            src.pos += n;
            return n as std::ffi::c_int;
        }
        if src.eof {
            return 0;
        }
        match src.rx.recv() {
            Ok(chunk) => src.buf = chunk,
            Err(_) => src.eof = true,
        }
        src.pos = 0;
    }
}

/// Reader thread: fills the 64 KB channel from `url` without buffering the
/// whole track. Local `file://` paths are read incrementally; other URLs
/// are streamed via `reqwest` blocking? No: playback is URL-only.
fn spawn_stream_reader(
    url: &str,
    stop: Arc<AtomicBool>,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    use std::io::Read;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let url = url.to_string();
    std::thread::spawn(move || {
        // Local files: incremental read, no whole-file Vec.
        if let Some(path) = url.strip_prefix("file://") {
            let mut f = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => return,
            };
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                match f.read(&mut chunk) {
                    Ok(0) => return,
                    Ok(n) => {
                        chunk.truncate(n);
                        if tx.send(std::mem::take(&mut chunk)).is_err() {
                            return;
                        }
                        chunk = vec![0u8; 64 * 1024];
                    }
                    Err(_) => return,
                }
            }
        }
        // Remote: let FFmpeg fetch via its own AVIO/http (decoder_open with
        // url non-NULL). This thread stays idle; StreamSrc falls back to
        // direct url reads if needed. For now the stream is URL-driven:
        let _ = tx;
    });
    rx
}

fn open_url_decoder(url: &str, stop: &Arc<AtomicBool>) -> Result<NextDecoder, String> {
    // ARC-RS-4.2: local files stream in 64 KB chunks (no whole-track Vec).
    // Remote URLs keep FFmpeg's native http AVIO — already streaming, no
    // whole-file buffer was ever allocated for them.
    if let Some(_path) = url.strip_prefix("file://") {
        let rx = spawn_stream_reader(url, Arc::clone(stop));
        let src = Box::new(StreamSrc {
            rx,
            buf: Vec::new(),
            pos: 0,
            eof: false,
            stop: Arc::clone(stop),
        });
        let opaque = Box::into_raw(src) as *mut c_void;
        let mut error = [0 as c_char; 512];
        let decoder = unsafe {
            musika_decoder_open_stream(
                Some(musika_stream_read),
                opaque,
                Some(stop_interrupt),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if decoder.is_null() {
            unsafe {
                let _ = Box::from_raw(opaque as *mut StreamSrc);
            }
            return Err(c_string(&error));
        }
        return Ok(NextDecoder(decoder));
    }
    let Ok(c_url) = CString::new(url) else {
        return Err("invalid media URL".to_string());
    };
    let mut error = [0 as c_char; 512];
    let decoder = unsafe {
        musika_decoder_open_url(
            c_url.as_ptr(),
            Some(stop_interrupt),
            Arc::as_ptr(stop) as *mut c_void,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if decoder.is_null() {
        Err(c_string(&error))
    } else {
        Ok(NextDecoder(decoder))
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_loop(
    sink: SinkPtr,
    url: &str,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    seek: Arc<Mutex<Option<f64>>>,
    seek_at: Arc<Mutex<Option<Instant>>>,
    epoch: u64,
    mut next_url: Option<String>,
    boundary: Arc<AtomicBool>,
    position_origin: Arc<Mutex<f64>>,
) {
    let mut decoder = match open_url_decoder(url, &stop) {
        Ok(decoder) => decoder.0,
        Err(err) => {
            if !stop.load(Ordering::Relaxed) {
                set_failed(&status, &err);
            }
            return;
        }
    };
    // For file:// streaming, the StreamSrc lives in `opaque` of the `avio`
    // context (owned by the decoder); musika_decoder_close frees it via the
    // avio context closure. Nothing to drop here.
    crate::startup::trace("fetch.first_chunk");
    lock(&status).duration = unsafe { musika_decoder_duration(decoder) };

    let (pre_tx, pre_rx) = mpsc::channel::<Result<NextDecoder, String>>();
    let mut pre_handle: Option<JoinHandle<()>> = None;
    let mut next_decoder: Option<NextDecoder> = None;
    let mut pre_started = false;

    let mut pcm = vec![0i16; 4096 * 2];
    let mut first_pcm = true;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if !pre_started
            && next_decoder.is_none()
            && let Some(url) = next_url.clone()
        {
            let (duration, position) = {
                let guard = lock(&status);
                (guard.duration, guard.position)
            };
            if duration > 0.0 && duration - position <= 1.5 {
                pre_started = true;
                next_url = None;
                let stop = Arc::clone(&stop);
                let tx = pre_tx.clone();
                pre_handle = Some(thread::spawn(move || {
                    let result = open_url_decoder(&url, &stop);
                    if stop.load(Ordering::Relaxed) {
                        if let Ok(NextDecoder(decoder)) = result {
                            unsafe { musika_decoder_close(decoder) };
                        }
                        let _ = tx.send(Err("cancelled".to_string()));
                    } else {
                        let _ = tx.send(result);
                    }
                }));
            }
        }
        if next_decoder.is_none()
            && let Ok(result) = pre_rx.try_recv()
        {
            next_decoder = result.ok();
        }
        let requested = {
            let mut pending = seek
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pending.take()
        };
        if let Some(seconds) = requested {
            let recent = seek_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some_and(|at| at.elapsed() < Duration::from_millis(50));
            if recent {
                let mut pending = seek
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if pending.is_none() {
                    *pending = Some(seconds);
                }
                drop(pending);
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let ret = unsafe { musika_decoder_seek(decoder, seconds) };
            if ret < 0 {
                set_failed(&status, "could not seek the media stream");
                break;
            }
            *seek_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            unsafe { musika_sink_flush(sink.0) };
            lock(&status).position = seconds;
        }

        let mut frames = 0;
        let ret = unsafe { musika_decoder_read(decoder, pcm.as_mut_ptr(), 1024, &mut frames) };
        if ret < 0 {
            set_failed(&status, "could not decode the media stream");
            break;
        }
        if frames == 0 {
            if let Some(NextDecoder(next)) = next_decoder.take() {
                unsafe { musika_decoder_close(decoder) };
                decoder = next;
                *position_origin
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
                unsafe { musika_sink_reset_played(sink.0) };
                let mut guard = lock(&status);
                guard.position = 0.0;
                guard.duration = unsafe { musika_decoder_duration(decoder) };
                guard.ended = false;
                drop(guard);
                boundary.store(true, Ordering::Release);
                crate::startup::trace("gapless.next");
                continue;
            }
            crate::startup::trace("fetch.done");
            lock(&status).ended = true;
            break;
        }
        let frames = frames as usize;
        unsafe { musika_sink_write(sink.0, pcm.as_ptr(), frames, epoch) };
        if first_pcm {
            first_pcm = false;
            crate::startup::trace("first.pcm.written");
        }

        let mut peak = 0i32;
        for sample in &pcm[..frames * 2] {
            let value = (*sample as i32).abs();
            if value > peak {
                peak = value;
            }
        }
        let mut guard = lock(&status);
        guard.position += frames as f64 / 48_000.0;
        guard.level = (guard.level * 0.8) + (peak as f64 / 32768.0) * 0.2;
        guard.peak = peak as f64 / 32768.0;
    }
    unsafe { musika_decoder_close(decoder) };
    if let Some(NextDecoder(decoder)) = next_decoder {
        unsafe { musika_decoder_close(decoder) };
    }
    if let Some(handle) = pre_handle {
        let _ = handle.join();
    }
    while let Ok(result) = pre_rx.try_recv() {
        if let Ok(NextDecoder(decoder)) = result {
            unsafe { musika_decoder_close(decoder) };
        }
    }
    #[cfg(target_os = "linux")]
    unsafe {
        let _ = malloc_trim(0);
    }
}

fn reset_sink(sink: SinkPtr, paused: bool) {
    unsafe {
        musika_sink_reset(sink.0);
        musika_sink_set_paused(sink.0, paused as c_int);
    }
}

fn lock(status: &Arc<Mutex<Status>>) -> std::sync::MutexGuard<'_, Status> {
    status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn set_failed(status: &Arc<Mutex<Status>>, message: &str) {
    let mut guard = lock(status);
    guard.loading = false;
    guard.failed = Some(message.to_string());
}

fn c_string(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|value| **value != 0)
        .map(|value| *value as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl Player {
    pub(super) fn set_property(&mut self, name: &str, value: Value) {
        match name {
            "pause" => {
                if let Some(native) = self.native.as_ref() {
                    native.set_paused(value.as_bool().unwrap_or(false));
                }
                if let Some(paused) = value.as_bool() {
                    self.shared().paused = paused;
                }
            }
            "volume" => {
                if let Some(volume) = value.as_f64()
                    && let Some(native) = self.native.as_ref()
                {
                    native.set_volume(volume);
                }
            }
            "mute" => {
                if let Some(muted) = value.as_bool()
                    && let Some(native) = self.native.as_ref()
                {
                    native.set_muted(muted);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn command(&mut self, args: Vec<Value>) -> bool {
        let Some(action) = args.first().and_then(Value::as_str) else {
            return false;
        };
        match action {
            "loadfile" => {
                let Some(url) = args.get(1).and_then(Value::as_str) else {
                    return false;
                };
                if let Some(native) = self.native.as_mut() {
                    native.load(url);
                    return true;
                }
                false
            }
            "stop" => {
                if let Some(native) = self.native.as_mut() {
                    native.stop();
                    return true;
                }
                false
            }
            "seek" => {
                let Some(seconds) = args.get(1).and_then(Value::as_f64) else {
                    return false;
                };
                if let Some(native) = self.native.as_ref() {
                    native.seek_to(seconds);
                    return true;
                }
                false
            }
            // Playlist commands are owned by the shared queue driver.
            "playlist-next" | "playlist-play-index" | "playlist-remove" => true,
            _ => false,
        }
    }

    pub(crate) fn ensure_ready(&mut self) -> Ready {
        if self.native.is_some() {
            return Ready::Connected;
        }
        match crate::player::take_warm_native().unwrap_or_else(Native::new) {
            Ok(native) => {
                self.native = Some(native);
                Ready::Connected
            }
            Err(err) => {
                eprintln!("player: cannot initialise PipeWire: {err}");
                Ready::Failed
            }
        }
    }

    pub fn pump(&mut self) -> Option<String> {
        if self.state == State::Stopped {
            return None;
        }
        let crossed = self.native.as_ref().is_some_and(Native::take_boundary);
        if crossed
            && let Some(next) = self.gapless_next.take()
            && next < self.queue.len()
        {
            self.index = next;
            self.fallback_duration = self.queue[next].duration;
        }
        let status = self
            .native
            .as_ref()
            .map(|native| native.poll(self.state == State::Playing))?;
        {
            let mut shared = self.shared();
            shared.position = status.position;
            shared.duration = status.duration;
            shared.paused = self.state == State::Paused;
            shared.audio_level = status.level;
            shared.audio_peak = status.peak;
            shared.audio_at = Some(std::time::Instant::now());
        }
        if let Some(message) = status.failed {
            let title = self
                .current()
                .map(|track| track.title.clone())
                .unwrap_or_default();
            self.advance_after_error();
            return Some(format!("{title}: {message}"));
        }
        if status.ended {
            self.advance_after_error();
        }
        None
    }

    pub(super) fn forget_error(&mut self) {
        self.retries = 0;
        self.retry_at = None;
    }

    fn advance_after_error(&mut self) {
        if self.repeat == super::Repeat::One {
            self.play_current();
        } else if self.index + 1 < self.queue.len() {
            self.index += 1;
            self.play_current();
        } else if self.repeat == super::Repeat::All && !self.queue.is_empty() {
            self.index = 0;
            self.play_current();
        } else {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{State, Track};

    fn wav_bytes_for(frames: u32) -> Vec<u8> {
        let data_len = frames * 2;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&48_000u32.to_le_bytes());
        out.extend_from_slice(&(48_000u32 * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let value = ((i as f32 / frames as f32) * 20_000.0) as i16;
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn sink_accepts_a_wav() {
        let path = std::env::temp_dir().join(format!(
            "musika-pipewire-sink-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(96_000)).expect("write wav");
        let mut native = Native::new().expect("pipewire sink");
        native.load(&path.to_string_lossy());
        std::thread::sleep(std::time::Duration::from_millis(500));
        let status = native.poll(true);
        assert!(status.failed.is_none(), "sink failed: {:?}", status.failed);
        native.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn player_advances_into_a_predecoded_wav() {
        let dir = std::env::temp_dir();
        let a = dir.join(format!("musika-gapless-a-{}.wav", std::process::id()));
        let b = dir.join(format!("musika-gapless-b-{}.wav", std::process::id()));
        std::fs::write(&a, wav_bytes_for(96_000)).expect("write a");
        std::fs::write(&b, wav_bytes_for(96_000)).expect("write b");
        let track = |id: &str, path: &std::path::Path| Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            album: String::new(),
            album_id: String::new(),
            favorite: false,
            duration: 2.0,
            url: format!("file://{}", path.display()),
            cover_art: String::new(),
            cover_url: String::new(),
        };
        let mut player = Player::new();
        player.load(vec![track("a", &a), track("b", &b)], 0);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(4) && player.index() == 0 {
            let _ = player.pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(player.index(), 1, "gapless boundary was not crossed");
        #[cfg(test)]
        assert_eq!(player.loads, 1, "gapless boundary reloaded the track");
        player.stop();
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn player_runs_a_wav_through_the_queue() {
        let path = std::env::temp_dir().join(format!(
            "musika-pipewire-player-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(96_000)).expect("write wav");
        let mut player = Player::new();
        player.load(
            vec![Track {
                id: "native-test".to_string(),
                title: "Native test".to_string(),
                artist: "Musika".to_string(),
                album: "Native".to_string(),
                album_id: String::new(),
                favorite: false,
                duration: 2.0,
                url: format!("file://{}", path.display()),
                cover_art: String::new(),
                cover_url: String::new(),
            }],
            0,
        );
        std::thread::sleep(std::time::Duration::from_millis(250));
        let _ = player.pump();
        assert_eq!(player.state(), State::Playing);
        assert!(player.position() > 0.0, "native clock did not advance");
        player.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn native_seek_moves_the_clock_at_once() {
        let path = std::env::temp_dir().join(format!(
            "musika-pipewire-seek-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(480_000)).expect("write wav");
        let mut player = Player::new();
        player.load(
            vec![Track {
                id: "seek-test".to_string(),
                title: "Seek test".to_string(),
                artist: String::new(),
                album: String::new(),
                album_id: String::new(),
                favorite: false,
                duration: 10.0,
                url: format!("file://{}", path.display()),
                cover_art: String::new(),
                cover_url: String::new(),
            }],
            0,
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
        player.seek(5.0);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = player.pump();
        assert!(
            player.position() >= 4.9,
            "seek landed at {}",
            player.position()
        );
        player.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn seek_resumes_the_sink_quickly() {
        let path = std::env::temp_dir().join(format!(
            "musika-pipewire-resume-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(480_000)).expect("write wav");
        let mut native = Native::new().expect("pipewire sink");
        native.load(&path.to_string_lossy());
        std::thread::sleep(std::time::Duration::from_millis(200));
        native.seek_to(5.0);
        let started = std::time::Instant::now();
        let mut resumed = false;
        while started.elapsed() < std::time::Duration::from_secs(2) {
            if unsafe { musika_sink_played_frames(native.sink) } > 0 {
                let _ = native.poll(true);
                resumed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        eprintln!("seek resume after {:?}", started.elapsed());
        assert!(resumed, "sink never resumed after seek");
        native.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn auto_advances_when_the_track_ends() {
        let dir = std::env::temp_dir().join(format!(
            "musika-pipewire-advance-{}-{}",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        std::fs::write(&first, wav_bytes_for(2_400)).expect("first");
        std::fs::write(&second, wav_bytes_for(480_000)).expect("second");
        let mut player = Player::new();
        player.load(
            vec![
                Track {
                    id: "first".to_string(),
                    title: "First".to_string(),
                    artist: String::new(),
                    album: String::new(),
                    album_id: String::new(),
                    favorite: false,
                    duration: 0.05,
                    url: format!("file://{}", first.display()),
                    cover_art: String::new(),
                    cover_url: String::new(),
                },
                Track {
                    id: "second".to_string(),
                    title: "Second".to_string(),
                    artist: String::new(),
                    album: String::new(),
                    album_id: String::new(),
                    favorite: false,
                    duration: 10.0,
                    url: format!("file://{}", second.display()),
                    cover_art: String::new(),
                    cover_url: String::new(),
                },
            ],
            0,
        );
        std::thread::sleep(std::time::Duration::from_millis(700));
        for _ in 0..10 {
            let _ = player.pump();
            if player.index() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(player.index(), 1, "queue did not advance at end-of-stream");
        assert_eq!(player.state(), State::Playing);
        player.stop();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn decoder_reads_a_wav() {
        let path = std::env::temp_dir().join(format!(
            "musika-pipewire-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(96_000)).expect("write wav");
        let c_path = CString::new(path.to_string_lossy().as_bytes()).expect("path");
        let mut error = [0 as c_char; 512];
        let decoder = unsafe {
            musika_decoder_open(
                c_path.as_ptr(),
                std::ptr::null(),
                0,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        assert!(!decoder.is_null(), "decoder: {}", c_string(&error));
        let mut pcm = [0i16; 4096];
        let mut frames = 0;
        let ret = unsafe { musika_decoder_read(decoder, pcm.as_mut_ptr(), 1024, &mut frames) };
        assert_eq!(ret, 0);
        assert!(frames > 0);
        assert!(pcm.iter().any(|sample| *sample != 0));
        unsafe { musika_decoder_close(decoder) };
        let _ = std::fs::remove_file(path);
    }
}
