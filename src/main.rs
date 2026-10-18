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
use std::sync::OnceLock;
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
mod download;
mod fanout;
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

static DBS: OnceLock<std::sync::Mutex<&'static db::Dbs>> = OnceLock::new();

/// The five databases opened at start-up.
pub fn dbs() -> Option<&'static db::Dbs> {
    DBS.get().map(|slot| *slot.lock().expect("dbs lock"))
}

/// Swaps in a freshly initialised set (the full wipe deletes the files and
/// re-inits; the old handle leaks, which is once-per-wipe).
pub fn set_dbs(dbs: db::Dbs) {
    let leaked: &'static db::Dbs = Box::leak(Box::new(dbs));
    match DBS.get() {
        Some(slot) => *slot.lock().expect("dbs lock") = leaked,
        None => {
            let _ = DBS.set(std::sync::Mutex::new(leaked));
        }
    }
}

/// Convenience accessor for the credentials database.
pub fn pool() -> Option<&'static SqlitePool> {
    dbs().map(|dbs| &dbs.credentials)
}

pub fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

fn main() {
    dotenvy::dotenv().ok();

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
