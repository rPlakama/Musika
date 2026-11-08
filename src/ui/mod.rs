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
//!
//! Module layout:
//! - `app/` — backend event application, key actions, navigation and search state.
//! - `views/` — render-only surfaces (shell, library, page, fullscreen, queue, overlays).
//! - `bridge/` — tokio request families and the single `Event` channel.
//! - `widgets/` — shared text editing, media and row primitives.
//! - `visuals/` — centralized typography and motion tokens.
//! - `text_modal` — the one editable-text engine used by every text box.

mod anim;
mod app;
mod bridge;
mod collage;
mod command_line;
mod covers;
#[cfg(debug_assertions)]
mod debug;
pub(crate) mod ipc_command;
mod line_search;
mod lyrics_layout;
mod metrics;
mod page_profile;
mod palette;
mod quick_actions;
mod remote_images;
mod scroll_modal;
mod state;
mod text_modal;
mod views;
mod visuals;
mod widgets;

use self::text_modal::TextModal;
use self::widgets::marquee::MarqueeText;
use self::widgets::media::{
    CoverSource, art, asset_root, collage_art, cover_source_with_remote, format_time,
    highlighted_text, icon, icon_rotated, loading_placeholder, loading_skeleton, page_key,
    remote_cover, repeat_label, transport_button,
};
use self::widgets::retained_cover::RetainedCover;
use self::widgets::retained_icon::RetainedIcon;
use self::widgets::retained_text::RetainedText;

use gpui::{
    Animation, AnimationExt, App, Application, AssetSource, Bounds, Context, FocusHandle,
    HighlightStyle, KeyBinding, Render, StyledText, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, actions, div, img, prelude::*, px, relative, rgb, size, svg, uniform_list,
};

use bridge::{Event, SyncMode};
use quick_actions::{AltGesture, Direction, QuickKind, QuickMenu, QuickVerb};
use state::{
    PageDescriptor, PageKind, PageState, Row, RowKind, RowWindow, Scope, Travel, ViewState,
};
use visuals::Visuals;

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
        .with_http_client(remote_images::app_http_client())
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
            .expect(
                "GPUI: could not open the application window (Vulkan/display initialization failed; run `vulkaninfo --summary`)",
            );
        });
}

struct Header {
    server_url: String,
    username: String,
    server_type: String,
    last_sync: Option<String>,
    counts: (i64, i64, i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LineSurface {
    Search,
    Queue,
    Page,
    Library,
    Lyrics,
    Status,
}

struct SearchState {
    editor: TextModal,
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
    radio_seed_kind: String,
    radio_seed_id: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginMode {
    SignIn,
    AddAccount,
}

struct LoginState {
    mode: LoginMode,
    server: TextModal,
    username: TextModal,
    password: TextModal,
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
    emphasis: f32,
}

struct RowEmphasis {
    cover: anim::Behavior,
    text: anim::Behavior,
}

impl RowEmphasis {
    fn new(active: bool) -> Self {
        let value = if active { 1.0 } else { 0.0 };
        Self {
            cover: anim::Behavior::new(value),
            text: anim::Behavior::new(value),
        }
    }
}

struct SearchRowPaint {
    scope: usize,
    index: usize,
    row_h: f32,
    selected: bool,
    hovered: bool,
    /// 0 = resting, 1 = selected/hovered. Animated with a short linear curve.
    scale: f32,
    /// Text factor, animated separately and 80% slower than the cover.
    text_scale: f32,
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
        SuperiorMode::Home => vec![TopBarAction::Sync],
        SuperiorMode::Album => page
            .map(|page| page_actions(page, false))
            .unwrap_or_default(),
        SuperiorMode::Artist => page
            .map(|page| page_actions(page, true))
            .unwrap_or_default(),
        SuperiorMode::Playlist => Vec::new(),
        SuperiorMode::Radio => vec![TopBarAction::Tool],
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
    /// Radio seed kind (`album`/`song`) for the Tool radio actions.
    radio_seed_kind: Option<String>,
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
    visuals: Visuals,
    image_cache: gpui::Entity<gpui::RetainAllImageCache>,
    row_scales:
        std::cell::RefCell<std::collections::HashMap<(LineSurface, usize, usize), RowEmphasis>>,
    library_hover_fill: anim::ColorBehavior,
    page_hover_fill: anim::ColorBehavior,
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
    resize_last_size: Option<(f32, f32)>,
    resize_settle_at: Option<f64>,
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
    #[cfg(debug_assertions)]
    frame_hud: debug::FrameHud,
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
            visuals: Visuals::default(),
            image_cache: gpui::RetainAllImageCache::new(cx),
            row_scales: std::cell::RefCell::new(std::collections::HashMap::new()),
            library_hover_fill: anim::ColorBehavior::new(palette::surface()),
            page_hover_fill: anim::ColorBehavior::new(palette::surface()),
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
            status_anchors: std::rc::Rc::new(std::cell::RefCell::new([
                None, None, None, None, None, None,
            ])),
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
            resize_last_size: None,
            resize_settle_at: None,
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
            #[cfg(debug_assertions)]
            frame_hud: debug::FrameHud::default(),
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
                editor: TextModal::from_text(&query),
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

    const SYNC_TURN_SECS: f64 = 1.05;
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

impl SearchState {
    fn kind_index(kind: state::RowKind) -> usize {
        match kind {
            state::RowKind::Song => 0,
            state::RowKind::Album => 1,
            state::RowKind::Artist => 2,
            state::RowKind::Playlist => 3,
        }
    }

    /// Install a search payload without replacing rows that are already equal.
    ///
    /// Search requests race typing, so a payload is often identical to what is
    /// already on screen. Keeping those rows (and the cursor/selection) avoids
    /// a pointless list rebuild and cover reload. When the length changes the
    /// vector is replaced; otherwise only rows whose value differs are written.
    fn merge_rows(&mut self, rows: Vec<Row>, most_played: bool) -> bool {
        if self.most_played == most_played && self.rows == rows {
            return false;
        }
        if self.rows.len() == rows.len() {
            for (current, incoming) in self.rows.iter_mut().zip(rows) {
                if *current != incoming {
                    *current = incoming;
                }
            }
        } else {
            self.rows = rows;
        }
        self.most_played = most_played;
        true
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

fn join_phrase(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests;
