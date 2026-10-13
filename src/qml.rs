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
    current_title: String,
    current_artist: String,
    current_album: String,
    current_cover_art: String,
    current_cover_url: String,
    playback_state: String,
    position: f64,
    duration: f64,
    queue_index: i64,
    queue_length: i64,
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
    qproperty!("albumCount", Member = album_count, Notify = album_count_changed);
    qproperty!("songCount", Member = song_count, Notify = song_count_changed);
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
        "playbackState",
        Member = playback_state,
        Notify = playback_state_changed
    );
    qproperty!("position", Member = position, Notify = position_changed);
    qproperty!("duration", Member = duration, Notify = duration_changed);
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
    #[qsignal(qml_name = "dataReady")]
    fn data_ready(&mut self, channel: String, payload: String);
    #[qsignal(qml_name = "errorRaised")]
    fn error_raised(&mut self, kind: String, message: String);
    #[qsignal]
    fn playback_state_changed(&mut self);
    #[qsignal]
    fn position_changed(&mut self);
    #[qsignal]
    fn duration_changed(&mut self);
    #[qsignal]
    fn queue_index_changed(&mut self);
    #[qsignal]
    fn queue_length_changed(&mut self);
    #[qsignal]
    fn library_changed(&mut self);
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
                Ok(()) => {
                    if let Some(pool) = pool.as_ref() {
                        if let Err(err) =
                            crate::sqlite::save_credentials(pool, &username, &password, &server_url)
                                .await
                        {
                            eprintln!("Failed to persist credentials: {err:#}");
                        }
                    }
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
        let invoker = self.get_qml_method_invoker();
        let sync_finish = self.get_qml_method_invoker();
        let sync_progress = self.get_qml_method_invoker();
        let Some(pool) = crate::pool().cloned() else {
            return;
        };

        crate::runtime().spawn(async move {
            match crate::sqlite::load_credentials(&pool).await {
                Ok(Some(credentials)) => {
                    if crate::http_client::authenticate(
                        &credentials.server_url,
                        &credentials.username,
                        &credentials.password,
                    )
                    .await
                    .is_ok()
                    {
                        invoker.invoke_method("login_succeeded");
                        spawn_index_sync(
                            sync_finish,
                            sync_progress,
                            credentials.server_url,
                            credentials.username,
                            credentials.password,
                        );
                    }
                }
                Ok(None) => {}
                Err(err) => eprintln!("Failed to load saved credentials: {err:#}"),
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

        match self.keymap.feed(&token) {
            Outcome::Action(action) => action.name().to_string(),
            Outcome::Pending => "PENDING".to_string(),
            Outcome::None => String::new(),
        }
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
                        format!(
                            "{} artists, {} albums, {} songs",
                            report.artists, report.albums, report.songs
                        )
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
                },
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

    #[qslot(qml_name = "requestAlbums")]
    fn request_albums(&mut self, artist: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            match load_albums_json(&dbs, &artist).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "emitData", "albums".to_string(), json);
                }
                Err(err) => eprintln!("library: could not read albums: {err:#}"),
            }
        });
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

    /// Floating palette search over the FTS5 index in `search.db`.
    #[qslot(qml_name = "requestPalette")]
    fn request_palette(&mut self, query: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };

        crate::runtime().spawn(async move {
            match crate::search::search_palette(&dbs.search, &query, 50).await {
                Ok(hits) => {
                    let json = serde_json::to_string(&hits).unwrap_or_else(|_| "[]".to_string());
                    qtbridge::invoke_method!(invoker, "emitData", "palette".to_string(), json);
                }
                Err(err) => eprintln!("palette: search failed: {err:#}"),
            }
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
                    qtbridge::invoke_method!(invoker, "raiseError", "playback".to_string(), format!("{err:#}"));
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
                    qtbridge::invoke_method!(invoker, "raiseError", "playback".to_string(), format!("{err:#}"));
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
    }

    #[qslot(qml_name = "playerPrevious")]
    fn player_previous(&mut self) {
        self.player.previous();
        self.sync_player();
    }

    #[qslot(qml_name = "playerStop")]
    fn player_stop(&mut self) {
        self.player.stop();
        self.sync_player();
    }

    /// Seeks to an absolute position (seconds).
    #[qslot(qml_name = "playerSeek")]
    fn player_seek(&mut self, seconds: f64) {
        self.player.seek(seconds);
        self.sync_player();
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
        let (title, artist, album, cover_art, cover_url) = match self.player.current() {
            Some(track) => (
                track.title.clone(),
                track.artist.clone(),
                track.album.clone(),
                track.cover_art.clone(),
                track.cover_url.clone(),
            ),
            None => (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        };
        self.current_title = title;
        self.current_title_changed();
        self.current_artist = artist;
        self.current_artist_changed();
        self.current_album = album;
        self.current_album_changed();
        self.current_cover_art = cover_art;
        self.current_cover_art_changed();
        self.current_cover_url = cover_url;
        self.current_cover_url_changed();

        self.playback_state = self.player.state().name().to_string();
        self.playback_state_changed();
        self.duration = self.player.duration();
        self.duration_changed();
        self.position = self.player.position();
        self.position_changed();
        self.queue_index = self.player.index() as i64;
        self.queue_index_changed();
        self.queue_length = self.player.len() as i64;
        self.queue_length_changed();
    }
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
                    "index: library updated: {} artists, {} albums, {} songs, {} new covers",
                    report.artists, report.albums, report.songs, report.covers.downloaded
                );
                qtbridge::invoke_method!(
                    finish_invoker,
                    "librarySyncFinished",
                    format!(
                        "{} artists, {} albums, {} songs",
                        report.artists, report.albums, report.songs
                    )
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

async fn load_albums_json(dbs: &crate::db::Dbs, artist: &str) -> anyhow::Result<String> {
    let filter = (!artist.is_empty()).then_some(artist);
    let albums = crate::library::albums(&dbs.library, filter).await?;
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
) -> anyhow::Result<String> {    let rows: Vec<serde_json::Value> = songs
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
    id.and_then(|id| covers.get(id)).cloned().unwrap_or_default()
}

async fn search_json(dbs: &crate::db::Dbs, query: &str) -> anyhow::Result<String> {
    if query.trim().is_empty() {
        return Ok("{\"artists\":[],\"albums\":[],\"songs\":[]}".to_string());
    }

    let (artists, albums, songs) = crate::library::search(&dbs.library, query, 20).await?;
    let covers = crate::covers::url_map(&dbs.covers).await?;

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
            serde_json::json!({
                "id": song.id,
                "title": song.title,
                "artist": song.artist,
                "album": song.album,
                "duration": song.duration,
                "cover": cover_url(song.cover_art.as_deref(), &covers),
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
