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

#![expect(
    dead_code,
    reason = "the first phases persist the settings shell; later playback phases consume the fields as they land"
)]

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;

const SETTINGS_KEY: &str = "playback";

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
    /// 0 = original stream; otherwise the requested kbps.
    pub stream_max_bitrate: u32,
    /// `None`/`original` or a Subsonic format (`mp3`, `opus`, …).
    pub stream_format: Option<String>,
    pub stream_cache: bool,
    pub stream_cache_mib: u64,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            repeat: 0,
            shuffle: false,
            crossfade: false,
            crossfade_seconds: 5.0,
            crossfade_curve: "equal-power".to_string(),
            crossfade_album_only: false,
            normalization: 0,
            eq_profile: None,
            output_device: None,
            follow_default: true,
            exclusive: false,
            stream_max_bitrate: 0,
            stream_format: None,
            stream_cache: true,
            stream_cache_mib: 2048,
        }
    }
}

impl PlaybackSettings {
    /// Clamp values that come from a hand-edited settings row.
    pub fn sanitized(mut self) -> Self {
        self.repeat = self.repeat.clamp(0, 2);
        self.normalization = self.normalization.min(2);
        self.crossfade_seconds = self.crossfade_seconds.clamp(0.0, 12.0);
        if self.crossfade_curve != "linear" {
            self.crossfade_curve = "equal-power".to_string();
        }
        self.stream_max_bitrate = self.stream_max_bitrate.min(512);
        self.stream_cache_mib = self.stream_cache_mib.clamp(64, 65_536);
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
