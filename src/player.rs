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

//! Playback engine built on an `mpv` subprocess controlled over its JSON IPC
//! socket.
//!
//! mpv decodes with FFmpeg and exposes real `time-pos`/`duration`/`pause`
//! state, plus seeking, so the UI gets an accurate progress bar. Unlike the
//! previous `ffplay` approach there is no wall-clock guessing and no
//! SIGSTOP hack: mpv runs in `--idle` mode for the whole session.
//!
//! Q-5 window model (`PLAYBACKSTUDY.md`): the current track is played as an
//! mpv *playlist* — the current entry plus the next one appended once the
//! current reports `file-loaded` — so mpv advances gap-free with
//! `--gapless-audio=yes` and a cache-bounded `--prefetch-playlist=yes`.
//! A load failure has no next entry to auto-advance to, so it can be
//! retried in place with `playlist-play-index` before the queue moves on.
//! The Rust queue stays the source of truth; `playlist-pos` maps back into
//! it (`index = window_base + pos`).
//!
//! A small reader thread owns the read half of the socket and mirrors mpv's
//! observed properties and events into a shared [`Shared`] snapshot that the
//! Qt thread reads during [`Player::pump`]. Stale `end-file` events from
//! replaced files are dropped while a load is pending, so fast skips never
//! depend on network timing (the queue advances on keypress, not on the
//! stream).

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::json;

mod mpv;

#[cfg(test)]
pub(crate) use mpv::apply_event;

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

/// A failed `end-file` for the UI thread to act on (Q-5).
#[derive(Debug, Clone)]
pub(crate) struct ErrorInfo {
    /// mpv's `file_error` string ("loading failed", …).
    pub(crate) message: String,
    /// `true` when the file had already produced media (`position > 0`):
    /// mid-track failures advance, load failures retry.
    pub(crate) played: bool,
}

/// Snapshot of mpv state, written by the reader thread and read by the UI.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) position: f64,
    pub(crate) duration: f64,
    pub(crate) paused: bool,
    /// The current entry's file loaded successfully (`file-loaded`).
    pub(crate) loaded: bool,
    /// mpv playlist index of the playing entry (`-1`/`None` when idle).
    pub(crate) playlist_pos: Option<i64>,
    /// mpv entered idle mode (end of the window).
    pub(crate) idle: bool,
    /// Set once per failed `end-file`, consumed by [`Player::pump`].
    pub(crate) error: Option<ErrorInfo>,
    /// `true` between issuing a load and mpv reporting `start-file`.
    /// While set, `end-file` events belong to the file we deliberately
    /// replaced and must not advance the queue (fast skip/previous races).
    pub(crate) pending_load: bool,
    /// Playback envelope from mpv astats metadata (0..1).
    pub(crate) audio_level: f64,
    pub(crate) audio_peak: f64,
    pub(crate) audio_at: Option<std::time::Instant>,
}

impl Shared {
    fn reset(&mut self) {
        self.position = 0.0;
        self.duration = 0.0;
        self.error = None;
        self.audio_level = 0.0;
        self.audio_peak = 0.0;
        self.audio_at = None;
    }
}

/// `playlist-play-index` attempts after a failed load before advancing (Q-5).

/// FIX-1/FIX-11: outcome of [`Player::ensure_ready`].
pub(crate) enum Ready {
    /// The IPC socket is connected; commands can flow.
    Connected,
    /// mpv is spawned (or resurrected from the previous attempt); its socket
    /// is not up yet. The start is deferred to the pump.
    Connecting,
    /// The spawn failed, or the connect deadline passed and the child was
    /// reaped. Nothing is attached; a later start re-spawns.
    Failed,
}

/// Playback queue plus the running `mpv` process and its IPC connection.
pub struct Player {
    /// Copy-on-write: readers (`emit_queue`, session snapshots) clone an
    /// `Arc`, and only a real row mutation pays for the deep copy. The queue
    /// can hold the whole library, so a full clone just to serialize one
    /// payload was the expensive part.
    pub(crate) queue: Arc<Vec<Track>>,
    pub(crate) index: usize,
    pub(crate) state: State,
    pub(crate) repeat: Repeat,
    pub(crate) child: Option<Child>,
    pub(crate) ipc: Option<UnixStream>,
    pub(crate) socket: Option<PathBuf>,
    pub(crate) shared: Arc<Mutex<Shared>>,
    pub(crate) request_id: i64,
    pub(crate) fallback_duration: f64,
    pub(crate) volume: f64,
    pub(crate) muted: bool,
    /// Queue index of mpv playlist entry 0; `index - window_base` is the
    /// current entry's playlist position. Fixed between rebuilds.
    pub(crate) window_base: usize,
    /// Entries currently in the mpv playlist.
    pub(crate) window_len: usize,
    /// Retries already spent on the current load (Q-5).
    pub(crate) retries: u32,
    /// When the next retry is due (backoff between attempts).
    pub(crate) retry_at: Option<Instant>,
    /// FIX-11: when the current deferred connect attempt times out; `None`
    /// once connected or reaped.
    pub(crate) connect_deadline: Option<Instant>,
    /// FIX-11: `play_current` is waiting for the socket; rebuild on the pump.
    pub(crate) deferred_start: bool,
    /// Row revision (QUEUEROWSTUDY QR-1): bumped by every mutation of the
    /// queue's rows, so `emit_queue` can skip its O(N) clone/read/serialize
    /// when only the index moved (a skip or an advance).
    pub(crate) revision: u64,
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
            child: None,
            ipc: None,
            socket: None,
            shared: Arc::new(Mutex::new(Shared::default())),
            request_id: 0,
            fallback_duration: 0.0,
            volume: crate::consts::DEFAULT_VOLUME,
            muted: false,
            window_base: 0,
            window_len: 0,
            retries: 0,
            retry_at: None,
            connect_deadline: None,
            deferred_start: false,
            revision: 0,
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

    pub(crate) fn audio_level(&self) -> f64 {
        self.shared().audio_level
    }

    pub(crate) fn audio_peak(&self) -> f64 {
        self.shared().audio_peak
    }

    /// Replaces the queue and starts at `start`.
    pub fn load(&mut self, tracks: Vec<Track>, start: usize) {
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
        match self.ensure_ready() {
            Ready::Connected => {
                self.deferred_start = false;
                self.rebuild();
            }
            Ready::Connecting => {
                // FIX-11: the socket is still coming up; the rebuild rides
                // the pump instead of sleeping here (UI pumping).
                self.state = State::Playing;
                self.deferred_start = true;
                *self.shared() = Shared::default();
            }
            Ready::Failed => self.state = State::Stopped,
        }
    }

    /// Resolves a deferred start now that the socket is live. While the
    /// connect was pending, `pause`/`seek` commands were dropped with no
    /// socket — replay that intent after the rebuild, so a restored session
    /// still comes up paused at its saved playhead instead of autoplaying.
    pub(crate) fn finish_deferred_start(&mut self) {
        let paused = self.state == State::Paused;
        let position = self.shared().position;
        self.deferred_start = false;
        self.rebuild();
        if paused {
            self.pause();
        }
        if position > 0.0 {
            self.seek(position);
        }
    }

    /// Loads the current track as a fresh one-entry playlist window. A
    /// `loadfile … replace` clears whatever was queued (`PLAYBACKSTUDY.md`
    /// §4); the tail entry is appended by [`Self::refill`] once this one is
    /// loaded, so a load failure has nothing to auto-advance to.
    fn rebuild(&mut self) {
        let track = self.queue[self.index].clone();
        *self.shared() = Shared {
            duration: track.duration,
            pending_load: true,
            ..Shared::default()
        };
        self.fallback_duration = track.duration;
        self.window_base = self.index;
        self.window_len = 1;
        self.retries = 0;
        self.retry_at = None;

        self.command(vec![
            json!("loadfile"),
            json!(track.url.clone()),
            json!("replace"),
        ]);
        self.push_metadata(&track);
        self.apply_repeat();
        self.set_property("pause", json!(false));
        self.state = State::Playing;
    }

    /// `mpris:artUrl` / `metadata` feed mpv's MPRIS (when the plugin is
    /// loaded); position and pause state follow automatically.
    fn push_metadata(&mut self, track: &Track) {
        let metadata = json!({
            "title": track.title,
            "artist": track.artist,
            "album": track.album,
            "artUrl": track.cover_url,
            "mpris:artUrl": track.cover_url,
        });
        self.set_property("metadata", metadata);
    }

    pub fn cycle_repeat(&mut self) -> Repeat {
        self.repeat = self.repeat.cycle();
        self.apply_repeat();
        self.repeat
    }

    fn apply_repeat(&mut self) {
        let mode = if self.repeat == Repeat::One { "inf" } else { "no" };
        self.set_property("loop-file", json!(mode));
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
        {
            let mut shared = self.shared();
            shared.reset();
            shared.idle = false;
            shared.playlist_pos = None;
        }
        self.window_base = self.index;
        self.window_len = 0;
        self.retries = 0;
        self.retry_at = None;
        self.deferred_start = false;
        self.state = State::Stopped;
    }

    pub fn next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.forget_error();
        let local = self.index - self.window_base;
        if local + 1 < self.window_len {
            // The next entry is already in the mpv playlist: it advances
            // natively (prefetched, cache-bounded), so the skip stays gapless.
            self.index += 1;
            self.command(vec![json!("playlist-next")]);
            self.set_property("pause", json!(false));
            self.state = State::Playing;
            let track = self.queue[self.index].clone();
            self.push_metadata(&track);
        } else if self.index + 1 < self.queue.len() {
            self.index += 1;
            self.play_current();
        } else if self.repeat == Repeat::All {
            self.index = 0;
            self.play_current();
        } else {
            self.stop();
        }
    }

    pub fn previous(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.forget_error();
        let target = self.index.saturating_sub(1);
        self.index = target;
        if target >= self.window_base && target - self.window_base < self.window_len {
            // The previous entry is still in the window: replay it in place.
            let local = target - self.window_base;
            self.command(vec![json!("playlist-play-index"), json!(local)]);
            self.set_property("pause", json!(false));
            self.state = State::Playing;
            let track = self.queue[target].clone();
            self.push_metadata(&track);
        } else {
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
        if index >= self.window_base {
            // In the mpv playlist (current or ahead): drop it there too.
            let local = index - self.window_base;
            if local < self.window_len {
                self.command(vec![json!("playlist-remove"), json!(local)]);
                self.window_len -= 1;
            }
        } else {
            // Before the window: entry 0 shifts down one queue index.
            self.window_base -= 1;
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
        let count = indices.len();
        for index in indices.into_iter().rev() {
            self.remove(index);
        }
        count
    }

    /// VI-8a: inserts cloned rows at `at` (clamped), the vim `p`/`P` paste.
    /// The current track keeps playing: its cursor shifts when the insert is
    /// at or above it, and an insert inside the mpv window re-anchors the
    /// window at the same playhead (the `move_item` contract).
    pub fn insert_many(&mut self, at: usize, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }
        let at = at.min(self.queue.len());
        let count = tracks.len();
        let playing = self.state != State::Stopped;
        let window_end = self.window_base + self.window_len;
        let touched = playing && (at < window_end || at <= self.index);
        let position = self.position();
        let paused = self.state == State::Paused;

        self.queue_mut().splice(at..at, tracks);
        if at <= self.index {
            self.index += count;
        }
        if touched {
            self.play_current();
            if paused {
                self.pause();
            }
            self.seek(position);
        }
        self.revision += 1;
    }

    /// Moves the track at `from` to position `to`.
    ///
    /// Reordering re-anchors the window when the move touches it (or the
    /// current entry) and seeks back to the playhead; a move that cannot
    /// reorder the mpv playlist is left to Rust alone.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.queue.len() || to >= self.queue.len() || from == to {
            return;
        }
        let queue = self.queue_mut();
        let track = queue.remove(from);
        queue.insert(to, track);
        self.revision += 1;
        // Track the current index across the reorder.
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

        if self.state == State::Stopped {
            return;
        }
        let end = self.window_base + self.window_len;
        let crosses = (from < self.window_base) != (to < self.window_base);
        let touched = crosses || [from, to].iter().any(|&i| i >= self.window_base && i < end);
        if touched {
            let position = self.position();
            let paused = self.state == State::Paused;
            self.play_current();
            if paused {
                self.pause();
            }
            self.seek(position);
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
