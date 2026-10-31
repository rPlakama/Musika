// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! Narrow liburing-backed file reader.
//!
//! The shim is intentionally synchronous: the benchmark (and a future
//! `spawn_blocking` cover worker) supplies the async policy, while C owns only
//! the ring submission and the byte buffer.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

unsafe extern "C" {
    fn musika_io_uring_read_file(
        path: *const std::ffi::c_char,
        out: *mut *mut u8,
        out_len: *mut usize,
    ) -> std::ffi::c_int;
    fn musika_io_uring_free(data: *mut u8);
}

/// Reads `path` through a fresh io_uring ring.
pub(crate) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut data: *mut u8 = ptr::null_mut();
    let mut len = 0usize;
    // SAFETY: the shim owns `data` on success; the path is NUL-terminated and
    // the output pointers are valid for the duration of the call.
    let error = unsafe { musika_io_uring_read_file(c_path.as_ptr(), &mut data, &mut len) };
    if error < 0 {
        return Err(io::Error::from_raw_os_error(-error));
    }
    if data.is_null() || len == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: the shim returns a malloc'd buffer of exactly `len` bytes; copy
    // it before releasing the C allocation.
    let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    unsafe { musika_io_uring_free(data) };
    Ok(bytes)
}
