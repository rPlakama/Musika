//! Rust PipeWire output sink.
//!
//! A dedicated thread owns every `!Send` PipeWire object and runs the main
//! loop. The decoder thread writes PCM into [`Ring`]; the RT process callback
//! drains it into the negotiated S16LE/stereo stream at the current pipeline
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

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
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
    Shutdown,
}

/// User data owned by the local stream listener: the PCM ring plus the shared
/// transport state the RT callback reads (boundary frame, engine eventfd).
struct RtContext {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
}

/// One connected PipeWire stream and its local listener.
///
/// Field order matters: `listener` unregisters its hook when dropped, so it
/// must be dropped while `stream` is still alive.
struct ActiveStream<'c> {
    listener: pw::stream::StreamListener<RtContext>,
    stream: pw::stream::StreamBox<'c>,
}

pub(crate) struct Sink {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    commands: pw::channel::Sender<SinkCommand>,
    thread: Option<JoinHandle<()>>,
}

impl Sink {
    pub(crate) fn new(shared: Arc<Shared>) -> Result<Self, String> {
        let ring = Arc::new(Ring::new()?);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (command_tx, command_rx) = pw::channel::channel::<SinkCommand>();
        let thread_ring = Arc::clone(&ring);
        let thread_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("musika-pipewire".to_string())
            .spawn(move || sink_thread(thread_ring, thread_shared, command_rx, ready_tx))
            .map_err(|error| format!("cannot start PipeWire thread: {error}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                ring,
                shared,
                commands: command_tx,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(error) => {
                let _ = thread.join();
                Err(format!("PipeWire sink did not start: {error}"))
            }
        }
    }

    pub(crate) fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Descriptor the engine polls for boundary/stream-state wakeups.
    pub(crate) fn wake_fd(&self) -> libc::c_int {
        self.shared.wake_fd()
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.ring.epoch()
    }

    /// Current PCM pipeline rate (Hz) shared by the ring and the stream.
    pub(crate) fn rate(&self) -> u32 {
        self.ring.rate()
    }

    /// Renegotiates the PipeWire stream to S16LE/stereo at `rate` (Hz).
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

    pub(crate) fn write(&self, pcm: &[i16], epoch: u64) {
        self.ring.write(pcm, epoch);
    }

    pub(crate) fn set_paused(&self, paused: bool) {
        self.ring.set_paused(paused);
        let _ = self.commands.send(SinkCommand::SetPaused(paused));
    }

    pub(crate) fn set_muted(&self, muted: bool) {
        self.ring.set_muted(muted);
        let _ = self.commands.send(SinkCommand::SetMuted(muted));
    }

    /// `volume` is the linear 0..=1 software gain applied by the RT callback.
    /// The same value is pushed to the system mixer on the PipeWire loop
    /// thread; when that succeeds the software gain is set back to unity so the
    /// two paths cannot attenuate the signal twice.
    pub(crate) fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        self.ring.set_volume(volume);
        let _ = self.commands.send(SinkCommand::SetVolume(volume as f32));
    }

    pub(crate) fn flush(&self) {
        self.ring.flush();
    }

    pub(crate) fn reset(&self) {
        self.ring.reset();
    }

    pub(crate) fn close_writes(&self) {
        self.ring.close_writes();
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.ring.played_frames()
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        self.ring.buffered_frames()
    }

    pub(crate) fn failed(&self) -> bool {
        self.ring.failed()
    }

    /// Last reported PipeWire stream state.
    pub(crate) fn stream_state(&self) -> SinkState {
        self.shared
            .sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .state
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.ring.close_writes();
        self.ring.wake();
        let _ = self.commands.send(SinkCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn sink_thread(
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    commands: pw::channel::Receiver<SinkCommand>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let result = run_sink(ring, shared, commands, ready.clone());
    if let Err(error) = result {
        let _ = ready.send(Err(error));
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
fn apply_volume(stream: &pw::stream::Stream, ring: &Ring, volume: f32) -> bool {
    let applied = stream
        .set_control(spa_sys::SPA_PROP_channelVolumes, &[volume, volume])
        .is_ok();
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
fn apply_mute(stream: &pw::stream::Stream, ring: &Ring, muted: bool) -> bool {
    let applied = stream
        .set_control(spa_sys::SPA_PROP_softMute, &[if muted { 1.0 } else { 0.0 }])
        .is_ok();
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
) -> Result<ActiveStream<'c>, String> {
    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Musika",
    };
    let stream = pw::stream::StreamBox::new(core, "Musika", props)
        .map_err(|error| format!("cannot create PipeWire stream: {error}"))?;

    let listener_ctx = RtContext {
        ring: Arc::clone(ring),
        shared: Arc::clone(shared),
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
    audio_info.set_format(spa::param::audio::AudioFormat::S16LE);
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

    Ok(ActiveStream { listener, stream })
}

/// Rebuilds the stream for a new pipeline rate and swaps it into `active`.
///
/// The ring rate (and its rate-scaled usable limit) is updated first, so a
/// producer that wakes up between the rebuild steps still sees the new limit;
/// the old stream is only dropped once the replacement is connected.
fn switch_format<'c>(
    active: &mut ActiveStream<'c>,
    core: &'c pw::core::Core,
    ring: &Arc<Ring>,
    shared: &Arc<Shared>,
    rate: u32,
) -> Result<(), String> {
    let rate = rate.clamp(1, ring::MAX_RATE);
    if rate == ring.rate() {
        return Ok(());
    }
    let previous_rate = ring.set_rate(rate);
    // Frames queued at the old rate must not be presented as if they were
    // produced at the new rate. The producer blocks on the flush until the
    // rebuilt stream's first process callback services it.
    ring.flush();
    match build_stream(core, ring, shared, rate) {
        Ok(next) => {
            ring.set_failed(false);
            shared.set_sink_state(SinkState::Connecting, None);
            if let Err(error) = next.stream.set_active(!ring.paused()) {
                crate::debug_log!("pipewire: cannot restore stream active state: {error}");
            }
            *active = next;
            Ok(())
        }
        Err(error) => {
            // Keep the old stream usable: restore the ring's previous rate.
            ring.set_rate(previous_rate);
            ring.flush();
            Err(error)
        }
    }
}

fn run_sink(
    ring: Arc<Ring>,
    shared: Arc<Shared>,
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

    let mut active = build_stream(&core, &ring, &shared, ring.rate())?;
    let mut volume = 1.0f32;
    let mut muted = false;
    // The freshly built node starts at unity; re-apply the mixer controls once
    // it reaches Paused/Streaming (a `set_control` before negotiation can be
    // refused even though the software path already carries the right gain).
    let mut controls_dirty = true;
    let _ = ready.send(Ok(()));

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
                    // A failure surfaces asynchronously as a stream-state
                    // error; the engine then recovers the sink.
                    if let Err(error) = active.stream.set_active(!paused) {
                        crate::debug_log!("pipewire: cannot set stream active state: {error}");
                    }
                }
                SinkCommand::SetVolume(value) => {
                    volume = value;
                    apply_volume(&active.stream, &ring, volume);
                }
                SinkCommand::SetMuted(value) => {
                    muted = value;
                    apply_mute(&active.stream, &ring, muted);
                }
                SinkCommand::SetFormat { rate, ack } => {
                    let result = switch_format(&mut active, &core, &ring, &shared, rate);
                    if result.is_ok() {
                        // Replay the mixer state on the rebuilt node.
                        let volume_applied = apply_volume(&active.stream, &ring, volume);
                        let mute_applied = apply_mute(&active.stream, &ring, muted);
                        controls_dirty = !(volume_applied && mute_applied);
                    }
                    if ack.send(result).is_err() {
                        crate::debug_log!("pipewire: format-change ack receiver was dropped");
                    }
                }
                SinkCommand::Shutdown => running = false,
            }
        }
        if running
            && controls_dirty
            && matches!(
                active.stream.state(),
                pw::stream::StreamState::Paused | pw::stream::StreamState::Streaming
            )
        {
            let volume_applied = apply_volume(&active.stream, &ring, volume);
            let mute_applied = apply_mute(&active.stream, &ring, muted);
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
    let max_frames = bytes.len() / (ring::CHANNELS * std::mem::size_of::<i16>());
    let frames = if requested > 0 {
        requested.min(max_frames)
    } else {
        max_frames
    };
    if frames == 0 {
        return;
    }
    let ptr = bytes.as_mut_ptr() as *mut i16;
    if (ptr as usize) % std::mem::align_of::<i16>() != 0 {
        return;
    }
    // SAFETY: the PipeWire buffer is at least `max_frames * 2` samples long and
    // is writable for the duration of the process callback.
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, frames * ring::CHANNELS) };
    let ring = &ctx.ring;
    let paused = ring.paused();
    // A deactivated PipeWire stream can still get a final/settling process
    // callback. Do not consume or advance the clock while paused; service a
    // pending reset/flush with a zero-frame read so the producer can wake.
    if paused {
        let _ = ring.read_into(out, 0);
        out.fill(0);
    } else {
        let written = ring.read_into(out, frames);
        out[written * ring::CHANNELS..].fill(0);
        ring::apply_gain(out, ring.muted(), ring.volume_milli(), frames);
    }
    // Gapless boundary as an event: the armed boundary frame lives in the same
    // absolute frame space as `played`. Wake the engine; it consumes the
    // boundary and disarms it, so the write happens at most a handful of times.
    if ctx.shared.boundary_active.load(Ordering::Acquire)
        && ring.played_frames() >= ctx.shared.boundary_frame.load(Ordering::Acquire)
    {
        ctx.shared.signal();
    }
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = (ring::CHANNELS * std::mem::size_of::<i16>()) as i32;
    *chunk.size_mut() = (frames * ring::CHANNELS * std::mem::size_of::<i16>()) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink() -> Sink {
        Sink::new(Arc::new(Shared::new())).expect("PipeWire sink")
    }

    #[test]
    #[ignore = "requires a running PipeWire session"]
    fn sink_starts_and_consumes_pcm() {
        let sink = sink();
        sink.set_volume(0.25);
        let epoch = sink.epoch();
        let pcm = vec![0i16; 4_800 * ring::CHANNELS];
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
        let pcm = vec![0i16; 44_100 * ring::CHANNELS / 2];
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
        let pcm = vec![0i16; 48_000 * 2 * 2];
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
        assert!(
            after.saturating_sub(before) < 480,
            "sink advanced {} frames while paused",
            after.saturating_sub(before)
        );
    }
}
