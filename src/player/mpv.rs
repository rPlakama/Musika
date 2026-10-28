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

//! mpv process, socket, event reader and the pump that mirrors its state.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{ErrorInfo, Player, Ready, Repeat, Shared, State};

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
const MAX_RETRIES: u32 = 2;
/// Backoff before a retry (the first retry is immediate).
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// FIX-11: how long a spawned mpv may take to expose its IPC socket before
/// the child is reaped and the next start re-spawns it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
impl Player {
    pub(super) fn set_property(&mut self, name: &str, value: Value) {
        self.command(vec![json!("set_property"), json!(name), value]);
    }

    pub(super) fn observe(&mut self, id: u32, name: &str) {
        self.command(vec![json!("observe_property"), json!(id), json!(name)]);
    }

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
            // its lookahead, or the queue ended. Repeat modes own the edge.
            if self.repeat == Repeat::One {
                self.play_current();
            } else if self.repeat == Repeat::All && !self.queue.is_empty() {
                self.index = if self.index + 1 < self.queue.len() {
                    self.index + 1
                } else {
                    0
                };
                self.play_current();
            } else if self.index + 1 < self.queue.len() {
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
    pub(super) fn forget_error(&mut self) {
        self.retries = 0;
        self.retry_at = None;
        self.shared().error = None;
    }

    /// FIX-1: reaps a spawned mpv whose IPC socket never came up (or was
    /// replaced by a later attempt), so a retried start cannot orphan the
    /// process and its reader thread.
    pub(crate) fn reap_child(&mut self) {
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
    pub(crate) fn ensure_ready(&mut self) -> Ready {
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
                .arg("--af=:astats=metadata=1:reset=1")
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
                let mut shared = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                apply_event(&mut shared, &value);
            });
        }
        self.ipc = Some(stream);

        // Observe the properties the UI needs, then apply volume and mute.
        self.observe(1, "time-pos");
        self.observe(2, "duration");
        self.observe(3, "pause");
        self.observe(4, "playlist-pos");
        self.observe(5, "af-metadata/astats");
        self.set_volume(self.volume);
        self.set_muted(self.muted);
    }

    pub(crate) fn command(&mut self, args: Vec<Value>) -> bool {
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
/// Converts one astats metadata map into the 0..1 playback envelope.
fn update_audio(shared: &mut Shared, data: &Value) {
    use std::time::{Duration, Instant};
    if shared
        .audio_at
        .is_some_and(|at| at.elapsed() < Duration::from_millis(33))
    {
        return;
    }
    let value = |key: &str| -> Option<f64> {
        data.get(key).and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
        })
    };
    let level = |db: f64| ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let rms = value("lavfi.astats.Overall.RMS_level").unwrap_or(-60.0);
    let peak = value("lavfi.astats.Overall.Peak_level").unwrap_or(-60.0);
    shared.audio_level = level(rms);
    shared.audio_peak = level(peak);
    shared.audio_at = Some(Instant::now());
}

pub(crate) fn apply_event(shared: &mut Shared, value: &Value) {
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
                Some("af-metadata/astats") => {
                    if let Some(data) = data {
                        update_audio(shared, data);
                    }
                }
                _ => {}
            }
        }
        Some("start-file") => {
            shared.audio_level = 0.0;
            shared.audio_peak = 0.0;
            shared.audio_at = None;
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
