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

//! GPUI front end (G1: real library data).
//!
//! Target is [GPUI](https://gpui.rs). The 64-file QML abstraction spec lives in
//! `docs/mds/gpuui/`; `09-gpui-retarget.md` maps it onto GPUI. This slice wires
//! the shared bridge into an entity root with scope tabs and a virtual list.
#![allow(dead_code)] // shared state carries page/travel fields ahead of G1 pages.

mod anim;
mod bridge;
mod covers;
mod line_search;
mod metrics;
mod quick_actions;
mod state;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Render, Window,
    AssetSource, WindowBounds, WindowOptions, actions, div, img, prelude::*, px, relative, rgb, rgba, size, svg, uniform_list,
    HighlightStyle, StyledText,
    UniformListScrollHandle,
};

use bridge::Event;
use quick_actions::{AltGesture, Direction, QuickMenu, QuickVerb};
use state::{PageDescriptor, PageKind, PageState, Row, RowKind, RowWindow, Scope, Travel, ViewState};

actions!(
    musika,
    [
        Quit,
        MoveDown,
        MoveUp,
        Top,
        Bottom,
        ScopeNext,
        ScopePrev,
        OrderNext,
        OrderPrev,
        Refresh,
        Enter,
        Back,
        PrevPlace,
        NextPlace,
        PageDown,
        PageUp,
        MoveLeft,
        MoveRight,
        Visual,
        VisualAll,
        Search,
        SearchNext,
        SearchPrev,
        Home,
        InsertMode,
        LineSearch,
        LyricsOverlay,
        PlayPause,
        Mute,
        VolumeUp,
        VolumeDown,
        SeekBack,
        SeekForward,
        NextTrack,
        PrevTrack,
        QueueToggle,
        QueueRemove,
        QueueUp,
        QueueDown,
        RepeatCycle,
        SearchScopeNext,
        SearchScopePrev,
        ToggleFullscreen,
    ]
);

const BG: u32 = 0x141218;
const SURFACE: u32 = 0x211F26;
const SURFACE_HIGH: u32 = 0x2B2930;
const ON_SURFACE: u32 = 0xE6E0E9;
const ON_VARIANT: u32 = 0xCAC4D0;
const PRIMARY: u32 = 0xD0BCFF;
const OUTLINE: u32 = 0x49454F;

pub fn run() {
    Application::new()
        .with_assets(Assets)
        .run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("q", Back, None),
            KeyBinding::new("ctrl-q", QueueToggle, None),
            KeyBinding::new("d d", QueueRemove, None),
            KeyBinding::new("ctrl-shift-k", QueueUp, None),
            KeyBinding::new("ctrl-shift-j", QueueDown, None),
            KeyBinding::new("ctrl-r", RepeatCycle, None),
            KeyBinding::new("tab", SearchScopeNext, None),
            KeyBinding::new("shift-tab", SearchScopePrev, None),
            KeyBinding::new("shift-enter", ToggleFullscreen, None),
            KeyBinding::new("j", MoveDown, None),
            KeyBinding::new("k", MoveUp, None),
            KeyBinding::new("g g", Top, None),
            KeyBinding::new("shift-g", Bottom, None),
            KeyBinding::new("shift-j", ScopeNext, None),
            KeyBinding::new("shift-k", ScopePrev, None),
            KeyBinding::new("shift-l", OrderNext, None),
            KeyBinding::new("shift-h", OrderPrev, None),
            KeyBinding::new("r", Refresh, None),
            KeyBinding::new("enter", Enter, None),
            KeyBinding::new("v", Visual, None),
            KeyBinding::new("shift-q", Home, None),
            KeyBinding::new("i", InsertMode, None),
            KeyBinding::new("ctrl-h", SearchScopePrev, None),
            KeyBinding::new("ctrl-l", SearchScopeNext, None),
            KeyBinding::new("shift-space f", Search, None),
            KeyBinding::new("shift-space shift-f", Search, None),
            KeyBinding::new("up", MoveUp, None),
            KeyBinding::new("down", MoveDown, None),
            KeyBinding::new("space", PlayPause, None),
            KeyBinding::new("m", Mute, None),
            KeyBinding::new("-", VolumeDown, None),
            KeyBinding::new("=", VolumeUp, None),
            KeyBinding::new("+", VolumeUp, None),
            KeyBinding::new(",", SeekBack, None),
            KeyBinding::new(".", SeekForward, None),
            KeyBinding::new("ctrl-n", NextTrack, None),
            KeyBinding::new("ctrl-p", PrevTrack, None),
            KeyBinding::new("shift-v", VisualAll, None),
            KeyBinding::new("[ b", PrevPlace, None),
            KeyBinding::new("] b", NextPlace, None),
            KeyBinding::new("ctrl-d", PageDown, None),
            KeyBinding::new("ctrl-u", PageUp, None),
            KeyBinding::new("pagedown", PageDown, None),
            KeyBinding::new("pageup", PageUp, None),
        ]);
        cx.on_window_closed(|cx| {
            crate::scroll_memory::persist();
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.activate(true);
                cx.new(|cx| AppRoot::new(window, cx))
            },
        )
        .expect("GPUI: could not open the application window");
    });
}

struct Header {
    server_url: String,
    username: String,
    server_type: String,
    last_sync: Option<String>,
    counts: (i64, i64, i64),
}

struct SearchState {
    query: String,
    rows: Vec<Row>,
    scope: usize,
    scope_cursors: [usize; 4],
    generation: u64,
    loading: bool,
    typing: bool,
}

struct LyricsState {
    song_id: String,
    lines: Vec<(Option<i64>, String)>,
    synced: bool,
    source: String,
}

struct DescriptionState {
    title: String,
    subtitle: String,
    body: String,
}

#[derive(Default)]
struct MarksState {
    generation: u64,
    songs_done: std::collections::HashSet<String>,
    albums_done: std::collections::HashSet<String>,
    pending: std::collections::HashSet<String>,
    requested: Option<(Vec<String>, Vec<String>)>,
    inflight: bool,
    dirty: bool,
}

impl MarksState {
    fn update(&mut self, generation: u64, result: crate::download::MarkWindowResult) {
        if generation < self.generation {
            return;
        }
        self.generation = generation;
        self.songs_done = result.songs_done.into_iter().collect();
        self.albums_done = result.albums_done.into_iter().collect();
        self.pending = result.pending.into_iter().collect();
        self.inflight = false;
    }

    fn downloaded(&self, song_id: &str, album_id: &str) -> bool {
        (!song_id.is_empty() && self.songs_done.contains(song_id))
            || (!album_id.is_empty() && self.albums_done.contains(album_id))
    }
}

#[derive(Default, Clone, Copy)]
struct ScrollFollow {
    from: f32,
    to: f32,
    started: f64,
    duration: f32,
    active: bool,
}

impl ScrollFollow {
    fn start(&mut self, from: f32, to: f32, now: f64, duration: f32) {
        self.from = from;
        self.to = to;
        self.started = now;
        self.duration = duration;
        self.active = true;
    }
}

#[derive(Clone)]
struct AltTarget {
    kind: RowKind,
    id: String,
    album_id: String,
    song_id: String,
    playlist_id: String,
}

impl AltTarget {
    fn queue_kind(&self) -> &'static str {
        match self.kind {
            RowKind::Song => "song",
            RowKind::Album => "album",
            RowKind::Artist => "artist",
            RowKind::Playlist => "playlist",
        }
    }
}

struct LoginState {
    server: String,
    username: String,
    password: String,
    field: usize,
    busy: bool,
    error: Option<String>,
    offline_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAction {
    ReloadLibrary,
    Playback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Error,
    Sync,
    Notice,
}

#[derive(Debug, Clone)]
struct ToastState {
    message: String,
    kind: ToastKind,
    retry: Option<RetryAction>,
    created_at: f64,
}

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        std::fs::read(path)
            .map(Into::into)
            .map_err(Into::into)
            .map(Some)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<gpui::SharedString>> {
        Ok(std::fs::read_dir(path)?
            .filter_map(|entry| {
                Some(gpui::SharedString::from(
                    entry.ok()?.path().to_string_lossy().into_owned(),
                ))
            })
            .collect())
    }
}

struct AppRoot {
    focus: FocusHandle,
    bridge: bridge::Bridge,
    view: ViewState,
    header: Option<Header>,
    error: Option<String>,
    generation: u64,
    scroll: UniformListScrollHandle,
    library_row_h: f32,
    library_row_ref: Option<f32>,
    library_row_ratio: f32,
    library_follow: ScrollFollow,
    page_follow: ScrollFollow,
    queue_follow: ScrollFollow,
    page_row_h: f32,
    page_row_ref: Option<f32>,
    page_row_ratio: f32,
    queue_row_h: f32,
    queue_row_ref: Option<f32>,
    queue_row_ratio: f32,
    search_row_h: f32,
    search_row_ref: Option<f32>,
    search_row_ratio: f32,
    page: Option<PageState>,
    travel: Travel,
    page_generation: u64,
    page_scroll: UniformListScrollHandle,
    search: Option<SearchState>,
    search_open: bool,
    search_anim: anim::Behavior,
    clock: anim::Clock,
    row_colors: std::collections::HashMap<String, anim::ColorBehavior>,
    row_cover_scales: std::collections::HashMap<String, anim::Behavior>,
    hovered_row: Option<usize>,
    hovered_page: Option<usize>,
    player: crate::player::Player,
    playback_error: Option<String>,
    queue_open: bool,
    queue_cursor: usize,
    queue_anim: anim::Behavior,
    queue_scroll: UniformListScrollHandle,
    repeat: crate::player::Repeat,
    login: Option<LoginState>,
    fullscreen: bool,
    fullscreen_anim: anim::Behavior,
    lyrics: Option<LyricsState>,
    lyrics_requested: Option<String>,
    quit_armed_at: Option<f64>,
    exiting: Option<f64>,
    quit_after_ms: Option<u64>,
    toasts: Vec<ToastState>,
    last_play: Option<(String, String)>,
    restored: bool,
    notes: Option<(String, String)>,
    description: Option<DescriptionState>,
    lyrics_overlay: bool,
    lyrics_overlay_wanted: bool,
    line_search: line_search::LineSearchState,
    line_search_anim: anim::Behavior,
    alt: AltGesture,
    alt_menu: Option<QuickMenu>,
    alt_target: Option<AltTarget>,
    alt_anchor: Option<(f32, f32)>,
    alt_spawn: anim::Behavior,
    alt_held: bool,
    alt_was_typing: bool,
    win_w: f32,
    win_h: f32,
    marks: MarksState,
    warm: std::collections::HashMap<(Scope, usize), (usize, Vec<Row>)>,
    preloaded: bool,
    display_covers: std::collections::HashMap<String, std::path::PathBuf>,
    display_requested: std::collections::HashSet<String>,
    trace: bool,
    frames: u64,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);

        crate::scroll_memory::hydrate();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let bridge = bridge::Bridge::new(tx);
        let mut view = ViewState::default();
        bridge.load_library(view.scope, view.order);
        view.loading = true;

        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.apply(event);
                    cx.notify();
                });
            }
        })
        .detach();

        let mut app = Self {
            focus,
            bridge,
            view,
            header: None,
            error: None,
            generation: 0,
            scroll: UniformListScrollHandle::new(),
            library_row_h: 58.0,
            library_row_ref: None,
            library_row_ratio: 58.0 / 600.0,
            library_follow: ScrollFollow::default(),
            page_follow: ScrollFollow::default(),
            queue_follow: ScrollFollow::default(),
            page_row_h: 36.0,
            page_row_ref: None,
            page_row_ratio: 36.0 / 600.0,
            queue_row_h: 56.0,
            queue_row_ref: None,
            queue_row_ratio: 56.0 / 600.0,
            search_row_h: 56.0,
            search_row_ref: None,
            search_row_ratio: 56.0 / 600.0,
            page: None,
            travel: Travel::default(),
            page_generation: 0,
            page_scroll: UniformListScrollHandle::new(),
            search: None,
            search_open: false,
            search_anim: anim::Behavior::new(0.0),
            clock: anim::Clock::new(),
            row_colors: std::collections::HashMap::new(),
            row_cover_scales: std::collections::HashMap::new(),
            hovered_row: None,
            hovered_page: None,
            player: crate::player::Player::new(),
            playback_error: None,
            queue_open: false,
            queue_cursor: 0,
            queue_anim: anim::Behavior::new(0.0),
            queue_scroll: UniformListScrollHandle::new(),
            repeat: crate::player::Repeat::Off,
            login: None,
            fullscreen: false,
            fullscreen_anim: anim::Behavior::new(0.0),
            lyrics: None,
            lyrics_requested: None,
            quit_armed_at: None,
            exiting: None,
            quit_after_ms: std::env::var("MUSIKA_GPUUI_QUIT_MS")
                .ok()
                .and_then(|ms| ms.parse::<u64>().ok()),
            toasts: Vec::new(),
            last_play: None,
            restored: false,
            notes: None,
            description: None,
            lyrics_overlay: false,
            lyrics_overlay_wanted: false,
            line_search: line_search::LineSearchState::new(),
            line_search_anim: anim::Behavior::new(0.0),
            alt: AltGesture::new(),
            alt_menu: None,
            alt_target: None,
            alt_anchor: None,
            alt_spawn: anim::Behavior::new(0.0),
            alt_held: false,
            alt_was_typing: false,
            win_w: 1200.0,
            win_h: 800.0,
            marks: MarksState::default(),
            warm: std::collections::HashMap::new(),
            preloaded: false,
            display_covers: std::collections::HashMap::new(),
            display_requested: std::collections::HashSet::new(),
            trace: std::env::var_os("MUSIKA_GPUUI_TRACE").is_some(),
            frames: 0,
        };
        if let Ok(message) = std::env::var("MUSIKA_GPUUI_TOAST")
            && !message.is_empty()
        {
            app.push_toast(message, ToastKind::Error, None);
        }
        if std::env::var_os("MUSIKA_GPUUI_FULLSCREEN").is_some() {
            app.fullscreen = true;
        }
        if std::env::var_os("MUSIKA_GPUUI_QUEUE").is_some() {
            app.queue_open = true;
            app.queue_cursor = app.player.index();
        }
        if let Ok(song_id) = std::env::var("MUSIKA_GPUUI_PLAY")
            && !song_id.is_empty()
        {
            app.bridge.play_song(song_id, "Artist".to_string());
        }
        if let Ok(query) = std::env::var("MUSIKA_GPUUI_SEARCH")
            && !query.is_empty()
        {
            app.search_open = true;
            app.hovered_row = None;
            app.search = Some(SearchState {
                query: query.clone(),
                rows: Vec::new(),
                scope: 0,
                scope_cursors: [0; 4],
                generation: 1,
                loading: true,
                typing: true,
            });
            app.bridge.search(query);
        }
        if let Ok(spec) = std::env::var("MUSIKA_GPUUI_PAGE")
            && let Some(descriptor) = parse_page_spec(&spec)
        {
            app.open_descriptor(descriptor);
        }
        app
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Header {
                server_url,
                username,
                server_type,
                last_sync,
                counts,
            } => {
                self.header = Some(Header {
                    server_url,
                    username,
                    server_type,
                    last_sync,
                    counts,
                });
                self.error = None;
                if !self.restored {
                    self.restored = true;
                    self.bridge.restore();
                }
            }
            Event::LibraryCount {
                generation,
                scope,
                order,
                count,
            } => {
                if generation < self.generation || scope != self.view.scope || order != self.view.order {
                    return;
                }
                self.generation = generation;
                self.view.rows.install_count(generation, count);
                self.view.loading = count == 0;
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.cursor = saved as usize;
                }
                self.view.cursor = self.view.cursor.min(count.saturating_sub(1));
                self.follow_library(false);
                self.ensure_library_window();
            }
            Event::LibraryWindow {
                generation,
                scope,
                order,
                start,
                rows,
            } => {
                if generation < self.generation || scope != self.view.scope || order != self.view.order {
                    return;
                }
                self.view.rows.install_window(start, rows);
                self.view.loading = false;
                self.view.error = None;
                self.ensure_marks();
            }
            Event::Library {
                generation,
                scope,
                order,
                rows,
            } => {
                if generation < self.generation || scope != self.view.scope || order != self.view.order {
                    return;
                }
                self.generation = generation;
                self.view.rows = RowWindow::full(rows);
                self.view.rows.generation = generation;
                self.view.loading = false;
                self.ensure_marks();
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.cursor = saved as usize;
                }
                self.view.cursor = self.view.cursor.min(self.view.rows.len().saturating_sub(1));
                self.follow_library(false);
            }
            Event::Notes { album_id, notes } => {
                self.notes = Some((album_id, notes));
            }
            Event::RowsPatched {
                kind,
                id,
                starred,
                rating,
            } => {
                let patch = |row: &mut Row| {
                    if row.id == id && row.kind == kind {
                        if let Some(starred) = starred {
                            row.starred = starred;
                        }
                        if let Some(rating) = rating {
                            row.rating = i64::from(rating);
                        }
                    }
                };
                for row in &mut self.view.rows.live {
                    patch(row);
                }
                if let Some(page) = self.page.as_mut() {
                    for row in &mut page.rows {
                        patch(row);
                    }
                }
                if let Some(search) = self.search.as_mut() {
                    for row in &mut search.rows {
                        patch(row);
                    }
                }
            }
            Event::QueueAppend { tracks } => {
                self.player.append(tracks);
            }
            Event::DownloadSettled => {
                self.marks.requested = None;
                self.ensure_marks();
            }
            Event::OpenPage { descriptor } => {
                self.open_descriptor(descriptor);
            }
            Event::DisplayCover { key, path } => {
                if let Some(path) = path {
                    self.display_covers.insert(key, path);
                }
            }
            Event::Preload {
                scope,
                order,
                count,
                rows,
            } => {
                if self.trace {
                    crate::debug_log!("GPUI trace: preload {} order {} count {}", scope.label(), order, count);
                }
                self.warm.insert((scope, order), (count, rows));
            }
            Event::Marks { generation, result } => {
                self.marks.update(generation, result);
                if self.marks.dirty {
                    self.marks.dirty = false;
                    self.ensure_marks();
                }
            }
            Event::Restore {
                volume,
                muted,
                tracks,
                start,
                position,
                page_kind,
                page_id,
            } => {
                self.player.set_volume(volume);
                self.player.set_muted(muted);
                if !tracks.is_empty() {
                    self.player.load_paused(tracks, start);
                    if position > 0.0 {
                        self.player.seek(position);
                    }
                }
                if self.page.is_none() && !page_kind.is_empty() && !page_id.is_empty() {
                    let kind = match page_kind.as_str() {
                        "album" => Some(PageKind::Album),
                        "artist" => Some(PageKind::Artist),
                        "playlist" => Some(PageKind::Playlist),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        self.open_descriptor(PageDescriptor { kind, id: page_id });
                    }
                }
            }
            Event::Page {
                generation,
                descriptor,
                header,
                rows,
            } => {
                if generation < self.page_generation {
                    return;
                }
                self.page_generation = generation;
                let page = self
                    .page
                    .get_or_insert_with(|| PageState::new(descriptor.clone()));
                if page.descriptor != descriptor {
                    *page = PageState::new(descriptor.clone());
                }
                crate::debug_log!("GPUI: page cover={:?} title={}", header.cover_url, header.title);
                page.header = header;
                page.rows = rows;
                page.loading = false;
                page.error = None;
                page.cursor = page.cursor.min(page.rows.len().saturating_sub(1));
                self.follow_page(false);
                self.marks.requested = None;
                self.ensure_marks();
            }
            Event::LoginRequired { offline_available } => {
                self.login = Some(LoginState {
                    server: String::new(),
                    username: String::new(),
                    password: String::new(),
                    field: 0,
                    busy: false,
                    error: None,
                    offline_available,
                });
            }
            Event::LoggedIn { server_url, username, offline } => {
                self.login = None;
                self.header = Some(Header {
                    server_url: server_url.clone(),
                    username: username.clone(),
                    server_type: String::new(),
                    last_sync: None,
                    counts: self.header.as_ref().map(|header| header.counts).unwrap_or((0, 0, 0)),
                });
                self.error = Some(if offline { "Offline mode".to_string() } else { "Logged in".to_string() });
                self.reload();
                if !offline {
                    self.bridge.refresh_header();
                }
            }
            Event::AuthError(message) => {
                if let Some(login) = self.login.as_mut() {
                    login.busy = false;
                    login.error = Some(message.clone());
                }
                self.error = Some(message);
            }
            Event::Lyrics { song_id, document } => {
                crate::debug_log!(
                    "GPUI: lyrics {} lines synced={} source={}",
                    song_id,
                    document.lines.len(),
                    document.source
                );
                self.lyrics = Some(LyricsState {
                    song_id,
                    synced: document.synced,
                    source: document.source,
                    lines: document
                        .lines
                        .into_iter()
                        .map(|line| (line.start_ms, line.value))
                        .collect(),
                });
            }
            Event::Queue { tracks, start } => {
                self.player.load(tracks, start);
                self.playback_error = None;
                self.request_current_lyrics();
            }
            Event::Search {
                generation,
                query,
                rows,
            } => {
                if let Some(search) = self.search.as_mut()
                    && generation >= search.generation
                {
                    search.generation = generation;
                    search.query = query;
                    search.rows = rows;
                    search.scope_cursors = [0; 4];
                    self.hovered_row = None;
                    search.loading = false;
                    crate::debug_log!(
                        "GPUI: search scopes songs={} albums={} artists={} playlists={}",
                        search.scope_count(0),
                        search.scope_count(1),
                        search.scope_count(2),
                        search.scope_count(3)
                    );
                    if search.scope_count(search.scope) == 0 {
                        for step in 0..4 {
                            let candidate = (search.scope + step) % 4;
                            if search.scope_count(candidate) > 0 {
                                search.scope = candidate;
                                break;
                            }
                        }
                    }
                }
            }
            Event::Error(message) => {
                self.view.loading = false;
                self.view.error = Some(message.clone());
                self.error = Some(message.clone());
                let lower = message.to_lowercase();
                let (kind, retry) = if lower.contains("playback") {
                    (ToastKind::Error, self.last_play.as_ref().map(|_| RetryAction::Playback))
                } else if lower.contains("library") || lower.contains("search") || lower.contains("sync") {
                    (ToastKind::Sync, Some(RetryAction::ReloadLibrary))
                } else {
                    (ToastKind::Error, None)
                };
                self.push_toast(message, kind, retry);
            }
        }
    }

    fn reload(&mut self) {
        if let Some(covers) = COVERS.get()
            && let Ok(mut covers) = covers.lock()
        {
            covers.clear();
        }
        self.view.loading = true;
        self.view.rows.clear();
        self.marks.requested = None;
        self.generation = self.bridge.load_library(self.view.scope, self.view.order);
    }

    fn scroll_target(handle: &UniformListScrollHandle, row_h: f32, count: usize, cursor: usize) -> f32 {
        let viewport = handle
            .0
            .borrow()
            .last_item_size
            .map(|size| f32::from(size.item.height))
            .filter(|height| *height > 0.0)
            .unwrap_or(600.0);
        let max = (count as f32 * row_h - viewport).max(0.0);
        let center = cursor as f32 * row_h + row_h * 0.5 - viewport * 0.5;
        center.clamp(0.0, max)
    }

    fn start_follow(follow: &mut ScrollFollow, handle: &UniformListScrollHandle, target: f32, now: f64, animated: bool) {
        let current = -f32::from(handle.0.borrow().base_handle.offset().y);
        let moved = (target - current).abs() > 0.5;
        let duration = if animated && moved {
            0.300
        } else {
            0.0
        };
        if trace_scroll() {
            crate::debug_log!("GPUI trace: follow from {current:.1} to {target:.1} duration {duration:.2}");
        }
        follow.start(current, target, now, duration);
    }

    fn apply_follow(follow: &mut ScrollFollow, handle: &UniformListScrollHandle, now: f64) -> bool {
        if !follow.active {
            return false;
        }
        let y = if follow.duration <= 0.0 {
            follow.active = false;
            follow.to
        } else {
            let progress = (((now - follow.started) / follow.duration as f64) as f32).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - progress).powi(3);
            let y = follow.from + (follow.to - follow.from) * eased;
            if progress >= 1.0 {
                follow.active = false;
            }
            y
        };
        handle
            .0
            .borrow()
            .base_handle
            .set_offset(gpui::point(px(0.0), px(-y)));
        if trace_scroll() {
            crate::debug_log!("GPUI trace: follow apply y {y:.1} active {}", follow.active);
        }
        follow.active
    }

    fn follow_library(&mut self, animated: bool) {
        let target = Self::scroll_target(
            &self.scroll,
            self.library_row_h,
            self.view.rows.count,
            self.view.cursor,
        );
        Self::start_follow(&mut self.library_follow, &self.scroll, target, self.clock.now, animated);
    }

    fn follow_page(&mut self, animated: bool) {
        let Some(page) = self.page.as_ref() else { return };
        let target = Self::scroll_target(&self.page_scroll, self.page_row_h, page.rows.len(), page.cursor);
        Self::start_follow(&mut self.page_follow, &self.page_scroll, target, self.clock.now, animated);
    }

    fn follow_queue(&mut self, animated: bool) {
        let target = Self::scroll_target(
            &self.queue_scroll,
            self.queue_row_h,
            self.player.len(),
            self.queue_cursor,
        );
        Self::start_follow(&mut self.queue_follow, &self.queue_scroll, target, self.clock.now, animated);
    }

    /// Ask the Rust row window for the band around the visible rows and the
    /// cursor. The request is deduped by the window coverage, exactly like
    /// QML's `ensureVirtualWindow`.
    fn ensure_library_window(&mut self) {
        if self.page.is_some() || self.view.scope == Scope::Playlists {
            return;
        }
        let count = self.view.rows.count;
        if count == 0 || self.view.rows.generation == 0 {
            return;
        }
        let (top, measured) = {
            let state = self.scroll.0.borrow();
            let top = state.base_handle.logical_scroll_top().0;
            let measured = state
                .last_item_size
                .map(|size| f32::from(size.item.height))
                .filter(|height| *height > 0.0);
            (top, measured)
        };
        if let Some(viewport) = measured {
            let reference = *self.library_row_ref.get_or_insert(viewport);
            self.library_row_ratio = 58.0 / reference;
            let next = metrics::adaptive_row(viewport, self.library_row_ratio, 58.0)
                .round()
                .max(18.0);
            if (next - self.library_row_h).abs() >= 0.5 {
                self.library_row_h = next;
                self.view.rows.requested = None;
                self.follow_library(false);
            }
        }
        let viewport = measured.unwrap_or(600.0);
        let row_h = self.library_row_h;
        let visible = ((viewport / row_h).ceil() as usize).max(1);
        let overscan = 10;
        let cursor = self.view.cursor.min(count - 1);
        let mut first = top.saturating_sub(overscan);
        let mut last = (top + visible + overscan).min(count - 1);
        first = first.min(cursor);
        last = last.max(cursor);
        let start = first.saturating_sub(20);
        let end = (last + 20).min(count - 1);
        if self.view.rows.covers(start, end) {
            return;
        }
        self.view.rows.set_requested(start, end);
        if self.trace {
            crate::debug_log!(
                "GPUI trace: row window {}..{} cursor {} count {} row_h {:.1}",
                start,
                end,
                self.view.cursor,
                count,
                row_h
            );
        }
        self.bridge.request_library_window(
            self.view.rows.generation,
            self.view.scope,
            self.view.order,
            start,
            end - start + 1,
        );
    }

    fn ensure_queue_metrics(&mut self) {
        let measured = {
            let state = self.queue_scroll.0.borrow();
            state
                .last_item_size
                .map(|size| f32::from(size.item.height))
                .filter(|height| *height > 0.0)
        };
        let Some(viewport) = measured else { return };
        let reference = *self.queue_row_ref.get_or_insert(viewport);
        self.queue_row_ratio = 56.0 / reference;
        let next = metrics::adaptive_row(viewport, self.queue_row_ratio, 56.0)
            .round()
            .max(18.0);
        if (next - self.queue_row_h).abs() >= 0.5 {
            self.queue_row_h = next;
            self.follow_queue(false);
        }
    }

    fn ensure_search_metrics(&mut self, window_height: f32) {
        let reference = match self.search_row_ref {
            Some(reference) => reference,
            None => {
                let reference = (window_height * 0.72).max(1.0);
                self.search_row_ref = Some(reference);
                reference
            }
        };
        self.search_row_ratio = 56.0 / reference;
        self.search_row_h = metrics::adaptive_row(
            window_height * 0.72,
            self.search_row_ratio,
            56.0,
        )
        .round()
        .max(18.0);
    }

    fn ensure_page_metrics(&mut self) {
        let measured = {
            let state = self.page_scroll.0.borrow();
            state
                .last_item_size
                .map(|size| f32::from(size.item.height))
                .filter(|height| *height > 0.0)
        };
        let Some(viewport) = measured else { return };
        let reference = *self.page_row_ref.get_or_insert(viewport);
        self.page_row_ratio = 36.0 / reference;
        let next = metrics::adaptive_row(viewport, self.page_row_ratio, 36.0)
            .round()
            .max(18.0);
        if (next - self.page_row_h).abs() >= 0.5 {
            self.page_row_h = next;
            self.marks.requested = None;
            self.follow_page(false);
        }
    }

    fn ensure_marks(&mut self) {
        if self.marks.inflight {
            self.marks.dirty = true;
            return;
        }
        let mut songs = Vec::new();
        let mut albums = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut push = |row: &Row, songs: &mut Vec<String>, albums: &mut Vec<String>| {
            if !row.song_id.is_empty() && seen.insert(format!("s:{}", row.song_id)) {
                songs.push(row.song_id.clone());
            }
            if !row.album_id.is_empty() && seen.insert(format!("a:{}", row.album_id)) {
                albums.push(row.album_id.clone());
            }
        };
        if let Some(page) = self.page.as_ref() {
            let (top, viewport) = {
                let state = self.page_scroll.0.borrow();
                let top = state.base_handle.logical_scroll_top().0;
                let viewport = state
                    .last_item_size
                    .map(|size| f32::from(size.item.height))
                    .unwrap_or(600.0);
                (top, viewport)
            };
            let visible = ((viewport / self.page_row_h).ceil() as usize).max(1);
            let lo = top.saturating_sub(10);
            let hi = (top + visible + 10).min(page.rows.len().saturating_sub(1));
            for row in page.rows.get(lo..=hi).unwrap_or_default() {
                push(row, &mut songs, &mut albums);
            }
        } else {
            for row in &self.view.rows.live {
                push(row, &mut songs, &mut albums);
            }
        }
        if songs.is_empty() && albums.is_empty() {
            return;
        }
        if self.marks.requested.as_ref() == Some(&(songs.clone(), albums.clone())) {
            return;
        }
        self.marks.requested = Some((songs.clone(), albums.clone()));
        self.marks.inflight = true;
        if self.trace {
            crate::debug_log!(
                "GPUI trace: marks window {} songs, {} albums",
                songs.len(),
                albums.len()
            );
        }
        self.bridge.request_marks(songs, albums);
    }

    fn ensure_display_cover(&mut self, cover_art: &str, target: u32) {
        if cover_art.is_empty() {
            return;
        }
        let key = format!("{cover_art}:{target}");
        if !self.display_requested.insert(key.clone()) {
            return;
        }
        self.bridge
            .request_display_cover(key, cover_art.to_string(), target);
    }

    fn display_cover(&self, cover_art: &str, target: u32) -> Option<std::path::PathBuf> {
        self.display_covers
            .get(&format!("{cover_art}:{target}"))
            .cloned()
    }

    fn set_scope(&mut self, scope: Scope) {
        if scope == self.view.scope {
            return;
        }
        self.remember_current();
        self.hovered_row = None;
        let wanted = (scope, 0);
        if let Some((count, rows)) = self.warm.remove(&wanted) {
            self.view.clear_for(scope);
            self.view.rows = RowWindow::full(rows);
            self.view.rows.count = count;
            self.view.loading = false;
            self.generation = self.bridge.load_library(scope, 0);
        } else {
            self.view.clear_for(scope);
            self.reload();
        }
    }

    fn select(&mut self, index: usize) {
        self.view.select(index);
        self.follow_library(true);
        self.remember_current();
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn on_move_down(&mut self, _: &MoveDown, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(1);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_cursor = self.queue_cursor.saturating_add(1).min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_add(1));
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_add(1));
        }
        cx.notify();
    }

    fn on_move_up(&mut self, _: &MoveUp, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(-1);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_cursor = self.queue_cursor.saturating_sub(1);
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_sub(1));
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_sub(1));
        }
        cx.notify();
    }

    fn on_top(&mut self, _: &Top, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.select(0);
            self.follow_page(false);
        } else {
            self.select(0);
        }
        cx.notify();
    }

    fn on_bottom(&mut self, _: &Bottom, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            let last = page.rows.len().saturating_sub(1);
            page.select(last);
        } else {
            self.select(self.view.rows.len().saturating_sub(1));
        }
        cx.notify();
    }

    fn on_scope_next(&mut self, _: &ScopeNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_scope(self.view.scope.next());
        cx.notify();
    }

    fn on_scope_prev(&mut self, _: &ScopePrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.set_scope(self.view.scope.prev());
        cx.notify();
    }

    fn on_order_next(&mut self, _: &OrderNext, _window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.move_search_scope(1);
            cx.notify();
            return;
        }
        self.cycle_order(true);
        cx.notify();
    }

    fn on_order_prev(&mut self, _: &OrderPrev, _window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.move_search_scope(-1);
            cx.notify();
            return;
        }
        self.cycle_order(false);
        cx.notify();
    }

    fn on_refresh(&mut self, _: &Refresh, _window: &mut Window, cx: &mut Context<Self>) {
        self.reload();
        cx.notify();
    }

    fn on_toggle_fullscreen(&mut self, _: &ToggleFullscreen, _window: &mut Window, cx: &mut Context<Self>) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            self.request_current_lyrics();
        }
        cx.notify();
    }

    fn request_current_lyrics(&mut self) {
        let Some(track) = self.player.current() else { return };
        let id = track.id.clone();
        if self.lyrics.as_ref().is_some_and(|lyrics| lyrics.song_id == id) {
            return;
        }
        if self.lyrics_requested.as_deref() == Some(id.as_str()) {
            return;
        }
        self.lyrics_requested = Some(id.clone());
        self.bridge.request_lyrics(id);
    }

    fn on_repeat_cycle(&mut self, _: &RepeatCycle, _window: &mut Window, cx: &mut Context<Self>) {
        self.repeat = self.player.cycle_repeat();
        cx.notify();
    }

    fn on_queue_toggle(&mut self, _: &QueueToggle, _window: &mut Window, cx: &mut Context<Self>) {
        self.queue_open = !self.queue_open;
        if self.queue_open {
            self.queue_cursor = self.player.index().min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
        }
        cx.notify();
    }

    fn on_queue_remove(&mut self, _: &QueueRemove, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open {
            self.player.remove(self.queue_cursor);
            self.queue_cursor = self.queue_cursor.min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
            cx.notify();
        }
    }

    fn on_queue_up(&mut self, _: &QueueUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open && self.queue_cursor > 0 {
            self.player.move_item(self.queue_cursor, self.queue_cursor - 1);
            self.queue_cursor -= 1;
            self.follow_queue(true);
            cx.notify();
        }
    }

    fn on_queue_down(&mut self, _: &QueueDown, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open && self.queue_cursor + 1 < self.player.len() {
            self.player.move_item(self.queue_cursor, self.queue_cursor + 1);
            self.queue_cursor += 1;
            self.follow_queue(true);
            cx.notify();
        }
    }

    fn login_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let Some(login) = self.login.as_mut() else { return };
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform;
        match key {
            "tab" => login.field = (login.field + 1) % 3,
            "backspace" => {
                let text = match login.field {
                    0 => &mut login.server,
                    1 => &mut login.username,
                    _ => &mut login.password,
                };
                text.pop();
            }
            "enter" => {
                if login.server.trim().is_empty() || login.username.trim().is_empty() || login.password.is_empty() {
                    login.error = Some("Server, username and password are all required.".to_string());
                } else {
                    login.busy = true;
                    login.error = None;
                    let (server, username, password) = (
                        login.server.trim().to_string(),
                        login.username.trim().to_string(),
                        login.password.clone(),
                    );
                    self.bridge.login(server, username, password);
                }
            }
            "f2" => {
                if login.offline_available {
                    let (server, username) = (
                        login.server.trim().to_string(),
                        login.username.trim().to_string(),
                    );
                    self.bridge.offline_login(server, username);
                }
            }
            _ if !modified => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !text.chars().any(char::is_control)
                {
                    match login.field {
                        0 => login.server.push_str(text),
                        1 => login.username.push_str(text),
                        _ => login.password.push_str(text),
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn login_view(&self) -> gpui::Div {
        let login = self.login.as_ref().expect("login view");
        let fields = [
            ("Server URL", login.server.as_str(), login.field == 0),
            ("Username", login.username.as_str(), login.field == 1),
            ("Password", if login.password.is_empty() { "" } else { "••••••••" }, login.field == 2),
        ];
        div()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(ON_SURFACE))
            .font_family("Overused Grotesk Roman")
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(relative(0.224))
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(26.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(PRIMARY))
                            .text_center()
                            .child("MUSIKA"),
                    )
                    .children(fields.into_iter().map(|(label, value, active)| {
                        div()
                            .h(px(52.0))
                            .px_3()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_normal()
                            .max_h(px(46.0))
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if active { PRIMARY } else { OUTLINE }))
                            .bg(rgb(0x36343B))
                            .text_size(px(14.0))
                            .text_color(rgb(if value.is_empty() { ON_VARIANT } else { ON_SURFACE }))
                            .child(if value.is_empty() {
                                label.to_string()
                            } else {
                                value.to_string()
                            })
                    }))
                    .child(
                        div()
                            .h(px(52.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgb(0x4F378B))
                            .text_size(px(14.0))
                            .text_color(rgb(0xEADDFF))
                            .child(if login.busy { "Logging in…" } else { "Log in" }),
                    )
                    .child(
                        div()
                            .min_h(px(52.0))
                            .w_full()
                            .px_2()
                            .py_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .when(login.error.is_some(), |this| this.border_1().border_color(rgb(0xF2B8B5)).bg(rgb(0x8C1D18)))
                            .text_size(px(11.0))
                            .text_color(rgb(if login.error.is_some() { 0xF9DEDC } else { ON_VARIANT }))
                            .text_center()
                            .whitespace_normal()
                            .child(login.error.clone().unwrap_or_else(|| {
                                if login.offline_available {
                                    "Tab moves · Enter logs in · F2 offline".to_string()
                                } else {
                                    "Tab moves · Enter logs in".to_string()
                                }
                            })),
                    ),
            )
    }

    fn flush_status(&self) {
        let Some(header) = self.header.as_ref() else { return };
        let Some(dbs) = crate::app::dbs() else { return };
        let queue = self.player.queue_snapshot();
        let queue: Vec<&str> = queue.iter().map(|track| track.id.as_str()).collect();
        let queue = serde_json::to_string(&queue).unwrap_or_default();
        let (page_kind, page_id) = match self.page.as_ref().map(|page| &page.descriptor) {
            Some(descriptor) => {
                let kind = match descriptor.kind {
                    PageKind::Album => "album",
                    PageKind::Artist => "artist",
                    PageKind::Playlist => "playlist",
                };
                (kind.to_string(), descriptor.id.clone())
            }
            None => (String::new(), String::new()),
        };
        let current = self.player.current();
        let status = crate::status::SessionStatus {
            server_url: header.server_url.clone(),
            username: header.username.clone(),
            album_id: current.and_then(|track| (!track.album_id.is_empty()).then(|| track.album_id.clone())),
            song_id: current.map(|track| track.id.clone()),
            page_kind,
            page_id,
            queue,
            queue_index: self.player.index() as i64,
            position: self.player.position(),
            volume: self.player.volume(),
            muted: self.player.muted(),
            updated_at: crate::consts::now_secs(),
        };
        crate::debug_log!("GPUI: flushing status (queue {} tracks)", self.player.len());
        if let Err(err) = crate::app::runtime().block_on(crate::status::save(&dbs.status, &status)) {
            eprintln!("status: could not save the session: {err:#}");
        }
    }

    fn exit_overlay(&self, progress: f32) -> gpui::Div {
        let letters = ["B", "y", "e", "!"];
        div()
            .absolute()
            .inset_0()
            .opacity(progress)
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.62))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .children(letters.into_iter().map(|letter| {
                        let scale = 0.6 + 0.4 * anim::Curve::OutBack.sample(progress);
                        div()
                            .text_size(px(72.0 * scale))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(gpui::white())
                            .opacity(progress)
                            .child(letter)
                    })),
            )
    }

    fn push_toast(&mut self, message: String, kind: ToastKind, retry: Option<RetryAction>) {
        if self.toasts.iter().any(|toast| toast.message == message) {
            return;
        }
        self.toasts.push(ToastState {
            message,
            kind,
            retry,
            created_at: self.clock.now,
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    fn retry(&mut self, action: RetryAction) {
        match action {
            RetryAction::ReloadLibrary => self.reload(),
            RetryAction::Playback => {
                if let Some((song, order)) = self.last_play.clone() {
                    self.bridge.play_song(song, order);
                }
            }
        }
    }

    fn toast_overlay(&self, cx: &Context<Self>) -> gpui::Div {
        div()
            .absolute()
            .bottom(px(88.0))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .children(self.toasts.iter().map(|toast| {
                let age = (self.clock.now - toast.created_at) as f32;
                let opacity = if age < 0.120 {
                    age / 0.120
                } else if age > 6.0 {
                    ((6.320 - age) / 0.320).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                let retry = toast.retry;
                div()
                    .w(relative(0.42))
                    .max_h(px(150.0))
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(OUTLINE))
                    .bg(rgb(SURFACE))
                    .opacity(opacity)
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(rgb(ON_SURFACE))
                            .whitespace_normal()
                            .child(toast.message.clone()),
                    )
                    .when_some(retry, |this, retry| {
                        this.child(
                            div()
                                .id("toast-retry")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(0x4F378B))
                                .text_size(px(11.0))
                                .text_color(rgb(0xEADDFF))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.retry(retry);
                                    cx.notify();
                                }))
                                .child("Retry"),
                        )
                    })
            }))
    }

    fn on_play_pause(&mut self, _: &PlayPause, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.toggle();
        cx.notify();
    }

    fn on_mute(&mut self, _: &Mute, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.toggle_mute();
        cx.notify();
    }

    fn on_volume_up(&mut self, _: &VolumeUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.set_volume((self.player.volume() + 5.0).min(100.0));
        cx.notify();
    }

    fn on_volume_down(&mut self, _: &VolumeDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.set_volume((self.player.volume() - 5.0).max(0.0));
        cx.notify();
    }

    fn on_seek_back(&mut self, _: &SeekBack, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.seek((self.player.position() - 5.0).max(0.0));
        cx.notify();
    }

    fn on_seek_forward(&mut self, _: &SeekForward, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.seek(self.player.position() + 5.0);
        cx.notify();
    }

    fn on_next_track(&mut self, _: &NextTrack, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.next();
        cx.notify();
    }

    fn on_prev_track(&mut self, _: &PrevTrack, _window: &mut Window, cx: &mut Context<Self>) {
        self.player.previous();
        cx.notify();
    }

    fn on_search_scope_next(&mut self, _: &SearchScopeNext, _window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.move_search_scope(1);
            cx.notify();
        }
    }

    fn on_search_scope_prev(&mut self, _: &SearchScopePrev, _window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.move_search_scope(-1);
            cx.notify();
        }
    }

    fn move_search_scope(&mut self, delta: i32) {
        let Some(search) = self.search.as_mut() else { return };
        let current = search.scope;
        search.scope_cursors[current] = search.cursor();
        let next = (current as i32 + delta).rem_euclid(4) as usize;
        search.scope = next;
        search.scope_cursors[next] = search
            .scope_cursors[next]
            .min(search.scope_count(next).saturating_sub(1));
    }

    fn open_search(&mut self, typing: bool) {
        self.search_open = true;
        self.search = Some(SearchState {
            query: String::new(),
            rows: Vec::new(),
            scope: 0,
            scope_cursors: [0; 4],
            generation: 0,
            loading: false,
            typing,
        });
    }

    fn on_search(&mut self, _: &Search, _window: &mut Window, cx: &mut Context<Self>) {
        self.open_search(true);
        cx.notify();
    }

    fn on_insert_mode(&mut self, _: &InsertMode, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.typing = true;
            self.search_open = true;
        } else {
            self.open_search(true);
        }
        cx.notify();
    }

    fn on_home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        self.line_search.close();
        self.line_search_anim.target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
        self.alt.cancel();
        self.alt_menu = None;
        self.alt_target = None;
        self.alt_spawn.target(0.0, anim::EMPHASIZED_MS, anim::Curve::Accelerate, &self.clock);
        self.search_open = false;
        self.search = None;
        self.queue_open = false;
        self.fullscreen = false;
        self.lyrics_overlay = false;
        self.lyrics_overlay_wanted = false;
        self.description = None;
        self.page = None;
        self.travel = Travel::default();
        self.view.selection.clear();
        cx.notify();
    }

    fn on_search_next(&mut self, _: &SearchNext, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(1);
        }
        cx.notify();
    }

    fn on_search_prev(&mut self, _: &SearchPrev, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(-1);
        }
        cx.notify();
    }

    fn alt_menu_for_row(&self, row: &Row) -> QuickMenu {
        match row.kind {
            RowKind::Song => QuickMenu::song_with(row.starred),
            RowKind::Album => QuickMenu::album_with(
                !row.album_id.is_empty() && self.marks.albums_done.contains(&row.album_id),
                row.starred,
                row.rating.clamp(0, 5) as u8,
            ),
            RowKind::Artist => QuickMenu::artist_with(row.starred, row.rating.clamp(0, 5) as u8),
            RowKind::Playlist => QuickMenu::playlist(),
        }
    }

    fn alt_target_for_row(&self, row: &Row) -> AltTarget {
        AltTarget {
            kind: row.kind,
            id: row.id.clone(),
            album_id: row.album_id.clone(),
            song_id: row.song_id.clone(),
            playlist_id: if row.kind == RowKind::Playlist {
                row.id.clone()
            } else {
                String::new()
            },
        }
    }

    fn alt_anchor_for(
        &self,
        handle: &UniformListScrollHandle,
        index: usize,
        row_h: f32,
    ) -> (f32, f32) {
        let state = handle.0.borrow();
        let bounds = state.base_handle.bounds();
        let offset = state.base_handle.offset();
        let x = f32::from(bounds.origin.x) + f32::from(bounds.size.width) * 0.5;
        let y = f32::from(bounds.origin.y)
            + index as f32 * row_h
            + f32::from(offset.y)
            + row_h * 0.5;
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    fn alt_snapshot(&self) -> Option<(QuickMenu, AltTarget, (f32, f32))> {
        if self.fullscreen {
            let track = self.player.current()?;
            let menu = QuickMenu::song_with(track.favorite);
            let target = AltTarget {
                kind: RowKind::Song,
                id: track.id.clone(),
                album_id: track.album_id.clone(),
                song_id: track.id.clone(),
                playlist_id: String::new(),
            };
            return Some((menu, target, (self.win_w * 0.5, self.win_h * 0.5)));
        }
        if let Some(search) = self.search.as_ref().filter(|_| self.search_open) {
            let scoped = search.scoped();
            let row = scoped.get(search.cursor())?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                (self.win_w * 0.5, self.win_h * 0.5),
            ));
        }
        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let track = queue.get(self.queue_cursor)?;
            let menu = QuickMenu::song_with(track.favorite);
            let target = AltTarget {
                kind: RowKind::Song,
                id: track.id.clone(),
                album_id: track.album_id.clone(),
                song_id: track.id.clone(),
                playlist_id: String::new(),
            };
            return Some((
                menu,
                target,
                self.alt_anchor_for(&self.queue_scroll, self.queue_cursor, self.queue_row_h),
            ));
        }
        if let Some(page) = self.page.as_ref() {
            let row = page.rows.get(page.cursor)?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                self.alt_anchor_for(&self.page_scroll, page.cursor, self.page_row_h),
            ));
        }
        let row = self.view.rows.get(self.view.cursor)?;
        Some((
            self.alt_menu_for_row(row),
            self.alt_target_for_row(row),
            self.alt_anchor_for(&self.scroll, self.view.cursor, self.library_row_h),
        ))
    }

    fn alt_arm(&mut self) {
        if self.login.is_some() || self.line_search.open {
            return;
        }
        let Some((menu, target, anchor)) = self.alt_snapshot() else {
            return;
        };
        if !self.alt.arm(&menu) {
            return;
        }
        if let Some(search) = self.search.as_mut()
            && search.typing
        {
            self.alt_was_typing = true;
            search.typing = false;
        }
        self.alt_menu = Some(menu);
        self.alt_target = Some(target);
        self.alt_anchor = Some(anchor);
        self.alt_spawn.target(1.0, anim::EMPHASIZED_MS, anim::Curve::OutBack, &self.clock);
    }

    fn alt_release(&mut self) {
        if self.alt_was_typing {
            if let Some(search) = self.search.as_mut() {
                search.typing = true;
            }
            self.alt_was_typing = false;
        }
        if self.alt.armed {
            let menu = self.alt_menu.clone();
            if let Some(menu) = menu
                && self.alt.release_hold()
            {
                if let Some(verb) = self.alt.confirm_plan(&menu) {
                    self.dispatch_quick(verb);
                }
                if let Some(verb) = self.alt.commit_rating(&menu) {
                    self.dispatch_quick(verb);
                }
            }
            self.alt.cancel();
        }
        self.alt_spawn.target(0.0, anim::EMPHASIZED_MS, anim::Curve::Accelerate, &self.clock);
    }

    fn dispatch_quick(&mut self, verb: QuickVerb) {
        let Some(target) = self.alt_target.clone() else { return };
        match verb {
            QuickVerb::AddToQueue => {
                self.bridge.quick_queue_add(target.queue_kind().to_string(), target.id.clone());
            }
            QuickVerb::StartRadio => {
                self.bridge.quick_radio(target.queue_kind().to_string(), target.id.clone());
            }
            QuickVerb::Favorite { on } => {
                self.bridge
                    .quick_favorite(target.queue_kind().to_string(), target.id.clone(), on);
            }
            QuickVerb::RateAlbum { level } => {
                self.bridge.quick_rate("album".to_string(), target.id.clone(), level);
            }
            QuickVerb::RateArtist { level } => {
                self.bridge.quick_rate("artist".to_string(), target.id.clone(), level);
            }
            QuickVerb::GoToAlbum => {
                self.bridge.quick_go_album(target.song_id.clone());
            }
            QuickVerb::GoToArtist => {
                self.bridge.quick_go_artist(target.album_id.clone());
            }
            QuickVerb::PlayPlaylist => {
                self.bridge.quick_play_playlist(target.playlist_id.clone());
            }
            QuickVerb::Download { on } => {
                self.bridge
                    .quick_download(target.queue_kind().to_string(), target.id.clone(), on);
            }
        }
    }

    fn on_modifiers_changed(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let alt = event.modifiers.alt;
        if alt && !self.alt_held {
            self.alt_held = true;
            self.alt_arm();
        } else if !alt && self.alt_held {
            self.alt_held = false;
            self.alt_release();
        }
        cx.notify();
    }

    fn on_alt_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        match key {
            "escape" => {
                self.alt.dismiss();
                self.alt_menu = None;
                self.alt_spawn.target(0.0, anim::EMPHASIZED_MS, anim::Curve::Accelerate, &self.clock);
            }
            "h" if shift => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(-1, &menu);
                }
            }
            "l" if shift => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(1, &menu);
                }
            }
            "h" | "left" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::West, &menu, false);
                }
            }
            "j" | "down" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::South, &menu, false);
                }
            }
            "k" | "up" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::North, &menu, false);
                }
            }
            "l" | "right" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::East, &menu, false);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn apply_line_hit(&mut self, hit: line_search::LineHit) {
        if self.fullscreen {
            return;
        }
        if self.search.as_ref().is_some() {
            if let Some(search) = self.search.as_mut() {
                search.scope_cursors[search.scope] = hit.row;
            }
        } else if self.queue_open {
            self.queue_cursor = hit.row.min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
        } else if let Some(page) = self.page.as_mut() {
            page.select(hit.row);
            self.follow_page(true);
        } else {
            self.select(hit.row);
        }
    }

    fn line_search_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let modified = control || event.keystroke.modifiers.alt || event.keystroke.modifiers.platform;
        match key {
            "escape" => {
                self.line_search.close();
                self.line_search_anim.target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
                cx.notify();
                return;
            }
            "enter" => {
                let hit = self.line_search.current_hit();
                if let Some(hit) = hit {
                    self.apply_line_hit(hit);
                    self.line_search.close();
                    self.line_search_anim.target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
                    self.on_enter(&Enter, window, cx);
                } else if self.line_search.query.is_empty() {
                    self.line_search.close();
                    self.line_search_anim.target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
                }
                cx.notify();
                return;
            }
            "tab" => {
                self.line_search.step(1);
                if let Some(hit) = self.line_search.current_hit() {
                    self.apply_line_hit(hit);
                }
                cx.notify();
                return;
            }
            "backtab" => {
                self.line_search.step(-1);
                if let Some(hit) = self.line_search.current_hit() {
                    self.apply_line_hit(hit);
                }
                cx.notify();
                return;
            }
            "up" | "ctrl-p" => {
                self.line_search.step(-1);
                if let Some(hit) = self.line_search.current_hit() {
                    self.apply_line_hit(hit);
                }
                cx.notify();
                return;
            }
            "down" | "ctrl-n" => {
                self.line_search.step(1);
                if let Some(hit) = self.line_search.current_hit() {
                    self.apply_line_hit(hit);
                }
                cx.notify();
                return;
            }
            "backspace" => {
                self.line_search.query.pop();
            }
            "w" if control => {
                line_search::delete_word_back(&mut self.line_search.query);
            }
            _ if !modified => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !text.chars().any(char::is_control)
                {
                    self.line_search.query.push_str(text);
                } else if key == "space" {
                    self.line_search.query.push(' ');
                } else {
                    return;
                }
            }
            _ => return,
        }
        let targets = self.line_search_targets();
        let query = self.line_search.query.clone();
        self.line_search.set_query(&query, &targets);
        if let Some(hit) = self.line_search.current_hit() {
            self.apply_line_hit(hit);
        }
        cx.notify();
    }

    fn activate_search_result(&mut self, cx: &mut Context<Self>) {
        let row = self
            .search
            .as_ref()
            .and_then(|search| search.scoped().get(search.cursor()).cloned().cloned());
        if let Some(row) = row {
            self.search_open = false;
            self.search = None;
            self.open_row_page(&row);
        }
        cx.notify();
    }

    fn search_edited(&mut self) {
        let Some(search) = self.search.as_ref() else { return };
        let query = search.query.clone();
        if query.is_empty() {
            if let Some(search) = self.search.as_mut() {
                search.rows.clear();
                search.loading = false;
            }
        } else {
            if let Some(search) = self.search.as_mut() {
                search.generation = search.generation.wrapping_add(1);
                search.loading = true;
            }
            self.bridge.search(query);
        }
    }

    fn toggle_description(&mut self) {
        if self.description.is_some() {
            self.description = None;
            return;
        }
        let Some(page) = self.page.as_ref() else { return };
        let notes = self
            .notes
            .as_ref()
            .filter(|(id, _)| *id == page.descriptor.id)
            .map(|(_, notes)| notes.clone())
            .unwrap_or_default();
        if notes.is_empty() {
            return;
        }
        self.description = Some(DescriptionState {
            title: page.header.title.clone(),
            subtitle: page.header.subtitle.clone(),
            body: notes,
        });
    }

    fn lyrics_inline(&self) -> bool {
        let inline_width = (self.win_w * 0.95 - self.win_w * 0.012) / 2.0;
        self.lyrics.as_ref().is_some_and(|lyrics| !lyrics.lines.is_empty()) && inline_width >= 300.0
    }

    fn refresh_lyrics_overlay(&mut self) {
        if !self.fullscreen {
            self.lyrics_overlay = false;
            self.lyrics_overlay_wanted = false;
            return;
        }
        let has_lyrics = self
            .lyrics
            .as_ref()
            .is_some_and(|lyrics| !lyrics.lines.is_empty());
        let inline = self.lyrics_inline();
        if !has_lyrics {
            self.lyrics_overlay = false;
            self.lyrics_overlay_wanted = false;
            return;
        }
        if self.lyrics_overlay_wanted && !inline {
            self.lyrics_overlay = true;
        } else if inline {
            self.lyrics_overlay = false;
        }
    }

    fn on_lyrics_overlay(&mut self, _: &LyricsOverlay, _window: &mut Window, cx: &mut Context<Self>) {
        if self.fullscreen {
            if self.lyrics.as_ref().is_some_and(|lyrics| !lyrics.lines.is_empty()) {
                self.lyrics_overlay_wanted = !self.lyrics_overlay_wanted;
                self.refresh_lyrics_overlay();
            }
        } else if self.page.as_ref().is_some_and(|page| {
            self.notes.as_ref().is_some_and(|(id, notes)| *id == page.descriptor.id && !notes.is_empty())
        }) {
            self.toggle_description();
        }
        cx.notify();
    }

    fn line_search_targets(&self) -> Vec<line_search::LineTarget> {
        use line_search::{LineRow, LineTarget};
        let mut targets = Vec::new();
        if self.fullscreen {
            if let Some(lyrics) = self.lyrics.as_ref() {
                targets.push(LineTarget::new(lyrics.lines.iter().map(|(_, line)| Some(LineRow::new(line.clone(), String::new(), String::new()))).collect()));
            }
            return targets;
        }
        if let Some(search) = self.search.as_ref() {
            targets.push(LineTarget::new(search.rows.iter().map(|row| Some(LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone()))).collect()));
            return targets;
        }
        if self.queue_open {
            let queue = self.player.queue_snapshot();
            targets.push(LineTarget::new(queue.iter().map(|track| Some(LineRow::new(track.title.clone(), track.artist.clone(), String::new()))).collect()));
            return targets;
        }
        if let Some(page) = self.page.as_ref() {
            targets.push(LineTarget::new(page.rows.iter().map(|row| Some(LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone()))).collect()));
            return targets;
        }
        let rows = self.view.rows.count;
        let mut lines = vec![None; rows];
        for (offset, row) in self.view.rows.live.iter().enumerate() {
            let index = self.view.rows.live_start + offset;
            if index < rows {
                lines[index] = Some(LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone()));
            }
        }
        targets.push(LineTarget::new(lines));
        targets
    }

    fn line_search_edited(&mut self) {
        let targets = self.line_search_targets();
        let query = self.line_search.query.clone();
        self.line_search.set_query(&query, &targets);
    }

    fn on_line_search(&mut self, _: &LineSearch, _window: &mut Window, cx: &mut Context<Self>) {
        self.line_search.open();
        let targets = self.line_search_targets();
        self.line_search.set_query("", &targets);
        self.line_search_anim.target(1.0, anim::STANDARD_MS, anim::Curve::Decelerate, &self.clock);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &gpui::KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.login.is_some() {
            cx.stop_propagation();
            self.login_key(event, cx);
            return;
        }
        if self.line_search.open {
            cx.stop_propagation();
            self.line_search_key(event, _window, cx);
            return;
        }
        if self.alt_held || self.alt.armed {
            cx.stop_propagation();
            self.on_alt_key(event, cx);
            return;
        }
        if self.search.is_none() || !self.search_open {
            let key = event.keystroke.key.as_str();
            if key == "/" || key == "?" || key == "slash" {
                cx.stop_propagation();
                if event.keystroke.modifiers.shift || key == "?" {
                    self.on_lyrics_overlay(&LyricsOverlay, _window, cx);
                } else {
                    self.on_line_search(&LineSearch, _window, cx);
                }
                return;
            }
            if key == "escape" {
                self.on_back(&Back, _window, cx);
            }
            return;
        }
        let Some(search) = self.search.as_ref() else { return };
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let modified = control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform;
        if !search.typing {
            match key {
                "escape" => {
                    self.search_open = false;
                    cx.notify();
                }
                "i" => {
                    if let Some(search) = self.search.as_mut() {
                        search.typing = true;
                    }
                    cx.notify();
                }
                "enter" => self.activate_search_result(cx),
                "tab" | "l" | "right" => {
                    self.on_search_scope_next(&SearchScopeNext, _window, cx);
                }
                "backtab" | "h" | "left" => {
                    self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                }
                "j" | "down" | "ctrl-n" => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(1);
                    }
                    cx.notify();
                }
                "k" | "up" | "ctrl-p" => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(-1);
                    }
                    cx.notify();
                }
                "g" => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] = 0;
                    }
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        match key {
            "escape" => {
                if let Some(search) = self.search.as_mut() {
                    search.typing = false;
                }
                cx.notify();
                return;
            }
            "enter" => {
                self.activate_search_result(cx);
                return;
            }
            "tab" => {
                self.on_search_scope_next(&SearchScopeNext, _window, cx);
                return;
            }
            "backtab" => {
                self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                return;
            }
            "up" | "ctrl-p" => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(-1);
                }
                cx.notify();
                return;
            }
            "down" | "ctrl-n" => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(1);
                }
                cx.notify();
                return;
            }
            "backspace" => {
                if let Some(search) = self.search.as_mut() {
                    search.query.pop();
                }
            }
            "w" if control => {
                if let Some(search) = self.search.as_mut() {
                    delete_word_back(&mut search.query);
                }
            }
            "h" if control => {
                self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                return;
            }
            "l" if control => {
                self.on_search_scope_next(&SearchScopeNext, _window, cx);
                return;
            }
            _ if !modified => {
                let Some(search) = self.search.as_mut() else { return };
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !text.chars().any(char::is_control)
                {
                    search.query.push_str(text);
                } else if key == "space" {
                    search.query.push(' ');
                } else {
                    return;
                }
            }
            _ => return,
        }
        self.search_edited();
        cx.notify();
    }

    fn on_page_down(&mut self, _: &PageDown, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_add(10));
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_add(10));
        }
        cx.notify();
    }

    fn on_page_up(&mut self, _: &PageUp, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_sub(10));
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_sub(10));
        }
        cx.notify();
    }

    fn on_visual(&mut self, _: &Visual, _window: &mut Window, cx: &mut Context<Self>) {
        let (active, cursor) = if let Some(page) = self.page.as_ref() {
            (page.selection.active(), page.cursor)
        } else {
            (self.view.selection.active(), self.view.cursor)
        };
        if active {
            if let Some(page) = self.page.as_mut() {
                page.selection.clear();
            } else {
                self.view.selection.clear();
            }
        } else if let Some(page) = self.page.as_mut() {
            page.selection.start(cursor);
        } else {
            self.view.selection.start(cursor);
        }
        cx.notify();
    }

    fn on_visual_all(&mut self, _: &VisualAll, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.selection.select_all();
            let last = page.rows.len().saturating_sub(1);
            page.select(last);
        } else {
            self.view.selection.select_all();
            let last = self.view.rows.len().saturating_sub(1);
            self.select(last);
        }
        cx.notify();
    }

    fn on_enter(&mut self, _: &Enter, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open {
            let index = self.queue_cursor;
            self.player.jump(index);
            self.queue_open = false;
            cx.notify();
            return;
        }
        if let Some(search) = self.search.as_ref() {
            let row = search.scoped().get(search.scope_cursors[search.scope]).cloned().cloned();
            if let Some(row) = row {
                self.search_open = false;
                self.search = None;
                self.activate_row(&row);
            }
            cx.notify();
            return;
        }
        let row = if let Some(page) = self.page.as_ref() {
            page.rows.get(page.cursor).cloned()
        } else {
            self.view.rows.get(self.view.cursor).cloned()
        };
        if let Some(row) = row {
            self.activate_row(&row);
        }
        cx.notify();
    }

    fn activate_row(&mut self, row: &Row) {
        if row.song_id.is_empty() && matches!(row.kind, state::RowKind::Song) {
            return;
        }
        if matches!(row.kind, state::RowKind::Song) {
            self.last_play = Some((row.song_id.clone(), self.view.order_label().to_string()));
            self.bridge.play_song(
                row.song_id.clone(),
                self.view.order_label().to_string(),
            );
            return;
        }
        self.open_row_page(row);
    }

    fn on_back(&mut self, _: &Back, _window: &mut Window, cx: &mut Context<Self>) {
        if self.line_search.open {
            self.line_search.close();
            self.line_search_anim.target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
            cx.notify();
            return;
        }
        if self.description.is_some() {
            self.description = None;
            cx.notify();
            return;
        }
        if self.lyrics_overlay {
            self.lyrics_overlay = false;
            self.lyrics_overlay_wanted = false;
            cx.notify();
            return;
        }
        if self.fullscreen || self.fullscreen_anim.value(&self.clock) > 0.01 {
            self.fullscreen = false;
            cx.notify();
            return;
        }
        if self.queue_open || self.queue_anim.value(&self.clock) > 0.01 {
            self.queue_open = false;
            cx.notify();
            return;
        }
        if self.search.is_some() {
            self.search_open = false;
            cx.notify();
            return;
        }
        if self.page.as_ref().is_some_and(|page| page.selection.active()) {
            if let Some(page) = self.page.as_mut() {
                page.selection.clear();
            }
            cx.notify();
            return;
        }
        if self.view.selection.active() && self.page.is_none() {
            self.view.selection.clear();
            cx.notify();
            return;
        }
        if self.page.is_some() {
            self.back_page();
            cx.notify();
            return;
        }
        let now = self.clock.now;
        if self.quit_armed_at.is_some_and(|at| now - at <= 0.25) {
            self.quit_armed_at = None;
            self.exiting = Some(now);
            cx.notify();
            return;
        }
        self.quit_armed_at = Some(now);
        cx.notify();
    }

    fn on_prev_place(&mut self, _: &PrevPlace, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_travel(-1);
        cx.notify();
    }

    fn on_next_place(&mut self, _: &NextPlace, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_travel(1);
        cx.notify();
    }

    fn open_row_page(&mut self, row: &Row) {
        let kind = match row.kind {
            state::RowKind::Album => PageKind::Album,
            state::RowKind::Artist => PageKind::Artist,
            state::RowKind::Playlist => PageKind::Playlist,
            state::RowKind::Song => return,
        };
        self.open_descriptor(PageDescriptor { kind, id: row.id.clone() });
    }

    fn open_descriptor(&mut self, descriptor: PageDescriptor) {
        match descriptor.kind {
            PageKind::Album => self.bridge.request_notes(descriptor.id.clone()),
            PageKind::Artist => self.bridge.request_artist_notes(descriptor.id.clone()),
            PageKind::Playlist => {}
        }
        self.remember_current();
        self.hovered_page = None;
        self.travel.open(descriptor.clone());
        self.page = Some(PageState::new(descriptor.clone()));
        self.page_generation = self.page_generation.wrapping_add(1);
        self.bridge.load_page(descriptor);
        self.follow_page(false);
    }

    fn back_page(&mut self) {
        self.remember_current();
        match self.travel.back() {
            Some(descriptor) => {
                self.hovered_page = None;
                self.page = Some(PageState::new(descriptor.clone()));
                self.page_generation = self.page_generation.wrapping_add(1);
                self.bridge.load_page(descriptor);
                self.follow_page(false);
            }
            None => self.page = None,
        }
    }

    fn step_travel(&mut self, delta: i32) {
        self.remember_current();
        if let Some(descriptor) = self.travel.step(delta) {
            self.page = Some(PageState::new(descriptor.clone()));
            self.page_generation = self.page_generation.wrapping_add(1);
            self.bridge.load_page(descriptor);
            self.follow_page(false);
        }
    }

    fn cycle_order(&mut self, next: bool) {
        let len = self.view.order_len();
        if len <= 1 {
            return;
        }
        self.view.order = if next {
            (self.view.order + 1) % len
        } else {
            (self.view.order + len - 1) % len
        };
        self.reload();
    }

    fn remember_current(&self) {
        if let Some(page) = self.page.as_ref() {
            let key = page_key(&page.descriptor);
            crate::scroll_memory::remember(
                &key,
                page.cursor as i64,
                0.0,
                page.descriptor.kind as i64,
                "list",
            );
        } else {
            crate::scroll_memory::remember(
                self.view.scope.key(),
                self.view.cursor as i64,
                0.0,
                self.view.scope as i64,
                "list",
            );
        }
    }

    fn top_bar(&self, _cx: &Context<Self>) -> impl IntoElement {
        let (welcome, offline) = match self.header.as_ref() {
            Some(header) => (
                format!("Welcome, {}", header.username),
                false,
            ),
            None => ("Welcome".to_string(), false),
        };
        let report = match self.header.as_ref() {
            Some(header) => {
                let sync = header
                    .last_sync
                    .as_deref()
                    .map(last_sync_text)
                    .unwrap_or_else(|| "—".to_string());
                let provider = if header.server_type.is_empty() {
                    String::new()
                } else {
                    let mut name = header.server_type.clone();
                    if let Some(first) = name.get_mut(0..1) {
                        first.make_ascii_uppercase();
                    }
                    format!(", {name}")
                };
                format!("Last Sync: {sync}{provider}")
            }
            None => String::new(),
        };
        div()
            .relative()
            .w_full()
            .h(relative(0.0648))
            .px_4()
            .bg(rgb(0x1D1B20))
            .flex()
            .items_center()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(19.0))
                            .text_color(rgb(if offline { 0xEFB8C8 } else { ON_VARIANT }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(welcome),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(rgb(ON_VARIANT))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(report),
                    )
                    .child(icon("reload-ui-2-svgrepo-com.svg", 18.0, ON_SURFACE))
                    .child(icon("delete-outline.svg", 18.0, ON_SURFACE)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.0))
                    .bg(rgb(OUTLINE)),
            )
    }

    fn list(&self, cx: &Context<Self>) -> impl IntoElement {
        let title = self.view.scope.label();
        let hint = "(Shift-J/K) cycle filter";
        let order = format!("Shift-H / Shift-L  /  {}", self.view.order_label());
        let selection_count = self.view.selection.count(self.view.cursor);
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .w_full()
            .h(px(72.0))
            .px_2()
            .child(div().text_size(px(26.0)).font_weight(gpui::FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(ON_VARIANT))
                    .child(hint),
            )
            .child(div().flex_1())
            .child(icon("outline-shuffle.svg", 14.0, ON_VARIANT))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(ON_VARIANT))
                    .child(order),
            )
            .when(selection_count > 0, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .bg(rgb(PRIMARY))
                        .text_size(px(11.0))
                        .text_color(rgb(0x381E72))
                        .child(selection_count.to_string()),
                )
            });

        if self.view.rows.is_empty() {
            let message = if self.view.loading {
                "Loading…".to_string()
            } else if let Some(error) = &self.error {
                error.clone()
            } else {
                "Nothing here".to_string()
            };
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .px(px(12.0))
                .child(header)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_size(px(15.0))
                        .text_color(rgb(ON_VARIANT))
                        .child(message),
                );
        }

        let count = self.view.rows.len();
        let scroll = self.scroll.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .px(px(12.0))
            .child(header)
            .child(
                uniform_list("library", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let row_h = this.library_row_h;
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let selected = index == this.view.cursor
                            || this.view.selection.contains(this.view.cursor, index);
                        let hovered = this.hovered_row == Some(index);
                        let emphasis = if selected || hovered { 1.08 } else { 1.0 };
                        let striped = index % 2 != 0;
                        let target = if selected {
                            0x4A4458FF
                        } else if hovered {
                            0x2B2930FF
                        } else if striped {
                            0x0F0D13FF
                        } else {
                            0x14121800
                        };
                        let Some(row) = this.view.rows.get(index) else {
                            // The QML RowWindow paints a blank geometry slot
                            // until the live window lands.
                            items.push(
                                div()
                                    .id(index)
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .h(px(row_h))
                                    .px_1()
                                    .rounded_sm()
                                    .bg(rgb(target)),
                            );
                            continue;
                        };
                        let title = row.title.clone();
                        let subtitle = row.subtitle.clone();
                        let meta = row.meta.clone();
                        let starred = row.starred;
                        let downloaded = this.marks.downloaded(&row.song_id, &row.album_id) || row.downloaded;
                        let rating = row.rating;
                        let cover = cover_path(&row.cover_url);
                        let color = this
                            .row_colors
                            .entry(row.id.clone())
                            .or_insert_with(|| anim::ColorBehavior::new(target))
                            .target(target, anim::QUICK_MS, anim::Curve::Standard, &this.clock);
                        items.push(
                            div()
                                .id(index)
                                .w_full()
                                .flex()
                                .items_center()
                                .h(px(row_h))
                                .px_1()
                                .rounded_sm()
                                .bg(rgba(color))
                                .cursor_pointer()
                                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                    if *hovered {
                                        this.hovered_row = Some(index);
                                    } else if this.hovered_row == Some(index) {
                                        this.hovered_row = None;
                                    }
                                    cx.notify();
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(index);
                                    cx.notify();
                                }))
                                .child(art(cover, row_h * 0.78))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .flex_1()
                                        .ml_3()
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .text_size(px(18.0 * emphasis))
                                                .font_weight(if selected { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                                                .whitespace_normal()
                                                .max_h(px(34.0))
                                                .overflow_hidden()
                                                .child(highlighted_text(&title, &this.line_search.query, ON_SURFACE)),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.0 * emphasis))
                                                .text_color(rgb(ON_VARIANT))
                                                .whitespace_normal()
                                                .max_h(px(22.0))
                                                .overflow_hidden()
                                                .child(highlighted_text(&subtitle, &this.line_search.query, ON_VARIANT)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_size(px(11.0 * emphasis))
                                                .text_color(rgb(ON_VARIANT))
                                                .child(highlighted_text(&meta, &this.line_search.query, ON_VARIANT)),
                                        )
                                        .when(downloaded, |this| {
                                            this.child(icon("outline-download.svg", 13.0, PRIMARY))
                                        })
                                        .when(starred, |this| {
                                            this.child(icon("baseline-favorite.svg", 12.0, 0xFFD54F))
                                        })
                                        .when(rating > 0, |this| {
                                            this.child(
                                                div()
                                                    .text_size(px(10.0))
                                                    .text_color(rgb(PRIMARY))
                                                    .child(rating.to_string()),
                                            )
                                        }),
                                ),
                        );
                    }
                    items
                }))
                .h_full()
                .w_full()
                .on_scroll_wheel(cx.listener(|this, _, _, _| {
                    this.library_follow.active = false;
                }))
                .track_scroll(scroll),
            )
    }

    fn description_overlay(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let description = self.description.as_ref()?;
        Some(
            div()
                .id("description-scrim")
                .absolute()
                .inset_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::black().opacity(0.50))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.description = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .w(relative(0.72))
                        .max_w(px(720.0))
                        .h_full()
                        .rounded_md()
                        .bg(gpui::black().opacity(0.72))
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(24.0))
                                .font_weight(gpui::FontWeight::BOLD)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(description.title.clone()),
                        )
                        .when(!description.subtitle.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(description.subtitle.clone()),
                            )
                        })
                        .child(
                            div()
                                .id("description-body")
                                .flex_1()
                                .w_full()
                                .overflow_y_scroll()
                                .text_size(px(14.0))
                                .text_color(rgb(ON_SURFACE))
                                .whitespace_normal()
                                .child(description.body.clone()),
                        ),
                ),
        )
        .map(|overlay| overlay.into_any_element())
    }

    fn alt_overlay(&self, progress: f32) -> gpui::AnyElement {
        let (Some(menu), Some((ax, ay))) = (self.alt_menu.as_ref(), self.alt_anchor) else {
            return div().into_any_element();
        };
        let side = 54.0 * (0.65 + 0.35 * progress);
        let gap = 72.0;
        let mut overlay = div().absolute().inset_0().size_full();
        for direction in Direction::ALL {
            let Some(slot) = menu.slot(direction) else { continue };
            let aimed = self.alt.aimed == Some(direction);
            let (cx, cy) = match direction {
                Direction::North => (ax, ay - gap),
                Direction::South => (ax, ay + gap),
                Direction::West => (ax - gap, ay),
                Direction::East => (ax + gap, ay),
            };
            let size = if aimed { side * 1.30 } else { side };
            let index = self.alt.action_index[direction.index()];
            let label = slot.active_label(index).to_string();
            let icon_name = slot.active_icon(index).to_string();
            let disabled = slot.disabled;
            let bg = if disabled {
                0x1D1B20
            } else if aimed {
                0x4F378B
            } else {
                0x2B2930
            };
            let fg = if disabled {
                0x49454F
            } else if aimed {
                0xEADDFF
            } else {
                0xCAC4D0
            };
            let mut tile = div()
                .absolute()
                .left(px(cx - size * 0.5))
                .top(px(cy - size * 0.5))
                .w(px(size))
                .h(px(size))
                .rounded_md()
                .border_1()
                .border_color(rgb(if aimed { PRIMARY } else { OUTLINE }))
                .bg(rgb(bg))
                .opacity(progress)
                .flex()
                .items_center()
                .justify_center();
            if icon_name.is_empty() {
                tile = tile.child(
                    div()
                        .text_size(px(16.0))
                        .text_color(rgb(fg))
                        .child(label),
                );
            } else {
                tile = tile.child(icon(&icon_name, size * 0.46, fg));
            }
            if direction == Direction::South && slot.is_rating_slot() {
                tile = tile.child(
                    div()
                        .absolute()
                        .right(px(4.0))
                        .bottom(px(3.0))
                        .text_size(px(10.0))
                        .text_color(rgb(PRIMARY))
                        .child(self.alt.rating_value.to_string()),
                );
            }
            overlay = overlay.child(tile);
        }
        overlay.into_any_element()
    }

    fn line_search_overlay(&self, progress: f32) -> gpui::Div {
        let counter = match (self.line_search.current_hit(), self.line_search.candidates.len()) {
            (Some(_), count) if count > 0 => {
                format!("[{}/{}]", self.line_search.index.unwrap_or(0) + 1, count)
            }
            _ => String::new(),
        };
        div()
            .absolute()
            .inset_0()
            .size_full()
            .bg(gpui::black().opacity(0.50 * progress))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-(1.0 - progress) * 96.0))
                    .h(relative(0.07))
                    .bg(rgb(SURFACE))
                    .border_t_1()
                    .border_color(rgb(PRIMARY))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(PRIMARY))
                            .child("/"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(16.0))
                            .text_color(rgb(ON_SURFACE))
                            .child(self.line_search.query.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(if counter.is_empty() {
                                "Tab cycle · Enter jump · Esc close".to_string()
                            } else {
                                counter
                            }),
                    ),
            )
    }

    fn lyrics_overlay_element(&self) -> gpui::AnyElement {
        if !self.lyrics_overlay {
            return div().into_any_element();
        }
        let Some(lyrics) = self.lyrics.as_ref() else {
            return div().into_any_element();
        };
        let position = self.player.position();
        let active = if lyrics.synced {
            let ms = (position * 1000.0) as i64;
            lyrics
                .lines
                .iter()
                .rposition(|(start, _)| start.is_some_and(|start| start <= ms))
                .unwrap_or(0)
        } else {
            0
        };
        let start = active.saturating_sub(6);
        let end = (active + 12).min(lyrics.lines.len());
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.35))
            .child(
                div()
                    .w(relative(0.72))
                    .max_w(px(720.0))
                    .h_full()
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(format!("Lyrics · {} · ? closes", lyrics.source)),
                    )
                    .children(lyrics.lines[start..end].iter().enumerate().map(|(offset, (_, text))| {
                        let index = start + offset;
                        let is_active = index == active;
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(if is_active { 18.0 } else { 14.0 }))
                            .text_color(rgb(if is_active { ON_SURFACE } else { ON_VARIANT }))
                            .text_center()
                            .whitespace_normal()
                            .when(is_active, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                            .child(text.clone())
                    })),
            )
            .into_any_element()
    }

    fn fullscreen_overlay(&self, progress: f32) -> gpui::Div {
        let track = self.player.current().cloned();
        let (title, artist, cover) = match &track {
            Some(track) => (
                track.title.clone(),
                track.artist.clone(),
                self.display_cover(&track.cover_art, 512)
                    .or_else(|| cover_path(&track.cover_url)),
            ),
            None => ("No track playing".to_string(), String::new(), None),
        };
        let position = self.player.position();
        let duration = self.player.duration();
        let progress_ratio = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let lyrics = self.lyrics.as_ref();
        let lines = lyrics.map(|lyrics| lyrics.lines.clone()).unwrap_or_default();
        let active = lyrics
            .filter(|lyrics| lyrics.synced)
            .and_then(|_| {
                let ms = (position * 1000.0) as i64;
                lines
                    .iter()
                    .rposition(|(start, _)| start.is_some_and(|start| start <= ms))
            })
            .unwrap_or(0);
        let start = active.saturating_sub(5);
        let end = (active + 10).min(lines.len());
        let visible: Vec<(usize, String)> = lines
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|(index, (_, text))| (index, text.clone()))
            .collect();
        let lyrics_label = match lyrics {
            Some(lyrics) if lyrics.lines.is_empty() => "No lyrics".to_string(),
            Some(lyrics) => format!("Lyrics · {}", lyrics.source),
            None => "Loading lyrics…".to_string(),
        };
        div()
            .absolute()
            .inset_0()
            .size_full()
            .opacity(progress)
            .bg(rgb(BG))
            .flex()
            .flex_col()
            .p(px(16.0))
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .w_full()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .w(relative(0.50))
                            .h_full()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .child(art(cover, 300.0)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .w(relative(0.50))
                            .h_full()
                            .min_w_0()
                            .overflow_hidden()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(lyrics_label),
                            )
                            .children(visible.into_iter().map(|(index, text)| {
                                let is_active = index == active;
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_size(px(if is_active { 18.0 } else { 14.0 }))
                                    .text_color(rgb(if is_active { ON_SURFACE } else { ON_VARIANT }))
                                    .text_center()
                                    .whitespace_normal()
                                    .when(is_active, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                                    .child(text)
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .w_full()
                            .text_size(px(24.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_size(px(13.0))
                            .text_color(rgb(ON_VARIANT))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(artist),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(format_time(position)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(rgb(SURFACE_HIGH))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(progress_ratio))
                                            .rounded_full()
                                            .bg(rgb(PRIMARY)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(format_time(duration)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_5()
                            .child(icon("skip-previous-solid-svgrepo-com.svg", 22.0, ON_SURFACE))
                            .child(if self.player.state() == crate::player::State::Playing {
                                icon("pause-svgrepo-com.svg", 28.0, ON_SURFACE)
                            } else {
                                icon("play-1001-svgrepo-com.svg", 28.0, ON_SURFACE)
                            })
                            .child(icon("skip-next-circle-solid-svgrepo-com.svg", 22.0, ON_SURFACE)),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .text_center()
                            .child("Shift-Enter or q closes · Space play/pause · Ctrl-Q queue"),
                    ),
            )
    }

    fn queue_overlay(&self, cx: &Context<Self>, progress: f32) -> gpui::Div {
        let count = self.player.len();
        let current = self.player.index();
        let cursor = self.queue_cursor;
        let scroll = self.queue_scroll.clone();
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .justify_end()
            .bg(gpui::black().opacity(0.35 * progress))
            .opacity(progress)
            .child(
                div()
                    .h_full()
                    .w(relative(0.36))
                    .flex()
                    .flex_col()
                    .bg(rgb(SURFACE))
                    .border_l_1()
                    .border_color(rgb(OUTLINE))
                    .shadow_lg()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .py_3()
                            .text_size(px(15.0))
                            .child("Queue")
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(format!("{} / {}", current.saturating_add(1), count)),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(rgb(OUTLINE)))
                    .child(
                        uniform_list("queue", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            let queue = this.player.queue_snapshot();
                            let mut items = Vec::with_capacity(range.len());
                            for index in range {
                                let Some(track) = queue.get(index) else { break };
                                let title = track.title.clone();
                                let artist = track.artist.clone();
                                let cover = cover_path(&track.cover_url);
                                let active = index == cursor;
                                let playing = index == this.player.index();
                                let emphasis = if active { 1.08 } else { 1.0 };
                                let cover_scale = this
                                    .row_cover_scales
                                    .entry(format!("queue:{index}"))
                                    .or_insert_with(|| anim::Behavior::new(1.0))
                                    .target(if active { 1.20 } else { 1.0 }, anim::QUICK_MS, anim::Curve::Standard, &this.clock);
                                items.push(
                                    div()
                                        .id(index)
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .h(px(this.queue_row_h))
                                        .px_3()
                                        .cursor_pointer()
                                        .when(active, |this| this.bg(rgb(0x4A4458)))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.queue_cursor = index;
                                            this.player.jump(index);
                                            cx.notify();
                                        }))
                                        .child(art_scaled(cover, this.queue_row_h * 0.714, cover_scale))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .min_w_0()
                                                .flex_1()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .text_size(px(14.0 * emphasis))
                                                                .font_weight(if active { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                                                                .when(playing, |this| this.text_color(rgb(PRIMARY)))
                                                                .whitespace_normal()
                                                                .max_h(px(20.0))
                                                                .overflow_hidden()
                                                                .child(title),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(10.0 * emphasis))
                                                        .text_color(rgb(ON_VARIANT))
                                                        .whitespace_normal()
                                                        .max_h(px(16.0))
                                                        .overflow_hidden()
                                                        .child(artist),
                                                ),
                                        ),
                                );
                            }
                            items
                        }))
                        .h_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.queue_follow.active = false;
                        }))
                        .track_scroll(scroll),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .child("Enter jumps · d d removes · Ctrl-Shift-J/K reorders · q closes"),
                    ),
            )
    }

    fn search_overlay(&self, cx: &Context<Self>, search: &SearchState, progress: f32) -> gpui::Div {
        let query = if search.query.is_empty() {
            "Search artists, albums, songs".to_string()
        } else if search.typing {
            format!("{}▌", search.query)
        } else {
            search.query.clone()
        };
        let results = search
            .scoped()
            .into_iter()
            .take(12)
            .enumerate()
            .map(|(index, row)| {
                let selected = index == search.cursor();
                let hovered = self.hovered_row == Some(index);
                let emphasis = if selected || hovered { 1.08 } else { 1.0 };
                let cover = cover_path(&row.cover_url);
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_5()
                    .id(("search-row", index))
                    .h(px(self.search_row_h))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.hovered_row = Some(index);
                        } else if this.hovered_row == Some(index) {
                            this.hovered_row = None;
                        }
                        cx.notify();
                    }))
                    .when(selected, |this| this.bg(rgb(0x4A4458)))
                    .when(hovered && !selected, |this| this.bg(rgb(SURFACE_HIGH)))
                    .child(art_scaled(
                        cover,
                        self.search_row_h * 0.714,
                        if selected || hovered { 1.20 } else { 1.0 },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .text_size(px(16.0 * emphasis))
                                    .font_weight(if selected { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                                    .whitespace_normal()
                                    .max_h(px(22.0))
                                    .overflow_hidden()
                                    .child(row.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0 * emphasis))
                                    .text_color(rgb(ON_VARIANT))
                                    .whitespace_normal()
                                    .max_h(px(18.0))
                                    .overflow_hidden()
                                    .child(row.subtitle.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(row.meta.clone()),
                    )
            })
            .collect::<Vec<_>>();

        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .bg(gpui::black().opacity(0.50))
            .opacity(progress)
            .child(
                div()
                    .mt(px((1.0 - progress) * 24.0))
                    .w(relative(0.69))
                    .h(relative(0.72))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(PRIMARY))
                    .bg(rgb(0x211F26))
                    .overflow_hidden()
                    .shadow_lg()
                    .child(
                        div()
                            .h(px(86.0))
                            .px_5()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .h(px(60.0))
                                    .rounded_md()
                                    .bg(rgb(0x36343B))
                                    .px_4()
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_normal()
                                    .max_h(px(44.0))
                                    .text_size(px(16.0))
                                    .text_color(if search.query.is_empty() { rgb(ON_VARIANT) } else { rgb(ON_SURFACE) })
                                    .child(query),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .h(px(52.0))
                            .px_5()
                            .children((0..4).map(|scope| {
                                let active = scope == search.scope;
                                let count = search.scope_count(scope);
                                div()
                                    .text_size(px(14.0))
                                    .text_color(rgb(if active { PRIMARY } else { ON_VARIANT }))
                                    .child(format!("{} {count}", search_scope_label(scope)))
                            })),
                    )
                    .child(div().h(px(1.0)).bg(rgb(OUTLINE)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_hidden()
                            .children(results),
                    )
                    .child(
                        div()
                            .h(px(72.0))
                            .w_full()
                            .px_4()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .text_center()
                            .whitespace_normal()
                            .child(if search.typing {
                                "Type to filter · Ctrl-H/L or Tab scope · ↑/↓ rows ·\nEnter open/play · Esc normal mode"
                            } else {
                                "Shift-H/L or Tab scope · j/k rows · i type · Enter open/play · Esc close"
                            }),
                    ),
            )
    }

    fn page_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(page) = self.page.as_ref() else {
            return div().flex().flex_1();
        };
        let title = page.header.title.clone();
        let subtitle = page.header.subtitle.clone();
        let meta = page.header.meta.clone();
        let cover = self
            .display_cover(&page.header.cover_art, 512)
            .or_else(|| cover_path(&page.header.cover_url));
        let notes = self
            .notes
            .as_ref()
            .filter(|(album_id, _)| *album_id == page.descriptor.id)
            .map(|(_, notes)| notes.clone())
            .unwrap_or_default();
        let desc_title = title.clone();
        let desc_subtitle = subtitle.clone();
        let header_h = if notes.is_empty() { 220.0 } else { 320.0 };
        let spacer_h = if notes.is_empty() { 220.0 } else { 0.0 };

        let header = div()
            .relative()
            .flex_none()
            .w_full()
            .h(px(header_h))
            .px(px(12.0))
            .child(
                div()
                    .absolute()
                    .left(px(12.0))
                    .top(px(18.0))
                    .w(relative(0.78))
                    .p_3()
                    .rounded_md()
                    .bg(gpui::black().opacity(0.40))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(30.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(rgb(ON_VARIANT))
                            .child("☆ ☆ ☆ ☆ ☆"),
                    )
                    .child(
                        div()
                            .text_size(px(16.0))
                            .text_color(rgb(PRIMARY))
                            .child(subtitle),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(ON_VARIANT))
                            .whitespace_nowrap()
                            .child(meta),
                    )
                    .when(!notes.is_empty(), |this| {
                        let title = desc_title.clone();
                        let subtitle = desc_subtitle.clone();
                        let body = notes.clone();
                        this.child(
                            div()
                                .id("album-notes")
                                .mt_1()
                                .max_h(px(64.0))
                                .overflow_hidden()
                                .text_size(px(11.0))
                                .text_color(rgb(ON_VARIANT))
                                .whitespace_normal()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.description = Some(DescriptionState {
                                        title: title.clone(),
                                        subtitle: subtitle.clone(),
                                        body: body.clone(),
                                    });
                                    cx.notify();
                                }))
                                .child(notes.clone()),
                        )
                    }),
            );

        if page.loading && page.rows.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .child(header)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_size(px(15.0))
                        .text_color(rgb(ON_VARIANT))
                        .child("Loading…"),
                );
        }
        if let Some(error) = &page.error {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .child(header)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.0))
                        .text_color(rgb(ON_VARIANT))
                        .child(error.clone()),
                );
        }

        let count = page.rows.len();
        let scroll = self.page_scroll.clone();
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .child(
                div()
                    .absolute()
                    .top(px(48.0))
                    .left_0()
                    .right_0()
                    .h(px(420.0))
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(art(cover, 390.0)),
            )
            .child(header)
            .child(div().h(px(spacer_h)))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .child(
                        uniform_list("page-rows", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            let row_h = 36.0;
                            let mut items = Vec::with_capacity(range.len());
                            for index in range {
                                let Some((title, subtitle, meta, selected, rating, starred, downloaded, key)) = this
                                    .page
                                    .as_ref()
                                    .map(|page| {
                                        let row = &page.rows[index];
                                        (
                                            row.title.clone(),
                                            row.subtitle.clone(),
                                            row.meta.clone(),
                                            index == page.cursor
                                                || page.selection.contains(page.cursor, index),
                                            row.rating,
                                            row.starred,
                                            row.downloaded,
                                            format!("page:{}:{}", page.descriptor.id, row.id),
                                        )
                                    })
                                else {
                                    break;
                                };
                                let hovered = this.hovered_page == Some(index);
                                let target = if selected {
                                    0x4A4458FF
                                } else if hovered {
                                    0x2B2930FF
                                } else {
                                    0x141218FF
                                };
                                let emphasis = if selected || hovered { 1.08 } else { 1.0 };
                                let color = this
                                    .row_colors
                                    .entry(key)
                                    .or_insert_with(|| anim::ColorBehavior::new(target))
                                    .target(target, anim::QUICK_MS, anim::Curve::Standard, &this.clock);
                                items.push(
                                    div()
                                        .id(index)
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .h(px(row_h))
                                        .px_4()
                                        .cursor_pointer()
                                        .bg(rgba(color))
                                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                            if *hovered {
                                                this.hovered_page = Some(index);
                                            } else if this.hovered_page == Some(index) {
                                                this.hovered_page = None;
                                            }
                                            cx.notify();
                                        }))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(page) = this.page.as_mut() {
                                                page.select(index);
                                            }
                                            this.follow_page(true);
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .min_w_0()
                                                .flex_1()
                                                .child(
                                                    div()
                                                        .text_size(px(15.0 * emphasis))
                                                        .font_weight(if selected { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                                                        .whitespace_nowrap()
                                                        .text_ellipsis()
                                                        .child(title),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0 * emphasis))
                                                        .text_color(rgb(ON_VARIANT))
                                                        .whitespace_nowrap()
                                                        .text_ellipsis()
                                                        .child(subtitle),
                                                ),
                                        )
                                        .child(
                                            div()
                                                    .text_size(px(11.0 * emphasis))
                                                .text_color(rgb(ON_VARIANT))
                                                .child(meta),
                                        )
                                        .when(downloaded, |this| {
                                            this.child(icon("outline-download.svg", 12.0, PRIMARY))
                                        })
                                        .when(starred, |this| {
                                            this.child(icon("baseline-favorite.svg", 11.0, 0xFFD54F))
                                        })
                                        .when(rating > 0, |this| {
                                            this.child(div().text_size(px(10.0)).text_color(rgb(PRIMARY)).child(rating.to_string()))
                                        }),
                                );
                            }
                            items
                        }))
                        .h_full()
                        .w_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.page_follow.active = false;
                        }))
                        .track_scroll(scroll),
                    ),
            )
    }

    fn player_bar(&self) -> impl IntoElement {
        let track = self.player.current().cloned();
        let state = self.player.state();
        let position = self.player.position();
        let duration = self.player.duration();
        let volume = self.player.volume();
        let muted = self.player.muted();
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let (title, artist, cover) = match &track {
            Some(track) => (track.title.clone(), track.artist.clone(), cover_path(&track.cover_url)),
            None => ("Nothing playing".to_string(), String::new(), None),
        };
        div()
            .relative()
            .w_full()
            .h(relative(0.0828))
            .bg(rgb(SURFACE))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(3.0))
                    .bg(rgb(SURFACE_HIGH))
                    .child(div().h_full().w(relative(progress)).bg(rgb(PRIMARY))),
            )
            .child(
                div()
                    .flex()
                    .size_full()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .max_w(relative(0.46))
                            .overflow_hidden()
                            .child(art(cover, 44.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(15.0))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(rgb(ON_VARIANT))
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(artist),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .w(px(200.0))
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .gap_5()
                            .child(icon("skip-previous-solid-svgrepo-com.svg", 20.0, ON_SURFACE))
                            .child(if state == crate::player::State::Playing {
                                icon("pause-svgrepo-com.svg", 24.0, ON_SURFACE)
                            } else {
                                icon("play-1001-svgrepo-com.svg", 24.0, ON_SURFACE)
                            })
                            .child(icon("skip-next-circle-solid-svgrepo-com.svg", 20.0, ON_SURFACE)),
                    )
                    .child(
                        div()
                            .flex()
                            .w(px(170.0))
                            .flex_none()
                            .items_center()
                            .justify_end()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(format!(
                                        "{} / {}",
                                        format_time(position),
                                        format_time(duration)
                                    )),
                            )
                            .when(self.repeat != crate::player::Repeat::Off, |this| {
                                this.child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(rgb(PRIMARY))
                                        .child(repeat_label(self.repeat)),
                                )
                            })
                            .child(icon(
                                if muted { "vol-mute-svgrepo-com.svg" }
                                else if volume <= 1.0 { "vol-zero-svgrepo-com.svg" }
                                else if volume <= 33.0 { "vol-low-svgrepo-com.svg" }
                                else { "vol-high-svgrepo-com.svg" },
                                16.0,
                                ON_VARIANT,
                            ))
                            .child(
                                div()
                                    .w(px(54.0))
                                    .h(px(3.0))
                                    .rounded_full()
                                    .bg(rgb(SURFACE_HIGH))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative((volume / 100.0) as f32))
                                            .rounded_full()
                                            .bg(rgb(PRIMARY)),
                                    ),
                            ),
                    ),
            )
    }

    fn status_bar(&self) -> impl IntoElement {
        let counts = self
            .header
            .as_ref()
            .map(|header| header.counts)
            .unwrap_or((0, 0, 0));
        div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(66.0))
            .bg(rgb(SURFACE))
            .text_size(px(11.0))
            .text_color(rgb(ON_VARIANT))
            .child({
                let visual = if let Some(page) = self.page.as_ref() {
                    page.selection.count(page.cursor)
                } else {
                    self.view.selection.count(self.view.cursor)
                };
                if visual > 0 {
                    format!("VISUAL {visual} · {} · {} · {}/{}", self.view.scope.label(), self.view.order_label(), counts.0, counts.1)
                } else {
                    format!(
                        "{} · {} · {} artists / {} albums / {} songs",
                        self.view.scope.label(),
                        self.view.order_label(),
                        counts.0,
                        counts.1,
                        counts.2
                    )
                }
            })
    }
}

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames = self.frames.wrapping_add(1);
        self.win_w = f32::from(window.viewport_size().width);
        self.win_h = f32::from(window.viewport_size().height);
        self.refresh_lyrics_overlay();
        if self.trace && self.frames.is_multiple_of(120) {
            crate::debug_log!("GPUI trace: frame {}", self.frames);
        }
        self.clock.tick();
        if self.exiting.is_none()
            && let Some(ms) = self.quit_after_ms
            && self.clock.now * 1000.0 >= ms as f64
        {
            self.exiting = Some(self.clock.now);
        }
        if self.login.is_some() {
            return self.login_view().into_any_element();
        }
        if self.player.state() != crate::player::State::Stopped
            && let Some(message) = self.player.pump()
        {
            self.playback_error = Some(message);
        }
        if self.player.state() == crate::player::State::Playing {
            window.request_animation_frame();
        }
        let progress = self.search_anim.target(
            if self.search_open { 1.0 } else { 0.0 },
            anim::STANDARD_MS,
            if self.search_open { anim::Curve::Decelerate } else { anim::Curve::Accelerate },
            &self.clock,
        );
        if self.search.is_some() && !self.search_open && progress <= 0.001 {
            self.search = None;
        }
        if self.search.is_some() && !self.search_anim.settled(&self.clock) {
            window.request_animation_frame();
        }
        let line_progress = self.line_search_anim.target(
            if self.line_search.open { 1.0 } else { 0.0 },
            anim::STANDARD_MS,
            if self.line_search.open { anim::Curve::Decelerate } else { anim::Curve::Accelerate },
            &self.clock,
        );
        if !self.line_search_anim.settled(&self.clock) {
            window.request_animation_frame();
        }
        let line_overlay = (self.line_search.open || line_progress > 0.001)
            .then(|| self.line_search_overlay(line_progress));
        let alt_progress = self.alt_spawn.target(
            if self.alt.armed { 1.0 } else { 0.0 },
            anim::EMPHASIZED_MS,
            if self.alt.armed { anim::Curve::OutBack } else { anim::Curve::Accelerate },
            &self.clock,
        );
        if !self.alt_spawn.settled(&self.clock) {
            window.request_animation_frame();
        }
        if !self.alt.armed && alt_progress <= 0.001 {
            self.alt_menu = None;
            self.alt_target = None;
        }
        let alt_overlay = (self.alt.armed || alt_progress > 0.001)
            .then(|| self.alt_overlay(alt_progress));
        let queue_progress = self.queue_anim.target(
            if self.queue_open { 1.0 } else { 0.0 },
            anim::EMPHASIZED_MS,
            if self.queue_open { anim::Curve::Decelerate } else { anim::Curve::Accelerate },
            &self.clock,
        );
        if !self.queue_anim.settled(&self.clock) {
            window.request_animation_frame();
        }
        let fullscreen_progress = self.fullscreen_anim.target(
            if self.fullscreen { 1.0 } else { 0.0 },
            anim::EMPHASIZED_MS,
            if self.fullscreen { anim::Curve::Decelerate } else { anim::Curve::Accelerate },
            &self.clock,
        );
        if !self.fullscreen_anim.settled(&self.clock) {
            window.request_animation_frame();
        }
        if self.fullscreen {
            self.request_current_lyrics();
        }
        if let Some(page) = self.page.as_ref() {
            let cover_art = page.header.cover_art.clone();
            self.ensure_display_cover(&cover_art, 512);
        }
        if let Some(track) = self.player.current().cloned() {
            self.ensure_display_cover(&track.cover_art, 512);
        }
        let fullscreen_overlay = (self.fullscreen || fullscreen_progress > 0.001)
            .then(|| self.fullscreen_overlay(fullscreen_progress));
        let exit_overlay = self.exiting.map(|started| {
            let progress = (((self.clock.now - started) / 0.200) as f32).clamp(0.0, 1.0);
            self.exit_overlay(progress)
        });
        if self.row_colors.values().any(|color| !color.settled(&self.clock))
            || self.row_cover_scales.values().any(|scale| !scale.settled(&self.clock))
        {
            window.request_animation_frame();
        }
        self.toasts.retain(|toast| self.clock.now - toast.created_at < 6.320);
        if self.toasts.iter().any(|toast| self.clock.now - toast.created_at < 6.320) {
            window.request_animation_frame();
        }
        let toast_overlay = (!self.toasts.is_empty()).then(|| self.toast_overlay(&*cx));
        let queue_overlay = (self.queue_open || queue_progress > 0.001)
            .then(|| self.queue_overlay(&*cx, queue_progress));
        let overlay = self.search.as_ref().map(|search| self.search_overlay(&*cx, search, progress));
        let description_overlay = self.description_overlay(&*cx);
        if let Some(started) = self.exiting {
            window.request_animation_frame();
            if self.clock.now - started >= 0.200 {
                self.flush_status();
                window.remove_window();
            }
        }
        if self.page.is_some() {
            self.ensure_page_metrics();
            self.ensure_marks();
        }
        if self.queue_open {
            self.ensure_queue_metrics();
        }
        if self.search.as_ref().is_some_and(|search| !search.query.is_empty()) {
            let height = f32::from(window.viewport_size().height);
            self.ensure_search_metrics(height);
        }
        self.ensure_library_window();
        if !self.preloaded && self.view.rows.count > 0 && self.view.scope != Scope::Playlists {
            self.preloaded = true;
            let start_scope = self.view.scope;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(350))
                    .await;
                let _ = this.update(cx, |this, _| {
                    for scope in Scope::ALL {
                        if scope != start_scope && scope != Scope::Playlists {
                            this.bridge.preload_library(scope, 0);
                        }
                    }
                });
            })
            .detach();
        }
        let following = Self::apply_follow(&mut self.library_follow, &self.scroll, self.clock.now)
            | Self::apply_follow(&mut self.page_follow, &self.page_scroll, self.clock.now)
            | Self::apply_follow(&mut self.queue_follow, &self.queue_scroll, self.clock.now);
        if following {
            window.request_animation_frame();
        }
        let top = self.top_bar(&*cx);
        let body = if self.page.is_some() {
            self.page_view(&*cx).into_any_element()
        } else {
            self.list(&*cx).into_any_element()
        };
        let status = self.player_bar();
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_top))
            .on_action(cx.listener(Self::on_bottom))
            .on_action(cx.listener(Self::on_scope_next))
            .on_action(cx.listener(Self::on_scope_prev))
            .on_action(cx.listener(Self::on_order_next))
            .on_action(cx.listener(Self::on_order_prev))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_visual))
            .on_action(cx.listener(Self::on_search))
            .on_action(cx.listener(Self::on_insert_mode))
            .on_action(cx.listener(Self::on_line_search))
            .on_action(cx.listener(Self::on_lyrics_overlay))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_search_scope_next))
            .on_action(cx.listener(Self::on_search_scope_prev))
            .on_action(cx.listener(Self::on_play_pause))
            .on_action(cx.listener(Self::on_queue_toggle))
            .on_action(cx.listener(Self::on_repeat_cycle))
            .on_action(cx.listener(Self::on_toggle_fullscreen))
            .on_action(cx.listener(Self::on_queue_remove))
            .on_action(cx.listener(Self::on_queue_up))
            .on_action(cx.listener(Self::on_queue_down))
            .on_action(cx.listener(Self::on_mute))
            .on_action(cx.listener(Self::on_volume_up))
            .on_action(cx.listener(Self::on_volume_down))
            .on_action(cx.listener(Self::on_seek_back))
            .on_action(cx.listener(Self::on_seek_forward))
            .on_action(cx.listener(Self::on_next_track))
            .on_action(cx.listener(Self::on_prev_track))
            .on_action(cx.listener(Self::on_search_next))
            .on_action(cx.listener(Self::on_search_prev))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_action(cx.listener(Self::on_visual_all))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_back))
            .on_action(cx.listener(Self::on_prev_place))
            .on_action(cx.listener(Self::on_next_place))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .font_family("Overused Grotesk Roman")
            .text_color(rgb(ON_SURFACE))
            .child(top)
            .child(body)
            .child(status)
            .child(match queue_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match fullscreen_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(self.lyrics_overlay_element())
            .child(match description_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match alt_overlay { Some(o) => o, None => div().into_any_element() })
            .child(match line_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match toast_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match exit_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .into_any_element()
    }
}

fn parse_page_spec(spec: &str) -> Option<PageDescriptor> {
    let (kind, id) = spec.split_once(':')?;
    if id.is_empty() {
        return None;
    }
    let kind = match kind {
        "album" => PageKind::Album,
        "artist" => PageKind::Artist,
        "playlist" => PageKind::Playlist,
        _ => return None,
    };
    Some(PageDescriptor { kind, id: id.to_string() })
}

static COVERS: std::sync::OnceLock<std::sync::Mutex<covers::CoverStore>> =
    std::sync::OnceLock::new();

fn cover_path(url: &str) -> Option<std::path::PathBuf> {
    COVERS
        .get_or_init(|| std::sync::Mutex::new(covers::CoverStore::new(768)))
        .lock()
        .ok()?
        .row_path(url)
}

fn highlighted_text(text: &str, query: &str, color: u32) -> gpui::AnyElement {
    let ranges = line_search::match_ranges(query, text);
    if ranges.is_empty() {
        return div().text_color(rgb(color)).child(text.to_string()).into_any_element();
    }
    let highlight = HighlightStyle {
        color: Some(rgb(color).into()),
        background_color: Some(rgb(0x4A4458).into()),
        ..Default::default()
    };
    StyledText::new(text.to_string())
        .with_highlights(ranges.into_iter().map(|range| (range, highlight)))
        .into_any_element()
}

fn art_scaled(path: Option<std::path::PathBuf>, base: f32, scale: f32) -> gpui::AnyElement {
    let size = base * scale;
    let offset = (size - base) * 0.5;
    div()
        .relative()
        .size(px(base))
        .child(
            div()
                .absolute()
                .left(px(-offset))
                .top(px(-offset))
                .child(art(path, size)),
        )
        .into_any_element()
}

fn art(path: Option<std::path::PathBuf>, size: f32) -> gpui::AnyElement {
    match path {
        Some(path) => img(path)
            .size(px(size))
            .rounded_sm()
            .bg(rgb(0x36343B))
            .into_any_element(),
        None => div()
            .size(px(size))
            .rounded_md()
            .bg(rgb(0x36343B))
            .into_any_element(),
    }
}

fn page_key(descriptor: &PageDescriptor) -> String {
    let kind = match descriptor.kind {
        PageKind::Album => "album",
        PageKind::Artist => "artist",
        PageKind::Playlist => "playlist",
    };
    format!("{kind}:{}", descriptor.id)
}

fn trace_scroll() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("MUSIKA_GPUUI_TRACE").is_some())
}

fn delete_word_back(text: &mut String) {
    let mut end = text.len();
    let bytes = text.as_bytes();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    while end > 0 && !bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    text.truncate(end);
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn repeat_label(repeat: crate::player::Repeat) -> &'static str {
    match repeat {
        crate::player::Repeat::Off => "off",
        crate::player::Repeat::One => "one",
        crate::player::Repeat::All => "all",
    }
}

impl SearchState {
    fn kind_index(kind: state::RowKind) -> usize {
        match kind {
            state::RowKind::Song => 0,
            state::RowKind::Album => 1,
            state::RowKind::Artist => 2,
            state::RowKind::Playlist => 3,
        }
    }

    fn scoped(&self) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|row| Self::kind_index(row.kind) == self.scope)
            .collect()
    }

    fn scope_count(&self, scope: usize) -> usize {
        self.rows
            .iter()
            .filter(|row| Self::kind_index(row.kind) == scope)
            .count()
    }

    fn cursor(&self) -> usize {
        self.scope_cursors[self.scope]
    }

    fn move_cursor(&mut self, delta: i32) {
        let len = self.scope_count(self.scope);
        if len == 0 {
            self.scope_cursors[self.scope] = 0;
            return;
        }
        let next = (self.cursor() as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.scope_cursors[self.scope] = next;
    }
}

fn search_scope_label(scope: usize) -> &'static str {
    match scope {
        0 => "Songs",
        1 => "Albums",
        2 => "Artists",
        _ => "Playlists",
    }
}

fn last_sync_text(raw: &str) -> String {
    let Ok(secs) = raw.parse::<i64>() else {
        return "—".to_string();
    };
    if secs <= 0 {
        return "—".to_string();
    }
    let age = crate::consts::now_secs().saturating_sub(secs);
    if age < 60 {
        return "Now".to_string();
    }
    let hours = age as f64 / 3600.0;
    if hours < 24.0 {
        return local_hhmm(secs);
    }
    let days = hours / 24.0;
    if days < 7.0 {
        return local_day(secs);
    }
    let weeks = days / 7.0;
    if weeks < 4.0 {
        return format!("{}w", weeks.floor() as i64);
    }
    let months = days / 30.0;
    if months < 12.0 {
        return format!("{}mo", months.floor() as i64);
    }
    format!("{}y", (days / 365.0).floor() as i64)
}

fn local_tm(secs: i64) -> Option<libc::tm> {
    let time = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::localtime_r(&time, &mut tm) };
    (!ok.is_null()).then_some(tm)
}

fn local_hhmm(secs: i64) -> String {
    local_tm(secs)
        .map(|tm| format!("{:02}:{:02}", tm.tm_hour, tm.tm_min))
        .unwrap_or_else(|| "—".to_string())
}

fn local_day(secs: i64) -> String {
    local_tm(secs)
        .map(|tm| format!("{:02}", tm.tm_mday))
        .unwrap_or_else(|| "—".to_string())
}

fn icon(name: &str, size: f32, color: u32) -> gpui::AnyElement {
    let path = format!("{}/src/icons/{}", env!("CARGO_MANIFEST_DIR"), name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .into_any_element()
}
