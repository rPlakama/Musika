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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use qtbridge::{QmlObject, qobject};

use crate::browse::{
    album_notes, load_album_songs_json, load_albums_json, load_artist_json, load_artists_json,
    load_songs_page_json, search_json,
};
use crate::http_client::SubsonicClient;
use crate::keybind::{Keymap, Outcome};
use crate::player::{Player, State};
use crate::queue::{album_tracks_json, library_queue_payload, song_album_payload};

#[path = "qml_backend/artist.rs"]
mod artist;
#[path = "qml_backend/download.rs"]
mod download;
#[path = "qml_backend/support.rs"]
mod support;
#[path = "qml_backend/sync.rs"]
mod sync;

use artist::{ArtistUserAction, apply_artist_user};
use download::{
    album_container_off, album_container_on, apply_window, artist_covers_album, completed_album,
    download_one, downloaded_music_available, fetch_marked, rule_off, selection_toggle,
};
pub(crate) use support::client;
use support::{album_cache_payload, album_state_payload, artist_cache_payload, emit, raise};
use sync::{index_value, online_authenticated, spawn_sync};

/// Whether the server is reachable for on-demand work outside the sync (cover
/// misses): offline logins and the pre-login state keep it false.
static ONLINE: AtomicBool = AtomicBool::new(false);

pub(crate) fn online() -> bool {
    ONLINE.load(Ordering::Relaxed)
}

/// Bumped by every status change; the debounced writer only persists when its
/// captured generation is still the latest (P-6 / roadmap "Session status").
static STATUS_WRITE_GEN: AtomicU64 = AtomicU64::new(0);

/// Bumped by every queue emit; the async ledger lookup only delivers when its
/// captured generation is still the latest (out-of-order emits would flip the
/// queue back to a stale index).
static QUEUE_EMIT_GEN: AtomicU64 = AtomicU64::new(0);

/// Bumped by every mark-window request; the debounced reader only applies
/// when its generation is still the latest (fast scrolling coalesces into
/// the final cursor window instead of stacking DB tasks).
static MARK_WINDOW_GEN: AtomicU64 = AtomicU64::new(0);

/// Bumped by every search request: a completed read only reaches the box if
/// no newer keystroke owns it, and a superseded task skips its DB work.
static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);

/// Bumped by every scroll-memory flush; an older async write must not land
/// after a newer one.
static SCROLL_WRITE_GEN: AtomicU64 = AtomicU64::new(0);

/// The union of ids requested since the last applied read: a cursor sweep
/// and every visible surface coalesce into one query.
#[derive(Default)]
struct MarkPending {
    songs: std::collections::HashSet<String>,
    albums: std::collections::HashSet<String>,
}

/// One id's download mark (the `mark_states` map's values).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkState {
    /// The song's blob is in the ledger.
    SongDone,
    /// Every track of the album is in the ledger.
    AlbumCached,
    /// The song's fetch is queued/active.
    SongInFlight,
}

fn remove_mark_id(ids: &mut Vec<String>, id: &str) -> bool {
    if let Some(position) = ids.iter().position(|item| item == id) {
        ids.remove(position);
        true
    } else {
        false
    }
}

fn add_mark_id(ids: &mut Vec<String>, id: &str) -> bool {
    if ids.iter().any(|item| item == id) {
        false
    } else {
        ids.push(id.to_string());
        true
    }
}

/// The hash of the last published queue rows ("RowAlreadyPublished"): the
/// automatic emits dedupe on it so skips don't republish the same rows.
static QUEUE_ROWS_HASH: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub struct Backend {
    server_url: String,
    username: String,
    password: String,
    status_message: String,
    busy: bool,
    logged_in: bool,
    /// CACHE-9: true while the app runs without the server, serving only the
    /// downloaded catalog.
    offline: bool,
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
    /// The playing track's album id — the fullscreen favourite/download
    /// buttons act in album scope.
    current_album_id: String,
    /// The finished-song ids of the *last requested window* (the visible
    /// rows around a cursor), read from download.db. Every row shows its
    /// download glyph iff its song id is in here — the whole ledger is never
    /// loaded or preloaded.
    downloaded_ids: Vec<String>,
    /// The window's album ids whose *every* track is finished — album rows
    /// wear the same glyph through this set.
    cached_album_ids: Vec<String>,
    /// The window's in-flight song ids (queued/active): the row marks show
    /// the loading glyph, spinning, for these.
    downloading_ids: Vec<String>,
    /// `WhileDownloadRequest`: any download request is in flight (it stays
    /// true for a minimum hold so the loading glyph is actually seen, even
    /// when a cached fetch settles instantly).
    while_download_request: bool,
    /// The hold's deadline for the running request burst.
    download_request_until: Option<std::time::Instant>,
    /// The mark truth: id -> state (done song / cached album / in-flight
    /// song). Window reads and per-id patches both write it, and the
    /// exposed sets re-derive from it — nothing can clobber anything (a
    /// favourite's reload used to hide a finished download exactly that
    /// way).
    mark_states: std::collections::HashMap<String, MarkState>,
    /// The ids the UI last asked about (the mark window).
    download_window_songs: Vec<String>,
    download_window_albums: Vec<String>,
    /// The recent window *requests*, newest last, capped. A download's
    /// refresh re-reads their union — the last request alone may carry no
    /// albums (a songs surface), which is why a finished album never
    /// re-evaluated until a reload re-requested its row's window.
    recent_mark_requests: std::collections::VecDeque<(Vec<String>, Vec<String>)>,
    /// Pending mark ids waiting on the debounced reader (PATH-5).
    mark_pending: std::sync::Arc<std::sync::Mutex<MarkPending>>,
    playback_state: String,
    position: f64,
    duration: f64,
    volume: f64,
    muted: bool,
    queue_index: i64,
    /// The queue rows' player revision at the last publish (QUEUEROWSTUDY
    /// QR-1): a non-forced emit whose revision matches returns before the
    /// O(N) clone/DB/serialize work — a skip or an advance moved only the
    /// index, which the `queueIndex` property carries.
    queue_revision: Option<u64>,
    /// A row change outside the player (library syncs, wipe reloads) must
    /// publish even when the revision matches; `TriggerRowReload` raises it,
    /// `emit_queue` consumes it. Favourites no longer ride this — they land
    /// per row (`favoriteChanged`) and never touch the queue payload.
    queue_rows_dirty: bool,
    /// The shuffle's generation: tasks captured before a toggle must not
    /// write into the new world.
    shuffle_gen: u64,
    /// The chip's state: on/off and which source is shuffled.
    shuffling: bool,
    shuffle_kind: String,
    queue_length: i64,
    album_order: String,
    song_order: String,
    artist_order: String,
    server_type: String,
    search_open: bool,
    /// ARC-UI-2 M0: the `/` line prompt is open (see `key_event`).
    line_search_open: bool,
    /// ARC-43: bare Alt held, reported by the window key handler.
    alt_held: bool,
    /// ARC-43: a half-typed keyboard sequence disarms the overlay.
    key_pending: bool,
    /// VI-7: the count register — mid-typing (`3` before `j`) it is the
    /// number being built (the chip's), at fire time it is the count the
    /// just-dispatched action carries (0 = none).
    key_count: i64,
    /// Last album page / song the session should return to (P-6).
    status_album: Option<String>,
    status_song: Option<String>,
    /// S3: playhead to apply once `Home` loads the restored queue; consumed
    /// by `playerLoadPaused` so a later manual load never seeks.
    status_restore_position: Option<f64>,
    /// S3: throttle for the periodic position write (10 s) while playing.
    last_position_save: Option<std::time::Instant>,
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
    qproperty!("offline", Member = offline, Notify = offline_changed);
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
        "currentAlbumId",
        Member = current_album_id,
        Notify = current_album_id_changed
    );
    qproperty!(
        "downloadedIds",
        Member = downloaded_ids,
        Notify = downloaded_ids_changed
    );
    qproperty!(
        "cachedAlbumIds",
        Member = cached_album_ids,
        Notify = cached_album_ids_changed
    );
    qproperty!(
        "downloadingIds",
        Member = downloading_ids,
        Notify = downloading_ids_changed
    );
    qproperty!(
        "whileDownloadRequest",
        Member = while_download_request,
        Notify = while_download_request_changed
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
        "songOrder",
        Member = song_order,
        Notify = song_order_changed
    );
    qproperty!(
        "artistOrder",
        Member = artist_order,
        Notify = artist_order_changed
    );
    qproperty!(
        "serverType",
        Member = server_type,
        Notify = server_type_changed
    );
    qproperty!("altHeld", Member = alt_held, Notify = alt_held_changed);
    qproperty!(
        "keyPending",
        Member = key_pending,
        Notify = key_pending_changed
    );
    qproperty!("keyCount", Member = key_count, Notify = key_count_changed);
    qproperty!("shuffling", Member = shuffling, Notify = shuffling_changed);
    qproperty!(
        "shuffleKind",
        Member = shuffle_kind,
        Notify = shuffle_kind_changed
    );
    qproperty!(
        "searchOpen",
        Member = search_open,
        Notify = search_open_changed
    );
    qproperty!(
        "lineSearchOpen",
        Member = line_search_open,
        Notify = line_search_open_changed
    );

    fn spawn_task<F, Fut>(&self, task: F)
    where
        F: FnOnce(crate::db::Dbs) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(task(dbs));
    }

    fn spawn_data<F, Fut>(&self, channel: &'static str, load: F)
    where
        F: FnOnce(crate::db::Dbs) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<String>> + Send,
    {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            match load(dbs).await {
                Ok(json) => emit(&invoker, channel, json),
                Err(err) => eprintln!("qml: {channel} load failed: {err:#}"),
            }
        });
    }

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
    fn offline_changed(&mut self);
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
    #[qsignal]
    fn current_album_id_changed(&mut self);
    #[qsignal]
    fn downloaded_ids_changed(&mut self);
    #[qsignal]
    fn cached_album_ids_changed(&mut self);
    #[qsignal]
    fn downloading_ids_changed(&mut self);
    #[qsignal]
    fn while_download_request_changed(&mut self);
    /// A fetch/reload changed the row state: every visible surface re-reads
    /// its own current mark window (the request half of the row update).
    #[qsignal(qml_name = "rowReload")]
    fn row_reload(&mut self);
    /// A favourite landed: surfaces patch the matching rows in place — the
    /// per-row twin of `rowReload`. A queue payload here would rebuild the
    /// models and re-follow the playing index (the cursor must not move for
    /// a heart write).
    #[qsignal(qml_name = "favoriteChanged")]
    fn favorite_changed(&mut self, kind: String, id: String, on: bool);
    /// UI-60: a rating landed (or failed and reverted): surfaces patch the
    /// matching row in place — the rating twin of `favoriteChanged`.
    #[qsignal(qml_name = "ratingChanged")]
    fn rating_changed(&mut self, kind: String, id: String, rating: i64);
    #[qsignal(qml_name = "dataReady")]
    fn data_ready(&mut self, channel: String, payload: String);
    #[qsignal(qml_name = "errorRaised")]
    fn error_raised(&mut self, kind: String, message: String);
    #[qsignal(qml_name = "commandIssued")]
    fn command_issued(&mut self, command: String);
    #[qsignal(qml_name = "loginRequired")]
    fn login_required(&mut self);
    #[qsignal]
    fn playback_state_changed(&mut self);
    /// The playhead jumped (a seek, not playback progression): the lyrics
    /// follower drops its manual-scroll hold and re-centres at once (LYR-8).
    #[qsignal]
    fn seeked(&mut self);
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
    fn song_order_changed(&mut self);
    #[qsignal]
    fn artist_order_changed(&mut self);
    #[qsignal]
    fn server_type_changed(&mut self);
    #[qsignal]
    fn alt_held_changed(&mut self);
    #[qsignal]
    fn key_pending_changed(&mut self);
    #[qsignal]
    fn key_count_changed(&mut self);
    #[qsignal]
    fn shuffling_changed(&mut self);
    #[qsignal]
    fn shuffle_kind_changed(&mut self);
    #[qsignal]
    fn search_open_changed(&mut self);
    #[qsignal]
    fn line_search_open_changed(&mut self);
    #[qsignal]
    fn library_changed(&mut self);
    /// P-6 S2: restored session payload `{album, queue, start}` for `Home`.
    #[qsignal(qml_name = "statusRestored")]
    fn status_restored(&mut self, payload: String);
    /// User skip (transport/keyboard/IPC), so the skip buttons can bloom.
    #[qsignal(qml_name = "trackSkipped")]
    fn track_skipped(&mut self, forward: bool);
    /// Play/pause toggled, from any source (keys, mouse, IPC, backend).
    #[qsignal(qml_name = "playToggled")]
    fn play_toggled(&mut self);
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

    #[qslot]
    fn startup(&mut self) {
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
        let Some(pool) = crate::pool().cloned() else {
            return;
        };

        // Auto-login with the ping: the stored account always tries to
        // complete the login on its own. With downloaded music the app opens
        // right away as `offline` (filtered to the downloaded catalog) while
        // the auth finishes in the background — on success the flag clears
        // and the full streaming library loads. Without downloads it waits
        // for the same ping and only returns to the Login form, prefilled,
        // when the server is unreachable.
        crate::runtime().spawn(async move {
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

    /// Login-only fallback: fills the form from the stored account.
    #[qslot(qml_name = "prefillCredentials")]
    fn prefill_credentials(&mut self, server_url: String, username: String, password: String) {
        self.remember_account(server_url, username);
        if self.password != password {
            self.password = password;
            self.password_changed();
        }
    }

    /// Records the account the session belongs to (the status row is
    /// account-scoped, P-6).
    #[qslot(qml_name = "rememberAccount")]
    fn remember_account(&mut self, server_url: String, username: String) {
        if self.server_url != server_url {
            self.server_url = server_url;
            self.server_url_changed();
        }
        if self.username != username {
            self.username = username;
            self.username_changed();
        }
    }

    /// CACHE-9: enters the app without the server when downloaded music
    /// exists; every list loader then serves only the downloaded catalog.
    #[qslot(qml_name = "offlineLogin")]
    fn offline_login(&mut self, server_url: String, username: String) {
        self.remember_account(server_url, username);
        ONLINE.store(false, Ordering::Relaxed);
        self.offline = true;
        self.offline_changed();
        crate::debug_log!("login: offline mode — showing downloaded music only");
        self.login_succeeded();
        self.status_message = "Offline — showing downloaded music only.".to_string();
        self.status_message_changed();
    }

    /// CACHE-9: the server answered again; the full streaming library returns
    /// at once (the catalog was always local) and the background sync
    /// refreshes it right after.
    #[qslot(qml_name = "noteOnline")]
    fn note_online(&mut self) {
        ONLINE.store(true, Ordering::Relaxed);
        if self.offline {
            self.offline = false;
            self.offline_changed();
            self.library_changed();
        }
    }

    #[qslot]
    fn login_succeeded(&mut self) {
        self.busy = false;
        self.busy_changed();
        self.logged_in = true;
        self.logged_in_changed();
        self.status_message = "Logged in.".to_string();
        self.status_message_changed();
        self.restore_status();
    }

    /// S1 restore: volume/mute only. Album/queue navigation and the paused
    /// queue are S2; transient UI is never restored.
    fn restore_status(&mut self) {
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let server_url = self.server_url.trim().to_string();
        let username = self.username.trim().to_string();
        let offline = self.offline;
        let invoker = self.get_qml_method_invoker();
        crate::runtime().spawn(async move {
            match crate::status::load(&dbs.status).await {
                Ok(Some(status))
                    if status.server_url == server_url && status.username == username =>
                {
                    let Ok(payload) = serde_json::to_string(&status) else {
                        return;
                    };
                    qtbridge::invoke_method!(invoker, "applyStatus", payload);
                    // S2: navigation + paused queue, validated against the
                    // local index (unknown ids are dropped in the payload).
                    match crate::queue::resume_payload(&dbs, &status, offline).await {
                        Ok(resume) => {
                            qtbridge::invoke_method!(invoker, "statusRestored", resume);
                        }
                        Err(err) => {
                            eprintln!("status: could not build the resume payload: {err:#}");
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => eprintln!("status: could not read the session: {err:#}"),
            }
        });
    }

    /// Applies a restored [`crate::status::SessionStatus`] payload.
    #[qslot(qml_name = "applyStatus")]
    fn apply_status(&mut self, payload: String) {
        let Ok(status) = serde_json::from_str::<crate::status::SessionStatus>(&payload) else {
            eprintln!("status: ignoring malformed session payload");
            return;
        };
        self.player.set_volume(status.volume);
        self.player.set_muted(status.muted);
        self.status_album = status.album_id;
        self.status_song = status.song_id;
        // S3: armed here, consumed by `playerLoadPaused` when Home loads the
        // restored queue; a queue-less restore leaves it harmlessly set.
        self.status_restore_position = (status.position > 0.0).then_some(status.position);
        self.sync_player();
    }

    /// Remembers the album page to reopen on the next launch (P-6 S2).
    #[qslot(qml_name = "rememberAlbum")]
    fn remember_album(&mut self, album_id: String) {
        if album_id.is_empty() || self.status_album.as_deref() == Some(album_id.as_str()) {
            return;
        }
        self.status_album = Some(album_id);
        self.queue_status_save();
    }

    /// Remembers the song to queue (paused) on the next launch (P-6 S2).
    fn remember_song(&mut self, song_id: &str) {
        if song_id.is_empty() || self.status_song.as_deref() == Some(song_id) {
            return;
        }
        self.status_song = Some(song_id.to_string());
        self.queue_status_save();
    }

    fn session_status(&self, queue: String) -> crate::status::SessionStatus {
        crate::status::SessionStatus {
            server_url: self.server_url.trim().to_string(),
            username: self.username.trim().to_string(),
            album_id: self.status_album.clone(),
            song_id: self.status_song.clone(),
            queue,
            queue_index: self.player.index() as i64,
            position: self.player.position(),
            volume: self.player.volume(),
            muted: self.player.muted(),
            updated_at: crate::consts::now_secs(),
        }
    }

    /// Arms the 500 ms session debounce: volume/mute drags coalesce into one
    /// write; a row from another account is discarded first (P-6 / S1).
    fn queue_status_save(&mut self) {
        if !self.logged_in || self.server_url.trim().is_empty() || self.username.trim().is_empty() {
            return;
        }
        let generation = STATUS_WRITE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        // PATH-7: clone the ids here, but serialize the full queue in the
        // worker after the debounce wins — never on the Qt thread.
        let ids: Vec<String> = self
            .player
            .queue()
            .iter()
            .map(|track| track.id.clone())
            .collect();
        let status = self.session_status(String::new());
        crate::runtime().spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if STATUS_WRITE_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            let mut status = status;
            status.queue = serde_json::to_string(&ids).unwrap_or_default();
            if let Err(err) = crate::status::save(&dbs.status, &status).await {
                eprintln!("status: could not save the session: {err:#}");
            }
        });
    }

    /// The player queue as a JSON id array (P-6 S3 snapshot).
    fn queue_ids_json(&self) -> String {
        let ids: Vec<&str> = self
            .player
            .queue()
            .iter()
            .map(|track| track.id.as_str())
            .collect();
        serde_json::to_string(&ids).unwrap_or_default()
    }

    /// Last-chance write on window close (P-6): the debounce task cannot be
    /// awaited during teardown, so this single local upsert blocks the Qt
    /// thread for the few ms it takes.
    #[qslot(qml_name = "flushStatus")]
    fn flush_status(&mut self) {
        if !self.logged_in || self.server_url.trim().is_empty() || self.username.trim().is_empty() {
            return;
        }
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        // Cancel a pending debounce so it cannot race (or overwrite) this.
        STATUS_WRITE_GEN.fetch_add(1, Ordering::Relaxed);
        let status = self.session_status(self.queue_ids_json());
        if let Err(err) = crate::runtime().block_on(crate::status::save(&dbs.status, &status)) {
            eprintln!("status: could not flush the session: {err:#}");
        }
    }

    /// P-6 S3: while playing, feed the playhead into the debounced writer
    /// every 10 s. Position only matters mid-playback, so it can ride the UI
    /// tick; all other status changes go through `queue_status_save` directly
    /// (the tick returns early while stopped).
    fn maybe_save_position(&mut self) {
        if self.player.state() != State::Playing || self.player.position() <= 0.0 {
            return;
        }
        let due = self.last_position_save.map_or(true, |at| {
            at.elapsed() >= std::time::Duration::from_secs(10)
        });
        if due {
            self.last_position_save = Some(std::time::Instant::now());
            self.queue_status_save();
        }
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
            self.mark_key_pending(false);
            self.set_key_count(0);
            return String::new();
        };

        // ARC-UI-2 M0: while the `/` prompt is up the keymap is dead — the
        // field owns every key (Esc/Enter stop at its own handlers, Tab and
        // Ctrl-W are accepted there, everything else types).
        if self.line_search_open {
            self.keymap.reset();
            self.mark_key_pending(false);
            self.set_key_count(0);
            return String::new();
        }

        match self.keymap.feed(&token) {
            Outcome::Action(action) => {
                self.mark_key_pending(false);
                // VI-7: the count rides the dispatch (`3j` → move_down ×3).
                let count = self.keymap.take_count() as i64;
                self.set_key_count(count);
                action.name().to_string()
            }
            Outcome::Pending => {
                self.mark_key_pending(true);
                // The chip shows the count being built (0 for plain
                // sequences like `g`).
                self.set_key_count(self.keymap.count() as i64);
                "PENDING".to_string()
            }
            Outcome::None => {
                self.mark_key_pending(false);
                self.set_key_count(0);
                String::new()
            }
        }
    }

    /// Marks the floating search as open/closed (see `key_event`).
    #[qslot(qml_name = "setSearchOpen")]
    fn set_search_open(&mut self, open: bool) {
        // Leaving the previous context may have left a half-typed sequence.
        self.keymap.reset();
        self.mark_key_pending(false);
        if self.search_open != open {
            self.search_open = open;
            self.search_open_changed();
        }
    }

    /// ARC-UI-2 M0: marks the `/` line prompt as open/closed (see
    /// `key_event`).
    #[qslot(qml_name = "setLineSearchOpen")]
    fn set_line_search_open(&mut self, open: bool) {
        self.keymap.reset();
        self.mark_key_pending(false);
        if self.line_search_open != open {
            self.line_search_open = open;
            self.line_search_open_changed();
        }
    }

    /// ARC-43: the window reports the bare Alt state; the overlay arms only
    /// while it is held.
    #[qslot(qml_name = "setAltHeld")]
    fn set_alt_held(&mut self, held: bool) {
        if self.alt_held != held {
            self.alt_held = held;
            self.alt_held_changed();
        }
    }

    /// Resolves a deferred key sequence after the inter-key timeout.
    #[qslot(qml_name = "keyTimeout")]
    fn key_timeout(&mut self) -> String {
        self.mark_key_pending(false);
        match self.keymap.feed_timeout() {
            Outcome::Action(action) => {
                let count = self.keymap.take_count() as i64;
                self.set_key_count(count);
                action.name().to_string()
            }
            Outcome::Pending | Outcome::None => {
                self.set_key_count(0);
                String::new()
            }
        }
    }

    /// ARC-43: pending-token bookkeeping for the quick-action overlay.
    fn mark_key_pending(&mut self, pending: bool) {
        if self.key_pending != pending {
            self.key_pending = pending;
            self.key_pending_changed();
        }
    }

    /// VI-7: the count register's QML face (0 = none).
    fn set_key_count(&mut self, count: i64) {
        if self.key_count != count {
            self.key_count = count;
            self.key_count_changed();
        }
    }

    /// Reloads the counts/time shown by the right status bar.
    #[qslot(qml_name = "refreshLibraryStatus")]
    fn refresh_library_status(&mut self) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
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
            let client = match client(&dbs).await {
                Ok(client) => client,
                Err(err) => {
                    qtbridge::invoke_method!(
                        finish_invoker,
                        "librarySyncFinished",
                        format!("Could not load credentials: {err:#}")
                    );
                    return;
                }
            };

            // The shared pipeline (ARC-8): the started state was already
            // shown synchronously above, so this only reports progress.
            spawn_sync(finish_invoker, progress_invoker, client, false);
        });
    }

    /// `changed` = the server reported new material this run (`updated` in
    /// the sync report). A no-op sync finishes quietly: the list/page
    /// reloads hang off `library_changed`, and every reload is a ScrollSystem
    /// payload swap — never tear the UI down for nothing (RECICLEISLOVE).
    #[qslot(qml_name = "librarySyncFinished")]
    fn library_sync_finished(&mut self, message: String, changed: bool) {
        self.library_syncing = false;
        self.library_syncing_changed();
        self.sync_progress = 1.0;
        self.sync_progress_changed();
        self.sync_message = message;
        self.sync_message_changed();
        self.refresh_library_status();
        if changed {
            self.refresh_queue();
            self.library_changed();
            // Changed songs/albums dropped their lyric cache above: reload the
            // open lyrics surface, not only on the next track change.
            self.request_lyrics();
        }
    }

    /// The secondary cover-cache pass finished (`cache_covers`): re-resolve
    /// the open queue and pages only when it actually replaced art.
    #[qslot(qml_name = "coversCached")]
    fn covers_cached(&mut self, changed: bool) {
        if changed {
            self.refresh_queue();
            self.library_changed();
        }
    }

    /// Re-resolves the queue's track metadata, cover ids and URLs, so a sync
    /// that replaced rows or art cannot keep rendering the old snapshot.
    fn refresh_queue(&mut self) {
        let invoker = self.get_qml_method_invoker();
        let tracks: Vec<(String, String, String)> = self
            .player
            .queue()
            .iter()
            .map(|track| {
                (
                    track.album_id.clone(),
                    track.cover_art.clone(),
                    track.id.clone(),
                )
            })
            .collect();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            let album_ids: Vec<String> = tracks
                .iter()
                .map(|(album_id, _, _)| album_id.clone())
                .filter(|id| !id.is_empty())
                .collect();
            let albums = crate::library::album_covers(&dbs.library, &album_ids)
                .await
                .unwrap_or_default();
            let song_ids: Vec<String> = tracks.iter().map(|(_, _, id)| id.clone()).collect();
            let songs = crate::library::songs_by_ids(&dbs.library, &song_ids)
                .await
                .unwrap_or_default();
            let cover_ids: Vec<String> = tracks
                .iter()
                .map(|(album_id, cover, _)| {
                    albums
                        .get(album_id)
                        .cloned()
                        .unwrap_or_else(|| cover.clone())
                })
                .filter(|id| !id.is_empty())
                .collect();
            let covers = crate::covers::url_map_of(&dbs, &cover_ids)
                .await
                .unwrap_or_default();
            let json = serde_json::json!({ "covers": covers, "albums": albums, "songs": songs })
                .to_string();
            qtbridge::invoke_method!(invoker, "applyQueueCovers", json);
        });
        self.request_colors();
    }

    /// Destructive maintenance (the SuperiorBar trashcan's quadrant menu —
    /// a click or Alt while it is scoped): `scope` is "all", "library",
    /// "indexes" or "covers". The databases are dropped and re-migrated from
    /// the embedded schema; "all" also removes credentials and returns to the
    /// Login screen.
    #[qslot(qml_name = "wipeDatabases")]
    fn wipe_databases(&mut self, scope: String) {
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        self.library_sync_started();
        self.sync_message = format!("Wiping {scope}…");
        self.sync_message_changed();
        crate::runtime().spawn(async move {
            let result = match scope.as_str() {
                "library" => crate::db::wipe_library(&dbs).await,
                "indexes" => crate::db::wipe_indexes(&dbs).await,
                "covers" => crate::db::wipe_covers(&dbs).await,
                _ => {
                    // The full reset wipes the config directory itself (the
                    // scheme is what "all" always promised); the databases
                    // come back as fresh files holding the current schema —
                    // an old-version schema cannot survive.
                    match crate::db::wipe_files(&dbs).await {
                        Ok(()) => match crate::db::init().await {
                            Ok(fresh) => {
                                crate::set_dbs(fresh);
                                Ok(())
                            }
                            Err(err) => Err(err),
                        },
                        Err(err) => Err(err),
                    }
                }
            };
            match result {
                Ok(()) => {
                    qtbridge::invoke_method!(invoker, "wipeFinished", scope);
                }
                Err(err) => {
                    eprintln!("db: wipe ({scope}) failed: {err:#}");
                    qtbridge::invoke_method!(
                        error_invoker,
                        "librarySyncFinished",
                        format!("Wipe failed: {err:#}")
                    );
                    raise(&error_invoker, "error", format!("Wipe failed: {err:#}"));
                }
            }
        });
    }

    /// Wipe completed on the runtime thread: "all" resets the session back to
    /// Login (credentials are gone); the partial wipes just refresh the UI.
    #[qslot(qml_name = "wipeFinished")]
    fn wipe_finished(&mut self, scope: String) {
        if scope == "all" {
            crate::runtime().spawn(async { support::invalidate_client().await });
            self.player.stop();
            self.sync_player();
            self.reset_colors();
            ONLINE.store(false, Ordering::Relaxed);
            self.offline = false;
            self.offline_changed();
            self.logged_in = false;
            self.logged_in_changed();
            self.status_message = "Databases wiped — sign in again.".to_string();
            self.status_message_changed();
            self.library_sync_finished("Databases wiped — sign in again.".to_string(), true);
            self.login_required();
            return;
        }
        // Every wipe invalidates cached UI state: the counts and list/page
        // payloads reload through `library_sync_finished`, and the queue +
        // palette are re-emitted here so nothing keeps rendering wiped data.
        self.library_sync_finished(
            match scope.as_str() {
                "library" => {
                    "Albums, songs, covers, lyrics and descriptions wiped — run a sync to refetch."
                        .to_string()
                }
                "indexes" => "Search indexes wiped — rebuilt on the next sync.".to_string(),
                "covers" => "Cover cache wiped — art refetches on demand.".to_string(),
                other => format!("Wiped {other}."),
            },
            true,
        );
    }

    /// Applies re-resolved queue cover ids and URLs (sync/wipe reload path).
    #[qslot(qml_name = "applyQueueCovers")]
    fn apply_queue_covers(&mut self, covers_json: String) {
        let payload: serde_json::Value =
            serde_json::from_str(&covers_json).unwrap_or(serde_json::Value::Null);
        let covers = serde_json::from_value(payload["covers"].clone()).unwrap_or_default();
        let albums = serde_json::from_value(payload["albums"].clone()).unwrap_or_default();
        let songs: Vec<crate::library::SongRow> =
            serde_json::from_value(payload["songs"].clone()).unwrap_or_default();
        self.player.refresh_track_metadata(&songs);
        self.player.refresh_cover_arts(&albums, &covers);
        self.sync_player();
        self.emit_queue(false);
    }

    /// Re-emits the current palette slot as `dark: null` so Main.qml falls
    /// back to the static palette (used when a wipe removed the colors).
    fn reset_colors(&mut self) {
        let payload = serde_json::json!({
            "cover_art": self.current_cover_art.clone(),
            "cover_hash": "",
            "scheme": "",
            "seed": 0,
            "role_hash": "",
            "dark": serde_json::Value::Null,
        })
        .to_string();
        self.data_ready("colors".to_string(), payload);
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
        let order = crate::library::effective_order(&crate::library::ORDERS, &self.album_order, 0)
            .to_string();
        let offline = self.offline;
        self.spawn_data("albums", move |dbs| async move {
            load_albums_json(&dbs, &artist, &order, offline).await
        });
    }

    /// ARC-44 M0: the Playlists segment — rescans the local m3u folder into
    /// `playlist.db` and emits the list rows. Subsonic playlists join in M1.
    #[qslot(qml_name = "requestPlaylists")]
    fn request_playlists(&mut self) {
        self.spawn_data("playlists", |dbs| async move {
            // Subsonic playlists + the local m3u folder; offline serves the
            // cached rows and the m3u half.
            match client(&dbs).await {
                Ok(client) => crate::playlists::refresh(&dbs, &client).await,
                Err(_) => crate::playlists::scan(&dbs).await,
            }
        });
    }

    /// Cycles the main-scope album order (Shift-L / Shift-H).
    #[qslot(qml_name = "albumOrderNext")]
    fn album_order_next(&mut self) {
        self.step_album_order(1);
    }

    #[qslot(qml_name = "albumOrderPrev")]
    fn album_order_prev(&mut self) {
        self.step_album_order(-1);
    }

    /// Cycles the Songs-segment order (Shift-L / Shift-H there).
    #[qslot(qml_name = "songOrderNext")]
    fn song_order_next(&mut self) {
        self.step_song_order(1);
    }

    #[qslot(qml_name = "songOrderPrev")]
    fn song_order_prev(&mut self) {
        self.step_song_order(-1);
    }

    /// Cycles the Artists-segment order (Shift-L / Shift-H there).
    #[qslot(qml_name = "artistOrderNext")]
    fn artist_order_next(&mut self) {
        self.step_artist_order(1);
    }

    #[qslot(qml_name = "artistOrderPrev")]
    fn artist_order_prev(&mut self) {
        self.step_artist_order(-1);
    }

    fn step_album_order(&mut self, delta: i64) {
        let next =
            crate::library::effective_order(&crate::library::ORDERS, &self.album_order, delta)
                .to_string();
        if self.album_order != next {
            self.album_order = next;
            self.album_order_changed();
        }
    }

    fn step_song_order(&mut self, delta: i64) {
        let next =
            crate::library::effective_order(&crate::library::ORDERS, &self.song_order, delta)
                .to_string();
        if self.song_order != next {
            self.song_order = next;
            self.song_order_changed();
        }
    }

    fn step_artist_order(&mut self, delta: i64) {
        let next = crate::library::effective_order(
            &crate::library::ARTIST_ORDERS,
            &self.artist_order,
            delta,
        )
        .to_string();
        if self.artist_order != next {
            self.artist_order = next;
            self.artist_order_changed();
        }
    }

    #[qslot(qml_name = "requestSongsPage")]
    fn request_songs_page(&mut self, offset: i64, limit: i64) {
        let order = crate::library::effective_order(&crate::library::ORDERS, &self.song_order, 0)
            .to_string();
        let offline = self.offline;
        self.spawn_data("songs_page", move |dbs| async move {
            load_songs_page_json(&dbs, offset, limit, &order, offline).await
        });
    }

    #[qslot(qml_name = "requestArtists")]
    fn request_artists(&mut self) {
        let order =
            crate::library::effective_order(&crate::library::ARTIST_ORDERS, &self.artist_order, 0)
                .to_string();
        let offline = self.offline;
        self.spawn_data("artists", move |dbs| async move {
            load_artists_json(&dbs, &order, offline).await
        });
    }

    #[qslot(qml_name = "requestArtist")]
    fn request_artist(&mut self, artist_id: String) {
        let order = crate::library::effective_order(&crate::library::ORDERS, &self.album_order, 0)
            .to_string();
        let offline = self.offline;
        self.spawn_data("artist", move |dbs| async move {
            load_artist_json(&dbs, &artist_id, &order, offline)
                .await?
                .ok_or_else(|| anyhow::anyhow!("artist {artist_id} is not indexed"))
        });
    }

    // -----------------------------------------------------------------------
    // Artist user state + biography (ARC-42)
    // -----------------------------------------------------------------------

    /// Serves the locally indexed artist user state (favourite + rating).
    #[qslot(qml_name = "requestArtistUser")]
    fn request_artist_user(&mut self, artist_id: String) {
        self.spawn_data("artist_user", move |dbs| async move {
            Ok(crate::artist_user::payload(&dbs.artist_user, &artist_id).await)
        });
    }

    /// Toggles the artist favourite on the server, then indexes it locally in
    /// `artist_user.db`. The UI toggles optimistically and reverts on the
    /// error toast.
    #[qslot(qml_name = "starArtist")]
    fn star_artist(&mut self, artist_id: String, on: bool) {
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = apply_artist_user(&dbs, &artist_id, ArtistUserAction::Star(on)).await;
            let json = crate::artist_user::payload(&dbs.artist_user, &artist_id).await;
            emit(&invoker, "artist_user", json);
            if let Err(err) = result {
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not toggle favourite: {err:#}"),
                );
            }
        });
    }

    /// Sets the artist rating (0..=5) on the server, then indexes it locally
    /// in `artist_user.db`. The UI toggles optimistically and reverts on the
    /// error toast.
    #[qslot(qml_name = "rateArtist")]
    fn rate_artist(&mut self, artist_id: String, rating: i32) {
        let rating = match u8::try_from(rating) {
            Ok(rating) if rating <= 5 => rating,
            _ => {
                eprintln!("artist: rating {rating} is outside the 0..=5 domain");
                return;
            }
        };
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = apply_artist_user(&dbs, &artist_id, ArtistUserAction::Rate(rating)).await;
            let json = crate::artist_user::payload(&dbs.artist_user, &artist_id).await;
            emit(&invoker, "artist_user", json);
            // UI-60: patch the artist rows in place (list/search), also on
            // failure — the stored value reverts the optimistic badge.
            let stored = crate::artist_user::get(&dbs.artist_user, &artist_id)
                .await
                .ok()
                .flatten()
                .map(|row| row.rating)
                .unwrap_or(0);
            qtbridge::invoke_method!(
                invoker,
                "ratingChanged",
                "artist".to_string(),
                artist_id.clone(),
                stored
            );
            if let Err(err) = result {
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not rate artist: {err:#}"),
                );
            }
        });
    }

    /// Fetches the artist biography (`getArtistInfo2` → `getArtistInfo`
    /// fallback), caches it in `library.db` (`artist_info`) and emits
    /// `artist_info`; offline serves the cached row.
    #[qslot(qml_name = "requestArtistInfo")]
    fn request_artist_info(&mut self, artist_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let cached = crate::library::artist_info(&dbs.library, &artist_id)
                .await
                .ok()
                .flatten();
            if let Some(row) = &cached {
                emit(
                    &invoker,
                    "artist_info",
                    serde_json::json!({
                        "id": artist_id,
                        "biography": row.biography,
                        "url": row.url,
                    })
                    .to_string(),
                );
                if let Ok(fetched_at) = row.fetched_at.parse::<i64>()
                    && crate::consts::now_secs() - fetched_at < crate::consts::INFO_TTL_SECS
                {
                    return;
                }
            }
            let fetched = async {
                let client = client(&dbs).await?;
                let (biography, url) =
                    match client.get_artist_info2(&artist_id, Some(10), None).await {
                        Ok(info) => (info.info.biography, info.info.last_fm_url),
                        Err(err) => {
                            eprintln!("artist: getArtistInfo2 failed for {artist_id}: {err:#}");
                            let info = client.get_artist_info(&artist_id, Some(10), None).await?;
                            (info.info.biography, info.info.last_fm_url)
                        }
                    };
                let biography = biography.unwrap_or_default();
                let url = url.unwrap_or_default();
                crate::library::upsert_artist_info(&dbs.library, &artist_id, &biography, &url)
                    .await?;
                Ok::<_, anyhow::Error>((biography, url))
            }
            .await;

            match fetched {
                Ok((biography, url)) => {
                    emit(
                        &invoker,
                        "artist_info",
                        serde_json::json!({
                            "id": artist_id,
                            "biography": biography,
                            "url": url,
                        })
                        .to_string(),
                    );
                }
                Err(err) => {
                    eprintln!("artist: info fetch failed for {artist_id}: {err:#}");
                    if cached.is_none() {
                        emit(
                            &invoker,
                            "artist_info",
                            serde_json::json!({
                                "id": artist_id,
                                "biography": "",
                                "url": "",
                            })
                            .to_string(),
                        );
                    }
                }
            }
        });
    }
    /// CACHE-1/8: an explicit user download. Idempotent — a cached blob or an
    /// in-flight fetch short-circuits; otherwise marks the ledger, fetches the
    /// original file, stores it and flips the ledger to `done` (or `error`
    /// with the reason). Emits the `download` state and an error toast.
    #[qslot(qml_name = "downloadSong")]
    fn download_song(&mut self, song_id: String) {
        // Optimistic and targeted: the row's loading glyph spins from the
        // first frame, the completion flips that row (and its album's) only.
        self.note_download_state(song_id.clone(), crate::download::ACTIVE.to_string());
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            match download_one(&dbs, &song_id).await {
                Ok(state) => {
                    qtbridge::invoke_method!(
                        invoker,
                        "noteDownloadState",
                        song_id.clone(),
                        state.to_string()
                    );
                    if state == crate::download::DONE {
                        if let Some(album) = completed_album(&dbs, &song_id).await {
                            qtbridge::invoke_method!(invoker, "markAlbumCached", album);
                        }
                    }
                }
                Err(err) => {
                    qtbridge::invoke_method!(
                        invoker,
                        "noteDownloadState",
                        song_id.clone(),
                        crate::download::ERROR.to_string()
                    );
                    raise(&error_invoker, "error", format!("Download failed: {err:#}"));
                }
            }
        });
    }

    /// The stored artist-container selection (`artist_cache` channel).
    #[qslot(qml_name = "requestArtistCache")]
    fn request_artist_cache(&mut self, artist_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            // The artist is "on" when every album of it holds an album rule
            // (the batch is per album now; a legacy artist rule has no role).
            let albums = crate::library::artist_album_ids(&dbs.library, &artist_id)
                .await
                .unwrap_or_default();
            let mut on = !albums.is_empty();
            for album in &albums {
                if !matches!(
                    crate::media::rule_for(&dbs.media, "album", album).await,
                    Ok(Some(_))
                ) {
                    on = false;
                    break;
                }
            }
            let json = artist_cache_payload(&artist_id, on, false);
            emit(&invoker, "artist_cache", json);
        });
    }

    /// The artist batch, reusable-style: it lists the artist's albums (the
    /// albums table owns that mapping) and runs the *album* container flow
    /// per album — selection, ledger marks, one shared fetch pass. No
    /// artist-kind rule exists anymore: an album whose `artist_id` disagrees
    /// with its songs' `artist_id` (server metadata) is still covered by its
    /// own album rule, which the old artist rule could never reach. Emits
    /// `artist_cache` for the ArtistEntry toggle.
    #[qslot(qml_name = "downloadArtist")]
    fn download_artist(&mut self, artist_id: String, on: bool) {
        // The request is initiated now: the row's loading glyph lights the
        // same frame (the pass' batch and completions follow per id).
        if on {
            self.note_download_states(serde_json::json!([artist_id]).to_string());
        }
        let invoker = self.get_qml_method_invoker();
        let fetch_invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let albums = match crate::library::artist_album_ids(&dbs.library, &artist_id).await {
                Ok(albums) => albums,
                Err(err) => {
                    eprintln!("media: could not list the artist's albums: {err:#}");
                    return;
                }
            };
            if on {
                let mut songs: Vec<String> = Vec::new();
                let mut queued_albums: Vec<String> = Vec::new();
                for album in &albums {
                    match album_container_on(&dbs, album).await {
                        Ok(album_songs) => {
                            eprintln!(
                                "download: artist {artist_id} album {album} queued ({} songs)",
                                album_songs.len()
                            );
                            songs.extend(album_songs);
                            queued_albums.push(album.clone());
                        }
                        Err(err) => {
                            raise(
                                &error_invoker,
                                "error",
                                format!("Could not select the album for download: {err:#}"),
                            );
                        }
                    }
                }
                // No bulk in-flight batch: the fetch pass lights each chunk
                // (three songs) and their albums as it reaches them — a
                // row's loading glyph means its track is fetching *now*.
                eprintln!(
                    "download: artist {artist_id} pass started ({} albums, {} songs)",
                    queued_albums.len(),
                    songs.len()
                );
                // The pass is now in flight: the chip shows its loading loop.
                let json = artist_cache_payload(&artist_id, true, true);
                emit(&invoker, "artist_cache", json);
                fetch_marked(&dbs, &songs, &artist_id, false, fetch_invoker).await;
                // The albums that had nothing to fetch never emitted a
                // completion: settle their rows here.
                for album in &queued_albums {
                    qtbridge::invoke_method!(invoker, "settleDownloadRequest", album.clone());
                }
                eprintln!("download: artist {artist_id} pass finished");
                let json = artist_cache_payload(&artist_id, true, false);
                emit(&invoker, "artist_cache", json);
            } else {
                for album in &albums {
                    album_container_off(&dbs, album).await;
                }
                // Legacy cleanup: the pre-album design kept an artist-kind
                // rule; it has no role anymore.
                if let Ok(Some(rule)) =
                    crate::media::rule_for(&dbs.media, "artist", &artist_id).await
                {
                    rule_off(&dbs, &rule.id).await;
                }
                eprintln!("download: artist {artist_id} removed from the cache");
                let json = artist_cache_payload(&artist_id, false, false);
                emit(&invoker, "artist_cache", json);
                // The marks lose the artist's songs: hard reset + re-read.
                qtbridge::invoke_method!(fetch_invoker, "ResetRowMarks");
            }
        });
    }

    /// The album container state for the fullscreen keybar's Download button
    /// (`album_cache`): an album rule, or an artist rule covering the album.
    #[qslot(qml_name = "requestAlbumCache")]
    fn request_album_cache(&mut self, album_id: String) {
        self.spawn_data("album_cache", move |dbs| async move {
            let album_rule = matches!(
                crate::media::rule_for(&dbs.media, "album", &album_id).await,
                Ok(Some(_))
            );
            let artist_covered =
                match crate::library::album_artist_id(&dbs.library, &album_id).await {
                    Ok(Some(artist_id)) => artist_covers_album(&dbs, &artist_id, &album_id).await,
                    _ => false,
                };
            Ok(album_cache_payload(
                &album_id,
                album_rule || artist_covered,
                false,
                artist_covered,
            ))
        });
    }

    /// The album favourite flag for the fullscreen keybar's heart.
    #[qslot(qml_name = "requestAlbumState")]
    fn request_album_state(&mut self, album_id: String) {
        self.spawn_data("album_state", move |dbs| async move {
            let starred = crate::library::album_starred(&dbs.library, &album_id)
                .await
                .unwrap_or(false);
            let rating = crate::library::album_rating(&dbs.library, &album_id)
                .await
                .unwrap_or(0);
            // Re-resolve the header cover here too: a sync can replace the
            // art (or a wipe can drop it) while the page stays open.
            let cover_ids: Vec<String> = crate::library::album(&dbs.library, &album_id)
                .await
                .ok()
                .flatten()
                .and_then(|album| album.cover_art)
                .into_iter()
                .collect();
            let covers = crate::covers::url_map_of(&dbs, &cover_ids)
                .await
                .unwrap_or_default();
            let cover = cover_ids
                .first()
                .and_then(|id| covers.get(id))
                .cloned()
                .unwrap_or_default();
            Ok(serde_json::json!({
                "id": album_id,
                "starred": starred,
                "rating": rating,
                "cover": cover,
            })
            .to_string())
        });
    }

    /// UI-60: sets an album's 0..=5 user rating on the server, then indexes
    /// it locally. The overlay updates the badge optimistically; the
    /// `ratingChanged` signal patches every visible row and the
    /// `album_state` payload reconciles the album header/keybar (also on
    /// failure, with the stored value).
    #[qslot(qml_name = "rateAlbum")]
    fn rate_album(&mut self, album_id: String, rating: i32) {
        let rating = match u8::try_from(rating) {
            Ok(rating) if rating <= 5 => rating,
            _ => {
                eprintln!("album: rating {rating} is outside the 0..=5 domain");
                return;
            }
        };
        let error_invoker = self.get_qml_method_invoker();
        let state_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = async {
                let client = client(&dbs).await?;
                client.set_rating(&album_id, rating).await?;
                crate::library::set_album_rating(&dbs.library, &album_id, rating).await
            }
            .await;
            // Re-read the stored row: on failure this reverts the optimistic
            // badge to the last accepted value.
            let stored = crate::library::album_rating(&dbs.library, &album_id)
                .await
                .unwrap_or(0);
            let starred = crate::library::album_starred(&dbs.library, &album_id)
                .await
                .unwrap_or(false);
            qtbridge::invoke_method!(
                state_invoker,
                "ratingChanged",
                "album".to_string(),
                album_id.clone(),
                stored
            );
            emit(
                &state_invoker,
                "album_state",
                album_state_payload(&album_id, starred, stored),
            );
            if let Err(err) = result {
                eprintln!("album: could not rate {album_id}: {err:#}");
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not rate album: {err:#}"),
                );
            }
        });
    }

    /// The album twin of [`downloadArtist`] — the keybar's Download button:
    /// ON marks and fetches the album's songs, unless an artist rule already
    /// covers them (then the request is skipped — "if the artist is already
    /// in download scope, don't"); OFF drops the rule, its marks and its
    /// rule-owned blobs. Emits `album_cache` throughout.
    #[qslot(qml_name = "downloadAlbum")]
    fn download_album(&mut self, album_id: String, on: bool) {
        // The request is initiated now: the row's loading glyph lights the
        // same frame (the pass' batch and completions follow per id).
        if on {
            self.note_download_states(serde_json::json!([album_id]).to_string());
        }
        let invoker = self.get_qml_method_invoker();
        let fetch_invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            if on {
                let artist_covered =
                    match crate::library::album_artist_id(&dbs.library, &album_id).await {
                        Ok(Some(artist_id)) => {
                            artist_covers_album(&dbs, &artist_id, &album_id).await
                        }
                        _ => false,
                    };
                if artist_covered {
                    // Redundant: the artist container already owns these songs.
                    eprintln!(
                        "download: album {album_id} already covered by the artist container; skipping"
                    );
                    let json = album_cache_payload(&album_id, true, false, true);
                    emit(&invoker, "album_cache", json);
                    return;
                }
                let songs = match album_container_on(&dbs, &album_id).await {
                    Ok(songs) => songs,
                    Err(err) => {
                        raise(&error_invoker, "error", format!("Could not select the album for download: {err:#}"));
                        return;
                    }
                };
                eprintln!(
                    "download: album {album_id} pass started ({} songs)",
                    songs.len()
                );
                // The pass is now in flight: the chip shows its loading loop.
                let json = album_cache_payload(&album_id, true, true, false);
                emit(&invoker, "album_cache", json);
                fetch_marked(&dbs, &songs, &album_id, true, fetch_invoker).await;
                eprintln!("download: album {album_id} pass finished");
                let json = album_cache_payload(&album_id, true, false, false);
                emit(&invoker, "album_cache", json);
            } else {
                album_container_off(&dbs, &album_id).await;
                eprintln!("download: album {album_id} removed from the cache");
                let json = album_cache_payload(&album_id, false, false, false);
                emit(&invoker, "album_cache", json);
                // The marks lose the album's songs: hard reset + re-read.
                qtbridge::invoke_method!(fetch_invoker, "ResetRowMarks");
            }
        });
    }

    #[qslot(qml_name = "requestAlbumSongs")]
    fn request_album_songs(&mut self, album_id: String) {
        let offline = self.offline;
        self.spawn_data("album_songs", move |dbs| async move {
            load_album_songs_json(&dbs, &album_id, offline).await
        });
    }

    /// Fetches an album's description ("notes") from the server.
    #[qslot(qml_name = "requestAlbumInfo")]
    fn request_album_info(&mut self, album_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let cached = crate::library::album_info(&dbs.library, &album_id)
                .await
                .ok()
                .flatten();
            if let Some((notes, fetched_at)) = &cached {
                emit(
                    &invoker,
                    "album_info",
                    serde_json::json!({ "notes": notes }).to_string(),
                );
                if crate::consts::now_secs() - fetched_at < crate::consts::INFO_TTL_SECS {
                    return;
                }
            }
            let fresh = album_notes(&dbs, &album_id).await;
            if fresh.is_empty() {
                if cached.is_none() {
                    emit(
                        &invoker,
                        "album_info",
                        serde_json::json!({ "notes": "" }).to_string(),
                    );
                }
                return;
            }
            if let Err(err) =
                crate::library::upsert_album_info(&dbs.library, &album_id, &fresh).await
            {
                eprintln!("album: could not cache notes for {album_id}: {err:#}");
            }
            emit(
                &invoker,
                "album_info",
                serde_json::json!({ "notes": fresh }).to_string(),
            );
        });
    }

    /// ARC-UI-7.5: the persisted scroll store (empty on first launch).
    #[qslot(qml_name = "scrollMemory")]
    fn scroll_memory(&self) -> String {
        let Ok(path) = crate::db::scroll_path() else {
            return String::new();
        };
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// ARC-UI-7.5: atomic write of the scroll store from the QML debounce.
    #[qslot(qml_name = "saveScrollMemory")]
    fn save_scroll_memory(&mut self, payload: String) {
        let Ok(path) = crate::db::scroll_path() else {
            return;
        };
        let generation = SCROLL_WRITE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        crate::runtime().spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                if SCROLL_WRITE_GEN.load(Ordering::Relaxed) != generation {
                    return;
                }
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, payload).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            })
            .await;
        });
    }

    /// Global library search (artists > albums > songs) for the search bar.
    #[qslot(qml_name = "requestSearch")]
    fn request_search(&mut self, query: String, limit: i64) {
        let offline = self.offline;
        // The UI passes the number of rows its columns can paint; keep a sane
        // ceiling for a malformed caller.
        let limit = limit.clamp(1, 100) as usize;
        let generation = SEARCH_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        crate::runtime().spawn(async move {
            // A newer keystroke owns the box: never start (or emit) a read
            // the user already superseded. The search pool is separate from
            // the library writer, so this task can never pin the sync.
            if SEARCH_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            let json = search_json(&dbs, &query, offline, limit)
                .await
                .unwrap_or_else(|err| {
                    eprintln!("search: {err:#}");
                    serde_json::json!({
                        "query": query,
                        "most_played": query.trim().is_empty(),
                        "artists": [],
                        "albums": [],
                        "songs": []
                    })
                    .to_string()
                });
            if SEARCH_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            emit(&invoker, "search", json);
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
        let offline = self.offline;
        crate::runtime().spawn(async move {
            match album_tracks_json(&dbs, &album_id, offline).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "playerLoad", json, start);
                }
                Err(err) => {
                    eprintln!("player: could not start playback: {err:#}");
                    raise(&invoker, "playback", format!("{err:#}"));
                }
            }
        });
    }

    /// Plays a song inside the Songs segment's queue: the whole library in
    /// list order, starting on the clicked row (tracks before it included);
    /// The reusable shuffle toggle (Album/Songs tabs first; the fullscreen
    /// queue follows the same shape). ON: the tab's source — `songs` = the
    /// library's songs in the active order, `albums` = the library's
    /// albums, each expanding to its tracklist — Fisher–Yates shuffled,
    /// queued in full and played from its head. A second press disables the
    /// shuffle (the queue keeps playing what is in it).
    #[qslot(qml_name = "toggleShuffle")]
    fn toggle_shuffle(&mut self, kind: String) {
        if self.shuffling {
            self.shuffle_gen = self.shuffle_gen.wrapping_add(1);
            self.set_shuffling(false, String::new());
            eprintln!("shuffle: off");
            return;
        }
        eprintln!("shuffle: {kind}");
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let offline = self.offline;
        let order = if kind == "albums" {
            self.album_order.clone()
        } else {
            self.song_order.clone()
        };
        self.shuffle_gen = self.shuffle_gen.wrapping_add(1);
        let generation = self.shuffle_gen;
        // Optimistic: the chip lights on the press, the state lands with
        // the whole queue or clears on failure.
        self.set_shuffling(true, kind.clone());
        crate::runtime().spawn(async move {
            let units = if kind == "albums" {
                crate::library::albums(&dbs.library, None, &order)
                    .await
                    .map(|rows| rows.into_iter().map(|album| album.id).collect::<Vec<_>>())
            } else {
                crate::library::all_songs(&dbs.library, &order)
                    .await
                    .map(|rows| rows.into_iter().map(|song| song.id).collect::<Vec<_>>())
            };
            let Ok(mut units) = units else {
                qtbridge::invoke_method!(
                    invoker,
                    "shuffleFailed",
                    generation as i64,
                    "Could not read the shuffle source.".to_string()
                );
                return;
            };
            if units.is_empty() {
                qtbridge::invoke_method!(
                    invoker,
                    "shuffleFailed",
                    generation as i64,
                    "Nothing to shuffle here.".to_string()
                );
                return;
            }
            let seed = crate::consts::now_nanos();
            crate::queue::shuffle_units(&mut units, seed);
            match crate::queue::shuffle_queue_json(&dbs, &kind, &units, offline).await {
                Ok(tracks_json) => {
                    let tracks = serde_json::from_str::<serde_json::Value>(&tracks_json)
                        .ok()
                        .and_then(|value| value.as_array().map(Vec::len))
                        .unwrap_or(0);
                    eprintln!(
                        "shuffle: {kind} starting — {tracks} tracks ({} units)",
                        units.len()
                    );
                    qtbridge::invoke_method!(invoker, "playerLoad", tracks_json, 0_i64);
                }
                Err(err) => {
                    eprintln!("shuffle: could not resolve the queue: {err:#}");
                    qtbridge::invoke_method!(
                        invoker,
                        "shuffleFailed",
                        generation as i64,
                        "The shuffle could not start.".to_string()
                    );
                }
            }
        });
    }

    /// The shuffle could not start (generation-guarded): clear the chip and
    /// surface the reason.
    #[qslot(qml_name = "shuffleFailed")]
    fn shuffle_failed(&mut self, generation: i64, message: String) {
        if generation.max(0) as u64 != self.shuffle_gen {
            return;
        }
        self.set_shuffling(false, String::new());
        self.error_raised("playback".to_string(), message);
    }

    fn set_shuffling(&mut self, active: bool, kind: String) {
        if self.shuffling != active {
            self.shuffling = active;
            self.shuffling_changed();
        }
        if self.shuffle_kind != kind {
            self.shuffle_kind = kind;
            self.shuffle_kind_changed();
        }
    }

    /// The artist page's shuffle chip: the artist's albums (the 2-D array
    /// albums → songs) Fisher–Yates shuffled, queued in full and played
    /// from the order's head — one album after another until it ends.
    #[qslot(qml_name = "shuffleArtist")]
    fn shuffle_artist(&mut self, artist_id: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let offline = self.offline;
        crate::runtime().spawn(async move {
            match crate::queue::artist_shuffle_json(&dbs, &artist_id, offline).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "playerLoad", json, 0_i64);
                }
                Err(err) => {
                    eprintln!("player: could not shuffle the artist: {err:#}");
                    raise(&invoker, "playback", format!("{err:#}"));
                }
            }
        });
    }

    /// never opens fullscreen (Q-10 refinement).
    #[qslot(qml_name = "playSong")]
    fn play_song(&mut self, song_id: String) {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let order = crate::library::effective_order(&crate::library::ORDERS, &self.song_order, 0)
            .to_string();
        let offline = self.offline;
        crate::runtime().spawn(async move {
            match library_queue_payload(&dbs, &song_id, &order, offline).await {
                Ok((json, start)) => {
                    qtbridge::invoke_method!(invoker, "playerLoadLibrary", json, start as i64);
                }
                Err(err) => {
                    eprintln!("player: could not play song: {err:#}");
                    raise(&invoker, "playback", format!("{err:#}"));
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
        let offline = self.offline;
        crate::runtime().spawn(async move {
            match song_album_payload(&dbs, &song_id, offline).await {
                Ok(payload) => {
                    emit(&invoker, "song_album", payload);
                }
                Err(err) => {
                    eprintln!("player: could not queue album: {err:#}");
                    raise(&invoker, "playback", format!("{err:#}"));
                }
            }
        });
    }

    /// Loads a serialized queue into the player and opens Fullscreen Playback
    /// (album pages, search hits).
    #[qslot(qml_name = "playerLoad")]
    fn player_load(&mut self, tracks_json: String, start: i64) {
        self.load_queue(tracks_json, start, true);
    }

    /// Loads the restored queue paused (P-6 S2): no autoplay, no fullscreen;
    /// `Space` resumes immediately.
    #[qslot(qml_name = "playerLoadPaused")]
    fn player_load_paused(&mut self, tracks_json: String, start: i64) {
        match serde_json::from_str::<Vec<crate::player::Track>>(&tracks_json) {
            Ok(tracks) => {
                self.player.load_paused(tracks, index_value(start));
                // S3: the saved playhead, applied once to the restored queue.
                if let Some(position) = self.status_restore_position.take() {
                    self.player.seek(position);
                }
                self.sync_player();
                self.emit_queue(false);
            }
            Err(err) => {
                eprintln!("player: bad queue payload: {err}");
                self.error_raised("playback".to_string(), format!("Bad queue payload: {err}"));
            }
        }
    }

    /// Loads the Songs-segment queue (the whole index from the clicked row)
    /// without the fullscreen flip (Q-10 refinement).
    #[qslot(qml_name = "playerLoadLibrary")]
    fn player_load_library(&mut self, tracks_json: String, start: i64) {
        self.load_queue(tracks_json, start, false);
    }

    fn load_queue(&mut self, tracks_json: String, start: i64, announce: bool) {
        match serde_json::from_str::<Vec<crate::player::Track>>(&tracks_json) {
            Ok(tracks) => {
                self.player.load(tracks, index_value(start));
                self.sync_player();
                self.emit_queue(false);
                if announce {
                    self.playback_started();
                }
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
        self.play_toggled();
        self.sync_player();
    }

    #[qslot(qml_name = "playerNext")]
    fn player_next(&mut self) {
        self.player.next();
        self.sync_player();
        self.emit_queue(false);
        self.track_skipped(true);
    }

    #[qslot(qml_name = "playerPrevious")]
    fn player_previous(&mut self) {
        self.player.previous();
        self.sync_player();
        self.emit_queue(false);
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

    /// Publishes the current queue to QML. The heart's truth is read from
    /// `library.db` here (the in-memory track's flag goes stale after
    /// `altFavorite`); the download ledger is NOT folded in — it has its own
    /// channel ([`Self::request_downloads`]), so a row mark is a direct
    /// database read, never a payload variable.
    ///
    /// `force` publishes even when the rows are already out (the explicit
    /// `requestQueue`); the automatic path (skips, advances, the 500 ms
    /// tick) dedupes on the rows — "RowAlreadyPublished" — and lets the
    /// `queueIndex` property carry the move instead, so a skip costs one
    /// property notify, not a payload, a parse and a model pass.
    ///
    /// QUEUEROWSTUDY QR-1: the player revision is checked *before* the
    /// clone/DB/serialize work — an unchanged revision (only the index
    /// moved) returns here. `queue_rows_dirty` is the bypass for row
    /// changes outside the player (`TriggerRowReload`: syncs, wipe reloads,
    /// download settles).
    fn emit_queue(&mut self, force: bool) {
        let revision = self.player.revision();
        if !force && !self.queue_rows_dirty && self.queue_revision == Some(revision) {
            return;
        }
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        // Record the revision only once the emit is actually going out: a
        // missing DB must leave the next call free to retry.
        self.queue_revision = Some(revision);
        self.queue_rows_dirty = false;
        let queue: Vec<crate::player::Track> = self.player.queue().to_vec();
        let index = self.player.index();
        let generation = QUEUE_EMIT_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        crate::runtime().spawn(async move {
            let starred: std::collections::HashSet<String> =
                crate::library::starred_song_ids(&dbs.library)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
            let items: Vec<serde_json::Value> = queue
                .iter()
                .map(|track| {
                    // QR-4: only the roles the queue delegates paint —
                    // `artist`/`album` fed the drawer's never-painted
                    // `subtitle` and are gone.
                    serde_json::json!({
                        "id": track.id,
                        "title": track.title,
                        "album_id": track.album_id,
                        "starred": starred.contains(&track.id),
                        "duration": track.duration,
                        "cover": track.cover_url,
                    })
                })
                .collect();
            // A newer emit already went out: this one is stale.
            if QUEUE_EMIT_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            // RowAlreadyPublished: the same row set is already out — a skip
            // or an advance only moved the index, and the `queueIndex`
            // property carries that. No payload, no parse, no model pass.
            let rows_json = serde_json::to_string(&items).unwrap_or_default();
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&rows_json, &mut hasher);
            let rows_hash = std::hash::Hasher::finish(&hasher);
            // QR-3: swap unconditionally — a forced publish must store its
            // hash too, or the next automatic emit of the same rows
            // republishes against a stale value.
            let previous = QUEUE_ROWS_HASH.swap(rows_hash, Ordering::Relaxed);
            if !force && previous == rows_hash {
                return;
            }
            crate::debug_log!("queue: published {} rows", items.len());
            let json = serde_json::json!({
                "index": index,
                "items": items,
                // PATH-11: the QML row-set check compares this revision string
                // instead of concatenating every row again in JS.
                "hash": rows_hash.to_string(),
            })
            .to_string();
            emit(&invoker, "queue", json);
        });
    }

    /// Re-reads the download ledger for the requested window (`songs` and
    /// `albums` are the JSON arrays of the currently visible rows) — the row
    /// marks' truth (the download twin of the hearts' library read). Called
    /// whenever the rendered-row window changes, never for the whole library.
    #[qslot(qml_name = "requestDownloads")]
    fn request_downloads(&mut self, songs: String, albums: String) {
        let songs: Vec<String> = serde_json::from_str(&songs).unwrap_or_default();
        let albums: Vec<String> = serde_json::from_str(&albums).unwrap_or_default();
        // Every surface asks on entry, on cursor moves and on rebuilds; the
        // window is the dedupe key — an unchanged window costs nothing.
        if songs == self.download_window_songs && albums == self.download_window_albums {
            return;
        }
        self.store_and_refresh(songs, albums);
    }

    /// The forced re-request (the `rowReload` half): a surface re-reads its
    /// own window after a fetch, even when the window itself is unchanged —
    /// the dedupe must not swallow the update.
    #[qslot(qml_name = "refreshDownloads")]
    fn refresh_downloads(&mut self, songs: String, albums: String) {
        let songs: Vec<String> = serde_json::from_str(&songs).unwrap_or_default();
        let albums: Vec<String> = serde_json::from_str(&albums).unwrap_or_default();
        self.store_and_refresh(songs, albums);
    }

    /// Stores the window (and its request, for the union refreshes) and
    /// starts the read.
    fn store_and_refresh(&mut self, songs: Vec<String>, albums: Vec<String>) {
        self.download_window_songs = songs.clone();
        self.download_window_albums = albums.clone();
        const RECENT_REQUESTS: usize = 12;
        // An identical re-sent window must not flood the union.
        if self
            .recent_mark_requests
            .back()
            .is_none_or(|(s, a)| *s != songs || *a != albums)
        {
            self.recent_mark_requests
                .push_back((songs.clone(), albums.clone()));
            while self.recent_mark_requests.len() > RECENT_REQUESTS {
                self.recent_mark_requests.pop_front();
            }
        }
        self.prune_mark_states();
        self.refresh_window(songs, albums);
    }

    /// Applies a window read on the main thread (the async reader's landing
    /// pad, invoked through the QML method invoker like `applyStatus`). The
    /// window's *queried* ids get their states rewritten; ids outside it are
    /// untouched — one surface's read can never erase another surface's
    /// marks, nor a per-id patch (the old union was clobbered by exactly
    /// that, e.g. a favourite's reload hiding a finished download).
    #[qslot(qml_name = "applyDownloadedIds")]
    fn apply_downloaded_ids(&mut self, ids_json: String) {
        let value: serde_json::Value = serde_json::from_str(&ids_json).unwrap_or_default();
        let songs: Vec<String> = serde_json::from_value(value["songs"].clone()).unwrap_or_default();
        let albums: Vec<String> =
            serde_json::from_value(value["albums"].clone()).unwrap_or_default();
        let downloading: Vec<String> =
            serde_json::from_value(value["downloading"].clone()).unwrap_or_default();
        let queried_songs: Vec<String> =
            serde_json::from_value(value["queried_songs"].clone()).unwrap_or_default();
        let queried_albums: Vec<String> =
            serde_json::from_value(value["queried_albums"].clone()).unwrap_or_default();
        let songs: std::collections::HashSet<String> = songs.into_iter().collect();
        let downloading: std::collections::HashSet<String> = downloading.into_iter().collect();
        let albums: std::collections::HashSet<String> = albums.into_iter().collect();
        for id in queried_songs {
            let state = if songs.contains(&id) {
                Some(MarkState::SongDone)
            } else if downloading.contains(&id) {
                Some(MarkState::SongInFlight)
            } else {
                None
            };
            self.set_mark_state(&id, state);
        }
        for id in queried_albums {
            let state = albums.contains(&id).then_some(MarkState::AlbumCached);
            self.set_mark_state(&id, state);
        }
        self.rebuild_mark_sets();
    }

    /// Writes one id's state (None removes it).
    fn set_mark_state(&mut self, id: &str, state: Option<MarkState>) {
        match state {
            Some(state) => {
                self.mark_states.insert(id.to_string(), state);
            }
            None => {
                self.mark_states.remove(id);
            }
        }
    }

    /// One id's state patch: keeps the exposed sets incremental instead of
    /// rebuilding (and re-sorting) the whole mark truth on every fetch step.
    fn patch_mark_state(&mut self, id: &str, state: Option<MarkState>) {
        let old = self.mark_states.get(id).copied();
        if old == state {
            return;
        }
        self.set_mark_state(id, state);
        let mut done_changed = false;
        let mut cached_changed = false;
        let mut pending_changed = false;
        if let Some(old) = old {
            match old {
                MarkState::SongDone => done_changed |= remove_mark_id(&mut self.downloaded_ids, id),
                MarkState::AlbumCached => {
                    cached_changed |= remove_mark_id(&mut self.cached_album_ids, id)
                }
                MarkState::SongInFlight => {
                    pending_changed |= remove_mark_id(&mut self.downloading_ids, id)
                }
            }
        }
        if let Some(state) = state {
            match state {
                MarkState::SongDone => done_changed |= add_mark_id(&mut self.downloaded_ids, id),
                MarkState::AlbumCached => {
                    cached_changed |= add_mark_id(&mut self.cached_album_ids, id)
                }
                MarkState::SongInFlight => {
                    pending_changed |= add_mark_id(&mut self.downloading_ids, id)
                }
            }
        }
        if done_changed {
            self.downloaded_ids_changed();
        }
        if cached_changed {
            self.cached_album_ids_changed();
        }
        if pending_changed {
            self.downloading_ids_changed();
        }
    }

    /// Drops mark entries that fell out of every recent surface window. The
    /// map therefore stays bounded by the recently requested rows instead of
    /// accumulating every id ever scrolled past.
    fn prune_mark_states(&mut self) {
        let mut allowed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (songs, albums) in &self.recent_mark_requests {
            allowed.extend(songs.iter().cloned());
            allowed.extend(albums.iter().cloned());
        }
        let before = self.mark_states.len();
        self.mark_states.retain(|id, _| allowed.contains(id));
        if self.mark_states.len() != before {
            self.rebuild_mark_sets();
        }
    }

    /// Re-derives the three exposed sets from the state map (sorted,
    /// notify-on-change).
    fn rebuild_mark_sets(&mut self) {
        let mut done: Vec<String> = Vec::new();
        let mut cached: Vec<String> = Vec::new();
        let mut pending: Vec<String> = Vec::new();
        for (id, state) in &self.mark_states {
            match state {
                MarkState::SongDone => done.push(id.clone()),
                MarkState::AlbumCached => cached.push(id.clone()),
                MarkState::SongInFlight => pending.push(id.clone()),
            }
        }
        done.sort();
        cached.sort();
        pending.sort();
        if done != self.downloaded_ids {
            self.downloaded_ids = done;
            self.downloaded_ids_changed();
        }
        if cached != self.cached_album_ids {
            self.cached_album_ids = cached;
            self.cached_album_ids_changed();
        }
        if pending != self.downloading_ids {
            self.downloading_ids = pending;
            self.downloading_ids_changed();
        }
    }

    /// Hard reset + re-read (a download was removed: the gone ids must not
    /// linger). Wired to every container-OFF path; the surfaces repopulate
    /// from the `rowReload` that follows.
    #[qslot(qml_name = "ResetRowMarks")]
    fn reset_row_marks(&mut self) {
        self.mark_states.clear();
        self.rebuild_mark_sets();
        self.refresh_download_window();
        self.row_reload();
    }

    /// One row's fetch transition (targeted): the loading glyph on for
    /// queued/active, the filled mark on done, cleared on error. Only the
    /// affected row's bindings re-evaluate — no window reloads.
    #[qslot(qml_name = "noteDownloadState")]
    fn note_download_state(&mut self, song_id: String, state: String) {
        let in_flight = state == crate::download::QUEUED || state == crate::download::ACTIVE;
        if in_flight {
            self.hold_download_request();
            self.patch_mark_state(&song_id, Some(MarkState::SongInFlight));
            return;
        }
        // The fetch settled: the row keeps its loading glyph until the hold
        // expires (a cached fetch must still be *seen*), then flips.
        if state == crate::download::DONE {
            self.patch_mark_state(&song_id, Some(MarkState::SongDone));
        }
        self.settle_download_request(song_id);
    }

    /// The batch kick at a container pass' start: every marked song's row
    /// spins its loading glyph; per-song completions follow.
    #[qslot(qml_name = "noteDownloadStates")]
    fn note_download_states(&mut self, ids_json: String) {
        let ids: Vec<String> = serde_json::from_str(&ids_json).unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        self.hold_download_request();
        for id in ids {
            self.set_mark_state(&id, Some(MarkState::SongInFlight));
        }
        self.rebuild_mark_sets();
    }

    /// Starts (or extends) the request hold: `WhileDownloadRequest` stays
    /// true for `DOWNLOAD_HOLD` so the loading glyph is visible even when
    /// the fetch itself takes no time.
    fn hold_download_request(&mut self) {
        const DOWNLOAD_HOLD: std::time::Duration = std::time::Duration::from_millis(800);
        self.download_request_until = Some(std::time::Instant::now() + DOWNLOAD_HOLD);
        if !self.while_download_request {
            self.while_download_request = true;
            self.while_download_request_changed();
        }
    }

    /// Ends one song's request: immediately when the hold has passed, else
    /// after the remaining hold (through the runtime + invoker, so the write
    /// lands on the main thread).
    fn settle_download_request(&mut self, song_id: String) {
        let delay = self
            .download_request_until
            .map(|until| until.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or_default();
        if delay.is_zero() {
            self.apply_download_settle(song_id);
            return;
        }
        let invoker = self.get_qml_method_invoker();
        crate::runtime().spawn(async move {
            tokio::time::sleep(delay).await;
            qtbridge::invoke_method!(invoker, "settleDownloadRequest", song_id);
        });
    }

    /// The hold-expired landing pad (also the immediate path).
    #[qslot(qml_name = "settleDownloadRequest")]
    fn settle_download_request_now(&mut self, song_id: String) {
        self.apply_download_settle(song_id);
    }

    fn apply_download_settle(&mut self, song_id: String) {
        crate::browse::clear_offline_filter();
        let touched = self
            .mark_states
            .get(&song_id)
            .is_some_and(|state| *state == MarkState::SongInFlight);
        if touched {
            // The completion already wrote SongDone for finished fetches;
            // only a still-in-flight mark falls away here.
            self.patch_mark_state(&song_id, None);
        }
        if self.downloading_ids.is_empty() && self.while_download_request {
            self.while_download_request = false;
            self.while_download_request_changed();
        }
    }

    /// A finished song completed its album: that album's row flips, alone.
    #[qslot(qml_name = "markAlbumCached")]
    fn mark_album_cached(&mut self, album_id: String) {
        if album_id.is_empty() {
            return;
        }
        self.patch_mark_state(&album_id, Some(MarkState::AlbumCached));
    }

    /// The row-status refresh hook: finished download passes trigger it (the
    /// "download: … pass finished" path), and any surface may call it when
    /// its rows need reconciling. Re-reads the download window (marks) and
    /// re-emits the queue (row content changed outside the player) —
    /// favourites do not come through here anymore.
    #[qslot(qml_name = "TriggerRowReload")]
    fn trigger_row_reload(&mut self) {
        crate::browse::clear_offline_filter();
        self.refresh_download_window();
        // Row content changed outside the player (syncs, wipe reloads,
        // download settles): the revision gate must not swallow this
        // publish. Favourites take the per-row `favoriteChanged` path.
        self.queue_rows_dirty = true;
        self.emit_queue(false);
        // Every visible surface re-reads its *own* current window: a fetch
        // that finished must update the rows that show it, whatever the
        // request-dedupe or window unions say (the success side of the
        // fetch → row-update loop).
        self.row_reload();
    }

    /// Re-reads the stored window (after a download changed the ledger).
    fn refresh_download_window(&mut self) {
        // Re-read over the union of the recent requests: a download's
        // success must re-evaluate every surface that is showing rows —
        // album ids included, even when the last request was a songs one.
        if self.recent_mark_requests.is_empty() {
            return;
        }
        let songs: Vec<String> = self
            .recent_mark_requests
            .iter()
            .flat_map(|(songs, _)| songs.iter().cloned())
            .collect();
        let albums: Vec<String> = self
            .recent_mark_requests
            .iter()
            .flat_map(|(_, albums)| albums.iter().cloned())
            .collect();
        self.refresh_window(songs, albums);
    }

    /// One refresh read over explicit windows.
    fn refresh_window(&mut self, songs: Vec<String>, albums: Vec<String>) {
        // PATH-5: accumulate every request in the debounce window and read
        // their union once; the newest generation owns the apply.
        let (songs_snapshot, albums_snapshot) = {
            let mut pending = self
                .mark_pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            pending.songs.extend(songs);
            pending.albums.extend(albums);
            (
                pending.songs.iter().cloned().collect::<Vec<_>>(),
                pending.albums.iter().cloned().collect::<Vec<_>>(),
            )
        };
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let pending = std::sync::Arc::clone(&self.mark_pending);
        let generation = MARK_WINDOW_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        crate::runtime().spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            if MARK_WINDOW_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            apply_window(&dbs, &songs_snapshot, &albums_snapshot, generation, invoker).await;
            // A newer request owns the rest; only a still-current apply
            // retires its snapshot from the pending union.
            if MARK_WINDOW_GEN.load(Ordering::Relaxed) == generation {
                let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
                for id in &songs_snapshot {
                    pending.songs.remove(id);
                }
                for id in &albums_snapshot {
                    pending.albums.remove(id);
                }
            }
        });
    }

    #[qslot(qml_name = "requestQueue")]
    fn request_queue(&mut self) {
        // The explicit request always publishes (the drawer/entry may have
        // missed the automatic stream).
        self.emit_queue(true);
    }

    #[qslot(qml_name = "queueJump")]
    fn queue_jump(&mut self, index: i64) {
        self.player.jump(index_value(index));
        self.sync_player();
        self.emit_queue(false);
    }

    #[qslot(qml_name = "queueRemove")]
    fn queue_remove(&mut self, index: i64) {
        self.player.remove(index_value(index));
        self.sync_player();
        self.emit_queue(false);
    }

    #[qslot(qml_name = "queueMove")]
    fn queue_move(&mut self, from: i64, to: i64) {
        self.player.move_item(index_value(from), index_value(to));
        self.emit_queue(false);
    }

    // -----------------------------------------------------------------------
    // ARC-43 — Alt-hold quick actions
    // -----------------------------------------------------------------------

    /// Appends a serialized track list to the live queue (ARC-43 "Add to
    /// Queue"); unlike `playerLoad`, nothing starts playing.
    #[qslot(qml_name = "playerAppend")]
    fn player_append(&mut self, tracks_json: String) {
        match serde_json::from_str::<Vec<crate::player::Track>>(&tracks_json) {
            Ok(tracks) => {
                self.player.append(tracks);
                self.sync_player();
                self.emit_queue(false);
                self.queue_status_save();
            }
            Err(err) => {
                eprintln!("player: bad append payload: {err}");
                self.error_raised("playback".to_string(), format!("Bad queue payload: {err}"));
            }
        }
    }

    /// Appends one song or a whole album to the queue (overlay North).
    #[qslot(qml_name = "altAddToQueue")]
    fn alt_add_to_queue(&mut self, kind: String, id: String) {
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let offline = self.offline;
        crate::runtime().spawn(async move {
            match crate::queue::quick_tracks_json(&dbs, &kind, &id, offline).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "playerAppend", json);
                }
                Err(err) => {
                    eprintln!("alt actions: could not queue {kind} {id}: {err:#}");
                    raise(
                        &error_invoker,
                        "playback",
                        format!("Could not add to queue: {err:#}"),
                    );
                }
            }
        });
    }

    /// VI-4: Alt "Add to Queue" on a visual row selection — every selected id
    /// is resolved and appended as one batch.
    #[qslot(qml_name = "queueAddSelection")]
    fn queue_add_selection(&mut self, kind: String, ids_json: String) {
        let ids: Vec<String> = match serde_json::from_str(&ids_json) {
            Ok(ids) => ids,
            Err(err) => {
                eprintln!("alt actions: bad selection payload: {err}");
                self.error_raised(
                    "playback".to_string(),
                    format!("Bad selection payload: {err}"),
                );
                return;
            }
        };
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::dbs().cloned() else {
            return;
        };
        let offline = self.offline;
        crate::runtime().spawn(async move {
            match crate::queue::quick_tracks_json_many(&dbs, &kind, &ids, offline).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "playerAppend", json);
                }
                Err(err) => {
                    eprintln!("alt actions: could not queue the selection ({kind}): {err:#}");
                    raise(
                        &error_invoker,
                        "playback",
                        format!("Could not add to queue: {err:#}"),
                    );
                }
            }
        });
    }

    /// The Alt overlay's Download verb under a visual selection (VI-4):
    /// a toggle like the ArtistEntry chip — a selection whose targets all
    /// hold their content is removed, anything missing downloads (songs
    /// explicitly; albums/artists as rule containers) through the shared
    /// fetch pass with the per-id row feedback.
    #[qslot(qml_name = "downloadSelection")]
    fn download_selection(&mut self, kind: String, ids_json: String) {
        let ids: Vec<String> = match serde_json::from_str(&ids_json) {
            Ok(ids) => ids,
            Err(err) => {
                eprintln!("alt actions: bad download selection payload: {err}");
                self.error_raised(
                    "playback".to_string(),
                    format!("Bad selection payload: {err}"),
                );
                return;
            }
        };
        if ids.is_empty() {
            return;
        }
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        let outer_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            if let Err(err) = selection_toggle(&dbs, &kind, &ids, invoker, error_invoker).await {
                eprintln!("download: selection ({kind}) failed: {err:#}");
                raise(
                    &outer_invoker,
                    "error",
                    format!("Could not download the selection: {err:#}"),
                );
            } else {
                eprintln!("download: selection ({kind}) done");
            }
        });
    }

    /// Favourites one song or album on the server (overlay South).
    #[qslot(qml_name = "altFavorite")]
    fn alt_favorite(&mut self, kind: String, id: String, on: bool) {
        let error_invoker = self.get_qml_method_invoker();
        let state_invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            let result = async {
                let client = client(&dbs).await?;
                let (songs, albums, artists): (Vec<&str>, Vec<&str>, Vec<&str>) =
                    match kind.as_str() {
                        "song" => (vec![id.as_str()], vec![], vec![]),
                        "album" => (vec![], vec![id.as_str()], vec![]),
                        "artist" => (vec![], vec![], vec![id.as_str()]),
                        other => return Err(anyhow::anyhow!("unknown favourite kind: {other}")),
                    };
                // ARC-I-4: the verb is now a real toggle — `on` picks star or
                // unstar instead of always starring.
                if on {
                    client.star(&songs, &albums, &artists).await?
                } else {
                    client.unstar(&songs, &albums, &artists).await?
                }
                // The local half: the hearts read these flags; the next sync
                // reconciles the exact server state (songs/albums →
                // library.db, artists → artist_user.db).
                match kind.as_str() {
                    "song" => crate::library::set_song_starred(&dbs.library, &id, on).await,
                    "album" => crate::library::set_album_starred(&dbs.library, &id, on).await,
                    _ => crate::artist_user::set_starred(&dbs.artist_user, &id, on).await,
                }
            }
            .await;
            if result.is_ok() {
                if kind == "album" {
                    // The fullscreen keybar heart reconciles through this.
                    let rating = crate::library::album_rating(&dbs.library, &id)
                        .await
                        .unwrap_or(0);
                    let json = album_state_payload(&id, on, rating);
                    emit(&state_invoker, "album_state", json);
                }
                // Per-row reconcile: the queue patches every matching row in
                // place. The old full `TriggerRowReload` re-emitted the queue
                // payload, whose apply rebuilt the models and re-followed the
                // playing index — favouriting stole the cursor.
                qtbridge::invoke_method!(
                    state_invoker,
                    "favoriteChanged",
                    kind.clone(),
                    id.clone(),
                    on
                );
            }
            if let Err(err) = result {
                eprintln!("alt actions: could not update favourite {kind} {id}: {err:#}");
                raise(
                    &error_invoker,
                    "error",
                    format!("Could not update the favourite: {err:#}"),
                );
            }
        });
    }

    /// Opens the album page owning a track (overlay East on track rows).
    #[qslot(qml_name = "altGoToAlbum")]
    fn alt_go_to_album(&mut self, song_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            match crate::queue::song_album_page_json(&dbs, &song_id).await {
                Ok(json) => {
                    emit(&invoker, "open_album", json);
                }
                Err(err) => {
                    eprintln!("alt actions: could not open the album for {song_id}: {err:#}");
                }
            }
        });
    }

    /// Opens the artist page owning an album (overlay East on album rows).
    #[qslot(qml_name = "altGoToArtist")]
    fn alt_go_to_artist(&mut self, album_id: String) {
        let invoker = self.get_qml_method_invoker();
        self.spawn_task(move |dbs| async move {
            match crate::queue::album_artist_page_json(&dbs, &album_id).await {
                Ok(json) => {
                    emit(&invoker, "open_artist", json);
                }
                Err(err) => {
                    eprintln!("alt actions: could not open the artist for {album_id}: {err:#}");
                }
            }
        });
    }

    /// Seeks to an absolute position (seconds).
    #[qslot(qml_name = "playerSeek")]
    fn player_seek(&mut self, seconds: f64) {
        self.player.seek(seconds);
        self.sync_player();
        self.seeked();
    }

    /// Sets the volume (0-100). Adjusting up unmutes.
    #[qslot(qml_name = "playerSetVolume")]
    fn player_set_volume(&mut self, volume: f64) {
        if volume > self.player.volume() && self.player.muted() {
            self.player.set_muted(false);
        }
        self.player.set_volume(volume);
        self.sync_player();
        self.queue_status_save();
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
        self.queue_status_save();
    }

    /// Mutes or unmutes (M).
    #[qslot(qml_name = "playerToggleMute")]
    fn player_toggle_mute(&mut self) {
        self.player.toggle_mute();
        self.sync_player();
        self.queue_status_save();
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
            let client = match client(&dbs).await {
                Ok(client) => Some(client),
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
            emit(&invoker, "lyrics", document.to_json());

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
            emit(&invoker, "colors", payload);
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
        // MPRIS controls travel as commands; run them through the same
        // dispatcher as keys/IPC, even while stopped (Q-8).
        for command in crate::mpris::drain() {
            self.command_issued(command);
        }
        if self.player.state() == State::Stopped {
            let mut state = self.mpris_snapshot();
            state.playback_status = "Stopped".to_string();
            crate::mpris::publish(state);
            return;
        }
        let index = self.player.index();
        if let Some(message) = self.player.pump() {
            self.error_raised("playback".to_string(), message);
        }
        if self.player.index() != index {
            self.emit_queue(false);
        }
        self.sync_player();
        self.maybe_save_position();
    }

    /// Publishes the player state to the QML properties.
    fn sync_player(&mut self) {
        // ARC-10: only emit when a value actually changed — the 500 ms tick
        // must not republish every string on every frame. Compare first (no
        // clones), clone only the fields that changed.
        macro_rules! sync_current {
            ($field:ident, $signal:ident, $field_name:ident) => {{
                let value = self
                    .player
                    .current()
                    .map(|track| track.$field_name.as_str())
                    .unwrap_or_default();
                let changed = self.$field.as_str() != value;
                if changed {
                    self.$field = self
                        .player
                        .current()
                        .map(|track| track.$field_name.clone())
                        .unwrap_or_default();
                    self.$signal();
                }
                changed
            }};
        }

        if sync_current!(current_song_id, current_song_id_changed, id) {
            // P-6 S2: starts, skips and auto-advance all count as "last
            // song"; clearing the track keeps the previous one.
            let song_id = self.current_song_id.clone();
            self.remember_song(&song_id);
        }
        sync_current!(current_album_id, current_album_id_changed, album_id);
        sync_current!(current_title, current_title_changed, title);
        sync_current!(current_artist, current_artist_changed, artist);
        sync_current!(current_album, current_album_changed, album);
        sync_current!(current_cover_art, current_cover_art_changed, cover_art);
        sync_current!(current_cover_url, current_cover_url_changed, cover_url);

        macro_rules! sync_field {
            ($field:ident, $signal:ident, $value:expr) => {{
                let value = $value;
                if self.$field != value {
                    self.$field = value;
                    self.$signal();
                }
            }};
        }

        sync_field!(
            playback_state,
            playback_state_changed,
            self.player.state().name().to_string()
        );
        sync_field!(duration, duration_changed, self.player.duration());
        sync_field!(position, position_changed, self.player.position());
        sync_field!(volume, volume_changed, self.player.volume());
        sync_field!(muted, muted_changed, self.player.muted());
        sync_field!(queue_index, queue_index_changed, self.player.index() as i64);
        sync_field!(queue_length, queue_length_changed, self.player.len() as i64);

        // Q-8: mirror the player into the native MPRIS service.
        crate::mpris::publish(self.mpris_snapshot());
    }

    /// Q-8: current player state in MPRIS terms (volume 0.0..=1.0).
    fn mpris_snapshot(&self) -> crate::mpris::Snapshot {
        crate::mpris::Snapshot {
            playback_status: self.player.state().mpris_status().to_string(),
            title: self.current_title.clone(),
            artist: self.current_artist.clone(),
            album: self.current_album.clone(),
            track_id: self.current_song_id.clone(),
            duration_us: (self.duration * 1_000_000.0) as i64,
            position_us: (self.position * 1_000_000.0) as i64,
            volume: if self.muted {
                0.0
            } else {
                (self.volume / 100.0).clamp(0.0, 1.0)
            },
            can_next: self.queue_index >= 0 && self.queue_index + 1 < self.queue_length,
            can_prev: self.queue_index > 0,
            art_url: self.current_cover_url.clone(),
        }
    }
}
