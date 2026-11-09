// Musika — GPUI backend bridge (G1).
//!
//! UI thread never touches tokio/DBs directly. Requests go to the shared
//! runtime and return as plain `Event`s over a tokio channel; the GPUI root
//! task drains them and notifies its entity.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::state::PageHeader;
use tokio::sync::mpsc::UnboundedSender;

mod helpers;
use helpers::*;

mod actions;
mod auth;
mod covers;
mod library;
mod maintenance;
mod marks;
mod page;
mod playback;
mod playlists;
mod radio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    Incremental,
    All,
    Repair,
}

pub enum Event {
    LoginRequired {
        offline_available: bool,
    },
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
        scope: super::state::Scope,
        order: usize,
        count: usize,
    },
    LibraryWindow {
        generation: u64,
        scope: super::state::Scope,
        order: usize,
        start: usize,
        rows: Vec<super::state::Row>,
    },
    Library {
        generation: u64,
        scope: super::state::Scope,
        order: usize,
        rows: Vec<super::state::Row>,
    },
    Page {
        generation: u64,
        descriptor: super::state::PageDescriptor,
        header: super::state::PageHeader,
        rows: Vec<super::state::Row>,
    },
    Notes {
        album_id: String,
        notes: String,
    },
    Marks {
        generation: u64,
        result: crate::download::MarkWindowResult,
    },
    Preload {
        scope: super::state::Scope,
        order: usize,
        count: usize,
        rows: Vec<super::state::Row>,
    },
    DisplayCover {
        key: String,
        cover_art: String,
        target: u32,
        path: Option<std::path::PathBuf>,
    },
    RowsPatched {
        kind: super::state::RowKind,
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
    PageContainerState {
        kind: super::state::PageKind,
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
    Palette {
        cover_art: String,
        roles: BTreeMap<String, String>,
    },
    PaletteReset {
        cover_art: String,
    },
    OpenPage {
        descriptor: super::state::PageDescriptor,
    },
    OpenPageReveal {
        descriptor: super::state::PageDescriptor,
        row_kind: super::state::RowKind,
        row_id: String,
    },
    Command(String),
    Restore {
        volume: f64,
        muted: bool,
        tracks: Vec<crate::player::Track>,
        start: usize,
        position: f64,
        page_kind: String,
        page_id: String,
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
        rows: Vec<super::state::Row>,
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

pub struct Bridge {
    tx: UnboundedSender<Event>,
    generation: AtomicU64,
    page_generation: AtomicU64,
    search_generation: Arc<AtomicU64>,
    marks_generation: AtomicU64,
    offline: AtomicBool,
    palette_inflight: Arc<Mutex<Option<String>>>,
    display_inflight: Arc<Mutex<HashSet<String>>>,
}

impl Bridge {
    pub fn new(tx: UnboundedSender<Event>) -> Self {
        let bridge = Self {
            tx,
            generation: AtomicU64::new(0),
            page_generation: AtomicU64::new(0),
            search_generation: Arc::new(AtomicU64::new(0)),
            marks_generation: AtomicU64::new(0),
            offline: AtomicBool::new(false),
            palette_inflight: Arc::new(Mutex::new(None)),
            display_inflight: Arc::new(Mutex::new(HashSet::new())),
        };
        bridge.refresh_header();
        bridge
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
                let _ = tx.send(Event::AlbumState {
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
