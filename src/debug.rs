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

//! Development-only diagnostics.
//!
//! `debug_log!` expands to `eprintln!` in debug builds and to an empty
//! statement in release, so routine sync/download/login summaries never
//! leak into a packaged run. Errors still use `eprintln!` directly.

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        #[cfg(debug_assertions)]
        eprintln!($($arg)*);
    }};
}
