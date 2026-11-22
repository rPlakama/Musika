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
//! Kept out of the bridge module so cache and payload modules can use the
//! client without depending on bridge state.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::watch;

use crate::http_client::SubsonicClient;

/// Whether the server is reachable for on-demand work outside the sync (cover
/// misses): offline logins and the pre-login state keep it false.
static ONLINE: AtomicBool = AtomicBool::new(false);
/// The same flag as a change feed. Long-running work (the library sync) parks
/// here instead of polling, and wakes as soon as the health probe reports a
/// transition.
static ONLINE_WATCH: OnceLock<watch::Sender<bool>> = OnceLock::new();

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
    ONLINE.load(Ordering::Acquire)
}

pub(crate) fn set_online(online: bool) -> bool {
    let mut was = online;
    let _ = online_watch().send_if_modified(|value| {
        was = *value;
        ONLINE.store(online, Ordering::Release);
        if *value == online {
            false
        } else {
            *value = online;
            true
        }
    });
    was
}

/// A receiver for connectivity transitions. The current value is delivered
/// immediately, so a sync starting while offline sees `false` before it waits.
pub(crate) fn watch_online() -> watch::Receiver<bool> {
    online_watch().subscribe()
}

fn online_watch() -> &'static watch::Sender<bool> {
    ONLINE_WATCH.get_or_init(|| {
        let (tx, _rx) = watch::channel(ONLINE.load(Ordering::Acquire));
        tx
    })
}

#[cfg(not(test))]
fn client_cache() -> &'static tokio::sync::Mutex<Option<CachedClient>> {
    CLIENT_CACHE.get_or_init(|| tokio::sync::Mutex::new(None))
}

#[cfg(not(test))]
fn install_locked(
    guard: &mut Option<CachedClient>,
    server_url: &str,
    username: &str,
    password: &str,
) -> SubsonicClient {
    if let Some(cached) = guard.as_ref()
        && cached.server_url == server_url
        && cached.username == username
        && cached.password == password
    {
        return cached.client.clone();
    }
    let client = SubsonicClient::new(server_url, username, password);
    *guard = Some(CachedClient {
        server_url: server_url.to_string(),
        username: username.to_string(),
        password: password.to_string(),
        client: client.clone(),
    });
    client
}

/// Installs the credentials just used to authenticate, so the first [`client`]
/// call on any action does not re-read and decrypt them from the database.
#[cfg(not(test))]
pub(crate) async fn install(server_url: &str, username: &str, password: &str) {
    let mut guard = client_cache().lock().await;
    let _ = install_locked(&mut guard, server_url, username, password);
}

#[cfg(not(test))]
pub(crate) async fn client(_dbs: &crate::db::Dbs) -> anyhow::Result<SubsonicClient> {
    let mut guard = client_cache().lock().await;
    if let Some(cached) = guard.as_ref() {
        return Ok(cached.client.clone());
    }
    let account =
        crate::accounts::active_account().ok_or_else(|| anyhow::anyhow!("no active account"))?;
    Ok(install_locked(
        &mut guard,
        &account.server_url,
        &account.username,
        &account.password,
    ))
}

#[cfg(test)]
#[expect(
    clippy::unused_async,
    reason = "the test stub mirrors the async non-test signature used by shared call sites"
)]
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
    *client_cache().lock().await = None;
}

#[cfg(test)]
#[expect(
    clippy::unused_async,
    reason = "the test stub mirrors the async non-test signature used by shared call sites"
)]
pub(crate) async fn invalidate() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_online_keeps_atomic_and_watch_consistent() {
        let rx = watch_online();
        for state in [true, false, true, true, false] {
            let _ = set_online(state);
            assert_eq!(online(), state);
            assert_eq!(*rx.borrow(), state);
        }
    }
}
