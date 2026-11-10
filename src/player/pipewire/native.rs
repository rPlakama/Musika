//! Rust transport façade: PipeWire sink + decode worker + sink-driven clock.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::player::{Player, Ready, State};

use super::sink::Sink;
use super::status::{Shared, Status};
use super::worker::Worker;

pub(crate) struct Native {
    sink: Arc<Sink>,
    worker: Worker,
    shared: Arc<Shared>,
}

impl Native {
    pub(crate) fn new() -> Result<Self, String> {
        let sink = Arc::new(Sink::new()?);
        let shared = Arc::new(Shared::new());
        let worker = Worker::new(Arc::clone(&shared), Arc::clone(&sink))?;
        shared
            .clock_start
            .store(sink.played_frames(), Ordering::Release);
        Ok(Self {
            sink,
            worker,
            shared,
        })
    }

    pub(crate) fn load_with_next(&mut self, url: &str, next_url: Option<String>, paused: bool) {
        crate::startup::trace("play.requested");
        self.worker.load(url.to_string(), next_url, paused);
    }

    pub(crate) fn stop(&mut self) {
        self.worker.stop();
        self.sink.set_paused(true);
    }

    pub(crate) fn seek_to(&self, seconds: f64) {
        self.worker.seek(seconds);
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        let start = self.shared.clock_start.load(Ordering::Acquire);
        let origin = *self.shared.clock_origin.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let position = origin + self.sink.played_frames().saturating_sub(start) as f64 / 48_000.0;
        self.worker.set_paused(paused, position);
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        self.sink.set_volume((volume / 100.0).clamp(0.0, 1.0));
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.sink.set_muted(muted);
    }

    pub(crate) fn take_boundary(&self) -> bool {
        if !self.shared.boundary_active.load(Ordering::Acquire) {
            return false;
        }
        let frame = self.shared.boundary_frame.load(Ordering::Acquire);
        if self.sink.played_frames() < frame {
            return false;
        }
        self.shared.boundary_active.store(false, Ordering::Release);
        let duration = *self
            .shared
            .boundary_duration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stream_size = self.shared.boundary_stream_size.load(Ordering::Acquire);
        {
            let mut status = self.shared.lock();
            status.duration = duration;
            status.stream_size = stream_size;
            status.loading = false;
        }
        self.shared.clock_start.store(frame, Ordering::Release);
        *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = 0.0;
        true
    }

    pub(crate) fn loading(&self) -> bool {
        self.shared.lock().loading
    }

    fn poll(&self, playing: bool) -> Status {
        let start = self.shared.clock_start.load(Ordering::Acquire);
        let origin = *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let played = self.sink.played_frames();
        let mut status = {
            let status = self.shared.lock();
            status.clone()
        };
        status.position = origin + played.saturating_sub(start) as f64 / 48_000.0;

        const MIN_REFERENCE: u64 = 64 * 1024;
        const MAX_REFERENCE: u64 = 2 * 1024 * 1024;
        const FALLBACK_REFERENCE: u64 = 1024 * 1024;
        let reference = if status.stream_size > 0 {
            (status.stream_size / 20).clamp(MIN_REFERENCE, MAX_REFERENCE)
        } else {
            FALLBACK_REFERENCE
        };
        let buffered_bytes = self.sink.buffered_frames() * 4;
        if status.loading && (buffered_bytes >= reference || (playing && played > start)) {
            status.loading = false;
            self.shared.lock().loading = false;
        }

        let failed = self.shared.lock().failed.take();
        status.failed = failed;
        if self.shared.decode_done.load(Ordering::Acquire) && self.sink.buffered_frames() == 0 {
            self.shared.decode_done.store(false, Ordering::Release);
            status.ended = true;
            status.loading = false;
        }
        if status.ended {
            self.shared.lock().ended = false;
        }
        if self.sink.failed() && status.failed.is_none() {
            status.failed = Some("PipeWire stream stopped".to_string());
        }
        status
    }
}

impl Player {
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
                self.last_error = Some(format!("cannot initialise PipeWire: {err}"));
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
        }
        if let Some(message) = status.failed {
            let title = self
                .current()
                .map(|track| track.title.clone())
                .unwrap_or_default();
            let text = format!("{title}: {message}");
            eprintln!("player: {text}");
            self.last_error = Some(text.clone());
            self.advance_after_error();
            return Some(text);
        }
        if status.ended {
            self.advance_after_error();
        }
        None
    }

    /// IPC diagnostics: a one-line dump of the transport state and metrics.
    pub fn debug_report(&self) -> String {
        let track = self.current();
        let mut report = format!(
            "state={:?} index={}/{} title={:?} url={:?} last_error={:?}",
            self.state,
            self.index,
            self.queue.len(),
            track.map(|track| track.title.as_str()),
            track.map(|track| track.url.as_str()),
            self.last_error,
        );
        match self.native.as_ref() {
            Some(native) => {
                let status = native.shared.lock().clone();
                report.push_str(&format!(
                    " native=ready loading={} failed={:?} ended={} position={:.2} duration={:.2} sink_failed={} played_frames={} buffered_frames={}",
                    status.loading,
                    status.failed,
                    status.ended,
                    status.position,
                    status.duration,
                    native.sink.failed(),
                    native.sink.played_frames(),
                    native.sink.buffered_frames(),
                ));
            }
            None => report.push_str(" native=none"),
        }
        report
    }

    fn advance_after_error(&mut self) {
        if self.repeat == crate::player::Repeat::One {
            self.play_current();
        } else if self.index + 1 < self.queue.len() {
            self.index += 1;
            self.play_current();
        } else if self.repeat == crate::player::Repeat::All && !self.queue.is_empty() {
            self.index = 0;
            self.play_current();
        } else {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::player::{Player, State, Track};

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
        for index in 0..frames {
            let value = (((index as f32 / frames as f32) * 20_000.0) as i16).to_le_bytes();
            out.extend_from_slice(&value);
        }
        out
    }

    fn track(id: &str, path: &std::path::Path, duration: f64) -> Track {
        Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            album: String::new(),
            album_id: String::new(),
            favorite: false,
            duration,
            url: format!("file://{}", path.display()),
            cover_art: String::new(),
            cover_url: String::new(),
            cover_remote: String::new(),
        }
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn rust_player_plays_a_wav() {
        let path = std::env::temp_dir().join(format!(
            "musika-rust-player-{}-{}.wav",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        std::fs::write(&path, wav_bytes_for(96_000)).expect("wav");
        let mut player = Player::new();
        player.load(vec![track("rust-test", &path, 2.0)], 0);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(3) && player.position() <= 0.0 {
            let _ = player.pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(player.state(), State::Playing, "{}", player.debug_report());
        assert!(player.position() > 0.0, "{}", player.debug_report());
        player.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn rust_player_advances_into_predecoded_wav() {
        let dir = std::env::temp_dir();
        let first = dir.join(format!("musika-rust-gap-a-{}.wav", std::process::id()));
        let second = dir.join(format!("musika-rust-gap-b-{}.wav", std::process::id()));
        std::fs::write(&first, wav_bytes_for(96_000 * 3)).expect("first");
        std::fs::write(&second, wav_bytes_for(96_000 * 2)).expect("second");
        let mut player = Player::new();
        player.load(vec![track("a", &first, 3.0), track("b", &second, 2.0)], 0);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(8) && player.index() == 0 {
            let _ = player.pump();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(player.index(), 1, "gapless boundary was not crossed");
        assert_eq!(player.state(), State::Playing);
        player.stop();
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }
}
