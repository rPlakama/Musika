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

//! Playback model and native transport building blocks.
//!
//! [`Track`], [`Repeat`] and [`State`] are the queue/transport vocabulary shared
//! by the core, the engine and the IPC protocol. [`core`] owns the pure queue
//! policy, [`engine`] owns the event-driven audio thread, and [`pipewire`]
//! owns the libpipewire sink plus the FFmpeg decode workers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

pub(crate) mod core;
pub(crate) mod engine;
pub(crate) mod pipewire;

/// The transport is created once at startup, off the daemon's async runtime;
/// the playback engine takes it from here instead of connecting PipeWire again.
static WARM_NATIVE: LazyLock<Mutex<Option<Result<pipewire::Native, String>>>> =
    LazyLock::new(|| Mutex::new(None));
static WARM_STARTED: AtomicBool = AtomicBool::new(false);
/// Set once the warm-up thread stored its result in `WARM_NATIVE`.
static WARM_DONE: AtomicBool = AtomicBool::new(false);

pub(crate) fn warm_native() {
    if WARM_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let native = pipewire::Native::new();
        *WARM_NATIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(native);
        WARM_DONE.store(true, Ordering::SeqCst);
    });
}

/// True while the startup warm-up thread is still creating the transport.
///
/// The event-driven engine consumes the warm transport instead of connecting a
/// second time; this predicate lets it wait for an in-flight warm-up rather
/// than racing it (a second `Native` would leak an idle PipeWire stream).
pub(crate) fn warm_native_pending() -> bool {
    WARM_STARTED.load(Ordering::SeqCst) && !WARM_DONE.load(Ordering::SeqCst)
}

pub(crate) fn take_warm_native() -> Option<Result<pipewire::Native, String>> {
    WARM_NATIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// A queued track, including the resolved `stream` URL. The shared
/// [`crate::model::ItemMeta`] carries the identity; playback adds the stream
/// extras. Field reads go through `Deref`, so `track.title` stays valid.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Track {
    #[serde(flatten)]
    pub item: crate::model::ItemMeta,
    pub artist: String,
    pub album: String,
    /// UI-56: the queue row's favourite mark; optional for old payloads.
    #[serde(default)]
    pub favorite: bool,
    pub duration: f64,
    pub url: String,
    pub cover_art: String,
}

impl std::ops::Deref for Track {
    type Target = crate::model::ItemMeta;

    fn deref(&self) -> &Self::Target {
        &self.item
    }
}

impl std::ops::DerefMut for Track {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.item
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

impl Repeat {
    pub fn code(self) -> i64 {
        match self {
            Self::Off => 0,
            Self::One => 1,
            Self::All => 2,
        }
    }

    fn cycle(self) -> Self {
        match self {
            Self::Off => Self::One,
            Self::One => Self::All,
            Self::All => Self::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Playing => "playing",
            Self::Paused => "paused",
        }
    }

    pub fn mpris_status(self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Playing => "Playing",
            Self::Paused => "Paused",
        }
    }
}
