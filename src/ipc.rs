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

//! Line-based control socket (`ipc.sock`) and the paths of the two sockets.
//!
//! `ipc.sock` is the external/text control surface: every line written to it
//! (trimmed, non-empty) is parsed by the shared [`crate::ipc_command`]
//! registry. Read-only commands (`status`, `queue`) answer with one JSON line;
//! every other command stays fire-and-forget, so the historical
//! `printf 'play_pause\n' | nc -U ~/.config/musika/ipc.sock` usage keeps
//! working unchanged.
//!
//! `daemon.sock` is the framed core/UI protocol (`crate::ipc_protocol`); this
//! module only owns its path so the UI and the daemon agree on it.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Largest accepted command line. Commands are tiny; a longer line is a client
/// bug and is dropped instead of growing the daemon's memory.
const MAX_COMMAND_BYTES: usize = 64 * 1024;

/// How long a text client may take to send one line before the connection is
/// dropped (keeps a stalled `nc` from parking a reader thread forever).
const COMMAND_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Path of the core/UI control socket (`MUSIKA_DAEMON_SOCKET` overrides).
pub fn daemon_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("MUSIKA_DAEMON_SOCKET")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    crate::db::config_socket("daemon.sock")
}

/// Path of the external command socket (`MUSIKA_IPC_SOCKET` overrides).
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("MUSIKA_IPC_SOCKET")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    crate::db::config_socket("ipc.sock")
}

/// True when a live core already owns the command socket. Probing the control
/// socket would register a fake UI connection, so use the IPC one.
pub fn daemon_is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Sends one command line to the running IPC socket, if any. Blocking but
/// bounded (connect + write only); tray/launcher callers use it as a cheap
/// hand-off into the core command queue.
pub fn send_line(line: &str) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    writeln!(stream, "{}", line.trim()).is_ok()
}

/// Sends one line and reads a single reply line (for `status`-style queries).
/// Bounded by a short read timeout; `None` when the core is absent or silent.
pub fn send_query(line: &str) -> Option<String> {
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .ok()?;
    writeln!(stream, "{}", line.trim()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply).ok()?;
    let reply = reply.trim().to_string();
    (!reply.is_empty()).then_some(reply)
}

/// Removes a stale socket file before binding.
pub fn remove_stale_socket(path: &Path) {
    _ = std::fs::remove_file(path);
}

/// Binds a socket with owner-only permissions, creating the parent directory.
pub fn bind_private(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        _ = std::fs::create_dir_all(parent);
        _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    remove_stale_socket(path);
    let listener = UnixListener::bind(path)?;
    _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}

/// True when the peer runs as the same user. Both sockets are user-private, so
/// anything else is refused instead of being allowed to drive the player.
pub fn same_uid<T: std::os::fd::AsRawFd + ?Sized>(stream: &T) -> bool {
    #[repr(C)]
    struct UCred {
        pid: libc::pid_t,
        uid: libc::uid_t,
        gid: libc::gid_t,
    }
    let mut cred = UCred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<UCred>() as libc::socklen_t;
    // SAFETY: `getsockopt(SO_PEERCRED)` writes one `struct ucred` into
    // `cred` (which is fully initialised and sized by `len`) and the stream fd
    // stays valid for the duration of the call; on failure the kernel writes
    // nothing and the result is checked below.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut UCred as *mut libc::c_void,
            &mut len,
        )
    };
    // SAFETY: `geteuid` has no preconditions and cannot fail; it returns the
    // process's effective uid as an integer, so no memory can be affected.
    result == 0 && cred.uid == unsafe { libc::geteuid() }
}

/// Serves command lines on `path` from a dedicated thread. Each accepted
/// stream gets its own short-lived reader thread (FIX-9: a silent client can
/// no longer block later clients), and every line funnels through one
/// dispatcher thread so the handler stays single-threaded.
///
/// The handler returns `Some(reply)` for read-only query commands; the reply
/// is written back to that client as one line. `None` means fire-and-forget.
pub fn serve<F>(path: PathBuf, mut handler: F)
where
    F: FnMut(String) -> Option<String> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<(String, Arc<UnixStream>)>();
    std::thread::spawn(move || {
        for (line, stream) in rx {
            if let Some(reply) = handler(line) {
                let mut writer = &*stream;
                _ = writeln!(writer, "{reply}");
                _ = writer.flush();
            }
        }
    });

    std::thread::spawn(move || {
        let listener = match bind_private(&path) {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("ipc: could not bind {}: {err}", path.display());
                return;
            }
        };
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            if !same_uid(&stream) {
                eprintln!("ipc: refused a connection from another user");
                continue;
            }
            let Ok(reply_stream) = stream.try_clone() else {
                continue;
            };
            let reply_stream = Arc::new(reply_stream);
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                _ = reader
                    .get_ref()
                    .set_read_timeout(Some(COMMAND_READ_TIMEOUT));
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    if line.len() > MAX_COMMAND_BYTES {
                        eprintln!("ipc: dropped an oversized command line");
                        break;
                    }
                    let command = line.trim();
                    if command.is_empty() {
                        continue;
                    }
                    if tx
                        .send((command.to_string(), Arc::clone(&reply_stream)))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }
    });
}
