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

//! Native PipeWire playback engine.
//!
//! The queue is owned here; [`pipewire`] owns the libpipewire sink, the
//! FFmpeg decoder thread and the native clock. [`Player::pump`] mirrors that
//! transport state into the shared snapshot used by QML and MPRIS.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use serde_json::json;

mod pipewire;

/// The sink is created once off the Qt thread at startup; the first play
/// takes it from here instead of connecting PipeWire on the GUI thread.
static WARM_NATIVE: LazyLock<Mutex<Option<Result<pipewire::Native, String>>>> =
    LazyLock::new(|| Mutex::new(None));
static WARM_STARTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn warm_native() {
    if WARM_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let native = pipewire::Native::new();
        *WARM_NATIVE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(native);
    });
}

pub(crate) fn take_warm_native() -> Option<Result<pipewire::Native, String>> {
    WARM_NATIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// A queued track, including the resolved `stream` URL.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// ARC-UI-1: source album id (the queue row's Go to Artist jump);
    /// optional so payloads serialized before the field still deserialize.
    #[serde(default)]
    pub album_id: String,
    /// UI-56: the queue row's favourite mark; optional for old payloads.
    #[serde(default)]
    pub favorite: bool,
    pub duration: f64,
    pub url: String,
    pub cover_art: String,
    pub cover_url: String,
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

/// Snapshot of native transport state, read by QML and MPRIS.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) paused: bool,
    pub(crate) audio_level: f64,
    pub(crate) audio_peak: f64,
    pub(crate) audio_at: Option<std::time::Instant>,
}

impl Shared {
    fn reset(&mut self) {
        self.position = 0.0;
        self.duration = 0.0;
        self.audio_level = 0.0;
        self.audio_peak = 0.0;
        self.audio_at = None;
    }
}

/// Outcome of [`Player::ensure_ready`].
pub(crate) enum Ready {
    Connected,
    Failed,
}

/// Playback queue plus the native transport.
pub struct Player {
    /// Copy-on-write: readers (`emit_queue`, session snapshots) clone an
    /// `Arc`, and only a real row mutation pays for the deep copy. The queue
    /// can hold the whole library, so a full clone just to serialize one
    /// payload was the expensive part.
    pub(crate) queue: Arc<Vec<Track>>,
    pub(crate) index: usize,
    pub(crate) state: State,
    pub(crate) repeat: Repeat,
    pub(crate) shared: Arc<Mutex<Shared>>,
    pub(crate) fallback_duration: f64,
    pub(crate) volume: f64,
    pub(crate) muted: bool,
    pub(crate) retries: u32,
    pub(crate) retry_at: Option<Instant>,
    /// Last transport/decoder error (IPC diagnostics + UI toast source).
    pub(crate) last_error: Option<String>,
    /// Row revision (QUEUEROWSTUDY QR-1): bumped by every mutation of the
    /// queue's rows, so `emit_queue` can skip its O(N) clone/read/serialize
    /// when only the index moved (a skip or an advance).
    pub(crate) revision: u64,
    pub(crate) native: Option<pipewire::Native>,
    /// Next queue index the native decoder is already predecoding.
    gapless_next: Option<usize>,
    #[cfg(test)]
    pub(crate) loads: u32,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Vec::new()),
            index: 0,
            state: State::Stopped,
            repeat: Repeat::Off,
            shared: Arc::new(Mutex::new(Shared::default())),
            fallback_duration: 0.0,
            volume: crate::consts::DEFAULT_VOLUME,
            muted: false,
            retries: 0,
            retry_at: None,
            last_error: None,
            revision: 0,
            native: None,
            gapless_next: None,
            #[cfg(test)]
            loads: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn current(&self) -> Option<&Track> {
        self.queue.get(self.index)
    }

    /// The queue rows' revision (bumped by every row mutation).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Re-resolves every queued track's cover id and URL: a wipe or a sync can
    /// change the album's `coverArt` id under a queue built earlier.
    pub fn refresh_cover_arts(
        &mut self,
        album_covers: &std::collections::HashMap<String, String>,
        covers: &std::collections::HashMap<String, String>,
    ) {
        for track in Arc::make_mut(&mut self.queue).iter_mut() {
            if let Some(cover) = album_covers
                .get(&track.album_id)
                .filter(|cover| !cover.is_empty())
            {
                track.cover_art = cover.clone();
            }
            track.cover_url = covers.get(&track.cover_art).cloned().unwrap_or_default();
        }
        self.revision += 1;
    }

    /// Replaces the queued tracks' metadata from the refreshed library rows:
    /// a provider sync can rename or re-album a track under a queue built
    /// earlier. The resolved URL and cover URLs are intentionally left to
    /// their own refresh paths.
    pub fn refresh_track_metadata(&mut self, songs: &[crate::library::SongRow]) {
        let by_id: std::collections::HashMap<&str, &crate::library::SongRow> =
            songs.iter().map(|song| (song.id.as_str(), song)).collect();
        for track in Arc::make_mut(&mut self.queue).iter_mut() {
            let Some(song) = by_id.get(track.id.as_str()) else {
                continue;
            };
            track.title = song.title.clone();
            track.artist = song.artist.clone().unwrap_or_default();
            track.album = song.album.clone().unwrap_or_default();
            track.album_id = song.album_id.clone().unwrap_or_default();
            track.favorite = song.starred.is_some();
            track.duration = song.duration.unwrap_or(0) as f64;
            track.cover_art = song.cover_art.clone().unwrap_or_default();
        }
        self.revision += 1;
    }

    /// Ids of the next `count` tracks after the current one (LYR-5 prefetch).
    pub fn upcoming(&self, count: usize) -> Vec<String> {
        self.queue
            .iter()
            .skip(self.index + 1)
            .take(count)
            .map(|track| track.id.clone())
            .collect()
    }
    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub(crate) fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn duration(&self) -> f64 {
        let shared = self.shared();
        if shared.duration > 0.0 {
            shared.duration
        } else {
            self.fallback_duration
        }
    }

    pub fn position(&self) -> f64 {
        self.shared().position
    }

    /// True while the current native decoder is still opening its stream.
    pub fn is_loading(&self) -> bool {
        self.native.as_ref().is_some_and(|native| native.loading())
    }

    pub(crate) fn audio_level(&self) -> f64 {
        self.shared().audio_level
    }

    pub(crate) fn audio_peak(&self) -> f64 {
        self.shared().audio_peak
    }

    /// Replaces the queue and starts at `start`.
    pub fn load(&mut self, tracks: Vec<Track>, start: usize) {
        self.repeat = Repeat::Off;
        self.queue = Arc::new(tracks);
        self.index = start.min(self.queue.len().saturating_sub(1));
        self.revision += 1;
        self.play_current();
    }

    /// Replaces the queue and loads `start` paused: a restored session waits
    /// for an explicit Space instead of autoplaying (P-6 S2).
    pub fn load_paused(&mut self, tracks: Vec<Track>, start: usize) {
        self.load(tracks, start);
        self.pause();
    }

    /// Appends tracks to the end of the queue (ARC-43 "Add to Queue"): nothing
    /// starts playing, and an empty queue starts on the next [`Self::toggle`].
    pub fn append(&mut self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        self.queue_mut().extend(tracks);
        self.revision += 1;
    }

    /// (Re)starts the current track from the beginning.
    pub fn play_current(&mut self) {
        if self.queue.is_empty() {
            self.state = State::Stopped;
            return;
        }
        #[cfg(test)]
        {
            self.loads += 1;
        }
        match self.ensure_ready() {
            Ready::Connected => {
                self.rebuild();
            }
            Ready::Failed => self.state = State::Stopped,
        }
    }

    /// Loads the current track into the native decoder/sink.
    fn rebuild(&mut self) {
        let track = self.queue[self.index].clone();
        *self.shared() = Shared {
            duration: track.duration,
            ..Shared::default()
        };
        self.fallback_duration = track.duration;
        self.gapless_next = if self.repeat == Repeat::One {
            Some(self.index)
        } else if self.index + 1 < self.queue.len() {
            Some(self.index + 1)
        } else if self.repeat == Repeat::All && !self.queue.is_empty() {
            Some(0)
        } else {
            None
        };
        self.retries = 0;
        self.retry_at = None;

        let next_url = self
            .gapless_next
            .and_then(|index| self.queue.get(index))
            .map(|next| next.url.clone());
        if let Some(native) = self.native.as_mut() {
            native.load_with_next(&track.url, next_url);
        } else {
            self.command(vec![
                json!("loadfile"),
                json!(track.url.clone()),
                json!("replace"),
            ]);
        }
        self.set_property("pause", json!(false));
        self.state = State::Playing;
    }

    pub fn cycle_repeat(&mut self) -> Repeat {
        self.repeat = self.repeat.cycle();
        self.repeat
    }

    pub fn toggle(&mut self) {
        match self.state {
            State::Playing => self.pause(),
            State::Paused => self.resume(),
            State::Stopped => {
                if !self.queue.is_empty() {
                    self.play_current();
                }
            }
        }
    }

    pub fn pause(&mut self) {
        if self.state != State::Playing {
            return;
        }
        self.set_property("pause", json!(true));
        self.state = State::Paused;
    }

    pub fn resume(&mut self) {
        if self.state != State::Paused {
            return;
        }
        self.set_property("pause", json!(false));
        self.state = State::Playing;
    }

    pub fn stop(&mut self) {
        self.command(vec![json!("stop")]);
        self.shared().reset();
        self.retries = 0;
        self.retry_at = None;
        self.gapless_next = None;
        self.state = State::Stopped;
    }

    pub fn next(&mut self) {
        self.advance_by(1);
    }

    pub fn previous(&mut self) {
        self.retreat_by(1);
    }

    /// Move forward by `steps` rows, loading at most once.
    pub fn advance_by(&mut self, steps: usize) {
        if self.queue.is_empty() || steps == 0 {
            return;
        }
        self.forget_error();
        let mut index = self.index;
        for _ in 0..steps {
            if index + 1 < self.queue.len() {
                index += 1;
            } else if self.repeat == Repeat::All {
                index = 0;
            } else {
                self.stop();
                return;
            }
        }
        if index != self.index {
            self.index = index;
            self.play_current();
        }
    }

    /// Move backward by `steps` rows, loading at most once.
    pub fn retreat_by(&mut self, steps: usize) {
        if self.queue.is_empty() || steps == 0 {
            return;
        }
        self.forget_error();
        let index = self.index.saturating_sub(steps);
        if index != self.index {
            self.index = index;
            self.play_current();
        }
    }

    /// The current queue, in order.
    pub fn queue(&self) -> &[Track] {
        self.queue.as_slice()
    }

    /// A cheap snapshot for a background payload: cloning the `Arc` avoids a
    /// deep copy of every row. The next row mutation pays the copy (COW).
    pub fn queue_snapshot(&self) -> Arc<Vec<Track>> {
        Arc::clone(&self.queue)
    }

    /// Mutable access for a row change; clones the rows only if a snapshot is
    /// still alive.
    fn queue_mut(&mut self) -> &mut Vec<Track> {
        Arc::make_mut(&mut self.queue)
    }

    /// Jumps to and plays the track at `index`.
    pub fn jump(&mut self, index: usize) {
        if index < self.queue.len() {
            self.repeat = Repeat::Off;
            self.index = index;
            self.play_current();
        }
    }

    /// Removes the track at `index` (playing the one that takes its place if it
    /// was current).
    pub fn remove(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        let was_current = index == self.index;
        self.queue_mut().remove(index);
        self.revision += 1;
        if self.queue.is_empty() {
            self.stop();
            return;
        }
        if was_current {
            if self.index >= self.queue.len() {
                self.index = self.queue.len() - 1;
            }
            self.play_current();
            return;
        }
        if index < self.index {
            self.index -= 1;
        }
    }

    /// Removes several tracks in one pass (VI-4 batch delete). Indices are
    /// queue positions as of the call; they are removed from the highest
    /// down so the remaining ones stay valid. Returns how many rows were
    /// actually removed (invalid indices and duplicates ignored).
    pub fn remove_many(&mut self, indices: &[usize]) -> usize {
        let mut indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&index| index < self.queue.len())
            .collect();
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return 0;
        }
        let old_index = self.index;
        let removed_current = indices.binary_search(&old_index).is_ok();
        let below = indices.partition_point(|&index| index < old_index);
        for index in indices.iter().rev() {
            self.queue_mut().remove(*index);
        }
        self.revision += 1;
        if self.queue.is_empty() {
            self.stop();
            return indices.len();
        }
        self.index = if removed_current {
            old_index.saturating_sub(below).min(self.queue.len() - 1)
        } else {
            old_index - below
        };
        if removed_current {
            self.play_current();
        }
        indices.len()
    }

    /// VI-8a: inserts cloned rows at `at` (clamped), the vim `p`/`P` paste.
    /// The current track keeps playing; its cursor shifts when the insert is
    /// at or above it, and the same playhead is reseeked.
    pub fn insert_many(&mut self, at: usize, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        let at = at.min(self.queue.len());
        let count = tracks.len();
        self.queue_mut().splice(at..at, tracks);
        if at <= self.index {
            self.index += count;
        }
        self.revision += 1;
    }

    /// Moves the track at `from` to position `to`. When the move crosses the
    /// current row, the transport re-anchors at the same playhead.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.queue.len() || to >= self.queue.len() || from == to {
            return;
        }
        let queue = self.queue_mut();
        let track = queue.remove(from);
        queue.insert(to, track);
        self.revision += 1;
        if self.index == from {
            self.index = to;
        } else {
            if from < self.index {
                self.index -= 1;
            }
            if to <= self.index {
                self.index += 1;
            }
        }
    }

    /// Seeks to an absolute position (seconds).
    pub fn seek(&mut self, seconds: f64) {
        let seconds = seconds.max(0.0);
        self.command(vec![json!("seek"), json!(seconds), json!("absolute")]);
        self.shared().position = seconds;
    }

    /// Sets volume (0-100).
    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 100.0);
        self.set_property("volume", json!(self.volume));
    }

    /// Mutes or unmutes without touching the volume value.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.set_property("mute", json!(self.muted));
    }

    pub fn toggle_mute(&mut self) {
        self.set_muted(!self.muted);
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    fn tracks(count: usize) -> Vec<Track> {
        (0..count)
            .map(|index| Track {
                id: format!("song-{index}"),
                title: format!("Song {index}"),
                artist: String::new(),
                album: String::new(),
                album_id: String::new(),
                favorite: false,
                duration: 0.0,
                url: String::new(),
                cover_art: String::new(),
                cover_url: String::new(),
            })
            .collect()
    }

    fn player(count: usize) -> Player {
        let mut player = Player::new();
        player.queue = Arc::new(tracks(count));
        player.state = State::Playing;
        player
    }

    #[test]
    fn advance_by_loads_once() {
        let mut player = player(10);
        player.advance_by(5);
        assert_eq!(player.index(), 5);
        assert_eq!(player.loads, 1);
    }

    #[test]
    fn insert_above_current_does_not_reload() {
        let mut player = player(10);
        player.index = 5;
        player.insert_many(0, tracks(2));
        assert_eq!(player.index(), 7);
        assert_eq!(player.loads, 0);
    }

    #[test]
    fn reorder_across_current_does_not_reload() {
        let mut player = player(10);
        player.index = 5;
        player.move_item(0, 7);
        assert_eq!(player.index(), 4);
        assert_eq!(player.loads, 0);
    }

    #[test]
    fn removing_current_loads_once() {
        let mut player = player(10);
        player.index = 5;
        player.remove_many(&[5]);
        assert_eq!(player.loads, 1);
    }
}

#[cfg(test)]
mod repeat_tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            album: String::new(),
            album_id: String::new(),
            favorite: false,
            duration: 0.0,
            url: String::new(),
            cover_art: String::new(),
            cover_url: String::new(),
        }
    }

    #[test]
    fn loading_a_queue_clears_repeat() {
        let mut player = Player::new();
        player.repeat = Repeat::All;
        player.load(vec![track("a"), track("b")], 0);
        assert_eq!(player.repeat, Repeat::Off);
    }

    #[test]
    fn jumping_clears_repeat() {
        let mut player = Player::new();
        player.queue = Arc::new(vec![track("a"), track("b")]);
        player.repeat = Repeat::One;
        player.jump(1);
        assert_eq!(player.repeat, Repeat::Off);
    }
}
