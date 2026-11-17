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

//! Idle inhibition while audio is playing.
//!
//! Uses systemd-logind's `Inhibit("idle")`; the returned file descriptor is
//! held open for the duration of playback. The D-Bus call runs on a dedicated
//! task so a slow/unavailable bus never stalls the daemon loop. Failure is
//! silent (headless/CI sessions often have no logind).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

static TX: OnceLock<watch::Sender<bool>> = OnceLock::new();
static STARTING: AtomicBool = AtomicBool::new(false);

/// Requests or releases the idle inhibitor. Cheap and idempotent.
pub fn set(playing: bool) {
    if let Some(tx) = TX.get() {
        _ = tx.send(playing);
        return;
    }
    if STARTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (tx, rx) = watch::channel(playing);
    if TX.set(tx).is_err() {
        return;
    }
    crate::app::runtime().spawn(worker(rx));
}

async fn worker(mut rx: watch::Receiver<bool>) {
    let mut connection: Option<zbus::Connection> = None;
    let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        let playing = *rx.borrow_and_update();
        if playing && inhibitor.is_none() {
            if connection.is_none() {
                match zbus::Connection::system().await {
                    Ok(conn) => connection = Some(conn),
                    Err(error) => {
                        crate::debug_log!("inhibit: no system bus: {error}");
                        // Avoid retrying the bus on every state flip.
                        return;
                    }
                }
            }
            let Some(conn) = connection.as_ref() else {
                continue;
            };
            let result = conn
                .call_method(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1",
                    Some("org.freedesktop.login1.Manager"),
                    "Inhibit",
                    &("idle", "Musika", "Playing audio", "block"),
                )
                .await;
            match result {
                Ok(reply) => match reply.body().deserialize::<zbus::zvariant::OwnedFd>() {
                    Ok(fd) => inhibitor = Some(fd),
                    Err(error) => crate::debug_log!("inhibit: bad reply: {error}"),
                },
                Err(error) => crate::debug_log!("inhibit: Inhibit failed: {error}"),
            }
        } else if !playing {
            inhibitor = None;
        }
    }
}
