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
            volume: 70.0,
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
    #[allow(dead_code)]
    pub fn volume(&self) -> f64 {
        self.volume
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

        // Run mpv under a tiny supervisor shell that watches our PID and
        // terminates mpv if we die (even on SIGKILL, where Drop never runs).
        let parent = std::process::id();
        let mpris = mpris_script_arg();
        let script = format!(
            "mpv{mpris} --no-video --idle=yes --no-terminal --really-quiet \
             --input-ipc-server='{socket}' & MPV=$!; \
             trap 'kill -TERM $MPV 2>/dev/null; exit 0' TERM INT; \
             while kill -0 {parent} 2>/dev/null; do sleep 0.5; done; \
             kill -TERM $MPV 2>/dev/null",
            mpris = mpris,
            socket = socket.display(),
            parent = parent,
        );

        let spawn = Command::new("sh")
            .arg("-c")
            .arg(script)
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

        // Observe the properties the UI needs, then apply the volume.
        self.command(vec![json!("observe_property"), json!(1), json!("time-pos")]);
        self.command(vec![json!("observe_property"), json!(2), json!("duration")]);
        self.command(vec![json!("observe_property"), json!(3), json!("pause")]);
        self.set_volume(self.volume);
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
            // The supervisor traps TERM and shuts mpv down; SIGKILL would skip
            // that, so signal it explicitly instead of `child.kill()`.
            signal(child.id(), "TERM");
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

/// Optional `--script=...` to load mpv's MPRIS plugin. Set
/// `MUSIKA_MPRIS_SCRIPT` to the `mpris.so`/`mpris.lua` path, or drop it into
/// `~/.config/mpv/scripts` (which mpv loads automatically).
fn mpris_script_arg() -> String {
    match std::env::var("MUSIKA_MPRIS_SCRIPT") {
        Ok(path) if !path.is_empty() => format!(" --script='{path}'"),
        _ => String::new(),
    }
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

fn signal(pid: u32, signal: &str) {
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("kill -{signal} {pid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
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
}
