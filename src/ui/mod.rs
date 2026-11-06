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
mod page_profile;
mod palette;
mod scroll_modal;
mod state;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Render, Window,
    AssetSource, WindowBounds, WindowOptions, Animation, AnimationExt, actions, div, img, prelude::*, px, relative, rgb, size, svg, uniform_list,
    HighlightStyle, StyledText,
    UniformListScrollHandle,
};

use bridge::Event;
use quick_actions::{AltGesture, Direction, QuickKind, QuickMenu, QuickVerb};
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

#[derive(Clone)]
struct AltTarget {
    kind: RowKind,
    id: String,
    album_id: String,
    song_id: String,
    playlist_id: String,
    ids: Vec<String>,
    selection: bool,
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
    library_scroll: scroll_modal::ScrollModal,
    page_scroll_modal: scroll_modal::ScrollModal,
    queue_scroll_modal: scroll_modal::ScrollModal,
    search_scroll: scroll_modal::ScrollModal,
    page: Option<PageState>,
    travel: Travel,
    page_generation: u64,
    search: Option<SearchState>,
    search_open: bool,
    search_anim: anim::Behavior,
    clock: anim::Clock,
    palette_motion: palette::PaletteMotion,
    palette_cover: Option<String>,
    hovered_row: Option<usize>,
    hovered_page: Option<usize>,
    visual_mode: bool,
    queue_selection: state::Selection,
    last_player_index: usize,
    row_scales: std::collections::HashMap<String, anim::Behavior>,
    row_hover_colors: std::collections::HashMap<String, anim::ColorBehavior>,
    player: crate::player::Player,
    playback_error: Option<String>,
    queue_open: bool,
    queue_cursor: usize,
    queue_anim: anim::Behavior,
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
    lyrics_active_line: Option<usize>,
    lyrics_active_anim: anim::Behavior,
    lyrics_line_opacity: std::collections::HashMap<usize, anim::Behavior>,
    lyrics_window_start: usize,
    lyrics_shift: anim::Behavior,
    lyrics_track_fade: anim::Behavior,
    transport_press: [Option<f64>; 3],
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
            library_scroll: scroll_modal::ScrollModal::new(scroll_modal::LIBRARY),
            page_scroll_modal: scroll_modal::ScrollModal::new(scroll_modal::ALBUM),
            queue_scroll_modal: scroll_modal::ScrollModal::new(scroll_modal::QUEUE),
            search_scroll: scroll_modal::ScrollModal::new(scroll_modal::SEARCH),
            page: None,
            travel: Travel::default(),
            page_generation: 0,
            search: None,
            search_open: false,
            search_anim: anim::Behavior::new(0.0),
            clock: anim::Clock::new(),
            palette_motion: palette::PaletteMotion::new(palette::Palette::default()),
            palette_cover: None,
            hovered_row: None,
            hovered_page: None,
            visual_mode: false,
            queue_selection: state::Selection::default(),
            last_player_index: 0,
            row_scales: std::collections::HashMap::new(),
            row_hover_colors: std::collections::HashMap::new(),
            player: crate::player::Player::new(),
            playback_error: None,
            queue_open: false,
            queue_cursor: 0,
            queue_anim: anim::Behavior::new(0.0),
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
            lyrics_active_line: None,
            lyrics_active_anim: anim::Behavior::new(1.0),
            lyrics_line_opacity: std::collections::HashMap::new(),
            lyrics_window_start: 0,
            lyrics_shift: anim::Behavior::new(0.0),
            lyrics_track_fade: anim::Behavior::new(1.0),
            transport_press: [None; 3],
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
                        self.open_descriptor(PageDescriptor::new(kind, page_id));
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
                let new_song = self.lyrics.as_ref().map(|lyrics| lyrics.song_id.clone()) != Some(song_id.clone());
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
                if new_song {
                    self.lyrics_window_start = 0;
                    self.lyrics_line_opacity.clear();
                    self.lyrics_track_fade.target(0.0, 0.0, anim::Curve::Standard, &self.clock);
                    self.lyrics_track_fade
                        .target(1.0, 300.0, anim::Curve::Decelerate, &self.clock);
                }
            }
            Event::Palette { cover_art, roles } => {
                let current = self
                    .player
                    .current()
                    .map(|track| track.cover_art.as_str())
                    .unwrap_or_default();
                if current == cover_art {
                    let next = palette::Palette::from_roles(&roles);
                    self.clock.tick();
                    self.palette_motion.set(next, &self.clock, true);
                }
            }
            Event::PaletteReset { cover_art } => {
                let current = self
                    .player
                    .current()
                    .map(|track| track.cover_art.as_str())
                    .unwrap_or_default();
                if current == cover_art {
                    self.clock.tick();
                    self.palette_motion.reset(&self.clock);
                }
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

    fn follow_library(&mut self, animated: bool) {
        self.clock.tick();
        self.library_scroll
            .start(self.view.rows.count, self.view.cursor, animated, self.clock.now);
    }

    fn follow_page(&mut self, animated: bool) {
        self.clock.tick();
        let Some(page) = self.page.as_ref() else { return };
        self.page_scroll_modal
            .start(page.rows.len(), page.cursor, animated, self.clock.now);
    }

    fn follow_queue(&mut self, animated: bool) {
        self.clock.tick();
        self.queue_scroll_modal
            .start(self.player.len(), self.queue_cursor, animated, self.clock.now);
    }

    fn land_library(&mut self, index: usize) {
        let cursor = self
            .library_scroll
            .land(self.view.rows.count, index);
        self.view.select(cursor);
        self.view.rows.requested = None;
        self.ensure_library_window();
        self.remember_current();
    }

    fn land_page(&mut self, index: usize) {
        let Some(page) = self.page.as_mut() else { return };
        let cursor = self.page_scroll_modal.land(page.rows.len(), index);
        page.select(cursor);
        self.hovered_page = None;
    }

    fn land_queue(&mut self, index: usize) {
        let cursor = self.queue_scroll_modal.land(self.player.len(), index);
        self.queue_cursor = cursor;
    }

    fn ensure_library_window(&mut self) {
        if self.page.is_some() || self.view.scope == Scope::Playlists {
            return;
        }
        let count = self.view.rows.count;
        if count == 0 || self.view.rows.generation == 0 {
            return;
        }
        let (top, measured) = {
            let state = self.library_scroll.handle.0.borrow();
            let top = state.base_handle.logical_scroll_top().0;
            let height = f32::from(state.base_handle.bounds().size.height);
            (top, (height > 0.0).then_some(height))
        };
        if let Some(viewport) = measured
            && self.library_scroll.ensure_metrics(viewport)
        {
            self.view.rows.requested = None;
            self.follow_library(false);
        }
        let viewport = measured.unwrap_or(600.0);
        let row_h = self.library_scroll.row_h;
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
            let height = f32::from(
                self.queue_scroll_modal
                    .handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds()
                    .size
                    .height,
            );
            (height > 0.0).then_some(height)
        };
        let Some(viewport) = measured else { return };
        if self.queue_scroll_modal.ensure_metrics(viewport) {
            self.follow_queue(false);
        }
    }

    fn ensure_search_metrics(&mut self, window_height: f32) {
        let viewport = (window_height * 0.72).max(1.0);
        let _ = self.search_scroll.ensure_metrics(viewport);
    }

    fn ensure_page_metrics(&mut self) {
        let measured = {
            let height = f32::from(
                self.page_scroll_modal
                    .handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds()
                    .size
                    .height,
            );
            (height > 0.0).then_some(height)
        };
        let Some(_viewport) = measured else { return };
        let reference = (self.win_h - 100.0).max(1.0);
        if self.page_scroll_modal.ensure_metrics_ref(reference) {
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
                let state = self.page_scroll_modal.handle.0.borrow();
                let top = state.base_handle.logical_scroll_top().0;
                let viewport = state
                    .last_item_size
                    .map(|size| f32::from(size.item.height))
                    .unwrap_or(600.0);
                (top, viewport)
            };
            let visible = ((viewport / self.page_scroll_modal.row_h).ceil() as usize).max(1);
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
        if !self.visual_mode {
            self.view.selection.clear();
        }
        self.view.select(index);
        self.hovered_row = None;
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
            if !self.visual_mode {
                page.selection.clear();
            }
            page.select(page.cursor.saturating_add(1));
            self.hovered_page = None;
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
            if !self.visual_mode {
                page.selection.clear();
            }
            page.select(page.cursor.saturating_sub(1));
            self.hovered_page = None;
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_sub(1));
        }
        cx.notify();
    }

    fn on_top(&mut self, _: &Top, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open {
            self.land_queue(0);
        } else if self.page.is_some() {
            self.land_page(0);
        } else {
            self.land_library(0);
        }
        cx.notify();
    }

    fn on_bottom(&mut self, _: &Bottom, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open {
            let last = self.player.len().saturating_sub(1);
            self.land_queue(last);
        } else if let Some(page) = self.page.as_ref() {
            let last = page.rows.len().saturating_sub(1);
            self.land_page(last);
        } else {
            let last = self.view.rows.len().saturating_sub(1);
            self.land_library(last);
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

    fn sync_palette(&mut self) {
        let cover = self
            .player
            .current()
            .map(|track| track.cover_art.clone())
            .unwrap_or_default();
        if self.palette_cover.as_deref() == Some(cover.as_str()) {
            return;
        }
        if cover.is_empty() {
            self.palette_motion.reset(&self.clock);
            self.palette_cover = None;
        } else {
            self.palette_cover = Some(cover.clone());
            self.bridge.request_colors(cover);
        }
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
            .bg(rgb(palette::background()))
            .text_color(rgb(palette::on_surface()))
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
                            .text_color(rgb(palette::primary()))
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
                            .border_color(rgb(if active { palette::primary() } else { palette::outline_variant() }))
                            .bg(rgb(palette::surface_container_highest()))
                            .text_size(px(14.0))
                            .text_color(rgb(if value.is_empty() { palette::on_surface_variant() } else { palette::on_surface() }))
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
                            .bg(rgb(palette::primary_container()))
                            .text_size(px(14.0))
                            .text_color(rgb(palette::on_primary_container()))
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
                            .when(login.error.is_some(), |this| this.border_1().border_color(rgb(palette::error())).bg(rgb(palette::error_container())))
                            .text_size(px(11.0))
                            .text_color(rgb(if login.error.is_some() { palette::on_error_container() } else { palette::on_surface_variant() }))
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
                    PageKind::Radio => "radio",
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
                    .border_color(rgb(palette::outline_variant()))
                    .bg(rgb(palette::surface_container()))
                    .opacity(opacity)
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(rgb(palette::on_surface()))
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
                                .bg(rgb(palette::primary_container()))
                                .text_size(px(11.0))
                                .text_color(rgb(palette::on_primary_container()))
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
        self.press_transport(1);
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
        self.press_transport(2);
        self.player.next();
        if self.queue_open {
            self.queue_cursor = self.player.index();
            self.follow_queue(true);
        }
        cx.notify();
    }

    fn on_prev_track(&mut self, _: &PrevTrack, _window: &mut Window, cx: &mut Context<Self>) {
        self.press_transport(0);
        self.player.previous();
        if self.queue_open {
            self.queue_cursor = self.player.index();
            self.follow_queue(true);
        }
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
            ids: vec![row.id.clone()],
            selection: false,
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

    fn quick_kind(kind: RowKind) -> QuickKind {
        match kind {
            RowKind::Song => QuickKind::Song,
            RowKind::Album => QuickKind::Album,
            RowKind::Artist => QuickKind::Artist,
            RowKind::Playlist => QuickKind::Playlist,
        }
    }

    fn alt_selection_target(
        &self,
        rows: Vec<Row>,
        cursor: usize,
        anchor: (f32, f32),
    ) -> Option<(QuickMenu, AltTarget, (f32, f32))> {
        if rows.is_empty() {
            return None;
        }
        let kind = rows[0].kind;
        let all_starred = rows.iter().all(|row| row.starred);
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let target = AltTarget {
            kind,
            id: rows[cursor.min(rows.len() - 1)].id.clone(),
            album_id: rows[cursor.min(rows.len() - 1)].album_id.clone(),
            song_id: rows[cursor.min(rows.len() - 1)].song_id.clone(),
            playlist_id: String::new(),
            ids,
            selection: true,
        };
        Some((QuickMenu::selection(Self::quick_kind(kind), all_starred), target, anchor))
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
                ids: vec![track.id.clone()],
                selection: false,
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
                ids: vec![track.id.clone()],
                selection: false,
            };
            return Some((
                menu,
                target,
                self.alt_anchor_for(&self.queue_scroll_modal.handle, self.queue_cursor, self.queue_scroll_modal.row_h),
            ));
        }
        if let Some(page) = self.page.as_ref()
            && page.selection.active()
        {
            let anchor_index = page.selection.anchor().unwrap_or(page.cursor);
            let lo = anchor_index.min(page.cursor);
            let hi = anchor_index.max(page.cursor);
            let rows = page.rows.get(lo..=hi).unwrap_or_default().to_vec();
            if let Some(hit) = self.alt_selection_target(
                rows,
                page.cursor.saturating_sub(lo),
                self.alt_anchor_for(&self.page_scroll_modal.handle, page.cursor, self.page_scroll_modal.row_h),
            ) {
                return Some(hit);
            }
        }
        if let Some(page) = self.page.as_ref() {
            let row = page.rows.get(page.cursor)?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                self.alt_anchor_for(&self.page_scroll_modal.handle, page.cursor, self.page_scroll_modal.row_h),
            ));
        }
        if self.view.selection.active() {
            let anchor_index = self.view.selection.anchor().unwrap_or(self.view.cursor);
            let lo = anchor_index.min(self.view.cursor);
            let hi = anchor_index.max(self.view.cursor);
            let rows: Vec<Row> = (lo..=hi)
                .filter_map(|index| self.view.rows.get(index).cloned())
                .collect();
            if let Some(hit) = self.alt_selection_target(
                rows,
                self.view.cursor.saturating_sub(lo),
                self.alt_anchor_for(&self.library_scroll.handle, self.view.cursor, self.library_scroll.row_h),
            ) {
                return Some(hit);
            }
        }
        let row = self.view.rows.get(self.view.cursor)?;
        Some((
            self.alt_menu_for_row(row),
            self.alt_target_for_row(row),
            self.alt_anchor_for(&self.library_scroll.handle, self.view.cursor, self.library_scroll.row_h),
        ))
    }

    fn alt_arm(&mut self) {
        self.clock.tick();
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
        self.clock.tick();
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
        if target.selection {
            match verb {
                QuickVerb::Favorite { on } => {
                    for id in &target.ids {
                        self.bridge.quick_favorite(
                            target.queue_kind().to_string(),
                            id.clone(),
                            on,
                        );
                    }
                }
                QuickVerb::AddToQueue => {
                    for id in &target.ids {
                        self.bridge
                            .quick_queue_add(target.queue_kind().to_string(), id.clone());
                    }
                }
                QuickVerb::Download { on } => {
                    for id in &target.ids {
                        self.bridge
                            .quick_download(target.queue_kind().to_string(), id.clone(), on);
                    }
                }
                _ => {}
            }
            return;
        }
        match verb {
            QuickVerb::AddToQueue => {
                self.bridge.quick_queue_add(target.queue_kind().to_string(), target.id.clone());
            }
            QuickVerb::StartRadio => {
                let seed_id = if !target.song_id.is_empty() {
                    target.song_id.clone()
                } else {
                    target.id.clone()
                };
                self.open_descriptor(PageDescriptor::radio(
                    target.queue_kind().to_string(),
                    seed_id,
                ));
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

    fn press_transport(&mut self, index: usize) {
        self.clock.tick();
        if let Some(slot) = self.transport_press.get_mut(index) {
            *slot = Some(self.clock.now);
        }
    }

    fn transport_scale(&self, index: usize) -> f32 {
        let Some(start) = self.transport_press.get(index).copied().flatten() else {
            return 1.0;
        };
        let elapsed = (self.clock.now - start) as f32;
        if elapsed < 0.0 || elapsed >= 0.32 {
            return 1.0;
        }
        if elapsed < 0.08 {
            1.0 - 0.14 * (elapsed / 0.08)
        } else {
            let progress = (elapsed - 0.08) / 0.24;
            0.86 + 0.14 * anim::Curve::Bounce.sample(progress)
        }
    }

    fn transport_press_live(&self) -> bool {
        self.transport_press.iter().flatten().any(|start| {
            let elapsed = (self.clock.now - start) as f32;
            (0.0..0.32).contains(&elapsed)
        })
    }

    fn refresh_lyrics_emphasis(&mut self) {
        let Some(lyrics) = self.lyrics.as_ref() else {
            self.lyrics_active_line = None;
            return;
        };
        if lyrics.lines.is_empty() {
            return;
        }
        let active = if lyrics.synced {
            let ms = (self.player.position() * 1000.0) as i64;
            lyrics
                .lines
                .iter()
                .rposition(|(start, _)| start.is_some_and(|start| start <= ms))
                .unwrap_or(0)
        } else {
            0
        };
        if self.lyrics_active_line != Some(active) {
            self.lyrics_active_line = Some(active);
            self.lyrics_active_anim.target(0.0, 0.0, anim::Curve::Standard, &self.clock);
            self.lyrics_active_anim
                .target(1.0, anim::QUICK_MS, anim::Curve::Standard, &self.clock);
        }
        let stride = (self.win_h * 0.055).clamp(13.0, 22.0) * 1.55;
        let visible = 15usize;
        let old_start = self.lyrics_window_start;
        let mut start = old_start.min(lyrics.lines.len().saturating_sub(1));
        if active < start + 2 {
            start = active.saturating_sub(2);
        } else if active + visible > start + visible.saturating_sub(2) && active + 2 >= start + visible {
            start = (active + 2).saturating_sub(visible);
        }
        if start != old_start {
            self.lyrics_window_start = start;
            self.lyrics_shift.target(
                (old_start as f32 - start as f32) * stride,
                0.0,
                anim::Curve::Standard,
                &self.clock,
            );
            self.lyrics_shift
                .target(0.0, 300.0, anim::Curve::Decelerate, &self.clock);
        } else {
            self.lyrics_shift.target(0.0, 0.0, anim::Curve::Standard, &self.clock);
        }
        for (index, _) in lyrics.lines.iter().enumerate() {
            let target = if !lyrics.synced {
                0.8
            } else {
                match index.abs_diff(active) {
                    0 => 1.0,
                    1 => 0.8,
                    _ => 0.5,
                }
            };
            self.lyrics_line_opacity
                .entry(index)
                .or_insert_with(|| anim::Behavior::new(target))
                .target(target, anim::QUICK_MS, anim::Curve::Standard, &self.clock);
        }
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
        self.lyrics_overlay = self.lyrics_overlay_wanted && !inline;
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
        if self.queue_open {
            let active = self.queue_selection.active();
            self.visual_mode = !active;
            if active {
                self.queue_selection.clear();
            } else {
                self.queue_selection.start(self.queue_cursor);
            }
            self.hovered_row = None;
            cx.notify();
            return;
        }
        let (active, cursor) = if let Some(page) = self.page.as_ref() {
            (page.selection.active(), page.cursor)
        } else {
            (self.view.selection.active(), self.view.cursor)
        };
        self.visual_mode = !active;
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
        self.visual_mode = true;
        if self.queue_open {
            self.queue_selection.select_all();
            self.queue_cursor = self.player.len().saturating_sub(1);
            self.follow_queue(false);
            cx.notify();
            return;
        }
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
        if self.page.is_some() {
            self.activate_page_cursor();
        } else {
            let row = self.view.rows.get(self.view.cursor).cloned();
            if let Some(row) = row {
                self.activate_row(&row);
            }
        }
        cx.notify();
    }

    fn activate_page_cursor(&mut self) {
        let Some(page) = self.page.as_ref() else { return };
        let Some(row) = page.rows.get(page.cursor) else { return };
        let profile = page_profile::PageProfile::for_descriptor(&page.descriptor);
        let action = profile.row_action(row);
        let index = page.cursor;
        let descriptor = page.descriptor.clone();
        let song_id = row.song_id.clone();
        let album_id = row.album_id.clone();
        match action {
            page_profile::PageAction::OpenAlbum => {
                if !album_id.is_empty() {
                    self.open_descriptor(PageDescriptor::new(PageKind::Album, album_id));
                }
            }
            page_profile::PageAction::PlayAlbum | page_profile::PageAction::PlayArtist => {
                if !song_id.is_empty() {
                    self.last_play = Some((song_id.clone(), self.view.order_label().to_string()));
                    self.bridge.play_song(song_id, self.view.order_label().to_string());
                }
            }
            page_profile::PageAction::PlayPlaylist => {
                self.bridge.quick_play_playlist(descriptor.id);
            }
            page_profile::PageAction::PlayRadio => {
                let seed_kind = descriptor
                    .radio_seed_kind
                    .clone()
                    .unwrap_or_else(|| "album".to_string());
                self.bridge.play_radio(seed_kind, descriptor.id, index);
            }
        }
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
        if self.queue_open && self.queue_selection.active() {
            self.queue_selection.clear();
            self.visual_mode = false;
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
        self.open_descriptor(PageDescriptor::new(kind, row.id.clone()));
    }

    fn open_descriptor(&mut self, descriptor: PageDescriptor) {
        let profile = page_profile::PageProfile::for_descriptor(&descriptor);
        if profile.notes {
            if descriptor.kind == PageKind::Artist {
                self.bridge.request_artist_notes(descriptor.id.clone());
            } else {
                self.bridge.request_notes(descriptor.id.clone());
            }
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
            .bg(rgb(palette::surface_container_low()))
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
                            .text_color(rgb(if offline { palette::tertiary() } else { palette::on_surface_variant() }))
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
                            .text_color(rgb(palette::on_surface_variant()))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(report),
                    )
                    .child(icon("reload-ui-2-svgrepo-com.svg", 18.0, palette::on_surface()))
                    .child(icon("delete-outline.svg", 18.0, palette::on_surface())),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.0))
                    .bg(rgb(palette::outline_variant())),
            )
    }

    fn list(&self, cx: &Context<Self>, window: &mut Window) -> impl IntoElement {
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
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(hint),
            )
            .child(div().flex_1())
            .child(icon("outline-shuffle.svg", 14.0, palette::on_surface_variant()))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(order),
            )
            .when(selection_count > 0, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .bg(rgb(palette::primary()))
                        .text_size(px(11.0))
                        .text_color(rgb(palette::on_primary()))
                        .child(selection_count.to_string()),
                )
            });

        if self.view.rows.is_empty() {
            if self.view.loading {
                window.request_animation_frame();
                return div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .px(px(12.0))
                    .child(header)
                    .child(loading_skeleton(
                        self.clock.now as f32,
                        self.library_scroll.row_h,
                        8,
                    ));
            }
            let message = if let Some(error) = &self.error {
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
                        .text_color(rgb(palette::on_surface_variant()))
                        .child(message),
                );
        }

        let count = self.view.rows.len();
        let scroll = self.library_scroll.handle.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .px(px(12.0))
            .child(header)
            .child(
                uniform_list("library", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let row_h = this.library_scroll.row_h;
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let selected = index == this.view.cursor
                            || this.view.selection.contains(this.view.cursor, index);
                        let hovered = this.hovered_row == Some(index);
                        let emphasis = if selected { 1.08 } else { 1.0 };
                        let striped = index % 2 != 0;
                                        let Some(row) = this.view.rows.get(index) else {
                            // RowWindow blank slot: a waving grey placeholder
                            // until the live window lands.
                            _window.request_animation_frame();
                            items.push(
                                div()
                                    .id(index)
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .h(px(row_h))
                                    .px_1()
                                    .rounded_sm()
                                    .bg(rgb(if selected { palette::secondary_container() } else { palette::surface() }))
                                    .child(row_placeholder(index, row_h, this.clock.now as f32)),
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
                        let cover_scale = this
                            .row_scales
                            .entry(format!("library-scale:{}", row.id))
                            .or_insert_with(|| anim::Behavior::new(1.0))
                            .target(
                                if selected { 1.20 } else { 1.0 },
                                anim::QUICK_MS,
                                anim::Curve::Standard,
                                &this.clock,
                            );
                        let base_fill = if striped { palette::surface_container_lowest() } else { palette::surface() };
                        let hover_fill = this
                            .row_hover_colors
                            .entry(format!("library-hover:{}", row.id))
                            .or_insert_with(|| anim::ColorBehavior::new(base_fill))
                            .target(
                                if hovered && !selected { palette::surface_container_high() } else { base_fill },
                                anim::QUICK_MS,
                                anim::Curve::Standard,
                                &this.clock,
                            );
                        items.push(
                            div()
                                .id(index)
                                .w_full()
                                .flex()
                                .items_center()
                                .h(px(row_h))
                                .px_1()
                                .rounded_sm()
                                .bg(if selected {
                                    rgb(palette::secondary_container())
                                } else {
                                    gpui::rgba(hover_fill)
                                })
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
                                .child(art_scaled(cover, row_h * 0.78, cover_scale))
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
                                                .child(highlighted_text(&title, &this.line_search.query, palette::on_surface())),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.0 * emphasis))
                                                .text_color(rgb(palette::on_surface_variant()))
                                                .whitespace_normal()
                                                .max_h(px(22.0))
                                                .overflow_hidden()
                                                .child(highlighted_text(&subtitle, &this.line_search.query, palette::on_surface_variant())),
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
                                                .text_color(rgb(palette::on_surface_variant()))
                                                .child(highlighted_text(&meta, &this.line_search.query, palette::on_surface_variant())),
                                        )
                                        .when(downloaded, |this| {
                                            this.child(icon("outline-download.svg", 13.0, palette::primary()))
                                        })
                                        .when(starred, |this| {
                                            this.child(icon("baseline-favorite.svg", 12.0, 0xFFD54F))
                                        })
                                        .when(rating > 0, |this| {
                                            this.child(
                                                div()
                                                    .text_size(px(10.0))
                                                    .text_color(rgb(palette::primary()))
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
                    this.library_scroll.follow.active = false;
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
                                    .text_color(rgb(palette::on_surface_variant()))
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
                                .text_color(rgb(palette::on_surface()))
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
        let page_h = (self.win_h - 120.0).max(1.0);
        let option_scale = (0.816
            * ((self.win_w - 20.0) / 1280.0).min((page_h - 20.0) / 720.0))
            .clamp(0.5, 0.816);
        let box_side = page_h * 0.07 * option_scale;
        let spawned = 0.72 + 0.28 * progress;
        let box_side = box_side * spawned;
        let frame_side = (box_side * 8.0).min(self.win_w - 20.0).min(page_h - 20.0);
        let board_x = (ax - frame_side * 0.5).clamp(10.0, (self.win_w - frame_side - 10.0).max(10.0));
        let board_y = (ay - frame_side * 0.5).clamp(10.0, (page_h - frame_side + 10.0).max(10.0));
        let mut board = div()
            .absolute()
            .left(px(board_x))
            .top(px(board_y))
            .w(px(frame_side))
            .h(px(frame_side))
            .opacity(progress);
        let center = (frame_side - box_side) * 0.5;
        for direction in Direction::ALL {
            let Some(slot) = menu.slot(direction) else { continue };
            let aimed = self.alt.aimed == Some(direction);
            let size = if aimed { box_side * 1.30 } else { box_side };
            let (cx, cy) = match direction {
                Direction::North => (center + box_side * 0.5, box_side * 0.5),
                Direction::South => (center + box_side * 0.5, frame_side - box_side * 0.5),
                Direction::West => (box_side * 0.5, center + box_side * 0.5),
                Direction::East => (frame_side - box_side * 0.5, center + box_side * 0.5),
            };
            let index = self.alt.action_index[direction.index()];
            let label = slot.active_label(index).to_string();
            let icon_name = slot.active_icon(index).to_string();
            let disabled = slot.disabled;
            let bg = if disabled {
                palette::surface_container_low()
            } else if aimed {
                palette::primary_container()
            } else {
                palette::surface_container_high()
            };
            let fg = if disabled {
                palette::outline_variant()
            } else if aimed {
                palette::on_primary_container()
            } else {
                palette::on_surface_variant()
            };
            let opacity = if disabled {
                0.7
            } else if self.alt.aimed.is_some() && !aimed {
                0.55
            } else {
                1.0
            };
            let left = cx - size * 0.5;
            let top = cy - size * 0.5;
            let mut tile = div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(size))
                .h(px(size))
                .rounded_md()
                .border_1()
                .border_color(rgb(if aimed { palette::primary() } else { palette::outline_variant() }))
                .bg(rgb(bg))
                .opacity(opacity)
                .flex()
                .items_center()
                .justify_center();
            if icon_name.is_empty() {
                tile = tile.child(
                    div()
                        .text_size(px((page_h * 0.018 * option_scale).max(11.0)))
                        .text_color(rgb(fg))
                        .child(label),
                );
            } else {
                tile = tile.child(icon(&icon_name, size * 0.55, fg));
            }
            if direction == Direction::South && slot.is_rating_slot() {
                tile = tile.child(
                    div()
                        .absolute()
                        .right(px(size * 0.07))
                        .bottom(px(size * 0.05))
                        .text_size(px((size * 0.28).max(9.0)))
                        .text_color(rgb(palette::primary()))
                        .child(self.alt.rating_value.to_string()),
                );
            }
            board = board.child(tile);
        }
        board = board.child(
            div()
                .absolute()
                .left(px(center + box_side * 0.5 - page_h * 0.013))
                .top(px(center + box_side * 0.5 - page_h * 0.013))
                .child(icon(
                    "arrow-move-20-regular.svg",
                    page_h * 0.026,
                    palette::primary(),
                ))
                .opacity(if self.alt.aimed.is_none() { 1.0 } else { 0.6 }),
        );
        board.into_any_element()
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
                    .bg(rgb(palette::surface_container()))
                    .border_t_1()
                    .border_color(rgb(palette::primary()))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette::primary()))
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
                            .text_color(rgb(palette::on_surface()))
                            .child(self.line_search.query.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette::on_surface_variant()))
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
        let start = self.lyrics_window_start.min(lyrics.lines.len().saturating_sub(1));
        let end = (start + 15).min(lyrics.lines.len());
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
                    .gap(px(12.0))
                    .opacity(self.lyrics_track_fade.value(&self.clock))
                    .mt(px(self.lyrics_shift.value(&self.clock)))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child(format!("Lyrics · {} · ? closes", lyrics.source)),
                    )
                    .children(lyrics.lines[start..end].iter().enumerate().map(|(offset, (_, text))| {
                        let index = start + offset;
                        let is_active = index == active;
                        let base = (self.win_h * 0.055).clamp(13.0, 22.0);
                        let active_size = (base
                            * (1.0 + 0.12 * self.lyrics_active_anim.value(&self.clock)))
                            .min(24.5);
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(if is_active { active_size } else { base }))
                            .text_color(rgb(if is_active { palette::on_surface() } else { palette::on_surface_variant() }))
                            .text_center()
                            .whitespace_normal()
                            .opacity(
                                self.lyrics_line_opacity
                                    .get(&index)
                                    .map(|anim| anim.value(&self.clock))
                                    .unwrap_or(1.0),
                            )
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
        let start = self.lyrics_window_start.min(lines.len().saturating_sub(1));
        let end = (start + 15).min(lines.len());
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
        let lyrics_inline = self.lyrics_inline();
        let lyric_base = (self.win_h * 0.055).clamp(13.0, 22.0);
        let lyric_active = (lyric_base * (1.0 + 0.12 * self.lyrics_active_anim.value(&self.clock))).min(24.5);
        let cover_side = if lyrics_inline {
            300.0
        } else {
            (self.win_w * 0.95).min(self.win_h * 0.62)
        };
        div()
            .absolute()
            .inset_0()
            .size_full()
            .opacity(progress)
            .bg(rgb(palette::background()))
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
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .child(art(cover, cover_side)),
                    )
                    .child(
                        div()
                            .flex()
                            .when(lyrics_inline, |this| this.flex_1())
                            .when(!lyrics_inline, |this| this.hidden())
                            .h_full()
                            .opacity(if lyrics_inline && !self.lyrics_overlay {
                                self.lyrics_track_fade.value(&self.clock)
                            } else {
                                0.0
                            })
                            .min_w_0()
                            .overflow_hidden()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px((lyric_base * 0.5).max(8.0)))
                            .mt(px(self.lyrics_shift.value(&self.clock)))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(lyrics_label),
                            )
                            .children(visible.into_iter().map(|(index, text)| {
                                let is_active = index == active;
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_size(px(if is_active { lyric_active } else { lyric_base }))
                                    .text_color(rgb(if is_active { palette::on_surface() } else { palette::on_surface_variant() }))
                                    .text_center()
                                    .whitespace_normal()
                                    .opacity(
                                        self.lyrics_line_opacity
                                            .get(&index)
                                            .map(|anim| anim.value(&self.clock))
                                            .unwrap_or(1.0),
                                    )
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
                            .text_color(rgb(palette::on_surface_variant()))
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
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(format_time(position)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(rgb(palette::surface_container_high()))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(progress_ratio))
                                            .rounded_full()
                                            .bg(rgb(palette::primary())),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(format_time(duration)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_5()
                            .child(transport_button("skip-previous-solid-svgrepo-com.svg", 22.0, palette::on_surface(), self.transport_scale(0)))
                            .child(transport_button(
                                if self.player.state() == crate::player::State::Playing {
                                    "pause-svgrepo-com.svg"
                                } else {
                                    "play-1001-svgrepo-com.svg"
                                },
                                28.0,
                                palette::on_surface(),
                                self.transport_scale(1),
                            ))
                            .child(transport_button("skip-next-circle-solid-svgrepo-com.svg", 22.0, palette::on_surface(), self.transport_scale(2))),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .text_center()
                            .child("Shift-Enter or q closes · Space play/pause · Ctrl-Q queue"),
                    ),
            )
    }

    fn queue_overlay(&self, cx: &Context<Self>, progress: f32) -> gpui::Div {
        let count = self.player.len();
        let current = self.player.index();
        let cursor = self.queue_cursor;
        let scroll = self.queue_scroll_modal.handle.clone();
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
                    .bg(rgb(palette::surface_container()))
                    .border_l_1()
                    .border_color(rgb(palette::outline_variant()))
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
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(format!("{} / {}", current.saturating_add(1), count)),
                            ),
                    )
                    .child(div().h(px(1.0)).bg(rgb(palette::outline_variant())))
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
                                let selected = active
                                    || this.queue_selection.contains(cursor, index)
                                    || this.queue_selection.contains(this.queue_cursor, index);
                                let playing = index == this.player.index();
                                let emphasis = if selected { 1.08 } else { 1.0 };
                                let cover_scale = this
                                    .row_scales
                                    .entry(format!("queue-scale:{index}"))
                                    .or_insert_with(|| anim::Behavior::new(1.0))
                                    .target(
                                        if selected { 1.20 } else { 1.0 },
                                        anim::QUICK_MS,
                                        anim::Curve::Standard,
                                        &this.clock,
                                    );
                                items.push(
                                    div()
                                        .id(index)
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .h(px(this.queue_scroll_modal.row_h))
                                        .px_3()
                                        .cursor_pointer()
                                        .when(selected, |this| this.bg(rgb(palette::secondary_container())))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if !this.visual_mode {
                                                this.queue_selection.clear();
                                            }
                                            this.queue_cursor = index;
                                            this.player.jump(index);
                                            cx.notify();
                                        }))
                                        .child(art_scaled(cover, this.queue_scroll_modal.row_h * 0.714, cover_scale))
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
                                                                .when(playing, |this| this.text_color(rgb(palette::primary())))
                                                                .whitespace_normal()
                                                                .max_h(px(20.0))
                                                                .overflow_hidden()
                                                                .child(highlighted_text(&title, &this.line_search.query, palette::on_surface())),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(10.0 * emphasis))
                                                        .text_color(rgb(palette::on_surface_variant()))
                                                        .whitespace_normal()
                                                        .max_h(px(16.0))
                                                        .overflow_hidden()
                                                        .child(highlighted_text(&artist, &this.line_search.query, palette::on_surface_variant())),
                                                ),
                                        ),
                                );
                            }
                            items
                        }))
                        .h_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.queue_scroll_modal.follow.active = false;
                        }))
                        .track_scroll(scroll),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
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
                    .h(px(self.search_scroll.row_h))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.hovered_row = Some(index);
                        } else if this.hovered_row == Some(index) {
                            this.hovered_row = None;
                        }
                        cx.notify();
                    }))
                    .when(selected, |this| this.bg(rgb(palette::secondary_container())))
                    .when(hovered && !selected, |this| this.bg(rgb(palette::surface_container_high())))
                    .child(art_scaled(
                        cover,
                        self.search_scroll.row_h * 0.714,
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
                                    .child(highlighted_text(&row.title, &self.line_search.query, palette::on_surface())),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0 * emphasis))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .whitespace_normal()
                                    .max_h(px(18.0))
                                    .overflow_hidden()
                                    .child(highlighted_text(&row.subtitle, &self.line_search.query, palette::on_surface_variant())),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette::on_surface_variant()))
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
                    .border_color(rgb(palette::primary()))
                    .bg(rgb(palette::surface_container()))
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
                                    .bg(rgb(palette::surface_container_highest()))
                                    .px_4()
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_normal()
                                    .max_h(px(44.0))
                                    .text_size(px(16.0))
                                    .text_color(if search.query.is_empty() { rgb(palette::on_surface_variant()) } else { rgb(palette::on_surface()) })
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
                                    .text_color(rgb(if active { palette::primary() } else { palette::on_surface_variant() }))
                                    .child(format!("{} {count}", search_scope_label(scope)))
                            })),
                    )
                    .child(div().h(px(1.0)).bg(rgb(palette::outline_variant())))
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
                            .text_color(rgb(palette::on_surface_variant()))
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
        let profile = page_profile::PageProfile::for_descriptor(&page.descriptor);
        let show_row_cover = profile.show_row_cover;
        let title = page.header.title.clone();
        let subtitle = page.header.subtitle.clone();
        let meta = page.header.meta.clone();
        let page_h = (self.win_h - 120.0).max(1.0);
        let page_w = self.win_w.max(1.0);
        let max_cover = (page_h - page_w * 0.04) * 0.75;
        let cover_side = page_w.min(page_h).min(max_cover).max(1.0);
        let cover_art_side = (cover_side - page_w * 0.04).max(1.0);
        let cover_bottom = cover_art_side + page_w * 0.02;
        let cover = self
            .display_cover(&page.header.cover_art, cover_bucket(cover_side))
            .or_else(|| cover_path(&page.header.cover_url));
        let notes = self
            .notes
            .as_ref()
            .filter(|(album_id, _)| *album_id == page.descriptor.id)
            .map(|(_, notes)| notes.clone())
            .unwrap_or_default();
        let desc_title = title.clone();
        let desc_subtitle = subtitle.clone();
        let header_h = cover_bottom.max(220.0);
        let spacer_h = 0.0;

        let header = div()
            .relative()
            .flex_none()
            .w_full()
            .h(px(header_h))
            .px(px(12.0))
            .child(
                div()
                    .absolute()
                    .left(px(page_w * 0.02))
                    .top(px(page_w * 0.02))
                    .w(relative(0.78))
                    .max_w(px(page_w * 0.96))
                    .p(px(page_w * 0.02))
                    .rounded_md()
                    .bg(gpui::black().opacity(0.40))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(page_h * 0.04 * (page_w / 1280.0).clamp(0.9, 1.6)))
                            .font_weight(gpui::FontWeight::BOLD)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(page_h * 0.024 * (page_w / 1280.0).clamp(0.9, 1.6)))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child("☆ ☆ ☆ ☆ ☆"),
                    )
                    .child(
                        div()
                            .text_size(px(page_h * 0.024 * (page_w / 1280.0).clamp(0.9, 1.6)))
                            .text_color(rgb(palette::primary()))
                            .child(subtitle),
                    )
                    .child(
                        div()
                            .text_size(px(page_h * 0.02 * (page_w / 1280.0).clamp(0.9, 1.6)))
                            .text_color(rgb(palette::on_surface_variant()))
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
                                .text_color(rgb(palette::on_surface_variant()))
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
                        .text_color(rgb(palette::on_surface_variant()))
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
                        .text_color(rgb(palette::on_surface_variant()))
                        .child(error.clone()),
                );
        }

        let count = page.rows.len();
        if count == 0 {
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
                        .text_color(rgb(palette::on_surface_variant()))
                        .child(profile.empty_text),
                );
        }
        let scroll = self.page_scroll_modal.handle.clone();
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .child(
                div()
                    .absolute()
                    .top(px(page_w * 0.02))
                    .left(px(page_w * 0.02))
                    .right(px(page_w * 0.02))
                    .h(px(cover_art_side))
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(art(cover, cover_art_side)),
            )
            .child(header)
            .child(div().h(px(spacer_h)))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .px(px(page_w * 0.02))
                    .pt(px(page_w * 0.02))
                    .child(
                        uniform_list("page-rows", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            let row_h = this.page_scroll_modal.row_h;
                            let mut items = Vec::with_capacity(range.len());
                            for index in range {
                                let Some((title, subtitle, meta, selected, rating, starred, downloaded, cover, show_cover)) = this
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
                                            cover_path(&row.cover_url),
                                            show_row_cover,
                                        )
                                    })
                                else {
                                    break;
                                };
                                let hovered = this.hovered_page == Some(index);
                                let emphasis = this
                                    .row_scales
                                    .entry(format!("page-text-scale:{index}"))
                                    .or_insert_with(|| anim::Behavior::new(1.0))
                                    .target(
                                        if selected { 1.08 } else { 1.0 },
                                        anim::QUICK_MS,
                                        anim::Curve::Standard,
                                        &this.clock,
                                    );
                                let page_base = if index % 2 != 0 { palette::surface_container_lowest() } else { palette::surface() };
                                let hover_fill = this
                                    .row_hover_colors
                                    .entry(format!("page-hover:{index}"))
                                    .or_insert_with(|| anim::ColorBehavior::new(page_base))
                                    .target(
                                        if hovered && !selected { palette::surface_container_high() } else { page_base },
                                        anim::QUICK_MS,
                                        anim::Curve::Standard,
                                        &this.clock,
                                    );
                                let cover_scale = show_cover.then(|| {
                                    this.row_scales
                                        .entry(format!("page-cover-scale:{index}"))
                                        .or_insert_with(|| anim::Behavior::new(1.0))
                                        .target(
                                            if selected { 1.20 } else { 1.0 },
                                            anim::QUICK_MS,
                                            anim::Curve::Standard,
                                            &this.clock,
                                        )
                                });
                                items.push(
                                    div()
                                        .id(index)
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .h(px((row_h - 2.0).max(18.0)))
                                        .mt(px(1.0))
                                        .px_3()
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .bg(if selected {
                                            rgb(palette::secondary_container())
                                        } else {
                                            gpui::rgba(hover_fill)
                                        })
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
                                            this.activate_page_cursor();
                                            cx.notify();
                                        }))
                                        .child(if let Some(scale) = cover_scale {
                                            div()
                                                .mr_2()
                                                .child(art_scaled(cover, row_h * 0.78, scale))
                                                .into_any_element()
                                        } else {
                                            div().into_any_element()
                                        })
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
                                                        .child(highlighted_text(&title, &this.line_search.query, palette::on_surface())),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0 * emphasis))
                                                        .text_color(rgb(palette::on_surface_variant()))
                                                        .whitespace_nowrap()
                                                        .text_ellipsis()
                                                        .child(subtitle),
                                                ),
                                        )
                                        .child(
                                            div()
                                                    .text_size(px(11.0 * emphasis))
                                                .text_color(rgb(palette::on_surface_variant()))
                                                .child(highlighted_text(&meta, &this.line_search.query, palette::on_surface_variant())),
                                        )
                                        .when(downloaded, |this| {
                                            this.child(icon("outline-download.svg", 12.0, palette::primary()))
                                        })
                                        .when(starred, |this| {
                                            this.child(icon("baseline-favorite.svg", 11.0, 0xFFD54F))
                                        })
                                        .when(rating > 0, |this| {
                                            this.child(div().text_size(px(10.0)).text_color(rgb(palette::primary())).child(rating.to_string()))
                                        }),
                                );
                            }
                            items
                        }))
                        .h_full()
                        .w_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.page_scroll_modal.follow.active = false;
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
            .bg(rgb(palette::surface_container()))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(3.0))
                    .bg(rgb(palette::surface_container_high()))
                    .child(div().h_full().w(relative(progress)).bg(rgb(palette::primary()))),
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
                                            .text_color(rgb(palette::on_surface_variant()))
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
                            .child(transport_button("skip-previous-solid-svgrepo-com.svg", 20.0, palette::on_surface(), self.transport_scale(0)))
                            .child(transport_button(
                                if state == crate::player::State::Playing {
                                    "pause-svgrepo-com.svg"
                                } else {
                                    "play-1001-svgrepo-com.svg"
                                },
                                24.0,
                                palette::on_surface(),
                                self.transport_scale(1),
                            ))
                            .child(transport_button("skip-next-circle-solid-svgrepo-com.svg", 20.0, palette::on_surface(), self.transport_scale(2))),
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
                                    .text_color(rgb(palette::on_surface_variant()))
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
                                        .text_color(rgb(palette::primary()))
                                        .child(repeat_label(self.repeat)),
                                )
                            })
                            .child(icon(
                                if muted { "vol-mute-svgrepo-com.svg" }
                                else if volume <= 1.0 { "vol-zero-svgrepo-com.svg" }
                                else if volume <= 33.0 { "vol-low-svgrepo-com.svg" }
                                else { "vol-high-svgrepo-com.svg" },
                                16.0,
                                palette::on_surface_variant(),
                            ))
                            .child(
                                div()
                                    .w(px(54.0))
                                    .h(px(3.0))
                                    .rounded_full()
                                    .bg(rgb(palette::surface_container_high()))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative((volume / 100.0) as f32))
                                            .rounded_full()
                                            .bg(rgb(palette::primary())),
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
            .bg(rgb(palette::surface_container()))
            .text_size(px(11.0))
            .text_color(rgb(palette::on_surface_variant()))
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
        palette::publish(self.palette_motion.value(&self.clock));
        if !self.palette_motion.settled(&self.clock) {
            window.request_animation_frame();
        }
        self.sync_palette();
        let playback_index = self.player.index();
        if playback_index != self.last_player_index {
            self.last_player_index = playback_index;
            if self.queue_open {
                self.queue_cursor = playback_index.min(self.player.len().saturating_sub(1));
                self.follow_queue(true);
            }
        }
        self.refresh_lyrics_emphasis();
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
            let page_h = (self.win_h - 120.0).max(1.0);
            let page_w = self.win_w.max(1.0);
            let max_cover = (page_h - page_w * 0.04) * 0.75;
            let cover_side = page_w.min(page_h).min(max_cover).max(1.0);
            self.ensure_display_cover(&cover_art, cover_bucket(cover_side));
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
        let following = self.library_scroll.apply(self.clock.now)
            | self.page_scroll_modal.apply(self.clock.now)
            | self.queue_scroll_modal.apply(self.clock.now);
        if following {
            window.request_animation_frame();
        }
        let top = self.top_bar(&*cx);
        let body = if self.page.is_some() {
            self.page_view(&*cx).into_any_element()
        } else {
            self.list(&*cx, window).into_any_element()
        };
        if self.row_scales.values().any(|scale| !scale.settled(&self.clock))
            || self
                .row_hover_colors
                .values()
                .any(|color| !color.settled(&self.clock))
        {
            window.request_animation_frame();
        }
        if !self.lyrics_active_anim.settled(&self.clock)
            || !self.lyrics_shift.settled(&self.clock)
            || !self.lyrics_track_fade.settled(&self.clock)
            || self
                .lyrics_line_opacity
                .values()
                .any(|value| !value.settled(&self.clock))
        {
            window.request_animation_frame();
        }
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
            .bg(rgb(palette::background()))
            .font_family("Overused Grotesk Roman")
            .text_color(rgb(palette::on_surface()))
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
        "radio" => {
            let (seed_kind, seed_id) = id.split_once(':')?;
            if seed_id.is_empty() {
                return None;
            }
            return Some(PageDescriptor::radio(seed_kind, seed_id));
        }
        _ => return None,
    };
    Some(PageDescriptor::new(kind, id))
}

static COVERS: std::sync::OnceLock<std::sync::Mutex<covers::CoverStore>> =
    std::sync::OnceLock::new();

fn cover_bucket(logical_size: f32) -> u32 {
    const BUCKETS: [u32; 11] = [384, 512, 640, 768, 960, 1280, 1536, 1920, 2560, 3072, 4096];
    let px = (logical_size * 1.5).ceil() as u32;
    BUCKETS
        .iter()
        .copied()
        .find(|bucket| px <= *bucket)
        .unwrap_or(4096)
}

fn cover_path(url: &str) -> Option<std::path::PathBuf> {
    COVERS
        .get_or_init(|| std::sync::Mutex::new(covers::CoverStore::new(768)))
        .lock()
        .ok()?
        .row_path(url)
}

fn transport_button(icon_name: &str, base: f32, color: u32, scale: f32) -> gpui::AnyElement {
    div()
        .size(px(base * 1.3))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(icon_name, base * scale, color))
        .into_any_element()
}

fn highlighted_text(text: &str, query: &str, color: u32) -> gpui::AnyElement {
    let ranges = line_search::match_ranges(query, text);
    if ranges.is_empty() {
        return div().text_color(rgb(color)).child(text.to_string()).into_any_element();
    }
    let highlight = HighlightStyle {
        color: Some(rgb(color).into()),
        background_color: Some(rgb(palette::secondary_container()).into()),
        ..Default::default()
    };
    StyledText::new(text.to_string())
        .with_highlights(ranges.into_iter().map(|range| (range, highlight)))
        .into_any_element()
}

fn row_wave_color(now: f32, index: usize, base: u32, peak: u32) -> u32 {
    let phase = (now * 2.6 + index as f32 * 0.22).sin() * 0.5 + 0.5;
    let mix = |_channel: u32, shift: u32| -> u32 {
        let a = (base >> shift) & 0xFF;
        let b = (peak >> shift) & 0xFF;
        (a as f32 + (b as f32 - a as f32) * phase) as u32
    };
    (mix(0, 16) << 16) | (mix(0, 8) << 8) | mix(0, 0)
}

fn row_placeholder(index: usize, row_h: f32, now: f32) -> gpui::AnyElement {
    let cover_color = row_wave_color(now, index, 0x26232B, 0x4A4554);
    let bar_color = row_wave_color(now, index + 3, 0x2B2833, 0x514B5C);
    div()
        .w_full()
        .h(px(row_h))
        .flex()
        .items_center()
        .px_1()
        .child(
            div()
                .size(px(row_h * 0.78))
                .rounded_sm()
                .bg(rgb(cover_color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .ml_3()
                .gap_2()
                .child(
                    div()
                        .w(relative(0.44))
                        .h(px((row_h * 0.18).max(6.0)))
                        .rounded_sm()
                        .bg(rgb(bar_color)),
                )
                .child(
                    div()
                        .w(relative(0.26))
                        .h(px((row_h * 0.12).max(4.0)))
                        .rounded_sm()
                        .bg(rgb(row_wave_color(now, index + 7, 0x24212A, 0x453F50))),
                ),
        )
        .into_any_element()
}

fn loading_skeleton(now: f32, row_h: f32, count: usize) -> gpui::AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .children((0..count).map(|index| row_placeholder(index, row_h, now)))
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
        Some(path) => {
            let id = gpui::SharedString::from(format!("cover-fade:{}", path.display()));
            img(path.clone())
                .size(px(size))
                .rounded_sm()
                .bg(rgb(palette::surface_container_highest()))
                .with_animation(
                    id,
                    Animation::new(std::time::Duration::from_millis(120)),
                    |this, delta| this.opacity(delta),
                )
                .into_any_element()
        }
        None => div()
            .size(px(size))
            .rounded_md()
            .bg(rgb(palette::surface_container_highest()))
            .into_any_element(),
    }
}

fn page_key(descriptor: &PageDescriptor) -> String {
    let kind = match descriptor.kind {
        PageKind::Album => "album",
        PageKind::Artist => "artist",
        PageKind::Playlist => "playlist",
        PageKind::Radio => "radio",
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
