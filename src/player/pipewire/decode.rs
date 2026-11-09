// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! FFmpeg decoder used by the Rust-owned PipeWire transport.
//!
//! The decoder presents a single fixed output format: interleaved S16 stereo
//! at 48 kHz. FFmpeg/swresample owns demuxing, codec selection, down/up-mix
//! and sample-rate conversion, so the transport worker only ever sees PCM
//! frames it can hand straight to the sink.
//!
//! Both local paths (`file:` URLs included) and network URLs are opened by
//! libavformat. A deliberately small read-ahead/probe configuration keeps
//! HTTP streams responsive; an exotic input that rejects those options is
//! retried once with FFmpeg defaults.

use std::ffi::{CStr, c_char, c_int};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use ffmpeg::codec::{self, threading};
use ffmpeg::format::{self, input_with_interrupt_and_dictionary};
use ffmpeg::frame;
use ffmpeg::packet::Mut as _;
use ffmpeg::software::resampling;
use ffmpeg::util::format::Sample;
use ffmpeg::{ChannelLayout, Dictionary, Error, Packet, Rational};
use ffmpeg_next as ffmpeg;

/// Output sample rate expected by the PipeWire sink.
const TARGET_RATE: u32 = 48_000;
/// Output channel count expected by the PipeWire sink.
const TARGET_CHANNELS: i32 = 2;
/// Packed (interleaved) S16, matching `TARGET_CHANNELS`/`TARGET_RATE`.
const TARGET_SAMPLE: Sample = Sample::I16(ffmpeg::format::sample::Type::Packed);

/// Network read-ahead. The transport worker streams a track instead of
/// buffering it whole, so AVIO gets a bounded 32 MiB window.
const STREAM_BUFFER_SIZE: &str = "33554432";
/// Bytes inspected before stream selection.
const STREAM_PROBE_SIZE: &str = "1048576";
/// Microseconds of stream analysis during open.
const STREAM_ANALYZE_DURATION: &str = "1000000";

static INIT: Once = Once::new();

/// Process-wide FFmpeg setup. `init` registers error strings and the network
/// protocols; log output is reduced to errors because the player surfaces
/// failures through `Result` instead of the terminal.
fn initialize() {
    INIT.call_once(|| {
        let _ = ffmpeg::init();
        ffmpeg::format::network::init();
        ffmpeg::log::set_level(ffmpeg::log::Level::Error);
    });
}

fn stopped() -> String {
    "decoder: playback stopped".to_string()
}

/// Render an FFmpeg error with its human-readable text and numeric code.
fn error_text(error: Error) -> String {
    let code: c_int = error.into();
    let mut buffer = [0 as c_char; 256];
    let text = unsafe {
        if ffmpeg::ffi::av_strerror(code, buffer.as_mut_ptr(), buffer.len()) < 0 {
            format!("{error}")
        } else {
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        }
    };
    format!("{text} (FFmpeg {code})")
}

fn context_error(context: &str, error: Error) -> String {
    format!("{context}: {}", error_text(error))
}

fn is_eagain(error: Error) -> bool {
    matches!(error, Error::Other { errno } if errno == ffmpeg::error::EAGAIN)
}

/// Open an input with the fast HTTP/stream options, falling back once to
/// FFmpeg defaults. The interrupt callback latches on `stop`, so a stalled
/// network open or read aborts promptly and the open error is not retried.
fn open_input(url: &str, stop: &Arc<AtomicBool>) -> Result<format::context::Input, String> {
    if stop.load(Ordering::Relaxed) {
        return Err(stopped());
    }

    let mut options = Dictionary::new();
    options.set("buffer_size", STREAM_BUFFER_SIZE);
    options.set("probesize", STREAM_PROBE_SIZE);
    options.set("analyzeduration", STREAM_ANALYZE_DURATION);
    options.set("fflags", "+fastseek");

    let interrupt_stop = Arc::clone(stop);
    let first = input_with_interrupt_and_dictionary(
        url,
        move || interrupt_stop.load(Ordering::Relaxed),
        options,
    );

    match first {
        Ok(input) => Ok(input),
        Err(first_error) => {
            if stop.load(Ordering::Relaxed) {
                return Err(stopped());
            }

            let interrupt_stop = Arc::clone(stop);
            match input_with_interrupt_and_dictionary(
                url,
                move || interrupt_stop.load(Ordering::Relaxed),
                Dictionary::new(),
            ) {
                Ok(input) => Ok(input),
                Err(second_error) => {
                    if stop.load(Ordering::Relaxed) {
                        Err(stopped())
                    } else {
                        Err(format!(
                            "decoder: cannot open '{url}': {}; fallback open: {}",
                            error_text(first_error),
                            error_text(second_error)
                        ))
                    }
                }
            }
        }
    }
}

/// A demux/decode/resample pipeline producing interleaved S16 stereo 48 kHz.
///
/// `read` never allocates from Rust: packet, decode frame, resample frame and
/// pending PCM all live in this struct, and the caller supplies the output.
pub(crate) struct Decoder {
    input: format::context::Input,
    audio: ffmpeg::decoder::Audio,
    packet: Packet,
    decoded: frame::Audio,
    converted: frame::Audio,
    converted_capacity: usize,
    resampler: Option<resampling::Context>,
    stream_index: usize,
    stream_time_base: Rational,
    duration: f64,
    seekable: bool,
    stop: Arc<AtomicBool>,
    /// Frames currently available in `converted`, and how many were consumed.
    pending_frames: usize,
    pending_offset: usize,
    /// A null packet was sent to the codec; only drain frames remain.
    drain_sent: bool,
    /// `receive_frame` reported EOF; resampler tail may still be pending.
    decoder_eof: bool,
    resampler_flushed: bool,
    /// Terminal state until `seek` clears it.
    eof: bool,
}

impl Decoder {
    pub(crate) fn open(url: &str, stop: Arc<AtomicBool>) -> Result<Self, String> {
        initialize();

        if url.is_empty() {
            return Err("decoder: empty media URL".to_string());
        }
        if url.as_bytes().contains(&0) {
            return Err("decoder: media URL contains a NUL byte".to_string());
        }
        if stop.load(Ordering::Relaxed) {
            return Err(stopped());
        }

        let input = open_input(url, &stop)?;

        let (stream_index, stream_time_base, duration, parameters) = {
            let stream = input
                .streams()
                .best(ffmpeg::media::Type::Audio)
                .ok_or_else(|| "decoder: input has no audio stream".to_string())?;
            let index = stream.index();
            let time_base = stream.time_base();

            let format_duration = input.duration();
            let mut duration = if format_duration > 0 {
                format_duration as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
            } else {
                0.0
            };
            if duration <= 0.0 {
                let stream_duration = stream.duration();
                if stream_duration > 0 && time_base.numerator() > 0 && time_base.denominator() > 0 {
                    duration = stream_duration as f64 * f64::from(time_base.numerator())
                        / f64::from(time_base.denominator());
                }
            }

            (index, time_base, duration.max(0.0), stream.parameters())
        };

        let mut codec_context = codec::Context::from_parameters(parameters).map_err(|error| {
            context_error("decoder: cannot apply audio codec parameters", error)
        })?;
        // Audio decode is cheap enough to keep on the player thread. More
        // importantly, this bounds FFmpeg's internal buffering and makes
        // seek/flush behaviour deterministic.
        codec_context.set_threading(threading::Config {
            kind: threading::Type::None,
            count: 1,
            ..threading::Config::default()
        });
        let audio = codec_context
            .decoder()
            .audio()
            .map_err(|error| context_error("decoder: cannot open audio decoder", error))?;

        let seekable = unsafe {
            let format_context = input.as_ptr();
            let io = (*format_context).pb;
            !io.is_null() && ((*io).seekable & ffmpeg::ffi::AVIO_SEEKABLE_NORMAL) != 0
        };

        Ok(Self {
            input,
            audio,
            packet: Packet::empty(),
            decoded: frame::Audio::empty(),
            converted: frame::Audio::empty(),
            converted_capacity: 0,
            resampler: None,
            stream_index,
            stream_time_base,
            duration,
            seekable,
            stop,
            pending_frames: 0,
            pending_offset: 0,
            drain_sent: false,
            decoder_eof: false,
            resampler_flushed: false,
            eof: false,
        })
    }

    pub(crate) fn duration(&self) -> f64 {
        self.duration
    }

    /// Byte size of the input when the protocol/container reports one,
    /// otherwise `-1` (network streams without Content-Length).
    pub(crate) fn stream_size(&self) -> i64 {
        self.input.io_size().unwrap_or(-1)
    }

    /// Seek to `seconds`, to the nearest earlier keyframe or demuxer
    /// position. Decoder, resampler and all pending PCM are reset.
    pub(crate) fn seek(&mut self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("decoder: invalid seek position".to_string());
        }
        if !self.seekable {
            return Err("decoder: stream is not seekable".to_string());
        }
        if self.stop.load(Ordering::Relaxed) {
            return Err(stopped());
        }

        let time_base = self.stream_time_base;
        if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
            return Err("decoder: stream has an invalid time base".to_string());
        }

        let target = (seconds * f64::from(time_base.denominator())
            / f64::from(time_base.numerator()))
        .round();
        let target = if target >= i64::MAX as f64 {
            i64::MAX
        } else {
            target as i64
        };

        let result = unsafe {
            ffmpeg::ffi::av_seek_frame(
                self.input.as_mut_ptr(),
                self.stream_index as c_int,
                target,
                ffmpeg::ffi::AVSEEK_FLAG_BACKWARD,
            )
        };
        if result < 0 {
            return Err(context_error("decoder: seek failed", Error::from(result)));
        }

        self.audio.flush();
        // Dropping the resampler is equivalent to swr_close + swr_init: no
        // stale filter history or buffered samples survive a seek.
        self.resampler = None;
        self.converted = frame::Audio::empty();
        self.converted_capacity = 0;
        self.pending_frames = 0;
        self.pending_offset = 0;
        self.drain_sent = false;
        self.decoder_eof = false;
        self.resampler_flushed = false;
        self.eof = false;
        self.unref_decoded();
        self.unref_packet();

        // A demuxer that reached EOF keeps a sticky AVIO latch. Seeking
        // clears the normal EOF state; clear an interrupt latch as well so a
        // later seek/read cannot inherit cancellation from an earlier stop.
        let _ = self.input.clear_eof();
        let _ = self.input.clear_interrupt();
        Ok(())
    }

    /// Fill `out` with interleaved S16 stereo 48 kHz PCM.
    ///
    /// At most `max_frames` stereo frames are written. Returns the number of
    /// frames written; `0` means end of stream. Partial resampler output is
    /// retained in the struct for the next call.
    pub(crate) fn read(&mut self, out: &mut [i16], max_frames: usize) -> Result<usize, String> {
        let max_frames = max_frames.min(out.len() / 2);
        if max_frames == 0 {
            return Ok(0);
        }
        if self.stop.load(Ordering::Relaxed) {
            return Err(stopped());
        }

        let mut frames_read = 0usize;
        while frames_read < max_frames {
            if self.pending_frames == 0 {
                if !self.decode_more()? {
                    break;
                }
            }

            let available = self.pending_frames.saturating_sub(self.pending_offset);
            if available == 0 {
                self.pending_frames = 0;
                self.pending_offset = 0;
                continue;
            }

            let take = available.min(max_frames - frames_read);
            let source_start = self.pending_offset * 2;
            let source_end = source_start + take * 2;
            let target_start = frames_read * 2;
            let target_end = target_start + take * 2;
            out[target_start..target_end]
                .copy_from_slice(&self.converted.plane::<i16>(0)[source_start..source_end]);

            self.pending_offset += take;
            frames_read += take;
            if self.pending_offset >= self.pending_frames {
                self.pending_frames = 0;
                self.pending_offset = 0;
            }

            if self.stop.load(Ordering::Relaxed) {
                return Err(stopped());
            }
        }

        Ok(frames_read)
    }

    pub(crate) fn is_seekable(&self) -> bool {
        self.seekable
    }

    /// Produce one resampled batch, or report terminal EOF.
    fn decode_more(&mut self) -> Result<bool, String> {
        loop {
            if self.eof {
                return Ok(false);
            }
            if self.stop.load(Ordering::Relaxed) {
                return Err(stopped());
            }

            if self.decoder_eof {
                if !self.resampler_flushed {
                    self.resampler_flushed = true;
                    let produced = self.flush_resampler()?;
                    if produced > 0 {
                        self.pending_frames = produced;
                        self.pending_offset = 0;
                        return Ok(true);
                    }
                }
                self.eof = true;
                return Ok(false);
            }

            match self.audio.receive_frame(&mut *self.decoded) {
                Ok(()) => {
                    if self.decoded.samples() == 0 {
                        self.unref_decoded();
                        continue;
                    }
                    let produced = self.resample_decoded();
                    self.unref_decoded();
                    let produced = produced?;
                    if produced > 0 {
                        self.pending_frames = produced;
                        self.pending_offset = 0;
                        return Ok(true);
                    }
                }
                Err(error) if is_eagain(error) => {
                    if self.drain_sent {
                        // A drained decoder should return EOF, not EAGAIN. Be
                        // defensive: stop decoding this input rather than
                        // spinning forever.
                        self.decoder_eof = true;
                        continue;
                    }
                    self.feed_decoder()?;
                }
                Err(Error::Eof) => {
                    self.decoder_eof = true;
                }
                Err(error) => {
                    return Err(context_error("decoder: audio decode failed", error));
                }
            }
        }
    }

    /// Read packets until one audio packet is accepted by the codec (or the
    /// demuxer EOF is signalled to the decoder).
    fn feed_decoder(&mut self) -> Result<(), String> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(stopped());
            }

            match self.packet.read(&mut self.input) {
                Ok(()) => {
                    if self.packet.stream() != self.stream_index {
                        self.unref_packet();
                        continue;
                    }
                    let sent = self.audio.send_packet(&self.packet);
                    self.unref_packet();
                    match sent {
                        Ok(()) => return Ok(()),
                        Err(error) if is_eagain(error) => return Ok(()),
                        Err(error) => {
                            return Err(context_error(
                                "decoder: cannot submit audio packet",
                                error,
                            ));
                        }
                    }
                }
                Err(Error::Eof) => {
                    if self.drain_sent {
                        return Ok(());
                    }
                    match self.audio.send_eof() {
                        Ok(()) => {
                            self.drain_sent = true;
                            return Ok(());
                        }
                        Err(error) if is_eagain(error) => {
                            self.drain_sent = true;
                            return Ok(());
                        }
                        Err(error) => {
                            return Err(context_error(
                                "decoder: cannot drain audio decoder",
                                error,
                            ));
                        }
                    }
                }
                Err(Error::Exit) => return Err(stopped()),
                Err(Error::InvalidData) => continue,
                Err(error) if is_eagain(error) => {
                    return Err("decoder: audio input is temporarily unavailable".to_string());
                }
                Err(error) => {
                    return Err(context_error("decoder: cannot read audio packet", error));
                }
            }
        }
    }

    /// Resample the frame currently in `decoded` into `converted`. The codec's
    /// output format/layout/rate is validated lazily because some decoders do
    /// not expose it until the first frame.
    fn resample_decoded(&mut self) -> Result<usize, String> {
        let source_format = self.decoded.format();
        let source_rate = if self.decoded.rate() > 0 {
            self.decoded.rate()
        } else {
            self.audio.rate()
        };
        let frame_layout = self.decoded.channel_layout();
        let mut source_layout = frame_layout;
        if source_layout.channels() <= 0 {
            source_layout = ChannelLayout::default(TARGET_CHANNELS);
        } else if source_layout.is_empty() {
            // swr_init canonicalises an unspecified (`UNSPEC`) layout to the
            // default native layout for that channel count. Do it here too,
            // otherwise swr_convert_frame reports AVERROR_INPUT_CHANGED
            // because it compares the raw frame layout with the
            // canonicalised resampler layout.
            source_layout = ChannelLayout::default(source_layout.channels());
        }
        if source_layout != frame_layout {
            // The resampler was configured with the canonical layout, so the
            // frame passed to swr_convert_frame must advertise the same one.
            self.decoded.set_channel_layout(source_layout);
        }
        if source_format == Sample::None || source_rate == 0 {
            return Err("decoder: decoded audio has no sample format or rate".to_string());
        }
        let rebuild = match self.resampler.as_ref() {
            Some(resampler) => {
                let input = resampler.input();
                input.format != source_format
                    || input.channel_layout != source_layout
                    || input.rate != source_rate
            }
            None => true,
        };
        if rebuild {
            self.resampler = Some(
                resampling::Context::get(
                    source_format,
                    source_layout,
                    source_rate,
                    TARGET_SAMPLE,
                    ChannelLayout::STEREO,
                    TARGET_RATE,
                )
                .map_err(|error| context_error("decoder: cannot initialise resampler", error))?,
            );
        }

        let delay = self
            .resampler
            .as_ref()
            .and_then(|resampler| resampler.delay())
            .map(|delay| delay.output.max(0) as usize)
            .unwrap_or(0);
        let input_samples = self.decoded.samples() as u64;
        let scaled = (input_samples * u64::from(TARGET_RATE) + u64::from(source_rate) - 1)
            / u64::from(source_rate);
        let needed = delay.saturating_add(scaled as usize).saturating_add(64);
        self.ensure_output_capacity(needed);
        // swr_convert_frame treats the frame's nb_samples as the output
        // capacity for this call. Give it the full room the input needs;
        // otherwise rate conversion would leave an ever-growing delay.
        self.converted.set_samples(needed);

        let result = match self.resampler.as_mut() {
            Some(resampler) => resampler.run(&self.decoded, &mut self.converted),
            None => return Err("decoder: resampler unavailable".to_string()),
        };
        match result {
            Ok(_) => Ok(self.converted.samples()),
            Err(error) => Err(context_error("decoder: resampling failed", error)),
        }
    }

    /// Drain the resampler tail at decoder EOF. One call is enough for the
    /// filter delay of swresample; a short buffer merely loses a few samples.
    fn flush_resampler(&mut self) -> Result<usize, String> {
        let delay = self
            .resampler
            .as_ref()
            .and_then(|resampler| resampler.delay())
            .map(|delay| delay.output.max(0) as usize)
            .unwrap_or(0);
        let needed = delay.saturating_add(256);
        self.ensure_output_capacity(needed);
        self.converted.set_samples(needed);

        let result = match self.resampler.as_mut() {
            Some(resampler) => resampler.flush(&mut self.converted),
            None => return Ok(0),
        };
        match result {
            Ok(_) => Ok(self.converted.samples()),
            Err(error) => Err(context_error("decoder: resampler flush failed", error)),
        }
    }

    /// Grow the reusable output frame only when a larger batch is required.
    fn ensure_output_capacity(&mut self, frames: usize) {
        if frames <= self.converted_capacity {
            return;
        }
        let capacity = frames.max(4096).saturating_add(frames / 2);
        self.converted = frame::Audio::new(TARGET_SAMPLE, capacity, ChannelLayout::STEREO);
        self.converted_capacity = capacity;
    }

    fn unref_decoded(&mut self) {
        unsafe {
            ffmpeg::ffi::av_frame_unref(self.decoded.as_mut_ptr());
        }
    }

    fn unref_packet(&mut self) {
        unsafe {
            ffmpeg::ffi::av_packet_unref(self.packet.as_mut_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Decoder;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct WavFile {
        path: PathBuf,
    }

    impl WavFile {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for WavFile {
        fn drop(&mut self) {
            if let Some(dir) = self.path.parent() {
                let _ = fs::remove_dir_all(dir);
            }
        }
    }

    fn write_wav(path: &Path, sample_rate: u32, channels: u16, samples: &[i16]) {
        let data_len = u32::try_from(samples.len() * 2).expect("test WAV too large");
        let byte_rate = sample_rate * u32::from(channels) * 2;
        let block_align = channels * 2;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // linear PCM
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        fs::write(path, bytes).expect("write test WAV");
    }

    fn wav_fixture(sample_rate: u32, channels: u16, samples: &[i16]) -> WavFile {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "musika-decode-{}-{}-{}",
            std::process::id(),
            serial,
            nanos
        ));
        fs::create_dir_all(&dir).expect("create test temp dir");
        let path = dir.join("fixture.wav");
        write_wav(&path, sample_rate, channels, samples);
        WavFile { path }
    }

    fn new_stop() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn mean_abs(samples: &[i16]) -> u64 {
        if samples.is_empty() {
            return 0;
        }
        let sum: u64 = samples
            .iter()
            .map(|sample| u64::from(sample.unsigned_abs()))
            .sum();
        sum / samples.len() as u64
    }

    #[test]
    fn decodes_path_and_file_url_to_stereo() {
        let rate = 44_100u32;
        let frames = (rate as f64 * 0.2) as usize;
        let mut mono = Vec::with_capacity(frames);
        for index in 0..frames {
            let time = index as f64 / f64::from(rate);
            let sample = 10_000.0 * (2.0 * std::f64::consts::PI * 440.0 * time).sin();
            mono.push(sample as i16);
        }
        let mono_file = wav_fixture(rate, 1, &mono);
        let mut decoder = Decoder::open(
            mono_file.path().to_str().expect("utf-8 temp path"),
            new_stop(),
        )
        .expect("open mono WAV by path");

        assert!(decoder.is_seekable());
        assert!(decoder.duration() > 0.15, "duration {}", decoder.duration());
        assert!(
            decoder.stream_size() > 44,
            "stream size {}",
            decoder.stream_size()
        );

        let mut out = vec![0i16; 512 * 2];
        let mut total = 0usize;
        let mut nonzero = false;
        loop {
            let read = decoder.read(&mut out, 512).expect("decode mono WAV");
            assert!(read <= 512);
            if read == 0 {
                break;
            }
            for frame in 0..read {
                let left = out[frame * 2];
                let right = out[frame * 2 + 1];
                assert_eq!(left, right, "mono must be duplicated to both channels");
                nonzero |= left != 0;
            }
            total += read;
            assert!(total < 48_000, "decoder never reached EOF");
        }
        assert!(nonzero, "decoded PCM is silent");
        assert!(
            (8_000..11_000).contains(&total),
            "unexpected 0.2 s frame count: {total}"
        );

        let mut stereo = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let time = index as f64 / f64::from(rate);
            let left = 8_000.0 * (2.0 * std::f64::consts::PI * 440.0 * time).sin();
            let right = 8_000.0 * (2.0 * std::f64::consts::PI * 880.0 * time).sin();
            stereo.push(left as i16);
            stereo.push(right as i16);
        }
        let stereo_file = wav_fixture(rate, 2, &stereo);
        let url = format!("file://{}", stereo_file.path().display());
        let mut decoder = Decoder::open(&url, new_stop()).expect("open file:// WAV");
        assert!(decoder.is_seekable());
        let read = decoder.read(&mut out, 512).expect("decode file:// WAV");
        assert!(read > 0, "file:// URL decoded no frames");
        assert!(out[..read * 2].iter().any(|sample| *sample != 0));
        assert!(
            out[..read * 2]
                .chunks_exact(2)
                .any(|frame| frame[0] != frame[1]),
            "stereo channels must stay interleaved, not collapsed to mono"
        );
    }

    #[test]
    fn seeks_forward_and_back() {
        let rate = 44_100u32;
        let frames = rate as usize; // one second
        let mut pcm = Vec::with_capacity(frames * 2);
        for index in 0..frames {
            let time = index as f64 / f64::from(rate);
            let sample = if index < frames / 2 {
                0
            } else {
                (16_000.0 * (2.0 * std::f64::consts::PI * 440.0 * time).sin()) as i16
            };
            pcm.push(sample);
            pcm.push(sample);
        }
        let file = wav_fixture(rate, 2, &pcm);
        let mut decoder = Decoder::open(file.path().to_str().expect("utf-8 temp path"), new_stop())
            .expect("open seek fixture");
        assert!(decoder.duration() > 0.9);

        decoder.seek(0.75).expect("seek into the tone");
        let mut out = vec![0i16; 4_800 * 2];
        let read = decoder.read(&mut out, 4_800).expect("read after seek");
        assert!(read > 0, "seek produced no PCM");
        let level = mean_abs(&out[..read * 2]);
        assert!(
            level > 1_000,
            "seek did not land in the tone (mean |sample| = {level})"
        );

        decoder.seek(0.05).expect("seek back into silence");
        let read = decoder.read(&mut out, 4_800).expect("read after back seek");
        assert!(read > 0, "back seek produced no PCM");
        let peak = out[..read * 2]
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(
            peak < 100,
            "seek back into silence produced audio (peak {peak})"
        );

        assert!(decoder.seek(-1.0).is_err());
        assert!(decoder.seek(f64::NAN).is_err());
    }
}
