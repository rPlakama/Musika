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
//! without depending on `legacy`.

#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::http_client::SubsonicClient;

/// Whether the server is reachable for on-demand work outside the sync (cover
/// misses): offline logins and the pre-login state keep it false.
static ONLINE: AtomicBool = AtomicBool::new(false);

/// One client per credential set: the reqwest pool/TLS context is shared by
/// every action (and the credential read only happens when the key changes).
#[cfg(not(test))]
static CLIENT_CACHE: OnceLock<tokio::sync::Mutex<Option<CachedClient>>> = OnceLock::new();

#[cfg(not(test))]
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

/// Installs the credentials just used to authenticate, so the first [`client`]
/// call on any action does not re-read and decrypt them from the database.
#[cfg(not(test))]
pub(crate) async fn install(server_url: &str, username: &str, password: &str) {
    let cache = CLIENT_CACHE.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = cache.lock().await;
    if let Some(cached) = guard.as_ref()
        && cached.server_url == server_url
        && cached.username == username
        && cached.password == password
    {
        return;
    }
    *guard = Some(CachedClient {
        server_url: server_url.to_string(),
        username: username.to_string(),
        password: password.to_string(),
        client: SubsonicClient::new(server_url, username, password),
    });
}

#[cfg(not(test))]
pub(crate) async fn client(dbs: &crate::db::Dbs) -> anyhow::Result<SubsonicClient> {
    let cache = CLIENT_CACHE.get_or_init(|| tokio::sync::Mutex::new(None));
    if let Some(cached) = cache.lock().await.as_ref() {
        return Ok(cached.client.clone());
    }
    let credentials = crate::sqlite::load_credentials(&dbs.credentials)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no saved credentials"))?;
    install(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    )
    .await;
    let client = cache
        .lock()
        .await
        .as_ref()
        .expect("client installed above")
        .client
        .clone();
    Ok(client)
}

#[cfg(test)]
pub(crate) async fn install(_server_url: &str, _username: &str, _password: &str) {}

/// Tests use independent in-memory credential DBs; bypass the process-wide
/// cache so one test cannot leak a session into another.
#[cfg(test)]
pub(crate) async fn client(dbs: &crate::db::Dbs) -> anyhow::Result<SubsonicClient> {
    let credentials = crate::sqlite::load_credentials(&dbs.credentials)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no saved credentials"))?;
    Ok(SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    ))
}

/// Drops the cached client (account switch, logout or a full wipe).
#[cfg(not(test))]
pub(crate) async fn invalidate() {
    if let Some(cache) = CLIENT_CACHE.get() {
        *cache.lock().await = None;
    }
}

#[cfg(test)]
pub(crate) async fn invalidate() {}
