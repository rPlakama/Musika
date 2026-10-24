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

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod sys {
    use std::ffi::c_int;

    pub const SIGTERM: c_int = 15;
    pub const PR_SET_PDEATHSIG: c_int = 1;

    unsafe extern "C" {
        pub fn prctl(option: c_int, ...) -> c_int;
        pub fn getppid() -> c_int;
        pub fn kill(pid: c_int, sig: c_int) -> c_int;
    }
}

/// A queued track, including the resolved `stream` URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
struct ErrorInfo {
    /// mpv's `file_error` string ("loading failed", …).
    message: String,
    /// `true` when the file had already produced media (`position > 0`):
    /// mid-track failures advance, load failures retry.
    played: bool,
}

/// Snapshot of mpv state, written by the reader thread and read by the UI.
#[derive(Debug, Default)]
struct Shared {
    position: f64,
    duration: f64,
    paused: bool,
    /// The current entry's file loaded successfully (`file-loaded`).
    loaded: bool,
    /// mpv playlist index of the playing entry (`-1`/`None` when idle).
    playlist_pos: Option<i64>,
    /// mpv entered idle mode (end of the window).
    idle: bool,
    /// Set once per failed `end-file`, consumed by [`Player::pump`].
    error: Option<ErrorInfo>,
    /// `true` between issuing a load and mpv reporting `start-file`.
    /// While set, `end-file` events belong to the file we deliberately
    /// replaced and must not advance the queue (fast skip/previous races).
    pending_load: bool,
}

impl Shared {
    fn reset(&mut self) {
        self.position = 0.0;
        self.duration = 0.0;
        self.error = None;
    }
}

/// `playlist-play-index` attempts after a failed load before advancing (Q-5).
const MAX_RETRIES: u32 = 2;
/// Backoff before a retry (the first retry is immediate).
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// FIX-11: how long a spawned mpv may take to expose its IPC socket before
/// the child is reaped and the next start re-spawns it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// FIX-1/FIX-11: outcome of [`Player::ensure_ready`].
enum Ready {
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
    queue: Vec<Track>,
    index: usize,
    state: State,
    child: Option<Child>,
    ipc: Option<UnixStream>,
    socket: Option<PathBuf>,
    shared: Arc<Mutex<Shared>>,
    request_id: i64,
    fallback_duration: f64,
    volume: f64,
    muted: bool,
    /// Queue index of mpv playlist entry 0; `index - window_base` is the
    /// current entry's playlist position. Fixed between rebuilds.
    window_base: usize,
    /// Entries currently in the mpv playlist.
    window_len: usize,
    /// Retries already spent on the current load (Q-5).
    retries: u32,
    /// When the next retry is due (backoff between attempts).
    retry_at: Option<Instant>,
    /// FIX-11: when the current deferred connect attempt times out; `None`
    /// once connected or reaped.
    connect_deadline: Option<Instant>,
    /// FIX-11: `play_current` is waiting for the socket; rebuild on the pump.
    deferred_start: bool,
    /// Row revision (QUEUEROWSTUDY QR-1): bumped by every mutation of the
    /// queue's rows, so `emit_queue` can skip its O(N) clone/read/serialize
    /// when only the index moved (a skip or an advance).
    revision: u64,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            index: 0,
            state: State::Stopped,
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
        for track in &mut self.queue {
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
        for track in &mut self.queue {
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

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap()
    }

    fn set_property(&mut self, name: &str, value: Value) {
        self.command(vec![json!("set_property"), json!(name), value]);
    }

    fn observe(&mut self, id: u32, name: &str) {
        self.command(vec![json!("observe_property"), json!(id), json!(name)]);
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

    /// Replaces the queue and starts at `start`.
    pub fn load(&mut self, tracks: Vec<Track>, start: usize) {
        self.queue = tracks;
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
        self.queue.extend(tracks);
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
    fn finish_deferred_start(&mut self) {
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

        self.command(vec![json!("loadfile"), json!(track.url.clone()), json!("replace")]);
        self.push_metadata(&track);
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
        &self.queue
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
        self.queue.remove(index);
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
    /// down so the remaining ones stay valid.
    pub fn remove_many(&mut self, indices: &[usize]) {
        let mut indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&index| index < self.queue.len())
            .collect();
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            self.remove(index);
        }
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
        let track = self.queue.remove(from);
        self.queue.insert(to, track);
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
        let touched = crosses
            || [from, to]
                .iter()
                .any(|&i| i >= self.window_base && i < end);
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

    /// Q-5 pump: mirrors mpv's native playlist advances into `index`, drains
    /// failed loads (retry in place, then advance) and keeps the window's
    /// one-entry lookahead. Returns a user-facing toast when a track failed.
    pub fn pump(&mut self) -> Option<String> {
        // FIX-11: finish a deferred start once the IPC socket is up; the
        // rebuild waits for readiness instead of the Qt thread.
        if self.deferred_start && self.state != State::Stopped {
            match self.ensure_ready() {
                Ready::Connected => {
                    self.finish_deferred_start();
                    return None;
                }
                Ready::Failed => {
                    self.deferred_start = false;
                    self.state = State::Stopped;
                    return None;
                }
                Ready::Connecting => return None,
            }
        }
        if self.state == State::Stopped {
            return None;
        }

        if let Some(at) = self.retry_at {
            if Instant::now() >= at && !self.pending_load() {
                self.retry_at = None;
                self.reload_current();
            }
        }

        let (pending, pos, loaded, idle, error) = {
            let mut shared = self.shared();
            (
                shared.pending_load,
                shared.playlist_pos,
                shared.loaded,
                shared.idle,
                shared.error.take(),
            )
        };
        if pending {
            return None;
        }

        let mut toast = None;
        let mut advanced = false;
        let mut retried = false;

        // A native mpv advance moves `playlist-pos`;
        // map it back onto the queue.
        if let Some(p) = pos {
            if p >= 0 && (p as usize) < self.window_len {
                let mapped = self.window_base + p as usize;
                if mapped != self.index && mapped < self.queue.len() {
                    self.index = mapped;
                    self.retries = 0;
                    self.retry_at = None;
                    advanced = true;
                }
            }
        }

        if let Some(err) = error {
            let title = self.current().map(|t| t.title.clone()).unwrap_or_default();
            if err.played || advanced {
                // Mid-track failure, or mpv already skipped the entry.
                if !advanced {
                    self.advance_after_error();
                    advanced = true;
                }
                toast = Some(format!("{title}: {}", err.message));
            } else if self.retries < MAX_RETRIES {
                self.retries += 1;
                if self.retries == 1 {
                    self.reload_current();
                    retried = true;
                } else {
                    self.retry_at = Some(Instant::now() + RETRY_BACKOFF);
                }
                toast = Some(format!(
                    "{title}: {} (retry {}/{MAX_RETRIES})",
                    err.message, self.retries
                ));
            } else {
                self.advance_after_error();
                advanced = true;
                toast = Some(format!("{title}: {}", err.message));
            }
        }

        if advanced && self.state != State::Stopped {
            let track = self.queue[self.index].clone();
            self.push_metadata(&track);
        }
        if idle && !advanced && !retried && self.retry_at.is_none() {
            // The window drained without an error: short track that outran
            // its lookahead, or the queue ended. Advance or stop.
            if self.index + 1 < self.queue.len() {
                self.index += 1;
                self.play_current();
            } else {
                self.state = State::Stopped;
            }
        } else if loaded && !advanced {
            self.refill();
        }
        toast
    }

    // -- internals ---------------------------------------------------------

    /// Keeps exactly one entry after the current one in the mpv playlist,
    /// once the current file is loaded: a load failure then has nothing to
    /// auto-advance to, so [`Self::reload_current`] can retry it in place.
    fn refill(&mut self) {
        let next_local = self.index - self.window_base + 1;
        if next_local >= self.window_len && self.window_base + next_local < self.queue.len() {
            let url = self.queue[self.window_base + next_local].url.clone();
            self.command(vec![json!("loadfile"), json!(url), json!("append")]);
            self.window_len += 1;
        }
    }

    /// Replays the current playlist entry (Q-5 retry); `playlist-play-index`
    /// reloads it without touching the rest of the playlist.
    fn reload_current(&mut self) {
        let local = self.index - self.window_base;
        {
            let mut shared = self.shared();
            shared.reset();
            shared.loaded = false;
            shared.idle = false;
        }
        self.command(vec![json!("playlist-play-index"), json!(local)]);
    }

    /// Moves on after an unrecoverable failure: the next queue track, or stop
    /// at the end.
    fn advance_after_error(&mut self) {
        if self.index + 1 < self.queue.len() {
            self.index += 1;
            self.play_current();
        } else {
            self.stop();
        }
    }

    fn pending_load(&self) -> bool {
        self.shared().pending_load
    }

    /// Explicit user moves supersede any recorded failure.
    fn forget_error(&mut self) {
        self.retries = 0;
        self.retry_at = None;
        self.shared().error = None;
    }

    /// FIX-1: reaps a spawned mpv whose IPC socket never came up (or was
    /// replaced by a later attempt), so a retried start cannot orphan the
    /// process and its reader thread.
    fn reap_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            // mpv shuts down cleanly on TERM; `child.kill()` would SIGKILL it.
            signal(child.id(), sys::SIGTERM);
            let _ = child.wait();
        }
        if let Some(socket) = self.socket.take() {
            crate::ipc::remove_stale_socket(&socket);
        }
        self.connect_deadline = None;
    }

    /// Readiness state machine (FIX-1/FIX-11): the child is stored only
    /// once its socket connects, a failed spawn/connect inverts to no
    /// child, and the sleeping retry loop is gone — one connect attempt
    /// per call, deferred to the pump.
    ///
    /// mpv is our direct child and the kernel TERMs it if we die (even on
    /// SIGKILL, where Drop never runs): PR_SET_PDEATHSIG in `pre_exec`, no
    /// supervisor shell. PDEATHSIG follows the spawning thread, and
    /// `ensure_ready` runs on the Qt thread, which lives for the process.
    fn ensure_ready(&mut self) -> Ready {
        if self.ipc.is_some() {
            return Ready::Connected;
        }

        let socket = crate::db::config_socket("mpv.sock");

        if self.child.is_none() {
            crate::ipc::remove_stale_socket(&socket);
            let parent = std::process::id();
            let socket_arg = format!("--input-ipc-server={}", socket.display());

            let mut command = Command::new("mpv");
            command
                .arg("--no-video")
                .arg("--idle=yes")
                .arg("--no-terminal")
                .arg("--really-quiet")
                // Hardware decode when the backend/source supports it; the
                // safe variant falls back to software instead of failing
                // playback. (The player is audio-only, so this matters for
                // video-bearing streams and codecs with a hw path; audio
                // keeps FFmpeg.)
                .arg("--hwdec=auto-safe")
                // Q-5: gap-free playlist transitions and HTTP drop recovery
                // (`PLAYBACKSTUDY.md` §2/§4). The prefetch is capped: the
                // demuxer caches default to 150+50 MiB per demuxer, and the
                // prefetched entry is a second one.
                .arg("--gapless-audio=yes")
                .arg("--prefetch-playlist=yes")
                .arg("--demuxer-max-bytes=32MiB")
                .arg("--demuxer-max-back-bytes=8MiB")
                .arg("--stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=5")
                .arg(&socket_arg);
            if let Some(script) = crate::mpris::mpris_script() {
                command.arg(format!("--script={script}"));
            }
            unsafe {
                command.pre_exec(move || {
                    if sys::prctl(sys::PR_SET_PDEATHSIG, sys::SIGTERM) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    // The parent may have died between fork and prctl;
                    // PDEATHSIG would not fire then, so bail out instead of
                    // leaking.
                    if sys::getppid() as u32 != parent {
                        return Err(std::io::Error::other("parent died before spawn"));
                    }
                    Ok(())
                });
            }

            match command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    // Stored before the connect, but never orphaned: the
                    // deadline path reaps it (FIX-1) and `Drop` covers the
                    // rest of the lifetime.
                    self.child = Some(child);
                    self.socket = Some(socket.clone());
                    self.connect_deadline = Some(Instant::now() + CONNECT_TIMEOUT);
                }
                Err(err) => {
                    eprintln!("player: could not start mpv: {err}");
                    return Ready::Failed;
                }
            }
        }

        // FIX-11: one connect attempt per call — a Unix socket connect on
        // a missing socket file returns immediately (ENOENT), so this never
        // blocks the Qt thread.
        match UnixStream::connect(&socket) {
            Ok(stream) => {
                self.connect_deadline = None;
                self.finish_connect(stream);
                Ready::Connected
            }
            Err(_) => {
                let deadline_exceeded = self
                    .connect_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline);
                if deadline_exceeded {
                    self.reap_child();
                    return Ready::Failed;
                }
                Ready::Connecting
            }
        }
    }

    /// Finishes the connect: the reader thread, the observed properties and
    /// the volume/mute carried over from the previous session.
    fn finish_connect(&mut self, stream: UnixStream) {
        if let Ok(reader) = stream.try_clone() {
            let shared = Arc::clone(&self.shared);
            crate::ipc::spawn_line_reader(reader, move |line| {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    return;
                };
                let mut shared = shared.lock().unwrap();
                apply_event(&mut shared, &value);
            });
        }
        self.ipc = Some(stream);

        // Observe the properties the UI needs, then apply volume and mute.
        self.observe(1, "time-pos");
        self.observe(2, "duration");
        self.observe(3, "pause");
        self.observe(4, "playlist-pos");
        self.set_volume(self.volume);
        self.set_muted(self.muted);
    }

    fn command(&mut self, args: Vec<Value>) -> bool {
        self.request_id += 1;
        let message = json!({ "command": args, "request_id": self.request_id });
        let mut line = message.to_string();
        line.push('\n');

        match &mut self.ipc {
            Some(ipc) => ipc.write_all(line.as_bytes()).is_ok(),
            None => false,
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.ipc = None;
        self.reap_child();
    }
}

fn signal(pid: u32, signal: std::ffi::c_int) {
    // Native kill(2), no `kill` binary spawn (ARC-16).
    unsafe {
        sys::kill(pid as std::ffi::c_int, signal);
    }
}

/// Mirrors one mpv event into [`Shared`].
///
/// `start-file` marks the load we requested as actually begun: it clears the
/// pending guard and drops the stale position/duration of the replaced file.
/// `end-file` events are ignored while a load is pending, because they belong
/// to the file that load replaced — this is what keeps fast skips/previous
/// from bouncing back into a track the user already moved away from. A
/// `file_error` is kept for [`Player::pump`] instead of acted on here;
/// `stop` (our own skip) is ignored, mpv's playlist moves on by itself.
fn apply_event(shared: &mut Shared, value: &Value) {
    match value.get("event").and_then(Value::as_str) {
        Some("property-change") => {
            let data = value.get("data");
            match value.get("name").and_then(Value::as_str) {
                Some("time-pos") => {
                    if let Some(position) = data.and_then(Value::as_f64) {
                        shared.position = position;
                    }
                }
                Some("duration") => {
                    if let Some(duration) = data.and_then(Value::as_f64) {
                        if duration > 0.0 {
                            shared.duration = duration;
                        }
                    }
                }
                Some("pause") => {
                    if let Some(paused) = data.and_then(Value::as_bool) {
                        shared.paused = paused;
                    }
                }
                Some("playlist-pos") => {
                    shared.playlist_pos = data.and_then(Value::as_i64);
                }
                _ => {}
            }
        }
        Some("start-file") => {
            shared.pending_load = false;
            shared.loaded = false;
            shared.idle = false;
            shared.position = 0.0;
            shared.duration = 0.0;
        }
        Some("file-loaded") => {
            shared.loaded = true;
        }
        Some("end-file") => {
            if shared.pending_load {
                return;
            }
            if value.get("reason").and_then(Value::as_str) == Some("error") {
                shared.error = Some(ErrorInfo {
                    message: value
                        .get("file_error")
                        .and_then(Value::as_str)
                        .unwrap_or("playback failed")
                        .to_string(),
                    played: shared.position > 0.0,
                });
            }
        }
        Some("idle") => {
            shared.idle = true;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_end_file_does_not_record_an_error_while_a_load_is_pending() {
        let mut shared = Shared {
            pending_load: true,
            ..Shared::default()
        };
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "stop" }),
        );
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error", "file_error": "loading failed" }),
        );
        assert!(shared.error.is_none());
    }

    #[test]
    fn start_file_clears_pending_and_the_stale_position() {
        let mut shared = Shared {
            position: 42.0,
            duration: 200.0,
            paused: true,
            loaded: true,
            idle: true,
            pending_load: true,
            ..Shared::default()
        };
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        assert_eq!(shared.position, 0.0);
        assert_eq!(shared.duration, 0.0);
        assert!(!shared.loaded);
        assert!(!shared.idle);
        assert!(!shared.pending_load);
    }

    #[test]
    fn stop_never_records_an_error_but_a_failure_does() {
        let mut shared = Shared::default();
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "stop" }),
        );
        assert!(shared.error.is_none());

        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error", "file_error": "loading failed" }),
        );
        let error = shared.error.take().expect("error recorded");
        assert_eq!(error.message, "loading failed");
        assert!(!error.played, "a failed load never produced media");
    }

    #[test]
    fn an_error_after_playback_is_marked_as_played() {
        let mut shared = Shared::default();
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "property-change", "name": "time-pos", "data": 7.5 }),
        );
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error", "file_error": "I/O error" }),
        );
        let error = shared.error.take().expect("error recorded");
        assert!(error.played);
    }

    #[test]
    fn playlist_pos_and_idle_are_mirrored() {
        let mut shared = Shared::default();
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "property-change", "name": "playlist-pos", "data": 1 }),
        );
        assert_eq!(shared.playlist_pos, Some(1));
        apply_event(&mut shared, &serde_json::json!({ "event": "file-loaded" }));
        assert!(shared.loaded);
        apply_event(&mut shared, &serde_json::json!({ "event": "idle" }));
        assert!(shared.idle);
    }

    fn track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            album: String::new(),
            album_id: String::new(),
            favorite: false,
            duration: 1.0,
            url: String::new(),
            cover_art: String::new(),
            cover_url: String::new(),
        }
    }

    #[test]
    fn append_extends_the_queue_without_moving_the_cursor() {
        let mut player = Player::new();
        player.append(vec![track("s1"), track("s2")]);
        assert_eq!(player.len(), 2);
        assert_eq!(player.index(), 0);
        assert_eq!(player.current().map(|t| t.id.as_str()), Some("s1"));
        player.append(vec![track("s3")]);
        assert_eq!(player.len(), 3);
        assert_eq!(player.index(), 0);
    }

    #[test]
    fn request_ids_increase_by_one_per_command() {
        let mut player = Player::new();
        player.command(vec![json!("stop")]);
        player.command(vec![json!("stop")]);
        assert_eq!(player.request_id, 2);
    }

    /// FIX-11 regression: `load_paused` pauses while the socket is still
    /// connecting (the command is dropped). Resolving the deferred start
    /// must not autoplay the restored queue.
    #[test]
    fn a_deferred_paused_start_stays_paused() {
        let mut player = Player::new();
        player.queue = vec![track("s1")];
        player.deferred_start = true;
        player.state = State::Paused;
        player.finish_deferred_start();
        assert_eq!(player.state(), State::Paused);
        assert!(!player.deferred_start);
    }

    /// FIX-11 regression: a seek issued while connecting (restored playhead)
    /// survives the deferred rebuild.
    #[test]
    fn a_deferred_start_keeps_the_pending_playhead() {
        let mut player = Player::new();
        player.queue = vec![track("s1")];
        player.deferred_start = true;
        player.state = State::Playing;
        player.shared().position = 42.0;
        player.finish_deferred_start();
        assert_eq!(player.state(), State::Playing);
        assert_eq!(player.position(), 42.0);
    }

    #[test]
    fn remove_many_drops_the_selection_and_keeps_the_current_track() {
        let mut player = Player::new();
        player.queue = (0..=5).map(|i| track(&format!("s{i}"))).collect();
        player.index = 0;
        // Unsorted, with a duplicate and an out-of-range index.
        player.remove_many(&[5, 2, 1, 4, 1, 9]);
        assert_eq!(player.len(), 2);
        assert_eq!(player.current().map(|t| t.id.as_str()), Some("s0"));
        assert_eq!(player.index(), 0);
        player.remove_many(&[1]);
        assert_eq!(player.len(), 1);
        player.remove_many(&[0]);
        assert!(player.queue.is_empty());
        assert_eq!(player.state(), State::Stopped);
    }

    /// FIX-1: reaping takes ownership of the spawned child (TERM + wait)
    /// and clears the recorded socket, so a retried start cannot orphan
    /// the process.
    #[test]
    fn reap_child_takes_the_child_and_the_down() {
        let mut player = Player::new();
        player.child = Some(Command::new("sleep").arg("1").spawn().unwrap());
        player.socket = Some(PathBuf::from("/tmp/nonexistent-musika.sock"));
        player.connect_deadline = Some(Instant::now() + Duration::from_secs(1));
        player.reap_child();
        assert!(player.child.is_none());
        assert!(player.socket.is_none());
        assert!(player.connect_deadline.is_none());
    }

    /// FIX-11: while the socket is coming up the start defers instead of
    /// sleeping on the caller; the same calendar retry never re-spawns,
    /// and `stop` during the deferral aborts the start.
    #[test]
    fn stop_during_a_deferred_start_aborts_it() {
        let mut player = Player::new();
        // Spawn mpv (connect pending) without connecting.
        if !matches!(player.ensure_ready(), Ready::Connecting) {
            return; // mpv not testable here; the live verify covers it.
        }
        player.state = State::Playing;
        player.deferred_start = true;
        player.stop();
        assert!(player.child.is_some(), "stop reuses the spawned child");
        assert!(!player.deferred_start);
        assert_eq!(player.state(), State::Stopped);
        player.pump();
        assert_eq!(player.state(), State::Stopped);
    }
}
