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
use sqlx::SqlitePool;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use tokio::runtime::Runtime;

mod api;
mod artist_user;
mod browse;
mod cache;
mod colors;
mod consts;
mod covers;
mod crypto;
mod db;
mod debug;
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
mod palette;
mod player;
mod playlists;
mod qml;
mod queue;
mod search;
mod sqlite;
mod status;

static DBS: OnceLock<RwLock<Arc<db::Dbs>>> = OnceLock::new();

/// The five databases opened at start-up. FIX-3: an owned handle over a
/// swappable slot — repeated wipes no longer leak a leaked database set per
/// pass; the old set is dropped and its pools closed.
pub fn dbs() -> Option<Arc<db::Dbs>> {
    let slot = DBS.get()?;
    let guard = slot.read().unwrap_or_else(PoisonError::into_inner);
    Some(Arc::clone(&guard))
}

/// Swaps in a freshly initialised set (the full wipe deletes the files and
/// re-inits). The previous set's pools — including the separate `search`
/// pool `wipe_files` never closed — are closed on the runtime afterwards, so
/// repeated wipes do not accumulate stale connections over unlinked files.
pub fn set_dbs(dbs: db::Dbs) {
    // ARC-SEARCH: the in-memory catalog belongs to the database it was built
    // from; a fresh set starts cold and rebuilds on first search/sync.
    search::index::clear();
    let dbs = Arc::new(dbs);
    match DBS.get() {
        Some(slot) => {
            let mut guard = slot.write().unwrap_or_else(PoisonError::into_inner);
            let old = std::mem::replace(&mut *guard, Arc::clone(&dbs));
            drop(guard);
            drop_dbs(old);
        }
        None => {
            let _ = DBS.set(RwLock::new(dbs));
        }
    }
}

/// Closes the distinct pools of an old database set off the Qt/DB threads.
/// `library`/`credentials` share one pool, so do `media`/`covers`/…; only
/// `search` is separate (FIX-3).
fn drop_dbs(old: Arc<db::Dbs>) {
    runtime().spawn(async move {
        for pool in [&old.library, &old.search, &old.media] {
            pool.clone().close().await;
        }
    });
}

/// Convenience accessor for the credentials database.
pub fn pool() -> Option<SqlitePool> {
    dbs().map(|dbs| dbs.credentials.clone())
}

pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

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

    load_dotenv();

    if std::env::var("QSG_RHI_BACKEND").is_err() {
        unsafe {
            std::env::set_var("QSG_RHI_BACKEND", "vulkan");
        }
    }

    let dbs = runtime()
        .block_on(db::init())
        .expect("failed to initialise the databases");
    set_dbs(dbs);

    let mut app = QApp::new();
    qtbridge::include_bytes_qml!("qml", "app");
    qtbridge::include_bytes_qml!("icons", "app");
    crate::mpris::init();

    app.application_name("musika")
        .register::<qml::Backend>()
        .register::<palette::Palette>()
        .load_qml_from_file("qrc:/app/qml/Main.qml")
        .run();
}
