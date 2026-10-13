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

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

/// Path of the IPC socket (`MUSIKA_IPC_SOCKET` overrides).
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("MUSIKA_IPC_SOCKET") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    crate::db::config_dir()
        .map(|dir| dir.join("ipc.sock"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/musika-ipc.sock"))
}

/// Serves commands on `path` from a dedicated thread, one connection at a time.
pub fn serve<F>(path: PathBuf, handler: F)
where
    F: Fn(String) + Send + 'static,
{
    std::thread::spawn(move || {
        let _ = std::fs::remove_file(&path);
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
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else {
                    break;
                };
                let command = line.trim();
                if !command.is_empty() {
                    handler(command.to_string());
                }
            }
        }
    });
}
