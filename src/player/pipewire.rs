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
use std::ffi::{CString, c_char, c_double, c_int};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::{ErrorInfo, Player, Ready, State};

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
    fn musika_sink_write(sink: *mut MusikaSink, data: *const i16, frames: usize);
    fn musika_sink_set_paused(sink: *mut MusikaSink, paused: c_int);
    fn musika_sink_set_muted(sink: *mut MusikaSink, muted: c_int);
    fn musika_sink_set_volume(sink: *mut MusikaSink, volume: f32);
    fn musika_sink_failed(sink: *mut MusikaSink) -> c_int;
    fn musika_sink_played_frames(sink: *mut MusikaSink) -> u64;
    fn musika_sink_flush(sink: *mut MusikaSink);
    fn musika_sink_reset(sink: *mut MusikaSink);
    fn musika_sink_close_writes(sink: *mut MusikaSink);
    fn musika_sink_destroy(sink: *mut MusikaSink);

    fn musika_decoder_open(
        url: *const c_char,
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

struct SinkPtr(*mut MusikaSink);
unsafe impl Send for SinkPtr {}

pub(crate) struct Native {
    sink: *mut MusikaSink,
    decoder: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    seek: Arc<Mutex<Option<f64>>>,
    position_origin: Arc<Mutex<f64>>,
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
            position_origin: Arc::new(Mutex::new(0.0)),
        })
    }

    fn lock_status(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn load(&mut self, url: &str) {
        self.stop();
        unsafe {
            musika_sink_reset(self.sink);
            musika_sink_set_paused(self.sink, 0);
        }
        let stop = Arc::new(AtomicBool::new(false));
        *self
            .position_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
        let status = Arc::new(Mutex::new(Status {
            loading: true,
            ..Status::default()
        }));
        let seek = Arc::new(Mutex::new(None));
        let sink = SinkPtr(self.sink);
        let thread_stop = Arc::clone(&stop);
        let thread_status = Arc::clone(&status);
        let thread_seek = Arc::clone(&seek);
        let url = url.to_string();
        self.decoder = Some(thread::spawn(move || {
            decode_loop(sink, &url, thread_stop, thread_status, thread_seek);
        }));
        self.stop = stop;
        self.status = status;
        self.seek = seek;
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        unsafe { musika_sink_close_writes(self.sink) };
        if let Some(decoder) = self.decoder.take() {
            let _ = decoder.join();
        }
        unsafe {
            musika_sink_reset(self.sink);
            musika_sink_set_paused(self.sink, 1);
        }
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
        *self
            .seek
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(seconds.max(0.0));
        let mut status = self.lock_status();
        status.position = seconds.max(0.0);
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

    fn poll(&self) -> Status {
        let mut status = self.lock_status().clone();
        let origin = *self
            .position_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        status.position =
            origin + unsafe { musika_sink_played_frames(self.sink) } as f64 / 48_000.0;
        status.failed = self.lock_status().failed.take();
        if status.ended {
            status.ended = false;
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
        self.stop();
        unsafe { musika_sink_destroy(self.sink) };
    }
}

fn decode_loop(
    sink: SinkPtr,
    url: &str,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Status>>,
    seek: Arc<Mutex<Option<f64>>>,
) {
    let Ok(c_url) = CString::new(url) else {
        set_failed(&status, "invalid media URL");
        return;
    };
    let mut error = [0 as c_char; 512];
    let decoder = unsafe { musika_decoder_open(c_url.as_ptr(), error.as_mut_ptr(), error.len()) };
    if decoder.is_null() {
        set_failed(&status, &c_string(&error));
        return;
    }
    {
        let mut guard = lock(&status);
        guard.loading = false;
        guard.duration = unsafe { musika_decoder_duration(decoder) };
    }

    let mut pcm = vec![0i16; 4096 * 2];
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if let Some(seconds) = seek
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let ret = unsafe { musika_decoder_seek(decoder, seconds) };
            if ret < 0 {
                set_failed(&status, "could not seek the media stream");
                break;
            }
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
            lock(&status).ended = true;
            break;
        }
        let frames = frames as usize;
        unsafe { musika_sink_write(sink.0, pcm.as_ptr(), frames) };

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
                if let Some(volume) = value.as_f64() {
                    if let Some(native) = self.native.as_ref() {
                        native.set_volume(volume);
                    }
                }
            }
            "mute" => {
                if let Some(muted) = value.as_bool() {
                    if let Some(native) = self.native.as_ref() {
                        native.set_muted(muted);
                    }
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
        match Native::new() {
            Ok(native) => {
                self.native = Some(native);
                Ready::Connected
            }
            Err(err) => {
                self.shared().error = Some(ErrorInfo {
                    message: err,
                    played: false,
                });
                Ready::Failed
            }
        }
    }

    pub fn pump(&mut self) -> Option<String> {
        if self.state == State::Stopped {
            return None;
        }
        if self.deferred_start {
            match self.ensure_ready() {
                Ready::Connected => self.finish_deferred_start(),
                Ready::Failed => {
                    self.state = State::Stopped;
                    self.deferred_start = false;
                    return None;
                }
                Ready::Connecting => return None,
            }
        }
        let Some(status) = self.native.as_ref().map(Native::poll) else {
            return None;
        };
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
        self.shared().error = None;
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
        let status = native.poll();
        assert!(status.failed.is_none(), "sink failed: {:?}", status.failed);
        native.stop();
        let _ = std::fs::remove_file(path);
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
    fn decoder_reads_a_wav() {
        let path = std::env::temp_dir().join(format!(
            "musika-pipewire-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(96_000)).expect("write wav");
        let c_path = CString::new(path.to_string_lossy().as_bytes()).expect("path");
        let mut error = [0 as c_char; 512];
        let decoder =
            unsafe { musika_decoder_open(c_path.as_ptr(), error.as_mut_ptr(), error.len()) };
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
