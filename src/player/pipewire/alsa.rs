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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleFormat {
    Float,
    S32,
    S24_3,
    S16,
}

impl SampleFormat {
    const fn bytes_per_sample(self) -> usize {
        match self {
            Self::Float | Self::S32 => 4,
            Self::S24_3 => 3,
            Self::S16 => 2,
        }
    }
}

const CANDIDATE_FORMATS: [(Format, SampleFormat); 4] = [
    (Format::FloatLE, SampleFormat::Float),
    (Format::S32LE, SampleFormat::S32),
    (Format::S243LE, SampleFormat::S24_3),
    (Format::S16LE, SampleFormat::S16),
];

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
                if let Err(error) =
                    run_sink(thread_ring, thread_shared, command_rx, &device, ready_tx)
                {
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
            Ok(Err(error)) => {
                _ = command_tx.send(AlsaCommand::Shutdown);
                _ = thread.join();
                Err(error)
            }
            Err(_) => {
                _ = command_tx.send(AlsaCommand::Shutdown);
                _ = thread.join();
                Err("ALSA sink did not become ready".to_string())
            }
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
        self.ring.request_fade(!paused);
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
        // A flush discards the frames the armed gapless boundary was measured
        // in; keep the marker from firing in the new timeline (P-01).
        self.shared.disarm_boundary();
        self.shared.bump_clock_epoch();
        self.ring.flush();
    }
    fn reset(&self) {
        self.shared.bump_clock_epoch();
        self.ring.reset();
    }
    fn close_writes(&self) {
        self.ring.close_writes();
    }
    fn played_frames(&self) -> u64 {
        self.ring.played_frames()
    }
    fn read_frames(&self) -> u64 {
        self.ring.read_frames()
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
    let (opened_pcm, mut sample_format) = match open_pcm(device, ring.rate()) {
        Ok(opened) => opened,
        Err(error) => {
            _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
    let mut pcm = Some(opened_pcm);
    ring.set_failed(false);
    shared.set_sink_state(SinkState::Streaming, None);
    shared.signal();
    ready
        .send(Ok(()))
        .map_err(|_| "ALSA ready receiver was dropped".to_string())?;

    let mut fade = FadeState::new();
    let mut buffer = vec![0.0f32; PERIOD_FRAMES * ring::CHANNELS];
    let mut raw_scratch = vec![0u8; PERIOD_FRAMES * ring::CHANNELS * 4];
    let mut was_paused = false;
    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                AlsaCommand::SetFormat { rate, ack } => {
                    let rate = rate.clamp(1, ring::MAX_RATE);
                    // Explicitly drop the old handle before reopening so an
                    // exclusive `hw:` device never fails with `EBUSY`.
                    pcm = None;
                    let result = open_pcm(device, rate).map(|(opened, next_format)| {
                        pcm = Some(opened);
                        sample_format = next_format;
                        ring.set_rate(rate);
                        shared.disarm_boundary();
                        shared.bump_clock_epoch();
                        ring.flush();
                        ring.set_failed(false);
                        shared.set_sink_state(SinkState::Streaming, None);
                        buffer.resize(PERIOD_FRAMES * ring::CHANNELS, 0.0);
                    });
                    if ack.send(result).is_err() {
                        crate::debug_log!("alsa: format-change ack receiver was dropped");
                    }
                }
                AlsaCommand::Shutdown => {
                    if let Some(active_pcm) = pcm.as_ref()
                        && let Err(error) = active_pcm.drain()
                    {
                        crate::debug_log!("alsa: drain failed: {error}");
                    }
                    return Ok(());
                }
            }
        }

        let Some(active_pcm) = pcm.as_ref() else {
            thread::sleep(Duration::from_millis(10));
            continue;
        };

        if ring.paused() || ring.buffering() {
            // Service pending reset/flush flags while paused/buffering so
            // `Ring::write` never deadlocks on a seek or load.
            _ = ring.read_into(&mut buffer, 0);
            if !was_paused {
                if let Err(error) = active_pcm.pause(true) {
                    crate::debug_log!("alsa: pause failed: {error}");
                }
                was_paused = true;
                shared.set_sink_state(SinkState::Paused, None);
            }
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        if was_paused {
            if let Err(error) = active_pcm.pause(false) {
                crate::debug_log!("alsa: resume failed, falling back to prepare: {error}");
                if let Err(prepare_error) = active_pcm.prepare() {
                    crate::debug_log!("alsa: prepare failed: {prepare_error}");
                }
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
            if shared.can_rebuffer() && ring.enter_buffering() {
                shared.signal();
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        buffer[frames * ring::CHANNELS..].fill(0.0);
        ring::apply_gain(&mut buffer, ring.muted(), ring.volume_milli(), frames);
        fade.apply(&mut buffer, frames);

        if let Err(error) =
            write_period(active_pcm, sample_format, &buffer, frames, &mut raw_scratch)
        {
            fail(&ring, &shared, error.clone());
            return Err(error);
        }
        if shared.boundary_reached(ring.epoch(), ring.read_frames()) {
            shared.signal();
        }
    }
}

fn clamp_unit_f32(sample: f32) -> f32 {
    if sample.is_nan() {
        0.0
    } else {
        sample.clamp(-1.0, 1.0)
    }
}

fn f32_to_s32_le(sample: f32) -> i32 {
    let clamped = f64::from(clamp_unit_f32(sample));
    (clamped * 2_147_483_648.0)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn f32_to_s24_3le(sample: f32) -> [u8; 3] {
    let clamped = f64::from(clamp_unit_f32(sample));
    let quantized = (clamped * 8_388_608.0)
        .round()
        .clamp(-8_388_608.0, 8_388_607.0) as i32;
    let le = quantized.to_le_bytes();
    [le[0], le[1], le[2]]
}

fn f32_to_s16_le(sample: f32) -> i16 {
    let clamped = f64::from(clamp_unit_f32(sample));
    (clamped * 32_768.0)
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

fn encode_pcm_bytes(format: SampleFormat, samples: &[f32], out: &mut [u8]) {
    match format {
        SampleFormat::Float => {
            for (sample, chunk) in samples.iter().zip(out.as_chunks_mut::<4>().0) {
                *chunk = clamp_unit_f32(*sample).to_le_bytes();
            }
        }
        SampleFormat::S32 => {
            for (sample, chunk) in samples.iter().zip(out.as_chunks_mut::<4>().0) {
                *chunk = f32_to_s32_le(*sample).to_le_bytes();
            }
        }
        SampleFormat::S24_3 => {
            for (sample, chunk) in samples.iter().zip(out.as_chunks_mut::<3>().0) {
                *chunk = f32_to_s24_3le(*sample);
            }
        }
        SampleFormat::S16 => {
            for (sample, chunk) in samples.iter().zip(out.as_chunks_mut::<2>().0) {
                *chunk = f32_to_s16_le(*sample).to_le_bytes();
            }
        }
    }
}

fn recover_pcm_xrun(pcm: &PCM, errno: i32) -> Result<(), alsa::Error> {
    if errno == libc::ESTRPIPE {
        loop {
            match pcm.resume() {
                Ok(()) => return Ok(()),
                Err(error) if error.errno() == libc::EAGAIN => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return pcm.prepare(),
            }
        }
    } else {
        pcm.prepare()
    }
}

fn write_period(
    pcm: &PCM,
    format: SampleFormat,
    samples: &[f32],
    frames: usize,
    scratch: &mut [u8],
) -> Result<(), String> {
    let frame_bytes = format.bytes_per_sample() * ring::CHANNELS;
    let total_bytes = frames * frame_bytes;
    encode_pcm_bytes(
        format,
        &samples[..frames * ring::CHANNELS],
        &mut scratch[..total_bytes],
    );
    let io = pcm.io_bytes();
    let mut written_frames = 0usize;
    while written_frames < frames {
        let byte_offset = written_frames * frame_bytes;
        match io.writei(&scratch[byte_offset..total_bytes]) {
            Ok(0) => {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(written) => {
                written_frames = (written_frames + written).min(frames);
            }
            Err(error) => {
                let errno = error.errno();
                if errno == libc::EINTR || errno == libc::EAGAIN {
                    continue;
                }
                if errno == libc::EPIPE || errno == libc::ESTRPIPE {
                    if let Err(recover_error) = recover_pcm_xrun(pcm, errno) {
                        crate::debug_log!(
                            "alsa: recovery after errno {errno} failed: {recover_error}"
                        );
                        return Err(format!("ALSA recovery: {recover_error}"));
                    }
                    continue;
                }
                return Err(format!("ALSA write: {error}"));
            }
        }
    }
    Ok(())
}

fn open_pcm(device: &str, rate: u32) -> Result<(PCM, SampleFormat), String> {
    let pcm = PCM::new(device, Direction::Playback, false)
        .map_err(|error| format!("ALSA open '{device}': {error}"))?;
    let sample_format = {
        let params = HwParams::any(&pcm).map_err(|error| format!("ALSA hw_params: {error}"))?;
        params
            .set_access(Access::RWInterleaved)
            .map_err(|error| format!("ALSA access: {error}"))?;
        let mut negotiated = None;
        let mut last_error = None;
        for (alsa_format, candidate) in CANDIDATE_FORMATS {
            match params.set_format(alsa_format) {
                Ok(()) => {
                    negotiated = Some(candidate);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let Some(sample_format) = negotiated else {
            return Err(format!(
                "ALSA format negotiation failed for '{device}': {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "no supported format".to_string())
            ));
        };
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
        sample_format
    };
    pcm.prepare()
        .map_err(|error| format!("ALSA prepare: {error}"))?;
    Ok((pcm, sample_format))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_to_s16_le_converts_and_clamps_past_unity() {
        assert_eq!(f32_to_s16_le(0.0), 0);
        assert_eq!(f32_to_s16_le(1.0), i16::MAX);
        assert_eq!(f32_to_s16_le(1.75), i16::MAX);
        assert_eq!(f32_to_s16_le(-1.0), i16::MIN);
        assert_eq!(f32_to_s16_le(-2.5), i16::MIN);
        assert_eq!(f32_to_s16_le(0.5), 16_384);
        assert_eq!(f32_to_s16_le(-0.5), -16_384);
        assert_eq!(f32_to_s16_le(f32::NAN), 0);

        let samples = [-1.5f32, -1.0, 0.0, 1.0, 1.5];
        let mut raw = vec![0u8; samples.len() * 2];
        encode_pcm_bytes(SampleFormat::S16, &samples, &mut raw);
        let decoded: Vec<i16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(i16::from_le_bytes)
            .collect();
        assert_eq!(decoded, vec![i16::MIN, i16::MIN, 0, i16::MAX, i16::MAX]);
    }

    #[test]
    fn f32_to_s24_3le_converts_and_clamps_past_unity() {
        assert_eq!(f32_to_s24_3le(0.0), [0x00, 0x00, 0x00]);
        assert_eq!(f32_to_s24_3le(1.0), [0xFF, 0xFF, 0x7F]);
        assert_eq!(f32_to_s24_3le(1.25), [0xFF, 0xFF, 0x7F]);
        assert_eq!(f32_to_s24_3le(-1.0), [0x00, 0x00, 0x80]);
        assert_eq!(f32_to_s24_3le(-1.25), [0x00, 0x00, 0x80]);
        assert_eq!(f32_to_s24_3le(f32::NAN), [0x00, 0x00, 0x00]);

        let samples = [-1.2f32, 0.0, 1.2];
        let mut raw = vec![0u8; samples.len() * 3];
        encode_pcm_bytes(SampleFormat::S24_3, &samples, &mut raw);
        assert_eq!(&raw[0..3], &[0x00, 0x00, 0x80]);
        assert_eq!(&raw[3..6], &[0x00, 0x00, 0x00]);
        assert_eq!(&raw[6..9], &[0xFF, 0xFF, 0x7F]);
    }

    #[test]
    fn f32_to_s32_le_converts_and_clamps_past_unity() {
        assert_eq!(f32_to_s32_le(0.0), 0);
        assert_eq!(f32_to_s32_le(1.0), i32::MAX);
        assert_eq!(f32_to_s32_le(1.5), i32::MAX);
        assert_eq!(f32_to_s32_le(-1.0), i32::MIN);
        assert_eq!(f32_to_s32_le(-1.5), i32::MIN);
        assert_eq!(f32_to_s32_le(0.5), 1_073_741_824);
        assert_eq!(f32_to_s32_le(-0.5), -1_073_741_824);
        assert_eq!(f32_to_s32_le(f32::NAN), 0);

        let samples = [-1.5f32, -1.0, 0.0, 1.0, 1.5];
        let mut raw = vec![0u8; samples.len() * 4];
        encode_pcm_bytes(SampleFormat::S32, &samples, &mut raw);
        let decoded: Vec<i32> = raw
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(i32::from_le_bytes)
            .collect();
        assert_eq!(decoded, vec![i32::MIN, i32::MIN, 0, i32::MAX, i32::MAX]);
    }
}
