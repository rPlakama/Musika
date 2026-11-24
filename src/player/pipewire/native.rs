//! Rust transport façade: PipeWire sink + decode worker + sink-driven clock.
//!
//! [`Native`] is a thin, blocking wrapper around the worker and the sink. The
//! event-driven engine (`crate::player::engine`) drives it through the
//! `take_*` accessors below and sleeps on [`Native::wake_fd`] instead of
//! polling.

use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "alsa-backend")]
use super::alsa::AlsaSink;
use super::sink::{PcmSink, Sink};
use super::status::{Shared, SinkStatus};
use super::worker::Worker;
use crate::player::Normalization;

/// Buffered-byte thresholds for the loading-clear heuristic: a twentieth of the
/// known stream size, clamped to the PCM ring.
const MIN_LOADING_REFERENCE: u64 = 64 * 1024;
const MAX_LOADING_REFERENCE: u64 = 2 * 1024 * 1024;
const FALLBACK_LOADING_REFERENCE: u64 = 1024 * 1024;

pub(crate) struct Native {
    sink: Arc<dyn PcmSink>,
    worker: Worker,
    shared: Arc<Shared>,
}

impl Native {
    pub(crate) fn new() -> Result<Self, String> {
        let shared = Arc::new(Shared::new());
        let settings = crate::playback_settings::current();
        let env_backend = std::env::var("MUSIKA_OUTPUT_BACKEND").ok();
        let want_alsa = env_backend
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("alsa"))
            || (env_backend.is_none()
                && settings.output_backend == crate::playback_settings::OutputBackend::Alsa);
        let sink: Arc<dyn PcmSink> = if want_alsa {
            match open_alsa_sink(Arc::clone(&shared)) {
                Ok(sink) => sink,
                Err(error) => {
                    crate::debug_log!("alsa: unavailable, falling back to PipeWire: {error}");
                    Arc::new(Sink::new(Arc::clone(&shared))?)
                }
            }
        } else {
            Arc::new(Sink::new(Arc::clone(&shared))?)
        };
        let worker = Worker::new(Arc::clone(&shared), Arc::clone(&sink))?;
        shared
            .clock_start
            .store(sink.played_frames(), Ordering::Release);
        crate::startup::trace(if want_alsa {
            "engine.alsa"
        } else {
            "engine.pipewire"
        });
        Ok(Self {
            sink,
            worker,
            shared,
        })
    }

    /// Loads `url` with the playback core's generation. A newer generation
    /// supersedes any queued load that has not reached the decoder yet.
    pub(crate) fn load_with_next_generation(
        &self,
        url: &str,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    ) {
        crate::startup::trace("play.requested");
        self.sink.ring().reset_underruns();
        self.worker
            .load(url.to_string(), next_url, paused, generation);
    }

    pub(crate) fn set_next_url(&self, next_url: Option<String>) {
        self.worker.set_next_url(next_url);
    }

    pub(crate) fn stop(&self) {
        self.worker.stop();
        self.sink.ring().reset_underruns();
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
        // The clock advances in frames at the current pipeline rate, which the
        // sink/ring own (a track may be 44.1 kHz, 96 kHz, ...).
        let rate = f64::from(self.sink.rate().max(1));
        let position = origin + self.sink.played_frames().saturating_sub(start) as f64 / rate;
        self.worker.set_paused(paused, position);
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        self.sink.set_volume((volume / 100.0).clamp(0.0, 1.0));
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.sink.set_muted(muted);
    }

    pub(crate) fn set_output_target(&self, target: Option<String>) -> Result<(), String> {
        self.sink.set_target(target)
    }

    pub(crate) fn set_exclusive(&self, exclusive: bool) -> Result<(), String> {
        self.sink.set_exclusive(exclusive)
    }

    pub(crate) fn set_bit_perfect(&self, enabled: bool) {
        self.shared.set_bit_perfect(enabled);
    }

    pub(crate) fn set_eq(&self, profile: Option<std::sync::Arc<crate::eq::EqProfile>>) {
        self.shared.set_eq(profile);
    }

    pub(crate) fn set_crossfade_config(&self, config: crate::player::CrossfadeConfig) {
        self.shared.set_crossfade_config(config);
    }

    pub(crate) fn set_crossfade_allowed(&self, allowed: bool) {
        self.shared.set_crossfade_allowed(allowed);
    }

    pub(crate) fn set_crossfade(&self, enabled: bool) {
        self.shared.crossfade.store(enabled, Ordering::Release);
    }

    pub(crate) fn set_normalization(&self, mode: Normalization) {
        self.shared
            .normalization
            .store(u32::from(mode.code()), Ordering::Release);
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
    pub(crate) fn take_boundary_event(&self) -> Option<(f64, u64, f64)> {
        if !self
            .shared
            .boundary_reached(self.sink.epoch(), self.sink.read_frames())
        {
            return None;
        }
        self.shared.boundary_active.store(false, Ordering::Release);
        let duration = *self
            .shared
            .boundary_duration
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stream_size = self.shared.boundary_stream_size.load(Ordering::Acquire);
        let origin = *self
            .shared
            .boundary_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let mut status = self.shared.lock();
            status.duration = duration;
            status.stream_size = stream_size;
            status.loading = false;
        }
        self.shared
            .clock_start
            .store(self.sink.played_frames(), Ordering::Release);
        *self
            .shared
            .clock_origin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = origin;
        Some((duration, stream_size, origin))
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
    pub(crate) fn take_failed(&self) -> Option<crate::player::PlaybackError> {
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

    /// Consumes a notification that the selected output node disappeared.
    pub(crate) fn take_output_fallback(&self) -> Option<String> {
        self.shared.take_output_fallback()
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
        // Never clear loading while the ring is intentionally held for remote
        // pre-buffering or mid-track rebuffering; the worker clears buffering
        // once the watermark (or EOF) is reached.
        if self.sink.ring().buffering() {
            return false;
        }
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
        let buffered_bytes = self.sink.buffered_frames() * 8;
        let start = self.shared.clock_start.load(Ordering::Acquire);
        let played = self.sink.played_frames();
        if buffered_bytes >= reference || (playing && played > start) {
            self.shared.lock().loading = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn buffering(&self) -> bool {
        self.sink.ring().buffering()
    }

    pub(crate) fn loading(&self) -> bool {
        self.shared.lock().loading || self.sink.ring().buffering()
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.sink.played_frames()
    }

    /// Hardware-clock epoch (bumped on seek/load/flush/rebuild).
    pub(crate) fn clock_epoch(&self) -> u64 {
        self.shared.clock_epoch()
    }

    /// Consumes a pending "decoder restarted at 0" notice.
    pub(crate) fn take_position_reset(&self) -> Option<f64> {
        self.shared.take_position_reset()
    }

    pub(crate) fn rate(&self) -> u32 {
        self.sink.rate()
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        self.sink.buffered_frames()
    }

    pub(crate) fn underruns(&self) -> u64 {
        self.sink.underruns()
    }
}

#[cfg(feature = "alsa-backend")]
fn open_alsa_sink(shared: Arc<Shared>) -> Result<Arc<dyn PcmSink>, String> {
    Ok(Arc::new(AlsaSink::new(shared)?))
}

#[cfg(not(feature = "alsa-backend"))]
fn open_alsa_sink(_shared: Arc<Shared>) -> Result<Arc<dyn PcmSink>, String> {
    Err("direct ALSA backend is unavailable (built without `alsa-backend` feature)".to_string())
}

#[cfg(all(test, not(feature = "alsa-backend")))]
mod tests {
    use super::*;

    #[test]
    fn alsa_sink_returns_descriptive_error_when_feature_disabled() {
        let err = match open_alsa_sink(Arc::new(Shared::new())) {
            Ok(_) => panic!("expected open_alsa_sink to fail when alsa-backend is disabled"),
            Err(err) => err,
        };
        assert!(
            err.contains("alsa-backend"),
            "unexpected error message: {err}"
        );
    }
}
