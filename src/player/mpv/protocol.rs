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

//! Pure MPV JSON IPC encoding/decoding, launch argv construction, device
//! normalization, and error log correlation.
//!
//! No I/O or process spawning lives here, so every protocol contract is covered
//! by deterministic unit tests.

use std::collections::VecDeque;
use std::path::Path;

use serde_json::{Value, json};

use crate::playback_settings::OutputBackend;
use crate::player::{Normalization, OutputDevice};

pub(crate) const PROP_TIME_POS: u64 = 1;
pub(crate) const PROP_DURATION: u64 = 2;
pub(crate) const PROP_PAUSE: u64 = 3;
pub(crate) const PROP_PAUSED_FOR_CACHE: u64 = 4;
pub(crate) const PROP_SEEKABLE: u64 = 5;
pub(crate) const PROP_SAMPLE_RATE: u64 = 6;
pub(crate) const PROP_CACHE_DURATION: u64 = 7;
pub(crate) const PROP_PLAYLIST_POS: u64 = 8;
pub(crate) const PROP_AUDIO_DEVICE: u64 = 9;
pub(crate) const PROP_AUDIO_DEVICE_LIST: u64 = 10;
pub(crate) const PROP_FILE_SIZE: u64 = 11;

pub(crate) const OBSERVED_PROPERTIES: &[(u64, &str)] = &[
    (PROP_TIME_POS, "time-pos"),
    (PROP_DURATION, "duration"),
    (PROP_PAUSE, "pause"),
    (PROP_PAUSED_FOR_CACHE, "paused-for-cache"),
    (PROP_SEEKABLE, "seekable"),
    (PROP_SAMPLE_RATE, "audio-params/samplerate"),
    (PROP_CACHE_DURATION, "demuxer-cache-duration"),
    (PROP_PLAYLIST_POS, "playlist-pos"),
    (PROP_AUDIO_DEVICE, "audio-device"),
    (PROP_AUDIO_DEVICE_LIST, "audio-device-list"),
    (PROP_FILE_SIZE, "file-size"),
];

/// Configuration distilled from `PlaybackSettings` + environment overrides for
/// spawning one headless audio-only `mpv` instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MpvLaunchConfig {
    pub backend: OutputBackend,
    pub output_device: Option<String>,
    pub alsa_device: String,
    pub exclusive: bool,
    pub bit_perfect: bool,
    pub fixed_sample_rate: Option<u32>,
    /// Optional explicit `--ao=` override (e.g., `"null"` in headless tests).
    pub ao_override: Option<String>,
}

impl Default for MpvLaunchConfig {
    fn default() -> Self {
        Self {
            backend: OutputBackend::Pipewire,
            output_device: None,
            alsa_device: "default".to_string(),
            exclusive: false,
            bit_perfect: false,
            fixed_sample_rate: None,
            ao_override: None,
        }
    }
}

impl MpvLaunchConfig {
    pub(crate) fn from_settings(settings: &crate::playback_settings::PlaybackSettings) -> Self {
        let env_backend = std::env::var("MUSIKA_OUTPUT_BACKEND").ok();
        let backend = match env_backend
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("alsa") => OutputBackend::Alsa,
            Some("pipewire") => OutputBackend::Pipewire,
            _ => settings.output_backend,
        };
        let output_device = if settings.follow_default {
            None
        } else {
            settings.output_device.clone()
        };
        let fixed_sample_rate = settings
            .fixed_pipeline_rate
            .then_some(settings.fixed_pipeline_hz);
        Self {
            backend,
            output_device,
            alsa_device: settings.alsa_device.clone(),
            exclusive: settings.exclusive,
            bit_perfect: settings.bit_perfect,
            fixed_sample_rate,
            ao_override: std::env::var("MUSIKA_MPV_AO")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        }
    }
}

/// Normalizes a user/persisted device identifier into MPV's `audio-device`
/// vocabulary (`auto`, `pipewire/<node>`, or `alsa/<device>`).
///
/// Backwards-compatible with bare PipeWire `node.name` strings persisted before
/// the MPV migration.
pub(crate) fn normalize_audio_device(
    backend: OutputBackend,
    target: Option<&str>,
    alsa_device: &str,
) -> String {
    match backend {
        OutputBackend::Alsa => {
            let raw = target
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| alsa_device.trim());
            if raw.is_empty() || raw.eq_ignore_ascii_case("auto") {
                return "auto".to_string();
            }
            if raw.contains('/') {
                raw.to_string()
            } else {
                format!("alsa/{raw}")
            }
        }
        OutputBackend::Pipewire => {
            let Some(raw) = target.map(str::trim).filter(|value| !value.is_empty()) else {
                return "auto".to_string();
            };
            if raw.eq_ignore_ascii_case("auto") {
                return "auto".to_string();
            }
            if raw.contains('/') {
                raw.to_string()
            } else {
                format!("pipewire/{raw}")
            }
        }
    }
}

/// Builds the deterministic argument vector for spawning headless audio-only `mpv`.
pub(crate) fn mpv_args(socket_path: &Path, config: &MpvLaunchConfig) -> Vec<String> {
    let ao = config
        .ao_override
        .as_deref()
        .unwrap_or(match config.backend {
            OutputBackend::Pipewire => "pipewire",
            OutputBackend::Alsa => "alsa",
        });
    let audio_device = if config.ao_override.is_some() {
        "auto".to_string()
    } else {
        normalize_audio_device(
            config.backend,
            config.output_device.as_deref(),
            &config.alsa_device,
        )
    };
    let mut args = vec![
        "--idle=yes".to_string(),
        "--no-video".to_string(),
        "--vid=no".to_string(),
        "--audio-display=no".to_string(),
        "--no-terminal".to_string(),
        "--really-quiet".to_string(),
        "--no-config".to_string(),
        "--load-scripts=no".to_string(),
        "--osd-level=0".to_string(),
        "--input-default-bindings=no".to_string(),
        "--input-vo-keyboard=no".to_string(),
        "--input-terminal=no".to_string(),
        "--gapless-audio=yes".to_string(),
        "--audio-client-name=musika".to_string(),
        "--cache=yes".to_string(),
        "--demuxer-max-bytes=8MiB".to_string(),
        "--demuxer-max-back-bytes=2MiB".to_string(),
        "--demuxer-readahead-secs=20".to_string(),
        "--stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=4,timeout=120000000"
            .to_string(),
        "--replaygain-clip=no".to_string(),
        "--volume-max=100".to_string(),
        format!("--ao={ao}"),
        format!("--audio-device={audio_device}"),
        format!(
            "--audio-exclusive={}",
            if config.exclusive { "yes" } else { "no" }
        ),
        format!("--input-ipc-server={}", socket_path.display()),
    ];
    if !config.bit_perfect
        && let Some(rate) = config.fixed_sample_rate
        && rate > 0
    {
        args.push(format!("--audio-samplerate={rate}"));
    }
    args
}

pub(crate) fn replaygain_value(mode: Normalization, bit_perfect: bool) -> &'static str {
    if bit_perfect {
        return "no";
    }
    match mode {
        Normalization::Off => "no",
        Normalization::Track => "track",
        Normalization::Album => "album",
    }
}

pub(crate) fn encode_command(request_id: u64, command: Value) -> String {
    let mut line = json!({
        "command": command,
        "request_id": request_id,
    })
    .to_string();
    line.push('\n');
    line
}

pub(crate) fn cmd_loadfile(request_id: u64, url: &str, append: bool) -> String {
    let mode = if append { "append" } else { "replace" };
    encode_command(request_id, json!(["loadfile", url, mode]))
}

pub(crate) fn cmd_playlist_remove(request_id: u64, index: usize) -> String {
    encode_command(request_id, json!(["playlist-remove", index]))
}

pub(crate) fn cmd_playlist_clear(request_id: u64) -> String {
    encode_command(request_id, json!(["playlist-clear"]))
}

pub(crate) fn cmd_stop(request_id: u64) -> String {
    encode_command(request_id, json!(["stop"]))
}

pub(crate) fn cmd_seek(request_id: u64, seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    encode_command(request_id, json!(["seek", seconds, "absolute+exact"]))
}

pub(crate) fn cmd_set_property(request_id: u64, name: &str, value: Value) -> String {
    encode_command(request_id, json!(["set_property", name, value]))
}

pub(crate) fn cmd_observe_property(request_id: u64, id: u64, name: &str) -> String {
    encode_command(request_id, json!(["observe_property", id, name]))
}

pub(crate) fn cmd_request_log_messages(request_id: u64, level: &str) -> String {
    encode_command(request_id, json!(["request_log_messages", level]))
}

pub(crate) fn cmd_quit(request_id: u64) -> String {
    encode_command(request_id, json!(["quit"]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Unknown,
}

impl EndReason {
    fn parse(raw: &str) -> Self {
        match raw {
            "eof" => Self::Eof,
            "stop" => Self::Stop,
            "quit" => Self::Quit,
            "error" => Self::Error,
            "redirect" => Self::Redirect,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MpvMessage {
    Response {
        request_id: u64,
        error: String,
        data: Option<Value>,
    },
    StartFile {
        playlist_entry_id: Option<u64>,
    },
    FileLoaded,
    PlaybackRestart,
    EndFile {
        reason: EndReason,
        file_error: Option<String>,
        playlist_entry_id: Option<u64>,
    },
    PropertyChange {
        id: u64,
        data: Option<Value>,
    },
    LogMessage {
        prefix: String,
        level: String,
        text: String,
    },
    Other,
}

/// Parses one newline-delimited JSON IPC message emitted by `mpv`.
/// Returns `None` on empty or malformed JSON lines (never panics).
pub(crate) fn parse_ipc_line(line: &str) -> Option<MpvMessage> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    let object = value.as_object()?;

    if let Some(event) = object.get("event").and_then(Value::as_str) {
        return Some(match event {
            "start-file" => MpvMessage::StartFile {
                playlist_entry_id: object.get("playlist_entry_id").and_then(Value::as_u64),
            },
            "file-loaded" => MpvMessage::FileLoaded,
            "playback-restart" => MpvMessage::PlaybackRestart,
            "end-file" => MpvMessage::EndFile {
                reason: object
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(EndReason::parse)
                    .unwrap_or(EndReason::Unknown),
                file_error: object
                    .get("file_error")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                playlist_entry_id: object.get("playlist_entry_id").and_then(Value::as_u64),
            },
            "property-change" => MpvMessage::PropertyChange {
                id: object.get("id").and_then(Value::as_u64).unwrap_or(0),
                data: object.get("data").cloned(),
            },
            "log-message" => MpvMessage::LogMessage {
                prefix: object
                    .get("prefix")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                level: object
                    .get("level")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                text: object
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            },
            _ => MpvMessage::Other,
        });
    }

    if let Some(request_id) = object.get("request_id").and_then(Value::as_u64) {
        let error = object
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let data = object.get("data").cloned();
        return Some(MpvMessage::Response {
            request_id,
            error,
            data,
        });
    }

    Some(MpvMessage::Other)
}

/// Extracts `playlist_entry_id` from a `loadfile` command response.
pub(crate) fn response_playlist_entry_id(data: Option<&Value>) -> Option<u64> {
    data?.as_object()?.get("playlist_entry_id")?.as_u64()
}

/// Parses MPV's `audio-device-list` JSON array into sorted, deduplicated
/// [`OutputDevice`] entries (filtering out the synthetic `"auto"` entry).
pub(crate) fn parse_audio_device_list(value: &Value) -> Vec<OutputDevice> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut list: Vec<OutputDevice> = items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let id = object.get("name")?.as_str()?.trim();
            if id.is_empty() || id.eq_ignore_ascii_case("auto") {
                return None;
            }
            let label = object
                .get("description")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .unwrap_or(id);
            Some(OutputDevice {
                id: id.to_string(),
                label: label.to_string(),
            })
        })
        .collect();
    list.sort_by_cached_key(|device| device.label.to_lowercase());
    list.dedup_by(|a, b| a.id == b.id);
    list
}

/// Parses `mpv --audio-device=help` stdout as a fallback when no live engine
/// is running. Expected format:
/// ```text
/// List of detected audio devices:
///   'auto' (Autoselect device)
///   'pipewire/alsa_output.pci-0000_00_1f.3' (Built-in Audio Analog Stereo)
/// ```
pub(crate) fn parse_audio_device_help(stdout: &str) -> Vec<OutputDevice> {
    let mut list = Vec::new();
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        let Some(rest) = line.strip_prefix('\'') else {
            continue;
        };
        let Some((id, tail)) = rest.split_once('\'') else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || id.eq_ignore_ascii_case("auto") {
            continue;
        }
        let label = tail
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(id);
        list.push(OutputDevice {
            id: id.to_string(),
            label: label.to_string(),
        });
    }
    list.sort_by_cached_key(|device| device.label.to_lowercase());
    list.dedup_by(|a, b| a.id == b.id);
    list
}

/// Checks whether a persisted/selected device matches an entry in MPV's
/// `audio-device-list`, tolerating bare PipeWire node names (`sink.one` matches
/// `pipewire/sink.one`).
pub(crate) fn device_id_matches(selected: &str, candidate: &str) -> bool {
    if selected == candidate {
        return true;
    }
    if let Some(stripped) = candidate.strip_prefix("pipewire/")
        && selected == stripped
    {
        return true;
    }
    if let Some(stripped) = selected.strip_prefix("pipewire/")
        && stripped == candidate
    {
        return true;
    }
    false
}

/// Picks the most diagnostic error message when `mpv` reports `end-file` with
/// `reason: "error"`. MPV's `file_error` is often generic (`"loading failed"`),
/// while the preceding `log-message` lines carry the real FFmpeg/HTTP/AO error
/// (`Server returned 401 Unauthorized`, `Connection refused`, etc.).
pub(crate) fn pick_diagnostic_message(
    file_error: Option<&str>,
    recent_logs: &VecDeque<String>,
) -> String {
    let generic = file_error
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("mpv playback error");
    // Prefer the most recent log line that yields a specific error kind (Auth,
    // Http, Network, Decode, Unsupported, Device).
    for line in recent_logs.iter().rev() {
        let classified =
            crate::player::PlaybackError::classify(crate::player::ErrorStage::Open, line);
        if classified.kind != crate::player::error::ErrorKind::Other {
            return line.clone();
        }
    }
    // Otherwise prefer the newest non-empty log line over generic "loading failed".
    if generic.eq_ignore_ascii_case("loading failed")
        && let Some(last) = recent_logs.back()
        && !last.is_empty()
    {
        return last.clone();
    }
    generic.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpv_args_cover_pipewire_alsa_exclusive_and_fixed_rate() {
        let sock = Path::new("/run/user/1000/musika/mpv.sock");
        let pw = mpv_args(
            sock,
            &MpvLaunchConfig {
                backend: OutputBackend::Pipewire,
                output_device: Some("alsa_output.pci-0000_00_1f.3".to_string()),
                alsa_device: "default".to_string(),
                exclusive: true,
                bit_perfect: false,
                fixed_sample_rate: Some(96_000),
                ao_override: None,
            },
        );
        assert!(pw.contains(&"--idle=yes".to_string()));
        assert!(pw.contains(&"--no-video".to_string()));
        assert!(pw.contains(&"--no-config".to_string()));
        assert!(pw.contains(&"--gapless-audio=yes".to_string()));
        assert!(pw.contains(&"--ao=pipewire".to_string()));
        assert!(pw.contains(&"--audio-device=pipewire/alsa_output.pci-0000_00_1f.3".to_string()));
        assert!(pw.contains(&"--audio-exclusive=yes".to_string()));
        assert!(pw.contains(&"--audio-samplerate=96000".to_string()));
        assert!(pw.contains(&"--input-ipc-server=/run/user/1000/musika/mpv.sock".to_string()));

        // Bit-perfect suppresses fixed sample-rate resampling.
        let bp = mpv_args(
            sock,
            &MpvLaunchConfig {
                backend: OutputBackend::Alsa,
                output_device: None,
                alsa_device: "hw:0,0".to_string(),
                exclusive: true,
                bit_perfect: true,
                fixed_sample_rate: Some(48_000),
                ao_override: None,
            },
        );
        assert!(bp.contains(&"--ao=alsa".to_string()));
        assert!(bp.contains(&"--audio-device=alsa/hw:0,0".to_string()));
        assert!(!bp.iter().any(|arg| arg.starts_with("--audio-samplerate=")));
    }

    #[test]
    fn parse_ipc_line_handles_events_replies_and_malformed_input() {
        assert_eq!(parse_ipc_line(""), None);
        assert_eq!(parse_ipc_line("not json"), None);
        assert_eq!(parse_ipc_line("[]"), None);

        let reply =
            parse_ipc_line(r#"{"data":{"playlist_entry_id":7},"request_id":42,"error":"success"}"#)
                .expect("reply");
        let MpvMessage::Response {
            request_id,
            error,
            data,
        } = reply
        else {
            panic!("unexpected: {reply:?}");
        };
        assert_eq!(request_id, 42);
        assert_eq!(error, "success");
        assert_eq!(response_playlist_entry_id(data.as_ref()), Some(7));

        let end = parse_ipc_line(
            r#"{"event":"end-file","reason":"error","file_error":"loading failed","playlist_entry_id":7}"#,
        )
        .expect("end-file");
        assert_eq!(
            end,
            MpvMessage::EndFile {
                reason: EndReason::Error,
                file_error: Some("loading failed".to_string()),
                playlist_entry_id: Some(7),
            }
        );

        let prop =
            parse_ipc_line(r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#)
                .expect("property-change");
        assert_eq!(
            prop,
            MpvMessage::PropertyChange {
                id: 1,
                data: Some(json!(12.5)),
            }
        );
    }

    #[test]
    fn parse_audio_devices_filters_auto_and_sorts() {
        let json_list = json!([
            {"name": "auto", "description": "Autoselect device"},
            {"name": "pipewire/sink.b", "description": "Zeta DAC"},
            {"name": "pipewire/sink.a", "description": "Alpha Speakers"},
            {"name": "pipewire/sink.a", "description": "Alpha Speakers"},
        ]);
        let devices = parse_audio_device_list(&json_list);
        assert_eq!(
            devices,
            vec![
                OutputDevice {
                    id: "pipewire/sink.a".to_string(),
                    label: "Alpha Speakers".to_string(),
                },
                OutputDevice {
                    id: "pipewire/sink.b".to_string(),
                    label: "Zeta DAC".to_string(),
                },
            ]
        );

        let help = "\
List of detected audio devices:
  'auto' (Autoselect device)
  'pipewire/sink.b' (Zeta DAC)
  'pipewire/sink.a' (Alpha Speakers)
";
        assert_eq!(parse_audio_device_help(help), devices);
        assert!(device_id_matches("sink.a", "pipewire/sink.a"));
        assert!(device_id_matches("pipewire/sink.a", "pipewire/sink.a"));
        assert!(!device_id_matches("sink.c", "pipewire/sink.a"));
    }

    #[test]
    fn pick_diagnostic_message_prefers_specific_log_over_generic_loading_failed() {
        let mut logs = VecDeque::new();
        logs.push_back("ffmpeg: https: Server returned 401 Unauthorized".to_string());
        logs.push_back("stream: Failed to open https://example/stream".to_string());
        let picked = pick_diagnostic_message(Some("loading failed"), &logs);
        assert!(picked.contains("401 Unauthorized"), "got: {picked}");
    }
}
