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

//! PERSIST-1: playback settings that outlive the session.
//!
//! Volume, mute and the queue snapshot already live in `session_status`.
//! This module owns the behaviour that is not per-track: repeat/shuffle,
//! crossfade, normalization, the selected EQ profile, the output device and
//! the stream quality/cache policy. One JSON document under one key keeps the
//! schema change-free when a later phase adds a field.

use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;

const SETTINGS_KEY: &str = "playback";

/// Process-wide copy used by queue construction (stream URLs and playback-cache
/// eligibility). The daemon and the playback service install every loaded or
/// changed document here.
static CURRENT: OnceLock<RwLock<PlaybackSettings>> = OnceLock::new();

pub fn install(settings: &PlaybackSettings) {
    let slot = CURRENT.get_or_init(|| RwLock::new(PlaybackSettings::default()));
    *slot.write().unwrap_or_else(std::sync::PoisonError::into_inner) = settings.clone().sanitized();
}

pub fn current() -> PlaybackSettings {
    CURRENT
        .get()
        .and_then(|slot| slot.read().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputBackend {
    #[default]
    Pipewire,
    Alsa,
}

/// Every persisted playback preference. Missing fields fall back to
/// [`Self::default`] through `#[serde(default)]`, so a settings row written by
/// an older release never fails to load.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlaybackSettings {
    /// 0 off, 1 one, 2 all (`Repeat::code`).
    pub repeat: i64,
    pub shuffle: bool,
    pub crossfade: bool,
    pub crossfade_seconds: f64,
    /// `equal-power` (default) or `linear`.
    pub crossfade_curve: String,
    pub crossfade_album_only: bool,
    /// 0 off, 1 track, 2 album (`Normalization::code`).
    pub normalization: u8,
    pub eq_profile: Option<String>,
    pub output_device: Option<String>,
    pub follow_default: bool,
    pub exclusive: bool,
    /// `pipewire` (default) or the optional direct ALSA backend.
    pub output_backend: OutputBackend,
    /// ALSA PCM device (`default`, `hw:0,0`, ...); ignored by PipeWire.
    pub alsa_device: String,
    /// 0 = original stream; otherwise the requested kbps.
    pub stream_max_bitrate: u32,
    /// `None`/`original` or a Subsonic format (`mp3`, `opus`, …).
    pub stream_format: Option<String>,
    pub stream_cache: bool,
    pub stream_cache_mib: u64,
    /// Resample every track to `fixed_pipeline_hz` instead of following native
    /// rates. Opt-in: it removes rate-change gaps but sacrifices native-rate
    /// playback/bit-perfect sourcing.
    pub fixed_pipeline_rate: bool,
    pub fixed_pipeline_hz: u32,
    /// Submit now-playing/completed plays to the server.
    pub scrobble: bool,
    /// Bypass EQ, ReplayGain and crossfade (a best-effort bit-perfect path).
    pub bit_perfect: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            repeat: 0,
            shuffle: false,
            crossfade: false,
            crossfade_seconds: crate::player::CrossfadeConfig::DEFAULT_SECONDS,
            crossfade_curve: "equal-power".to_string(),
            crossfade_album_only: false,
            normalization: 0,
            eq_profile: None,
            output_device: None,
            follow_default: true,
            exclusive: false,
            output_backend: OutputBackend::Pipewire,
            alsa_device: "default".to_string(),
            stream_max_bitrate: 0,
            stream_format: None,
            stream_cache: true,
            stream_cache_mib: 2048,
            fixed_pipeline_rate: false,
            fixed_pipeline_hz: 48_000,
            scrobble: true,
            bit_perfect: false,
        }
    }
}

/// One live crossfade-policy change from the command line/UI.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CrossfadeChange {
    Length(f64),
    /// `true` = equal-power, `false` = legacy linear.
    Curve(bool),
    AlbumOnly(bool),
}

impl CrossfadeChange {
    pub fn apply(self, settings: &mut PlaybackSettings) {
        match self {
            Self::Length(seconds) => {
                settings.crossfade_seconds =
                    crate::player::CrossfadeConfig::with_seconds(seconds).seconds;
            }
            Self::Curve(equal_power) => {
                settings.crossfade_curve = if equal_power {
                    "equal-power".to_string()
                } else {
                    "linear".to_string()
                };
            }
            Self::AlbumOnly(enabled) => settings.crossfade_album_only = enabled,
        }
    }
}

/// One live change to the stream-quality policy. The UI/command line sends a
/// single field, so changing quality does not reset format/cache/size.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum StreamChange {
    /// kbps cap; 0 restores the original stream.
    Quality(u32),
    /// `None`/`original` restores the original container.
    Format(Option<String>),
    Cache(bool),
    /// Playback-cache cap in MiB.
    CacheSize(u64),
    /// Force every decoder to one pipeline rate (`None` = native rates).
    FixedRate(Option<u32>),
}

impl StreamChange {
    pub fn apply(self, settings: &mut PlaybackSettings) {
        match self {
            Self::Quality(bitrate) => {
                settings.stream_max_bitrate = PlaybackSettings::clamp_stream_bitrate(bitrate);
            }
            Self::Format(format) => {
                settings.stream_format = PlaybackSettings::normalize_stream_format(format);
            }
            Self::Cache(enabled) => settings.stream_cache = enabled,
            Self::CacheSize(mib) => {
                settings.stream_cache_mib = PlaybackSettings::clamp_cache_mib(mib);
            }
            Self::FixedRate(rate) => {
                settings.fixed_pipeline_rate = rate.is_some();
                if let Some(rate) = rate {
                    settings.fixed_pipeline_hz = PlaybackSettings::clamp_fixed_rate(rate);
                }
            }
        }
    }
}

impl PlaybackSettings {
    pub fn crossfade_config(&self) -> crate::player::CrossfadeConfig {
        crate::player::CrossfadeConfig {
            enabled: self.crossfade,
            seconds: self.crossfade_seconds,
            equal_power: self.crossfade_curve != "linear",
            album_only: self.crossfade_album_only,
        }
        .sanitized()
    }

    fn clamp_stream_bitrate(bitrate: u32) -> u32 {
        bitrate.min(512)
    }

    fn normalize_stream_format(format: Option<String>) -> Option<String> {
        format.and_then(|format| {
            let format = format.trim().to_ascii_lowercase();
            (!format.is_empty() && format != "original").then_some(format)
        })
    }

    fn clamp_cache_mib(mib: u64) -> u64 {
        mib.clamp(64, 65_536)
    }

    fn clamp_fixed_rate(rate: u32) -> u32 {
        rate.clamp(44_100, 192_000)
    }

    /// True when no transcode/bitrate cap is requested.
    pub fn stream_is_original(&self) -> bool {
        self.stream_max_bitrate == 0
            && self
                .stream_format
                .as_deref()
                .is_none_or(|format| format.is_empty() || format == "original")
    }

    /// Identity of the requested stream bytes: the cache row and the media
    /// fingerprint both key on this so a quality change cannot reuse a blob.
    pub fn stream_quality_key(&self) -> String {
        if self.stream_is_original() {
            "original".to_string()
        } else {
            format!(
                "stream:{}:{}",
                self.stream_max_bitrate,
                self.stream_format.as_deref().unwrap_or("original")
            )
        }
    }

    /// Clamp values that come from a hand-edited settings row.
    pub fn sanitized(mut self) -> Self {
        self.repeat = self.repeat.clamp(0, 2);
        self.normalization = self.normalization.min(2);
        let crossfade = crate::player::CrossfadeConfig {
            enabled: self.crossfade,
            seconds: self.crossfade_seconds,
            equal_power: self.crossfade_curve != "linear",
            album_only: self.crossfade_album_only,
        }
        .sanitized();
        self.crossfade = crossfade.enabled;
        self.crossfade_seconds = crossfade.seconds;
        self.crossfade_curve = if crossfade.equal_power {
            "equal-power".to_string()
        } else {
            "linear".to_string()
        };
        self.crossfade_album_only = crossfade.album_only;
        if self.alsa_device.trim().is_empty() {
            self.alsa_device = "default".to_string();
        }
        self.fixed_pipeline_hz = Self::clamp_fixed_rate(self.fixed_pipeline_hz);
        self.stream_format = Self::normalize_stream_format(self.stream_format);
        self.stream_max_bitrate = Self::clamp_stream_bitrate(self.stream_max_bitrate);
        self.stream_cache_mib = Self::clamp_cache_mib(self.stream_cache_mib);
        self
    }
}

/// Reads the one settings document; a missing row or unreadable JSON is the
/// defaults, never a failed daemon start.
pub async fn load(pool: &SqlitePool) -> PlaybackSettings {
    let row: Option<String> =
        sqlx::query_scalar("SELECT value FROM playback_settings WHERE key = ?1")
            .bind(SETTINGS_KEY)
            .fetch_optional(pool)
            .await
            .unwrap_or_default();
    let Some(json) = row else {
        return PlaybackSettings::default();
    };
    match serde_json::from_str::<PlaybackSettings>(&json) {
        Ok(settings) => settings.sanitized(),
        Err(err) => {
            eprintln!("playback settings: ignoring corrupt row: {err:#}");
            PlaybackSettings::default()
        }
    }
}

pub async fn save(pool: &SqlitePool, settings: &PlaybackSettings) -> Result<()> {
    let value =
        serde_json::to_string(&settings.clone().sanitized()).context("encode playback settings")?;
    sqlx::query(
        "INSERT INTO playback_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(SETTINGS_KEY)
    .bind(value)
    .execute(pool)
    .await
    .context("write playback settings")?;
    Ok(())
}
