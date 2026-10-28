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
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use qtbridge::{QmlObject, qobject};
use serde::ser::{Serialize, SerializeSeq, SerializeStruct, Serializer};

use crate::browse::{
    load_album_songs_json, load_albums_json, load_artist_json, load_artists_json,
    load_songs_page_json, search_json,
};
use crate::keybind::{Keymap, Outcome};
use crate::player::{Player, State};
use crate::queue::{album_tracks_json, library_queue_payload, song_album_payload};

#[path = "qml_backend/artist.rs"]
mod artist;
#[path = "qml_backend/auth.rs"]
mod auth;
#[path = "qml_backend/download.rs"]
pub(crate) mod download;
#[path = "qml_backend/library.rs"]
mod library;
#[path = "qml_backend/marks.rs"]
mod marks;
#[path = "qml_backend/player.rs"]
mod player;
#[path = "qml_backend/support.rs"]
mod support;
#[path = "qml_backend/sync.rs"]
mod sync;

use crate::session::client;
use download::selection_toggle;
use support::{album_state_payload, emit, raise};
use sync::{index_value, spawn_sync};

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

/// FIX-4: one generation per data channel (`spawn_data`); a newer request
/// owns the channel and any load still running under an older generation
/// drops its payload. Poison carries no invariant (a monotonic counter), so
/// the lock recovers instead of panicking.
static DATA_GEN: LazyLock<Mutex<HashMap<&'static str, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn bump_data_gen(channel: &'static str) -> u64 {
    let mut map = DATA_GEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let generation = map.entry(channel).or_default();
    *generation += 1;
    *generation
}

fn data_gen(channel: &'static str) -> u64 {
    DATA_GEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel)
        .copied()
        .unwrap_or_default()
}

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

/// Plural word for the queue clipboard toasts.
fn rows(count: usize) -> &'static str {
    if count == 1 { "row" } else { "rows" }
}

/// The hash of the last published queue rows ("RowAlreadyPublished"): the
/// automatic emits dedupe on it so skips don't republish the same rows.
static QUEUE_ROWS_HASH: AtomicU64 = AtomicU64::new(0);

/// One queue row, streamed straight from the player's queue. The queue can
/// hold the whole library, so building a `Value` map per track before
/// serializing doubled the transient payload cost.
struct QueueRow<'a> {
    track: &'a crate::player::Track,
    starred: bool,
}

impl Serialize for QueueRow<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("queue_row", 10)?;
        state.serialize_field("id", &self.track.id)?;
        state.serialize_field("title", &self.track.title)?;
        state.serialize_field("album_id", &self.track.album_id)?;
        state.serialize_field("starred", &self.starred)?;
        state.serialize_field("duration", &self.track.duration)?;
        state.serialize_field("cover", &self.track.cover_url)?;
        state.serialize_field("subtitle", &self.track.artist)?;
        state.serialize_field(
            "meta",
            &crate::browse::rows::format_duration(Some(self.track.duration as i32)),
        )?;
        state.serialize_field("songId", &self.track.id)?;
        state.serialize_field("albumId", &self.track.album_id)?;
        state.end()
    }
}

pub(crate) struct QueueRows<'a> {
    pub(crate) queue: &'a [crate::player::Track],
    pub(crate) starred: &'a std::collections::HashSet<String>,
}

impl Serialize for QueueRows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.queue.len()))?;
        for track in self.queue {
            seq.serialize_element(&QueueRow {
                track,
                starred: self.starred.contains(&track.id),
            })?;
        }
        seq.end()
    }
}

/// The `applyQueueCovers` payload, deserialized straight into its final
/// collections.
///
/// The old handler parsed a `Value` and then cloned its `covers`/`albums`/
/// `songs` subtrees before deserializing them; for a library-wide queue that
/// meant three extra copies of the whole payload on the UI thread.
#[derive(Default, serde::Deserialize)]
pub(crate) struct QueueCoversPayload {
    #[serde(default)]
    pub(crate) covers: HashMap<String, String>,
    #[serde(default)]
    pub(crate) albums: HashMap<String, String>,
    #[serde(default)]
    pub(crate) songs: Vec<crate::library::SongRow>,
}

/// Producer twin of [`QueueCoversPayload`]: writes the three collections
/// straight into the payload string instead of building a `Value` tree of the
/// whole queue first.
#[derive(serde::Serialize)]
pub(crate) struct QueueCoversEnvelope<'a> {
    pub(crate) covers: &'a HashMap<String, String>,
    pub(crate) albums: &'a HashMap<String, String>,
    pub(crate) songs: &'a [crate::library::SongRow],
}

/// The download-window payload (`applyDownloadedIds`), same rationale.
#[derive(Default, serde::Deserialize)]
struct DownloadedIdsPayload {
    #[serde(default)]
    songs: Vec<String>,
    #[serde(default)]
    albums: Vec<String>,
    #[serde(default)]
    downloading: Vec<String>,
    #[serde(default)]
    queried_songs: Vec<String>,
    #[serde(default)]
    queried_albums: Vec<String>,
}

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
    /// FIX-5: the song id whose lyrics are being fetched; repeated requests
    /// skip while it matches, and `lyricsReady` clears it on delivery.
    lyrics_in_flight: Option<String>,
    current_title: String,
    current_artist: String,
    current_album: String,
    current_cover_art: String,
    current_cover_url: String,
    current_song_id: String,
    current_song_starred: bool,
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
    /// 0 = off, 1 = repeat one, 2 = repeat queue.
    repeat_mode: i64,
    position: f64,
    duration: f64,
    volume: f64,
    /// Playback envelope published for the fluid backdrop (0..1).
    audio_level: f64,
    audio_peak: f64,
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
    /// VI-8a: rows in the queue clipboard (0 = nothing yanked).
    clipboard_count: i64,
    /// VI-8a: the universal row clipboard — kind (`song`/`album`/`artist`)
    /// and ids in yank order (duplicates preserved). Paste resolves it into
    /// the queue only.
    clipboard_kind: String,
    clipboard_ids: Vec<String>,
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
        "currentSongStarred",
        Member = current_song_starred,
        Notify = current_song_starred_changed
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
    qproperty!(
        "repeatMode",
        Member = repeat_mode,
        Notify = repeat_mode_changed
    );
    qproperty!("position", Member = position, Notify = position_changed);
    qproperty!("duration", Member = duration, Notify = duration_changed);
    qproperty!("volume", Member = volume, Notify = volume_changed);
    qproperty!("audioLevel", Member = audio_level, Notify = audio_level_changed);
    qproperty!("audioPeak", Member = audio_peak, Notify = audio_peak_changed);
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
        "clipboardCount",
        Member = clipboard_count,
        Notify = clipboard_count_changed
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
        F: FnOnce(Arc<crate::db::Dbs>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        crate::app::runtime().spawn(task(dbs));
    }

    fn spawn_data<F, Fut>(&self, channel: &'static str, load: F)
    where
        F: FnOnce(Arc<crate::db::Dbs>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<String>> + Send,
    {
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        // FIX-4: one generation per channel, bumped by every request — a
        // newer order/filter/page owns the channel, so a load still running
        // under an older generation never emits (SEARCH_GEN pattern).
        let generation = bump_data_gen(channel);
        crate::app::runtime().spawn(async move {
            if data_gen(channel) != generation {
                return;
            }
            match load(dbs).await {
                Ok(json) => {
                    if data_gen(channel) == generation {
                        emit(&invoker, channel, json);
                    }
                }
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
    fn current_song_starred_changed(&mut self);
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
    /// VI-8a: neutral log line (the `ToastModal` banner): queue yank/paste
    /// and future informational notices. Errors keep `errorRaised`.
    #[qsignal(qml_name = "noticeRaised")]
    fn notice_raised(&mut self, message: String);
    #[qsignal(qml_name = "commandIssued")]
    fn command_issued(&mut self, command: String);
    #[qsignal(qml_name = "loginRequired")]
    fn login_required(&mut self);
    #[qsignal]
    fn playback_state_changed(&mut self);
    #[qsignal(qml_name = "repeatModeChanged")]
    fn repeat_mode_changed(&mut self);
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
    fn audio_level_changed(&mut self);
    #[qsignal]
    fn audio_peak_changed(&mut self);
    #[qsignal]
    fn muted_changed(&mut self);
    #[qsignal]
    fn queue_index_changed(&mut self);
    #[qsignal]
    fn queue_length_changed(&mut self);
    #[qsignal]
    fn clipboard_count_changed(&mut self);
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
        self.login_impl();
    }
    #[qslot]
    fn startup(&mut self) {
        self.startup_impl();
    }

    /// Debug only: the first rendered frame (no-op in release).
    #[qslot(qml_name = "frontendReady")]
    fn frontend_ready(&mut self) {
        #[cfg(debug_assertions)]
        crate::debug_info::since_start("frontend.ready");
    }

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
        crate::session::set_online(false);
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
        crate::session::set_online(true);
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let server_url = self.server_url.trim().to_string();
        let username = self.username.trim().to_string();
        let offline = self.offline;
        let invoker = self.get_qml_method_invoker();
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        // PATH-7: take a COW snapshot here and serialize it in the worker
        // after the debounce wins — never on the Qt thread, and never one
        // `String` clone per queued id on every status write (FIX-25).
        let queue = self.player.queue_snapshot();
        let status = self.session_status(String::new());
        crate::app::runtime().spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if STATUS_WRITE_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            let mut status = status;
            let ids: Vec<&str> = queue.iter().map(|track| track.id.as_str()).collect();
            status.queue = serde_json::to_string(&ids).unwrap_or_default();
            if let Err(err) = crate::status::save(&dbs.status, &status).await {
                eprintln!("status: could not save the session: {err:#}");
            }
        });
    }

    /// Last-chance write on window close (P-6): the debounce task cannot be
    /// awaited during teardown, so this single local upsert blocks the Qt
    /// thread for the few ms it takes.
    #[qslot(qml_name = "flushStatus")]
    fn flush_status(&mut self) {
        if !self.logged_in || self.server_url.trim().is_empty() || self.username.trim().is_empty() {
            return;
        }
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        // Cancel a pending debounce so it cannot race (or overwrite) this.
        STATUS_WRITE_GEN.fetch_add(1, Ordering::Relaxed);
        let status = self.session_status(self.queue_ids_json());
        if let Err(err) = crate::app::runtime().block_on(crate::status::save(&dbs.status, &status))
        {
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
        let Some(dbs) = crate::app::dbs() else {
            qtbridge::invoke_method!(
                finish_invoker,
                "librarySyncFinished",
                "Library index is not available.".to_string()
            );
            return;
        };

        crate::app::runtime().spawn(async move {
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
    #[qslot(qml_name = "wipeDatabases")]
    fn wipe_databases(&mut self, scope: String) {
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        self.library_sync_started();
        self.sync_message = format!("Wiping {scope}…");
        self.sync_message_changed();
        crate::app::runtime().spawn(async move {
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
                                crate::app::set_dbs(fresh);
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
            crate::app::runtime().spawn(async { crate::session::invalidate().await });
            self.player.stop();
            self.sync_player();
            self.reset_colors();
            crate::session::set_online(false);
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
        // One deserialization into the final maps/rows: no intermediate
        // `Value`, no subtree clones (FIX-33).
        let payload: QueueCoversPayload = crate::cache::parse_or_default(&covers_json);
        self.player.refresh_track_metadata(&payload.songs);
        self.player
            .refresh_cover_arts(&payload.albums, &payload.covers);
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
        #[cfg(debug_assertions)]
        let started = std::time::Instant::now();
        self.spawn_data("albums", move |dbs| async move {
            let payload = load_albums_json(&dbs, &artist, &order, offline).await?;
            #[cfg(debug_assertions)]
            {
                let rows = serde_json::from_str::<serde_json::Value>(&payload)
                    .ok()
                    .and_then(|value| value.as_array().map(Vec::len))
                    .unwrap_or(0);
                crate::debug_info::albums_loaded(started, rows, payload.len());
            }
            Ok(payload)
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
        self.star_artist_impl(artist_id, on);
    }

    /// Sets the artist rating (0..=5) on the server, then indexes it locally
    /// in `artist_user.db`. The UI toggles optimistically and reverts on the
    /// error toast.
    #[qslot(qml_name = "rateArtist")]
    fn rate_artist(&mut self, artist_id: String, rating: i32) {
        self.rate_artist_impl(artist_id, rating);
    }

    /// Fetches the artist biography (`getArtistInfo2` → `getArtistInfo`
    /// fallback), caches it in `library.db` (`artist_info`) and emits
    /// `artist_info`; offline serves the cached row.
    #[qslot(qml_name = "requestArtistInfo")]
    fn request_artist_info(&mut self, artist_id: String) {
        self.request_artist_info_impl(artist_id);
    }
    /// CACHE-1/8: an explicit user download. Idempotent — a cached blob or an
    /// in-flight fetch short-circuits; otherwise marks the ledger, fetches the
    /// original file, stores it and flips the ledger to `done` (or `error`
    /// with the reason). Emits the `download` state and an error toast.
    #[qslot(qml_name = "downloadSong")]
    fn download_song(&mut self, song_id: String) {
        self.download_song_impl(song_id);
    }

    /// The stored artist-container selection (`artist_cache` channel).
    #[qslot(qml_name = "requestArtistCache")]
    fn request_artist_cache(&mut self, artist_id: String) {
        self.request_artist_cache_impl(artist_id);
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
        self.download_artist_impl(artist_id, on);
    }

    /// The album container state for the fullscreen keybar's Download button
    /// (`album_cache`): an album rule, or an artist rule covering the album.
    #[qslot(qml_name = "requestAlbumCache")]
    fn request_album_cache(&mut self, album_id: String) {
        self.request_album_cache_impl(album_id);
    }

    /// The album favourite flag for the fullscreen keybar's heart.
    #[qslot(qml_name = "requestAlbumState")]
    fn request_album_state(&mut self, album_id: String) {
        self.request_album_state_impl(album_id);
    }

    /// UI-60: sets an album's 0..=5 user rating on the server, then indexes
    /// it locally. The overlay updates the badge optimistically; the
    /// `ratingChanged` signal patches every visible row and the
    /// `album_state` payload reconciles the album header/keybar (also on
    /// failure, with the stored value).
    #[qslot(qml_name = "rateAlbum")]
    fn rate_album(&mut self, album_id: String, rating: i32) {
        self.rate_album_impl(album_id, rating);
    }

    /// The album twin of [`downloadArtist`] — the keybar's Download button:
    /// ON marks and fetches the album's songs, unless an artist rule already
    /// covers them (then the request is skipped — "if the artist is already
    /// in download scope, don't"); OFF drops the rule, its marks and its
    /// rule-owned blobs. Emits `album_cache` throughout.
    #[qslot(qml_name = "downloadAlbum")]
    fn download_album(&mut self, album_id: String, on: bool) {
        self.download_album_impl(album_id, on);
    }

    #[qslot(qml_name = "requestAlbumSongs")]
    fn request_album_songs(&mut self, album_id: String) {
        let offline = self.offline;
        self.spawn_data("album_songs", move |dbs| async move {
            load_album_songs_json(&dbs, &album_id, offline).await
        });
    }

    /// Resolves the album's owning artist id for the header's artist link.
    #[qslot(qml_name = "requestAlbumArtist")]
    fn request_album_artist(&mut self, album_id: String) {
        self.spawn_data("album_artist", move |dbs| async move {
            let artist_id = crate::library::album_artist_id(&dbs.library, &album_id).await?;
            Ok(serde_json::json!({
                "album_id": album_id,
                "artist_id": artist_id,
            })
            .to_string())
        });
    }

    /// Fetches an album's description ("notes") from the server.
    #[qslot(qml_name = "requestAlbumInfo")]
    fn request_album_info(&mut self, album_id: String) {
        self.request_album_info_impl(album_id);
    }

    /// ARC-UI-7.5: hydrate the Rust-owned scroll store once.
    #[qslot(qml_name = "scrollHydrate")]
    fn scroll_hydrate(&mut self) {
        crate::scroll_memory::hydrate();
    }

    #[qslot(qml_name = "scrollRemember")]
    fn scroll_remember(&mut self, key: String, index: i64, y: f64, scope: i64, layout: String) {
        crate::scroll_memory::remember(&key, index, y, scope, &layout);
    }

    #[qslot(qml_name = "scrollRememberLayout")]
    fn scroll_remember_layout(&mut self, key: String, layout: String) {
        crate::scroll_memory::remember_layout(&key, &layout);
    }

    #[qslot(qml_name = "scrollEntry")]
    fn scroll_entry(&self, key: String) -> String {
        crate::scroll_memory::entry_json(&key)
    }

    #[qslot(qml_name = "scrollSavedIndex")]
    fn scroll_saved_index(&self, key: String) -> i64 {
        crate::scroll_memory::saved_index(&key)
    }

    #[qslot(qml_name = "scrollSavedLayout")]
    fn scroll_saved_layout(&self, key: String) -> String {
        crate::scroll_memory::saved_layout(&key)
    }

    #[qslot(qml_name = "scrollClear")]
    fn scroll_clear(&mut self, key: String) {
        crate::scroll_memory::clear(&key);
    }

    #[qslot(qml_name = "scrollFlush")]
    fn scroll_flush(&mut self) {
        let payload = crate::scroll_memory::serialize();
        self.write_scroll_memory(payload);
    }

    /// ARC-UI-7.5: the persisted scroll store (compatibility read).
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
        self.write_scroll_memory(payload);
    }

    fn write_scroll_memory(&self, payload: String) {
        let Ok(path) = crate::db::scroll_path() else {
            return;
        };
        let generation = SCROLL_WRITE_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let offline = self.offline;
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
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
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let offline = self.offline;
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let order = crate::library::effective_order(&crate::library::ORDERS, &self.song_order, 0)
            .to_string();
        let offline = self.offline;
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let offline = self.offline;
        crate::app::runtime().spawn(async move {
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

    #[qslot(qml_name = "cycleRepeat")]
    fn cycle_repeat(&mut self) {
        let mode = self.player.cycle_repeat().code();
        if self.repeat_mode != mode {
            self.repeat_mode = mode;
            self.repeat_mode_changed();
        }
    }

    #[qslot(qml_name = "noteIdle")]
    fn note_idle(&mut self, idle: bool) {
        #[cfg(debug_assertions)]
        crate::debug_info::idle(idle);
    }

    #[qslot(qml_name = "debugBuild")]
    fn debug_build(&self) -> bool {
        cfg!(debug_assertions)
    }

    #[qslot(qml_name = "playerNext")]
    fn player_next(&mut self) {
        self.player.next();
        self.sync_player();
        self.emit_queue(false);
        self.track_skipped(true);
    }

    #[qslot(qml_name = "playerNextBy")]
    fn player_next_by(&mut self, count: i64) {
        let steps = (count.max(1) as usize).min(self.player.queue().len().max(1));
        for _ in 0..steps {
            self.player.next();
        }
        self.sync_player();
        self.emit_queue(false);
        self.track_skipped(true);
    }

    #[qslot(qml_name = "playerPreviousBy")]
    fn player_previous_by(&mut self, count: i64) {
        let steps = (count.max(1) as usize).min(self.player.queue().len().max(1));
        for _ in 0..steps {
            self.player.previous();
        }
        self.sync_player();
        self.emit_queue(false);
        self.track_skipped(false);
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
    /// marks' truth (the download twin of the hearts' library read). Called
    /// whenever the rendered-row window changes, never for the whole library.
    #[qslot(qml_name = "requestDownloads")]
    fn request_downloads(&mut self, songs: String, albums: String) {
        let songs: Vec<String> = crate::cache::parse_or_default(&songs);
        let albums: Vec<String> = crate::cache::parse_or_default(&albums);
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
        let songs: Vec<String> = crate::cache::parse_or_default(&songs);
        let albums: Vec<String> = crate::cache::parse_or_default(&albums);
        self.store_and_refresh(songs, albums);
    }

    /// Stores the window (and its request, for the union refreshes) and
    /// starts the read.
    /// Applies a window read on the main thread (the async reader's landing
    /// pad, invoked through the QML method invoker like `applyStatus`). The
    /// window's *queried* ids get their states rewritten; ids outside it are
    /// untouched — one surface's read can never erase another surface's
    /// marks, nor a per-id patch (the old union was clobbered by exactly
    /// that, e.g. a favourite's reload hiding a finished download).
    #[qslot(qml_name = "applyDownloadedIds")]
    fn apply_downloaded_ids(&mut self, ids_json: String) {
        // One typed deserialization: a window can be a page of ids, and this
        // runs per reload — no `Value` tree and no per-array clone (FIX-33).
        let payload: DownloadedIdsPayload = crate::cache::parse_or_default(&ids_json);
        let songs: std::collections::HashSet<String> = payload.songs.into_iter().collect();
        let downloading: std::collections::HashSet<String> =
            payload.downloading.into_iter().collect();
        let albums: std::collections::HashSet<String> = payload.albums.into_iter().collect();
        for id in payload.queried_songs {
            let state = if songs.contains(&id) {
                Some(MarkState::SongDone)
            } else if downloading.contains(&id) {
                Some(MarkState::SongInFlight)
            } else {
                None
            };
            self.set_mark_state(&id, state);
        }
        for id in payload.queried_albums {
            let state = albums.contains(&id).then_some(MarkState::AlbumCached);
            self.set_mark_state(&id, state);
        }
        self.rebuild_mark_sets();
    }

    /// Writes one id's state (None removes it).
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
        let ids: Vec<String> = crate::cache::parse_or_default(&ids_json);
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
    /// The hold-expired landing pad (also the immediate path).
    #[qslot(qml_name = "settleDownloadRequest")]
    fn settle_download_request_now(&mut self, song_id: String) {
        self.apply_download_settle(song_id);
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
        crate::offline::clear_cache();
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
        let index = index_value(index);
        if index >= self.player.len() {
            return;
        }
        self.player.remove(index);
        self.sync_player();
        self.emit_queue(false);
        self.notice_raised(format!("(Deleted!) 1 {}", rows(1)));
    }

    /// VI-4: removes a visual selection in one pass, one payload.
    #[qslot(qml_name = "queueRemoveMany")]
    fn queue_remove_many(&mut self, indices_json: String) {
        let removed = match serde_json::from_str::<Vec<usize>>(&indices_json) {
            Ok(indices) => self.player.remove_many(&indices),
            Err(err) => {
                eprintln!("queue: bad remove selection: {err}");
                return;
            }
        };
        if removed == 0 {
            return;
        }
        self.sync_player();
        self.emit_queue(false);
        self.notice_raised(format!("(Deleted!) {removed} {}", rows(removed)));
    }

    /// VI-8a: yanks a list/queue row range into the universal clipboard.
    /// `kind` is `song`/`album`/`artist`; ids keep their order and duplicates.
    #[qslot(qml_name = "yankRows")]
    fn yank_rows(&mut self, kind: String, ids_json: String) {
        if !matches!(kind.as_str(), "song" | "album" | "artist") {
            eprintln!("queue: bad yank kind: {kind}");
            return;
        }
        let ids: Vec<String> = match serde_json::from_str(&ids_json) {
            Ok(ids) => ids,
            Err(err) => {
                eprintln!("queue: bad yank selection: {err}");
                return;
            }
        };
        let ids: Vec<String> = ids.into_iter().filter(|id| !id.is_empty()).collect();
        if ids.is_empty() {
            return;
        }
        let count = ids.len();
        self.clipboard_kind = kind;
        self.clipboard_ids = ids;
        self.clipboard_count = count as i64;
        self.clipboard_count_changed();
        self.notice_raised(format!("(Yanked!) {count} {}", rows(count)));
    }

    /// VI-8a: resolves the row clipboard and pastes it at the queue cursor
    /// (`above` = vim `P`). Paste targets only the queue drawer; resolution
    /// is the same kind expansion the Alt "Add to Queue" verb uses.
    #[qslot(qml_name = "pasteQueueRows")]
    fn paste_queue_rows(&mut self, cursor: i64, above: bool) {
        if self.clipboard_ids.is_empty() {
            return;
        }
        let kind = self.clipboard_kind.clone();
        let ids = self.clipboard_ids.clone();
        let offline = self.offline;
        let invoker = self.get_qml_method_invoker();
        let error_invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        crate::app::runtime().spawn(async move {
            match crate::queue::quick_tracks_json_many(&dbs, &kind, &ids, offline).await {
                Ok(json) => {
                    qtbridge::invoke_method!(invoker, "pasteQueueTracks", cursor, above, json);
                }
                Err(err) => raise(
                    &error_invoker,
                    "playback",
                    format!("Could not paste: {err:#}"),
                ),
            }
        });
    }

    /// Completes a queue paste once the clipboard rows resolved to tracks.
    #[qslot(qml_name = "pasteQueueTracks")]
    fn paste_queue_tracks(&mut self, cursor: i64, above: bool, tracks_json: String) {
        let tracks: Vec<crate::player::Track> = match serde_json::from_str(&tracks_json) {
            Ok(tracks) => tracks,
            Err(err) => {
                eprintln!("queue: bad paste payload: {err}");
                return;
            }
        };
        if tracks.is_empty() {
            return;
        }
        let count = tracks.len();
        let at = index_value(cursor) + usize::from(!above);
        self.player.insert_many(at, tracks);
        self.sync_player();
        self.emit_queue(false);
        self.notice_raised(format!("(Pasted!) {count} {}", rows(count)));
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let offline = self.offline;
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let offline = self.offline;
        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let Some(song_id) = self.player.current().map(|track| track.id.clone()) else {
            return;
        };
        // FIX-5: repeated requests for the same song coalesce — the fetch's
        // delivery clears the marker (`lyricsReady`), so a later song change
        // re-arms it. Prefetch ids dedupe the same way.
        if self.lyrics_in_flight.as_deref() == Some(song_id.as_str()) {
            return;
        }
        self.lyrics_in_flight = Some(song_id.clone());
        // LYR-5: warm the cache for the next few queue tracks, bounded.
        let upcoming = self.player.upcoming(3);

        crate::app::runtime().spawn(async move {
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
            qtbridge::invoke_method!(invoker, "lyricsReady", document.song_id.clone());

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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let Some(song_id) = self.player.current().map(|track| track.id.clone()) else {
            return;
        };
        // FIX-5: the manual pass must be able to re-request, so the deliver
        // marker of the first fetch does not suppress it.
        self.lyrics_in_flight = None;

        crate::app::runtime().spawn(async move {
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
        let Some(dbs) = crate::app::dbs() else {
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

        crate::app::runtime().spawn(async move {
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

    /// Clears the in-flight marker once a lyric job has been delivered (FIX-5).
    #[qslot(qml_name = "lyricsReady")]
    fn lyrics_ready(&mut self, song_id: String) {
        if self.lyrics_in_flight.as_deref() == Some(song_id.as_str()) {
            self.lyrics_in_flight = None;
        }
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
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let cover = self.current_cover_art.clone();
        if cover.is_empty() {
            return;
        }
        self.colors_in_flight = None;

        crate::app::runtime().spawn(async move {
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
    /// UI-facing audio envelope pump: the fullscreen backdrop polls this at
    /// ~25 Hz while it is visible.
    #[qslot(qml_name = "audioTick")]
    fn audio_tick(&mut self) {
        let mut level = self.player.audio_level();
        let mut peak = self.player.audio_peak();
        if self.player.state() != State::Playing {
            level *= 0.82;
            peak *= 0.82;
        }
        if (self.audio_level - level).abs() > 0.002 {
            self.audio_level = level;
            self.audio_level_changed();
        }
        if (self.audio_peak - peak).abs() > 0.002 {
            self.audio_peak = peak;
            self.audio_peak_changed();
        }
    }

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
