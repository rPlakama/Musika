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

//! The live server session: online flag + one credential-keyed client.
//!
//! Kept out of the Qt bridge so cache and payload modules can use the client
//! without depending on `qml`.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::http_client::SubsonicClient;

/// Whether the server is reachable for on-demand work outside the sync (cover
/// misses): offline logins and the pre-login state keep it false.
static ONLINE: AtomicBool = AtomicBool::new(false);

/// One client per credential set: the reqwest pool/TLS context is shared by
/// every action (and the credential read only happens when the key changes).
static CLIENT_CACHE: OnceLock<tokio::sync::Mutex<Option<CachedClient>>> = OnceLock::new();

struct CachedClient {
    server_url: String,
    username: String,
    password: String,
    client: SubsonicClient,
}

pub(crate) fn online() -> bool {
    ONLINE.load(Ordering::Relaxed)
}

pub(crate) fn set_online(online: bool) {
    ONLINE.store(online, Ordering::Relaxed);
}

pub(crate) async fn client(dbs: &crate::db::Dbs) -> anyhow::Result<SubsonicClient> {
    let credentials = crate::sqlite::load_credentials(&dbs.credentials)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no saved credentials"))?;
    let cache = CLIENT_CACHE.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = cache.lock().await;
    if let Some(cached) = guard.as_ref()
        && cached.server_url == credentials.server_url
        && cached.username == credentials.username
        && cached.password == credentials.password
    {
        return Ok(cached.client.clone());
    }
    let client = SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    );
    *guard = Some(CachedClient {
        server_url: credentials.server_url,
        username: credentials.username,
        password: credentials.password,
        client: client.clone(),
    });
    Ok(client)
}

/// Drops the cached client (account switch, logout or a full wipe).
pub(crate) async fn invalidate() {
    if let Some(cache) = CLIENT_CACHE.get() {
        *cache.lock().await = None;
    }
}
