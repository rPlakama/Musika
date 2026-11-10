//! Rust transport façade: PipeWire sink + decode worker + sink-driven clock.
//!
//! [`Native`] is a thin, blocking wrapper around the worker and the sink. The
//! event-driven engine (`crate::player::engine`) drives it through the
//! `take_*` accessors below and sleeps on [`Native::wake_fd`] instead of
//! polling.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::sink::Sink;
use super::status::{Shared, SinkStatus};
use super::worker::Worker;

/// Buffered-byte thresholds for the loading-clear heuristic: a twentieth of the
/// known stream size, clamped to the PCM ring.
const MIN_LOADING_REFERENCE: u64 = 64 * 1024;
const MAX_LOADING_REFERENCE: u64 = 2 * 1024 * 1024;
const FALLBACK_LOADING_REFERENCE: u64 = 1024 * 1024;

pub(crate) struct Native {
    sink: Arc<Sink>,
    worker: Worker,
    shared: Arc<Shared>,
}

impl Native {
    pub(crate) fn new() -> Result<Self, String> {
        let shared = Arc::new(Shared::new());
        let sink = Arc::new(Sink::new(Arc::clone(&shared))?);
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

    /// Loads `url` with the playback core's generation. A newer generation
    /// supersedes any queued load that has not reached the decoder yet.
    pub(crate) fn load_with_next_generation(
        &mut self,
        url: &str,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    ) {
        crate::startup::trace("play.requested");
        self.worker
            .load(url.to_string(), next_url, paused, generation);
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
        let origin = *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let position = origin + self.sink.played_frames().saturating_sub(start) as f64 / 48_000.0;
        self.worker.set_paused(paused, position);
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        self.sink.set_volume((volume / 100.0).clamp(0.0, 1.0));
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.sink.set_muted(muted);
    }

    /// Descriptor the engine polls for boundary / stream-state / decoder
    /// wakeups. `-1` when the shared eventfd could not be created.
    pub(crate) fn wake_fd(&self) -> libc::c_int {
        self.shared.wake_fd()
    }

    pub(crate) fn drain_wake(&self) {
        self.shared.drain_wake();
    }

    /// Consumes a crossed gapless boundary, returning the next track's
    /// duration and stream size.
    pub(crate) fn take_boundary_event(&self) -> Option<(f64, u64)> {
        if !self.shared.boundary_active.load(Ordering::Acquire) {
            return None;
        }
        let frame = self.shared.boundary_frame.load(Ordering::Acquire);
        if self.sink.played_frames() < frame {
            return None;
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
        Some((duration, stream_size))
    }

    /// True once for each decoder that opened since the last call.
    pub(crate) fn take_loaded(&self) -> Option<(f64, u64, bool)> {
        let mut status = self.shared.lock();
        if !status.loaded {
            return None;
        }
        status.loaded = false;
        Some((status.duration, status.stream_size, status.seekable))
    }

    /// True once when the first PCM of the current load was written.
    pub(crate) fn take_first_pcm(&self) -> bool {
        self.shared.first_pcm.swap(false, Ordering::AcqRel)
    }

    /// Takes the decoder/track failure (if any).
    pub(crate) fn take_failed(&self) -> Option<String> {
        self.shared.lock().failed.take()
    }

    /// True once when the decoder reached EOF and the sink drained the ring.
    pub(crate) fn take_ended(&self) -> bool {
        if !self.shared.decode_done.load(Ordering::Acquire) || self.sink.buffered_frames() > 0 {
            return false;
        }
        self.shared.decode_done.store(false, Ordering::Release);
        self.shared.lock().loading = false;
        true
    }

    /// Consumes the latest PipeWire stream transition (state + error text).
    pub(crate) fn take_sink_transition(&self) -> Option<SinkStatus> {
        let seq = self.shared.sink_seq.load(Ordering::Acquire);
        if seq == self.shared.sink_seen.load(Ordering::Acquire) {
            return None;
        }
        let status = self
            .shared
            .sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        self.shared.sink_seen.store(seq, Ordering::Release);
        Some(status)
    }

    /// Clears `loading` once enough audio is buffered or playback started.
    /// Returns `true` on the transition only, so the engine can emit
    /// `EngineEvent::LoadingCleared` exactly once.
    pub(crate) fn loading_clear(&self, playing: bool) -> bool {
        let loaded = {
            let status = self.shared.lock();
            if !status.loading {
                return false;
            }
            status.stream_size
        };
        let reference = if loaded > 0 {
            (loaded / 20).clamp(MIN_LOADING_REFERENCE, MAX_LOADING_REFERENCE)
        } else {
            FALLBACK_LOADING_REFERENCE
        };
        let buffered_bytes = self.sink.buffered_frames() * 4;
        let start = self.shared.clock_start.load(Ordering::Acquire);
        let played = self.sink.played_frames();
        if buffered_bytes >= reference || (playing && played > start) {
            self.shared.lock().loading = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn loading(&self) -> bool {
        self.shared.lock().loading
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.sink.played_frames()
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        self.sink.buffered_frames()
    }

    pub(crate) fn failed(&self) -> bool {
        self.sink.failed()
    }
}
