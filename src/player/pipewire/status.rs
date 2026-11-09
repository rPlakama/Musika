//! Shared transport state between the decode worker, clock and Native façade.

use std::sync::atomic::{AtomicBool, AtomicU64};
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
    pub(crate) boundary_stream_size: AtomicU64,
    /// Generation used to abort stale decoders.
    pub(crate) generation: AtomicU64,
    /// Interrupt flag of the decoder currently owned by the worker.
    pub(crate) current_stop: Mutex<Option<std::sync::Arc<AtomicBool>>>,
    /// The current decoder reached EOF; the sink may still be playing the
    /// final buffered frames. Native::poll turns this into `Status::ended`
    /// only after the sink drains, so a short track is never cut off.
    pub(crate) decode_done: AtomicBool,
}

impl Shared {
    pub(crate) fn new() -> Self {
        Self {
            status: Mutex::new(Status::default()),
            clock_start: AtomicU64::new(0),
            clock_origin: Mutex::new(0.0),
            boundary_frame: AtomicU64::new(0),
            boundary_active: AtomicBool::new(false),
            boundary_duration: Mutex::new(0.0),
            boundary_stream_size: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            current_stop: Mutex::new(None),
            decode_done: AtomicBool::new(false),
        }
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
