//! Optional direct ALSA sink.
//!
//! This is the "exclusive/bit-perfect" exit from PipeWire: a blocking
//! `hw:...` PCM configured for packed F32 stereo at the pipeline rate. It
//! shares the same lock-free ring and shared status as the PipeWire sink, so
//! the decoder/worker do not know which backend is active.
//!
//! The backend is selected before the engine starts (`output_backend` in
//! playback settings or `MUSIKA_OUTPUT_BACKEND=alsa`); changing it requires a
//! restart. If ALSA cannot open/configure the PCM (including a rate the device
//! does not support), `Native::new` logs the failure and falls back to
//! PipeWire rather than leaving the app silent.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};

use super::ring::{self, Ring};
use super::sink::{FadeState, PcmSink};
use super::status::{Shared, SinkState};
use crate::player::error::ErrorKind;
use crate::player::{ErrorStage, PlaybackError};

const PERIOD_FRAMES: usize = 1024;
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

enum AlsaCommand {
    SetFormat {
        rate: u32,
        ack: mpsc::SyncSender<Result<(), String>>,
    },
    Shutdown,
}

pub(crate) struct AlsaSink {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    commands: mpsc::Sender<AlsaCommand>,
    thread: Option<JoinHandle<()>>,
}

impl AlsaSink {
    pub(crate) fn new(shared: Arc<Shared>) -> Result<Self, String> {
        let ring = Arc::new(Ring::new()?);
        let device = crate::playback_settings::current().alsa_device;
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (command_tx, command_rx) = mpsc::channel();
        let thread_ring = Arc::clone(&ring);
        let thread_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("musika-alsa".to_string())
            .spawn(move || {
                if let Err(error) = run_sink(thread_ring, thread_shared, command_rx, &device, ready_tx) {
                    crate::debug_log!("alsa: sink stopped: {error}");
                }
            })
            .map_err(|error| format!("cannot start ALSA sink thread: {error}"))?;
        match ready_rx.recv_timeout(ACK_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                ring,
                shared,
                commands: command_tx,
                thread: Some(thread),
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("ALSA sink did not become ready".to_string()),
        }
    }

    pub(crate) fn set_format(&self, rate: u32) -> Result<(), String> {
        let (ack, ack_rx) = mpsc::sync_channel(1);
        self.commands
            .send(AlsaCommand::SetFormat { rate, ack })
            .map_err(|_| "ALSA sink is not running".to_string())?;
        match ack_rx.recv_timeout(ACK_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err("ALSA sink did not acknowledge the format change".to_string()),
        }
    }
}

impl Drop for AlsaSink {
    fn drop(&mut self) {
        _ = self.commands.send(AlsaCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            _ = thread.join();
        }
    }
}

impl PcmSink for AlsaSink {
    fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }
    fn epoch(&self) -> u64 {
        self.ring.epoch()
    }
    fn rate(&self) -> u32 {
        self.ring.rate()
    }
    fn set_format(&self, rate: u32) -> Result<(), String> {
        AlsaSink::set_format(self, rate)
    }
    fn set_target(&self, _target: Option<String>) -> Result<(), String> {
        // ALSA's device comes from `alsa_device`; PipeWire routing does not
        // apply here.
        Ok(())
    }
    fn set_exclusive(&self, _exclusive: bool) -> Result<(), String> {
        // A direct `hw:` PCM is already exclusive; a `default`/`dmix` device
        // is chosen by the user's ALSA configuration.
        Ok(())
    }
    fn write(&self, pcm: &[f32], epoch: u64) {
        self.ring.write(pcm, epoch);
    }
    fn set_paused(&self, paused: bool) {
        self.ring.set_paused(paused);
    }
    fn request_fade_in(&self) {
        self.ring.request_fade(true);
    }
    fn request_fade_out(&self) {
        self.ring.request_fade(false);
    }
    fn set_muted(&self, muted: bool) {
        self.ring.set_muted(muted);
    }
    fn set_volume(&self, volume: f64) {
        self.ring.set_volume(volume);
    }
    fn flush(&self) {
        self.ring.flush();
    }
    fn reset(&self) {
        self.ring.reset();
    }
    fn close_writes(&self) {
        self.ring.close_writes();
    }
    fn played_frames(&self) -> u64 {
        self.ring.played_frames()
    }
    fn buffered_frames(&self) -> u64 {
        self.ring.buffered_frames()
    }
    fn underruns(&self) -> u64 {
        self.ring.underruns()
    }
    fn failed(&self) -> bool {
        self.ring.failed()
    }
}

fn run_sink(
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<AlsaCommand>,
    device: &str,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    let mut pcm = open_pcm(device, ring.rate())?;
    ring.set_failed(false);
    shared.set_sink_state(SinkState::Streaming, None);
    shared.signal();
    ready
        .send(Ok(()))
        .map_err(|_| "ALSA ready receiver was dropped".to_string())?;

    let mut fade = FadeState::new();
    let mut buffer = vec![0.0f32; PERIOD_FRAMES * ring::CHANNELS];
    let mut was_paused = false;
    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                AlsaCommand::SetFormat { rate, ack } => {
                    let result = (|| {
                        pcm.prepare().map_err(|error| format!("ALSA prepare: {error}"))?;
                        pcm = open_pcm(device, rate)?;
                        Ok(())
                    })();
                    if result.is_ok() {
                        ring.set_failed(false);
                        shared.set_sink_state(SinkState::Streaming, None);
                        buffer.resize(PERIOD_FRAMES * ring::CHANNELS, 0.0);
                    }
                    if ack.send(result).is_err() {
                        crate::debug_log!("alsa: format-change ack receiver was dropped");
                    }
                }
                AlsaCommand::Shutdown => {
                    if let Err(error) = pcm.drain() {
                        crate::debug_log!("alsa: drain failed: {error}");
                    }
                    return Ok(());
                }
            }
        }

        if ring.closed() {
            return Ok(());
        }
        if ring.paused() {
            if !was_paused {
                if let Err(error) = pcm.pause(true) {
                    crate::debug_log!("alsa: pause failed: {error}");
                }
                was_paused = true;
                shared.set_sink_state(SinkState::Paused, None);
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        if was_paused {
            if let Err(error) = pcm.pause(false) {
                crate::debug_log!("alsa: resume failed: {error}");
            }
            if let Err(error) = pcm.prepare() {
                crate::debug_log!("alsa: prepare failed: {error}");
            }
            was_paused = false;
            shared.set_sink_state(SinkState::Streaming, None);
        }

        match ring.take_fade_request() {
            1 => fade.start(1.0, ring.rate()),
            2 => fade.start(0.0, ring.rate()),
            _ => {}
        }
        let frames = ring.read_into(&mut buffer, PERIOD_FRAMES);
        if frames == 0 {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        buffer[frames * ring::CHANNELS..].fill(0.0);
        ring::apply_gain(&mut buffer, ring.muted(), ring.volume_milli(), frames);
        fade.apply(&mut buffer, frames);

        let io = match pcm.io_f32() {
            Ok(io) => io,
            Err(error) => {
                fail(&ring, &shared, format!("ALSA io: {error}"));
                return Err(error.to_string());
            }
        };
        if let Err(error) = io.writei(&buffer[..frames * ring::CHANNELS]) {
            if error.errno() == libc::EPIPE {
                if let Err(error) = pcm.prepare() {
                    crate::debug_log!("alsa: prepare after EPIPE failed: {error}");
                }
                continue;
            }
            fail(&ring, &shared, format!("ALSA write: {error}"));
            return Err(error.to_string());
        }
        if shared.boundary_active.load(Ordering::Acquire)
            && ring.played_frames() >= shared.boundary_frame.load(Ordering::Acquire)
        {
            shared.signal();
        }
        shared.signal();
    }
}

fn open_pcm(device: &str, rate: u32) -> Result<PCM, String> {
    let pcm = PCM::new(device, Direction::Playback, false)
        .map_err(|error| format!("ALSA open '{device}': {error}"))?;
    {
        let params = HwParams::any(&pcm).map_err(|error| format!("ALSA hw_params: {error}"))?;
        params
            .set_access(Access::RWInterleaved)
            .map_err(|error| format!("ALSA access: {error}"))?;
        params
            .set_format(Format::FloatLE)
            .map_err(|error| format!("ALSA FloatLE is not supported: {error}"))?;
        params
            .set_channels(ring::CHANNELS as u32)
            .map_err(|error| format!("ALSA channels: {error}"))?;
        let actual_rate = params
            .set_rate_near(rate, ValueOr::Nearest)
            .map_err(|error| format!("ALSA rate {rate}: {error}"))?;
        if actual_rate != rate {
            return Err(format!(
                "ALSA device chose {actual_rate} Hz instead of {rate} Hz"
            ));
        }
        params
            .set_period_size_near(PERIOD_FRAMES as alsa::pcm::Frames, ValueOr::Nearest)
            .map_err(|error| format!("ALSA period: {error}"))?;
        params
            .set_buffer_size_near((PERIOD_FRAMES * 4) as alsa::pcm::Frames)
            .map_err(|error| format!("ALSA buffer: {error}"))?;
        pcm.hw_params(&params)
            .map_err(|error| format!("ALSA hw_params apply: {error}"))?;
    }
    pcm.prepare().map_err(|error| format!("ALSA prepare: {error}"))?;
    Ok(pcm)
}

fn fail(ring: &Ring, shared: &Shared, message: String) {
    ring.set_failed(true);
    shared.set_sink_state(SinkState::Error, Some(message.clone()));
    shared.lock().failed = Some(PlaybackError::new(
        ErrorKind::Device,
        ErrorStage::Sink,
        message,
    ));
    shared.signal();
}
