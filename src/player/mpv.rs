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

//! Lightweight single-process MPV playback transport over Unix JSON IPC.
//!
//! One persistent `mpv --idle=yes --no-video --no-config` child process owns
//! demuxing, decoding, ReplayGain, `lavfi` Equalizer APO filters, native
//! gapless audio, and PipeWire/ALSA output. Rust communicates with it over a
//! private Unix domain socket with zero audio FFI bindings or custom PCM rings.

pub(crate) mod process;
pub(crate) mod protocol;
pub(crate) mod transport;

use std::process::Command;
use std::sync::{LazyLock, Mutex};

pub(crate) use transport::MpvTransport;

use crate::player::OutputDevice;

/// Live output device snapshot updated whenever the running `mpv` instance
/// emits an `audio-device-list` property change. Reading from this cache avoids
/// spawning a helper subprocess for `:output list` or unplug checks while the
/// engine is alive.
static CACHED_DEVICES: LazyLock<Mutex<Option<Vec<OutputDevice>>>> =
    LazyLock::new(|| Mutex::new(None));

pub(crate) fn update_cached_devices(devices: Vec<OutputDevice>) {
    *CACHED_DEVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(devices);
}

/// Enumerates audio output devices known to MPV.
///
/// Uses the live engine's observed `audio-device-list` when available; falls
/// back to a one-shot `mpv --audio-device=help` probe when no engine is active.
pub(crate) fn list_output_devices() -> Result<Vec<OutputDevice>, String> {
    if let Some(devices) = CACHED_DEVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    {
        return Ok(devices);
    }

    let bin = process::mpv_binary();
    let output = Command::new(&bin)
        .args(["--no-config", "--audio-device=help"])
        .output()
        .map_err(|error| format!("cannot run {bin:?} to list audio devices: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "mpv --audio-device=help failed ({}): {}",
            output.status,
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(protocol::parse_audio_device_help(&stdout))
}
