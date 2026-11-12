//! Event-driven playback engine thread over [`Native`].
//!
//! One std thread owns the native transport (PipeWire sink + decode worker) and
//! a `std::sync::mpsc` command channel. It never polls the position: it sleeps
//! on an `eventfd` that the sink's RT callback writes on a gapless boundary, the
//! sink loop thread writes on a stream state change and the decode worker
//! writes on a decoder lifecycle event. The timeouts (10 ms while
//! playing/loading, 250 ms idle) are a safety net, not the transport clock.
//!
//! The engine is deliberately dumb: it applies [`AudioCommand`]s to the
//! transport and reports [`EngineEvent`]s. Queue policy lives in
//! [`crate::player::core::PlaybackCore`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::pipewire::{Native, SinkState};

/// Wake cadence while audible or loading: fast enough that a boundary or a
/// drained ring is handled within one audio period.
const ACTIVE_WAIT: Duration = Duration::from_millis(10);
/// Wake cadence while paused/stopped. Wakes are still event-driven.
const IDLE_WAIT: Duration = Duration::from_millis(250);

/// Transport commands the playback core emits for the engine.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AudioCommand {
    Load {
        url: String,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    },
    Pause,
    Resume,
    Seek(f64),
    /// 0..=100 UI scale.
    SetVolume(f64),
    SetMuted(bool),
    Stop,
}

/// Transport/decoder events the engine reports to the playback core.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EngineEvent {
    /// First PCM of the current load was written to the ring.
    FirstPcm,
    /// A decoder opened: duration/stream size/seekability are known.
    Loaded {
        duration: f64,
        stream_size: u64,
        seekable: bool,
    },
    /// The gapless boundary was crossed: the next track is audible now.
    Boundary {
        duration: f64,
        stream_size: u64,
    },
    /// Decoder EOF and the sink drained the ring: advance the queue.
    Ended,
    /// Track/decoder failure: advance the queue (old `advance_after_error`).
    Failed {
        message: String,
    },
    /// PipeWire stream failure: recover the engine, do NOT advance.
    SinkFailed {
        message: String,
    },
    SinkRecovered,
    /// The loading heuristic fired; emit once per load.
    LoadingCleared,
}

/// Cheap transport clock read by the service (MPRIS/diagnostics).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EngineClock {
    pub played_frames: u64,
    pub buffered_frames: u64,
    pub loading: bool,
    pub failed: bool,
}

#[derive(Debug, Default)]
struct ClockState {
    played_frames: AtomicU64,
    buffered_frames: AtomicU64,
    loading: AtomicBool,
    failed: AtomicBool,
}

/// Handle to the engine thread. Dropping it shuts the engine down.
pub(crate) struct Engine {
    commands: Option<Sender<AudioCommand>>,
    clock: Arc<ClockState>,
    wake_fd: libc::c_int,
    join: Option<JoinHandle<()>>,
}

impl Engine {
    /// `Err` when there is no usable PipeWire session; never panics.
    pub(crate) fn new(events: Sender<EngineEvent>) -> Result<Self, String> {
        // Reuse the transport created by the startup warm-up
        // (`entry::init_backend` calls `player::warm_native`). Waiting for an
        // in-flight warm-up instead of racing it is what guarantees a single
        // PipeWire stream: a second `Native` would never be dropped.
        let native = match take_warm_transport() {
            Some(Ok(native)) => native,
            Some(Err(error)) => {
                return Err(format!("cannot start the audio engine: {error}"));
            }
            None => {
                Native::new().map_err(|error| format!("cannot start the audio engine: {error}"))?
            }
        };
        let (command_tx, command_rx) = mpsc::channel::<AudioCommand>();
        let clock = Arc::new(ClockState::default());
        // SAFETY: `eventfd` has no Rust-side preconditions: it creates a new
        // kernel eventfd and returns a descriptor (or -1). The descriptor is
        // owned by the resulting `Engine` and closed exactly once by
        // `close_fd`, either on the spawn-error path below or in `Drop`.
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let thread_wake = if wake_fd < 0 { -1 } else { wake_fd };
        let thread_clock = Arc::clone(&clock);
        let join = match thread::Builder::new()
            .name("musika-engine".to_string())
            .spawn(move || engine_thread(native, command_rx, events, thread_clock, thread_wake))
        {
            Ok(join) => join,
            Err(error) => {
                close_fd(wake_fd);
                return Err(format!("cannot start the engine thread: {error}"));
            }
        };
        Ok(Self {
            commands: Some(command_tx),
            clock,
            wake_fd: if wake_fd < 0 { -1 } else { wake_fd },
            join: Some(join),
        })
    }

    /// Queues a transport command. Never blocks.
    pub(crate) fn send(&self, command: AudioCommand) {
        let Some(commands) = self.commands.as_ref() else {
            return;
        };
        _ = commands.send(command);
        self.signal();
    }

    /// Last pumped transport counters. While paused these are frozen at the
    /// last pump (nothing advances), so they are stable to sample.
    pub(crate) fn clock(&self) -> EngineClock {
        EngineClock {
            played_frames: self.clock.played_frames.load(Ordering::Relaxed),
            buffered_frames: self.clock.buffered_frames.load(Ordering::Relaxed),
            loading: self.clock.loading.load(Ordering::Relaxed),
            failed: self.clock.failed.load(Ordering::Relaxed),
        }
    }

    /// Stops the thread and releases the PipeWire stream (joins every thread).
    pub(crate) fn shutdown(self) {
        drop(self);
    }

    fn signal(&self) {
        signal_fd(self.wake_fd);
    }
}

/// Takes the startup warm-up transport, waiting while it is still connecting.
///
/// `None` means no warm-up was started (a direct `Native::new` is then the only
/// transport). `Some(Err(..))` means the warm-up failed: the caller must not
/// silently start a second one.
fn take_warm_transport() -> Option<Result<Native, String>> {
    if let Some(result) = crate::player::take_warm_native() {
        return Some(result);
    }
    if !crate::player::warm_native_pending() {
        return None;
    }
    // The warm-up thread is bounded by the sink's 10 s connect timeout.
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while std::time::Instant::now() < deadline {
        if let Some(result) = crate::player::take_warm_native() {
            return Some(result);
        }
        if !crate::player::warm_native_pending() {
            return crate::player::take_warm_native();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Closing the command channel makes the thread break out of its loop;
        // the wakeup cuts the current `poll` short so shutdown is bounded.
        drop(self.commands.take());
        self.signal();
        if let Some(join) = self.join.take() {
            _ = join.join();
        }
        close_fd(self.wake_fd);
    }
}

fn engine_thread(
    native: Native,
    commands: Receiver<AudioCommand>,
    events: Sender<EngineEvent>,
    clock: Arc<ClockState>,
    wake_fd: libc::c_int,
) {
    let shared_fd = native.wake_fd();
    let mut paused = true;
    let mut sink_failed = false;
    loop {
        let timeout = if !paused || native.loading() {
            ACTIVE_WAIT
        } else {
            IDLE_WAIT
        };
        wait(wake_fd, shared_fd, timeout);
        // Both descriptors are level-triggered eventfds; drain them so the next
        // `poll` blocks instead of spinning.
        drain_fd(wake_fd);
        native.drain_wake();

        let mut disconnected = false;
        loop {
            match commands.try_recv() {
                Ok(command) => apply_command(&native, command, &mut paused),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            break;
        }

        if !pump(&native, &events, &paused, &mut sink_failed) {
            break;
        }

        clock
            .played_frames
            .store(native.played_frames(), Ordering::Relaxed);
        clock
            .buffered_frames
            .store(native.buffered_frames(), Ordering::Relaxed);
        clock.loading.store(native.loading(), Ordering::Relaxed);
        clock.failed.store(native.failed(), Ordering::Relaxed);
    }
}

/// Applies one transport command. `paused` mirrors the transport so the wait
/// cadence and the loading heuristic stay honest.
fn apply_command(native: &Native, command: AudioCommand, paused: &mut bool) {
    match command {
        AudioCommand::Load {
            url,
            next_url,
            paused: load_paused,
            generation,
        } => {
            native.load_with_next_generation(&url, next_url, load_paused, generation);
            *paused = load_paused;
        }
        AudioCommand::Pause => {
            native.set_paused(true);
            *paused = true;
        }
        AudioCommand::Resume => {
            native.set_paused(false);
            *paused = false;
        }
        AudioCommand::Seek(seconds) => native.seek_to(seconds),
        AudioCommand::SetVolume(volume) => {
            // Volume/mute are ring atomics plus a loop-thread stream control,
            // so they apply while paused too.
            native.set_volume(volume);
        }
        AudioCommand::SetMuted(muted) => native.set_muted(muted),
        AudioCommand::Stop => {
            native.stop();
            *paused = true;
        }
    }
}

/// Drains the native transport into events. Returns `false` when the event
/// receiver closed (the engine must end).
fn pump(
    native: &Native,
    events: &Sender<EngineEvent>,
    paused: &bool,
    sink_failed: &mut bool,
) -> bool {
    let emit = |event: EngineEvent| -> bool { events.send(event).is_ok() };

    if let Some((duration, stream_size)) = native.take_boundary_event()
        && !emit(EngineEvent::Boundary {
            duration,
            stream_size,
        })
    {
        return false;
    }
    if let Some((duration, stream_size, seekable)) = native.take_loaded()
        && !emit(EngineEvent::Loaded {
            duration,
            stream_size,
            seekable,
        })
    {
        return false;
    }
    if native.take_first_pcm() && !emit(EngineEvent::FirstPcm) {
        return false;
    }
    if let Some(message) = native.take_failed()
        && !emit(EngineEvent::Failed { message })
    {
        return false;
    }
    if native.take_ended() && !emit(EngineEvent::Ended) {
        return false;
    }
    if let Some(status) = native.take_sink_transition() {
        match status.state {
            SinkState::Error => {
                *sink_failed = true;
                let message = status
                    .error
                    .unwrap_or_else(|| "PipeWire stream stopped".to_string());
                if !emit(EngineEvent::SinkFailed { message }) {
                    return false;
                }
            }
            _ if *sink_failed => {
                // A rebuilt or auto-reconnected stream is live again. The
                // service replays the current row (`CoreCommand::Recover`).
                *sink_failed = false;
                if !emit(EngineEvent::SinkRecovered) {
                    return false;
                }
            }
            SinkState::Unconnected
            | SinkState::Connecting
            | SinkState::Paused
            | SinkState::Streaming => {}
        }
    }
    // The loading heuristic moved out of `Native::poll`: the legacy player keeps
    // its own copy through the same helper, and the engine emits the transition
    // once per load.
    if native.loading_clear(!*paused) && !emit(EngineEvent::LoadingCleared) {
        return false;
    }
    true
}

/// Blocks until a command fd, a shared wake fd or the timeout fires.
fn wait(command_fd: libc::c_int, shared_fd: libc::c_int, timeout: Duration) {
    let mut fds = [
        libc::pollfd {
            fd: command_fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: shared_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: `fds` is a valid array of two `pollfd`s for the duration of the
    // call; a negative descriptor is ignored by `poll(2)`.
    unsafe {
        _ = libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis);
    }
}

fn signal_fd(fd: libc::c_int) {
    if fd < 0 {
        return;
    }
    let one = 1u64;
    // SAFETY: `fd` is the eventfd owned by `Engine` and it is closed only when
    // the engine is dropped, so it cannot be closed while a signal is in
    // flight from the engine thread or its command senders. `one` is a live
    // `u64` and the requested length matches it; the descriptor was created
    // with `EFD_NONBLOCK`, so the write cannot block.
    unsafe {
        _ = libc::write(
            fd,
            &one as *const u64 as *const libc::c_void,
            std::mem::size_of::<u64>(),
        );
    }
}

fn drain_fd(fd: libc::c_int) {
    if fd < 0 {
        return;
    }
    let mut value = 0u64;
    // SAFETY: `fd` is the engine's owned eventfd (still open; only `Drop`
    // closes it) and `value` is a live `u64` used as the 8-byte output buffer
    // eventfd expects. The descriptor is non-blocking, and an empty counter is
    // reported as an error that is intentionally ignored.
    unsafe {
        _ = libc::read(
            fd,
            &mut value as *mut u64 as *mut libc::c_void,
            std::mem::size_of::<u64>(),
        );
    }
}

fn close_fd(fd: libc::c_int) {
    if fd < 0 {
        return;
    }
    // SAFETY: `fd` is a valid open descriptor owned by the engine; the caller
    // closes it exactly once (the `Engine::new` spawn-error path or
    // `Engine::drop`, never both), and no other code uses it afterwards.
    unsafe {
        _ = libc::close(fd);
    }
}
