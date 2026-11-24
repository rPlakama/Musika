//! Rust PipeWire output sink.
//!
//! A dedicated thread owns every `!Send` PipeWire object and runs the main
//! loop. The decoder thread writes PCM into [`Ring`]; the RT process callback
//! drains it into the negotiated F32LE/stereo stream at the current pipeline
//! rate. [`Sink::set_format`] lets the decode worker renegotiate that rate
//! before it writes a track: the loop thread rebuilds the stream (destroy,
//! reconnect, re-register the listener) and acknowledges, because changing the
//! rate of a `pw_stream` reliably means replacing it. Controls and shutdown are
//! delivered through a PipeWire channel processed on the loop thread, so no
//! PipeWire object crosses threads.
//!
//! The RT callback also turns the gapless boundary into an event: once the
//! played-frame counter reaches the armed boundary frame it writes the shared
//! engine eventfd, so the engine thread advances the queue without polling.
//! Stream-state transitions are recorded here as well, so a `StreamState::Error`
//! wakes the engine and surfaces as `EngineEvent::SinkFailed` instead of being
//! folded into a track failure.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pipewire as pw;
use pw::spa::sys as spa_sys;
use pw::{properties::properties, spa};
use spa::pod::Pod;

use super::ring::{self, Ring};
use super::status::{Shared, SinkState};

/// How long [`Sink::set_format`] waits for the loop thread to apply the
/// rebuild. A rebuild only creates/connects a local stream object; the
/// negotiation itself happens asynchronously in the graph.
const FORMAT_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Length of the anti-click output envelope used for pause/resume and for the
/// start of a fresh track/seek. They are short enough not to feel like a
/// transition, but long enough to remove the discontinuity click (~20 ms).
const FADE_MS: f32 = 20.0;
/// Sleep before deactivating the stream on pause so the RT callback can finish
/// the fade-out envelope (the graph still calls it while the loop thread
/// sleeps). The stream is deactivated afterwards exactly as before.
const FADE_STOP_GRACE: Duration = Duration::from_millis(30);

/// Logged once when PipeWire refuses a stream control (system volume/mute).
static CONTROL_ERROR_LOGGED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

enum SinkCommand {
    SetPaused(bool),
    /// Linear 0..=1 stream volume for the system mixer.
    SetVolume(f32),
    SetMuted(bool),
    /// Rebuild the stream for a new pipeline rate. The ack fires on the loop
    /// thread once the stream is connected, or with the rebuild error.
    SetFormat {
        rate: u32,
        ack: mpsc::SyncSender<Result<(), String>>,
    },
    /// Retarget the stream to a named PipeWire sink (`None` = default).
    SetTarget {
        target: Option<String>,
        ack: mpsc::SyncSender<Result<(), String>>,
    },
    /// Request/release exclusive device access and rebuild the stream.
    SetExclusive {
        exclusive: bool,
        ack: mpsc::SyncSender<Result<(), String>>,
    },
    Shutdown,
}

/// User data owned by the local stream listener: the PCM ring plus the shared
/// transport state the RT callback reads (boundary frame, engine eventfd).
struct RtContext {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    fade: FadeState,
}

/// One linear output-gain ramp. The RT callback owns the state; requests are
/// handed over through an atomic on the ring, so no locks or allocations run
/// on the audio thread.
pub(crate) struct FadeState {
    from: f32,
    to: f32,
    remaining: u32,
    total: u32,
}

impl FadeState {
    pub(crate) fn new() -> Self {
        Self {
            from: 1.0,
            to: 1.0,
            remaining: 0,
            total: 1,
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.remaining > 0
    }

    pub(crate) fn current(&self) -> f32 {
        if self.remaining == 0 {
            self.to
        } else {
            let progress = self.remaining as f32 / self.total as f32;
            self.to + (self.from - self.to) * progress
        }
    }

    pub(crate) fn start(&mut self, target: f32, rate: u32) {
        self.from = self.current();
        self.to = target.clamp(0.0, 1.0);
        let frames = (rate.max(1) as f32 * FADE_MS / 1000.0).round() as u32;
        self.total = frames.max(1);
        self.remaining = self.total;
    }

    /// Multiplies `frames` stereo frames by the ramp, advancing one step per
    /// frame. A zero-frame callback (paused/empty ring) leaves the ramp where
    /// it is unless the caller forces completion.
    pub(crate) fn apply(&mut self, out: &mut [f32], frames: usize) {
        if frames == 0 {
            return;
        }
        let frames = frames.min(out.len() / ring::CHANNELS);
        if !self.active() {
            if self.to == 0.0 {
                out[..frames * ring::CHANNELS].fill(0.0);
            } else if (self.to - 1.0).abs() > f32::EPSILON {
                for sample in &mut out[..frames * ring::CHANNELS] {
                    *sample *= self.to;
                }
            }
            return;
        }
        for index in 0..frames {
            let gain = self.current();
            for channel in 0..ring::CHANNELS {
                out[index * ring::CHANNELS + channel] *= gain;
            }
            self.remaining = self.remaining.saturating_sub(1);
        }
    }
}

/// The decoder-facing surface shared by the PipeWire and ALSA sinks. Both
/// own the same `Ring`, so worker code is backend-agnostic.
pub(crate) trait PcmSink: Send + Sync {
    fn ring(&self) -> &Arc<ring::Ring>;
    fn epoch(&self) -> u64;
    fn rate(&self) -> u32;
    fn set_format(&self, rate: u32) -> Result<(), String>;
    fn set_target(&self, target: Option<String>) -> Result<(), String>;
    fn set_exclusive(&self, exclusive: bool) -> Result<(), String>;
    fn write(&self, pcm: &[f32], epoch: u64);
    fn set_paused(&self, paused: bool);
    fn request_fade_in(&self);
    fn request_fade_out(&self);
    fn set_muted(&self, muted: bool);
    fn set_volume(&self, volume: f64);
    fn flush(&self);
    fn reset(&self);
    fn close_writes(&self);
    fn played_frames(&self) -> u64;
    fn read_frames(&self) -> u64;
    fn buffered_frames(&self) -> u64;
    fn underruns(&self) -> u64;
    #[expect(
        dead_code,
        reason = "the failure flag is inspected through Sink::failed, never through the trait object"
    )]
    fn failed(&self) -> bool;
}

/// One connected PipeWire stream and its local listener.
///
/// Field order matters: `listener` unregisters its hook when dropped, so it
/// must be dropped while `stream` is still alive.
struct ActiveStream<'c> {
    #[expect(
        dead_code,
        reason = "kept only for its drop side effect: dropping the listener unregisters the stream hook"
    )]
    listener: pw::stream::StreamListener<RtContext>,
    stream: pw::stream::StreamBox<'c>,
}

pub(crate) struct Sink {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    hw_volume_active: Arc<AtomicBool>,
    hw_mute_active: Arc<AtomicBool>,
    commands: pw::channel::Sender<SinkCommand>,
    thread: Option<JoinHandle<()>>,
}

impl Sink {
    pub(crate) fn new(shared: Arc<Shared>) -> Result<Self, String> {
        let ring = Arc::new(Ring::new()?);
        let hw_volume_active = Arc::new(AtomicBool::new(false));
        let hw_mute_active = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (command_tx, command_rx) = pw::channel::channel::<SinkCommand>();
        let thread_ring = Arc::clone(&ring);
        let thread_shared = Arc::clone(&shared);
        let thread_hw_volume = Arc::clone(&hw_volume_active);
        let thread_hw_mute = Arc::clone(&hw_mute_active);
        let thread = thread::Builder::new()
            .name("musika-pipewire".to_string())
            .spawn(move || {
                sink_thread(
                    thread_ring,
                    thread_shared,
                    thread_hw_volume,
                    thread_hw_mute,
                    command_rx,
                    ready_tx,
                )
            })
            .map_err(|error| format!("cannot start PipeWire thread: {error}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                ring,
                shared,
                hw_volume_active,
                hw_mute_active,
                commands: command_tx,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                _ = thread.join();
                Err(error)
            }
            Err(error) => {
                _ = thread.join();
                Err(format!("PipeWire sink did not start: {error}"))
            }
        }
    }

    pub(crate) fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.ring.epoch()
    }

    /// Current PCM pipeline rate (Hz) shared by the ring and the stream.
    pub(crate) fn rate(&self) -> u32 {
        self.ring.rate()
    }

    /// Renegotiates the PipeWire stream to F32LE/stereo at `rate` (Hz).
    ///
    /// Called by the decode worker before it writes PCM for a track whose rate
    /// differs from the current pipeline rate. Blocks until the sink loop
    /// thread has applied the change, so once this returns the ring's usable
    /// limit and the graph format agree. Queued PCM from the previous rate is
    /// discarded as part of the change.
    pub(crate) fn set_format(&self, rate: u32) -> Result<(), String> {
        let (ack, ack_rx) = mpsc::sync_channel(1);
        self.commands
            .send(SinkCommand::SetFormat { rate, ack })
            .map_err(|_| "PipeWire sink is not running".to_string())?;
        match ack_rx.recv_timeout(FORMAT_ACK_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err("PipeWire sink did not acknowledge the format change".to_string()),
        }
    }

    /// Retargets the stream to `target` (`None` = PipeWire default) without
    /// discarding the already-queued PCM. Blocks until the loop thread has
    /// swapped the stream in.
    pub(crate) fn set_target(&self, target: Option<String>) -> Result<(), String> {
        let (ack, ack_rx) = mpsc::sync_channel(1);
        self.commands
            .send(SinkCommand::SetTarget { target, ack })
            .map_err(|_| "PipeWire sink is not running".to_string())?;
        match ack_rx.recv_timeout(FORMAT_ACK_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err("PipeWire sink did not acknowledge the device change".to_string()),
        }
    }

    pub(crate) fn set_exclusive(&self, exclusive: bool) -> Result<(), String> {
        let (ack, ack_rx) = mpsc::sync_channel(1);
        self.commands
            .send(SinkCommand::SetExclusive { exclusive, ack })
            .map_err(|_| "PipeWire sink is not running".to_string())?;
        match ack_rx.recv_timeout(FORMAT_ACK_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err("PipeWire sink did not acknowledge the exclusive change".to_string()),
        }
    }

    pub(crate) fn write(&self, pcm: &[f32], epoch: u64) {
        self.ring.write(pcm, epoch);
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.ring.set_paused(paused);
        self.ring.request_fade(!paused);
        _ = self.commands.send(SinkCommand::SetPaused(paused));
    }

    /// Requests an output fade-in when the next PCM arrives. Used after a
    /// load/seek reset so a fresh track starts without a click; gapless
    /// handoffs deliberately do not call this.
    pub(crate) fn request_fade_in(&self) {
        self.ring.request_fade(true);
    }

    pub(crate) fn request_fade_out(&self) {
        self.ring.request_fade(false);
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        update_software_mute(&self.ring, &self.hw_mute_active, muted);
        _ = self.commands.send(SinkCommand::SetMuted(muted));
    }

    /// `volume` is the linear 0..=1 software gain applied by the RT callback.
    /// The same value is pushed to the system mixer on the PipeWire loop
    /// thread; when that succeeds the software gain is set back to unity so the
    /// two paths cannot attenuate the signal twice.
    pub(crate) fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        update_software_volume(&self.ring, &self.hw_volume_active, volume);
        _ = self.commands.send(SinkCommand::SetVolume(volume as f32));
    }

    pub(crate) fn flush(&self) {
        // A seek/flush discards the frames the armed gapless boundary was
        // measured in; disarming prevents a spurious `Boundary` when the read
        // counter reaches the old marker in the new timeline (P-01).
        self.shared.disarm_boundary();
        // The ring frame timeline restarts: hardware-clock deltas from before
        // the flush must not be applied to the new playhead.
        self.shared.bump_clock_epoch();
        self.ring.flush();
    }

    pub(crate) fn reset(&self) {
        self.shared.bump_clock_epoch();
        self.ring.reset();
    }

    pub(crate) fn close_writes(&self) {
        self.ring.close_writes();
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.ring.played_frames()
    }

    pub(crate) fn read_frames(&self) -> u64 {
        self.ring.read_frames()
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        self.ring.buffered_frames()
    }

    pub(crate) fn underruns(&self) -> u64 {
        self.ring.underruns()
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the PipeWire sink tests inspect the failure flag"
        )
    )]
    pub(crate) fn failed(&self) -> bool {
        self.ring.failed()
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.ring.close_writes();
        self.ring.wake();
        _ = self.commands.send(SinkCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            _ = thread.join();
        }
    }
}

fn update_software_volume(ring: &Ring, hw_volume_active: &AtomicBool, volume: f64) {
    if !hw_volume_active.load(Ordering::Acquire) {
        ring.set_volume(volume);
    }
}

fn update_software_mute(ring: &Ring, hw_mute_active: &AtomicBool, muted: bool) {
    if !hw_mute_active.load(Ordering::Acquire) {
        ring.set_muted(muted);
    }
}

fn sink_thread(
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    hw_volume_active: Arc<AtomicBool>,
    hw_mute_active: Arc<AtomicBool>,
    commands: pw::channel::Receiver<SinkCommand>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let result = run_sink(
        ring,
        shared,
        hw_volume_active,
        hw_mute_active,
        commands,
        ready.clone(),
    );
    if let Err(error) = result {
        _ = ready.send(Err(error));
    }
}

/// Logs the first refused stream control; the software path remains the
/// fallback for every refusal.
fn log_refused_control(id: u32) {
    if !CONTROL_ERROR_LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!("player: PipeWire refused stream control {id} (using the software path)");
    }
}

/// Applies the requested stream volume to the graph node and the software
/// fallback. Returns whether the node accepted the control.
fn apply_volume(
    stream: &pw::stream::Stream,
    ring: &Ring,
    hw_volume_active: &AtomicBool,
    volume: f32,
) -> bool {
    let applied = stream
        .set_control(spa_sys::SPA_PROP_channelVolumes, &[volume, volume])
        .is_ok();
    hw_volume_active.store(applied, Ordering::Release);
    // The graph applies the gain when it accepted the control; then the sample
    // path stays at unity so volume is not squared. A refused control keeps the
    // software gain at the requested value.
    ring.set_volume(if applied { 1.0 } else { f64::from(volume) });
    if !applied {
        log_refused_control(spa_sys::SPA_PROP_channelVolumes);
    }
    applied
}

/// Applies the requested soft-mute to the graph node and the software
/// fallback. Returns whether the node accepted the control.
fn apply_mute(
    stream: &pw::stream::Stream,
    ring: &Ring,
    hw_mute_active: &AtomicBool,
    muted: bool,
) -> bool {
    let applied = stream
        .set_control(spa_sys::SPA_PROP_softMute, &[if muted { 1.0 } else { 0.0 }])
        .is_ok();
    hw_mute_active.store(applied, Ordering::Release);
    // The graph mutes or the samples are zeroed, never both silently left to
    // the wrong path.
    ring.set_muted(if applied { false } else { muted });
    if !applied {
        log_refused_control(spa_sys::SPA_PROP_softMute);
    }
    applied
}

/// Creates, registers and connects a fresh stream for `rate`.
fn build_stream<'c>(
    core: &'c pw::core::Core,
    ring: &Arc<Ring>,
    shared: &Arc<Shared>,
    rate: u32,
    target: Option<&str>,
    exclusive: bool,
) -> Result<ActiveStream<'c>, String> {
    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Musika",
    };
    if let Some(target) = target {
        // Object name or object.serial; the engine stores the stable node
        // name from the device picker.
        props.insert(*pw::keys::TARGET_OBJECT, target);
    }
    if exclusive {
        // A request, not a hard guarantee: PipeWire decides whether the
        // device can be released to this node. The stream is rebuilt when
        // the user toggles it.
        props.insert(*pw::keys::NODE_EXCLUSIVE, "true");
    }
    let stream = pw::stream::StreamBox::new(core, "Musika", props)
        .map_err(|error| format!("cannot create PipeWire stream: {error}"))?;

    let listener_ctx = RtContext {
        ring: Arc::clone(ring),
        shared: Arc::clone(shared),
        fade: FadeState::new(),
    };
    let listener = stream
        .add_local_listener_with_user_data(listener_ctx)
        .state_changed(|_, ctx, _old, new| {
            let (state, error) = match &new {
                pw::stream::StreamState::Connecting => (SinkState::Connecting, None),
                pw::stream::StreamState::Paused => (SinkState::Paused, None),
                pw::stream::StreamState::Streaming => (SinkState::Streaming, None),
                pw::stream::StreamState::Unconnected => (SinkState::Unconnected, None),
                pw::stream::StreamState::Error(message) => {
                    (SinkState::Error, Some(message.clone()))
                }
            };
            let failed = state.is_failed();
            ctx.ring.set_failed(failed);
            ctx.shared.set_sink_state(state, error);
        })
        .process(process)
        .register()
        .map_err(|error| format!("cannot register PipeWire listener: {error}"))?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(rate);
    audio_info.set_channels(ring::CHANNELS as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa_sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa_sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa_sys::SPA_TYPE_OBJECT_Format,
            id: spa_sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .map_err(|error| format!("cannot build PipeWire format: {error}"))?
    .0
    .into_inner();
    let pod = Pod::from_bytes(&values).ok_or_else(|| "invalid PipeWire format pod".to_string())?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|error| format!("cannot connect PipeWire stream: {error}"))?;

    // Start suspended. A connected node is scheduled at graph rate even with
    // no track loaded (the core otherwise wakes its RT thread ~50x/s while
    // idle); `Command::Load`/`SetPaused` activates it before any PCM is
    // written, and `switch_format` restores the ring's paused state after a
    // rebuild. A failure is only a fallback to the old always-active stream.
    if let Err(error) = stream.set_active(false) {
        crate::debug_log!("pipewire: cannot start the stream inactive: {error}");
    }

    Ok(ActiveStream { listener, stream })
}

/// Rebuilds the stream for a new pipeline rate and swaps it into `active`.
///
/// The ring rate (and its rate-scaled usable limit) is updated first, so a
/// producer that wakes up between the rebuild steps still sees the new limit;
/// the old stream is dropped before the replacement is built so its teardown
/// callback cannot clobber `SinkState::Connecting`.
fn switch_format<'c>(
    active: &mut Option<ActiveStream<'c>>,
    core: &'c pw::core::Core,
    ring: &Arc<Ring>,
    shared: &Arc<Shared>,
    rate: u32,
    target: Option<&str>,
    exclusive: bool,
) -> Result<(), String> {
    let rate = rate.clamp(1, ring::MAX_RATE);
    if rate == ring.rate() && active.is_some() {
        return Ok(());
    }
    let previous_rate = ring.set_rate(rate);
    // Frames queued at the old rate must not be presented as if they were
    // produced at the new rate. The producer blocks on the flush until the
    // rebuilt stream's first process callback services it.
    shared.disarm_boundary();
    shared.bump_clock_epoch();
    ring.flush();
    *active = None;
    match build_stream(core, ring, shared, rate, target, exclusive) {
        Ok(next) => {
            ring.set_failed(false);
            shared.set_sink_state(SinkState::Connecting, None);
            if let Err(error) = next.stream.set_active(!ring.paused()) {
                crate::debug_log!("pipewire: cannot restore stream active state: {error}");
            }
            *active = Some(next);
            Ok(())
        }
        Err(error) => {
            ring.set_rate(previous_rate);
            shared.disarm_boundary();
            shared.bump_clock_epoch();
            ring.flush();
            Err(error)
        }
    }
}

/// Rebuilds the stream for a different target at the current rate. Unlike a
/// format change this keeps the queued PCM: only the graph node changes.
fn rebuild_stream<'c>(
    active: &mut Option<ActiveStream<'c>>,
    core: &'c pw::core::Core,
    ring: &Arc<Ring>,
    shared: &Arc<Shared>,
    rate: u32,
    target: Option<&str>,
    exclusive: bool,
) -> Result<(), String> {
    *active = None;
    let next = build_stream(core, ring, shared, rate, target, exclusive)?;
    ring.set_failed(false);
    shared.set_sink_state(SinkState::Connecting, None);
    if let Err(error) = next.stream.set_active(!ring.paused()) {
        crate::debug_log!("pipewire: cannot restore stream active state: {error}");
    }
    *active = Some(next);
    Ok(())
}

/// Records a `default.audio.sink` property notification and returns `true`
/// only when the default sink actually changed after the initial bind seed.
fn record_default_sink_change(last_default: &mut Option<String>, value: Option<&str>) -> bool {
    let next = value.unwrap_or_default();
    let changed = match last_default.as_deref() {
        Some(previous) => previous != next,
        None => false,
    };
    *last_default = Some(next.to_string());
    changed
}

/// Clears `target` when the registry-ready snapshot has no sink with that
/// node name. Returns the vanished name so the engine can persist
/// follow-default. The registry is the authority; a missing default sink does
/// not count as "target missing" because `target` would already be `None`.
fn take_missing_target(
    target: &Rc<RefCell<Option<String>>>,
    sinks: &Rc<RefCell<HashMap<u32, String>>>,
    registry_ready: bool,
) -> Option<String> {
    if !registry_ready {
        return None;
    }
    let selected = target.borrow().clone()?;
    if sinks.borrow().values().any(|name| name == &selected) {
        return None;
    }
    *target.borrow_mut() = None;
    Some(selected)
}

fn run_sink(
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    hw_volume_active: Arc<AtomicBool>,
    hw_mute_active: Arc<AtomicBool>,
    commands: pw::channel::Receiver<SinkCommand>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| format!("cannot create PipeWire main loop: {error}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("cannot create PipeWire context: {error}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|error| format!("cannot connect to PipeWire: {error}"))?;

    // Commands are queued by the PipeWire loop callback and applied below, on
    // the loop thread: a format change rebuilds the `!Send` stream, and the
    // volume/mute controls need the current stream object.
    let pending: Rc<RefCell<VecDeque<SinkCommand>>> = Rc::new(RefCell::new(VecDeque::new()));
    let _command_listener = commands.attach(mainloop.loop_(), {
        let pending = Rc::clone(&pending);
        move |command| pending.borrow_mut().push_back(command)
    });

    let pause_deactivate_due = Rc::new(Cell::new(false));
    let pause_timer = mainloop.loop_().add_timer({
        let pause_deactivate_due = Rc::clone(&pause_deactivate_due);
        move |_expirations| {
            pause_deactivate_due.set(true);
        }
    });
    // The stream stayed inactive/errored when a resume asked it to be active
    // again: rebuild it instead of leaving the ring unconsumed (silent stuck
    // transport after a long pause).
    let reactivate_due = Rc::new(Cell::new(false));

    // Follow the default sink live: PipeWire publishes it through the
    // `default` metadata object. Rebinding replaces the previous proxy; the
    // callback only sets a flag, so a graph change never re-enters the stream
    // builder from inside the loop callback.
    let registry = core
        .get_registry_rc()
        .map_err(|error| format!("cannot read PipeWire registry: {error}"))?;
    let default_changed = Rc::new(Cell::new(false));
    let last_default: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let metadata_keep: Rc<
        RefCell<Option<(pw::metadata::Metadata, pw::metadata::MetadataListener)>>,
    > = Rc::new(RefCell::new(None));
    // The selected node is checked against this live registry snapshot. A
    // removed node is not necessarily a stream error: PipeWire can fall back
    // to the default sink silently, so waiting for `SinkFailed` would leave
    // the dead device persisted in the settings.
    let sinks: Rc<RefCell<HashMap<u32, String>>> = Rc::new(RefCell::new(HashMap::new()));
    let target: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let registry_ready = Rc::new(Cell::new(false));
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let registry = registry.clone();
            let default_changed = Rc::clone(&default_changed);
            let last_default = Rc::clone(&last_default);
            let metadata_keep = Rc::clone(&metadata_keep);
            let sinks = Rc::clone(&sinks);
            move |global| {
                if global.type_ == pw::types::ObjectType::Node {
                    if let Some(props) = global.props.as_ref()
                        && props.get(&pw::keys::MEDIA_CLASS) == Some("Audio/Sink")
                        && let Some(name) = props.get(&pw::keys::NODE_NAME)
                    {
                        sinks.borrow_mut().insert(global.id, name.to_string());
                    }
                    return;
                }
                if global.type_ != pw::types::ObjectType::Metadata {
                    return;
                }
                let is_default = global
                    .props
                    .as_ref()
                    .and_then(|props| props.get("metadata.name"))
                    == Some("default");
                if !is_default {
                    return;
                }
                let Ok(metadata) = registry.bind::<pw::metadata::Metadata, _>(global) else {
                    return;
                };
                let changed = Rc::clone(&default_changed);
                let last_default = Rc::clone(&last_default);
                let listener = metadata
                    .add_listener_local()
                    .property(move |_subject, key, _type, value| {
                        if key == Some("default.audio.sink")
                            && record_default_sink_change(&mut last_default.borrow_mut(), value)
                        {
                            changed.set(true);
                        }
                        0
                    })
                    .register();
                *metadata_keep.borrow_mut() = Some((metadata, listener));
            }
        })
        .global_remove({
            let sinks = Rc::clone(&sinks);
            move |id| {
                sinks.borrow_mut().remove(&id);
            }
        })
        .register();

    // A target can be applied before the initial registry enumeration ends.
    // Delay the missing-node verdict until the sync round-trip says the
    // snapshot is complete; otherwise a perfectly valid sink would look gone.
    let sync_ready = Rc::clone(&registry_ready);
    let pending_sync = core.sync(0).map_err(|error| error.to_string())?;
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending_sync {
                sync_ready.set(true);
            }
        })
        .register();

    let mut exclusive = false;
    let mut active = Some(build_stream(
        &core,
        &ring,
        &shared,
        ring.rate(),
        target.borrow().as_deref(),
        exclusive,
    )?);
    let mut volume = 1.0f32;
    let mut muted = false;
    // The freshly built node starts at unity; re-apply the mixer controls once
    // it reaches Paused/Streaming (a `set_control` before negotiation can be
    // refused even though the software path already carries the right gain).
    let mut controls_dirty = true;
    _ = ready.send(Ok(()));

    let mut running = true;
    while running {
        // Fully event-driven, like `mainloop.run()`: the loop blocks until a
        // stream event or a queued command wakes it, so idle playback does not
        // add periodic wakeups.
        if mainloop.loop_().iterate(pw::loop_::Timeout::Infinite) < 0 {
            break;
        }
        let batch: Vec<SinkCommand> = pending.borrow_mut().drain(..).collect();
        for command in batch {
            match command {
                SinkCommand::SetPaused(paused) => {
                    if paused {
                        // Let the RT callback consume the buffered tail
                        // through the fade-out envelope before deactivating
                        // the stream without blocking the PipeWire loop thread.
                        pause_deactivate_due.set(false);
                        _ = pause_timer.update_timer(Some(FADE_STOP_GRACE), None);
                    } else {
                        pause_deactivate_due.set(false);
                        _ = pause_timer.update_timer(None, None);
                        match active.as_ref() {
                            Some(active)
                                if matches!(
                                    active.stream.state(),
                                    pw::stream::StreamState::Paused
                                        | pw::stream::StreamState::Streaming
                                ) =>
                            {
                                if let Err(error) = active.stream.set_active(true) {
                                    crate::debug_log!(
                                        "pipewire: cannot resume stream, rebuilding: {error}"
                                    );
                                    reactivate_due.set(true);
                                }
                            }
                            // Deactivated/dead stream: `set_active(true)` cannot
                            // revive it, rebuild from scratch below.
                            _ => reactivate_due.set(true),
                        }
                    }
                }
                SinkCommand::SetVolume(value) => {
                    volume = value;
                    if let Some(active) = active.as_ref() {
                        apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                    }
                }
                SinkCommand::SetMuted(value) => {
                    muted = value;
                    if let Some(active) = active.as_ref() {
                        apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                    }
                }
                SinkCommand::SetFormat { rate, ack } => {
                    pause_deactivate_due.set(false);
                    _ = pause_timer.update_timer(None, None);
                    let result = switch_format(
                        &mut active,
                        &core,
                        &ring,
                        &shared,
                        rate,
                        target.borrow().as_deref(),
                        exclusive,
                    );
                    if result.is_ok()
                        && let Some(active) = active.as_ref()
                    {
                        // Replay the mixer state on the rebuilt node.
                        let volume_applied =
                            apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                        let mute_applied =
                            apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                        controls_dirty = !(volume_applied && mute_applied);
                    }
                    if ack.send(result).is_err() {
                        crate::debug_log!("pipewire: format-change ack receiver was dropped");
                    }
                }
                SinkCommand::SetTarget { target: next, ack } => {
                    pause_deactivate_due.set(false);
                    _ = pause_timer.update_timer(None, None);
                    *target.borrow_mut() = next;
                    let fallback = take_missing_target(&target, &sinks, registry_ready.get());
                    let result = rebuild_stream(
                        &mut active,
                        &core,
                        &ring,
                        &shared,
                        ring.rate(),
                        target.borrow().as_deref(),
                        exclusive,
                    );
                    if result.is_ok()
                        && let Some(active) = active.as_ref()
                    {
                        let volume_applied =
                            apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                        let mute_applied =
                            apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                        controls_dirty = !(volume_applied && mute_applied);
                    }
                    if let Some(lost) = fallback {
                        shared.set_output_fallback(lost);
                    }
                    if ack.send(result).is_err() {
                        crate::debug_log!("pipewire: target-change ack receiver was dropped");
                    }
                }
                SinkCommand::SetExclusive {
                    exclusive: next,
                    ack,
                } => {
                    pause_deactivate_due.set(false);
                    _ = pause_timer.update_timer(None, None);
                    exclusive = next;
                    let result = rebuild_stream(
                        &mut active,
                        &core,
                        &ring,
                        &shared,
                        ring.rate(),
                        target.borrow().as_deref(),
                        exclusive,
                    );
                    if result.is_ok()
                        && let Some(active) = active.as_ref()
                    {
                        let volume_applied =
                            apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                        let mute_applied =
                            apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                        controls_dirty = !(volume_applied && mute_applied);
                    }
                    if ack.send(result).is_err() {
                        crate::debug_log!("pipewire: exclusive-change ack receiver was dropped");
                    }
                }
                SinkCommand::Shutdown => running = false,
            }
        }
        if running && !ring.paused() && reactivate_due.replace(false) {
            // A resume asked for an active stream but the old one was already
            // gone/unusable. Rebuilding keeps the queued PCM (the ring owns it)
            // and reattaches to the graph; without this the ring would fill and
            // the decode worker would block forever while the UI shows Playing.
            match rebuild_stream(
                &mut active,
                &core,
                &ring,
                &shared,
                ring.rate(),
                target.borrow().as_deref(),
                exclusive,
            ) {
                Ok(()) => {
                    if let Some(active) = active.as_ref() {
                        let _ = apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                        let _ = apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                    }
                }
                Err(error) => {
                    crate::debug_log!("pipewire: cannot rebuild the paused stream: {error}");
                }
            }
        }
        if running
            && pause_deactivate_due.replace(false)
            && ring.paused()
            && let Some(active) = active.as_ref()
            && let Err(error) = active.stream.set_active(false)
        {
            crate::debug_log!("pipewire: cannot set stream active state: {error}");
        }
        if running && let Some(lost) = take_missing_target(&target, &sinks, registry_ready.get()) {
            // PipeWire can silently relink a stream whose target vanished;
            // make the fallback explicit and tell the service to persist it.
            pause_deactivate_due.set(false);
            _ = pause_timer.update_timer(None, None);
            let result = rebuild_stream(
                &mut active,
                &core,
                &ring,
                &shared,
                ring.rate(),
                None,
                exclusive,
            );
            if result.is_ok()
                && let Some(active) = active.as_ref()
            {
                let volume_applied = apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                let mute_applied = apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                controls_dirty = !(volume_applied && mute_applied);
            }
            shared.set_output_fallback(lost);
        }
        if running && default_changed.replace(false) && target.borrow().is_none() {
            pause_deactivate_due.set(false);
            _ = pause_timer.update_timer(None, None);
            let result = rebuild_stream(
                &mut active,
                &core,
                &ring,
                &shared,
                ring.rate(),
                None,
                exclusive,
            );
            if result.is_ok()
                && let Some(active) = active.as_ref()
            {
                let volume_applied = apply_volume(&active.stream, &ring, &hw_volume_active, volume);
                let mute_applied = apply_mute(&active.stream, &ring, &hw_mute_active, muted);
                controls_dirty = !(volume_applied && mute_applied);
            }
        }
        if running
            && controls_dirty
            && let Some(active) = active.as_ref()
            && matches!(
                active.stream.state(),
                pw::stream::StreamState::Paused | pw::stream::StreamState::Streaming
            )
        {
            let volume_applied = apply_volume(&active.stream, &ring, &hw_volume_active, volume);
            let mute_applied = apply_mute(&active.stream, &ring, &hw_mute_active, muted);
            controls_dirty = !(volume_applied && mute_applied);
        }
    }
    drop(active);
    Ok(())
}

fn process(stream: &pw::stream::Stream, ctx: &mut RtContext) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let requested = buffer.requested() as usize;
    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return;
    }
    let data = &mut datas[0];
    let Some(bytes) = data.data() else {
        return;
    };
    let max_frames = bytes.len() / (ring::CHANNELS * std::mem::size_of::<f32>());
    let frames = if requested > 0 {
        requested.min(max_frames)
    } else {
        max_frames
    };
    if frames == 0 {
        return;
    }
    let ptr = bytes.as_mut_ptr() as *mut f32;
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<f32>()) {
        return;
    }
    // SAFETY: the PipeWire buffer is at least `max_frames * 2` samples long and
    // is writable for the duration of the process callback.
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, frames * ring::CHANNELS) };
    let ring = &ctx.ring;
    let buffering = ring.buffering();
    // While pre-/re-buffering, hold playback completely silent without
    // consuming ring frames or pending fade-in requests (the worker arms
    // fade-in right before clearing `buffering`).
    if buffering {
        _ = ring.read_into(out, 0);
        out.fill(0.0);
        ctx.fade.remaining = 0;
    } else {
        match ring.take_fade_request() {
            1 => ctx.fade.start(1.0, ring.rate()),
            2 => ctx.fade.start(0.0, ring.rate()),
            _ => {}
        }
        let paused = ring.paused();
        // A deactivated PipeWire stream can still get a final/settling process
        // callback. Do not consume or advance the clock while paused; service a
        // pending reset/flush with a zero-frame read so the producer can wake.
        // While an envelope is active the callback keeps consuming the already
        // buffered audio: this is what makes the fade-out audible before the
        // stream is deactivated and the fade-in audible on resume.
        if paused && !ctx.fade.active() {
            _ = ring.read_into(out, 0);
            out.fill(0.0);
        } else {
            let written = ring.read_into(out, frames);
            out[written * ring::CHANNELS..].fill(0.0);
            ring::apply_gain(out, ring.muted(), ring.volume_milli(), frames);
            ctx.fade.apply(out, written);
            // A paused ring with nothing buffered cannot advance a fade-out:
            // let it finish so the next callback takes the silent branch.
            if paused && written == 0 {
                ctx.fade.remaining = 0;
            }
            // Ring drained completely while the decoder is still live before
            // EOF: hold playback in buffering state until the worker refills to
            // the rebuffer watermark instead of stuttering silence while time
            // advances.
            if !paused
                && written == 0
                && frames > 0
                && ctx.shared.can_rebuffer()
                && ring.enter_buffering()
            {
                ctx.shared.signal();
            }
        }
    }
    // Gapless boundary as an event: the armed boundary frame lives in the same
    // absolute frame space as `read`. Wake the engine; it consumes the
    // boundary and disarms it, so the write happens at most a handful of times.
    if ctx
        .shared
        .boundary_reached(ring.epoch(), ring.read_frames())
    {
        ctx.shared.signal();
    }
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = (ring::CHANNELS * std::mem::size_of::<f32>()) as i32;
    *chunk.size_mut() = (frames * ring::CHANNELS * std::mem::size_of::<f32>()) as u32;
}

impl PcmSink for Sink {
    fn ring(&self) -> &Arc<ring::Ring> {
        self.ring()
    }
    fn epoch(&self) -> u64 {
        self.epoch()
    }
    fn rate(&self) -> u32 {
        self.rate()
    }
    fn set_format(&self, rate: u32) -> Result<(), String> {
        self.set_format(rate)
    }
    fn set_target(&self, target: Option<String>) -> Result<(), String> {
        self.set_target(target)
    }
    fn set_exclusive(&self, exclusive: bool) -> Result<(), String> {
        self.set_exclusive(exclusive)
    }
    fn write(&self, pcm: &[f32], epoch: u64) {
        self.write(pcm, epoch);
    }
    fn set_paused(&self, paused: bool) {
        self.set_paused(paused);
    }
    fn request_fade_in(&self) {
        self.request_fade_in();
    }
    fn request_fade_out(&self) {
        self.request_fade_out();
    }
    fn set_muted(&self, muted: bool) {
        self.set_muted(muted);
    }
    fn set_volume(&self, volume: f64) {
        self.set_volume(volume);
    }
    fn flush(&self) {
        self.flush();
    }
    fn reset(&self) {
        self.reset();
    }
    fn close_writes(&self) {
        self.close_writes();
    }
    fn played_frames(&self) -> u64 {
        self.played_frames()
    }
    fn read_frames(&self) -> u64 {
        self.read_frames()
    }
    fn buffered_frames(&self) -> u64 {
        self.buffered_frames()
    }
    fn underruns(&self) -> u64 {
        self.underruns()
    }
    fn failed(&self) -> bool {
        self.failed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanished_selected_sink_clears_the_target_after_sync() {
        let target = Rc::new(RefCell::new(Some("sink.one".to_string())));
        let sinks = Rc::new(RefCell::new(HashMap::from([(7, "sink.one".to_string())])));

        assert_eq!(
            take_missing_target(&target, &sinks, false),
            None,
            "the initial registry snapshot is not authoritative yet"
        );
        assert_eq!(target.borrow().as_deref(), Some("sink.one"));
        assert_eq!(
            take_missing_target(&target, &sinks, true),
            None,
            "a present node keeps the target"
        );
        sinks.borrow_mut().clear();
        assert_eq!(
            take_missing_target(&target, &sinks, true),
            Some("sink.one".to_string())
        );
        assert!(target.borrow().is_none(), "the dead node is forgotten");
    }

    #[test]
    fn fade_out_ramps_to_silence() {
        let mut fade = FadeState::new();
        fade.start(0.0, 48_000);
        assert!(fade.active());
        let mut first = vec![0.5f32; 240 * ring::CHANNELS];
        fade.apply(&mut first, 240);
        assert!(
            first.iter().all(|sample| *sample > 0.0),
            "first fade-out block must be attenuated, not hard-zeroed"
        );
        assert!(fade.active());
        let mut rest = vec![0.5f32; 960 * ring::CHANNELS];
        fade.apply(&mut rest, 720);
        assert!(!fade.active());
        assert_eq!(fade.current(), 0.0);
        assert!(
            rest[..720 * ring::CHANNELS]
                .iter()
                .all(|sample| (0.0..=0.5).contains(sample)),
            "the fade-out is monotonically attenuated"
        );
        assert!(
            rest[(720 - 8) * ring::CHANNELS..720 * ring::CHANNELS]
                .iter()
                .all(|sample| *sample <= 0.01),
            "the tail of the fade-out is near silence"
        );
        let mut after = vec![0.5f32; 64 * ring::CHANNELS];
        fade.apply(&mut after, 64);
        assert!(
            after.iter().all(|sample| *sample == 0.0),
            "once the ramp is done, FadeState holds 0.0 to prevent post-fade pops"
        );
    }

    #[test]
    fn fade_state_holds_zero_after_fade_out_completes() {
        let mut fade = FadeState::new();
        fade.start(0.0, 48_000);
        let mut block = vec![1.0f32; 1200 * ring::CHANNELS];
        fade.apply(&mut block, 1200);
        assert!(!fade.active());
        assert_eq!(fade.current(), 0.0);

        let mut tail = vec![0.8f32; 128 * ring::CHANNELS];
        fade.apply(&mut tail, 128);
        assert!(
            tail.iter().all(|sample| *sample == 0.0),
            "completed fade-out must continue zeroing buffered tail frames until fade-in"
        );
    }

    #[test]
    fn software_ring_volume_is_not_double_applied_when_hw_volume_active() {
        let ring = Ring::new().expect("ring");
        let hw_volume_active = AtomicBool::new(false);
        let hw_mute_active = AtomicBool::new(false);

        // When hardware controls are inactive, software fallback updates the ring.
        update_software_volume(&ring, &hw_volume_active, 0.4);
        assert_eq!(ring.volume_milli(), 400);
        update_software_mute(&ring, &hw_mute_active, true);
        assert!(ring.muted());

        // When PipeWire hardware controls are active, ring stays at unity/unmuted
        // and Sink::set_volume / set_muted do not overwrite it with software attenuation.
        hw_volume_active.store(true, Ordering::Release);
        hw_mute_active.store(true, Ordering::Release);
        ring.set_volume(1.0);
        ring.set_muted(false);

        update_software_volume(&ring, &hw_volume_active, 0.25);
        assert_eq!(
            ring.volume_milli(),
            1000,
            "ring volume must stay at unity when hw_volume_active is true"
        );
        update_software_mute(&ring, &hw_mute_active, true);
        assert!(
            !ring.muted(),
            "ring mute must stay false when hw_mute_active is true"
        );
    }

    #[test]
    fn initial_default_sink_property_does_not_mark_default_changed() {
        let mut last_default = None;
        assert!(
            !record_default_sink_change(&mut last_default, Some("alsa_output.one")),
            "initial bind enumeration must seed last_default without triggering a rebuild"
        );
        assert_eq!(last_default.as_deref(), Some("alsa_output.one"));
        assert!(
            !record_default_sink_change(&mut last_default, Some("alsa_output.one")),
            "repeating the same sink name must not trigger a rebuild"
        );
        assert!(
            record_default_sink_change(&mut last_default, Some("alsa_output.two")),
            "changing to a new default sink must trigger a rebuild"
        );
        assert_eq!(last_default.as_deref(), Some("alsa_output.two"));
    }

    #[test]
    fn fade_in_starts_at_silence_and_reaches_unity() {
        let mut fade = FadeState::new();
        fade.start(0.0, 48_000);
        let mut silence = vec![1.0f32; 960 * ring::CHANNELS];
        fade.apply(&mut silence, 960);
        assert_eq!(fade.current(), 0.0);

        fade.start(1.0, 48_000);
        let mut block = vec![1.0f32; 960 * ring::CHANNELS];
        fade.apply(&mut block, 960);
        assert!(!fade.active());
        assert_eq!(fade.current(), 1.0);
        assert!(
            block[..10].iter().all(|sample| *sample < 0.99),
            "the first frames of a fade-in are near silence"
        );
    }

    #[test]
    fn empty_callbacks_do_not_advance_the_envelope() {
        let mut fade = FadeState::new();
        fade.start(0.0, 48_000);
        let before = fade.remaining;
        let mut empty: [f32; 0] = [];
        fade.apply(&mut empty, 0);
        assert_eq!(fade.remaining, before);
    }

    fn sink() -> Sink {
        Sink::new(Arc::new(Shared::new())).expect("PipeWire sink")
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn sink_starts_and_consumes_pcm() {
        let sink = sink();
        sink.set_volume(0.25);
        let epoch = sink.epoch();
        let pcm = vec![0.0f32; 4_800 * ring::CHANNELS];
        sink.write(&pcm, epoch);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(3) {
            if sink.played_frames() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!sink.failed(), "PipeWire sink reported a stream error");
        assert!(sink.played_frames() > 0, "sink consumed no frames");
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn sink_renegotiates_the_stream_rate() {
        let sink = sink();
        assert_eq!(sink.rate(), ring::DEFAULT_RATE);

        sink.set_format(44_100).expect("renegotiate to 44.1 kHz");
        assert_eq!(sink.rate(), 44_100);
        assert_eq!(
            sink.ring().usable_frames(),
            (ring::RING_FRAMES as u64 * 44_100) / u64::from(ring::MAX_RATE)
        );

        let epoch = sink.epoch();
        let pcm = vec![0.0f32; 44_100 * ring::CHANNELS / 2];
        sink.write(&pcm, epoch);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(3) {
            if sink.played_frames() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!sink.failed(), "rebuilt stream reported an error");
        assert!(
            sink.played_frames() > 0,
            "rebuilt stream consumed no frames"
        );

        sink.set_format(96_000).expect("renegotiate to 96 kHz");
        assert_eq!(sink.rate(), 96_000);
        assert!(!sink.failed(), "second rebuild reported an error");
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn paused_sink_does_not_advance_played_frames() {
        let sink = Sink::new(Arc::new(Shared::new())).expect("PipeWire sink");
        let epoch = sink.epoch();
        let pcm = vec![0.0f32; 48_000 * 2 * 2];
        sink.write(&pcm, epoch);
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(2) && sink.played_frames() == 0 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let before = sink.played_frames();
        assert!(before > 0, "sink did not start: {:?}", sink.ring().failed());
        sink.set_paused(true);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after = sink.played_frames();
        eprintln!("paused sink played {before} -> {after}");
        // The pause command intentionally consumes the 20 ms fade-out tail
        // before silencing the stream; anything beyond that would be a real
        // paused-clock advance.
        let fade_frames = 48_000 / 1000 * 20;
        assert!(
            after.saturating_sub(before) <= fade_frames + 480,
            "sink advanced {} frames while paused (fade budget {fade_frames})",
            after.saturating_sub(before)
        );
    }
}
