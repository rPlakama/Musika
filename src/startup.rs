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

//! Process-relative startup timing shared by debug diagnostics and the
//! release trace.
//!
//! `MUSIKA_STARTUP_TRACE=1` prints one mark per startup phase to stderr;
//! when it is unset the environment is read once and every later mark is a
//! cached boolean check. `MUSIKA_STARTUP_EXIT=1` closes the window after the
//! first rendered frame so the whole startup can be benchmarked.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

static PROCESS_START: OnceLock<Instant> = OnceLock::new();
static TRACE: OnceLock<bool> = OnceLock::new();
static EXIT_AFTER_READY: OnceLock<bool> = OnceLock::new();

/// Starts the process-wide clock. Call once, after the `--version` probe.
pub(crate) fn start() {
    _ = PROCESS_START.set(Instant::now());
}

/// Time since [`start`] (or since the first call when `start` was skipped).
pub(crate) fn elapsed() -> Duration {
    PROCESS_START.get_or_init(Instant::now).elapsed()
}

pub(crate) fn trace(label: &str) {
    trace_elapsed(label, elapsed());
}

pub(crate) fn trace_elapsed(label: &str, elapsed: Duration) {
    if trace_enabled() {
        eprintln!("{}", line(label, elapsed));
    }
}

fn trace_enabled() -> bool {
    *TRACE.get_or_init(|| env_flag("MUSIKA_STARTUP_TRACE"))
}

/// Whether the benchmark mode should quit after the first frame.
pub(crate) fn exit_after_ready() -> bool {
    *EXIT_AFTER_READY.get_or_init(|| env_flag("MUSIKA_STARTUP_EXIT"))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

fn line(label: &str, elapsed: Duration) -> String {
    format!(
        "[musika/startup] {label}: {:.2} ms",
        elapsed.as_secs_f64() * 1000.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_process_relative_milliseconds() {
        assert_eq!(
            line("db.init", Duration::from_micros(12_346)),
            "[musika/startup] db.init: 12.35 ms"
        );
        assert_eq!(
            line("frontend.ready", Duration::ZERO),
            "[musika/startup] frontend.ready: 0.00 ms"
        );
    }

    #[test]
    fn elapsed_is_monotonic() {
        start();
        let first = elapsed();
        let second = elapsed();
        assert!(second >= first, "{second:?} < {first:?}");
    }
}
