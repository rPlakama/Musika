// Transitional: legacy backend helpers remain compiled until they are
// cleaned up or reused by the GPUI front end.
#![allow(dead_code)]

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

mod accounts;
mod api;
mod app;
mod artist_user;
#[cfg(feature = "gpuui")]
mod auth;
mod bridge;
mod browse;
mod cache;
mod colors;
mod consts;
mod covers;
mod crypto;
mod daemon;
mod db;
mod debug;
#[cfg(any(debug_assertions, test))]
#[path = "test/debug.rs"]
mod debug_info;
mod digest;
mod download;
#[cfg(feature = "gpuui")]
mod download_command;
mod fanout;
mod hex;
mod http_client;
mod index;
#[cfg(feature = "io-uring")]
mod io_uring_read;
mod ipc;
mod ipc_command;
mod library;
mod linux;
mod lyrics;
mod media;
mod media_actions;
mod model;
mod mpris;
mod offline;
mod player;
mod playlists;
mod queue;
mod radio;
mod rows;
mod scroll_memory;
mod search;
mod session;
mod sqlite;
mod startup;
mod status;
mod tray;
#[cfg(feature = "gpuui")]
mod ui;

#[cfg(test)]
mod test;

/// Test-only heap probe (see `test::colors::bench_palette_decode_peak`).
///
/// The memory work needs numbers, not guesses: `/proc/self/status` only
/// exposes a monotonic high-water mark, so a per-thread allocator counter
/// measures one operation's peak while the fixture stays alive. Tracking is
/// off unless a test opts in on its own thread; the production build does not
/// compile this module or the allocator below.
#[cfg(test)]
pub(crate) mod test_alloc {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        static TRACK: Cell<bool> = const { Cell::new(false) };
        static CURRENT: Cell<isize> = const { Cell::new(0) };
        static PEAK: Cell<isize> = const { Cell::new(0) };
    }

    /// A `System` allocator that records live bytes on the measuring thread.
    pub(crate) struct TrackingAllocator;

    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                add(layout.size() as isize);
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                add(layout.size() as isize);
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            add(-(layout.size() as isize));
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let new = unsafe { System.realloc(ptr, layout, new_size) };
            if !new.is_null() {
                add(new_size as isize - layout.size() as isize);
            }
            new
        }
    }

    fn add(delta: isize) {
        // Runs on every allocation of every test: touch TLS only when this
        // thread is measuring, and never allocate from inside the hook.
        let _ = TRACK.try_with(|track| {
            if !track.get() {
                return;
            }
            let _ = CURRENT.try_with(|current| {
                let live = current.get() + delta;
                current.set(live);
                let _ = PEAK.try_with(|peak| {
                    if live > peak.get() {
                        peak.set(live);
                    }
                });
            });
        });
    }

    /// Runs `f` with tracking armed; returns its value and the peak live heap
    /// bytes observed on this thread.
    pub(crate) fn measure<T>(f: impl FnOnce() -> T) -> (T, isize) {
        CURRENT.with(|current| current.set(0));
        PEAK.with(|peak| peak.set(0));
        TRACK.with(|track| track.set(true));
        let value = f();
        TRACK.with(|track| track.set(false));
        (value, PEAK.with(|peak| peak.get()))
    }

    /// Runs `f` with tracking armed and returns its value and the **retained**
    /// live heap after it returns. Use when the fixture is created inside the
    /// closure and its consumed buffers should cancel out (the search catalog
    /// benchmark).
    pub(crate) fn live<T>(f: impl FnOnce() -> T) -> (T, isize) {
        CURRENT.with(|current| current.set(0));
        TRACK.with(|track| track.set(true));
        let value = f();
        TRACK.with(|track| track.set(false));
        (value, CURRENT.with(|current| current.get()))
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: test_alloc::TrackingAllocator = test_alloc::TrackingAllocator;

pub mod entry;
