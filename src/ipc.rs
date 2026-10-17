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
//! Commands are things like `search`, `play_pause`, `next`, `back`, ...
//! Payload commands append an argument, e.g.
//! `play_song_in_album <song-id>` (opens the album page and plays from there).

use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Path of the IPC socket (`MUSIKA_IPC_SOCKET` overrides).
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("MUSIKA_IPC_SOCKET") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    crate::db::config_socket("ipc.sock")
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

/// Serves commands on `path` from a dedicated thread, one connection at a time.
pub fn serve<F>(path: PathBuf, handler: F)
where
    F: Fn(String) + Send + 'static,
{
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

        let mut handler = handler;
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            read_lines(stream, &mut handler);
        }
    });
}
