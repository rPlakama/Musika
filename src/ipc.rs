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

//! Line-based IPC over a Unix socket.
//!
//! Musika listens on `<config>/ipc.sock`; every line written to it (trimmed,
//! non-empty) is delivered to the UI as a command — the same commands the
//! keybind engine emits. This lets external tools drive the player, e.g.:
//!
//! ```sh
//! echo search | socat - UNIX-CONNECT:~/.config/musika/ipc.sock
//! ```
//!
//! Commands are parsed by the shared `ui::ipc_command` registry, so the
//! socket and the `:` command line accept the same vocabulary: navigation and
//! list actions (`top`, `move_down`, `queue_toggle`, ...), transport
//! (`play_pause`, `next`, `volume 40`, `seek 30`), payload commands
//! (`play_song_in_album <song-id>`, `download_song <song-id>`), maintenance
//! (`sync`, `sync all`, `sync repair`, `wipe album|lyrics|covers`) and the
//! legacy/MPRIS aliases.

use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Path of the core/UI control socket (`MUSIKA_DAEMON_SOCKET` overrides).
pub fn daemon_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("MUSIKA_DAEMON_SOCKET")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    crate::db::config_socket("daemon.sock")
}

/// Path of the core/UI control socket (`MUSIKA_IPC_SOCKET` overrides).
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("MUSIKA_IPC_SOCKET")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    crate::db::config_socket("ipc.sock")
}

/// True when a live core already owns the command socket. Probing the
/// control socket would register a fake UI connection, so use the IPC one.
pub fn daemon_is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Sends one command line to the running IPC socket, if any. Blocking but
/// bounded (connect + write only); tray/launcher callers use it as a cheap
/// hand-off into the core command queue.
pub fn send_line(line: &str) -> bool {
    use std::io::Write;
    use std::time::Duration;
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    writeln!(stream, "{}", line.trim()).is_ok()
}

pub fn remove_stale_socket(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn spawn_line_reader<F>(stream: UnixStream, on_line: F)
where
    F: FnMut(String) + Send + 'static,
{
    std::thread::spawn(move || read_lines(stream, on_line));
}

fn read_lines<F>(stream: UnixStream, mut on_line: F)
where
    F: FnMut(String),
{
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let command = line.trim();
        if !command.is_empty() {
            on_line(command.to_string());
        }
    }
}

/// Serves commands on `path` from a dedicated thread. FIX-9: each accepted
/// stream gets its own short-lived reader thread, so one silent client that
/// keeps its socket open can no longer block every later client on the
/// accept loop. Lines from every client funnel through one dispatcher
/// thread, so the handler stays single-threaded (and need not be `Sync`).
pub fn serve<F>(path: PathBuf, mut handler: F)
where
    F: FnMut(String) + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in rx {
            handler(line);
        }
    });

    std::thread::spawn(move || {
        remove_stale_socket(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let listener = match UnixListener::bind(&path) {
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
            let tx = tx.clone();
            spawn_line_reader(stream, move |line| {
                let _ = tx.send(line);
            });
        }
    });
}
