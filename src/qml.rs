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

use std::collections::HashMap;
use std::sync::Arc;

use qtbridge::{QmlMethodInvoker, QmlObject, qobject};

use crate::http_client::SubsonicClient;
use crate::keybind::{Keymap, Outcome};
use crate::player::{Player, State};

#[derive(Default)]
pub struct Backend {
    server_url: String,
    username: String,
    password: String,
    status_message: String,
    busy: bool,
    logged_in: bool,
    library_syncing: bool,
    artist_count: i64,
    album_count: i64,
    song_count: i64,
    sync_progress: f64,
    sync_message: String,
    last_sync: String,
    keymap: Keymap,
    player: Player,
    colors_in_flight: Option<String>,
    current_title: String,
    current_artist: String,
    current_album: String,
    current_cover_art: String,
    current_cover_url: String,
    current_song_id: String,
    playback_state: String,
    position: f64,
    duration: f64,
    volume: f64,
    muted: bool,
    queue_index: i64,
    queue_length: i64,
    album_order: String,
    server_type: String,
    search_open: bool,
}

#[qobject(Singleton)]
impl Backend {
    qproperty!(
        "serverURL",
        Member = server_url,
        Write = set_server_url,
        Notify = server_url_changed
    );
    qproperty!(
        "username",
        Member = username,
        Write = set_username,
        Notify = username_changed
    );
    qproperty!(
        "password",
        Member = password,
        Write = set_password,
        Notify = password_changed
    );
    qproperty!(
        "statusMessage",
        Member = status_message,
        Notify = status_message_changed
    );
    qproperty!("busy", Member = busy, Notify = busy_changed);
    qproperty!("loggedIn", Member = logged_in, Notify = logged_in_changed);
    qproperty!(
        "librarySyncing",
        Member = library_syncing,
        Notify = library_syncing_changed
    );
    qproperty!(
        "artistCount",
        Member = artist_count,
        Notify = artist_count_changed
    );
    qproperty!(
        "albumCount",
        Member = album_count,
        Notify = album_count_changed
    );
    qproperty!(
        "songCount",
        Member = song_count,
        Notify = song_count_changed
    );
    qproperty!(
        "syncProgress",
        Member = sync_progress,
        Notify = sync_progress_changed
    );
    qproperty!(
        "syncMessage",
        Member = sync_message,
        Notify = sync_message_changed
    );
    qproperty!("lastSync", Member = last_sync, Notify = last_sync_changed);
    qproperty!(
        "currentTitle",
        Member = current_title,
        Notify = current_title_changed
    );
    qproperty!(
        "currentArtist",
        Member = current_artist,
        Notify = current_artist_changed
    );
    qproperty!(
        "currentAlbum",
        Member = current_album,
        Notify = current_album_changed
    );
    qproperty!(
        "currentCoverArt",
        Member = current_cover_art,
        Notify = current_cover_art_changed
    );
    qproperty!(
        "currentCoverUrl",
        Member = current_cover_url,
        Notify = current_cover_url_changed
    );
    qproperty!(
        "currentSongId",
        Member = current_song_id,
        Notify = current_song_id_changed
    );
    qproperty!(
        "playbackState",
        Member = playback_state,
        Notify = playback_state_changed
    );
    qproperty!("position", Member = position, Notify = position_changed);
    qproperty!("duration", Member = duration, Notify = duration_changed);
    qproperty!("volume", Member = volume, Notify = volume_changed);
    qproperty!("muted", Member = muted, Notify = muted_changed);
    qproperty!(
        "queueIndex",
        Member = queue_index,
        Notify = queue_index_changed
    );
    qproperty!(
        "queueLength",
        Member = queue_length,
        Notify = queue_length_changed
    );
    qproperty!(
        "albumOrder",
        Member = album_order,
        Notify = album_order_changed
    );
    qproperty!(
        "serverType",
        Member = server_type,
        Notify = server_type_changed
    );

    fn set_server_url(&mut self, v: String) {
        self.server_url = v;
        self.server_url_changed();
    }
    fn set_username(&mut self, v: String) {
        self.username = v;
        self.username_changed();
    }
    fn set_password(&mut self, v: String) {
        self.password = v;
        self.password_changed();
    }

    #[qsignal]
    fn server_url_changed(&mut self);
    #[qsignal]
    fn username_changed(&mut self);
    #[qsignal]
    fn password_changed(&mut self);
    #[qsignal]
    fn status_message_changed(&mut self);
    #[qsignal]
    fn busy_changed(&mut self);
    #[qsignal]
    fn logged_in_changed(&mut self);
    #[qsignal]
    fn library_syncing_changed(&mut self);
    #[qsignal]
    fn artist_count_changed(&mut self);
    #[qsignal]
    fn album_count_changed(&mut self);
    #[qsignal]
    fn song_count_changed(&mut self);
    #[qsignal]
    fn sync_progress_changed(&mut self);
    #[qsignal]
    fn sync_message_changed(&mut self);
    #[qsignal]
    fn last_sync_changed(&mut self);
    #[qsignal]
    fn current_title_changed(&mut self);
    #[qsignal]
    fn current_artist_changed(&mut self);
    #[qsignal]
    fn current_album_changed(&mut self);
    #[qsignal]
    fn current_cover_art_changed(&mut self);
    #[qsignal]
    fn current_cover_url_changed(&mut self);
    #[qsignal]
    fn current_song_id_changed(&mut self);
    #[qsignal(qml_name = "dataReady")]
    fn data_ready(&mut self, channel: String, payload: String);
    #[qsignal(qml_name = "errorRaised")]
    fn error_raised(&mut self, kind: String, message: String);
    #[qsignal(qml_name = "commandIssued")]
    fn command_issued(&mut self, command: String);
    #[qsignal(qml_name = "loginRequired")]
    fn login_required(&mut self);
    #[qsignal(qml_name = "loginPending")]
    fn login_pending(&mut self);
    #[qsignal]
    fn playback_state_changed(&mut self);
    #[qsignal]
    fn position_changed(&mut self);
    #[qsignal]
    fn duration_changed(&mut self);
    #[qsignal]
    fn volume_changed(&mut self);
    #[qsignal]
    fn muted_changed(&mut self);
    #[qsignal]
    fn queue_index_changed(&mut self);
    #[qsignal]
    fn queue_length_changed(&mut self);
    #[qsignal]
    fn album_order_changed(&mut self);
    #[qsignal]
    fn server_type_changed(&mut self);
    #[qsignal]
    fn library_changed(&mut self);
    /// User skip (transport/keyboard/IPC), so the skip buttons can bloom.
    #[qsignal(qml_name = "trackSkipped")]
    fn track_skipped(&mut self, forward: bool);
    /// A new queue started playing; the UI opens Fullscreen Playback (Q-10).
    #[qsignal(qml_name = "playbackStarted")]
    fn playback_started(&mut self);
    #[qsignal(qml_name = "focusMain")]
    fn focus_main(&mut self);

    #[qslot]
    fn login(&mut self) {
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
        let pool = crate::pool().cloned();

        crate::runtime().spawn(async move {
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
                    qtbridge::invoke_method!(invoker, "noteServerType", info.kind.clone());
                    invoker.invoke_method("login_succeeded");
                    spawn_index_sync(sync_finish, sync_progress, server_url, username, password);
                }
                Err(err) => {
                    qtbridge::invoke_method!(invoker, "login_failed", format!("{err:#}"));
                }
            }
        });
    }

    #[qslot]
    fn startup(&mut self) {
        if self.album_order.is_empty() {
            self.album_order = crate::library::ALBUM_ORDERS[0].to_string();
            self.album_order_changed();
        }
        let invoker = self.get_qml_method_invoker();
        let sync_finish = self.get_qml_method_invoker();
        let sync_progress = self.get_qml_method_invoker();
        let Some(pool) = crate::pool().cloned() else {
            return;
        };

        crate::runtime().spawn(async move {
            match crate::sqlite::load_credentials(&pool).await {
                Ok(Some(credentials)) => {
                    // Cached login: show a loading screen while we ping.
                    invoker.invoke_method("login_pending");

                    let ping = tokio::time::timeout(
                        std::time::Duration::from_millis(5000),
                        crate::http_client::authenticate(
                            &credentials.server_url,
                            &credentials.username,
                            &credentials.password,
                        ),
                    )
                    .await;

                    match ping {
                        Ok(Ok(info)) => {
                            qtbridge::invoke_method!(invoker, "noteServerType", info.kind.clone());
                            if credentials.server_type != info.kind {
                                if let Err(err) = crate::sqlite::set_server_type(
                                    &pool,
                                    &credentials.username,
                                    &credentials.server_url,
                                    &info.kind,
                                )
                                .await
                                {
                                    eprintln!("Failed to refresh server type: {err:#}");
                                }
                            }
                            invoker.invoke_method("login_succeeded");
                            spawn_index_sync(
                                sync_finish,
                                sync_progress,
                                credentials.server_url,
                                credentials.username,
                                credentials.password,
                            );
                        }
                        Ok(Err(err)) => {
                            qtbridge::invoke_method!(invoker, "login_failed", format!("{err:#}"));
                        }
                        Err(_) => {
                            qtbridge::invoke_method!(
                                invoker,
                                "login_failed",
                                "Timed out waiting for server".to_string()
                            );
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

    #[qslot]
    fn login_succeeded(&mut self) {
        self.busy = false;
        self.busy_changed();
        self.logged_in = true;
        self.logged_in_changed();
        self.status_message = "Logged in.".to_string();
        self.status_message_changed();
    }

    #[qslot]
    fn login_failed(&mut self, message: String) {
        self.busy = false;
        self.busy_changed();
        self.logged_in = false;
        self.logged_in_changed();
        self.status_message = message;
        self.status_message_changed();
    }

    /// Records the server implementation reported by the ping envelope
    /// (`navidrome`, `subsonic`, …); drives the status-bar badge. Unknown
    /// servers fall back to the generic Subsonic icon.
    #[qslot(qml_name = "noteServerType")]
    fn note_server_type(&mut self, kind: String) {
        let kind = if kind.trim().is_empty() {
            "subsonic".to_string()
        } else {
            kind.trim().to_ascii_lowercase()
        };
        if self.server_type != kind {
            self.server_type = kind;
            self.server_type_changed();
        }
    }

    /// Forwards a raw Qt key event to the keybind engine.
    ///
    /// Returns the action name when a binding fires, `"PENDING"` when more
    /// input (or a timeout) is needed, otherwise an empty string.
    #[qslot(qml_name = "keyEvent")]
    fn key_event(&mut self, key: i32, modifiers: i32) -> String {
        let Some(token) = crate::keybind::token_from_qt(key, modifiers) else {
            self.keymap.reset();
            return String::new();
        };

        // While the search overlay is open the scope switch (`Ctrl-W`) is
        // simply unavailable: the overlay owns the keyboard context, and the
        // field handles Ctrl-W itself as word-delete while typing.
        if self.search_open && token == "ctrl-w" {
            self.keymap.reset();
            return String::new();
        }

        match self.keymap.feed(&token) {
            Outcome::Action(action) => action.name().to_string(),
            Outcome::Pending => "PENDING".to_string(),
            Outcome::None => String::new(),
        }
    }

    /// Marks the floating search as open/closed (see `key_event`).
    #[qslot(qml_name = "setSearchOpen")]
    fn set_search_open(&mut self, open: bool) {
        self.search_open = open;
        // Leaving the previous context may have left a half-typed sequence.
        self.keymap.reset();
    }

    /// Resolves a deferred key sequence after the inter-key timeout.
    #[qslot(qml_name = "keyTimeout")]
    fn key_timeout(&mut self) -> String {
        match self.keymap.feed_timeout() {
            Outcome::Action(action) => action.name().to_string(),
            Outcome::Pending | Outcome::None => String::new(),
        }
    }

    /// Reloads the counts/time shown by the right status bar.
    #[qslot(qml_name = "refreshLibraryStatus")]
    fn refresh_library_status(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };

        crate::runtime().spawn(async move {
            match crate::library::status(&dbs.library).await {
                Ok(status) => {
                    qtbridge::invoke_method!(
                        invoker,
                        "libraryStatus",
                        status.artists,
                        status.albums,
                        status.songs,
                        status.last_sync.unwrap_or_default()
                    );
                }
                Err(err) => eprintln!("index: could not read the library status: {err:#}"),
            }
        });
    }

    /// Applies a [`crate::library::LibraryStatus`] snapshot.
    #[qslot(qml_name = "libraryStatus")]
    fn library_status(&mut self, artists: i64, albums: i64, songs: i64, last_sync: String) {
        self.artist_count = artists;
        self.artist_count_changed();
        self.album_count = albums;
        self.album_count_changed();
        self.song_count = songs;
        self.song_count_changed();
        self.last_sync = last_sync;
        self.last_sync_changed();
    }

    #[qslot(qml_name = "librarySyncStarted")]
    fn library_sync_started(&mut self) {
        self.library_syncing = true;
        self.library_syncing_changed();
        self.sync_progress = 0.0;
        self.sync_progress_changed();
        self.sync_message = "Starting…".to_string();
        self.sync_message_changed();
    }

    #[qslot(qml_name = "librarySyncProgress")]
    fn library_sync_progress(&mut self, message: String, done: i64, total: i64) {
        self.sync_message = message;
        self.sync_message_changed();
        self.sync_progress = if total > 0 {
            done as f64 / total as f64
        } else {
            0.0
        };
        self.sync_progress_changed();
    }

    /// Manual entry point for library indexing (wired to the right status bar).
    /// Uses the saved credentials.
    #[qslot(qml_name = "syncLibrary")]
    fn sync_library(&mut self) {
        // Re-entrancy guard (ARC-28) and immediate "Syncing…" state (UI-30):
        // the manual path used to silently read "Up to date" for its duration.
        if self.library_syncing {
            return;
        }
        self.library_sync_started();
        let finish_invoker = self.get_qml_method_invoker();
        let progress_invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            qtbridge::invoke_method!(
                finish_invoker,
                "librarySyncFinished",
                "Library index is not available.".to_string()
            );
            return;
        };

        crate::runtime().spawn(async move {
            let credentials = match crate::sqlite::load_credentials(&dbs.credentials).await {
                Ok(Some(credentials)) => credentials,
                Ok(None) => {
                    qtbridge::invoke_method!(
                        finish_invoker,
                        "librarySyncFinished",
                        "No saved credentials to sync with.".to_string()
                    );
                    return;
                }
                Err(err) => {
                    qtbridge::invoke_method!(
                        finish_invoker,
                        "librarySyncFinished",
                        format!("Could not load credentials: {err:#}")
                    );
                    return;
                }
            };

            let client = Arc::new(SubsonicClient::new(
                &credentials.server_url,
                &credentials.username,
                &credentials.password,
            ));

            let progress = move |update: crate::index::SyncProgress| {
                qtbridge::invoke_method!(
                    progress_invoker,
                    "librarySyncProgress",
                    update.message,
                    update.done as i64,
                    update.total as i64
                );
            };

            match crate::index::sync_library(&dbs, client, progress).await {
                Ok(report) => {
                    qtbridge::invoke_method!(
                        finish_invoker,
                        "librarySyncFinished",
                        sync_summary(&report)
                    );
                }
                Err(err) => {
                    qtbridge::invoke_method!(
                        finish_invoker,
                        "librarySyncFinished",
                        format!("Sync failed: {err:#}")
                    );
                    qtbridge::invoke_method!(
                        finish_invoker,
                        "raiseError",
                        "sync".to_string(),
                        format!("{err:#}")
                    );
                }
            };
        });
    }

    #[qslot(qml_name = "librarySyncFinished")]
    fn library_sync_finished(&mut self, message: String) {
        self.library_syncing = false;
        self.library_syncing_changed();
        self.sync_progress = 1.0;
        self.sync_progress_changed();
        self.sync_message = message;
        self.sync_message_changed();
        self.refresh_library_status();
        self.library_changed();
    }

    // -----------------------------------------------------------------------
    // Async library reads (never block the Qt thread)
    // -----------------------------------------------------------------------

    /// Delivers an async payload to QML via the `dataReady` signal.
    #[qslot(qml_name = "emitData")]
    fn emit_data(&mut self, channel: String, payload: String) {
        self.data_ready(channel, payload);
    }

    /// Reports an error to the UI (`kind` is e.g. "sync" / "playback").
    #[qslot(qml_name = "raiseError")]
    fn raise_error(&mut self, kind: String, message: String) {
        self.error_raised(kind, message);
    }

    /// Runs a command received from the IPC socket. Forwarded to the UI as the
    /// same `commandIssued` stream the keybinds produce.
    #[qslot(qml_name = "handleCommand")]
    fn handle_command(&mut self, command: String) {
        self.command_issued(command);
    }

    /// Starts the external command socket (`<config>/ipc.sock`).
    #[qslot(qml_name = "startIpc")]
    fn start_ipc(&mut self) {
        let invoker = self.get_qml_method_invoker();
        crate::ipc::serve(crate::ipc::socket_path(), move |command| {
            qtbridge::invoke_method!(invoker, "handleCommand", command);
        });
    }

    #[qslot(qml_name = "requestAlbums")]
    fn request_albums(&mut self, artist: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let order = if self.album_order.is_empty() {
            crate::library::ALBUM_ORDERS[0].to_string()
        } else {
            self.album_order.clone()
        };
        crate::runtime().spawn(async move {
            match load_albums_json(&dbs, &artist, &order).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "emitData", "albums".to_string(), json);
                }
                Err(err) => eprintln!("library: could not read albums: {err:#}"),
            }
        });
    }

    /// Cycles the main-scope album order (Shift-L / Shift-H).
    #[qslot(qml_name = "albumOrderNext")]
    fn album_order_next(&mut self) {
        self.cycle_album_order(1);
    }

    #[qslot(qml_name = "albumOrderPrev")]
    fn album_order_prev(&mut self) {
        self.cycle_album_order(-1);
    }

    fn cycle_album_order(&mut self, delta: i64) {
        let current = if self.album_order.is_empty() {
            crate::library::ALBUM_ORDERS[0]
        } else {
            self.album_order.as_str()
        };
        let next = crate::library::cycle_album_order(current, delta);
        if self.album_order != next {
            self.album_order = next.to_string();
            self.album_order_changed();
        }
    }

    #[qslot(qml_name = "requestSongs")]
    fn request_songs(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match load_songs_json(&dbs).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "emitData", "songs".to_string(), json);
                }
                Err(err) => eprintln!("library: could not read songs: {err:#}"),
            }
        });
    }

    #[qslot(qml_name = "requestArtists")]
    fn request_artists(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match load_artists_json(&dbs).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "emitData", "artists".to_string(), json);
                }
                Err(err) => eprintln!("library: could not read artists: {err:#}"),
            }
        });
    }

    #[qslot(qml_name = "requestAlbumSongs")]
    fn request_album_songs(&mut self, album_id: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match load_album_songs_json(&dbs, &album_id).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "emitData", "album_songs".to_string(), json);
                }
                Err(err) => eprintln!("library: could not read album songs: {err:#}"),
            }
        });
    }

    /// Fetches an album's description ("notes") from the server.
    #[qslot(qml_name = "requestAlbumInfo")]
    fn request_album_info(&mut self, album_id: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            let notes = album_notes(&dbs, &album_id).await;
            let json = serde_json::json!({ "notes": notes }).to_string();
            qtbridge::invoke_method!(invoker, "emitData", "album_info".to_string(), json);
        });
    }

    /// Global library search (artists > albums > songs) for the search bar.
    #[qslot(qml_name = "requestSearch")]
    fn request_search(&mut self, query: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            let json = match search_json(&dbs, &query).await {
                Ok(json) => json,
                Err(err) => {
                    eprintln!("search: {err:#}");
                    "{\"artists\":[],\"albums\":[],\"songs\":[]}".to_string()
                }
            };
            qtbridge::invoke_method!(invoker, "emitData", "search".to_string(), json);
        });
    }

    // -----------------------------------------------------------------------
    // Playback
    // -----------------------------------------------------------------------

    /// Queues an album (from `library.db`) and starts playing at `start`.
    #[qslot(qml_name = "playAlbum")]
    fn play_album(&mut self, album_id: String, start: i64) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match album_tracks_json(&dbs, &album_id).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "playerLoad", json, start);
                }
                Err(err) => {
                    eprintln!("player: could not start playback: {err:#}");
                    qtbridge::invoke_method!(
                        invoker,
                        "raiseError",
                        "playback".to_string(),
                        format!("{err:#}")
                    );
                }
            }
        });
    }

    /// Queues a single song by id.
    #[qslot(qml_name = "playSong")]
    fn play_song(&mut self, song_id: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match song_queue_json(&dbs, &song_id).await {
                Ok((json, start)) => {
                    qtbridge::invoke_method!(invoker, "playerLoad", json, start);
                }
                Err(err) => {
                    eprintln!("player: could not play song: {err:#}");
                    qtbridge::invoke_method!(
                        invoker,
                        "raiseError",
                        "playback".to_string(),
                        format!("{err:#}")
                    );
                }
            }
        });
    }

    /// Resolves the song's album context and pumps a single `song_album` event
    /// (queue JSON, start index, album metadata). The UI then calls
    /// [`player_load`] and opens the album page itself, so no two crossed
    /// invocations are needed.
    ///
    /// Also reachable from the external IPC pump:
    /// `play_song_in_album <song-id>`.
    #[qslot(qml_name = "playSongInAlbum")]
    fn play_song_in_album(&mut self, song_id: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match song_album_payload(&dbs, &song_id).await {
                Ok(payload) => {
                    qtbridge::invoke_method!(
                        invoker,
                        "emitData",
                        "song_album".to_string(),
                        payload
                    );
                }
                Err(err) => {
                    eprintln!("player: could not queue album: {err:#}");
                    qtbridge::invoke_method!(
                        invoker,
                        "raiseError",
                        "playback".to_string(),
                        format!("{err:#}")
                    );
                }
            }
        });
    }

    /// Loads a serialized queue into the player (runs on the Qt thread).
    #[qslot(qml_name = "playerLoad")]
    fn player_load(&mut self, tracks_json: String, start: i64) {
        match serde_json::from_str::<Vec<crate::player::Track>>(&tracks_json) {
            Ok(tracks) => {
                self.player.load(tracks, start.max(0) as usize);
                self.sync_player();
                self.emit_queue();
                self.playback_started();
            }
            Err(err) => {
                eprintln!("player: bad queue payload: {err}");
                self.error_raised("playback".to_string(), format!("Bad queue payload: {err}"));
            }
        }
    }

    #[qslot(qml_name = "playerToggle")]
    fn player_toggle(&mut self) {
        self.player.toggle();
        self.sync_player();
    }

    #[qslot(qml_name = "playerNext")]
    fn player_next(&mut self) {
        self.player.next();
        self.sync_player();
        self.emit_queue();
        self.track_skipped(true);
    }

    #[qslot(qml_name = "playerPrevious")]
    fn player_previous(&mut self) {
        self.player.previous();
        self.sync_player();
        self.emit_queue();
        self.track_skipped(false);
    }

    #[qslot(qml_name = "playerStop")]
    fn player_stop(&mut self) {
        self.player.stop();
        self.sync_player();
    }

    /// Retries the current track after a playback error (UI-23).
    #[qslot(qml_name = "retryPlayback")]
    fn retry_playback(&mut self) {
        self.player.play_current();
        self.sync_player();
    }

    // -----------------------------------------------------------------------
    // Queue (drawer)
    // -----------------------------------------------------------------------

    /// Publishes the current queue to QML.
    fn emit_queue(&mut self) {
        let items: Vec<serde_json::Value> = self
            .player
            .queue()
            .iter()
            .map(|track| {
                serde_json::json!({
                    "id": track.id,
                    "title": track.title,
                    "artist": track.artist,
                    "album": track.album,
                    "duration": track.duration,
                    "cover": track.cover_url,
                })
            })
            .collect();
        let json = serde_json::json!({
            "index": self.player.index(),
            "items": items,
        })
        .to_string();
        self.data_ready("queue".to_string(), json);
    }

    #[qslot(qml_name = "requestQueue")]
    fn request_queue(&mut self) {
        self.emit_queue();
    }

    #[qslot(qml_name = "queueJump")]
    fn queue_jump(&mut self, index: i64) {
        self.player.jump(index.max(0) as usize);
        self.sync_player();
        self.emit_queue();
    }

    #[qslot(qml_name = "queueRemove")]
    fn queue_remove(&mut self, index: i64) {
        self.player.remove(index.max(0) as usize);
        self.sync_player();
        self.emit_queue();
    }

    #[qslot(qml_name = "queueMove")]
    fn queue_move(&mut self, from: i64, to: i64) {
        self.player
            .move_item(from.max(0) as usize, to.max(0) as usize);
        self.emit_queue();
    }

    /// Seeks to an absolute position (seconds).
    #[qslot(qml_name = "playerSeek")]
    fn player_seek(&mut self, seconds: f64) {
        self.player.seek(seconds);
        self.sync_player();
    }

    /// Sets the volume (0-100). Adjusting up unmutes.
    #[qslot(qml_name = "playerSetVolume")]
    fn player_set_volume(&mut self, volume: f64) {
        if volume > self.player.volume() && self.player.muted() {
            self.player.set_muted(false);
        }
        self.player.set_volume(volume);
        self.sync_player();
    }

    /// Steps the volume by `delta`; stepping up unmutes.
    #[qslot(qml_name = "playerAdjustVolume")]
    fn player_adjust_volume(&mut self, delta: f64) {
        let volume = self.player.volume() + delta;
        if delta > 0.0 && self.player.muted() {
            self.player.set_muted(false);
        }
        self.player.set_volume(volume);
        self.sync_player();
    }

    /// Mutes or unmutes (M).
    #[qslot(qml_name = "playerToggleMute")]
    fn player_toggle_mute(&mut self) {
        self.player.toggle_mute();
        self.sync_player();
    }

    /// Requests lyrics for the current track (override → cache → server).
    #[qslot(qml_name = "requestLyrics")]
    fn request_lyrics(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let Some(song_id) = self.player.current().map(|track| track.id.clone()) else {
            return;
        };
        // LYR-5: warm the cache for the next few queue tracks, bounded.
        let upcoming = self.player.upcoming(3);

        crate::runtime().spawn(async move {
            let client = match crate::sqlite::load_credentials(&dbs.credentials).await {
                Ok(Some(credentials)) => Some(SubsonicClient::new(
                    &credentials.server_url,
                    &credentials.username,
                    &credentials.password,
                )),
                Ok(None) => None,
                Err(err) => {
                    eprintln!("lyrics: could not load credentials: {err:#}");
                    None
                }
            };

            let document = match crate::lyrics::fetch(&dbs, client.as_ref(), &song_id).await {
                Ok(document) => document,
                Err(err) => {
                    eprintln!("lyrics: could not load lyrics for {song_id}: {err:#}");
                    crate::lyrics::Document::missing(&song_id)
                }
            };
            qtbridge::invoke_method!(
                invoker,
                "emitData",
                "lyrics".to_string(),
                document.to_json()
            );

            for id in upcoming {
                if let Err(err) = crate::lyrics::fetch(&dbs, client.as_ref(), &id).await {
                    eprintln!("lyrics: prefetch for {id} failed: {err:#}");
                }
            }
        });
    }

    /// Manual lyric refresh (`lyrics_refresh`): drops the current song's cache
    /// row and re-requests, so a server-side change shows up immediately.
    #[qslot(qml_name = "refreshLyrics")]
    fn refresh_lyrics(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let Some(song_id) = self.player.current().map(|track| track.id.clone()) else {
            return;
        };

        crate::runtime().spawn(async move {
            if let Err(err) = crate::lyrics::invalidate(&dbs.lyrics, &song_id).await {
                eprintln!("lyrics: could not invalidate {song_id}: {err:#}");
            }
            invoker.invoke_method("requestLyrics");
        });
    }

    /// Requests the cover-seeded palette for the current track (THM-3).
    ///
    /// One in-flight job per cover: repeated requests for the same art are
    /// coalesced, and the payload carries `cover_art` so the UI can drop a
    /// stale result after a skip.
    #[qslot(qml_name = "requestColors")]
    fn request_colors(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let cover = self.current_cover_art.clone();
        if cover.is_empty() {
            return;
        }
        if self.colors_in_flight.as_deref() == Some(cover.as_str()) {
            return;
        }
        self.colors_in_flight = Some(cover.clone());

        crate::runtime().spawn(async move {
            let cover_hash = crate::covers::hash(&dbs.covers, &cover)
                .await
                .ok()
                .flatten();
            let (scheme_name, seed, role_hash, dark) =
                match crate::colors::fetch(&dbs, &cover).await {
                    Ok(Some(scheme)) => (
                        serde_json::Value::String(scheme.scheme),
                        scheme.seed,
                        serde_json::Value::String(scheme.role_hash),
                        serde_json::to_value(&scheme.roles).unwrap_or(serde_json::Value::Null),
                    ),
                    Ok(None) => {
                        eprintln!("colors: no cached cover for {cover}; keeping static palette");
                        (
                            serde_json::Value::Null,
                            0,
                            serde_json::Value::Null,
                            serde_json::Value::Null,
                        )
                    }
                    Err(err) => {
                        eprintln!("colors: could not generate for {cover}: {err:#}");
                        (
                            serde_json::Value::Null,
                            0,
                            serde_json::Value::Null,
                            serde_json::Value::Null,
                        )
                    }
                };

            let payload = serde_json::json!({
                "cover_art": cover,
                "cover_hash": cover_hash,
                "scheme": scheme_name,
                "seed": seed,
                "role_hash": role_hash,
                "dark": dark,
            })
            .to_string();

            qtbridge::invoke_method!(invoker, "colorsReady", cover.clone());
            qtbridge::invoke_method!(invoker, "emitData", "colors".to_string(), payload);
        });
    }

    /// Clears the in-flight marker once a color job has been delivered.
    #[qslot(qml_name = "colorsReady")]
    fn colors_ready(&mut self, cover_art: String) {
        if self.colors_in_flight.as_deref() == Some(cover_art.as_str()) {
            self.colors_in_flight = None;
        }
    }

    /// Manual palette refresh: drops the cached scheme and regenerates it
    /// (`:theme refresh` / IPC `colors_refresh`) (THM-5).
    #[qslot(qml_name = "refreshColors")]
    fn refresh_colors(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let cover = self.current_cover_art.clone();
        if cover.is_empty() {
            return;
        }
        self.colors_in_flight = None;

        crate::runtime().spawn(async move {
            if let Err(err) = crate::colors::invalidate(&dbs, &cover).await {
                eprintln!("colors: could not invalidate {cover}: {err:#}");
            }
            invoker.invoke_method("requestColors");
        });
    }

    /// Lets the UI hand keyboard focus back to the global key handler (e.g.
    /// when leaving a text field), so vi bindings resume.
    #[qslot(qml_name = "releaseFocus")]
    fn release_focus(&mut self) {
        self.focus_main();
    }

    /// Periodic pump from the UI: updates position and advances on track end.
    #[qslot]
    fn tick(&mut self) {
        if self.player.state() == State::Stopped {
            return;
        }
        if self.player.poll_end() {
            self.player.next();
        }
        self.sync_player();
    }

    /// Publishes the player state to the QML properties.
    fn sync_player(&mut self) {
        // ARC-10: only emit when a value actually changed — the 500 ms tick
        // must not republish every string on every frame. Compare first (no
        // clones), clone only the fields that changed.
        let (
            present,
            id_changed,
            title_changed,
            artist_changed,
            album_changed,
            art_changed,
            url_changed,
        ) = match self.player.current() {
            Some(track) => (
                true,
                self.current_song_id != track.id,
                self.current_title != track.title,
                self.current_artist != track.artist,
                self.current_album != track.album,
                self.current_cover_art != track.cover_art,
                self.current_cover_url != track.cover_url,
            ),
            None => (false, false, false, false, false, false, false),
        };

        if !present {
            if !self.current_song_id.is_empty() {
                self.current_song_id.clear();
                self.current_song_id_changed();
            }
            if !self.current_title.is_empty() {
                self.current_title.clear();
                self.current_title_changed();
            }
            if !self.current_artist.is_empty() {
                self.current_artist.clear();
                self.current_artist_changed();
            }
            if !self.current_album.is_empty() {
                self.current_album.clear();
                self.current_album_changed();
            }
            if !self.current_cover_art.is_empty() {
                self.current_cover_art.clear();
                self.current_cover_art_changed();
            }
            if !self.current_cover_url.is_empty() {
                self.current_cover_url.clear();
                self.current_cover_url_changed();
            }
        } else {
            if id_changed {
                self.current_song_id = self
                    .player
                    .current()
                    .map(|track| track.id.clone())
                    .unwrap_or_default();
                self.current_song_id_changed();
            }
            if title_changed {
                self.current_title = self
                    .player
                    .current()
                    .map(|track| track.title.clone())
                    .unwrap_or_default();
                self.current_title_changed();
            }
            if artist_changed {
                self.current_artist = self
                    .player
                    .current()
                    .map(|track| track.artist.clone())
                    .unwrap_or_default();
                self.current_artist_changed();
            }
            if album_changed {
                self.current_album = self
                    .player
                    .current()
                    .map(|track| track.album.clone())
                    .unwrap_or_default();
                self.current_album_changed();
            }
            if art_changed {
                self.current_cover_art = self
                    .player
                    .current()
                    .map(|track| track.cover_art.clone())
                    .unwrap_or_default();
                self.current_cover_art_changed();
            }
            if url_changed {
                self.current_cover_url = self
                    .player
                    .current()
                    .map(|track| track.cover_url.clone())
                    .unwrap_or_default();
                self.current_cover_url_changed();
            }
        }

        let state = self.player.state().name().to_string();
        if self.playback_state != state {
            self.playback_state = state;
            self.playback_state_changed();
        }
        let duration = self.player.duration();
        if self.duration != duration {
            self.duration = duration;
            self.duration_changed();
        }
        let position = self.player.position();
        if self.position != position {
            self.position = position;
            self.position_changed();
        }
        let volume = self.player.volume();
        if self.volume != volume {
            self.volume = volume;
            self.volume_changed();
        }
        let muted = self.player.muted();
        if self.muted != muted {
            self.muted = muted;
            self.muted_changed();
        }
        let queue_index = self.player.index() as i64;
        if self.queue_index != queue_index {
            self.queue_index = queue_index;
            self.queue_index_changed();
        }
        let queue_length = self.player.len() as i64;
        if self.queue_length != queue_length {
            self.queue_length = queue_length;
            self.queue_length_changed();
        }
    }
}

/// Status-bar summary of a finished sync; lyrics only when any were fetched.
fn sync_summary(report: &crate::index::SyncReport) -> String {
    let mut summary = format!(
        "{} artists, {} albums, {} songs",
        report.artists, report.albums, report.songs
    );
    if report.lyrics > 0 {
        summary.push_str(&format!(", {} lyrics", report.lyrics));
    }
    summary
}

/// Kicks off a background index sync (used after a successful login).
///
/// Progress is reported back for the status bar; failures are only logged, as
/// the index is a cache and login has already succeeded.
fn spawn_index_sync(
    finish_invoker: QmlMethodInvoker,
    progress_invoker: QmlMethodInvoker,
    server_url: String,
    username: String,
    password: String,
) {
    let Some(dbs) = crate::dbs().cloned() else {
        eprintln!("index: databases are not initialised");
        return;
    };

    crate::runtime().spawn(async move {
        qtbridge::invoke_method!(progress_invoker, "librarySyncStarted");

        let client = Arc::new(SubsonicClient::new(&server_url, &username, &password));
        let progress = move |update: crate::index::SyncProgress| {
            qtbridge::invoke_method!(
                progress_invoker,
                "librarySyncProgress",
                update.message,
                update.done as i64,
                update.total as i64
            );
        };

        match crate::index::sync_library(&dbs, client, progress).await {
            Ok(report) => {
                eprintln!(
                    "index: library updated: {} artists, {} albums, {} songs, {} lyrics, {} new covers",
                    report.artists,
                    report.albums,
                    report.songs,
                    report.lyrics,
                    report.covers.downloaded
                );
                qtbridge::invoke_method!(
                    finish_invoker,
                    "librarySyncFinished",
                    sync_summary(&report)
                );
            }
            Err(err) => {
                eprintln!("index: library sync failed: {err:#}");
                qtbridge::invoke_method!(
                    finish_invoker,
                    "librarySyncFinished",
                    format!("Sync failed: {err:#}")
                );
                qtbridge::invoke_method!(
                    finish_invoker,
                    "raiseError",
                    "sync".to_string(),
                    format!("{err:#}")
                );
            }
        }
    });
}

/// Builds a playable [`crate::player::Track`] from an indexed song.
fn build_track(
    client: &SubsonicClient,
    song: crate::library::SongRow,
    covers: &HashMap<String, String>,
) -> anyhow::Result<crate::player::Track> {
    let url = client.stream_url(&song.id)?;
    let cover_art = song.cover_art.unwrap_or_default();
    let cover_url = covers.get(&cover_art).cloned().unwrap_or_default();
    Ok(crate::player::Track {
        id: song.id,
        title: song.title,
        artist: song.artist.unwrap_or_default(),
        album: song.album.unwrap_or_default(),
        duration: song.duration.unwrap_or(0) as f64,
        url: url.to_string(),
        cover_art,
        cover_url,
    })
}

// ---------------------------------------------------------------------------
// Off-thread list loaders (produce JSON with resolved cover URLs)
// ---------------------------------------------------------------------------

async fn load_albums_json(
    dbs: &crate::db::Dbs,
    artist: &str,
    order: &str,
) -> anyhow::Result<String> {
    let filter = (!artist.is_empty()).then_some(artist);
    let albums = crate::library::albums(&dbs.library, filter, order).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    let rows: Vec<serde_json::Value> = albums
        .into_iter()
        .map(|album| {
            let cover = album
                .cover_art
                .as_deref()
                .and_then(|id| covers.get(id))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "id": album.id,
                "title": album.title,
                "artist": album.artist,
                "year": album.year,
                "song_count": album.song_count,
                "cover": cover,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

async fn load_songs_json(dbs: &crate::db::Dbs) -> anyhow::Result<String> {
    let songs = crate::library::all_songs(&dbs.library).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    song_rows_json(&songs, &covers)
}

async fn load_album_songs_json(dbs: &crate::db::Dbs, album_id: &str) -> anyhow::Result<String> {
    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    song_rows_json(&songs, &covers)
}

async fn load_artists_json(dbs: &crate::db::Dbs) -> anyhow::Result<String> {
    let artists = crate::library::artists(&dbs.library).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;
    let rows: Vec<serde_json::Value> = artists
        .into_iter()
        .map(|artist| {
            let cover = artist
                .cover_art
                .as_deref()
                .and_then(|id| covers.get(id))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "id": artist.id,
                "name": artist.name,
                "album_count": artist.album_count,
                "cover": cover,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

fn song_rows_json(
    songs: &[crate::library::SongRow],
    covers: &HashMap<String, String>,
) -> anyhow::Result<String> {
    let rows: Vec<serde_json::Value> = songs
        .iter()
        .map(|song| {
            let cover = song
                .cover_art
                .as_deref()
                .and_then(|id| covers.get(id))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "id": song.id,
                "title": song.title,
                "artist": song.artist,
                "album": song.album,
                "track": song.track,
                "duration": song.duration,
                "cover": cover,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

async fn album_tracks_json(dbs: &crate::db::Dbs, album_id: &str) -> anyhow::Result<String> {
    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
    tracks_json(dbs, songs).await
}

/// Fetches an album's description text from the server (`getAlbumInfo2`).
async fn album_notes(dbs: &crate::db::Dbs, album_id: &str) -> String {
    let Ok(Some(credentials)) = crate::sqlite::load_credentials(&dbs.credentials).await else {
        return String::new();
    };
    let client = SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    );
    match client.get_album_info2(album_id).await {
        Ok(info) => info.notes.unwrap_or_default(),
        Err(err) => {
            eprintln!("album: could not fetch info for {album_id}: {err:#}");
            String::new()
        }
    }
}

fn cover_url(id: Option<&str>, covers: &HashMap<String, String>) -> String {
    id.and_then(|id| covers.get(id))
        .cloned()
        .unwrap_or_default()
}

async fn search_json(dbs: &crate::db::Dbs, query: &str) -> anyhow::Result<String> {
    if query.trim().is_empty() {
        return Ok("{\"artists\":[],\"albums\":[],\"songs\":[]}".to_string());
    }

    let (artists, albums) = crate::library::search_artists_albums(&dbs.library, query, 20).await?;
    let songs = crate::search::search(&dbs.search, query, 20).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;

    let song_ids: Vec<String> = songs.iter().map(|song| song.song_id.clone()).collect();
    let song_covers = crate::library::song_cover_ids(&dbs.library, &song_ids).await?;

    let artist_rows: Vec<serde_json::Value> = artists
        .into_iter()
        .map(|artist| {
            serde_json::json!({
                "id": artist.id,
                "name": artist.name,
                "album_count": artist.album_count,
                "cover": cover_url(artist.cover_art.as_deref(), &covers),
            })
        })
        .collect();
    let album_rows: Vec<serde_json::Value> = albums
        .into_iter()
        .map(|album| {
            serde_json::json!({
                "id": album.id,
                "title": album.title,
                "artist": album.artist,
                "year": album.year,
                "song_count": album.song_count,
                "cover": cover_url(album.cover_art.as_deref(), &covers),
            })
        })
        .collect();
    let song_rows: Vec<serde_json::Value> = songs
        .iter()
        .map(|song| {
            let cover = song_covers
                .get(&song.song_id)
                .and_then(|cover_art| covers.get(cover_art))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({
                "id": song.song_id,
                "title": song.title,
                "artist": song.artist,
                "album": song.album,
                "duration": song.duration,
                "cover": cover,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "artists": artist_rows,
        "albums": album_rows,
        "songs": song_rows,
    })
    .to_string())
}

/// Builds the queue for a song: its album (starting at that song) when known,
/// otherwise just the song itself.
async fn song_queue_json(dbs: &crate::db::Dbs, song_id: &str) -> anyhow::Result<(String, i64)> {
    let Some(song) = crate::library::song(&dbs.library, song_id).await? else {
        return Err(anyhow::anyhow!("song not found in index"));
    };
    let (songs, start) = match song.album_id.as_deref() {
        Some(album_id) if !album_id.is_empty() => {
            let album = crate::library::album_songs(&dbs.library, album_id).await?;
            let index = album.iter().position(|s| s.id == song.id).unwrap_or(0) as i64;
            (album, index)
        }
        _ => (vec![song], 0),
    };
    Ok((tracks_json(dbs, songs).await?, start))
}

/// Single pumped event for "play this song inside its album": the serialized
/// queue, the start index and the album metadata (or null).
async fn song_album_payload(dbs: &crate::db::Dbs, song_id: &str) -> anyhow::Result<String> {
    let Some(song) = crate::library::song(&dbs.library, song_id).await? else {
        return Err(anyhow::anyhow!("song not found in index"));
    };
    let Some(album_id) = song.album_id.as_deref().filter(|id| !id.is_empty()) else {
        let queue = tracks_json(dbs, vec![song]).await?;
        return Ok(serde_json::json!({
            "queue": queue,
            "start": 0,
            "album": serde_json::Value::Null,
        })
        .to_string());
    };

    let songs = crate::library::album_songs(&dbs.library, album_id).await?;
    let start = songs.iter().position(|s| s.id == song.id).unwrap_or(0) as i64;
    let queue = tracks_json(dbs, songs).await?;

    let album = match crate::library::album(&dbs.library, album_id).await? {
        Some(album) => {
            let covers = crate::covers::url_map(&dbs.covers).await?;
            serde_json::json!({
                "id": album.id,
                "title": album.title,
                "artist": album.artist,
                "year": album.year,
                "tracks": album.song_count,
                "cover": cover_url(album.cover_art.as_deref(), &covers),
            })
        }
        None => serde_json::Value::Null,
    };

    Ok(serde_json::json!({
        "queue": queue,
        "start": start,
        "album": album,
    })
    .to_string())
}

async fn tracks_json(
    dbs: &crate::db::Dbs,
    songs: Vec<crate::library::SongRow>,
) -> anyhow::Result<String> {
    let credentials = crate::sqlite::load_credentials(&dbs.credentials).await?;
    let Some(credentials) = credentials else {
        return Err(anyhow::anyhow!("no saved credentials to stream with"));
    };
    let client = SubsonicClient::new(
        &credentials.server_url,
        &credentials.username,
        &credentials.password,
    );
    let covers = crate::covers::url_map(&dbs.covers).await?;

    let mut tracks = Vec::with_capacity(songs.len());
    for song in songs {
        tracks.push(build_track(&client, song, &covers)?);
    }
    Ok(serde_json::to_string(&tracks)?)
}
