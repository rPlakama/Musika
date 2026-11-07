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
//! Target is [GPUI](https://gpui.rs). The 64-file legacy UI abstraction spec lives in
//! `docs/mds/gpuui/`; `09-gpui-retarget.md` maps it onto GPUI. This slice wires
//! the shared bridge into an entity root with scope tabs and a virtual list.
#![allow(dead_code)] // shared state carries page/travel fields ahead of G1 pages.

mod anim;
mod bridge;
mod covers;
mod command_line;
mod ipc_command;
mod line_search;
mod lyrics_layout;
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

use bridge::{Event, SyncMode};
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
            KeyBinding::new("q", Back, Some("main")),
            KeyBinding::new("ctrl-q", QueueToggle, Some("main")),
            KeyBinding::new("d d", QueueRemove, Some("main")),
            KeyBinding::new("ctrl-shift-k", QueueUp, Some("main")),
            KeyBinding::new("ctrl-shift-j", QueueDown, Some("main")),
            KeyBinding::new("ctrl-r", RepeatCycle, Some("main")),
            KeyBinding::new("tab", SearchScopeNext, Some("main")),
            KeyBinding::new("shift-tab", SearchScopePrev, Some("main")),
            KeyBinding::new("shift-enter", ToggleFullscreen, Some("main")),
            KeyBinding::new("j", MoveDown, Some("main")),
            KeyBinding::new("k", MoveUp, Some("main")),
            KeyBinding::new("g g", Top, Some("main")),
            KeyBinding::new("shift-g", Bottom, Some("main")),
            KeyBinding::new("shift-j", ScopeNext, Some("main")),
            KeyBinding::new("shift-k", ScopePrev, Some("main")),
            KeyBinding::new("shift-l", OrderNext, Some("main")),
            KeyBinding::new("shift-h", OrderPrev, Some("main")),
            KeyBinding::new("r", Refresh, Some("main")),
            KeyBinding::new("enter", Enter, Some("main")),
            KeyBinding::new("v", Visual, Some("main")),
            KeyBinding::new("shift-q", Home, Some("main")),
            KeyBinding::new("i", InsertMode, Some("main")),
            KeyBinding::new("ctrl-h", SearchScopePrev, Some("main")),
            KeyBinding::new("ctrl-l", SearchScopeNext, Some("main")),
            KeyBinding::new("shift-space f", Search, Some("main")),
            KeyBinding::new("shift-space shift-f", Search, Some("main")),
            KeyBinding::new("up", MoveUp, Some("main")),
            KeyBinding::new("down", MoveDown, Some("main")),
            KeyBinding::new("space", PlayPause, Some("main")),
            KeyBinding::new("m", Mute, Some("main")),
            KeyBinding::new("-", VolumeDown, Some("main")),
            KeyBinding::new("=", VolumeUp, Some("main")),
            KeyBinding::new("+", VolumeUp, Some("main")),
            KeyBinding::new(",", SeekBack, Some("main")),
            KeyBinding::new(".", SeekForward, Some("main")),
            KeyBinding::new("ctrl-n", NextTrack, Some("main")),
            KeyBinding::new("ctrl-p", PrevTrack, Some("main")),
            KeyBinding::new("shift-v", VisualAll, Some("main")),
            KeyBinding::new("[ b", PrevPlace, Some("main")),
            KeyBinding::new("] b", NextPlace, Some("main")),
            KeyBinding::new("ctrl-d", PageDown, Some("main")),
            KeyBinding::new("ctrl-u", PageUp, Some("main")),
            KeyBinding::new("pagedown", PageDown, Some("main")),
            KeyBinding::new("pageup", PageUp, Some("main")),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineSurface {
    Search,
    Queue,
    Page,
    Library,
    Lyrics,
    Status,
}

struct SearchState {
    editor: line_search::LineEditor,
    rows: Vec<Row>,
    most_played: bool,
    scope: usize,
    scope_cursors: [usize; 4],
    selection: state::Selection,
    generation: u64,
    loading: bool,
    typing: bool,
}

struct PendingReveal {
    page_kind: state::PageKind,
    page_id: String,
    row_kind: state::RowKind,
    row_id: String,
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

const PAGE_HERO_PX: f32 = 720.0;
const FULLSCREEN_HERO_PX: f32 = 720.0;
struct RowPaint {
    flags: page_profile::RowFlags,
    surface: LineSurface,
    index: usize,
    selected: bool,
    height: f32,
    cover_scale: f32,
}

struct SearchRowPaint {
    scope: usize,
    index: usize,
    row_h: f32,
    selected: bool,
    hovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FullscreenFocus {
    Cover,
    Title,
    Lyrics,
}

impl FullscreenFocus {
    fn next(self) -> Self {
        match self {
            Self::Cover => Self::Title,
            Self::Title => Self::Lyrics,
            Self::Lyrics => Self::Cover,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Cover => Self::Lyrics,
            Self::Title => Self::Cover,
            Self::Lyrics => Self::Title,
        }
    }
}

#[derive(Debug, Default)]
struct FullscreenAnchors {
    cover: Option<(f32, f32)>,
    title: Option<(f32, f32)>,
}

type StatusAnchorSlots = std::rc::Rc<std::cell::RefCell<[Option<(f32, f32)>; 6]>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopBarAction {
    Sync,
    Wipe,
    Download,
    Favorite,
    Tool,
}

impl TopBarAction {
    const fn slot(self) -> usize {
        match self {
            Self::Sync => 0,
            Self::Wipe => 1,
            Self::Download => 2,
            Self::Favorite => 4,
            Self::Tool => 5,
        }
    }

    const fn label(self, mode: SuperiorMode) -> &'static str {
        match self {
            Self::Sync => "Sync",
            Self::Wipe => "Wipe",
            Self::Download => match mode {
                SuperiorMode::Album => "Download album",
                SuperiorMode::Artist => "Download artist",
                _ => "Download",
            },
            Self::Favorite => match mode {
                SuperiorMode::Album => "Favorite album",
                SuperiorMode::Artist => "Favorite artist",
                _ => "Favorite",
            },
            Self::Tool => "Tool",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuperiorMode {
    Home,
    Album,
    Artist,
    Playlist,
    Radio,
}

impl SuperiorMode {
    fn from_page(page: Option<&PageState>) -> Self {
        match page.map(|page| page.descriptor.kind) {
            None => Self::Home,
            Some(PageKind::Album) => Self::Album,
            Some(PageKind::Artist) => Self::Artist,
            Some(PageKind::Playlist) => Self::Playlist,
            Some(PageKind::Radio) => Self::Radio,
        }
    }

    fn is_page(self) -> bool {
        !matches!(self, Self::Home)
    }
}

fn page_tool_visible(page: &PageBarState) -> bool {
    !page.starred || !page.cached
}

fn page_actions(page: &PageBarState, artist_tool: bool) -> Vec<TopBarAction> {
    let mut actions = Vec::new();
    if page.starred {
        actions.push(TopBarAction::Favorite);
    }
    if page.cached || page.downloading {
        actions.push(TopBarAction::Download);
    }
    // The ArtistEntry Tool also carries Shuffle, so it stays available even
    // when both status insights are on.
    if artist_tool || page_tool_visible(page) {
        actions.push(TopBarAction::Tool);
    }
    actions
}

fn topbar_actions_for(mode: SuperiorMode, page: Option<&PageBarState>) -> Vec<TopBarAction> {
    match mode {
        SuperiorMode::Home => vec![TopBarAction::Sync, TopBarAction::Wipe],
        SuperiorMode::Album => page.map(|page| page_actions(page, false)).unwrap_or_default(),
        SuperiorMode::Artist => page.map(|page| page_actions(page, true)).unwrap_or_default(),
        SuperiorMode::Playlist | SuperiorMode::Radio => Vec::new(),
    }
}

fn next_tab_slot(order: &[usize], current: Option<usize>, backward: bool) -> Option<usize> {
    let Some(current) = current else {
        return if backward {
            order.last().copied()
        } else {
            order.first().copied()
        };
    };
    order
        .iter()
        .position(|slot| *slot == current)
        .and_then(|position| {
            if backward {
                position.checked_sub(1)
            } else {
                Some(position + 1)
            }
        })
        .and_then(|position| order.get(position).copied())
}

#[derive(Debug, Clone)]
struct PageBarState {
    kind: PageKind,
    id: String,
    title: String,
    subtitle: String,
    meta: String,
    rating: u8,
    starred: bool,
    cached: bool,
    downloading: bool,
}

const LIBRARY_ROW_FLAGS: page_profile::RowFlags = page_profile::RowFlags {
    show_cover: true,
    show_subtitle: true,
    show_length: true,
    compact: false,
};

struct CoverLoadTrace {
    cover_art: String,
    bucket: u32,
    started: std::time::Instant,
    thumb_logged: bool,
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
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
    search_loading_since: Option<f64>,
    search_denied_at: Option<f64>,
    search_denied_query: String,
    search_anim: anim::Behavior,
    clock: anim::Clock,
    palette_motion: palette::PaletteMotion,
    palette_cover: Option<String>,
    hovered_row: Option<usize>,
    hovered_page: Option<usize>,
    selected_id: Option<String>,
    pending_page_reveal: Option<PendingReveal>,
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
    fullscreen_focus: FullscreenFocus,
    fullscreen_anchors: std::rc::Rc<std::cell::RefCell<FullscreenAnchors>>,
    fullscreen_album: Option<(String, bool, u8)>,
    fullscreen_album_requested: Option<String>,
    status_anchors: StatusAnchorSlots,
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
    lyrics_overlay_anim: anim::Behavior,
    lyrics_active_line: Option<usize>,
    lyrics_line_opacity: std::collections::HashMap<usize, anim::Behavior>,
    lyrics_line_emphasis: std::collections::HashMap<usize, anim::Behavior>,
    lyrics_line_color: std::collections::HashMap<usize, anim::ColorBehavior>,
    lyrics_layout: Option<lyrics_layout::LyricsLayout>,
    lyrics_window_start: usize,
    lyrics_shift: anim::Behavior,
    lyrics_track_fade: anim::Behavior,
    transport_press: [Option<f64>; 3],
    line_search: line_search::LineSearchState,
    line_search_anim: anim::Behavior,
    command_line: command_line::CommandLineState,
    command_line_anim: anim::Behavior,
    pending_ipc: Vec<ipc_command::IpcCommand>,
    line_search_snapshot: Vec<line_search::LineTarget>,
    line_search_surfaces: Vec<LineSurface>,
    status_focus: Option<usize>,
    syncing: bool,
    sync_progress: f32,
    sync_fill: anim::Behavior,
    sync_spin_started: f64,
    sync_spin_offset: f32,
    sync_settle: anim::Behavior,
    line_search_denied_at: Option<f64>,
    line_search_denied_query: String,
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
    display_retry: std::collections::HashMap<String, f64>,
    scale_factor: f32,
    cover_trace: Option<CoverLoadTrace>,
    trace: bool,
    frames: u64,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);

        crate::scroll_memory::hydrate();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let command_tx = tx.clone();
        let bridge = bridge::Bridge::new(tx);
        crate::ipc::serve(crate::ipc::socket_path(), move |command| {
            let _ = command_tx.send(Event::Command(command));
        });
        let mut view = ViewState::default();
        bridge.load_library(view.scope, view.order);
        view.loading = true;

        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.recv().await {
                let _ = this.update(cx, |this, cx| {
                    match event {
                        Event::Command(command) => this.handle_command(&command, cx),
                        event => this.apply(event),
                    }
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
            search_loading_since: None,
            search_denied_at: None,
            search_denied_query: String::new(),
            search_anim: anim::Behavior::new(0.0),
            clock: anim::Clock::new(),
            palette_motion: palette::PaletteMotion::new(palette::Palette::default()),
            palette_cover: None,
            hovered_row: None,
            hovered_page: None,
            selected_id: None,
            pending_page_reveal: None,
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
            fullscreen_focus: FullscreenFocus::Cover,
            fullscreen_anchors: std::rc::Rc::new(std::cell::RefCell::new(
                FullscreenAnchors::default(),
            )),
            fullscreen_album: None,
            fullscreen_album_requested: None,
            status_anchors: std::rc::Rc::new(std::cell::RefCell::new([None, None, None, None, None, None])),
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
            lyrics_overlay_anim: anim::Behavior::new(0.0),
            lyrics_active_line: None,
            lyrics_line_opacity: std::collections::HashMap::new(),
            lyrics_line_emphasis: std::collections::HashMap::new(),
            lyrics_line_color: std::collections::HashMap::new(),
            lyrics_layout: None,
            lyrics_window_start: 0,
            lyrics_shift: anim::Behavior::new(0.0),
            lyrics_track_fade: anim::Behavior::new(1.0),
            transport_press: [None; 3],
            line_search: line_search::LineSearchState::new(),
            line_search_anim: anim::Behavior::new(0.0),
            command_line: command_line::CommandLineState::new(),
            command_line_anim: anim::Behavior::new(0.0),
            pending_ipc: Vec::new(),
            line_search_snapshot: Vec::new(),
            line_search_surfaces: Vec::new(),
            status_focus: None,
            syncing: false,
            sync_progress: 0.0,
            sync_fill: anim::Behavior::new(0.0),
            sync_spin_started: 0.0,
            sync_spin_offset: 0.0,
            sync_settle: anim::Behavior::new(0.0),
            line_search_denied_at: None,
            line_search_denied_query: String::new(),
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
            display_retry: std::collections::HashMap::new(),
            scale_factor: window.scale_factor(),
            cover_trace: None,
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
                editor: line_search::LineEditor::from_text(&query),
                rows: Vec::new(),
                most_played: false,
                scope: 0,
                scope_cursors: [0; 4],
                selection: state::Selection::default(),
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
            Event::Command(_) => {}
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
                if matches!(kind, RowKind::Album | RowKind::Artist)
                    && let Some(page) = self.page.as_mut().filter(|page| {
                        page.descriptor.kind != PageKind::Playlist
                            && page.descriptor.kind != PageKind::Radio
                            && page.descriptor.id == id
                    })
                {
                    if let Some(starred) = starred {
                        page.header.starred = starred;
                    }
                    if let Some(rating) = rating {
                        page.header.rating = i64::from(rating);
                    }
                }
                if kind == RowKind::Song
                    && let Some(starred) = starred
                    && self
                        .player
                        .current()
                        .is_some_and(|track| track.id == id)
                {
                    self.player.set_track_favorite(&id, starred);
                }
                if kind == RowKind::Album
                    && let Some((album_id, album_starred, album_rating)) =
                        self.fullscreen_album.as_mut()
                    && *album_id == id
                {
                    if let Some(starred) = starred {
                        *album_starred = starred;
                    }
                    if let Some(rating) = rating {
                        *album_rating = rating;
                    }
                }
            }
            Event::AlbumState {
                id,
                starred,
                rating,
            } => {
                if self
                    .player
                    .current()
                    .is_some_and(|track| track.album_id == id)
                {
                    self.fullscreen_album = Some((id, starred, rating));
                }
            }
            Event::SyncProgress {
                message: _,
                done,
                total,
            } => {
                self.mark_sync_progress(done, total);
            }
            Event::LibrarySynced => {
                self.sync_progress = 1.0;
                self.finish_sync();
                self.marks.requested = None;
                self.bridge.refresh_header();
                self.reload();
                if let Some(page) = self.page.as_ref() {
                    let descriptor = page.descriptor.clone();
                    self.bridge.load_page(descriptor);
                }
                self.push_toast("Library synced".to_string(), ToastKind::Notice, None);
            }
            Event::Wiped { scope } => {
                self.marks.requested = None;
                match scope.as_str() {
                    "covers" => {
                        if let Some(covers) = COVERS.get()
                            && let Ok(mut covers) = covers.lock()
                        {
                            covers.clear();
                        }
                        self.display_covers.clear();
                        self.display_requested.clear();
                        self.display_retry.clear();
                        self.palette_cover = None;
                        self.palette_motion.reset(&self.clock);
                    }
                    "lyrics" => {
                        self.lyrics = None;
                        self.lyrics_requested = self.player.current().map(|track| track.id.clone());
                        self.lyrics_overlay = false;
                        self.lyrics_overlay_wanted = false;
                        self.lyrics_layout = None;
                    }
                    "album" => {
                        if let Some(page) = self.page.as_ref()
                            && matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        {
                            let kind = page.descriptor.kind;
                            let id = page.descriptor.id.clone();
                            self.bridge.request_container_state(kind, id);
                        }
                    }
                    _ => {}
                }
                self.bridge.refresh_header();
                self.reload();
                if let Some(page) = self.page.as_ref() {
                    let descriptor = page.descriptor.clone();
                    self.bridge.load_page(descriptor);
                }
                self.push_toast(format!("Wiped {scope}"), ToastKind::Notice, None);
            }

            Event::PageContainerState {
                kind,
                id,
                cached,
                starred,
                rating,
            } => {
                if let Some(page) = self
                    .page
                    .as_mut()
                    .filter(|page| page.descriptor.kind == kind && page.descriptor.id == id)
                {
                    page.container_cached = cached;
                    page.header.starred = starred;
                    page.header.rating = i64::from(rating);
                    if !cached {
                        page.container_downloading = false;
                    }
                }
            }
            Event::QueueAppend { tracks } => {
                self.player.append(tracks);
            }
            Event::DownloadSettled => {
                self.marks.requested = None;
                self.ensure_marks();
                if let Some((kind, id)) = self.page.as_ref().and_then(|page| {
                    matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        .then(|| (page.descriptor.kind, page.descriptor.id.clone()))
                }) {
                    if let Some(page) = self.page.as_mut() {
                        page.container_downloading = false;
                    }
                    self.bridge.request_container_state(kind, id);
                }
            }
            Event::OpenPage { descriptor } => {
                self.open_descriptor(descriptor);
            }
            Event::OpenPageReveal {
                descriptor,
                row_kind,
                row_id,
            } => {
                let reveal = PendingReveal {
                    page_kind: descriptor.kind,
                    page_id: descriptor.id.clone(),
                    row_kind,
                    row_id,
                };
                self.open_descriptor_reveal(descriptor, Some(reveal));
            }
            Event::DisplayCover {
                key,
                cover_art,
                target,
                path,
            } => {
                match path {
                    Some(path) => {
                        self.display_retry.remove(&key);
                        self.display_covers.insert(key, path.clone());
                        if let Some(trace) = self.cover_trace.as_ref()
                            && trace.cover_art == cover_art
                            && trace.bucket == target
                        {
                            crate::debug_log!(
                                "album entry: proper res loaded cover={} bucket={} path={} elapsed_ms={:.1} at={}",
                                cover_art,
                                target,
                                path.display(),
                                trace.started.elapsed().as_secs_f64() * 1000.0,
                                unix_ms()
                            );
                        }
                    }
                    None => {
                        self.display_requested.remove(&key);
                        self.display_retry.insert(key, self.clock.now + 5.0);
                        crate::debug_log!(
                            "album entry: cover load failed cover={} bucket={} path=none",
                            cover_art,
                            target
                        );
                    }
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
                let reveal_index = self.pending_page_reveal.as_ref().and_then(|pending| {
                    if page.descriptor.kind != pending.page_kind
                        || page.descriptor.id != pending.page_id
                    {
                        return None;
                    }
                    page.rows.iter().position(|row| match pending.row_kind {
                        state::RowKind::Song => {
                            row.song_id == pending.row_id || row.id == pending.row_id
                        }
                        _ => row.id == pending.row_id,
                    })
                });
                if let Some(index) = reveal_index {
                    self.pending_page_reveal = None;
                    page.cursor = index;
                    page.selection.clear();
                }
                self.follow_page(false);
                if let Some(index) = reveal_index {
                    self.land_page(index);
                }
                self.marks.requested = None;
                self.ensure_marks();
                if let Some((kind, id)) = self.page.as_ref().and_then(|page| {
                    matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        .then(|| (page.descriptor.kind, page.descriptor.id.clone()))
                }) {
                    self.bridge.request_container_state(kind, id);
                }
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
                    self.lyrics_line_emphasis.clear();
                    self.lyrics_line_color.clear();
                    self.lyrics_layout = None;
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
                if self.queue_open {
                    self.land_queue(self.player.index());
                }
            }
            Event::Search {
                generation,
                query,
                rows,
                most_played,
            } => {
                if let Some(search) = self.search.as_mut()
                    && generation >= search.generation
                {
                    if search.editor.text != query {
                        return;
                    }
                    search.generation = generation;
                    search.rows = rows;
                    search.most_played = most_played;
                    search.scope_cursors = [0; 4];
                    search.selection.clear();
                    self.hovered_row = None;
                    search.loading = false;
                    self.search_loading_since = None;
                    if !most_played
                        && !search.editor.text.is_empty()
                        && search.rows.is_empty()
                    {
                        self.search_denied_at = Some(self.clock.now);
                        self.search_denied_query = search.editor.text.clone();
                    }
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
                self.follow_search(false);
            }
            Event::Error(message) => {
                self.finish_sync();
                let reconcile = self
                    .page
                    .as_mut()
                    .filter(|page| page.container_downloading)
                    .map(|page| {
                        page.container_downloading = false;
                        (page.descriptor.kind, page.descriptor.id.clone())
                    });
                if let Some((kind, id)) = reconcile
                    && matches!(kind, PageKind::Album | PageKind::Artist)
                {
                    self.bridge.request_container_state(kind, id);
                }
                if let Some(search) = self.search.as_mut() {
                    search.loading = false;
                }
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
        self.status_focus = None;
        self.clock.tick();
        let Some(page) = self.page.as_ref() else { return };
        self.selected_id = page.rows.get(page.cursor).map(|row| row.id.clone());
        self.page_scroll_modal
            .start(page.rows.len(), page.cursor, animated, self.clock.now);
    }

    fn follow_search(&mut self, animated: bool) {
        let Some(search) = self.search.as_ref() else { return };
        let scope = search.scope;
        let count = search.scope_count(scope);
        let cursor = search.cursor();
        self.clock.tick();
        self.search_scroll
            .start(count, cursor, animated, self.clock.now);
    }

    fn follow_queue(&mut self, animated: bool) {
        self.status_focus = None;
        self.clock.tick();
        self.selected_id = self
            .player
            .queue_snapshot()
            .get(self.queue_cursor)
            .map(|track| track.id.clone());
        self.queue_scroll_modal
            .start(self.player.len(), self.queue_cursor, animated, self.clock.now);
    }

    fn land_library(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        let cursor = self
            .library_scroll
            .land(self.view.rows.count, index);
        self.view.select(cursor);
        self.selected_id = self.view.rows.get(cursor).map(|row| row.id.clone());
        self.view.rows.requested = None;
        self.ensure_library_window();
        self.remember_current();
    }

    fn land_page(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        let Some(page) = self.page.as_mut() else { return };
        let cursor = self.page_scroll_modal.land(page.rows.len(), index);
        page.select(cursor);
        self.selected_id = page.rows.get(cursor).map(|row| row.id.clone());
        self.hovered_page = None;
    }

    fn land_queue(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        let cursor = self.queue_scroll_modal.land(self.player.len(), index);
        self.queue_cursor = cursor;
        self.selected_id = self
            .player
            .queue_snapshot()
            .get(cursor)
            .map(|track| track.id.clone());
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
        let measured = f32::from(
            self.search_scroll
                .handle
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .height,
        );
        let viewport = if measured > 0.0 {
            measured
        } else {
            (window_height * 0.72).max(1.0)
        };
        if self.search_scroll.ensure_metrics(viewport) {
            self.follow_search(false);
        }
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
        // The fullscreen title shows the current track's marks even when it is
        // not part of the visible list window.
        if let Some(track) = self.player.current() {
            if !track.id.is_empty() && seen.insert(format!("s:{}", track.id)) {
                songs.push(track.id.clone());
            }
            if !track.album_id.is_empty() && seen.insert(format!("a:{}", track.album_id)) {
                albums.push(track.album_id.clone());
            }
        }
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

    fn refresh_fullscreen_album_state(&mut self) {
        let Some(track) = self.player.current() else {
            return;
        };
        let album_id = track.album_id.clone();
        if album_id.is_empty() {
            return;
        }
        if self
            .fullscreen_album
            .as_ref()
            .is_some_and(|(id, _, _)| *id == album_id)
        {
            return;
        }
        if self.fullscreen_album_requested.as_deref() == Some(album_id.as_str()) {
            return;
        }
        self.fullscreen_album_requested = Some(album_id.clone());
        self.bridge.request_album_state(album_id);
    }

    fn page_hero_bucket(&self) -> u32 {
        covers::bucket_for(PAGE_HERO_PX, self.scale_factor)
    }

    fn fullscreen_hero_bucket(&self) -> u32 {
        covers::bucket_for(FULLSCREEN_HERO_PX, self.scale_factor)
    }

    fn hero_cover(&self, url: &str, target: u32) -> Option<std::path::PathBuf> {
        COVERS
            .get_or_init(|| std::sync::Mutex::new(covers::CoverStore::new(768)))
            .lock()
            .ok()?
            .hero_url(url, target)
    }

    fn ensure_display_cover(&mut self, cover_art: &str, target: u32) {
        if cover_art.is_empty() {
            return;
        }
        let key = format!("{cover_art}:{target}");
        if let Some(retry_at) = self.display_retry.get(&key)
            && *retry_at > self.clock.now
        {
            return;
        }
        self.display_retry.remove(&key);
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

    fn reset_row_scales(&mut self) {
        self.clock.tick();
        for scale in self.row_scales.values_mut() {
            scale.target(1.0, 0.0, anim::Curve::Standard, &self.clock);
        }
    }

    fn select(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        if !self.visual_mode {
            self.view.selection.clear();
        }
        self.view.select(index);
        self.selected_id = self.view.rows.get(index).map(|row| row.id.clone());
        self.hovered_row = None;
        self.follow_library(true);
        self.remember_current();
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn on_move_down(&mut self, _: &MoveDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.status_focus = None;
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(1);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.reset_row_scales();
            self.queue_cursor = self.queue_cursor.saturating_add(1).min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
            cx.notify();
            return;
        }
        self.reset_row_scales();
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
        self.status_focus = None;
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(-1);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.reset_row_scales();
            self.queue_cursor = self.queue_cursor.saturating_sub(1);
            self.follow_queue(true);
            cx.notify();
            return;
        }
        self.reset_row_scales();
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

    const SYNC_TURN_SECS: f64 = 1.05;

    fn sync_angle(&self) -> f32 {
        if self.syncing {
            let elapsed = ((self.clock.now - self.sync_spin_started).max(0.0)
                / Self::SYNC_TURN_SECS) as f32;
            (self.sync_spin_offset + elapsed * 360.0).rem_euclid(360.0)
        } else {
            self.sync_settle.value(&self.clock)
        }
    }

    fn start_sync(&mut self, mode: SyncMode) {
        if self.syncing {
            self.push_toast(
                "A sync is already running.".to_string(),
                ToastKind::Notice,
                None,
            );
            return;
        }
        self.clock.tick();
        self.syncing = true;
        self.sync_progress = 0.0;
        self.sync_fill = anim::Behavior::new(0.0);
        self.sync_spin_started = self.clock.now;
        self.sync_spin_offset = self.sync_settle.value(&self.clock);
        self.bridge.sync(mode);
    }

    fn finish_sync(&mut self) {
        if !self.syncing {
            return;
        }
        self.clock.tick();
        let angle = self.sync_angle();
        self.syncing = false;
        let target = ((angle / 360.0).ceil() * 360.0).max(360.0);
        self.sync_fill
            .target(1.0, anim::QUICK_MS, anim::Curve::Linear, &self.clock);
        self.sync_settle = anim::Behavior::new(angle);
        self.sync_settle.target(
            target,
            anim::STANDARD_MS,
            anim::Curve::Bounce,
            &self.clock,
        );
    }

    fn mark_sync_progress(&mut self, done: usize, total: usize) {
        self.clock.tick();
        if !self.syncing {
            self.syncing = true;
            self.sync_spin_started = self.clock.now;
            self.sync_spin_offset = self.sync_settle.value(&self.clock);
        }
        self.sync_progress = (done as f32 / total.max(1) as f32).clamp(0.0, 1.0);
        self.sync_fill.target(
            self.sync_progress,
            anim::QUICK_MS,
            anim::Curve::Linear,
            &self.clock,
        );
    }

    fn activate_status(&mut self, slot: usize) {
        self.status_focus = Some(slot);
        match slot {
            0 => self.start_sync(SyncMode::Incremental),
            1 => {
                if self.syncing {
                    self.push_toast(
                        "Wait for the sync to finish before wiping.".to_string(),
                        ToastKind::Notice,
                        None,
                    );
                    return;
                }
                self.open_wipe_menu();
            }
            2 => self.activate_container_download(),
            4 => self.activate_container_favorite(),
            5 => self.open_page_menu(5, true),
            _ => {}
        }
    }

    fn activate_container_download(&mut self) {
        let Some(page) = self.page_bar() else { return };
        if page.downloading {
            return;
        }
        let on = !page.cached;
        if let Some(page) = self.page.as_mut() {
            page.container_cached = on;
            page.container_downloading = true;
        }
        match page.kind {
            PageKind::Album => {
                self.bridge.quick_download("album".to_string(), page.id, on);
            }
            PageKind::Artist => {
                self.bridge.download_artist(page.id, on);
            }
            _ => {}
        }
    }

    fn activate_container_favorite(&mut self) {
        let Some(page) = self.page_bar() else { return };
        let kind = match page.kind {
            PageKind::Album => "album",
            PageKind::Artist => "artist",
            _ => return,
        };
        if let Some(page) = self.page.as_mut() {
            page.header.starred = !page.header.starred;
        }
        self.bridge
            .quick_favorite(kind.to_string(), page.id, !page.starred);
    }

    fn animate_alt(&mut self, target: f32) {
        // Keep the legacy PopFx pop on open; the faster ALT_MS applies to
        // both directions, and the collapse keeps its plain ease-in.
        let curve = if target > 0.5 {
            anim::Curve::OutBack
        } else {
            anim::Curve::Accelerate
        };
        self.alt_spawn
            .target(target, anim::ALT_MS, curve, &self.clock);
    }

    fn page_alt_menu(&self, page: &PageBarState) -> Option<(QuickMenu, AltTarget)> {
        let (menu, kind) = match page.kind {
            PageKind::Album => (
                QuickMenu::album_tool(page.cached, page.starred, page.rating),
                RowKind::Album,
            ),
            PageKind::Artist => (
                QuickMenu::artist_tool(page.cached, page.starred, page.rating),
                RowKind::Artist,
            ),
            _ => return None,
        };
        Some((
            menu,
            AltTarget {
                kind,
                id: page.id.clone(),
                album_id: if page.kind == PageKind::Album {
                    page.id.clone()
                } else {
                    String::new()
                },
                song_id: String::new(),
                playlist_id: String::new(),
                ids: vec![page.id.clone()],
                selection: false,
            },
        ))
    }

    fn open_page_menu(&mut self, anchor_slot: usize, sticky: bool) {
        let Some(page) = self.page_bar() else { return };
        let Some((menu, target)) = self.page_alt_menu(&page) else {
            return;
        };
        self.clock.tick();
        self.alt = AltGesture::with_sticky(sticky);
        if !self.alt.arm(&menu) {
            return;
        }
        self.alt_menu = Some(menu);
        self.alt_target = Some(target);
        self.alt_anchor = Some(self.status_anchor(anchor_slot));
        self.animate_alt(1.0);
    }

    fn open_wipe_menu(&mut self) {
        self.clock.tick();
        let menu = QuickMenu::wipe();
        self.alt = AltGesture::with_sticky(true);
        if !self.alt.arm(&menu) {
            return;
        }
        self.alt_menu = Some(menu);
        self.alt_target = None;
        self.alt_anchor = Some(self.status_anchor(1));
        self.animate_alt(1.0);
    }

    fn on_refresh(&mut self, _: &Refresh, _window: &mut Window, cx: &mut Context<Self>) {
        self.start_sync(SyncMode::Incremental);
        cx.notify();
    }

    fn cycle_fullscreen_focus(&mut self, backward: bool) {
        let mut order = vec![FullscreenFocus::Cover, FullscreenFocus::Title];
        if self.lyrics_inline() && !self.lyrics_overlay {
            order.push(FullscreenFocus::Lyrics);
        }
        let position = order
            .iter()
            .position(|focus| *focus == self.fullscreen_focus)
            .unwrap_or(0);
        let next = if backward {
            (position + order.len() - 1) % order.len()
        } else {
            (position + 1) % order.len()
        };
        self.fullscreen_focus = order[next];
    }

    fn on_toggle_fullscreen(&mut self, _: &ToggleFullscreen, _window: &mut Window, cx: &mut Context<Self>) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            self.request_current_lyrics();
            self.ensure_marks();
            self.refresh_fullscreen_album_state();
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
        self.status_focus = None;
        self.queue_open = !self.queue_open;
        if self.queue_open {
            self.land_queue(self.player.index());
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
            self.land_queue(self.player.index());
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
        if self.line_search.open {
            self.line_search_step(1);
            cx.notify();
            return;
        }
        if self.search.is_some() && self.search_open {
            self.move_search_scope(1);
            cx.notify();
            return;
        }
        self.on_tab(false, cx);
    }

    fn on_search_scope_prev(&mut self, _: &SearchScopePrev, _window: &mut Window, cx: &mut Context<Self>) {
        if self.line_search.open {
            self.line_search_step(-1);
            cx.notify();
            return;
        }
        if self.search.is_some() && self.search_open {
            self.move_search_scope(-1);
            cx.notify();
            return;
        }
        self.on_tab(true, cx);
    }

    /// Tab's contextual walk. The superior bar exposes the current page's
    /// chips (the artist entry adds Download/Shuffle/Favorite) followed by
    /// the status actions; leaving the end drops back to the content.
    fn on_tab(&mut self, backward: bool, cx: &mut Context<Self>) {
        if self.fullscreen {
            if !self.lyrics_overlay {
                self.cycle_fullscreen_focus(backward);
            }
            cx.notify();
            return;
        }
        let order = self.topbar_tab_order();
        self.status_focus = next_tab_slot(&order, self.status_focus, backward);
        cx.notify();
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
        search.selection.clear();
        self.follow_search(false);
    }

    fn open_search(&mut self, typing: bool) {
        self.status_focus = None;
        self.search_open = true;
        self.search = Some(SearchState {
            editor: line_search::LineEditor::new(),
            rows: Vec::new(),
            most_played: false,
            scope: 0,
            scope_cursors: [0; 4],
            selection: state::Selection::default(),
            generation: 0,
            loading: true,
            typing,
        });
        self.clock.tick();
        self.search_loading_since = Some(self.clock.now);
        self.search_denied_at = None;
        self.bridge.search(String::new());
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
        self.close_line_search();
        self.alt.cancel();
        self.alt_menu = None;
        self.alt_target = None;
        self.animate_alt(0.0);
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

    fn fullscreen_anchor(&self, focus: FullscreenFocus) -> (f32, f32) {
        let anchors = self.fullscreen_anchors.borrow();
        let stored = match focus {
            FullscreenFocus::Cover => anchors.cover,
            FullscreenFocus::Title => anchors.title,
            FullscreenFocus::Lyrics => None,
        };
        let (x, y) = stored.unwrap_or(match focus {
            FullscreenFocus::Cover => (self.win_w * 0.5, self.win_h * 0.42),
            FullscreenFocus::Title => (self.win_w * 0.5, self.win_h * 0.86),
            FullscreenFocus::Lyrics => (self.win_w * 0.5, self.win_h * 0.5),
        });
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    fn alt_scroll_anchor(
        &self,
        modal: &scroll_modal::ScrollModal,
        index: usize,
        count: usize,
    ) -> (f32, f32) {
        modal
            .row_center(index, count)
            .map(|(x, y)| {
                (
                    x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
                    y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
                )
            })
            .unwrap_or_else(|| self.alt_anchor_for(&modal.handle, index, modal.row_h))
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

    fn search_anchor_for(&self, index: usize, row_h: f32) -> (f32, f32) {
        if let Some(search) = self.search.as_ref() {
            let count = search.scope_count(search.scope);
            if let Some((x, y)) = self.search_scroll.row_center(index, count) {
                return (
                    x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
                    y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
                );
            }
        }
        let state = self.search_scroll.handle.0.borrow();
        let bounds = state.base_handle.bounds();
        let height = f32::from(bounds.size.height);
        let (x, y) = if height > 0.0 {
            (
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) * 0.5,
                f32::from(bounds.origin.y)
                    + index as f32 * row_h
                    + f32::from(state.base_handle.offset().y)
                    + row_h * 0.5,
            )
        } else {
            (
                self.win_w * 0.5,
                self.win_h * 0.14 + 16.0 + 54.0 + 12.0 + 30.0 + 12.0
                    + index as f32 * row_h
                    + row_h * 0.5,
            )
        };
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    fn superior_mode(&self) -> SuperiorMode {
        SuperiorMode::from_page(self.page.as_ref())
    }

    fn page_bar(&self) -> Option<PageBarState> {
        let page = self.page.as_ref()?;
        Some(PageBarState {
            kind: page.descriptor.kind,
            id: page.descriptor.id.clone(),
            title: page.header.title.clone(),
            subtitle: page.header.subtitle.clone(),
            meta: page.header.meta.clone(),
            rating: page.header.rating.clamp(0, 5) as u8,
            starred: page.header.starred,
            cached: page.container_cached,
            downloading: page.container_downloading,
        })
    }

    fn topbar_actions(&self) -> Vec<TopBarAction> {
        let page = self.page_bar();
        topbar_actions_for(self.superior_mode(), page.as_ref())
    }

    fn topbar_tab_order(&self) -> Vec<usize> {
        self.topbar_actions()
            .into_iter()
            .map(TopBarAction::slot)
            .collect()
    }

    fn status_anchor(&self, scope: usize) -> (f32, f32) {
        let stored = self.status_anchors.borrow()[scope.min(5)];
        let (x, y) = stored.unwrap_or((
            (self.win_w - 42.0).max(30.0),
            (self.win_h * 0.0648 * 0.5).max(30.0),
        ));
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    fn alt_snapshot(&self) -> Option<(QuickMenu, AltTarget, (f32, f32))> {
        if self.fullscreen {
            let track = self.player.current()?;
            if self.fullscreen_focus == FullscreenFocus::Lyrics {
                return None;
            }
            if self.fullscreen_focus == FullscreenFocus::Cover && !track.album_id.is_empty() {
                let album_id = track.album_id.clone();
                let (starred, rating) = self
                    .fullscreen_album
                    .as_ref()
                    .filter(|(id, _, _)| *id == album_id)
                    .map(|(_, starred, rating)| (*starred, *rating))
                    .unwrap_or((false, 0));
                let cached = self.marks.albums_done.contains(&album_id);
                let menu = QuickMenu::album_with(cached, starred, rating);
                let target = AltTarget {
                    kind: RowKind::Album,
                    id: album_id.clone(),
                    album_id: album_id.clone(),
                    song_id: String::new(),
                    playlist_id: String::new(),
                    ids: vec![album_id],
                    selection: false,
                };
                return Some((
                    menu,
                    target,
                    self.fullscreen_anchor(FullscreenFocus::Cover),
                ));
            }
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
                self.fullscreen_anchor(FullscreenFocus::Title),
            ));
        }
        if let Some(focus) = self.status_focus {
            // The scoped status region owns Alt. Wipe opens the maintenance
            // board; Favorite and Tool open the page's contextual actions.
            if focus == TopBarAction::Wipe.slot() {
                if self.syncing {
                    return None;
                }
                return Some((
                    QuickMenu::wipe(),
                    AltTarget {
                        kind: RowKind::Song,
                        id: String::new(),
                        album_id: String::new(),
                        song_id: String::new(),
                        playlist_id: String::new(),
                        ids: Vec::new(),
                        selection: false,
                    },
                    self.status_anchor(focus),
                ));
            }
            if focus == TopBarAction::Favorite.slot() || focus == TopBarAction::Tool.slot() {
                let page = self.page_bar()?;
                let (menu, target) = self.page_alt_menu(&page)?;
                return Some((menu, target, self.status_anchor(focus)));
            }
            return None;
        }
        if let Some(search) = self.search.as_ref().filter(|_| self.search_open) {
            let scoped = search.scoped();
            let cursor = search.cursor();
            let anchor = self.search_anchor_for(cursor, 56.0);
            if search.selection.active() {
                let anchor_index = search.selection.anchor().unwrap_or(cursor);
                let lo = anchor_index.min(cursor);
                let hi = anchor_index.max(cursor);
                let rows = scoped
                    .get(lo..=hi.min(scoped.len().saturating_sub(1)))
                    .unwrap_or_default()
                    .iter()
                    .map(|row| (*row).clone())
                    .collect::<Vec<_>>();
                if let Some(hit) =
                    self.alt_selection_target(rows, cursor.saturating_sub(lo), anchor)
                {
                    return Some(hit);
                }
            }
            let row = scoped.get(cursor)?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                anchor,
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
                self.alt_scroll_anchor(&self.queue_scroll_modal, self.queue_cursor, self.player.len()),
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
                self.alt_scroll_anchor(&self.page_scroll_modal, page.cursor, page.rows.len()),
            ) {
                return Some(hit);
            }
        }
        if let Some(page) = self.page.as_ref() {
            let row = page.rows.get(page.cursor)?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                self.alt_scroll_anchor(&self.page_scroll_modal, page.cursor, page.rows.len()),
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
                self.alt_scroll_anchor(&self.library_scroll, self.view.cursor, self.view.rows.count),
            ) {
                return Some(hit);
            }
        }
        let row = self.view.rows.get(self.view.cursor)?;
        Some((
            self.alt_menu_for_row(row),
            self.alt_target_for_row(row),
            self.alt_scroll_anchor(&self.library_scroll, self.view.cursor, self.view.rows.count),
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
        if self.status_focus.is_some() {
            self.alt = AltGesture::new();
        }
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
        self.animate_alt(1.0);
    }

    fn alt_release(&mut self) {
        self.clock.tick();
        if self.alt_was_typing {
            if let Some(search) = self.search.as_mut() {
                search.selection.clear();
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
        self.animate_alt(0.0);
    }

    fn dispatch_quick(&mut self, verb: QuickVerb) {
        match verb {
            QuickVerb::WipeLibrary => {
                self.bridge.wipe("library".to_string());
                return;
            }
            QuickVerb::WipeIndexes => {
                self.bridge.wipe("indexes".to_string());
                return;
            }
            QuickVerb::WipeCovers => {
                self.bridge.wipe("covers".to_string());
                return;
            }
            _ => {}
        }
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
            QuickVerb::ShuffleArtist => {
                self.bridge.shuffle_artist(target.id.clone());
            }
            QuickVerb::ShuffleAlbum => {
                self.bridge.shuffle_album(target.id.clone());
            }
            QuickVerb::Download { on } => {
                if target.kind == RowKind::Artist {
                    self.bridge.download_artist(target.id.clone(), on);
                } else {
                    self.bridge
                        .quick_download(target.queue_kind().to_string(), target.id.clone(), on);
                }
            }
            QuickVerb::WipeLibrary | QuickVerb::WipeIndexes | QuickVerb::WipeCovers => {}
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
                self.animate_alt(0.0);
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
        let Some(surface) = self.line_search_surfaces.get(hit.target).copied() else {
            return;
        };
        match surface {
            LineSurface::Search => {
                if let Some(search) = self.search.as_mut() {
                    search.scope_cursors[search.scope] = hit.row;
                }
            }
            LineSurface::Queue => {
                self.queue_cursor = hit.row.min(self.player.len().saturating_sub(1));
                self.follow_queue(true);
            }
            LineSurface::Page => {
                if let Some(page) = self.page.as_mut() {
                    page.select(hit.row);
                }
                self.follow_page(true);
            }
            LineSurface::Library => {
                self.select(hit.row);
            }
            LineSurface::Lyrics => {
                self.lyrics_window_start = hit.row.saturating_sub(2);
            }
            LineSurface::Status => {
                if let Some(slot) = self.topbar_tab_order().get(hit.row) {
                    self.status_focus = Some(*slot);
                }
            }
        }
    }

    fn line_search_step(&mut self, delta: isize) {
        self.line_search.step(delta);
        if let Some(hit) = self.line_search.current_hit() {
            self.apply_line_hit(hit);
        }
    }

    fn line_search_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A typo after a valid prefix must not make Enter a no-op: fall back
        // to the last candidate that actually matched in this session.
        if let Some(hit) = self.line_search.latest_hit() {
            let status = self.line_search_surfaces.get(hit.target) == Some(&LineSurface::Status);
            let status_slot = self.topbar_tab_order().get(hit.row).copied();
            self.apply_line_hit(hit);
            self.close_line_search();
            if status {
                if let Some(slot) = status_slot {
                    self.activate_status(slot);
                }
            } else {
                self.on_enter(&Enter, window, cx);
            }
        } else if self.line_search.query().is_empty() {
            self.close_line_search();
        } else {
            self.flash_line_search_denied();
        }
        cx.notify();
    }

    fn line_search_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        let modified = control || event.keystroke.modifiers.alt || event.keystroke.modifiers.platform;
        match key {
            "escape" => {
                self.close_line_search();
                cx.notify();
                return;
            }
            "enter" => {
                self.line_search_commit(window, cx);
                return;
            }
            "tab" => {
                self.line_search_step(1);
                cx.notify();
                return;
            }
            "backtab" => {
                self.line_search_step(-1);
                cx.notify();
                return;
            }
            "left" => self.line_search.editor_mut().move_left(shift),
            "right" => self.line_search.editor_mut().move_right(shift),
            "home" => self.line_search.editor_mut().home(shift),
            "end" => self.line_search.editor_mut().end(shift),
            "backspace" => self.line_search.editor_mut().backspace(),
            "delete" => self.line_search.editor_mut().delete_forward(),
            "a" if control => self.line_search.editor_mut().select_all(),
            "w" if control => self.line_search.editor_mut().delete_word_back(),
            "c" if control => {
                if let Some(text) = self.line_search.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                if let Some(text) = self.line_search.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.line_search.editor_mut().replace_selection("");
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.line_search.editor_mut().insert(&text);
                }
            }
            _ if !modified => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !text.chars().any(char::is_control)
                {
                    self.line_search.editor_mut().insert(text);
                } else if key == "space" {
                    self.line_search.editor_mut().insert(" ");
                } else {
                    return;
                }
            }
            _ => return,
        }
        self.refresh_line_search();
        cx.notify();
    }


    fn activate_search_result(&mut self, cx: &mut Context<Self>) {
        let row = self
            .search
            .as_ref()
            .and_then(|search| search.scoped().get(search.cursor()).cloned().cloned());
        let Some(row) = row else {
            cx.notify();
            return;
        };
        self.search_open = false;
        self.search = None;
        // A chosen result is a navigation: dismiss the surfaces the search was
        // floating over so the destination is visible.
        self.fullscreen = false;
        self.lyrics_overlay = false;
        self.lyrics_overlay_wanted = false;
        self.queue_open = false;
        self.description = None;
        match row.kind {
            RowKind::Song => {
                if row.album_id.is_empty() {
                    self.activate_row(&row);
                } else {
                    let reveal = PendingReveal {
                        page_kind: PageKind::Album,
                        page_id: row.album_id.clone(),
                        row_kind: RowKind::Song,
                        row_id: row.song_id.clone(),
                    };
                    self.open_descriptor_reveal(
                        PageDescriptor::new(PageKind::Album, row.album_id.clone()),
                        Some(reveal),
                    );
                }
            }
            RowKind::Album => {
                self.bridge.open_album_artist_reveal(row.id.clone());
            }
            RowKind::Artist => {
                self.open_descriptor(PageDescriptor::new(PageKind::Artist, row.id.clone()));
            }
            RowKind::Playlist => {
                self.open_descriptor(PageDescriptor::new(PageKind::Playlist, row.id.clone()));
            }
        }
        cx.notify();
    }

    fn search_edited(&mut self) {
        let Some(search) = self.search.as_ref() else { return };
        let query = search.editor.text.clone();
        let empty = query.is_empty();
        if let Some(search) = self.search.as_mut() {
            search.loading = true;
            if empty {
                search.rows.clear();
                search.most_played = true;
            }
        }
        self.clock.tick();
        self.search_loading_since = Some(self.clock.now);
        self.search_denied_at = None;
        self.bridge.search(query);
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
        self.lyrics_active_line = Some(active);
        for (index, _) in lyrics.lines.iter().enumerate() {
            let opacity = if !lyrics.synced {
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
                .or_insert_with(|| anim::Behavior::new(opacity))
                .target(opacity, anim::QUICK_MS, anim::Curve::Standard, &self.clock);

            let emphasis = if lyrics.synced && index == active {
                lyrics_layout::EMPHASIS
            } else {
                1.0
            };
            self.lyrics_line_emphasis
                .entry(index)
                .or_insert_with(|| anim::Behavior::new(1.0))
                .target(emphasis, anim::QUICK_MS, anim::Curve::Standard, &self.clock);

            let color = if lyrics.synced && index == active {
                palette::on_surface()
            } else {
                palette::on_surface_variant()
            };
            self.lyrics_line_color
                .entry(index)
                .or_insert_with(|| anim::ColorBehavior::new(color))
                .target(color, anim::MICRO_MS, anim::Curve::Standard, &self.clock);
        }
    }

    fn lyric_wrap_count(window: &Window, text: &str, size: f32, width: f32) -> usize {
        if width <= 0.0 || text.is_empty() {
            return 1;
        }
        let mut wrapper = window
            .text_system()
            .line_wrapper(gpui::font("Overused Grotesk Roman"), px(size));
        wrapper
            .wrap_line(&[gpui::LineFragment::text(text)], px(width))
            .count()
            .max(1)
    }

    fn sync_lyrics_layout(&mut self, window: &Window) {
        let Some(lyrics) = self.lyrics.as_ref() else {
            self.lyrics_layout = None;
            return;
        };
        if lyrics.lines.is_empty() {
            self.lyrics_layout = None;
            return;
        }
        let overlay = self.lyrics_overlay;
        let (width, height) = if overlay {
            (
                ((self.win_w * 0.72).min(720.0) - 32.0).max(1.0),
                (self.win_h * 0.80).max(1.0),
            )
        } else if self.fullscreen {
            (
                ((self.win_w * 0.95 - 24.0) * 0.5 - 24.0).max(1.0),
                (self.win_h * 0.55).max(1.0),
            )
        } else {
            self.lyrics_layout = None;
            return;
        };
        let active = self
            .lyrics_active_line
            .unwrap_or(0)
            .min(lyrics.lines.len() - 1);
        let mut base = (height * 0.055).clamp(13.0, 22.0);
        let mut layout = lyrics_layout::layout(
            &lyrics.lines,
            active,
            width,
            height,
            base,
            |text, size| Self::lyric_wrap_count(window, text, size, width),
        );
        // A single very long line can still be taller than the pane at the
        // nominal size. Step the type down until it fits, then keep it.
        while base > 13.0
            && layout
                .rows
                .iter()
                .any(|row| row.height > height)
        {
            base -= 1.0;
            layout = lyrics_layout::layout(
                &lyrics.lines,
                active,
                width,
                height,
                base,
                |text, size| Self::lyric_wrap_count(window, text, size, width),
            );
        }
        let old_start = self.lyrics_window_start;
        if layout.start != old_start {
            let unit = layout.text_height.max(1.0);
            self.lyrics_shift.target(
                (old_start as f32 - layout.start as f32) * unit,
                0.0,
                anim::Curve::Standard,
                &self.clock,
            );
            self.lyrics_shift
                .target(0.0, 300.0, anim::Curve::OutCubic, &self.clock);
        } else {
            self.lyrics_shift.target(0.0, 0.0, anim::Curve::Standard, &self.clock);
        }
        self.lyrics_window_start = layout.start;
        self.lyrics_layout = Some(layout);
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

    fn clear_line_search_selection(&mut self) {
        self.visual_mode = false;
        self.queue_selection.clear();
        self.view.selection.clear();
        if let Some(page) = self.page.as_mut() {
            page.selection.clear();
        }
    }

    fn line_search_targets(&self) -> (Vec<line_search::LineTarget>, Vec<LineSurface>) {
        use line_search::{LineRow, LineTarget};
        let mut targets = Vec::new();
        let mut surfaces = Vec::new();

        if self.search_open
            && let Some(search) = self.search.as_ref()
        {
            let rows = search.scoped();
            let visible = 12usize;
            let mut lines = vec![None; rows.len()];
            for (index, row) in rows.iter().take(visible).enumerate() {
                lines[index] = Some(LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone()));
            }
            targets.push(LineTarget::new(lines));
            surfaces.push(LineSurface::Search);
            return (targets, surfaces);
        }

        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let range = self.queue_scroll_modal.visible_range(queue.len());
            let mut lines = vec![None; queue.len()];
            for (index, track) in queue.iter().enumerate() {
                if range.contains(&index) {
                    let duration = crate::browse::rows::format_duration(Some(track.duration as i32));
                    lines[index] = Some(
                        LineRow::with_extra(track.title.clone(), track.artist.clone(), "", vec![duration])
                            .with_search_fields([0, 1, 3]),
                    );
                }
            }
            targets.push(LineTarget::new(lines));
            surfaces.push(LineSurface::Queue);
            return (targets, surfaces);
        }

        if self.fullscreen {
            if let Some(lyrics) = self.lyrics.as_ref() {
                let count = lyrics.lines.len();
                let visible = 15usize;
                let start = self.lyrics_window_start.min(count);
                let end = (start + visible).min(count);
                let mut lines = vec![None; count];
                for (index, (_, text)) in lyrics.lines.iter().enumerate() {
                    if (start..end).contains(&index) {
                        lines[index] = Some(LineRow::new(text.clone(), "", "").with_search_fields([0]));
                    }
                }
                targets.push(LineTarget::new(lines));
                surfaces.push(LineSurface::Lyrics);
            }
            return (targets, surfaces);
        }

        // The status bar's two objects are part of the plain-window buffer,
        // before the active list, exactly like the legacy `/` contract.
        let mode = self.superior_mode();
        let status_rows = self
            .topbar_actions()
            .into_iter()
            .map(|action| Some(LineRow::new(action.label(mode), "Superior bar", "")))
            .collect();
        targets.push(LineTarget::new(status_rows));
        surfaces.push(LineSurface::Status);

        if let Some(page) = self.page.as_ref() {
            let range = self.page_scroll_modal.visible_range(page.rows.len());
            let artist = matches!(page.descriptor.kind, state::PageKind::Artist);
            let mut lines = vec![None; page.rows.len()];
            for (index, row) in page.rows.iter().enumerate() {
                if !range.contains(&index) {
                    continue;
                }
                let line = if artist {
                    LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone())
                } else {
                    LineRow::with_extra(
                        row.title.clone(),
                        row.subtitle.clone(),
                        row.meta.clone(),
                        vec![row.track.clone()],
                    )
                    .with_search_fields([0, 1, 2, 3])
                };
                lines[index] = Some(line);
            }
            targets.push(LineTarget::new(lines));
            surfaces.push(LineSurface::Page);
            return (targets, surfaces);
        }

        let count = self.view.rows.count;
        let range = self.library_scroll.visible_range(count);
        let mut lines = vec![None; count];
        for (offset, row) in self.view.rows.live.iter().enumerate() {
            let index = self.view.rows.live_start + offset;
            if index < count && range.contains(&index) {
                lines[index] = Some(
                    LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone())
                        .with_search_fields([0]),
                );
            }
        }
        targets.push(LineTarget::new(lines));
        surfaces.push(LineSurface::Library);
        (targets, surfaces)
    }

    fn line_search_current(&self, surface: LineSurface, row: usize) -> bool {
        self.line_search.current_hit().is_some_and(|hit| {
            self.line_search_surfaces.get(hit.target) == Some(&surface) && hit.row == row
        })
    }

    fn flash_line_search_denied(&mut self) {
        if self.line_search.query().is_empty() {
            return;
        }
        self.line_search_denied_at = Some(self.clock.now);
        self.line_search_denied_query = self.line_search.query().to_string();
    }

    fn refresh_line_search(&mut self) {
        let query = self.line_search.query().to_string();
        self.line_search.refresh(&self.line_search_snapshot);
        if let Some(hit) = self.line_search.current_hit() {
            self.apply_line_hit(hit);
        }
        if query.is_empty() {
            self.line_search_denied_query.clear();
        } else if self.line_search.candidates.is_empty()
            && self.line_search.last_hit.is_none()
            && query != self.line_search_denied_query
        {
            self.flash_line_search_denied();
        }
    }

    fn close_line_search(&mut self) {
        self.line_search.close();
        self.line_search_snapshot.clear();
        self.line_search_surfaces.clear();
        self.line_search_denied_at = None;
        self.line_search_denied_query.clear();
        self.line_search_anim
            .target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
    }

    fn on_line_search(&mut self, _: &LineSearch, _window: &mut Window, cx: &mut Context<Self>) {
        self.clear_line_search_selection();
        self.line_search.open();
        let (targets, surfaces) = self.line_search_targets();
        self.line_search_snapshot = targets;
        self.line_search_surfaces = surfaces;
        self.line_search.set_query("", &self.line_search_snapshot);
        self.line_search_denied_at = None;
        self.line_search_denied_query.clear();
        self.line_search_anim
            .target(1.0, anim::STANDARD_MS, anim::Curve::Decelerate, &self.clock);
        cx.notify();
    }

    fn open_command_line(&mut self) {
        self.command_line.open();
        self.clock.tick();
        self.command_line_anim
            .target(1.0, anim::STANDARD_MS, anim::Curve::Decelerate, &self.clock);
    }

    fn close_command_line(&mut self) {
        self.command_line.close();
        self.command_line_anim
            .target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
    }

    fn command_line_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        let modified =
            control || event.keystroke.modifiers.alt || event.keystroke.modifiers.platform;
        match key {
            "escape" => {
                self.close_command_line();
                cx.notify();
                return;
            }
            "enter" => {
                let text = self.command_line.query().to_string();
                if text.trim().is_empty() {
                    self.close_command_line();
                    cx.notify();
                    return;
                }
                match ipc_command::parse(&text) {
                    Ok(command) => {
                        self.close_command_line();
                        self.pending_ipc.push(command);
                    }
                    Err(error) => {
                        let now = self.clock.now;
                        self.command_line.deny(now, error.to_string());
                    }
                }
                cx.notify();
                return;
            }
            "tab" => {
                self.command_line.complete(false);
                self.command_line.clear_denied();
                cx.notify();
                return;
            }
            "backtab" => {
                self.command_line.complete(true);
                self.command_line.clear_denied();
                cx.notify();
                return;
            }
            "left" => self.command_line.move_left(shift),
            "right" => self.command_line.move_right(shift),
            "home" => self.command_line.home(shift),
            "end" => self.command_line.end(shift),
            "backspace" => self.command_line.backspace(),
            "delete" => self.command_line.delete_forward(),
            "a" if control => self.command_line.select_all(),
            "w" if control => self.command_line.delete_word_back(),
            "c" if control => {
                if let Some(text) = self.command_line.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                if let Some(text) = self.command_line.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.command_line.editor.replace_selection("");
                    self.command_line.reset_completion();
                    self.command_line.refresh();
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.command_line.insert(&text);
                }
            }
            _ if !modified => {
                if let Some(text) = event.keystroke.key_char.as_ref()
                    && !text.chars().any(char::is_control)
                {
                    self.command_line.insert(text);
                } else if key == "space" {
                    self.command_line.insert(" ");
                } else {
                    return;
                }
            }
            _ => return,
        }
        self.command_line.clear_denied();
        cx.notify();
    }

    fn command_line_field(&self, bar_h: f32) -> gpui::AnyElement {
        let editor = &self.command_line.editor;
        let text_size = px(bar_h * 0.36);
        if editor.text.is_empty() {
            return div()
                .flex_1()
                .min_w_0()
                .text_size(text_size)
                .text_color(rgb(palette::on_surface_variant()))
                .child("Type a command…")
                .into_any_element();
        }
        if let Some(range) = editor.selection() {
            let highlight = HighlightStyle {
                color: Some(rgb(palette::on_primary_container()).into()),
                background_color: Some(rgb(palette::primary_container()).into()),
                ..Default::default()
            };
            return StyledText::new(editor.text.clone())
                .with_highlights([(range, highlight)])
                .into_any_element();
        }
        let cursor = editor.cursor.min(editor.text.len());
        let (before, after) = editor.text.split_at(cursor);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .text_size(text_size)
            .text_color(rgb(palette::on_surface()))
            .child(before.to_string())
            .child(
                div()
                    .w(px(2.0))
                    .h(px(bar_h * 0.46))
                    .bg(rgb(palette::primary())),
            )
            .child(after.to_string())
            .into_any_element()
    }

    fn command_line_overlay(&self, progress: f32) -> gpui::Div {
        let bar_h = (self.win_h * 0.07).max(32.0);
        let row_h = (bar_h * 0.44).max(18.0);
        let suggestions: Vec<_> = self
            .command_line
            .suggestions
            .iter()
            .take(6)
            .enumerate()
            .map(|(index, suggestion)| {
                let selected = index == self.command_line.selected;
                div()
                    .h(px(row_h))
                    .px(px(self.win_w * 0.04))
                    .flex()
                    .items_center()
                    .gap(px(bar_h * 0.12))
                    .bg(if selected {
                        rgb(palette::primary_container())
                    } else {
                        rgb(palette::surface_container())
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(row_h * 0.44))
                            .text_color(if selected {
                                rgb(palette::on_primary_container())
                            } else {
                                rgb(palette::on_surface())
                            })
                            .child(suggestion.label.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(self.win_w * 0.48))
                            .text_size(px(row_h * 0.36))
                            .text_color(if selected {
                                rgb(palette::on_primary_container())
                            } else {
                                rgb(palette::on_surface_variant())
                            })
                            .child(suggestion.description.clone()),
                    )
            })
            .collect();
        let list_h = row_h * suggestions.len() as f32;
        let denied = self.command_line.denied_opacity(self.clock.now);
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
                    .bottom(px(bar_h))
                    .h(px(list_h))
                    .bg(rgb(palette::surface_container()))
                    .opacity(if progress > 0.0 { progress } else { 0.0 })
                    .flex()
                    .flex_col()
                    .children(suggestions),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-(1.0 - progress) * bar_h))
                    .h(px(bar_h))
                    .rounded_md()
                    .bg(rgb(palette::surface()))
                    .px(px(self.win_w * 0.04))
                    .flex()
                    .items_center()
                    .gap(px(bar_h * 0.08))
                    .child(
                        div()
                            .text_size(px(bar_h * 0.42))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette::primary()))
                            .child(":"),
                    )
                    .child(self.command_line_field(bar_h))
                    .child(
                        div()
                            .text_size(px(bar_h * 0.28))
                            .text_color(if self.command_line.denied_text.is_empty() {
                                rgb(palette::on_surface_variant())
                            } else {
                                rgb(palette::error())
                            })
                            .child(if self.command_line.denied_text.is_empty() {
                                format!("[{}]", self.command_line.mode_label())
                            } else {
                                self.command_line.denied_text.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(bar_h * 0.24))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child("Tab complete"),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(2.0))
                            .rounded_full()
                            .bg(rgb(palette::error()))
                            .opacity(denied),
                    ),
            )
    }

    fn on_key_down(&mut self, event: &gpui::KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.login.is_some() {
            cx.stop_propagation();
            self.login_key(event, cx);
            return;
        }
        if self.command_line.is_open() {
            cx.stop_propagation();
            self.command_line_key(event, _window, cx);
            return;
        }
        if self.line_search.open {
            cx.stop_propagation();
            self.line_search_key(event, _window, cx);
            return;
        }
        if self.fullscreen
            && !self.lyrics_overlay
            && !self.alt_held
            && !self.alt.armed
            && !(self.search.is_some() && self.search_open)
        {
            let key = event.keystroke.key.as_str();
            if key == "tab" || key == "backtab" {
                cx.stop_propagation();
                self.cycle_fullscreen_focus(key == "backtab");
                cx.notify();
                return;
            }
        }
        if self.alt_held || self.alt.armed {
            cx.stop_propagation();
            self.on_alt_key(event, cx);
            return;
        }
        if self.search.is_none() || !self.search_open {
            let key = event.keystroke.key.as_str();
            if key == ":" || key == "colon" || (key == ";" && event.keystroke.modifiers.shift) {
                cx.stop_propagation();
                self.open_command_line();
                cx.notify();
                return;
            }
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
        let shift = event.keystroke.modifiers.shift;
        let modified = control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform;
        if !search.typing {
            match key {
                "escape" => {
                    let had_selection = self
                        .search
                        .as_ref()
                        .is_some_and(|search| search.selection.active());
                    if had_selection {
                        if let Some(search) = self.search.as_mut() {
                            search.selection.clear();
                        }
                    } else {
                        self.search_open = false;
                        self.search_denied_at = None;
                    }
                    cx.notify();
                }
                "i" => {
                    if let Some(search) = self.search.as_mut() {
                        search.selection.clear();
                        search.typing = true;
                    }
                    cx.notify();
                }
                "enter" => {
                    if let Some(search) = self.search.as_mut() {
                        search.selection.clear();
                    }
                    self.activate_search_result(cx);
                }
                "tab" | "l" | "right" => {
                    self.on_search_scope_next(&SearchScopeNext, _window, cx);
                }
                "backtab" | "h" | "left" => {
                    self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                }
                "v" | "V" => {
                    if let Some(search) = self.search.as_mut() {
                        let cursor = search.cursor();
                        if key == "V" || shift {
                            search.selection.select_all();
                        } else if search.selection.active() {
                            search.selection.clear();
                        } else {
                            search.selection.start(cursor);
                        }
                    }
                    cx.notify();
                }
                "j" | "down" | "ctrl-n" => {
                    if let Some(search) = self.search.as_mut() {
                        if shift {
                            let cursor = search.cursor();
                            search.selection.start(cursor);
                        }
                        search.move_cursor(1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "k" | "up" | "ctrl-p" => {
                    if let Some(search) = self.search.as_mut() {
                        if shift {
                            let cursor = search.cursor();
                            search.selection.start(cursor);
                        }
                        search.move_cursor(-1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "n" if control => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "p" if control => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(-1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "pagedown" | "ctrl-d" => {
                    if let Some(search) = self.search.as_mut() {
                        for _ in 0..state::SEARCH_VISIBLE_ROWS {
                            search.move_cursor(1);
                        }
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "pageup" | "ctrl-u" => {
                    if let Some(search) = self.search.as_mut() {
                        for _ in 0..state::SEARCH_VISIBLE_ROWS {
                            search.move_cursor(-1);
                        }
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "g" => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] = 0;
                    }
                    self.follow_search(false);
                    cx.notify();
                }
                "G" => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] =
                            search.scope_count(search.scope).saturating_sub(1);
                    }
                    self.follow_search(false);
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
                self.follow_search(true);
                cx.notify();
                return;
            }
            "down" | "ctrl-n" => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "p" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(-1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "n" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "u" if control => {
                if let Some(search) = self.search.as_mut() {
                    for _ in 0..state::SEARCH_VISIBLE_ROWS {
                        search.move_cursor(-1);
                    }
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "d" if control => {
                if let Some(search) = self.search.as_mut() {
                    for _ in 0..state::SEARCH_VISIBLE_ROWS {
                        search.move_cursor(1);
                    }
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "left" => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.move_left(shift);
                }
            }
            "right" => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.move_right(shift);
                }
            }
            "home" => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.home(shift);
                }
            }
            "end" => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.end(shift);
                }
            }
            "backspace" => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.backspace();
                }
            }
            "delete" => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.delete_forward();
                }
            }
            "a" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.select_all();
                }
            }
            "w" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.editor.delete_word_back();
                }
            }
            "c" if control => {
                if let Some(search) = self.search.as_ref()
                    && let Some(text) = search.editor.selected_text()
                {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                let cut = self.search.as_ref().and_then(|search| {
                    search.editor.selected_text().map(str::to_string)
                });
                if let Some(text) = cut {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    if let Some(search) = self.search.as_mut() {
                        search.editor.replace_selection("");
                    }
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                    && let Some(search) = self.search.as_mut()
                {
                    search.editor.insert(&text);
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
                    search.editor.insert(text);
                } else if key == "space" {
                    search.editor.insert(" ");
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
        if self.line_search.open {
            self.line_search_commit(_window, cx);
            return;
        }
        if self.alt.armed && self.alt.sticky {
            let menu = self.alt_menu.clone();
            if let Some(menu) = menu
                && let Some(verb) = self.alt.confirm_plan(&menu)
            {
                self.dispatch_quick(verb);
            }
            self.alt.cancel();
            self.alt_menu = None;
            self.alt_target = None;
                self.animate_alt(0.0);
            cx.notify();
            return;
        }
        if let Some(slot) = self.status_focus {
            self.activate_status(slot);
            cx.notify();
            return;
        }
        if self.search.is_some() {
            self.activate_search_result(cx);
            return;
        }
        if self.queue_open {
            let index = self.queue_cursor;
            self.player.jump(index);
            self.queue_open = false;
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
            page_profile::PageAction::PlayAlbum => {
                if let Some(page) = self.page.as_ref() {
                    self.bridge.play_album(
                        page.descriptor.id.clone(),
                        page.cursor,
                    );
                }
            }
            page_profile::PageAction::PlayArtist => {
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

    fn handle_command(&mut self, command: &str, cx: &mut Context<Self>) {
        match ipc_command::parse(command) {
            Ok(command) => self.pending_ipc.push(command),
            Err(error) => crate::debug_log!("ipc: ignored {command:?}: {error}"),
        }
        cx.notify();
    }

    /// Execute one shared IPC/`:` command. Runs with a live `Window`, so it can
    /// reuse the existing action handlers instead of duplicating them.
    fn execute_ipc_command(
        &mut self,
        command: ipc_command::IpcCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use ipc_command::{IpcCommand, QuickDirection, WipeScope};
        match command {
            IpcCommand::Sync(mode) => self.start_sync(mode),
            IpcCommand::Wipe(scope) => {
                if self.syncing {
                    self.push_toast(
                        "Wait for the sync to finish before wiping.".to_string(),
                        ToastKind::Notice,
                        None,
                    );
                } else {
                    let scope = match scope {
                        WipeScope::Album => "album",
                        WipeScope::Lyrics => "lyrics",
                        WipeScope::Covers => "covers",
                    };
                    self.bridge.wipe(scope.to_string());
                }
            }
            IpcCommand::WipeDatabases => self.open_wipe_menu(),
            IpcCommand::Refresh => self.on_refresh(&Refresh, window, cx),
            IpcCommand::Top => self.on_top(&Top, window, cx),
            IpcCommand::Bottom => self.on_bottom(&Bottom, window, cx),
            IpcCommand::MoveUp => self.on_move_up(&MoveUp, window, cx),
            IpcCommand::MoveDown => self.on_move_down(&MoveDown, window, cx),
            IpcCommand::ScrollUp | IpcCommand::ScrollDown => {
                let down = matches!(command, IpcCommand::ScrollDown);
                let modal = if self.queue_open {
                    &mut self.queue_scroll_modal
                } else if self.page.is_some() {
                    &mut self.page_scroll_modal
                } else {
                    &mut self.library_scroll
                };
                let current = -f32::from(modal.handle.0.borrow().base_handle.offset().y);
                let next = current + if down { modal.row_h } else { -modal.row_h };
                modal.on_wheel();
                modal.set_offset_now(next.max(0.0));
            }
            IpcCommand::PageUp => self.on_page_up(&PageUp, window, cx),
            IpcCommand::PageDown => self.on_page_down(&PageDown, window, cx),
            IpcCommand::MoveLeft | IpcCommand::MoveRight => {
                let left = matches!(command, IpcCommand::MoveLeft);
                if self.search.is_some() && self.search_open {
                    if left {
                        self.on_search_scope_prev(&SearchScopePrev, window, cx);
                    } else {
                        self.on_search_scope_next(&SearchScopeNext, window, cx);
                    }
                } else if self.fullscreen {
                    self.cycle_fullscreen_focus(left);
                }
            }
            IpcCommand::PrevPlace => self.step_travel(-1),
            IpcCommand::NextPlace => self.step_travel(1),
            IpcCommand::Home => self.on_home(&Home, window, cx),
            IpcCommand::Back => self.on_back(&Back, window, cx),
            IpcCommand::Enter => self.on_enter(&Enter, window, cx),
            IpcCommand::ScopeNext => self.on_scope_next(&ScopeNext, window, cx),
            IpcCommand::ScopePrev => self.on_scope_prev(&ScopePrev, window, cx),
            IpcCommand::OrderNext => self.on_order_next(&OrderNext, window, cx),
            IpcCommand::OrderPrev => self.on_order_prev(&OrderPrev, window, cx),
            IpcCommand::Visual => self.on_visual(&Visual, window, cx),
            IpcCommand::VisualAll => self.on_visual_all(&VisualAll, window, cx),
            IpcCommand::QueueToggle => self.on_queue_toggle(&QueueToggle, window, cx),
            IpcCommand::QueueRemove => self.on_queue_remove(&QueueRemove, window, cx),
            IpcCommand::QueueUp => self.on_queue_up(&QueueUp, window, cx),
            IpcCommand::QueueDown => self.on_queue_down(&QueueDown, window, cx),
            IpcCommand::RepeatCycle => self.on_repeat_cycle(&RepeatCycle, window, cx),
            IpcCommand::ToggleFullscreen => {
                self.on_toggle_fullscreen(&ToggleFullscreen, window, cx)
            }
            IpcCommand::LyricsOverlay => self.on_lyrics_overlay(&LyricsOverlay, window, cx),
            IpcCommand::PaneCycle { backward } => self.on_tab(backward, cx),
            IpcCommand::Search { query } => {
                self.open_search(true);
                if !query.is_empty() {
                    if let Some(search) = self.search.as_mut() {
                        search.editor.set_text(&query);
                    }
                    self.search_edited();
                }
            }
            IpcCommand::InsertMode { query } => {
                self.open_search(true);
                if !query.is_empty() {
                    if let Some(search) = self.search.as_mut() {
                        search.editor.set_text(&query);
                        search.typing = true;
                    }
                    self.search_edited();
                }
            }
            IpcCommand::SearchNext => self.on_search_next(&SearchNext, window, cx),
            IpcCommand::SearchPrev => self.on_search_prev(&SearchPrev, window, cx),
            IpcCommand::SearchScopeNext => self.on_search_scope_next(&SearchScopeNext, window, cx),
            IpcCommand::SearchScopePrev => self.on_search_scope_prev(&SearchScopePrev, window, cx),
            IpcCommand::LineSearch => self.on_line_search(&LineSearch, window, cx),
            IpcCommand::PlayPause => self.on_play_pause(&PlayPause, window, cx),
            IpcCommand::Play => self.player.resume(),
            IpcCommand::Pause => self.player.pause(),
            IpcCommand::Stop => self.player.stop(),
            IpcCommand::NextTrack => self.on_next_track(&NextTrack, window, cx),
            IpcCommand::PrevTrack => self.on_prev_track(&PrevTrack, window, cx),
            IpcCommand::Mute => self.on_mute(&Mute, window, cx),
            IpcCommand::VolumeUp => self.on_volume_up(&VolumeUp, window, cx),
            IpcCommand::VolumeDown => self.on_volume_down(&VolumeDown, window, cx),
            IpcCommand::SetVolume(volume) => self.player.set_volume(volume),
            IpcCommand::SeekBack => self.on_seek_back(&SeekBack, window, cx),
            IpcCommand::SeekForward => self.on_seek_forward(&SeekForward, window, cx),
            IpcCommand::Seek(position) => self.player.seek(position),
            IpcCommand::SeekRelative(offset) => {
                self.player.seek((self.player.position() + offset).max(0.0))
            }
            IpcCommand::PlaySong(id) => {
                self.last_play = Some((id.clone(), self.view.order_label().to_string()));
                self.bridge
                    .play_song(id, self.view.order_label().to_string());
            }
            IpcCommand::PlaySongInAlbum(id) => self.bridge.play_song_in_album(id),
            IpcCommand::OpenSongAlbum(id) => self.bridge.quick_go_album(id),
            IpcCommand::DownloadSong(id) => {
                self.bridge.quick_download("song".to_string(), id, true)
            }
            IpcCommand::QuickHold(true) => {
                self.alt_held = true;
                self.alt_arm();
            }
            IpcCommand::QuickHold(false) => {
                self.alt_held = false;
                self.alt_release();
            }
            IpcCommand::QuickAim(direction) => {
                if let Some(menu) = self.alt_menu.clone() {
                    let direction = match direction {
                        QuickDirection::North => Direction::North,
                        QuickDirection::South => Direction::South,
                        QuickDirection::East => Direction::East,
                        QuickDirection::West => Direction::West,
                    };
                    self.alt.aim(direction, &menu, false);
                }
            }
            IpcCommand::QuickCycle { next } => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(if next { 1 } else { -1 }, &menu);
                }
            }
            IpcCommand::QuickCancel => {
                self.alt.dismiss();
                self.alt_menu = None;
                self.alt_target = None;
                self.animate_alt(0.0);
            }
            IpcCommand::Quit => self.on_quit(&Quit, window, cx),
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
            self.close_line_search();
            cx.notify();
            return;
        }
        if self.alt.armed || self.alt_menu.is_some() {
            self.alt.cancel();
            self.alt_menu = None;
            self.alt_target = None;
                self.animate_alt(0.0);
            cx.notify();
            return;
        }
        if self.status_focus.is_some() {
            self.status_focus = None;
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
        self.status_focus = None;
        self.pending_page_reveal = None;
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

    fn open_descriptor_reveal(
        &mut self,
        descriptor: PageDescriptor,
        reveal: Option<PendingReveal>,
    ) {
        self.open_descriptor(descriptor);
        self.pending_page_reveal = reveal;
    }

    fn back_page(&mut self) {
        self.status_focus = None;
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
        self.status_focus = None;
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

    fn topbar_action_chip(
        &self,
        action: TopBarAction,
        page: Option<&PageBarState>,
        anchors: StatusAnchorSlots,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let slot = action.slot();
        let focused = self.status_focus == Some(slot);
        let (icon_el, opacity) = match action {
            TopBarAction::Sync => (
                icon_rotated(
                    "reload-ui-2-svgrepo-com.svg",
                    18.0,
                    if self.syncing {
                        palette::on_surface_variant()
                    } else {
                        palette::on_surface()
                    },
                    self.sync_angle(),
                ),
                if self.syncing { 0.38 } else { 0.85 },
            ),
            TopBarAction::Wipe => (
                icon("delete-outline.svg", 18.0, palette::on_surface()),
                1.0,
            ),
            TopBarAction::Download => {
                let cached = page.is_some_and(|page| page.cached);
                let downloading = page.is_some_and(|page| page.downloading);
                (
                    icon(
                        if cached {
                            "baseline-download.svg"
                        } else {
                            "outline-download.svg"
                        },
                        18.0,
                        palette::on_surface_variant(),
                    ),
                    if downloading { 0.38 } else { 1.0 },
                )
            }
            TopBarAction::Favorite => {
                let starred = page.is_some_and(|page| page.starred);
                (
                    icon(
                        if starred {
                            "baseline-favorite.svg"
                        } else {
                            "heart-add-fill.svg"
                        },
                        18.0,
                        if starred {
                            palette::primary()
                        } else {
                            palette::on_surface_variant()
                        },
                    ),
                    1.0,
                )
            }
            TopBarAction::Tool => (
                icon("dots-fill.svg", 18.0, palette::on_surface_variant()),
                1.0,
            ),
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(if focused {
                palette::primary()
            } else {
                palette::surface_container_low()
            }))
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(first) = bounds.first() {
                    let x = f32::from(first.origin.x) + f32::from(first.size.width) * 0.5;
                    let y = f32::from(first.origin.y) + f32::from(first.size.height) * 0.5;
                    anchors.borrow_mut()[slot] = Some((x, y));
                }
            })
            .child(
                div()
                    .id(gpui::ElementId::named_usize("topbar-action", slot))
                    .p(px(3.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_status(slot);
                        cx.notify();
                    }))
                    .opacity(opacity)
                    .child(icon_el),
            )
            .into_any_element()
    }

    fn bar_separator(&self) -> gpui::AnyElement {
        div()
            .text_size(px(14.0))
            .text_color(rgb(palette::on_surface_variant()))
            .child("·")
            .into_any_element()
    }

    fn star_glyphs(rating: u8) -> [bool; 5] {
        std::array::from_fn(|index| index < usize::from(rating.min(5)))
    }

    fn star_counter(&self, rating: u8) -> gpui::AnyElement {
        let mut row = div().flex_none().flex().items_center().gap_1();
        for filled in Self::star_glyphs(rating) {
            row = row.child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(if filled {
                        palette::primary()
                    } else {
                        palette::on_surface_variant()
                    }))
                    .child(if filled { "★" } else { "☆" }),
            );
        }
        row.into_any_element()
    }

    fn home_bar(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let (welcome, offline) = match self.header.as_ref() {
            Some(header) => (format!("Welcome, {}", header.username), false),
            None => ("Welcome".to_string(), false),
        };
        let report = match self.header.as_ref() {
            Some(header) => {
                let sync = if self.syncing {
                    "Syncing…".to_string()
                } else {
                    header
                        .last_sync
                        .as_deref()
                        .map(last_sync_text)
                        .unwrap_or_else(|| "—".to_string())
                };
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
            .flex()
            .items_center()
            .w_full()
            .h_full()
            .px_4()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(19.0))
                            .text_color(rgb(if offline {
                                palette::tertiary()
                            } else {
                                palette::on_surface_variant()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(welcome),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .justify_end()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(14.0))
                            .text_color(rgb(if self.syncing {
                                palette::tertiary()
                            } else {
                                palette::on_surface_variant()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(report),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(2.0))
                            .h(px((self.win_h * 0.0648 * 0.16).max(2.0)))
                            .rounded_full()
                            .bg(rgb(palette::outline_variant()))
                            .overflow_hidden()
                            .child(
                                div()
                                    .w(px(2.0 * self.sync_fill.value(&self.clock)))
                                    .h_full()
                                    .bg(rgb(if self.syncing {
                                        palette::tertiary()
                                    } else {
                                        palette::primary()
                                    })),
                            ),
                    )
                    .children(
                        self.topbar_actions()
                            .into_iter()
                            .map(|action| {
                                self.topbar_action_chip(
                                    action,
                                    None,
                                    self.status_anchors.clone(),
                                    cx,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
            )
            .into_any_element()
    }

    fn page_context_bar(&self, mode: SuperiorMode, cx: &Context<Self>) -> gpui::AnyElement {
        let Some(page) = self.page_bar() else {
            return div().into_any_element();
        };
        let title = if page.title.is_empty() {
            page.kind.label().to_string()
        } else {
            page.title.clone()
        };
        let mut identity = div()
            .flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .text_size(px(19.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette::on_surface()))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            );
        if !page.subtitle.is_empty() {
            identity = identity.child(self.bar_separator()).child(
                div()
                    .flex_none()
                    .text_size(px(14.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(page.subtitle.clone()),
            );
        }
        if matches!(mode, SuperiorMode::Album | SuperiorMode::Playlist | SuperiorMode::Radio)
            && !page.meta.is_empty()
        {
            identity = identity.child(self.bar_separator()).child(
                div()
                    .flex_none()
                    .text_size(px(14.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(page.meta.clone()),
            );
        }
        let mut row = div()
            .flex()
            .items_center()
            .w_full()
            .h_full()
            .px_4()
            .gap_2()
            .child(identity);
        match mode {
            SuperiorMode::Album | SuperiorMode::Artist => {
                if page.starred {
                    row = row.child(self.topbar_action_chip(
                        TopBarAction::Favorite,
                        Some(&page),
                        self.status_anchors.clone(),
                        cx,
                    ));
                }
                row = row.child(self.star_counter(page.rating));
                if page.cached || page.downloading {
                    row = row.child(self.topbar_action_chip(
                        TopBarAction::Download,
                        Some(&page),
                        self.status_anchors.clone(),
                        cx,
                    ));
                }
                if mode == SuperiorMode::Artist || page_tool_visible(&page) {
                    row = row.child(self.topbar_action_chip(
                        TopBarAction::Tool,
                        Some(&page),
                        self.status_anchors.clone(),
                        cx,
                    ));
                }
            }
            _ => {}
        }
        row.into_any_element()
    }

    fn top_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mode = self.superior_mode();
        let content = if mode.is_page() {
            self.page_context_bar(mode, cx)
        } else {
            self.home_bar(cx)
        };
        div()
            .relative()
            .w_full()
            .h(relative(0.0648))
            .bg(rgb(palette::surface_container_low()))
            .flex()
            .items_center()
            .child(content)
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
                        let selected = this.view.rows.get(index).is_some_and(|row| {
                            this.selected_id.as_deref() == Some(row.id.as_str())
                        }) || index == this.view.cursor
                            || this.view.selection.contains(this.view.cursor, index);
                        let hovered = this.hovered_row == Some(index);
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
                        let cover_scale = if selected { 1.20 } else { 1.0 };
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
                                .id(gpui::ElementId::named_usize(if selected { "library-row-selected" } else { "library-row" }, index))
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
                                .child(this.row_content(
                                    row,
                                    RowPaint {
                                        flags: LIBRARY_ROW_FLAGS,
                                        surface: LineSurface::Library,
                                        index,
                                        selected,
                                        height: row_h,
                                        cover_scale,
                                    },
                                )),
                        );
                    }
                    items
                }))
                .pt(px(8.0)).pb(px(8.0)).h_full()
                .w_full()
                .on_scroll_wheel(cx.listener(|this, _, _, _| {
                    this.library_scroll.on_wheel();
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
        let spawned = 0.72 + 0.28 * progress;
        let box_side = page_h * 0.07 * option_scale * spawned;
        let frame_side = (box_side * 8.0)
            .min(self.win_w - 20.0)
            .min(page_h - 20.0);
        // The board is anchored at the focused item but may slide inside the
        // safe window area: topbar/status anchors often sit too close to an
        // edge for a full-size board to stay centred there.
        let wall_gap = 10.0;
        let board_x = (ax - frame_side * 0.5)
            .clamp(wall_gap, (self.win_w - frame_side - wall_gap).max(wall_gap));
        let board_y = (ay - frame_side * 0.5)
            .clamp(wall_gap, (page_h - frame_side + wall_gap).max(wall_gap));
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

    fn line_search_denied_opacity(&self) -> f32 {
        let Some(started) = self.line_search_denied_at else {
            return 0.0;
        };
        let elapsed = ((self.clock.now - started) as f32).max(0.0);
        if elapsed < 0.03 {
            elapsed / 0.03
        } else {
            (1.0 - (elapsed - 0.03) / 0.12).clamp(0.0, 1.0)
        }
    }

    fn line_search_field(&self, bar_h: f32) -> gpui::AnyElement {
        let editor = &self.line_search.editor;
        let text_size = px(bar_h * 0.36);
        if editor.text.is_empty() {
            return div()
                .flex_1()
                .min_w_0()
                .text_size(text_size)
                .text_color(rgb(palette::on_surface_variant()))
                .child("Search this surface")
                .into_any_element();
        }
        if let Some(range) = editor.selection() {
            let highlight = HighlightStyle {
                color: Some(rgb(palette::on_primary_container()).into()),
                background_color: Some(rgb(palette::primary_container()).into()),
                ..Default::default()
            };
            return StyledText::new(editor.text.clone())
                .with_highlights([(range, highlight)])
                .into_any_element();
        }
        let cursor = editor.cursor.min(editor.text.len());
        let (before, after) = editor.text.split_at(cursor);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .text_size(text_size)
            .text_color(rgb(palette::on_surface()))
            .child(before.to_string())
            .child(
                div()
                    .w(px(2.0))
                    .h(px(bar_h * 0.46))
                    .bg(rgb(palette::primary())),
            )
            .child(after.to_string())
            .into_any_element()
    }

    fn line_search_overlay(&self, progress: f32) -> gpui::Div {
        let counter = match (self.line_search.current_hit(), self.line_search.candidates.len()) {
            (Some(_), count) if count > 0 => {
                format!("[{}/{}]", self.line_search.index.unwrap_or(0) + 1, count)
            }
            _ => String::new(),
        };
        let bar_h = (self.win_h * 0.07).max(32.0);
        let denied = self.line_search_denied_opacity();
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
                    .bottom(px(-(1.0 - progress) * bar_h))
                    .h(px(bar_h))
                    .rounded_md()
                    .bg(rgb(palette::surface()))
                    .px(px(self.win_w * 0.04))
                    .flex()
                    .items_center()
                    .gap(px(bar_h * 0.012))
                    .child(
                        div()
                            .text_size(px(bar_h * 0.42))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette::primary()))
                            .child("/"),
                    )
                    .child(self.line_search_field(bar_h))
                    .child(
                        div()
                            .text_size(px(bar_h * 0.30))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child(counter),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(2.0))
                            .rounded_full()
                            .bg(rgb(palette::error()))
                            .opacity(denied),
                    ),
            )
    }

    fn lyric_line_elements(
        &self,
        layout: &lyrics_layout::LyricsLayout,
        lines: &[(Option<i64>, String)],
        synced: bool,
    ) -> Vec<gpui::AnyElement> {
        layout
            .rows
            .iter()
            .map(|row| {
                let is_active = synced && row.active;
                let emphasis = self
                    .lyrics_line_emphasis
                    .get(&row.index)
                    .map(|anim| anim.value(&self.clock))
                    .unwrap_or(1.0);
                let color = self
                    .lyrics_line_color
                    .get(&row.index)
                    .map(|anim| anim.value(&self.clock))
                    .unwrap_or_else(|| {
                        if is_active {
                            palette::on_surface()
                        } else {
                            palette::on_surface_variant()
                        }
                    });
                let opacity = self
                    .lyrics_line_opacity
                    .get(&row.index)
                    .map(|anim| anim.value(&self.clock))
                    .unwrap_or(1.0);
                let text = lines
                    .get(row.index)
                    .map(|(_, text)| text.clone())
                    .unwrap_or_default();
                div()
                    .w(px(layout.width))
                    .min_w_0()
                    .h(px(row.height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(layout.base_size * emphasis))
                    .line_height(px(layout.text_height * emphasis))
                    .text_color(rgb(color))
                    .text_center()
                    .whitespace_normal()
                    .opacity(opacity)
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .whitespace_normal()
                            .child(text),
                    )
                    .into_any_element()
            })
            .collect()
    }


    fn lyrics_overlay_element(&self, progress: f32) -> gpui::AnyElement {
        if !self.lyrics_overlay && progress <= 0.001 {
            return div().into_any_element();
        }
        let Some(lyrics) = self.lyrics.as_ref() else {
            return div().into_any_element();
        };
        let Some(layout) = self.lyrics_layout.as_ref() else {
            return div().into_any_element();
        };
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.35 * progress))
            .opacity(progress.clamp(0.0, 1.0))
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
                    .gap(px(layout.gap))
                    .opacity(self.lyrics_track_fade.value(&self.clock))
                    .mt(px(self.lyrics_shift.value(&self.clock)))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child(format!("Lyrics · {} · ? closes", lyrics.source)),
                    )
                    .children(self.lyric_line_elements(layout, &lyrics.lines, lyrics.synced)),
            )
            .into_any_element()
    }


    fn fullscreen_overlay(&self, progress: f32) -> gpui::Div {
        let track = self.player.current().cloned();
        let (title, artist, cover) = match &track {
            Some(track) => (
                track.title.clone(),
                track.artist.clone(),
                self.display_cover(&track.cover_art, self.fullscreen_hero_bucket())
                    .or_else(|| {
                        self.hero_cover(&track.cover_url, self.fullscreen_hero_bucket())
                    }),
            ),
            None => ("No track playing".to_string(), String::new(), None),
        };
        let title_size = (metrics::font_size("headline", self.win_h)
            * metrics::width_scale(self.win_w))
        .clamp(18.0, 32.0);
        let favorite = track.as_ref().is_some_and(|track| track.favorite);
        let downloaded = track
            .as_ref()
            .is_some_and(|track| self.marks.downloaded(&track.id, &track.album_id));
        let cover_focus = self.fullscreen_focus == FullscreenFocus::Cover;
        let title_focus = self.fullscreen_focus == FullscreenFocus::Title;
        let lyrics_focus = self.fullscreen_focus == FullscreenFocus::Lyrics;
        let cover_anchor_slot = self.fullscreen_anchors.clone();
        let title_anchor_slot = self.fullscreen_anchors.clone();
        let position = self.player.position();
        let duration = self.player.duration();
        let progress_ratio = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let lyrics = self.lyrics.as_ref();
        let lines = lyrics.map(|lyrics| lyrics.lines.clone()).unwrap_or_default();
        let lyrics_label = match lyrics {
            Some(lyrics) if lyrics.lines.is_empty() => "No lyrics".to_string(),
            Some(lyrics) => format!("Lyrics · {}", lyrics.source),
            None => "Loading lyrics…".to_string(),
        };
        let lyrics_inline = self.lyrics_inline();
        let lyrics_synced = lyrics.is_some_and(|lyrics| lyrics.synced);
        let lyric_gap = self
            .lyrics_layout
            .as_ref()
            .map(|layout| layout.gap)
            .unwrap_or(12.0);
        let lyric_children = self
            .lyrics_layout
            .as_ref()
            .map(|layout| self.lyric_line_elements(layout, &lines, lyrics_synced))
            .unwrap_or_default();
        // The overlay is a centered lyrics panel over the full player: keep the
        // cover as the backdrop instead of hiding it and splitting the pane.
        let split_lyrics = lyrics_inline && !self.lyrics_overlay;
        let cover_side = if split_lyrics {
            ((self.win_w * 0.95 - 24.0) * 0.5).min(self.win_h * 0.62)
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
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if cover_focus {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .on_children_prepainted(move |bounds, _, _| {
                                if let Some(bounds) = bounds.first() {
                                    let x = f32::from(bounds.origin.x)
                                        + f32::from(bounds.size.width) * 0.5;
                                    let y = f32::from(bounds.origin.y)
                                        + f32::from(bounds.size.height) * 0.5;
                                    cover_anchor_slot.borrow_mut().cover = Some((x, y));
                                }
                            })
                            .child(art(cover, cover_side)),
                    )
                    .child(
                        div()
                            .flex()
                            .when(split_lyrics, |this| this.flex_1())
                            .when(!split_lyrics, |this| this.hidden())
                            .h_full()
                            .opacity(if split_lyrics {
                                self.lyrics_track_fade.value(&self.clock)
                            } else {
                                0.0
                            })
                            .min_w_0()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if lyrics_focus {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(lyric_gap))
                            .mt(px(self.lyrics_shift.value(&self.clock)))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(lyrics_label),
                            )
                            .children(lyric_children),
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
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if title_focus {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .px_2()
                            .py_1()
                            .on_children_prepainted(move |bounds, _, _| {
                                if let Some(first) = bounds.first() {
                                    let mut left = f32::from(first.origin.x);
                                    let mut right = left + f32::from(first.size.width);
                                    let mut top = f32::from(first.origin.y);
                                    let mut bottom = top + f32::from(first.size.height);
                                    for bounds in bounds.iter().skip(1) {
                                        left = left.min(f32::from(bounds.origin.x));
                                        right = right.max(
                                            f32::from(bounds.origin.x)
                                                + f32::from(bounds.size.width),
                                        );
                                        top = top.min(f32::from(bounds.origin.y));
                                        bottom = bottom.max(
                                            f32::from(bounds.origin.y)
                                                + f32::from(bounds.size.height),
                                        );
                                    }
                                    title_anchor_slot
                                        .borrow_mut()
                                        .title = Some(((left + right) * 0.5, (top + bottom) * 0.5));
                                }
                            })
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(title_size))
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    .when(favorite, |this| {
                                        this.child(icon(
                                            "baseline-favorite.svg",
                                            (title_size * 0.62).clamp(13.0, 19.0),
                                            0xFFD54F,
                                        ))
                                    })
                                    .when(downloaded, |this| {
                                        this.child(icon(
                                            "outline-download.svg",
                                            (title_size * 0.58).clamp(12.0, 18.0),
                                            palette::primary(),
                                        ))
                                    }),
                            ),
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
                            .child("Tab cover/title/lyrics · Alt actions · Shift-Enter/q closes · Space play/pause · Ctrl-Q queue"),
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
                                let selected = this.selected_id.as_deref()
                                    == Some(track.id.as_str())
                                    || active
                                    || this.queue_selection.contains(cursor, index)
                                    || this.queue_selection.contains(this.queue_cursor, index);
                                let playing = index == this.player.index();
                                let emphasis = if selected { 1.08 } else { 1.0 };
                                let cover_scale = if selected { 1.20 } else { 1.0 };
                                items.push(
                                    div()
                                        .id(gpui::ElementId::named_usize(if selected { "queue-row-selected" } else { "queue-row" }, index))
                                        .flex()
                                        .w_full()
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
                                                                .child(highlighted_text(&title, this.line_search.query(), palette::on_surface(), this.line_search_current(LineSurface::Queue, index))),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(10.0 * emphasis))
                                                        .text_color(rgb(palette::on_surface_variant()))
                                                        .whitespace_normal()
                                                        .max_h(px(16.0))
                                                        .overflow_hidden()
                                                        .child(highlighted_text(&artist, this.line_search.query(), palette::on_surface_variant(), this.line_search_current(LineSurface::Queue, index))),
                                                ),
                                        ),
                                );
                            }
                            items
                        }))
                        .pt(px(8.0))
                        .pb(px(8.0))
                        .h_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.queue_scroll_modal.on_wheel();
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

    fn search_denied_opacity(&self) -> f32 {
        let Some(started) = self.search_denied_at else {
            return 0.0;
        };
        let elapsed = ((self.clock.now - started) as f32).max(0.0);
        if elapsed < 0.03 {
            elapsed / 0.03
        } else {
            (1.0 - (elapsed - 0.03) / 0.12).clamp(0.0, 1.0)
        }
    }

    fn search_loading_visible(&self, search: &SearchState) -> bool {
        search.loading
            && self
                .search_loading_since
                .is_some_and(|started| self.clock.now - started >= 0.06)
    }

    fn search_field(&self, search: &SearchState, height: f32, denied: f32) -> gpui::AnyElement {
        let editor = &search.editor;
        let text_size = px(height * 0.32);
        let field = div()
            .h(px(height))
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(rgb(if denied > 0.0 {
                palette::error()
            } else if search.typing {
                palette::primary()
            } else {
                palette::outline_variant()
            }))
            .bg(rgb(palette::surface_container_highest()))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .overflow_hidden()
            .child(icon(
                "search-right-1505-svgrepo-com.svg",
                height * 0.40,
                if search.typing {
                    palette::primary()
                } else {
                    palette::on_surface_variant()
                },
            ));
        if editor.text.is_empty() {
            return field
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(text_size)
                        .text_color(rgb(palette::on_surface_variant()))
                        .child("Search artists, albums, songs"),
                )
                .into_any_element();
        }
        if let Some(range) = editor.selection() {
            let highlight = HighlightStyle {
                color: Some(rgb(palette::on_primary_container()).into()),
                background_color: Some(rgb(palette::primary_container()).into()),
                ..Default::default()
            };
            return field
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(text_size)
                        .text_color(rgb(palette::on_surface()))
                        .child(
                            StyledText::new(editor.text.clone())
                                .with_highlights([(range, highlight)]),
                        ),
                )
                .into_any_element();
        }
        if !search.typing {
            return field
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(text_size)
                        .text_color(rgb(palette::on_surface()))
                        .child(editor.text.clone()),
                )
                .into_any_element();
        }
        let cursor = editor.cursor.min(editor.text.len());
        let (before, after) = editor.text.split_at(cursor);
        field
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .text_size(text_size)
                    .text_color(rgb(palette::on_surface()))
                    .child(before.to_string())
                    .child(
                        div()
                            .w(px(2.0))
                            .h(px(height * 0.42))
                            .bg(rgb(palette::primary())),
                    )
                    .child(after.to_string()),
            )
            .into_any_element()
    }

    fn search_result_row(
        &self,
        cx: &Context<Self>,
        row: &Row,
        paint: SearchRowPaint,
    ) -> gpui::AnyElement {
        let SearchRowPaint {
            scope,
            index,
            row_h,
            selected,
            hovered,
        } = paint;
        let cover = cover_path(&row.cover_url);
        div()
            .id(("search-row", index))
            .flex()
            .w_full()
            .items_center()
            .gap_3()
            .h(px(row_h))
            .px_3()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(rgb(palette::primary_container())))
            .when(hovered && !selected, |this| {
                this.bg(rgb(palette::surface_container_high()))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_row = Some(index);
                } else if this.hovered_row == Some(index) {
                    this.hovered_row = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(search) = this.search.as_mut() {
                    search.scope = scope;
                    search.scope_cursors[scope] = index;
                }
                this.activate_search_result(cx);
            }))
            .child(art_scaled(cover, row_h * 0.714, if selected || hovered { 1.08 } else { 1.0 }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(if selected {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(if selected {
                                palette::on_primary_container()
                            } else {
                                palette::on_surface()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(if selected {
                                palette::on_primary_container()
                            } else {
                                palette::on_surface_variant()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(row.subtitle.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(if selected {
                        palette::on_primary_container()
                    } else {
                        palette::on_surface_variant()
                    }))
                    .whitespace_nowrap()
                    .child(row.meta.clone()),
            )
            .into_any_element()
    }

    fn search_overlay(&self, cx: &Context<Self>, search: &SearchState, progress: f32) -> gpui::AnyElement {
        let row_h = 56.0;
        let field_h = 54.0;
        let panel_w = (self.win_w * 0.60).clamp(360.0, 680.0);
        let panel_h = field_h + 12.0 + 34.0 + 10.0 + 24.0 + row_h * state::SEARCH_VISIBLE_ROWS as f32 + 36.0 + 32.0;
        let denied = self.search_denied_opacity();
        let loading = self.search_loading_visible(search);
        let query_empty = search.editor.text.is_empty();
        let scoped = search.scoped();
        let nothing = !loading && !query_empty && scoped.is_empty();
        let selection_count = search.selection.count(search.cursor());
        let footer = if selection_count > 0 {
            format!(
                "{} selected · Shift-J/K extend · v clear · Alt actions · ↵ open",
                selection_count
            )
        } else if search.typing {
            "Type to filter · Ctrl-N/P rows · ⇥ scope · ↵ open · esc normal".to_string()
        } else {
            "j/k rows · Ctrl-N/P scroll · v visual · shift-J/K extend · i type · esc close"
                .to_string()
        };

        let result_area: gpui::AnyElement = if loading {
            loading_skeleton(self.clock.now as f32, row_h, state::SEARCH_VISIBLE_ROWS)
        } else if nothing {
            div()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(rgb(palette::on_surface_variant()))
                .child(if query_empty {
                    "Nothing played yet"
                } else {
                    "No Results"
                })
                .into_any_element()
        } else {
            let scroll = self.search_scroll.handle.clone();
            let scope = search.scope;
            uniform_list(
                "search-results",
                search.scope_count(scope),
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    let Some(search) = this.search.as_ref() else {
                        return items;
                    };
                    let rows = search.scoped();
                    let cursor = search.cursor();
                    for index in range {
                        let Some(row) = rows.get(index) else { break };
                        let selected = index == cursor
                            || search.selection.contains(cursor, index);
                        let hovered = this.hovered_row == Some(index);
                        items.push(this.search_result_row(
                            cx,
                            row,
                            SearchRowPaint {
                                scope,
                                index,
                                row_h,
                                selected,
                                hovered,
                            },
                        ));
                    }
                    items
                }),
            )
            .h(px(row_h * state::SEARCH_VISIBLE_ROWS as f32))
            .w_full()
            .track_scroll(scroll)
            .into_any_element()
        };

        let scopes = (0..4)
            .map(|scope| {
                let active = scope == search.scope;
                let count = search.scope_count(scope);
                div()
                    .id(("search-scope", scope))
                    .h(px(28.0))
                    .px_3()
                    .rounded_full()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .text_size(px(11.0))
                    .bg(rgb(if active {
                        palette::primary()
                    } else {
                        palette::surface_container_high()
                    }))
                    .text_color(rgb(if active {
                        palette::on_primary()
                    } else {
                        palette::on_surface_variant()
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(search) = this.search.as_mut() {
                            search.scope_cursors[search.scope] = search.cursor();
                            search.scope = scope;
                            search.scope_cursors[scope] =
                                search.scope_cursors[scope].min(search.scope_count(scope).saturating_sub(1));
                            search.selection.clear();
                        }
                        this.follow_search(false);
                        cx.notify();
                    }))
                    .child(format!("{} {count}", search_scope_label(scope)))
            })
            .collect::<Vec<_>>();

        div()
            .id("search-backdrop")
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .justify_center()
            .bg(gpui::black().opacity(0.50 * progress))
            .opacity(progress)
            .on_click(cx.listener(|this, _, _, cx| {
                this.search_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("search-panel")
                    .absolute()
                    .top(px(self.win_h * 0.14))
                    .w(px(panel_w))
                    .h(px(panel_h))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(palette::outline_variant()))
                    .bg(rgb(palette::surface_container()))
                    .shadow_lg()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(self.search_field(search, field_h, denied))
                    .child(
                        div()
                            .h(px(30.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .children(scopes),
                    )
                    .when(query_empty, |this| {
                        this.child(
                            div()
                                .h(px(22.0))
                                .px_1()
                                .flex()
                                .items_center()
                                .text_size(px(11.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette::on_surface_variant()))
                                .child("Most Played"),
                        )
                    })
                    .child(div().flex_1().min_h_0().overflow_hidden().child(result_area))
                    .child(
                        div()
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child(footer),
                    ),
            )
            .into_any_element()
    }

    /// One row body for every list surface. The data lives in `Row`; the
    /// surface supplies the flags that decide which cells are materialized.
    fn row_content(&self, row: &state::Row, paint: RowPaint) -> gpui::Div {
        let emphasis = if paint.selected { 1.08 } else { 1.0 };
        let current = self.line_search_current(paint.surface, paint.index);
        let mut body = div().flex().items_center().min_w_0().flex_1();
        if paint.flags.show_cover {
            let cover = cover_path(&row.cover_url);
            body = body.child(
                div()
                    .mr_3()
                    .child(art_scaled(cover, paint.height * 0.78, paint.cover_scale)),
            );
        }
        let mut cells = div().flex().flex_col().min_w_0().flex_1().overflow_hidden();
        let title_size = if paint.flags.compact { 15.0 } else { 18.0 } * emphasis;
        let title = div()
            .text_size(px(title_size))
            .font_weight(if paint.selected {
                gpui::FontWeight::BOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(rgb(palette::on_surface()))
            .overflow_hidden()
            .child(highlighted_text(
                &row.title,
                self.line_search.query(),
                palette::on_surface(),
                current,
            ));
        let title = if paint.flags.compact {
            title.whitespace_nowrap().text_ellipsis()
        } else {
            title.whitespace_normal().max_h(px(34.0))
        };
        cells = cells.child(title);
        if paint.flags.show_subtitle && !row.subtitle.is_empty() {
            let subtitle = div()
                .text_size(px(if paint.flags.compact { 11.0 } else { 12.0 } * emphasis))
                .text_color(rgb(palette::on_surface_variant()))
                .overflow_hidden()
                .child(highlighted_text(
                    &row.subtitle,
                    self.line_search.query(),
                    palette::on_surface_variant(),
                    current,
                ));
            let subtitle = if paint.flags.compact {
                subtitle.whitespace_nowrap().text_ellipsis()
            } else {
                subtitle.whitespace_normal().max_h(px(22.0))
            };
            cells = cells.child(subtitle);
        }
        body = body.child(cells);
        let downloaded = self
            .marks
            .downloaded(&row.song_id, &row.album_id)
            || row.downloaded;
        let mut trailing = div().flex().items_center().gap_2();
        if paint.flags.show_length && !row.meta.is_empty() {
            trailing = trailing.child(
                div()
                    .text_size(px(11.0 * emphasis))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(highlighted_text(
                        &row.meta,
                        self.line_search.query(),
                        palette::on_surface_variant(),
                        current,
                    )),
            );
        }
        if downloaded {
            trailing = trailing.child(icon("outline-download.svg", 12.0, palette::primary()));
        }
        if row.starred {
            trailing = trailing.child(icon("baseline-favorite.svg", 11.0, 0xFFD54F));
        }
        if row.rating > 0 {
            trailing = trailing.child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(palette::primary()))
                    .child(row.rating.to_string()),
            );
        }
        body.child(trailing).into_element()
    }


    fn page_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(page) = self.page.as_ref() else {
            return div().flex().flex_1();
        };
        let profile = page_profile::PageProfile::for_descriptor(&page.descriptor);
        let row_flags = profile.row_flags;
        let page_h = (self.win_h - 120.0).max(1.0);
        let page_w = self.win_w.max(1.0);
        let max_cover = (page_h - page_w * 0.04) * 0.75;
        let cover_side = page_w.min(page_h).min(max_cover).max(1.0);
        let cover_art_side = (cover_side - page_w * 0.04).max(1.0);
        let cover_bottom = cover_art_side + page_w * 0.02;
        let hero_bucket = self.page_hero_bucket();
        let cover = self
            .display_cover(&page.header.cover_art, hero_bucket)
            .or_else(|| self.hero_cover(&page.header.cover_url, hero_bucket));
        let header_h = cover_bottom.max(240.0);

        if page.loading && page.rows.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
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
            .child(div().w_full().h(px(header_h)))
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
                                let selected_id = this.selected_id.clone();
                                let Some((row, selected)) = this.page.as_ref().and_then(|page| {
                                    page.rows.get(index).map(|row| {
                                        let selected = selected_id.as_deref()
                                            == Some(row.id.as_str())
                                            || index == page.cursor
                                            || page.selection.contains(page.cursor, index);
                                        (row.clone(), selected)
                                    })
                                }) else {
                                    break;
                                };
                                let hovered = this.hovered_page == Some(index);
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
                                let cover_scale = if selected { 1.20 } else { 1.0 };
                                items.push(
                                    div()
                                        .id(gpui::ElementId::named_usize(if selected { "page-row-selected" } else { "page-row" }, index))
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
                                            this.status_focus = None;
                                            if let Some(page) = this.page.as_mut() {
                                                page.select(index);
                                            }
                                            this.follow_page(true);
                                            this.activate_page_cursor();
                                            cx.notify();
                                        }))
                                        .child(this.row_content(
                                            &row,
                                            RowPaint {
                                                flags: row_flags,
                                                surface: LineSurface::Page,
                                                index,
                                                selected,
                                                height: row_h,
                                                cover_scale,
                                            },
                                        )),
                                );
                            }
                            items
                        }))
                        .pt(px(8.0))
                        .pb(px(8.0))
                        .h_full()
                        .w_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.page_scroll_modal.on_wheel();
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
        let m = metrics::PlayerBarMetrics::new(self.win_w, self.win_h);
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
                            .overflow_hidden()
                            .child(art(cover, m.cover_side))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(m.title_size))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    .when(!artist.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .text_size(px(m.artist_size))
                                                .text_color(rgb(palette::on_surface_variant()))
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .child(artist),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .gap(px(m.transport_spacing))
                            .child(transport_button(
                                "skip-previous-solid-svgrepo-com.svg",
                                m.transport_unit,
                                palette::on_surface(),
                                self.transport_scale(0),
                            ))
                            .child(transport_button(
                                if state == crate::player::State::Playing {
                                    "pause-svgrepo-com.svg"
                                } else {
                                    "play-1001-svgrepo-com.svg"
                                },
                                m.play_size,
                                palette::on_surface(),
                                self.transport_scale(1),
                            ))
                            .child(transport_button(
                                "skip-next-circle-solid-svgrepo-com.svg",
                                m.transport_unit,
                                palette::on_surface(),
                                self.transport_scale(2),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .overflow_hidden()
                            .justify_end()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(m.time_size))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .text_right()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(format!(
                                        "{} / {}",
                                        format_time(position),
                                        format_time(duration)
                                    )),
                            )
                            .when(self.repeat != crate::player::Repeat::Off, |this| {
                                this.child(
                                    div()
                                        .text_size(px(m.time_size))
                                        .text_color(rgb(palette::primary()))
                                        .child(repeat_label(self.repeat)),
                                )
                            })
                            .child(icon(
                                if muted { "vol-mute-svgrepo-com.svg" }
                                else if volume <= 1.0 { "vol-zero-svgrepo-com.svg" }
                                else if volume <= 33.0 { "vol-low-svgrepo-com.svg" }
                                else { "vol-high-svgrepo-com.svg" },
                                m.volume_icon,
                                palette::on_surface_variant(),
                            ))
                            .child(
                                div()
                                    .w(px(m.volume_bar))
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
        self.scale_factor = window.scale_factor();
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
                self.land_queue(playback_index);
            }
        }
        self.refresh_lyrics_emphasis();
        self.sync_lyrics_layout(window);
        if self.exiting.is_none()
            && let Some(ms) = self.quit_after_ms
            && self.clock.now * 1000.0 >= ms as f64
        {
            self.exiting = Some(self.clock.now);
        }
        if self.login.is_some() {
            self.pending_ipc.clear();
            return self.login_view().into_any_element();
        }
        let pending = std::mem::take(&mut self.pending_ipc);
        for command in pending {
            self.execute_ipc_command(command, window, cx);
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
        if self.search.as_ref().is_some_and(|search| search.loading)
            || self.search_denied_opacity() > 0.0
        {
            window.request_animation_frame();
        }
        let line_progress = self.line_search_anim.target(
            if self.line_search.open { 1.0 } else { 0.0 },
            anim::STANDARD_MS,
            if self.line_search.open { anim::Curve::Decelerate } else { anim::Curve::Accelerate },
            &self.clock,
        );
        if !self.line_search_anim.settled(&self.clock) || self.line_search_denied_opacity() > 0.0 {
            window.request_animation_frame();
        }
        let line_overlay = (self.line_search.open || line_progress > 0.001)
            .then(|| self.line_search_overlay(line_progress));
        let command_progress = self.command_line_anim.target(
            self.command_line.animation_target(),
            anim::STANDARD_MS,
            self.command_line.anim_curve(),
            &self.clock,
        );
        if !self.command_line_anim.settled(&self.clock)
            || self.command_line.denied_opacity(self.clock.now) > 0.0
        {
            window.request_animation_frame();
        }
        let command_overlay = (self.command_line.is_open() || command_progress > 0.001)
            .then(|| self.command_line_overlay(command_progress));
        self.animate_alt(if self.alt.armed { 1.0 } else { 0.0 });
        let alt_progress = self.alt_spawn.value(&self.clock);
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
            self.ensure_marks();
            self.refresh_fullscreen_album_state();
        }
        if let Some(page) = self.page.as_ref() {
            let cover_art = page.header.cover_art.clone();
            let bucket = self.page_hero_bucket();
            let changed = self
                .cover_trace
                .as_ref()
                .map(|trace| (trace.cover_art.as_str(), trace.bucket))
                != Some((cover_art.as_str(), bucket));
            if changed {
                let started = std::time::Instant::now();
                crate::debug_log!(
                    "album entry: init album cover load cover={} bucket={} at={}",
                    cover_art,
                    bucket,
                    unix_ms()
                );
                self.cover_trace = Some(CoverLoadTrace {
                    cover_art: cover_art.clone(),
                    bucket,
                    started,
                    thumb_logged: false,
                });
            }
            if self.display_cover(&cover_art, bucket).is_none()
                && let Some(trace) = self.cover_trace.as_mut()
                && !trace.thumb_logged
            {
                trace.thumb_logged = true;
                crate::debug_log!(
                    "album entry: album cover thumb shown cover={} bucket={} elapsed_ms={:.1}",
                    cover_art,
                    bucket,
                    trace.started.elapsed().as_secs_f64() * 1000.0
                );
            }
            self.ensure_display_cover(&cover_art, bucket);
        }
        if let Some(track) = self.player.current().cloned() {
            self.ensure_display_cover(&track.cover_art, self.fullscreen_hero_bucket());
        }
        let lyrics_overlay_progress = self.lyrics_overlay_anim.target(
            if self.lyrics_overlay { 1.0 } else { 0.0 },
            anim::QUICK_MS,
            anim::Curve::Standard,
            &self.clock,
        );
        if !self.lyrics_overlay_anim.settled(&self.clock) {
            window.request_animation_frame();
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
        if self.search.as_ref().is_some_and(|search| !search.editor.text.is_empty()) {
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
            | self.queue_scroll_modal.apply(self.clock.now)
            | self.search_scroll.apply(self.clock.now);
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
        if !self.lyrics_shift.settled(&self.clock)
            || !self.lyrics_track_fade.settled(&self.clock)
            || self
                .lyrics_line_opacity
                .values()
                .any(|value| !value.settled(&self.clock))
            || self
                .lyrics_line_emphasis
                .values()
                .any(|value| !value.settled(&self.clock))
            || self
                .lyrics_line_color
                .values()
                .any(|color| !color.settled(&self.clock))
        {
            window.request_animation_frame();
        }
        if self.syncing
            || !self.sync_settle.settled(&self.clock)
            || !self.sync_fill.settled(&self.clock)
        {
            window.request_animation_frame();
        }
        let status = self.player_bar();
        div()
            .track_focus(&self.focus)
            .key_context(if self.command_line.is_open() {
                "command-line"
            } else if self.line_search.open {
                "line-search"
            } else if self.search.is_some() && self.search_open {
                "search"
            } else {
                "main"
            })
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
            .child(match fullscreen_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(self.lyrics_overlay_element(lyrics_overlay_progress))
            .child(match description_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match line_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match command_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match toast_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match exit_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match alt_overlay { Some(o) => o, None => div().into_any_element() })
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

fn highlighted_text(text: &str, query: &str, color: u32, current: bool) -> gpui::AnyElement {
    let ranges = line_search::match_ranges(query, text);
    if ranges.is_empty() {
        return div().text_color(rgb(color)).child(text.to_string()).into_any_element();
    }
    let (foreground, background) = if current {
        (palette::on_primary(), palette::primary())
    } else {
        (palette::on_primary_container(), palette::primary_container())
    };
    let highlight = HighlightStyle {
        color: Some(rgb(foreground).into()),
        background_color: Some(rgb(background).into()),
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
        .w(px(size))
        .h(px(size))
        .ml(px(-offset))
        .mr(px(-offset))
        .mt(px(-offset))
        .mb(px(-offset))
        .child(art_plain(path, size))
        .into_any_element()
}

fn art_plain(path: Option<std::path::PathBuf>, size: f32) -> gpui::AnyElement {
    match path {
        Some(path) => img(path)
            .size(px(size))
            .rounded_sm()
            .bg(rgb(palette::surface_container_highest()))
            .into_any_element(),
        None => div()
            .size(px(size))
            .rounded_md()
            .bg(rgb(palette::surface_container_highest()))
            .into_any_element(),
    }
}

fn art(path: Option<std::path::PathBuf>, size: f32) -> gpui::AnyElement {
    match path {
        Some(path) => {
            let id = gpui::SharedString::from(format!("cover-fade:{}:{size:.0}", path.display()));
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

fn asset_root() -> &'static str {
    static ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::var("MUSIKA_ASSET_DIR")
            .ok()
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| format!("{}/src", env!("CARGO_MANIFEST_DIR")))
    })
}

fn icon(name: &str, size: f32, color: u32) -> gpui::AnyElement {
    let path = format!("{}/icons/{}", asset_root(), name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .into_any_element()
}

fn icon_rotated(name: &str, size: f32, color: u32, degrees: f32) -> gpui::AnyElement {
    let path = format!("{}/icons/{}", asset_root(), name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .with_transformation(gpui::Transformation::rotate(gpui::radians(
            degrees.to_radians(),
        )))
        .into_any_element()
}

fn join_phrase(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod fullscreen_focus_tests {
    use super::FullscreenFocus;

    #[test]
    fn tab_cycles_cover_title_and_lyrics() {
        assert_eq!(FullscreenFocus::Cover.next(), FullscreenFocus::Title);
        assert_eq!(FullscreenFocus::Title.next(), FullscreenFocus::Lyrics);
        assert_eq!(FullscreenFocus::Lyrics.next(), FullscreenFocus::Cover);
        assert_eq!(FullscreenFocus::Cover.prev(), FullscreenFocus::Lyrics);
        assert_eq!(FullscreenFocus::Lyrics.prev(), FullscreenFocus::Title);
        assert_eq!(FullscreenFocus::Title.prev(), FullscreenFocus::Cover);
    }
}

#[cfg(test)]
mod contextual_bar_tests {
    use super::*;

    fn page_state(kind: PageKind, starred: bool, cached: bool, downloading: bool) -> PageBarState {
        PageBarState {
            kind,
            id: "id".to_string(),
            title: String::new(),
            subtitle: String::new(),
            meta: String::new(),
            rating: 0,
            starred,
            cached,
            downloading,
        }
    }

    #[test]
    fn contexts_pick_the_right_action_matrix() {
        use TopBarAction::*;
        assert_eq!(
            topbar_actions_for(SuperiorMode::Home, None),
            vec![Sync, Wipe]
        );
        assert!(topbar_actions_for(SuperiorMode::Playlist, None).is_empty());
        assert!(topbar_actions_for(SuperiorMode::Radio, None).is_empty());

        let neutral = page_state(PageKind::Album, false, false, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&neutral)),
            vec![Tool]
        );
        let favorite = page_state(PageKind::Album, true, false, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&favorite)),
            vec![Favorite, Tool]
        );
        let cached = page_state(PageKind::Album, false, true, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&cached)),
            vec![Download, Tool]
        );
        let both = page_state(PageKind::Album, true, true, false);
        assert_eq!(
            topbar_actions_for(SuperiorMode::Album, Some(&both)),
            vec![Favorite, Download]
        );

        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&neutral)),
            vec![Tool]
        );
        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&favorite)),
            vec![Favorite, Tool]
        );
        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&cached)),
            vec![Download, Tool]
        );
        assert_eq!(
            topbar_actions_for(SuperiorMode::Artist, Some(&both)),
            vec![Favorite, Download, Tool]
        );
        assert!(page_tool_visible(&neutral));
        assert!(page_tool_visible(&favorite));
        assert!(page_tool_visible(&cached));
        assert!(!page_tool_visible(&both));
        let downloading = page_state(PageKind::Album, false, true, true);
        assert!(topbar_actions_for(SuperiorMode::Album, Some(&downloading)).contains(&Download));
    }

    #[test]
    fn page_kinds_map_to_contexts() {
        use PageKind;
        assert_eq!(SuperiorMode::from_page(None), SuperiorMode::Home);
        for (kind, mode) in [
            (PageKind::Album, SuperiorMode::Album),
            (PageKind::Artist, SuperiorMode::Artist),
            (PageKind::Playlist, SuperiorMode::Playlist),
            (PageKind::Radio, SuperiorMode::Radio),
        ] {
            let page = PageState::new(PageDescriptor::new(kind, "id"));
            assert_eq!(SuperiorMode::from_page(Some(&page)), mode);
        }
    }

    #[test]
    fn stars_fill_only_the_rating() {
        assert_eq!(AppRoot::star_glyphs(0), [false; 5]);
        assert_eq!(AppRoot::star_glyphs(3), [true, true, true, false, false]);
        assert_eq!(AppRoot::star_glyphs(5), [true; 5]);
        assert_eq!(AppRoot::star_glyphs(9), [true; 5]);
    }

    #[test]
    fn tab_wraps_through_the_action_order() {
        let album = [4, 5, 3];
        assert_eq!(next_tab_slot(&album, None, false), Some(4));
        assert_eq!(next_tab_slot(&album, Some(4), false), Some(5));
        assert_eq!(next_tab_slot(&album, Some(5), false), Some(3));
        assert_eq!(next_tab_slot(&album, Some(3), false), None);
        assert_eq!(next_tab_slot(&album, None, true), Some(3));
        assert_eq!(next_tab_slot(&album, Some(3), true), Some(5));
        assert_eq!(next_tab_slot(&album, Some(5), true), Some(4));
        assert_eq!(next_tab_slot(&album, Some(4), true), None);
        assert_eq!(next_tab_slot(&[], None, false), None);
    }

    #[test]
    fn album_counts_are_pluralized() {
        assert_eq!(crate::ui::bridge::albums_label(0), "0 albums");
        assert_eq!(crate::ui::bridge::albums_label(1), "1 album");
        assert_eq!(crate::ui::bridge::albums_label(4), "4 albums");
    }

    #[test]
    fn alt_actions_are_thirty_percent_faster() {
        assert!((anim::ALT_MS - anim::EMPHASIZED_MS * 0.7).abs() < f32::EPSILON);
        assert!((anim::ALT_MS - 168.0).abs() < 0.001);
    }
}
