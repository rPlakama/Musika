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

//! Stateful single-process MPV transport translating [`AudioCommand`]s to MPV
//! JSON IPC and MPV events/properties to [`EngineEvent`]s + [`EngineClock`].

use std::collections::VecDeque;

use serde_json::{Value, json};

use crate::playback_settings::OutputBackend;
use crate::player::engine::{AudioCommand, EngineEvent};
use crate::player::{ErrorStage, Normalization, PlaybackError};

use super::process::{MpvProcess, ReadOutcome};
use super::protocol::{
    self, EndReason, MpvLaunchConfig, MpvMessage, PROP_AUDIO_DEVICE_LIST, PROP_CACHE_DURATION,
    PROP_DURATION, PROP_FILE_SIZE, PROP_PAUSE, PROP_PAUSED_FOR_CACHE, PROP_PLAYLIST_POS,
    PROP_SAMPLE_RATE, PROP_SEEKABLE, PROP_TIME_POS, cmd_loadfile, cmd_playlist_clear,
    cmd_playlist_remove, cmd_seek, cmd_set_property, cmd_stop,
};

const MAX_RECENT_LOGS: usize = 16;
const DEFAULT_SAMPLE_RATE: u32 = 48_000;

pub(crate) struct MpvTransport {
    process: MpvProcess,
    config: MpvLaunchConfig,
    /// `playlist_entry_id` of the currently playing/loading track.
    active_entry_id: Option<u64>,
    /// `playlist_entry_id` of the gapless next track appended at index 1.
    next_entry_id: Option<u64>,
    pending_load_req: Option<u64>,
    pending_next_req: Option<u64>,
    /// Set when `start-file` promotes `next_entry_id` and we are awaiting
    /// `file-loaded` to emit `EngineEvent::Boundary` with updated duration.
    pending_boundary: bool,
    /// Set if `end-file(eof)` arrived for the outgoing track while `next_entry_id`
    /// is queued and mpv is about to emit `start-file` for it.
    waiting_for_gapless_start: bool,
    recent_logs: VecDeque<String>,
    paused: bool,
    loading: bool,
    file_loaded: bool,
    first_pcm_sent: bool,
    seeking: bool,
    buffering: bool,
    failed_sink: bool,
    time_pos: f64,
    duration: f64,
    file_size: u64,
    seekable: bool,
    sample_rate: u32,
    cache_duration: f64,
    underruns: u64,
    clock_epoch: u64,
    normalization: Normalization,
    eq_af: Option<String>,
    bit_perfect: bool,
    fixed_sample_rate: Option<u32>,
    pinned_target: Option<String>,
}

impl MpvTransport {
    pub(crate) fn new() -> Result<Self, String> {
        let settings = crate::playback_settings::current();
        let config = MpvLaunchConfig::from_settings(&settings);
        Self::with_config(config)
    }

    pub(crate) fn with_config(config: MpvLaunchConfig) -> Result<Self, String> {
        let process = MpvProcess::spawn(&config)?;
        crate::startup::trace(match config.backend {
            OutputBackend::Alsa => "engine.mpv.alsa",
            OutputBackend::Pipewire => "engine.mpv.pipewire",
        });
        Ok(Self::from_parts(process, config))
    }

    #[cfg(test)]
    pub(crate) fn from_test_stream(
        stream: std::os::unix::net::UnixStream,
        config: MpvLaunchConfig,
    ) -> Result<Self, String> {
        let process = MpvProcess::from_test_stream(stream)?;
        Ok(Self::from_parts(process, config))
    }

    fn from_parts(process: MpvProcess, config: MpvLaunchConfig) -> Self {
        let pinned_target = config.output_device.clone();
        let bit_perfect = config.bit_perfect;
        let fixed_sample_rate = config.fixed_sample_rate;
        Self {
            process,
            config,
            active_entry_id: None,
            next_entry_id: None,
            pending_load_req: None,
            pending_next_req: None,
            pending_boundary: false,
            waiting_for_gapless_start: false,
            recent_logs: VecDeque::with_capacity(MAX_RECENT_LOGS),
            paused: true,
            loading: false,
            file_loaded: false,
            first_pcm_sent: false,
            seeking: false,
            buffering: false,
            failed_sink: false,
            time_pos: 0.0,
            duration: 0.0,
            file_size: 0,
            seekable: false,
            sample_rate: DEFAULT_SAMPLE_RATE,
            cache_duration: 0.0,
            underruns: 0,
            clock_epoch: 0,
            normalization: Normalization::Off,
            eq_af: None,
            bit_perfect,
            fixed_sample_rate,
            pinned_target,
        }
    }

    pub(crate) fn ipc_fd(&self) -> libc::c_int {
        self.process.raw_fd()
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.paused || self.loading || self.seeking
    }

    pub(crate) fn loading(&self) -> bool {
        self.loading || self.buffering
    }

    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate.max(1)
    }

    pub(crate) fn played_frames(&self) -> u64 {
        let rate = f64::from(self.sample_rate());
        (self.time_pos.max(0.0) * rate).round() as u64
    }

    pub(crate) fn buffered_frames(&self) -> u64 {
        let rate = f64::from(self.sample_rate());
        (self.cache_duration.max(0.0) * rate).round() as u64
    }

    pub(crate) fn underruns(&self) -> u64 {
        self.underruns
    }

    pub(crate) fn clock_epoch(&self) -> u64 {
        self.clock_epoch
    }

    /// Applies one [`AudioCommand`] from the engine thread. Any IPC write
    /// failure is surfaced immediately as [`EngineEvent::SinkFailed`].
    pub(crate) fn apply_command(
        &mut self,
        command: AudioCommand,
        emit: &mut dyn FnMut(EngineEvent) -> bool,
    ) -> bool {
        let result = match command {
            AudioCommand::Load {
                url,
                next_url,
                paused,
                generation: _,
            } => self.cmd_load(&url, next_url.as_deref(), paused),
            AudioCommand::SetNextUrl(next_url) => self.cmd_set_next_url(next_url.as_deref()),
            AudioCommand::Pause => {
                self.paused = true;
                self.send_property("pause", json!(true))
            }
            AudioCommand::Resume => {
                self.paused = false;
                self.send_property("pause", json!(false))
            }
            AudioCommand::Seek(seconds) => self.cmd_seek(seconds),
            AudioCommand::SetVolume(volume) => {
                self.send_property("volume", json!(volume.clamp(0.0, 100.0)))
            }
            AudioCommand::SetMuted(muted) => self.send_property("mute", json!(muted)),
            AudioCommand::SetNormalization(mode) => {
                self.normalization = mode;
                self.sync_dsp()
            }
            AudioCommand::SetOutputTarget(target) => self.cmd_set_output_target(target),
            AudioCommand::SetEq(af) => {
                self.eq_af = af.filter(|value| !value.is_empty());
                self.sync_dsp()
            }
            AudioCommand::SetExclusive(exclusive) => {
                self.config.exclusive = exclusive;
                self.send_property("audio-exclusive", json!(if exclusive { "yes" } else { "no" }))
            }
            AudioCommand::SetBitPerfect(enabled) => {
                self.bit_perfect = enabled;
                self.config.bit_perfect = enabled;
                self.sync_dsp()
            }
            AudioCommand::SetSampleRate(rate) => {
                self.fixed_sample_rate = rate;
                self.config.fixed_sample_rate = rate;
                self.sync_dsp()
            }
            AudioCommand::Stop => self.cmd_stop(),
        };

        if let Err(message) = result {
            return self.report_sink_failure(message, emit);
        }
        true
    }

    /// Reads and dispatches all pending MPV IPC messages. Returns `false` if
    /// the service event receiver closed.
    pub(crate) fn pump(&mut self, emit: &mut dyn FnMut(EngineEvent) -> bool) -> bool {
        match self.process.drain_messages() {
            ReadOutcome::Messages(messages) => {
                for message in messages {
                    if !self.handle_message(message, emit) {
                        return false;
                    }
                }
                if !self.failed_sink
                    && let Some(reason) = self.process.check_exited()
                {
                    return self.report_sink_failure(reason, emit);
                }
                true
            }
            ReadOutcome::Disconnected(reason) => self.report_sink_failure(reason, emit),
        }
    }

    fn cmd_load(&mut self, url: &str, next_url: Option<&str>, paused: bool) -> Result<(), String> {
        crate::startup::trace("play.requested");
        self.clock_epoch = self.clock_epoch.wrapping_add(1);
        self.paused = paused;
        self.loading = !paused;
        self.file_loaded = false;
        self.first_pcm_sent = false;
        self.seeking = false;
        self.buffering = false;
        self.pending_boundary = false;
        self.waiting_for_gapless_start = false;
        self.time_pos = 0.0;
        self.duration = 0.0;
        self.file_size = 0;
        self.seekable = false;
        self.cache_duration = 0.0;
        self.underruns = 0;
        self.active_entry_id = None;
        self.next_entry_id = None;
        self.pending_load_req = None;
        self.pending_next_req = None;
        self.recent_logs.clear();

        self.send_property("pause", json!(paused))?;
        let req = self.process.alloc_req();
        self.pending_load_req = Some(req);
        self.process.write_raw(&cmd_loadfile(req, url, false))?;

        if let Some(next) = next_url {
            let next_req = self.process.alloc_req();
            self.pending_next_req = Some(next_req);
            self.process.write_raw(&cmd_loadfile(next_req, next, true))?;
        }
        Ok(())
    }

    fn cmd_set_next_url(&mut self, next_url: Option<&str>) -> Result<(), String> {
        self.next_entry_id = None;
        self.pending_next_req = None;
        let clear_req = self.process.alloc_req();
        self.process.write_raw(&cmd_playlist_clear(clear_req))?;
        if let Some(url) = next_url {
            let req = self.process.alloc_req();
            self.pending_next_req = Some(req);
            self.process.write_raw(&cmd_loadfile(req, url, true))?;
        }
        Ok(())
    }

    fn cmd_seek(&mut self, seconds: f64) -> Result<(), String> {
        self.clock_epoch = self.clock_epoch.wrapping_add(1);
        self.time_pos = seconds.max(0.0);
        self.seeking = true;
        if !self.paused {
            self.loading = true;
        }
        let req = self.process.alloc_req();
        self.process.write_raw(&cmd_seek(req, seconds))
    }

    fn cmd_stop(&mut self) -> Result<(), String> {
        self.clock_epoch = self.clock_epoch.wrapping_add(1);
        self.paused = true;
        self.loading = false;
        self.file_loaded = false;
        self.seeking = false;
        self.buffering = false;
        self.first_pcm_sent = false;
        self.pending_boundary = false;
        self.waiting_for_gapless_start = false;
        self.active_entry_id = None;
        self.next_entry_id = None;
        self.pending_load_req = None;
        self.pending_next_req = None;
        self.time_pos = 0.0;
        self.cache_duration = 0.0;
        self.underruns = 0;
        let req = self.process.alloc_req();
        self.process.write_raw(&cmd_stop(req))
    }

    fn cmd_set_output_target(&mut self, target: Option<String>) -> Result<(), String> {
        self.pinned_target = target.clone();
        self.config.output_device = target.clone();
        if self.config.ao_override.is_some() {
            return Ok(());
        }
        let device = protocol::normalize_audio_device(
            self.config.backend,
            target.as_deref(),
            &self.config.alsa_device,
        );
        self.send_property("audio-device", json!(device))
    }

    fn sync_dsp(&mut self) -> Result<(), String> {
        let rg = protocol::replaygain_value(self.normalization, self.bit_perfect);
        self.send_property("replaygain", json!(rg))?;

        let af = if self.bit_perfect {
            ""
        } else {
            self.eq_af.as_deref().unwrap_or("")
        };
        self.send_property("af", json!(af))?;

        let rate = if self.bit_perfect {
            0
        } else {
            self.fixed_sample_rate.unwrap_or(0)
        };
        self.send_property("audio-samplerate", json!(rate))
    }

    fn send_property(&mut self, name: &str, value: Value) -> Result<(), String> {
        let req = self.process.alloc_req();
        self.process.write_raw(&cmd_set_property(req, name, value))
    }

    fn handle_message(
        &mut self,
        message: MpvMessage,
        emit: &mut dyn FnMut(EngineEvent) -> bool,
    ) -> bool {
        match message {
            MpvMessage::Response {
                request_id,
                error,
                data,
            } => self.handle_response(request_id, &error, data.as_ref(), emit),
            MpvMessage::StartFile { playlist_entry_id } => {
                self.handle_start_file(playlist_entry_id);
                true
            }
            MpvMessage::FileLoaded => self.handle_file_loaded(emit),
            MpvMessage::PlaybackRestart => self.handle_playback_restart(emit),
            MpvMessage::EndFile {
                reason,
                file_error,
                playlist_entry_id,
            } => self.handle_end_file(reason, file_error.as_deref(), playlist_entry_id, emit),
            MpvMessage::PropertyChange { id, data } => {
                self.handle_property_change(id, data.as_ref(), emit)
            }
            MpvMessage::LogMessage {
                prefix,
                level,
                text,
            } => {
                if !text.is_empty() && matches!(level.as_str(), "warn" | "error" | "fatal") {
                    let entry = if prefix.is_empty() {
                        text
                    } else {
                        format!("{prefix}: {text}")
                    };
                    if self.recent_logs.len() >= MAX_RECENT_LOGS {
                        _ = self.recent_logs.pop_front();
                    }
                    self.recent_logs.push_back(entry);
                }
                true
            }
            MpvMessage::Other => true,
        }
    }

    fn handle_response(
        &mut self,
        request_id: u64,
        error: &str,
        data: Option<&Value>,
        emit: &mut dyn FnMut(EngineEvent) -> bool,
    ) -> bool {
        if self.pending_load_req == Some(request_id) {
            self.pending_load_req = None;
            if error == "success" {
                if let Some(entry_id) = protocol::response_playlist_entry_id(data) {
                    self.active_entry_id = Some(entry_id);
                }
            } else {
                self.loading = false;
                let err = PlaybackError::classify(ErrorStage::Load, error);
                return emit(EngineEvent::Failed { error: err });
            }
        } else if self.pending_next_req == Some(request_id) {
            self.pending_next_req = None;
            if error == "success" {
                self.next_entry_id = protocol::response_playlist_entry_id(data);
            } else {
                crate::debug_log!("mpv: failed to queue gapless next track: {error}");
            }
        } else if error != "success" && !error.is_empty() {
            crate::debug_log!("mpv: request {request_id} returned error: {error}");
        }
        true
    }

    fn handle_start_file(&mut self, playlist_entry_id: Option<u64>) {
        if let Some(entry_id) = playlist_entry_id {
            if self.next_entry_id == Some(entry_id) {
                self.active_entry_id = self.next_entry_id.take();
                self.pending_boundary = true;
                self.waiting_for_gapless_start = false;
                self.file_loaded = false;
                self.duration = 0.0;
                self.file_size = 0;
                self.time_pos = 0.0;
                self.recent_logs.clear();
                return;
            }
            if self.active_entry_id.is_none() {
                self.active_entry_id = Some(entry_id);
            }
        }
        self.waiting_for_gapless_start = false;
    }

    fn handle_file_loaded(&mut self, emit: &mut dyn FnMut(EngineEvent) -> bool) -> bool {
        self.file_loaded = true;
        if self.pending_boundary {
            self.pending_boundary = false;
            self.clock_epoch = self.clock_epoch.wrapping_add(1);
            self.time_pos = 0.0;
            self.loading = false;
            self.seeking = false;
            self.first_pcm_sent = true;
            crate::startup::trace("gapless.next");
            // Keep MPV's internal playlist bounded to the current item.
            let req = self.process.alloc_req();
            if let Err(error) = self.process.write_raw(&cmd_playlist_remove(req, 0)) {
                return self.report_sink_failure(error, emit);
            }
            return emit(EngineEvent::Boundary {
                duration: self.duration,
                stream_size: self.file_size,
                position: 0.0,
            });
        }
        emit(EngineEvent::Loaded {
            duration: self.duration,
            stream_size: self.file_size,
            seekable: self.seekable,
        })
    }

    fn handle_playback_restart(&mut self, emit: &mut dyn FnMut(EngineEvent) -> bool) -> bool {
        if self.paused {
            return true;
        }
        if !self.first_pcm_sent {
            self.first_pcm_sent = true;
            crate::startup::trace("first.pcm.written");
            if !emit(EngineEvent::FirstPcm) {
                return false;
            }
        }
        if self.loading || self.seeking {
            self.loading = false;
            self.seeking = false;
            if !emit(EngineEvent::LoadingCleared) {
                return false;
            }
        }
        true
    }

    fn handle_end_file(
        &mut self,
        reason: EndReason,
        file_error: Option<&str>,
        playlist_entry_id: Option<u64>,
        emit: &mut dyn FnMut(EngineEvent) -> bool,
    ) -> bool {
        // Ignore end-file events for superseded loads or already-promoted entries.
        if let (Some(active), Some(ended)) = (self.active_entry_id, playlist_entry_id)
            && active != ended
        {
            return true;
        }
        match reason {
            EndReason::Eof => {
                if self.next_entry_id.is_some() || self.pending_next_req.is_some() {
                    // MPV is transitioning gaplessly to the queued next item;
                    // `start-file` + `file-loaded` will emit `Boundary`.
                    self.waiting_for_gapless_start = true;
                    return true;
                }
                self.active_entry_id = None;
                self.loading = false;
                self.seeking = false;
                emit(EngineEvent::Ended)
            }
            EndReason::Error => {
                self.loading = false;
                let stage = if self.seeking {
                    ErrorStage::Seek
                } else if self.first_pcm_sent {
                    ErrorStage::Read
                } else {
                    ErrorStage::Open
                };
                self.seeking = false;
                self.active_entry_id = None;
                let message = protocol::pick_diagnostic_message(file_error, &self.recent_logs);
                let error = PlaybackError::classify(stage, message);
                emit(EngineEvent::Failed { error })
            }
            EndReason::Stop | EndReason::Quit | EndReason::Redirect | EndReason::Unknown => true,
        }
    }

    fn handle_property_change(
        &mut self,
        id: u64,
        data: Option<&Value>,
        emit: &mut dyn FnMut(EngineEvent) -> bool,
    ) -> bool {
        match id {
            PROP_TIME_POS => {
                if let Some(pos) = data.and_then(Value::as_f64)
                    && pos.is_finite()
                    && pos >= 0.0
                {
                    self.time_pos = pos;
                }
            }
            PROP_DURATION => {
                if let Some(duration) = data.and_then(Value::as_f64)
                    && duration.is_finite()
                    && duration >= 0.0
                    && (duration - self.duration).abs() > f64::EPSILON
                {
                    self.duration = duration;
                    if self.file_loaded {
                        return emit(EngineEvent::Loaded {
                            duration: self.duration,
                            stream_size: self.file_size,
                            seekable: self.seekable,
                        });
                    }
                }
            }
            PROP_PAUSE => {
                if let Some(paused) = data.and_then(Value::as_bool) {
                    self.paused = paused;
                }
            }
            PROP_PAUSED_FOR_CACHE => {
                let buffering = data.and_then(Value::as_bool).unwrap_or(false);
                if buffering != self.buffering {
                    self.buffering = buffering;
                    if buffering && !self.paused && self.first_pcm_sent {
                        self.underruns = self.underruns.saturating_add(1);
                    }
                    if !emit(EngineEvent::Buffering(buffering)) {
                        return false;
                    }
                }
            }
            PROP_SEEKABLE => {
                if let Some(seekable) = data.and_then(Value::as_bool)
                    && seekable != self.seekable
                {
                    self.seekable = seekable;
                    if self.file_loaded {
                        return emit(EngineEvent::Loaded {
                            duration: self.duration,
                            stream_size: self.file_size,
                            seekable: self.seekable,
                        });
                    }
                }
            }
            PROP_SAMPLE_RATE => {
                if let Some(rate) = data.and_then(Value::as_u64)
                    && rate > 0
                    && rate <= u64::from(u32::MAX)
                {
                    self.sample_rate = rate as u32;
                }
            }
            PROP_CACHE_DURATION => {
                if let Some(secs) = data.and_then(Value::as_f64)
                    && secs.is_finite()
                    && secs >= 0.0
                {
                    self.cache_duration = secs;
                }
            }
            PROP_PLAYLIST_POS => {
                // If MPV dropped back to idle (-1) while we were waiting for a
                // gapless start, the queued item vanished; advance cleanly.
                if self.waiting_for_gapless_start
                    && data.and_then(Value::as_i64).is_some_and(|pos| pos < 0)
                {
                    self.waiting_for_gapless_start = false;
                    self.active_entry_id = None;
                    return emit(EngineEvent::Ended);
                }
            }
            PROP_AUDIO_DEVICE_LIST => {
                if let Some(value) = data {
                    let devices = protocol::parse_audio_device_list(value);
                    super::update_cached_devices(devices.clone());
                    if self.config.backend == OutputBackend::Pipewire
                        && self.config.ao_override.is_none()
                        && let Some(target) = self.pinned_target.clone()
                        && !devices.is_empty()
                        && !devices
                            .iter()
                            .any(|device| protocol::device_id_matches(&target, &device.id))
                    {
                        self.pinned_target = None;
                        self.config.output_device = None;
                        if let Err(error) = self.send_property("audio-device", json!("auto")) {
                            return self.report_sink_failure(error, emit);
                        }
                        if !emit(EngineEvent::OutputFallback { target }) {
                            return false;
                        }
                    }
                }
            }
            PROP_FILE_SIZE => {
                if let Some(size) = data.and_then(Value::as_u64)
                    && size != self.file_size
                {
                    self.file_size = size;
                    if self.file_loaded {
                        return emit(EngineEvent::Loaded {
                            duration: self.duration,
                            stream_size: self.file_size,
                            seekable: self.seekable,
                        });
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn report_sink_failure(
        &mut self,
        message: String,
        emit: &mut dyn FnMut(EngineEvent) -> bool,
    ) -> bool {
        if self.failed_sink {
            return true;
        }
        self.failed_sink = true;
        self.loading = false;
        emit(EngineEvent::SinkFailed { message })
    }
}
