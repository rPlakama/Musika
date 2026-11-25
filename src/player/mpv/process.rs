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

//! Persistent child `mpv` process and non-blocking Unix-domain JSON IPC client.

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::protocol::{
    self, MpvLaunchConfig, MpvMessage, OBSERVED_PROPERTIES, cmd_observe_property, cmd_quit,
    cmd_request_log_messages,
};

/// Monotonic counter so multiple transports (or restarts) never collide on
/// IPC socket paths.
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// Maximum buffered line length before discarding a pathological partial frame.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// How long startup waits for `mpv` to bind its Unix IPC socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long `Drop` waits for a graceful `quit` before sending SIGKILL.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);

/// Resolves the `mpv` executable path (`MUSIKA_MPV_BIN` or `"mpv"` on `PATH`).
pub(crate) fn mpv_binary() -> String {
    std::env::var("MUSIKA_MPV_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "mpv".to_string())
}

pub(crate) enum ReadOutcome {
    Messages(Vec<MpvMessage>),
    Disconnected(String),
}

pub(crate) struct MpvProcess {
    child: Option<Child>,
    stream: UnixStream,
    socket_path: Option<PathBuf>,
    read_buf: Vec<u8>,
    next_req: u64,
}

impl MpvProcess {
    /// Spawns one headless audio-only `mpv` instance, connects to its Unix IPC
    /// socket, and registers all property observers.
    pub(crate) fn spawn(config: &MpvLaunchConfig) -> Result<Self, String> {
        let socket_path = allocate_socket_path()?;
        _ = std::fs::remove_file(&socket_path);

        let bin = mpv_binary();
        let args = protocol::mpv_args(&socket_path, config);

        let mut command = Command::new(&bin);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child_setup = || -> std::io::Result<()> {
            // SAFETY: `prctl(PR_SET_PDEATHSIG, SIGTERM)` takes scalar integer
            // arguments, allocates nothing, and is async-signal-safe in the
            // forked child. It guarantees `mpv` terminates if the daemon dies.
            let _ = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
            crate::linux::close_inherited_fds()?;
            Ok(())
        };
        // SAFETY: `child_setup` is async-signal-safe, holds no locks, and
        // performs no heap allocation.
        unsafe {
            command.pre_exec(child_setup);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot start mpv ({bin}): {error}"))?;

        let stream = match connect_with_retry(&socket_path, &mut child) {
            Ok(stream) => stream,
            Err(error) => {
                _ = child.kill();
                _ = child.wait();
                _ = std::fs::remove_file(&socket_path);
                return Err(error);
            }
        };

        let mut process = Self::init_stream(Some(child), stream, Some(socket_path))?;
        process.subscribe_initial()?;
        Ok(process)
    }

    /// Wraps an already-connected Unix stream (used by mock-socket unit tests
    /// without spawning a real `mpv` child).
    #[cfg(test)]
    pub(crate) fn from_test_stream(stream: UnixStream) -> Result<Self, String> {
        let mut process = Self::init_stream(None, stream, None)?;
        process.subscribe_initial()?;
        Ok(process)
    }

    fn init_stream(
        child: Option<Child>,
        stream: UnixStream,
        socket_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("cannot configure mpv IPC socket non-blocking: {error}"))?;
        Ok(Self {
            child,
            stream,
            socket_path,
            read_buf: Vec::with_capacity(8192),
            next_req: 1,
        })
    }

    fn subscribe_initial(&mut self) -> Result<(), String> {
        let req = self.alloc_req();
        self.write_raw(&cmd_request_log_messages(req, "warn"))?;
        for &(id, name) in OBSERVED_PROPERTIES {
            let req = self.alloc_req();
            self.write_raw(&cmd_observe_property(req, id, name))?;
        }
        Ok(())
    }

    pub(crate) fn alloc_req(&mut self) -> u64 {
        let req = self.next_req;
        self.next_req = self.next_req.wrapping_add(1).max(1);
        req
    }

    pub(crate) fn raw_fd(&self) -> libc::c_int {
        self.stream.as_raw_fd()
    }

    /// Writes a pre-encoded newline-terminated JSON command line.
    pub(crate) fn write_raw(&mut self, line: &str) -> Result<(), String> {
        let bytes = line.as_bytes();
        let mut written = 0usize;
        let deadline = Instant::now() + Duration::from_millis(250);
        while written < bytes.len() {
            match self.stream.write(&bytes[written..]) {
                Ok(0) => return Err("mpv IPC socket closed while writing".to_string()),
                Ok(n) => written += n,
                Err(error) if error.kind() == IoErrorKind::Interrupted => continue,
                Err(error) if error.kind() == IoErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("mpv IPC write timed out".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(format!("mpv IPC write failed: {error}")),
            }
        }
        Ok(())
    }

    /// Drains all currently readable JSON IPC messages from the socket.
    pub(crate) fn drain_messages(&mut self) -> ReadOutcome {
        let mut chunk = [0u8; 4096];
        let mut disconnected: Option<String> = None;
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    disconnected = Some(self.exit_reason());
                    break;
                }
                Ok(n) => {
                    if self.read_buf.len().saturating_add(n) > MAX_LINE_BYTES {
                        self.read_buf.clear();
                    }
                    self.read_buf.extend_from_slice(&chunk[..n]);
                }
                Err(error) if error.kind() == IoErrorKind::Interrupted => continue,
                Err(error) if error.kind() == IoErrorKind::WouldBlock => break,
                Err(error) => {
                    disconnected = Some(format!("mpv IPC read failed: {error}"));
                    break;
                }
            }
        }

        let mut messages = Vec::new();
        while let Some(pos) = self.read_buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.read_buf.drain(..=pos).collect();
            if let Ok(text) = std::str::from_utf8(&line_bytes)
                && let Some(msg) = protocol::parse_ipc_line(text)
            {
                messages.push(msg);
            }
        }

        // Return final buffered messages first; the next poll sees EOF.
        if let Some(reason) = disconnected
            && messages.is_empty()
        {
            return ReadOutcome::Disconnected(reason);
        }
        ReadOutcome::Messages(messages)
    }

    /// Checks whether the child process has unexpectedly exited.
    pub(crate) fn check_exited(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(format!("mpv exited unexpectedly ({status})")),
            Ok(None) => None,
            Err(error) => Some(format!("mpv wait failed: {error}")),
        }
    }

    fn exit_reason(&mut self) -> String {
        self.check_exited()
            .unwrap_or_else(|| "mpv IPC connection closed".to_string())
    }
}

impl Drop for MpvProcess {
    fn drop(&mut self) {
        let req = self.alloc_req();
        _ = self.write_raw(&cmd_quit(req));
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + SHUTDOWN_GRACE;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    _ => {
                        _ = child.kill();
                        _ = child.wait();
                        break;
                    }
                }
            }
        }
        if let Some(path) = self.socket_path.take() {
            _ = std::fs::remove_file(path);
        }
    }
}

fn connect_with_retry(socket_path: &Path, child: &mut Child) -> Result<UnixStream, String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!("mpv exited during startup ({status})"));
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out connecting to mpv IPC socket {}: {error}",
                        socket_path.display()
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn allocate_socket_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("musika");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("cannot create mpv runtime dir {}: {error}", dir.display()))?;
    _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    let id = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!("mpv-{}-{id}.sock", std::process::id())))
}
