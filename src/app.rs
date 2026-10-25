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

//! Process-wide application state: the swappable database set and the Tokio
//! runtime. Kept out of `main` so modules never depend on the binary root.

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use sqlx::SqlitePool;
use tokio::runtime::Runtime;

static DBS: OnceLock<RwLock<Arc<crate::db::Dbs>>> = OnceLock::new();

/// The databases opened at start-up. An owned handle over a swappable slot —
/// repeated wipes no longer leak a database set per pass; the old set is
/// dropped and its pools closed.
pub fn dbs() -> Option<Arc<crate::db::Dbs>> {
    let slot = DBS.get()?;
    let guard = slot.read().unwrap_or_else(PoisonError::into_inner);
    Some(Arc::clone(&guard))
}

/// Swaps in a freshly initialised set (the full wipe deletes the files and
/// re-inits). The previous set's pools — including the separate `search`
/// pool the wipe never closed — are closed on the runtime afterwards.
pub fn set_dbs(dbs: crate::db::Dbs) {
    // ARC-SEARCH: the in-memory catalog belongs to the database it was built
    // from; a fresh set starts cold and rebuilds on first search/sync.
    crate::search::index::clear();
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
fn drop_dbs(old: Arc<crate::db::Dbs>) {
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
    RT.get_or_init(|| match Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("runtime: could not start Tokio: {err}");
            std::process::exit(1);
        }
    })
}
