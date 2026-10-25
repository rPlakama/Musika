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

//! Login, auto-login and startup account restoration for the bridge.

use qtbridge::QmlObject;

use crate::http_client::SubsonicClient;

use super::Backend;
use super::download::downloaded_music_available;
use super::sync::online_authenticated;

impl Backend {
    pub(super) fn login_impl(&mut self) {
        let server_url = self.server_url.trim().to_string();
        let username = self.username.trim().to_string();
        let password = self.password.clone();

        if server_url.is_empty() || username.is_empty() || password.is_empty() {
            self.status_message = "Server, username and password are all required.".to_string();
            self.status_message_changed();
            return;
        }

        self.busy = true;
        self.busy_changed();
        self.status_message = format!("Connecting to {server_url}…");
        self.status_message_changed();

        let invoker = self.get_qml_method_invoker();
        let sync_finish = self.get_qml_method_invoker();
        let sync_progress = self.get_qml_method_invoker();
        let pool = crate::app::pool();

        crate::app::runtime().spawn(async move {
            match crate::http_client::authenticate(&server_url, &username, &password).await {
                Ok(info) => {
                    if let Some(pool) = pool.as_ref() {
                        if let Err(err) = crate::sqlite::save_credentials(
                            pool,
                            &username,
                            &password,
                            &server_url,
                            &info.kind,
                        )
                        .await
                        {
                            eprintln!("Failed to persist credentials: {err:#}");
                        }
                    }
                    online_authenticated(
                        &invoker,
                        sync_finish,
                        sync_progress,
                        SubsonicClient::new(&server_url, &username, &password),
                        info.kind,
                        true,
                    );
                }
                Err(err) => {
                    // CACHE-9: with downloaded music, a dead server enters
                    // offline mode instead of bouncing back to the Login.
                    if downloaded_music_available().await {
                        qtbridge::invoke_method!(
                            invoker,
                            "offlineLogin",
                            server_url.clone(),
                            username.clone()
                        );
                    } else {
                        qtbridge::invoke_method!(invoker, "login_failed", format!("{err:#}"));
                    }
                }
            }
        });
    }

    pub(super) fn startup_impl(&mut self) {
        #[cfg(debug_assertions)]
        crate::debug_info::since_start("qml.completed");
        let album_order =
            crate::library::effective_order(&crate::library::ORDERS, &self.album_order, 0)
                .to_string();
        if self.album_order != album_order {
            self.album_order = album_order;
            self.album_order_changed();
        }
        let song_order =
            crate::library::effective_order(&crate::library::ORDERS, &self.song_order, 0)
                .to_string();
        if self.song_order != song_order {
            self.song_order = song_order;
            self.song_order_changed();
        }
        let artist_order =
            crate::library::effective_order(&crate::library::ARTIST_ORDERS, &self.artist_order, 0)
                .to_string();
        if self.artist_order != artist_order {
            self.artist_order = artist_order;
            self.artist_order_changed();
        }
        let invoker = self.get_qml_method_invoker();
        let sync_finish = self.get_qml_method_invoker();
        let sync_progress = self.get_qml_method_invoker();
        let Some(pool) = crate::app::pool() else {
            return;
        };

        // Auto-login with the ping: the stored account always tries to
        // complete the login on its own. With downloaded music the app opens
        // right away as `offline` (filtered to the downloaded catalog) while
        // the auth finishes in the background — on success the flag clears
        // and the full streaming library loads. Without downloads it waits
        // for the same ping and only returns to the Login form, prefilled,
        // when the server is unreachable.
        crate::app::runtime().spawn(async move {
            match crate::sqlite::load_credentials(&pool).await {
                Ok(Some(credentials)) => {
                    // The stored discriminator keeps "Last Sync: …, <server>"
                    // populated before the first fresh ping (offline too).
                    qtbridge::invoke_method!(
                        invoker,
                        "noteServerType",
                        credentials.server_type.clone()
                    );

                    let has_downloads = downloaded_music_available().await;
                    if has_downloads {
                        qtbridge::invoke_method!(
                            invoker,
                            "offlineLogin",
                            credentials.server_url.clone(),
                            credentials.username.clone()
                        );
                    }

                    // Finish the login inside the app (the ping). A failure
                    // with a cached catalog just stays offline; without one it
                    // falls back to the Login form with the account prefilled.
                    match crate::http_client::authenticate(
                        &credentials.server_url,
                        &credentials.username,
                        &credentials.password,
                    )
                    .await
                    {
                        Ok(info) => {
                            // Auto-login has no login form to set these: record the
                            // account before the sync so the header greets the user
                            // and the account-scoped status can restore.
                            qtbridge::invoke_method!(
                                invoker,
                                "rememberAccount",
                                credentials.server_url.clone(),
                                credentials.username.clone()
                            );
                            online_authenticated(
                                &invoker,
                                sync_finish,
                                sync_progress,
                                SubsonicClient::new(
                                    &credentials.server_url,
                                    &credentials.username,
                                    &credentials.password,
                                ),
                                info.kind,
                                !has_downloads,
                            );
                        }
                        Err(err) if has_downloads => {
                            // Stay offline; the auth is retried on the next
                            // sync/login.
                            eprintln!("login: background auth failed, staying offline: {err:#}");
                        }
                        Err(err) => {
                            eprintln!("login: startup auth failed: {err:#}");
                            qtbridge::invoke_method!(
                                invoker,
                                "prefillCredentials",
                                credentials.server_url,
                                credentials.username,
                                credentials.password
                            );
                            invoker.invoke_method("login_required");
                        }
                    }
                }
                Ok(None) => {
                    invoker.invoke_method("login_required");
                }
                Err(err) => {
                    eprintln!("Failed to load saved credentials: {err:#}");
                    invoker.invoke_method("login_required");
                }
            }
        });
    }
}
