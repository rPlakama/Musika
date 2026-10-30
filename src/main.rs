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

use qtbridge::QApp;

mod api;
mod app;
mod artist_user;
mod browse;
mod cache;
mod colors;
mod consts;
mod covers;
mod crypto;
mod db;
mod debug;
#[cfg(debug_assertions)]
#[path = "test/debug.rs"]
mod debug_info;
mod digest;
mod download;
mod fanout;
mod hex;
mod http_client;
mod index;
mod ipc;
mod keybind;
mod library;
mod lyrics;
mod media;
mod mpris;
mod offline;
mod palette;
mod player;
mod playlists;
mod qml;
mod queue;
mod scroll_memory;
mod search;
mod session;
mod sqlite;
mod status;

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

fn load_dotenv() {
    let Ok(text) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches(['"', '\'']);
        if !key.is_empty() && std::env::var_os(key).is_none() {
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

fn main() {
    // CLI probe before any Qt/database work, so `musika --version` also
    // works on a headless host.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("musika {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    #[cfg(debug_assertions)]
    crate::debug_info::start();

    load_dotenv();

    if std::env::var("QSG_RHI_BACKEND").is_err() {
        unsafe {
            std::env::set_var("QSG_RHI_BACKEND", "vulkan");
        }
    }

    #[cfg(debug_assertions)]
    let db_started = std::time::Instant::now();
    let dbs = match crate::app::runtime().block_on(db::init()) {
        Ok(dbs) => dbs,
        Err(err) => {
            eprintln!("database: failed to initialise: {err:#}");
            std::process::exit(1);
        }
    };
    #[cfg(debug_assertions)]
    crate::debug_info::report("db.init", db_started);

    // The login screen must not flash for an account already in the DB.
    // Read it before QML loads; the async startup ping still owns the
    // online/offline state and the failure fallback.
    let credentials = crate::app::runtime()
        .block_on(crate::sqlite::load_credentials(&dbs.credentials))
        .ok()
        .flatten();
    let has_stored_auth = credentials.is_some();
    crate::qml::set_bootstrap_has_auth(has_stored_auth);

    // If the saved session points at an album or artist, build its row now so
    // Home can push that page before its first frame instead of flashing the
    // library. Old rows without page_kind fall back to album_id.
    let initial_target = credentials.as_ref().and_then(|credentials| {
        let status = crate::app::runtime()
            .block_on(crate::status::load(&dbs.status))
            .ok()
            .flatten()?;
        if status.server_url != credentials.server_url || status.username != credentials.username {
            return None;
        }
        let (kind, id) = if !status.page_kind.is_empty() && !status.page_id.is_empty() {
            (status.page_kind.clone(), status.page_id.clone())
        } else {
            (
                "album".to_string(),
                status.album_id.clone().filter(|id| !id.is_empty())?,
            )
        };
        let page = match kind.as_str() {
            "album" => {
                let album = crate::app::runtime()
                    .block_on(crate::library::album(&dbs.library, &id))
                    .ok()
                    .flatten()?;
                let cover_ids: Vec<String> = album.cover_art.clone().into_iter().collect();
                let covers = crate::app::runtime()
                    .block_on(crate::covers::url_map_of(&dbs, &cover_ids))
                    .unwrap_or_default();
                crate::browse::album_json(&album, &covers, None)
            }
            "artist" => {
                let artist = crate::app::runtime()
                    .block_on(crate::library::artist(&dbs.library, &id))
                    .ok()
                    .flatten()?;
                let cover_ids: Vec<String> = artist.cover_art.clone().into_iter().collect();
                let covers = crate::app::runtime()
                    .block_on(crate::covers::url_map_of(&dbs, &cover_ids))
                    .unwrap_or_default();
                let user_starred = crate::app::runtime()
                    .block_on(crate::artist_user::starred_ids(&dbs.artist_user))
                    .unwrap_or_default();
                let starred = artist.starred.is_some() || user_starred.contains(&artist.id);
                let rating = crate::app::runtime()
                    .block_on(crate::artist_user::ratings(&dbs.artist_user))
                    .ok()
                    .and_then(|ratings| ratings.get(&artist.id).copied())
                    .unwrap_or(0);
                crate::browse::artist_json(&artist, &covers, None, starred, rating)
            }
            _ => return None,
        };
        Some(serde_json::json!({ "kind": kind, "page": page }).to_string())
    });
    crate::qml::set_bootstrap_target(initial_target);
    crate::app::set_dbs(dbs);

    let mut app = QApp::new();
    qtbridge::include_bytes_qml!("qml", "app");
    qtbridge::include_bytes_qml!("icons", "app");
    crate::mpris::init();

    app.application_name("musika")
        .register::<qml::Backend>()
        .register::<palette::Palette>()
        .load_qml_from_file("qrc:/app/qml/Main.qml");
    #[cfg(debug_assertions)]
    {
        crate::debug_info::since_start("qml.loaded");
        crate::debug_info::print_diagnostics();
    }
    app.run();
}
