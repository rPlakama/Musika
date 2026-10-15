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
//! SIGSTOP hack: mpv runs in `--idle` mode for the whole session and files are
//! swapped with `loadfile`.
//!
//! A small reader thread owns the read half of the socket and mirrors mpv's
//! observed properties into a shared [`Shared`] snapshot that the Qt thread
//! reads during `tick`. Stale `end-file` events from replaced files are
//! dropped while a load is pending, so fast skips never depend on network
//! timing (the queue advances on keypress, not on the stream).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
}

/// Snapshot of mpv state, written by the reader thread and read by the UI.
#[derive(Debug, Default)]
struct Shared {
    position: f64,
    duration: f64,
    paused: bool,
    eof: bool,
    error: bool,
    /// `true` between issuing a `loadfile` and mpv reporting `start-file`.
    /// While set, `end-file` events belong to the file we deliberately
    /// replaced and must not advance the queue (fast skip/previous races).
    pending_load: bool,
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

    pub fn duration(&self) -> f64 {
        let shared = self.shared.lock().unwrap();
        if shared.duration > 0.0 {
            shared.duration
        } else {
            self.fallback_duration
        }
    }

    pub fn position(&self) -> f64 {
        self.shared.lock().unwrap().position
    }

    /// Replaces the queue and starts at `start`.
    pub fn load(&mut self, tracks: Vec<Track>, start: usize) {
        self.queue = tracks;
        self.index = start.min(self.queue.len().saturating_sub(1));
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
        self.queue.extend(tracks);
    }

    /// (Re)starts the current track from the beginning.
    pub fn play_current(&mut self) {
        if self.queue.is_empty() {
            self.state = State::Stopped;
            return;
        }
        if !self.ensure_ready() {
            self.state = State::Stopped;
            return;
        }

        let track = &self.queue[self.index];
        let url = track.url.clone();
        let metadata = json!({
            "title": track.title,
            "artist": track.artist,
            "album": track.album,
            "artUrl": track.cover_url,
            "mpris:artUrl": track.cover_url,
        });
        self.fallback_duration = track.duration;

        {
            let mut shared = self.shared.lock().unwrap();
            *shared = Shared {
                position: 0.0,
                duration: track.duration,
                paused: false,
                eof: false,
                error: false,
                pending_load: true,
            };
        }

        // `mpris:artUrl` / `metadata` feed mpv's MPRIS (when the plugin is
        // loaded); position and pause state follow automatically.
        self.command(vec![json!("loadfile"), json!(url), json!("replace")]);
        self.command(vec![json!("set_property"), json!("metadata"), metadata]);
        self.command(vec![json!("set_property"), json!("pause"), json!(false)]);
        self.state = State::Playing;
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
        self.command(vec![json!("set_property"), json!("pause"), json!(true)]);
        self.state = State::Paused;
    }

    pub fn resume(&mut self) {
        if self.state != State::Paused {
            return;
        }
        self.command(vec![json!("set_property"), json!("pause"), json!(false)]);
        self.state = State::Playing;
    }

    pub fn stop(&mut self) {
        self.command(vec![json!("stop")]);
        {
            let mut shared = self.shared.lock().unwrap();
            shared.position = 0.0;
            shared.eof = false;
            shared.error = false;
        }
        self.state = State::Stopped;
    }

    pub fn next(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if self.index + 1 < self.queue.len() {
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
        self.index = self.index.saturating_sub(1);
        self.play_current();
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
        if self.queue.is_empty() {
            self.stop();
            return;
        }
        if index < self.index {
            self.index -= 1;
        } else if was_current {
            if self.index >= self.queue.len() {
                self.index = self.queue.len() - 1;
            }
            self.play_current();
        }
    }

    /// Moves the track at `from` to position `to`.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.queue.len() || to >= self.queue.len() || from == to {
            return;
        }
        let track = self.queue.remove(from);
        self.queue.insert(to, track);
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
    }

    /// Seeks to an absolute position (seconds).
    pub fn seek(&mut self, seconds: f64) {
        let seconds = seconds.max(0.0);
        self.command(vec![json!("seek"), json!(seconds), json!("absolute")]);
        self.shared.lock().unwrap().position = seconds;
    }

    /// Sets volume (0-100).
    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 100.0);
        self.command(vec![
            json!("set_property"),
            json!("volume"),
            json!(self.volume),
        ]);
    }

    /// Mutes or unmutes without touching the volume value.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.command(vec![
            json!("set_property"),
            json!("mute"),
            json!(self.muted),
        ]);
    }

    pub fn toggle_mute(&mut self) {
        self.set_muted(!self.muted);
    }

    /// Advances end-of-track detection. Returns `true` when a track ended
    /// naturally and the caller should move on.
    pub fn poll_end(&mut self) -> bool {
        if self.state == State::Stopped {
            return false;
        }
        let mut shared = self.shared.lock().unwrap();
        if !shared.eof {
            return false;
        }
        shared.eof = false;
        !shared.error
    }

    // -- internals ---------------------------------------------------------

    fn ensure_ready(&mut self) -> bool {
        if self.ipc.is_some() {
            return true;
        }

        let socket = socket_path();
        let _ = std::fs::remove_file(&socket);

        // mpv is our direct child and the kernel TERMs it if we die (even on
        // SIGKILL, where Drop never runs): PR_SET_PDEATHSIG in `pre_exec`, no
        // supervisor shell. PDEATHSIG follows the spawning thread, and
        // `ensure_ready` runs on the Qt thread, which lives for the process.
        let parent = std::process::id();
        let socket_arg = format!("--input-ipc-server={}", socket.display());

        let mut command = Command::new("mpv");
        command
            .arg("--no-video")
            .arg("--idle=yes")
            .arg("--no-terminal")
            .arg("--really-quiet")
            .arg(&socket_arg);
        if let Ok(script) = std::env::var("MUSIKA_MPRIS_SCRIPT") {
            if !script.is_empty() {
                command.arg(format!("--script={script}"));
            }
        }
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have died between fork and prctl; PDEATHSIG
                // would not fire then, so bail out instead of leaking.
                if libc::getppid() as u32 != parent {
                    return Err(std::io::Error::other("parent died before spawn"));
                }
                Ok(())
            });
        }

        let spawn = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match spawn {
            Ok(child) => self.child = Some(child),
            Err(err) => {
                eprintln!("player: could not start mpv: {err}");
                return false;
            }
        }

        let Some(stream) = connect_retry(&socket, Duration::from_secs(3)) else {
            eprintln!("player: could not connect to mpv IPC socket");
            return false;
        };

        if let Ok(reader) = stream.try_clone() {
            spawn_reader(reader, Arc::clone(&self.shared));
        }
        self.ipc = Some(stream);
        self.socket = Some(socket);

        // Observe the properties the UI needs, then apply volume and mute.
        self.command(vec![json!("observe_property"), json!(1), json!("time-pos")]);
        self.command(vec![json!("observe_property"), json!(2), json!("duration")]);
        self.command(vec![json!("observe_property"), json!(3), json!("pause")]);
        self.set_volume(self.volume);
        self.set_muted(self.muted);
        true
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
        if let Some(mut child) = self.child.take() {
            // mpv shuts down cleanly on TERM; `child.kill()` would SIGKILL it.
            signal(child.id(), libc::SIGTERM);
            let _ = child.wait();
        }
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

fn socket_path() -> PathBuf {
    crate::db::config_dir()
        .map(|dir| dir.join("mpv.sock"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/musika-mpv.sock"))
}

fn connect_retry(path: &Path, timeout: Duration) -> Option<UnixStream> {
    let start = Instant::now();
    loop {
        if let Ok(stream) = UnixStream::connect(path) {
            return Some(stream);
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn signal(pid: u32, signal: libc::c_int) {
    // Native kill(2), no `kill` binary spawn (ARC-16).
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

fn spawn_reader(stream: UnixStream, shared: Arc<Mutex<Shared>>) {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let mut shared = shared.lock().unwrap();
            apply_event(&mut shared, &value);
        }
    });
}

/// Mirrors one mpv event into [`Shared`].
///
/// `start-file` marks the load we requested as actually begun: it clears the
/// pending guard and drops the stale position/duration of the replaced file.
/// `end-file` events are ignored while a load is pending, because they belong
/// to the file that load replaced — this is what keeps fast skips/previous
/// from bouncing back into a track the user already moved away from.
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
                _ => {}
            }
        }
        Some("start-file") => {
            shared.pending_load = false;
            shared.eof = false;
            shared.error = false;
            shared.position = 0.0;
            shared.duration = 0.0;
        }
        Some("end-file") => {
            if shared.pending_load {
                return;
            }
            let reason = value.get("reason").and_then(Value::as_str).unwrap_or("");
            shared.eof = true;
            shared.error = reason == "error";
            shared.paused = true;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_end_file_does_not_advance_while_a_load_is_pending() {
        let mut shared = Shared {
            pending_load: true,
            ..Shared::default()
        };
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "stop" }),
        );
        assert!(!shared.eof);
        assert!(!shared.error);
    }

    #[test]
    fn end_file_advances_once_the_requested_load_has_started() {
        let mut shared = Shared::default();
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "eof" }),
        );
        assert!(shared.eof);
        assert!(!shared.error);
    }

    #[test]
    fn start_file_resets_stale_position_and_duration() {
        let mut shared = Shared {
            position: 42.0,
            duration: 200.0,
            paused: true,
            eof: true,
            error: false,
            pending_load: true,
        };
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        assert_eq!(shared.position, 0.0);
        assert_eq!(shared.duration, 0.0);
        assert!(!shared.eof);
        assert!(!shared.pending_load);
    }

    #[test]
    fn error_end_file_stalls_instead_of_advancing() {
        let mut shared = Shared::default();
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error" }),
        );
        assert!(shared.eof);
        assert!(shared.error);
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
}
