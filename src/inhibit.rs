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
//!
//! The daemon republishes the playback state about once a second, so [`set`]
//! only wakes the worker when the verdict actually changes (`send_if_modified`)
//! and the worker always processes the current value before waiting for the
//! next one. A missing or dropped system bus is retried with a throttle
//! instead of disabling inhibition for the rest of the process.

use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::watch;

/// How long to wait before retrying a failed system-bus connection or
/// `Inhibit` call, so a session without logind is not polled hard.
const RECONNECT_EVERY: Duration = Duration::from_secs(30);

static TX: OnceLock<watch::Sender<bool>> = OnceLock::new();

/// Requests or releases the idle inhibitor. Cheap and idempotent: repeated
/// calls with the same verdict do not wake the worker.
pub fn set(playing: bool) {
    let tx = TX.get_or_init(|| {
        let (tx, rx) = watch::channel(false);
        crate::app::runtime().spawn(worker(rx));
        tx
    });
    tx.send_if_modified(|value| {
        let changed = *value != playing;
        *value = playing;
        changed
    });
}

async fn worker(mut rx: watch::Receiver<bool>) {
    let mut connection: Option<zbus::Connection> = None;
    let mut inhibitor: Option<zbus::zvariant::OwnedFd> = None;
    // The initial value is the current verdict: `set(true)` may land before
    // this task first runs, and must not be treated as already seen.
    let mut playing = *rx.borrow_and_update();
    let mut retry_at: Option<tokio::time::Instant> = None;
    loop {
        if playing && inhibitor.is_none() {
            let due = retry_at.is_none_or(|at| tokio::time::Instant::now() >= at);
            if due {
                retry_at = None;
                if connection.is_none() {
                    match zbus::Connection::system().await {
                        Ok(conn) => connection = Some(conn),
                        Err(error) => {
                            crate::debug_log!("inhibit: system bus unavailable: {error}");
                            retry_at = Some(tokio::time::Instant::now() + RECONNECT_EVERY);
                        }
                    }
                }
                if let Some(conn) = connection.as_ref() {
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
                        Ok(reply) => {
                            match reply.body().deserialize::<zbus::zvariant::OwnedFd>() {
                                Ok(fd) => inhibitor = Some(fd),
                                Err(error) => {
                                    crate::debug_log!("inhibit: bad reply: {error}");
                                    // A broken connection cannot be trusted for
                                    // a later call; rebuild it on the retry.
                                    connection = None;
                                    retry_at = Some(tokio::time::Instant::now() + RECONNECT_EVERY);
                                }
                            }
                        }
                        Err(error) => {
                            crate::debug_log!("inhibit: Inhibit failed: {error}");
                            connection = None;
                            retry_at = Some(tokio::time::Instant::now() + RECONNECT_EVERY);
                        }
                    }
                }
            }
        } else if !playing {
            inhibitor = None;
            retry_at = None;
        }

        // Wait for a new verdict, or for the reconnect throttle to expire
        // while inhibition is still wanted.
        let Some(at) = retry_at.filter(|_| playing && inhibitor.is_none()) else {
            if rx.changed().await.is_err() {
                break;
            }
            playing = *rx.borrow_and_update();
            continue;
        };
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    break;
                }
                playing = *rx.borrow_and_update();
            }
            _ = tokio::time::sleep_until(at) => {}
        }
    }
}
