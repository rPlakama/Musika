// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! Reusable liburing file reader.
//!
//! A [`Reader`] owns one ring for its lifetime. [`Reader::read_many`] opens a
//! batch, submits all reads before waiting, and returns one result per path.
//! No ring is created per file.

use std::cell::RefCell;
use std::ffi::{CString, c_char, c_int};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

#[repr(C)]
struct RawResult {
    data: *mut u8,
    len: usize,
    error: c_int,
}

#[repr(C)]
struct RawReader {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn musika_uring_reader_new(
        entries: u32,
        err: *mut c_char,
        err_len: usize,
    ) -> *mut RawReader;
    fn musika_uring_reader_free(reader: *mut RawReader);
    fn musika_uring_read_many(
        reader: *mut RawReader,
        paths: *const *const c_char,
        count: usize,
        results: *mut RawResult,
    ) -> c_int;
    fn musika_uring_result_free(result: *mut RawResult);
}

pub(crate) struct Reader {
    raw: *mut RawReader,
}

impl Reader {
    pub(crate) fn new() -> io::Result<Self> {
        let mut error = [0 as c_char; 256];
        // 64 slots is enough for a cover thumbnail batch and keeps the ring
        // small enough to live for a worker's lifetime.
        let raw = unsafe { musika_uring_reader_new(64, error.as_mut_ptr(), error.len()) };
        if raw.is_null() {
            return Err(io::Error::other(c_string(&error)));
        }
        Ok(Self { raw })
    }

    pub(crate) fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let mut results = self.read_many(std::slice::from_ref(&path.to_path_buf()));
        results
            .pop()
            .unwrap_or_else(|| Err(io::Error::from(io::ErrorKind::InvalidInput)))
    }

    pub(crate) fn read_many(&self, paths: &[PathBuf]) -> Vec<io::Result<Vec<u8>>> {
        if paths.is_empty() {
            return Vec::new();
        }
        let c_paths: Vec<Option<CString>> = paths
            .iter()
            .map(|path| CString::new(path.as_os_str().as_bytes()).ok())
            .collect();
        let raw_paths: Vec<*const c_char> = c_paths
            .iter()
            .map(|path| path.as_ref().map_or(ptr::null(), |path| path.as_ptr()))
            .collect();
        let mut raw_results: Vec<RawResult> = (0..paths.len())
            .map(|_| RawResult {
                data: ptr::null_mut(),
                len: 0,
                error: 0,
            })
            .collect();
        let batch_error = unsafe {
            musika_uring_read_many(
                self.raw,
                raw_paths.as_ptr(),
                raw_paths.len(),
                raw_results.as_mut_ptr(),
            )
        };
        if batch_error < 0 {
            return paths
                .iter()
                .map(|_| Err(io::Error::from_raw_os_error(-batch_error)))
                .collect();
        }

        let mut results = Vec::with_capacity(raw_results.len());
        for result in &mut raw_results {
            if result.error < 0 {
                results.push(Err(io::Error::from_raw_os_error(-result.error)));
            } else if result.data.is_null() || result.len == 0 {
                results.push(Ok(Vec::new()));
            } else {
                // SAFETY: C owns `data` until `musika_uring_result_free`; copy
                // before releasing it.
                let bytes = unsafe { std::slice::from_raw_parts(result.data, result.len) }.to_vec();
                results.push(Ok(bytes));
            }
            unsafe { musika_uring_result_free(result) };
        }
        results
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { musika_uring_reader_free(self.raw) };
    }
}

thread_local! {
    static WORKER_READER: RefCell<Option<Reader>> = const { RefCell::new(None) };
}

/// Reads one file through the calling thread's persistent reader.
pub(crate) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    WORKER_READER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(Reader::new()?);
        }
        slot.as_ref()
            .ok_or_else(|| io::Error::other("io_uring reader unavailable"))?
            .read(path)
    })
}

/// Batch read for a worker pool: one ring, one submit wave.
pub(crate) fn read_many(paths: &[PathBuf]) -> io::Result<Vec<io::Result<Vec<u8>>>> {
    let reader = Reader::new()?;
    Ok(reader.read_many(paths))
}

fn c_string(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|value| **value != 0)
        .map(|value| *value as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
