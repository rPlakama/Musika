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

//! Shared constants (ARC-14).

/// Bounded concurrency for network fetch fan-outs (index, covers, lyrics).
pub const FETCH_CONCURRENCY: usize = 8;

/// Bounded concurrency for download passes — at most this many song fetches
/// in flight at once (the shared pool stays polite to the server).
pub const DOWNLOAD_CONCURRENCY: usize = 3;

/// Default player volume before mpv/UI overrides.
pub const DEFAULT_VOLUME: f64 = 70.0;

/// PATH-8: album and artist metadata is served from cache while this fresh.
pub const INFO_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// PATH-9: an unchanged sync revalidates covers at most this often.
pub const COVER_CHECK_TTL_SECS: i64 = 24 * 60 * 60;

/// Seconds since the Unix epoch (0 on clock errors), shared by every cache
/// timestamp (ARC-36; replaces four local `now()` helpers).
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}
