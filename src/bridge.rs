// Musika — GPUI backend bridge (G1).
//!
//! UI thread never touches tokio/DBs directly. Requests go to the shared
//! runtime and return as plain `Event`s over a tokio channel; the GPUI root
//! task drains them and notifies its entity.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc::UnboundedSender;

mod helpers;
use helpers::*;

mod actions;
mod auth;
mod covers;
mod library;
mod maintenance;
pub(crate) use maintenance::SyncQueue;
#[cfg(test)]
pub(crate) use maintenance::drive_syncs;
mod marks;
mod page;
mod playback;
mod playlists;
mod radio;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SyncMode {
    Incremental,
    All,
    Repair,
}

/// Source file facts for the fullscreen/status top bars. `bit_rate` is the
/// provider's kbps value; `size` lets the UI derive a rate when it is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackInfo {
    pub song_id: String,
    pub suffix: Option<String>,
    pub bit_rate: Option<i64>,
    pub size: Option<i64>,
}

/// Which catalog entity an [`Event::Notes`] payload describes.
///
/// Album descriptions and artist biographies live in separate cache tables
/// (`album_info` / `artist_info`) and carry unrelated provider ids, so the
/// payload must say which id it names instead of passing an artist id through
/// the album field (D-27).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotesKind {
    /// `album_info` description for an album id.
    #[default]
    Album,
    /// `artist_info` biography for an artist id.
    Artist,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Event {
    LoginRequired {
        offline_available: bool,
    },
    /// The server health probe changed the effective connection state. No
    /// toast is attached: pings are a background invariant, and the UI only
    /// refreshes its catalog on a transition.
    Connection {
        online: bool,
    },
    /// Core answer: the available PipeWire output sinks.
    OutputDevices {
        devices: Vec<crate::player::pipewire::OutputDevice>,
    },
    /// Core answer: the active EQ profile (`None` = off).
    EqChanged(Option<String>),
    /// Core answer: the persisted playback preference snapshot.
    Settings(Box<crate::playback_settings::PlaybackSettings>),
    /// Core answer: source-file metadata for the current queue row.
    TrackInfo(TrackInfo),
    LoggedIn {
        server_url: String,
        username: String,
        offline: bool,
    },
    AuthError(String),
    LoggedOut,
    AccountRemoved {
        label: String,
        next: Option<String>,
    },
    CommandSuggestions {
        generation: u64,
        start: usize,
        end: usize,
        rows: Vec<(String, String)>,
    },
    PlaylistDeleted {
        id: String,
        name: String,
        error: Option<String>,
    },
    PlaylistSaved {
        name: String,
        offline: bool,
        created: bool,
        removed: bool,
        /// A playlist-page reorder; success is silent (no add/remove toast).
        moved: bool,
        /// Number of rows added/removed by a page paste/delete; 0 for the
        /// command-driven create/add/rm toasts.
        rows: usize,
        /// Playlist page the edit targeted (None for command-driven saves).
        playlist_id: Option<String>,
        /// Cursor row to reveal when the page reloads (Some for page edits).
        cursor: Option<usize>,
        error: Option<String>,
    },
    LibraryCount {
        generation: u64,
        scope: crate::model::Scope,
        order: usize,
        count: usize,
    },
    LibraryWindow {
        generation: u64,
        scope: crate::model::Scope,
        order: usize,
        start: usize,
        rows: Vec<crate::model::Row>,
    },
    Library {
        generation: u64,
        scope: crate::model::Scope,
        order: usize,
        rows: Vec<crate::model::Row>,
    },
    Page {
        generation: u64,
        descriptor: crate::model::PageDescriptor,
        header: crate::model::PageHeader,
        rows: Vec<crate::model::Row>,
    },
    Notes {
        /// Album or artist id the payload belongs to.
        id: String,
        /// Which catalog entity [`Event::Notes::id`] names (D-27). Defaulted
        /// so a payload serialized before the discriminator existed still
        /// reads back as album notes, the only kind that shipped then.
        #[serde(default)]
        kind: NotesKind,
        notes: String,
    },
    Marks {
        generation: u64,
        result: crate::download::MarkWindowResult,
    },
    Preload {
        scope: crate::model::Scope,
        order: usize,
        count: usize,
        rows: Vec<crate::model::Row>,
    },
    RowsPatched {
        kind: crate::model::RowKind,
        id: String,
        starred: Option<bool>,
        rating: Option<u8>,
    },
    AlbumState {
        id: String,
        starred: bool,
        rating: u8,
    },
    LibrarySynced,
    RadioSaved {
        name: String,
        offline: bool,
        remote_error: Option<String>,
    },
    SyncProgress {
        message: String,
        done: usize,
        total: usize,
    },
    /// The live sync stopped because the server went away. It keeps its mode
    /// and resumes on the next online transition; no toast is attached.
    SyncPaused,
    PageContainerState {
        kind: crate::model::PageKind,
        id: String,
        cached: bool,
        starred: bool,
        rating: u8,
    },
    Wiped {
        scope: String,
    },
    QueueAppend {
        tracks: Vec<crate::player::Track>,
    },
    QueuePaste {
        tracks: Vec<crate::player::Track>,
        at: usize,
    },
    DownloadSettled,
    DownloadStarted {
        kind: String,
        id: String,
        on: bool,
    },
    Palette {
        cover_art: String,
        roles: BTreeMap<String, String>,
    },
    PaletteReset {
        cover_art: String,
    },
    OpenPage {
        descriptor: crate::model::PageDescriptor,
    },
    OpenPageReveal {
        descriptor: crate::model::PageDescriptor,
        row_kind: crate::model::RowKind,
        row_id: String,
    },
    Command(String),
    /// One parsed IPC/`:` command that ran (or is about to). The attached UI
    /// echoes it in a toast so scripted calls are visible.
    IpcRan(String),
    Restore {
        volume: f64,
        muted: bool,
        tracks: Vec<crate::player::Track>,
        start: usize,
        position: f64,
        /// The page to reopen, already typed (the restore path is the only
        /// producer; it no longer ships loose `page_kind`/`page_id` strings).
        page: Option<crate::model::PageDescriptor>,
    },
    Lyrics {
        song_id: String,
        document: crate::lyrics::Document,
    },
    Queue {
        tracks: Vec<crate::player::Track>,
        start: usize,
    },
    Search {
        generation: u64,
        query: String,
        rows: Vec<crate::model::Row>,
        most_played: bool,
    },
    Header {
        server_url: String,
        username: String,
        server_type: String,
        last_sync: Option<String>,
        counts: (i64, i64, i64),
    },
    Error(String),
}

/// One playlist edit result. Every unrelated flag defaults, so a bridge
/// handler only names what its operation actually changed instead of
/// restating the whole nine-field [`Event::PlaylistSaved`] shape.
#[derive(Debug, Clone, Default)]
pub struct PlaylistSave {
    pub name: String,
    pub offline: bool,
    pub created: bool,
    pub removed: bool,
    pub moved: bool,
    pub rows: usize,
    pub playlist_id: Option<String>,
    pub cursor: Option<usize>,
    pub error: Option<String>,
}

impl PlaylistSave {
    /// A successful edit of `name`.
    pub fn ok(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// A failed edit; the UI toasts `error` under the playlist name.
    pub fn failed(name: impl Into<String>, error: impl std::fmt::Display) -> Self {
        Self {
            name: name.into(),
            error: Some(error.to_string()),
            ..Self::default()
        }
    }

    /// `playlist create`: the UI says "Created" instead of "Added".
    pub fn created(mut self, offline: bool) -> Self {
        self.created = true;
        self.offline = offline;
        self
    }

    /// `playlist rm`: the UI says "Removed from".
    pub fn removed(mut self) -> Self {
        self.removed = true;
        self
    }

    /// A page reorder; success is silent (the list itself shows it).
    pub fn moved(mut self) -> Self {
        self.moved = true;
        self
    }

    /// How many rows the edit added/removed (`playlist add selected`/paste).
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    /// The edited page plus the row to reveal after its reload.
    pub fn at(mut self, playlist_id: impl Into<String>, cursor: Option<usize>) -> Self {
        self.playlist_id = Some(playlist_id.into());
        self.cursor = cursor;
        self
    }
}

impl From<PlaylistSave> for Event {
    fn from(save: PlaylistSave) -> Self {
        Event::PlaylistSaved {
            name: save.name,
            offline: save.offline,
            created: save.created,
            removed: save.removed,
            moved: save.moved,
            rows: save.rows,
            playlist_id: save.playlist_id,
            cursor: save.cursor,
            error: save.error,
        }
    }
}

pub struct Bridge {
    tx: UnboundedSender<Event>,
    generation: AtomicU64,
    page_generation: AtomicU64,
    search_generation: Arc<AtomicU64>,
    marks_generation: AtomicU64,
    /// Attached by the daemon immediately after the playback service is
    /// created. Settings requests go straight to the service; only core→UI
    /// notifications use `Event`.
    playback: OnceLock<crate::daemon::playback::PlaybackControl>,
    palette_inflight: Arc<Mutex<Option<String>>>,
    /// Serialises library passes and remembers one interrupted by offline.
    sync: Arc<SyncQueue>,
}

impl Bridge {
    pub fn new(tx: UnboundedSender<Event>) -> Self {
        let bridge = Self {
            tx,
            generation: AtomicU64::new(0),
            page_generation: AtomicU64::new(0),
            search_generation: Arc::new(AtomicU64::new(0)),
            marks_generation: AtomicU64::new(0),
            playback: OnceLock::new(),
            palette_inflight: Arc::new(Mutex::new(None)),
            sync: Arc::new(SyncQueue::new()),
        };
        bridge.refresh_header();
        bridge
    }

    pub(crate) fn attach_playback(&self, playback: crate::daemon::playback::PlaybackControl) {
        if self.playback.set(playback).is_err() {
            crate::debug_log!("bridge: playback control was already attached");
        }
    }

    fn send_playback(&self, input: crate::daemon::playback::PlaybackInput) {
        if let Some(playback) = self.playback.get() {
            let _ = playback.send(input);
        }
    }

    /// Emits a user-facing error event (toast) for a core-side rejection.
    pub fn error(&self, message: impl Into<String>) {
        _ = self.tx.send(Event::Error(message.into()));
    }

    /// Echoes one executed IPC command to the attached UI (canonical text).
    pub fn ipc_ran(&self, command: String) {
        _ = self.tx.send(Event::IpcRan(command));
    }

    /// Fullscreen cover focus: fetch the playing album's favourite/rating.
    pub fn request_album_state(&self, album_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let state = crate::library::album_states(&dbs.library, std::slice::from_ref(&album_id))
                .await
                .ok()
                .and_then(|states| states.get(&album_id).copied());
            if let Some((starred, rating)) = state {
                _ = tx.send(Event::AlbumState {
                    id: album_id,
                    starred,
                    rating: rating.clamp(0, 5) as u8,
                });
            }
        });
    }
}

pub(crate) fn albums_label(count: i64) -> String {
    if count == 1 {
        "1 album".to_string()
    } else {
        format!("{count} albums")
    }
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    #[test]
    fn bridge_events_round_trip_json() {
        for event in [
            Event::OpenPage {
                descriptor: crate::model::PageDescriptor::new(crate::model::PageKind::Album, "al1"),
            },
            Event::IpcRan("playlist add selected; \"Mix\"".to_string()),
            Event::Connection { online: false },
            Event::SyncPaused,
            Event::OutputDevices {
                devices: vec![crate::player::pipewire::OutputDevice {
                    id: "sink.one".to_string(),
                    label: "Sink One".to_string(),
                }],
            },
            Event::TrackInfo(TrackInfo {
                song_id: "song.one".to_string(),
                suffix: Some("flac".to_string()),
                bit_rate: Some(1_411),
                size: Some(42_000_000),
            }),
        ] {
            let json = serde_json::to_string(&event).unwrap();
            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
    }

    #[test]
    fn playlist_save_only_names_what_changed() {
        let event = Event::from(PlaylistSave::ok("Mix").rows(3).removed().at("p1", Some(2)));
        let Event::PlaylistSaved {
            name,
            offline,
            created,
            removed,
            moved,
            rows,
            playlist_id,
            cursor,
            error,
        } = event
        else {
            panic!("expected PlaylistSaved");
        };
        assert_eq!(name, "Mix");
        assert_eq!(rows, 3);
        assert!(removed && !offline && !created && !moved);
        assert_eq!(playlist_id.as_deref(), Some("p1"));
        assert_eq!(cursor, Some(2));
        assert!(error.is_none());

        let failed = Event::from(PlaylistSave::failed("Mix", "boom").created(true));
        let Event::PlaylistSaved {
            created,
            offline,
            error,
            rows,
            ..
        } = failed
        else {
            panic!("expected PlaylistSaved");
        };
        assert!(created && offline, "a --offline create keeps its flag");
        assert_eq!(rows, 0);
        assert_eq!(error.as_deref(), Some("boom"));
    }
}
