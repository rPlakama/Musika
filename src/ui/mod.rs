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
mod collage;
mod command_line;
mod covers;
pub(crate) mod daemon_client;
#[cfg(debug_assertions)]
mod debug;
pub(crate) use crate::ipc_command;
mod line_search;
mod lyrics_layout;
mod metrics;
mod page_profile;
mod palette;
mod perf;
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
    CoverSource, FAVORITE_GOLD, RowMarks, art, collage_art_from_sources, cover_element,
    cover_source_with_remote, format_time, highlighted_text, icon, icon_rotated,
    loading_placeholder, loading_skeleton, page_key, remote_cover, repeat_label, progress_bar,
    row_mark_icons, transport_controls, volume_meter,
};
use self::widgets::retained_cover::RetainedCover;
use self::widgets::retained_icon::RetainedIcon;
use self::widgets::retained_text::RetainedText;

use gpui::{
    Animation, AnimationExt, App, Application, AssetSource, Bounds, Context, FocusHandle,
    HighlightStyle, KeyBinding, Render, StyledText, UniformListScrollHandle, Window, WindowBounds,
    WindowOptions, actions, div, img, prelude::*, px, relative, rgb, size, svg, uniform_list,
};

use crate::bridge::{Event, SyncMode};
use daemon_client::{AppPlayer, Client, RemoteBridge, UiUpdate};
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
        DeleteRow,
        Yank,
        YankRow,
        PasteBelow,
        PasteAbove,
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
                KeyBinding::new("d d", DeleteRow, Some("main")),
                KeyBinding::new("y", Yank, Some("main")),
                KeyBinding::new("shift-y", YankRow, Some("main")),
                KeyBinding::new("p", PasteBelow, Some("main")),
                KeyBinding::new("shift-p", PasteAbove, Some("main")),
                KeyBinding::new("ctrl-shift-k", QueueUp, Some("main")),
                KeyBinding::new("ctrl-shift-j", QueueDown, Some("main")),
                KeyBinding::new("ctrl-r", RepeatCycle, Some("main")),
                KeyBinding::new("tab", SearchScopeNext, Some("main")),
                KeyBinding::new("shift-tab", SearchScopePrev, Some("main")),
                KeyBinding::new("shift-enter", ToggleFullscreen, Some("main")),
                KeyBinding::new("j", MoveDown, Some("main")),
                KeyBinding::new("k", MoveUp, Some("main")),
                KeyBinding::new("ctrl-j", MoveDown, Some("main")),
                KeyBinding::new("ctrl-k", MoveUp, Some("main")),
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
    /// Ids an in-flight removal already optimistically dropped: the marks
    /// window is the download truth, and this is its only optimistic overlay.
    cleared: std::collections::HashSet<String>,
    /// Hash of the window (player ids + visible row ids) last requested.
    /// `None` forces a new request after a library/page/wipe change.
    signature: Option<u64>,
    inflight: bool,
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
        self.cleared.clear();
        self.inflight = false;
    }

    fn downloaded(&self, song_id: &str, album_id: &str) -> bool {
        let cleared = (!song_id.is_empty() && self.cleared.contains(song_id))
            || (!album_id.is_empty() && self.cleared.contains(album_id));
        !cleared
            && ((!song_id.is_empty() && self.songs_done.contains(song_id))
                || (!album_id.is_empty() && self.albums_done.contains(album_id)))
    }

    fn pending(&self, song_id: &str, album_id: &str) -> bool {
        (!song_id.is_empty() && self.pending.contains(song_id))
            || (!album_id.is_empty() && self.pending.contains(album_id))
    }

    /// Row-level pending check: rows without a song/album id (artists) still
    /// show the loading glyph for their own id.
    fn pending_row(&self, row: &crate::model::Row) -> bool {
        self.pending(&row.song_id, &row.album_id)
            || (!row.id.is_empty() && self.pending.contains(&row.id))
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
    fn api_kind(&self) -> &'static str {
        self.kind.api_kind()
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

/// One queued IPC/`:` command plus whether the surface that queued it must
/// echo it in a toast. The command line does; commands forwarded from the
/// core were already announced there (and key presses are not IPC at all).
struct PendingIpc {
    command: ipc_command::IpcCommand,
    announce: bool,
}

#[derive(Debug, Clone)]
struct ToastState {
    message: String,
    kind: ToastKind,
    retry: Option<RetryAction>,
    created_at: f64,
}

/// Sticky mute bubble. Unlike [`HoldToast`] it has no TTL: it is cleared only
/// when the core reports the mute state back as off, so it stays on screen for
/// as long as the output is muted.
#[derive(Debug, Clone)]
struct MuteToast {
    message: String,
    /// `clock.now` of creation, used only for the fade-in.
    created_at: f64,
}

/// Which accumulator a playback key feeds. `,`/`.` share the seek accumulator
/// and `-`/`+` share the volume one, so alternating between the two keys of a
/// pair keeps folding into a single bubble instead of restarting the count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldKind {
    Seek,
    Volume,
}

impl HoldKind {
    /// Which accumulator a playback key feeds.
    fn of(key: char) -> Option<Self> {
        match key {
            ',' | '.' => Some(Self::Seek),
            '-' | '+' => Some(Self::Volume),
            _ => None,
        }
    }

    /// Signed step one tap contributes: seconds for seeks, volume points for
    /// volume. Both use the same 5-unit magnitude.
    fn step(self, key: char) -> Option<f64> {
        let sign = match key {
            ',' | '-' => -1.0,
            '.' | '+' => 1.0,
            _ => return None,
        };
        match self {
            HoldKind::Seek | HoldKind::Volume => Some(sign * HOLD_STEP),
        }
    }
}

/// The transient "held key" bubble (`,`, `.`, `-`, `+`). Every repeat refreshes
/// the message and `at_ns`, so it needs none of the normal toast rules: it never
/// stacks, never dedupes and never enters the 3-item cap.
#[derive(Debug, Clone)]
struct HoldToast {
    message: String,
    /// Chevron for the volume pair (`up-fill` / `down-fill`); seeks have none.
    icon: Option<&'static str>,
    /// `CLOCK_MONOTONIC` ns of the last refresh. Deliberately not `clock.now`:
    /// that only advances while frames render, so an idle window would make the
    /// first press look ancient and expire the bubble immediately.
    at_ns: u64,
}

impl HoldToast {
    fn new(message: String, icon: Option<&'static str>, now_ns: u64) -> Self {
        Self {
            message,
            icon,
            at_ns: now_ns,
        }
    }

    /// Whether the bubble outlived its TTL after the last refresh.
    fn expired(&self, now_ns: u64) -> bool {
        now_ns.saturating_sub(self.at_ns) >= HOLD_TOAST_TTL_NS
    }
}

/// One held playback key group. Repeats fold into `count` and a signed `delta`
/// until the key is released.
#[derive(Debug, Clone, Copy, PartialEq)]
struct KeyHold {
    kind: HoldKind,
    /// Signed accumulated distance: seconds for [`HoldKind::Seek`], volume
    /// points for [`HoldKind::Volume`]. `,` cancels `.` and `-` cancels `+`, so
    /// spamming a mixed pair nets out while the count still adds up.
    delta: f64,
    count: u32,
    /// `CLOCK_MONOTONIC` ns of the last key-down, for the stale-hold reset.
    at_ns: u64,
}

/// A same-key key-down within this window (and without an intervening key-up)
/// is a repeat. GPUI re-sends a bare key-down for X11 auto-repeat, so timing is
/// the only portable signal; the window must outlast the auto-repeat *delay*
/// (600-660 ms by default, up to ~1 s when tuned) or the first repeat would
/// restart the count instead of incrementing it.
const HOLD_STALE_NS: u64 = 1_500_000_000;

/// Seek/volume step of one playback key tap; the core uses the same 5 s / 5 %.
pub(in crate::ui) const HOLD_STEP: f64 = 5.0;

/// How long the held-key counter stays up after its last refresh. A live hold
/// refreshes it on every repeat, so this only bounds the fade-out and repairs a
/// lost key-up (focus change) instead of leaving a stale bubble on screen. It
/// must also outlast the auto-repeat delay so the bubble does not blink between
/// the initial press and the first repeat.
const HOLD_TOAST_TTL_NS: u64 = 1_200_000_000;

impl KeyHold {
    /// Folds one key-down into the hold. A key of the same family inside
    /// [`HOLD_STALE_NS`] accumulates; another family or a stale hold restarts.
    fn record(previous: Option<KeyHold>, key: char, now_ns: u64) -> Self {
        let Some(kind) = HoldKind::of(key) else {
            return Self {
                kind: HoldKind::Seek,
                delta: 0.0,
                count: 0,
                at_ns: now_ns,
            };
        };
        let step = kind.step(key).unwrap_or(0.0);
        match previous {
            Some(hold) if hold.kind == kind && now_ns.saturating_sub(hold.at_ns) < HOLD_STALE_NS => {
                Self {
                    kind,
                    delta: hold.delta + step,
                    count: hold.count.saturating_add(1),
                    at_ns: now_ns,
                }
            }
            _ => Self {
                kind,
                delta: step,
                count: 1,
                at_ns: now_ns,
            },
        }
    }
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

fn page_tool_visible(page: &PageBarState<'_>) -> bool {
    !page.starred || !page.cached
}

fn page_actions(page: &PageBarState<'_>, artist_tool: bool) -> Vec<TopBarAction> {
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

fn topbar_actions_for(mode: SuperiorMode, page: Option<&PageBarState<'_>>) -> Vec<TopBarAction> {
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

/// The overlay rows (search results, queue) size their square cover as this
/// fraction of the row height; both surfaces previously restated `0.714`.
pub(in crate::ui) const OVERLAY_ROW_COVER_RATIO: f32 = 0.714;

/// The alternating row base fill: the same stripe rule on every list.
pub(in crate::ui) fn row_base_fill(index: usize) -> u32 {
    if !index.is_multiple_of(2) {
        palette::surface_container_lowest()
    } else {
        palette::surface()
    }
}

/// The rendered hover fill for a row: the behaviour's current colour while the
/// row is hovered and not selected, otherwise the striped base.
pub(in crate::ui) fn row_hover_fill(
    behavior: &anim::ColorBehavior,
    clock: &anim::Clock,
    index: usize,
    selected: bool,
    hovered: bool,
) -> u32 {
    if hovered && !selected {
        behavior.value(clock)
    } else {
        row_base_fill(index)
    }
}

/// Retarget the hover fill for a row (the row's `on_hover` path).
pub(in crate::ui) fn target_row_hover_fill(
    behavior: &mut anim::ColorBehavior,
    clock: &anim::Clock,
    index: usize,
    selected: bool,
    hovered: bool,
) {
    behavior.target(
        if hovered && !selected {
            palette::surface_container_high()
        } else {
            row_base_fill(index)
        },
        anim::QUICK_MS,
        anim::Curve::Standard,
        clock,
    );
}

/// The superior bar's projection of the open page. Borrowed from
/// [`PageState`] so opening/rendering the bar does not clone its identity
/// strings; the action code that mutates a page reads what it needs first.
#[derive(Debug, Clone, Copy)]
struct PageBarState<'a> {
    kind: PageKind,
    id: &'a str,
    title: &'a str,
    subtitle: &'a str,
    meta: &'a str,
    rating: u8,
    starred: bool,
    cached: bool,
    downloading: bool,
    /// Radio seed kind (`album`/`song`) for the Tool radio actions.
    radio_seed_kind: Option<&'a str>,
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

/// Wayland/GPUI cannot activate an application on Linux (the platform
/// implementation is a no-op and xdg_activation tokens are rejected without a
/// real input serial). Under niri, use its IPC to focus our window on an
/// explicit `show` request; elsewhere this is a harmless best effort.
fn activate_niri_window() {
    if std::env::var_os("NIRI_SOCKET").is_none() {
        return;
    }
    let pid = std::process::id();
    let Ok(output) = std::process::Command::new("niri")
        .args(["msg", "-j", "windows"])
        .output()
    else {
        return;
    };
    let Ok(windows) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) else {
        return;
    };
    let Some(id) = windows
        .iter()
        .find(|window| window.get("pid").and_then(|pid| pid.as_u64()) == Some(pid as u64))
        .and_then(|window| window.get("id").and_then(|id| id.as_u64()))
    else {
        return;
    };
    crate::debug_log!("GPUI: niri activate window {id}");
    let _ = std::process::Command::new("niri")
        .args(["msg", "action", "focus-window", "--id", &id.to_string()])
        .output();
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

struct AppRoot {
    focus: FocusHandle,
    bridge: RemoteBridge,
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
    pending_page_reveal: Option<PendingReveal>,
    /// (playlist id, cursor row, keep selection) to apply when an edited
    /// playlist page reloads.
    pending_playlist_cursor: Option<(String, usize, bool)>,
    visual_mode: bool,
    /// In-app row clipboard (VI-8a): kind + ids in yank order, duplicates kept.
    clipboard_kind: Option<RowKind>,
    clipboard_ids: Vec<String>,
    /// Clipboard taken by an in-flight paste; restored only if that paste
    /// fails and nothing newer was yanked meanwhile.
    paste_backup: Option<(RowKind, Vec<String>)>,
    last_player_index: usize,
    visuals: Visuals,
    image_cache: gpui::Entity<gpui::RetainAllImageCache>,
    row_scales:
        std::cell::RefCell<std::collections::HashMap<(LineSurface, usize, usize), RowEmphasis>>,
    library_hover_fill: anim::ColorBehavior,
    page_hover_fill: anim::ColorBehavior,
    client: Client,
    player: AppPlayer,
    /// Startup-only: the core was not reachable after the initial connect
    /// attempts, so the root renders a retry surface instead of the UI.
    awaiting_core: bool,
    /// Reconnect banner text (transport lost, reconnecting).
    reconnecting: Option<String>,
    player_bar_cover: views::shell::PlayerBarCover,
    playback_error: Option<String>,
    queue_open: bool,
    queue_list: state::ListState,
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
    /// LYR-8: while a manual scroll holds the pane, this is the line it is
    /// centred on instead of the played line. The idle resume timer clears it;
    /// `lyrics_hold_generation` retires timers of superseded holds.
    lyrics_hold: Option<usize>,
    lyrics_hold_generation: u64,
    lyrics_shift: anim::Behavior,
    lyrics_track_fade: anim::Behavior,
    description_scroll: gpui::ScrollHandle,
    transport_press: [Option<f64>; 3],
    playback_key_hold: Option<KeyHold>,
    /// Live counter bubble for the held playback key; TTL-owned by the render
    /// loop so a lost key-up cannot leave it behind.
    hold_toast: Option<HoldToast>,
    /// Sticky mute bubble; cleared only when the core reports mute off.
    mute_toast: Option<MuteToast>,
    /// Volume level remembered when mute was engaged, so the sticky bubble can
    /// report the level that will come back on unmute.
    mute_toast_level: Option<i64>,
    line_search: line_search::LineSearchState,
    line_search_anim: anim::Behavior,
    command_line: command_line::CommandLineState,
    command_line_anim: anim::Behavior,
    command_suggestions_generation: u64,
    pending_ipc: Vec<PendingIpc>,
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
    resize_recenter: bool,
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
    /// The window activation observer is installed on the first render (the
    /// entity does not exist yet while `new` runs).
    activation_observed: bool,
    #[cfg(debug_assertions)]
    frame_hud: debug::FrameHud,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);

        crate::scroll_memory::hydrate();
        // A dead/slow core must not kill the UI: start disconnected, show the
        // retry surface, and let the supervisor connect when the core is back.
        let (client, mut updates, startup_notice) = match Client::connect() {
            Ok((client, updates)) => (client, updates, None),
            Err(error) => {
                let (client, updates) = Client::connect_when_available();
                eprintln!("musika-ui: waiting for the core daemon: {error}");
                (
                    client,
                    updates,
                    Some(format!("Core daemon unavailable ({error}). Retrying…")),
                )
            }
        };
        let bridge = RemoteBridge::new(client.clone());
        let player = AppPlayer::new(client.clone());
        let mut view = ViewState::default();
        bridge.load_library(view.scope, view.order);
        view.loading = true;

        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.recv().await {
                let keep = this.update(cx, |this, cx| {
                    let mut notify = true;
                    match update {
                        UiUpdate::Event(Event::Command(command)) => {
                            this.handle_command(&command, cx);
                        }
                        UiUpdate::Event(event) => this.apply(event),
                        UiUpdate::State(state) => {
                            // Playback frames arrive on transport changes only;
                            // the anchor carries the position between them.
                            let old_state = this.player.state();
                            let old_index = this.player.index();
                            let old_loading = this.player.is_loading();
                            this.player.apply_state(state);
                            this.repeat = this.player.repeat();
                            this.clamp_queue();
                            this.drain_resync();
                            this.sync_mute_toast(this.player.muted());
                            notify = old_state != this.player.state()
                                || old_index != this.player.index()
                                || old_loading != this.player.is_loading();
                        }
                        UiUpdate::QueueDelta(delta) => {
                            this.player.apply_delta(delta);
                            this.clamp_queue();
                            this.drain_resync();
                        }
                        UiUpdate::QueueRows(rows) => {
                            this.player.apply_rows(rows);
                            this.clamp_queue();
                            this.drain_resync();
                        }
                        UiUpdate::PlayerReply(reply) => {
                            this.player.apply_player_reply(reply);
                            this.drain_resync();
                            if !reply.ok {
                                this.push_toast(
                                    format!(
                                        "Core rejected a playback command ({:?}).",
                                        reply.code
                                    ),
                                    ToastKind::Error,
                                    None,
                                );
                            }
                        }
                        UiUpdate::Reply(reply) => {
                            notify = false;
                            if !reply.ok {
                                crate::debug_log!(
                                    "ipc: request {} failed ({:?}): {}",
                                    reply.req,
                                    reply.code,
                                    reply.message.unwrap_or_default()
                                );
                            }
                        }
                        UiUpdate::Activate => {
                            // Linux/GPUI activate() is a no-op and the Wayland
                            // activation token is rejected without a real input
                            // serial; use the compositor IPC fallback instead.
                            cx.activate(true);
                            activate_niri_window();
                        }
                        UiUpdate::Connected { session } => {
                            crate::debug_log!("musika-ui: connected to core session {session}");
                            this.awaiting_core = false;
                            this.reconnecting = None;
                            this.repeat = this.player.repeat();
                            // A (re)connect starts a fresh stream: rebuild the
                            // bridge surfaces the core did not replay itself.
                            this.bridge.refresh_header();
                            this.bridge.load_library(this.view.scope, this.view.order);
                            if let Some(page) = this.page.as_ref() {
                                let descriptor = page.descriptor.clone();
                                this.bridge.load_page(descriptor);
                            }
                            this.marks.signature = None;
                        }
                        UiUpdate::Reconnecting(reason) => {
                            let message = match reason {
                                Some(reason) => {
                                    format!("Core connection lost ({reason}); reconnecting…")
                                }
                                None => "Core connection lost; reconnecting…".to_string(),
                            };
                            this.reconnecting = Some(message.clone());
                            this.push_toast(message, ToastKind::Notice, None);
                        }
                        UiUpdate::Fatal(message) => {
                            this.error = Some(message.clone());
                            this.reconnecting = Some(message.clone());
                            this.push_toast(message, ToastKind::Error, None);
                            cx.notify();
                            cx.quit();
                            return false;
                        }
                        UiUpdate::Shutdown => {
                            cx.quit();
                            return false;
                        }
                    }
                    if notify {
                        cx.notify();
                    }
                    true
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();

        let mut app = Self {
            focus,
            client,
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
            pending_page_reveal: None,
            pending_playlist_cursor: None,
            visual_mode: false,
            clipboard_kind: None,
            clipboard_ids: Vec::new(),
            paste_backup: None,
            last_player_index: 0,
            visuals: Visuals::default(),
            image_cache: gpui::RetainAllImageCache::new(cx),
            row_scales: std::cell::RefCell::new(std::collections::HashMap::new()),
            library_hover_fill: anim::ColorBehavior::new(palette::surface()),
            page_hover_fill: anim::ColorBehavior::new(palette::surface()),
            awaiting_core: startup_notice.is_some(),
            reconnecting: startup_notice,
            player,
            player_bar_cover: views::shell::PlayerBarCover::default(),
            playback_error: None,
            queue_open: false,
            queue_list: state::ListState::default(),
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
            lyrics_hold: None,
            lyrics_hold_generation: 0,
            lyrics_shift: anim::Behavior::new(0.0),
            lyrics_track_fade: anim::Behavior::new(1.0),
            description_scroll: gpui::ScrollHandle::new(),
            transport_press: [None; 3],
            playback_key_hold: None,
            hold_toast: None,
            mute_toast: None,
            mute_toast_level: None,
            line_search: line_search::LineSearchState::new(),
            line_search_anim: anim::Behavior::new(0.0),
            command_line: command_line::CommandLineState::new(),
            command_line_anim: anim::Behavior::new(0.0),
            command_suggestions_generation: 0,
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
            resize_recenter: false,
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
            activation_observed: false,
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
            app.queue_list.cursor = app.player.index();
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

    /// Answers the view's "my rows/transport are stale" flag with one resync
    /// request: a `seq`/`rev` gap or a dropped row chunk means the projection
    /// can only be repaired by replaying the core snapshot.
    fn drain_resync(&mut self) {
        if self.player.take_resync() {
            self.client.request_resync();
        }
    }

    /// Keeps the queue cursor inside the row window the core reported.
    fn clamp_queue(&mut self) {
        self.queue_list.cursor = self.queue_list.cursor.min(self.player.len().saturating_sub(1));
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
