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

//! Linux-only filesystem hints.
//!
//! These are advisory only: a failed hint must never affect correctness, so
//! callers treat every result as best-effort.

#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::path::Path;

/// Ask the kernel to start reading a file into the page cache.
///
/// This is the same intent as a buffered read, without copying the bytes into
/// userspace. The existing warm/read paths still run as a fallback.
#[cfg(target_os = "linux")]
pub(crate) fn advise_willneed(path: &Path) -> std::io::Result<()> {
    const POSIX_FADV_WILLNEED: std::ffi::c_int = 3;
    unsafe extern "C" {
        fn posix_fadvise(
            fd: std::ffi::c_int,
            offset: std::ffi::c_long,
            len: std::ffi::c_long,
            advice: std::ffi::c_int,
        ) -> std::ffi::c_int;
    }
    let file = std::fs::File::open(path)?;
    let _ = unsafe { posix_fadvise(file.as_raw_fd(), 0, 0, POSIX_FADV_WILLNEED) };
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn advise_willneed(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
