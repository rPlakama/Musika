// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Event-driven playback engine thread over [`MpvTransport`].
//!
//! One std thread owns the persistent `mpv` child process, its Unix IPC socket,
//! and a `std::sync::mpsc` command channel. It sleeps in `poll(2)` on the
//! command `eventfd` and the `mpv` IPC descriptor, waking immediately on either
//! a new [`AudioCommand`] or an asynchronous JSON event from `mpv`.
//!
//! The engine is deliberately dumb: it applies [`AudioCommand`]s to the
//! transport and reports [`EngineEvent`]s. Queue policy lives in
//! [`crate::player::core::PlaybackCore`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::Normalization;
use super::mpv::MpvTransport;

/// Safety-net wake cadence while playing/loading; real events wake `poll(2)`
/// immediately through the `mpv` IPC socket or the command `eventfd`.
const ACTIVE_WAIT: Duration = Duration::from_millis(250);
/// Safety-net wake cadence while paused/stopped.
const IDLE_WAIT: Duration = Duration::from_secs(1);

/// Transport commands the playback core emits for the engine.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AudioCommand {
    Load {
        url: String,
        next_url: Option<String>,
        paused: bool,
        generation: u64,
    },
    SetNextUrl(Option<String>),
    Pause,
    Resume,
    Seek(f64),
    /// 0..=100 UI scale.
    SetVolume(f64),
    SetMuted(bool),
    /// ReplayGain loudness normalization mode.
    SetNormalization(Normalization),
    /// Retarget the output stream (`None` = follow default / `auto`).
    SetOutputTarget(Option<String>),
    /// Install/replace the EQ profile (`None` = bypass).
    SetEq(Option<std::sync::Arc<crate::eq::EqProfile>>),
    SetExclusive(bool),
    SetBitPerfect(bool),
    /// Force a fixed output sample rate (`None` = native rate).
    SetSampleRate(Option<u32>),
    Stop,
}

/// Transport/decoder events the engine reports to the playback core.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum EngineEvent {
    /// First audio of the current load reached output (`playback-restart`).
    FirstPcm,
    /// A file opened: duration/stream size/seekability are known.
    Loaded {
        duration: f64,
        stream_size: u64,
        seekable: bool,
    },
    /// The gapless boundary was crossed: the next track is audible now.
    Boundary {
        duration: f64,
        stream_size: u64,
        /// Start position in the next track (0.0 for gapless).
        position: f64,
    },
    /// Current track reached EOF with no queued next item: advance the queue.
    Ended,
    /// Track/decoder failure: retry or advance depending on the kind.
    Failed {
        error: crate::player::PlaybackError,
    },
    /// MPV process or IPC failure: recover the engine, do NOT advance.
    SinkFailed {
        message: String,
    },
    /// The selected output device disappeared from MPV's device list. The
    /// transport already reverted to `auto`; the service must persist
    /// follow-default instead of retrying the dead target.
    OutputFallback {
        target: String,
    },
    /// Loading/seek finished; emit once per load/seek.
    LoadingCleared,
    /// MPV entered (`true`) or exited (`false`) `paused-for-cache` buffering.
    Buffering(bool),
    /// The decoder had to restart at `position` (non-seekable stream), so the
    /// core must re-anchor the playhead instead of trusting the seek target.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "consumed by PlaybackCore non-seekable seek tests")
    )]
    PositionReset {
        position: f64,
    },
}

/// Cheap transport clock read by the service (MPRIS/diagnostics).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EngineClock {
    pub played_frames: u64,
    pub buffered_frames: u64,
    pub underruns: u64,
    pub sample_rate: u32,
    pub loading: bool,
    /// Hardware-clock epoch: changes on seek/load/boundary/stop, so consumers
    /// discard `played_frames` deltas that span a timeline reset.
    pub epoch: u64,
}

#[derive(Debug, Default)]
struct ClockState {
    played_frames: AtomicU64,
    buffered_frames: AtomicU64,
    underruns: AtomicU64,
    sample_rate: AtomicU32,
    loading: AtomicBool,
    epoch: AtomicU64,
}

/// Handle to the engine thread. Dropping it shuts the engine and `mpv` down.
pub(crate) struct Engine {
    commands: Option<Sender<AudioCommand>>,
    clock: Arc<ClockState>,
    wake_fd: libc::c_int,
    join: Option<JoinHandle<()>>,
}

/// Callback the transport uses to report events to the playback service.
pub(crate) type EventSink = Arc<dyn Fn(EngineEvent) -> bool + Send + Sync>;

impl Engine {
    /// `Err` when `mpv` cannot be spawned or connected; never panics.
    pub(crate) fn new(events: EventSink) -> Result<Self, String> {
        // Reuse the transport created by the startup warm-up
        // (`entry::init_backend` calls `player::warm_native`). Waiting for an
        // in-flight warm-up guarantees a single `mpv` child at startup.
        let transport = match take_warm_transport() {
            Some(Ok(transport)) => transport,
            Some(Err(error)) => {
                return Err(format!("cannot start the audio engine: {error}"));
            }
            None => MpvTransport::new()
                .map_err(|error| format!("cannot start the audio engine: {error}"))?,
        };
        Self::from_transport(transport, events)
    }

    pub(crate) fn from_transport(
        transport: MpvTransport,
        events: EventSink,
    ) -> Result<Self, String> {
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
            .spawn(move || engine_thread(transport, command_rx, events, thread_clock, thread_wake))
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

    /// Last pumped transport counters.
    pub(crate) fn clock(&self) -> EngineClock {
        EngineClock {
            played_frames: self.clock.played_frames.load(Ordering::Relaxed),
            buffered_frames: self.clock.buffered_frames.load(Ordering::Relaxed),
            underruns: self.clock.underruns.load(Ordering::Relaxed),
            sample_rate: self.clock.sample_rate.load(Ordering::Relaxed),
            loading: self.clock.loading.load(Ordering::Relaxed),
            epoch: self.clock.epoch.load(Ordering::Relaxed),
        }
    }

    /// Stops the thread and terminates the child `mpv` cleanly.
    pub(crate) fn shutdown(self) {
        drop(self);
    }

    fn signal(&self) {
        signal_fd(self.wake_fd);
    }
}

/// Takes the startup warm-up transport, waiting while it is still starting.
fn take_warm_transport() -> Option<Result<MpvTransport, String>> {
    if let Some(result) = crate::player::take_warm_native() {
        return Some(result);
    }
    if !crate::player::warm_native_pending() {
        return None;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
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
        drop(self.commands.take());
        self.signal();
        if let Some(join) = self.join.take() {
            _ = join.join();
        }
        close_fd(self.wake_fd);
    }
}

fn engine_thread(
    mut transport: MpvTransport,
    commands: Receiver<AudioCommand>,
    events: EventSink,
    clock: Arc<ClockState>,
    wake_fd: libc::c_int,
) {
    let mut emit = |event: EngineEvent| -> bool { events(event) };

    // Drain any initial startup property events immediately (e.g., audio-device-list).
    if !transport.pump(&mut emit) {
        return;
    }
    store_clock(&clock, &transport);

    loop {
        let timeout = if transport.is_active() {
            ACTIVE_WAIT
        } else {
            IDLE_WAIT
        };
        wait(wake_fd, transport.ipc_fd(), timeout);
        drain_fd(wake_fd);

        let mut disconnected = false;
        let mut alive = true;
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if !transport.apply_command(command, &mut emit) {
                        alive = false;
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected || !alive {
            break;
        }

        if !transport.pump(&mut emit) {
            break;
        }
        store_clock(&clock, &transport);
    }
}

fn store_clock(clock: &ClockState, transport: &MpvTransport) {
    clock
        .played_frames
        .store(transport.played_frames(), Ordering::Relaxed);
    clock
        .buffered_frames
        .store(transport.buffered_frames(), Ordering::Relaxed);
    clock
        .underruns
        .store(transport.underruns(), Ordering::Relaxed);
    clock
        .sample_rate
        .store(transport.sample_rate(), Ordering::Relaxed);
    clock.loading.store(transport.loading(), Ordering::Relaxed);
    clock
        .epoch
        .store(transport.clock_epoch(), Ordering::Relaxed);
}

/// Blocks until a command eventfd, the MPV IPC socket, or the timeout fires.
fn wait(command_fd: libc::c_int, ipc_fd: libc::c_int, timeout: Duration) {
    let mut fds = [
        libc::pollfd {
            fd: command_fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: ipc_fd,
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
    // SAFETY: `fd` is the eventfd owned by `Engine` and closed only in `Drop`.
    // `one` is a live `u64` and the descriptor is non-blocking.
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
    // SAFETY: `fd` is the engine's owned non-blocking eventfd and `value` is a
    // live 8-byte buffer.
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
    // SAFETY: `fd` is a valid open descriptor owned by `Engine` and closed
    // exactly once here.
    unsafe {
        _ = libc::close(fd);
    }
}
