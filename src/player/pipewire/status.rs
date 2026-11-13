//! Shared transport state between the decode worker, clock and Native façade.
//!
//! Everything here is plain atomics/mutexes plus one `eventfd`: the engine
//! thread polls that descriptor instead of re-reading the status, so a gapless
//! boundary, a stream error or a decoder lifecycle event wakes it immediately
//! (see `crate::player::engine`).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

#[derive(Debug, Clone, Default)]
pub(crate) struct Status {
    pub(crate) loading: bool,
    pub(crate) ended: bool,
    pub(crate) failed: Option<String>,
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) stream_size: u64,
    pub(crate) level: f64,
    pub(crate) peak: f64,
    /// The worker's current decoder is seekable, so a pause can keep it alive
    /// instead of interrupting it and reopening the stream on resume.
    pub(crate) seekable: bool,
    /// The worker currently owns a decoder (live; a parked decoder is released
    /// and shows up as `false`).
    pub(crate) decoder_live: bool,
    /// A decoder opened since the last engine poll: `EngineEvent::Loaded`.
    pub(crate) loaded: bool,
}

/// PipeWire stream state as reported by the sink thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SinkState {
    #[default]
    Unconnected,
    Connecting,
    Paused,
    Streaming,
    Error,
}

impl SinkState {
    pub(crate) fn is_failed(self) -> bool {
        self == Self::Error
    }
}

/// Last stream status plus the error text of the most recent failure.
#[derive(Debug, Clone, Default)]
pub(crate) struct SinkStatus {
    pub(crate) state: SinkState,
    pub(crate) error: Option<String>,
}

pub(crate) struct Shared {
    pub(crate) status: Mutex<Status>,
    /// Absolute sink frame at which the current track started.
    pub(crate) clock_start: AtomicU64,
    /// Displayed position at `clock_start` (nonzero after a seek).
    pub(crate) clock_origin: Mutex<f64>,
    /// Absolute ring frame where the next gapless track begins.
    pub(crate) boundary_frame: AtomicU64,
    pub(crate) boundary_active: AtomicBool,
    pub(crate) boundary_duration: Mutex<f64>,
    /// Position (seconds) the gapless next track starts at; nonzero only for
    /// a crossfade, where the next decoder has already played its fade-in.
    pub(crate) boundary_origin: Mutex<f64>,
    pub(crate) boundary_stream_size: AtomicU64,
    /// Generation used to abort stale decoders.
    pub(crate) generation: AtomicU64,
    /// Interrupt flag of the decoder currently owned by the worker.
    pub(crate) current_stop: Mutex<Option<std::sync::Arc<AtomicBool>>>,
    /// The current decoder reached EOF; the sink may still be playing the
    /// final buffered frames. The engine turns this into `Ended` only after the
    /// sink drains, so a short track is never cut off.
    pub(crate) decode_done: AtomicBool,
    /// First PCM written for the current load (`EngineEvent::FirstPcm`).
    pub(crate) first_pcm: AtomicBool,
    /// Last PipeWire stream status.
    pub(crate) sink: Mutex<SinkStatus>,
    /// Bumped by every stream status change; the engine compares it with
    /// `sink_seen` to consume each transition exactly once.
    pub(crate) sink_seq: AtomicU64,
    pub(crate) sink_seen: AtomicU64,
    /// ReplayGain normalization mode (`Normalization::code`).
    pub(crate) normalization: AtomicU32,
    /// Optional crossfade between tracks.
    pub(crate) crossfade: AtomicBool,
    /// Engine wakeup, written from the RT process callback (gapless boundary),
    /// the sink loop thread (stream status) and the decode worker (lifecycle).
    wake_fd: libc::c_int,
}

impl Shared {
    pub(crate) fn new() -> Self {
        // The wakeup descriptor is best-effort: if `eventfd` fails the engine
        // still pumps on its timeout, so playback stays correct (only latency
        // degrades). `poll(2)` ignores a negative descriptor.
        //
        // SAFETY: `eventfd` has no Rust-side preconditions; it creates a new
        // kernel eventfd and returns a descriptor (or -1). A successful
        // descriptor is owned by `Shared` and closed exactly once by `Drop`; a
        // failed one is stored as -1 and never dereferenced or written to
        // because every user checks for that first.
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let loaded = Status {
            loaded: false,
            ..Status::default()
        };
        Self {
            status: Mutex::new(loaded),
            clock_start: AtomicU64::new(0),
            clock_origin: Mutex::new(0.0),
            boundary_frame: AtomicU64::new(0),
            boundary_active: AtomicBool::new(false),
            boundary_duration: Mutex::new(0.0),
            boundary_origin: Mutex::new(0.0),
            boundary_stream_size: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            current_stop: Mutex::new(None),
            decode_done: AtomicBool::new(false),
            first_pcm: AtomicBool::new(false),
            sink: Mutex::new(SinkStatus::default()),
            sink_seq: AtomicU64::new(0),
            sink_seen: AtomicU64::new(0),
            normalization: AtomicU32::new(0),
            crossfade: AtomicBool::new(false),
            wake_fd: if wake_fd < 0 { -1 } else { wake_fd },
        }
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// File descriptor the engine can `poll`; `-1` when the eventfd failed.
    pub(crate) fn wake_fd(&self) -> libc::c_int {
        self.wake_fd
    }

    /// Wakes the engine thread. Safe to call from the RT process callback: it
    /// is a non-blocking 8-byte write with no allocation and no locks.
    pub(crate) fn signal(&self) {
        if self.wake_fd < 0 {
            return;
        }
        let one = 1u64;
        // SAFETY: `wake_fd` is the eventfd owned by this `Shared` and is
        // non-negative on this path (checked above); it is closed only in
        // `Drop`, which runs after the last `Arc<Shared>` clone is gone. `one`
        // is a live 8-byte `u64` and the requested size matches it. This is
        // called from the RT process callback, the sink loop and the decode
        // worker: all write the same fd, which the kernel serialises, and
        // `EFD_NONBLOCK` keeps the call from blocking the RT thread.
        unsafe {
            _ = libc::write(
                self.wake_fd,
                &one as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }

    /// Drains a pending wakeup (the eventfd counter is coalesced).
    pub(crate) fn drain_wake(&self) {
        if self.wake_fd < 0 {
            return;
        }
        let mut value = 0u64;
        // SAFETY: `wake_fd` is non-negative (checked above) and owned by this
        // `Shared`, which keeps it open until `Drop`; `value` is a live 8-byte
        // `u64` used as eventfd's read buffer. The read is non-blocking and an
        // empty counter is intentionally ignored.
        unsafe {
            _ = libc::read(
                self.wake_fd,
                &mut value as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            );
        }
    }

    /// Records a PipeWire stream transition and wakes the engine.
    pub(crate) fn set_sink_state(&self, state: SinkState, error: Option<String>) {
        {
            let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
            sink.state = state;
            if error.is_some() {
                sink.error = error;
            } else if !state.is_failed() {
                sink.error = None;
            }
        }
        self.sink_seq.fetch_add(1, Ordering::AcqRel);
        self.signal();
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if self.wake_fd >= 0 {
            // SAFETY: `wake_fd` was created by `eventfd` in `Shared::new` and
            // is non-negative on this path; `Drop` runs exactly once, when the
            // last `Arc<Shared>` is dropped, after which no clone can use it.
            // Failing to close is intentionally ignored and there is no other
            // owner that could close the same descriptor.
            unsafe {
                _ = libc::close(self.wake_fd);
            }
        }
    }
}
