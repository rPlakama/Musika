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
mod state;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Render, ScrollStrategy, Window,
    AssetSource, WindowBounds, WindowOptions, actions, div, img, prelude::*, px, relative, rgb, rgba, size, svg, uniform_list,
    UniformListScrollHandle,
};

use bridge::Event;
use state::{PageDescriptor, PageKind, PageState, Row, Scope, Travel, ViewState};

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
            KeyBinding::new("escape", Back, None),
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
            KeyBinding::new("/", Search, None),
            KeyBinding::new("shift-space f", Search, None),
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
}

struct LyricsState {
    song_id: String,
    lines: Vec<(Option<i64>, String)>,
    synced: bool,
    source: String,
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

#[derive(Debug, Clone)]
struct ToastState {
    message: String,
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
    page: Option<PageState>,
    travel: Travel,
    page_generation: u64,
    page_scroll: UniformListScrollHandle,
    search: Option<SearchState>,
    search_open: bool,
    search_anim: anim::Behavior,
    clock: anim::Clock,
    row_colors: std::collections::HashMap<String, anim::ColorBehavior>,
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
            page: None,
            travel: Travel::default(),
            page_generation: 0,
            page_scroll: UniformListScrollHandle::new(),
            search: None,
            search_open: false,
            search_anim: anim::Behavior::new(0.0),
            clock: anim::Clock::new(),
            row_colors: std::collections::HashMap::new(),
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
        };
        if let Ok(message) = std::env::var("MUSIKA_GPUUI_TOAST")
            && !message.is_empty()
        {
            app.push_toast(message, None);
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
            app.search = Some(SearchState {
                query: query.clone(),
                rows: Vec::new(),
                scope: 0,
                scope_cursors: [0; 4],
                generation: 1,
                loading: true,
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
                self.view.rows = rows;
                self.view.loading = false;
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.cursor = saved as usize;
                }
                self.view.cursor = self.view.cursor.min(self.view.rows.len().saturating_sub(1));
                self.scroll
                    .scroll_to_item(self.view.cursor, ScrollStrategy::Center);
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
                self.page_scroll
                    .scroll_to_item(page.cursor, ScrollStrategy::Center);
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
                let retry = if message.to_lowercase().contains("playback") {
                    self.last_play.as_ref().map(|_| RetryAction::Playback)
                } else if message.to_lowercase().contains("library") || message.to_lowercase().contains("search") {
                    Some(RetryAction::ReloadLibrary)
                } else {
                    None
                };
                self.push_toast(message, retry);
            }
        }
    }

    fn reload(&mut self) {
        self.view.loading = true;
        self.generation = self.generation.wrapping_add(1);
        self.bridge.load_library(self.view.scope, self.view.order);
    }

    fn set_scope(&mut self, scope: Scope) {
        if scope == self.view.scope {
            return;
        }
        self.remember_current();
        self.view.clear_for(scope);
        self.reload();
    }

    fn select(&mut self, index: usize) {
        self.view.select(index);
        self.scroll
            .scroll_to_item(self.view.cursor, ScrollStrategy::Center);
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
            self.queue_scroll.scroll_to_item(self.queue_cursor, ScrollStrategy::Center);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_add(1));
            self.page_scroll.scroll_to_item(page.cursor, ScrollStrategy::Center);
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
            self.queue_scroll.scroll_to_item(self.queue_cursor, ScrollStrategy::Center);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_sub(1));
            self.page_scroll.scroll_to_item(page.cursor, ScrollStrategy::Center);
        } else {
            self.select(self.view.cursor.saturating_sub(1));
        }
        cx.notify();
    }

    fn on_top(&mut self, _: &Top, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.select(0);
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
            self.queue_scroll.scroll_to_item(self.queue_cursor, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn on_queue_remove(&mut self, _: &QueueRemove, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open {
            self.player.remove(self.queue_cursor);
            self.queue_cursor = self.queue_cursor.min(self.player.len().saturating_sub(1));
            self.queue_scroll.scroll_to_item(self.queue_cursor, ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn on_queue_up(&mut self, _: &QueueUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open && self.queue_cursor > 0 {
            self.player.move_item(self.queue_cursor, self.queue_cursor - 1);
            self.queue_cursor -= 1;
            self.queue_scroll.scroll_to_item(self.queue_cursor, ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn on_queue_down(&mut self, _: &QueueDown, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open && self.queue_cursor + 1 < self.player.len() {
            self.player.move_item(self.queue_cursor, self.queue_cursor + 1);
            self.queue_cursor += 1;
            self.queue_scroll.scroll_to_item(self.queue_cursor, ScrollStrategy::Center);
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
            ("Server", login.server.as_str(), login.field == 0),
            ("Username", login.username.as_str(), login.field == 1),
            ("Password", if login.password.is_empty() { "" } else { "••••••••" }, login.field == 2),
        ];
        div()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(ON_SURFACE))
            .font_family("Overused Grotesk")
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(420.0))
                    .gap_4()
                    .child(div().text_size(px(30.0)).text_center().child("Musika"))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(ON_VARIANT))
                            .text_center()
                            .child("Sign in to your Subsonic server"),
                    )
                    .children(fields.into_iter().map(|(label, value, active)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .h(px(40.0))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(if active { PRIMARY } else { OUTLINE }))
                                    .bg(rgb(SURFACE))
                                    .text_size(px(14.0))
                                    .child(if value.is_empty() {
                                        "…".to_string()
                                    } else {
                                        value.to_string()
                                    }),
                            )
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(if login.error.is_some() { rgb(0xFFB4AB) } else { rgb(ON_VARIANT) })
                            .child(login.error.clone().unwrap_or_else(|| {
                                if login.busy {
                                    "Connecting…".to_string()
                                } else if login.offline_available {
                                    "Enter logs in · Tab moves · F2 uses downloads offline".to_string()
                                } else {
                                    "Enter logs in · Tab moves".to_string()
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

    fn push_toast(&mut self, message: String, retry: Option<RetryAction>) {
        if self.toasts.iter().any(|toast| toast.message == message) {
            return;
        }
        self.toasts.push(ToastState {
            message,
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
                            .child(toast.message.clone()),
                    )
                    .when_some(retry, |this, retry| {
                        this.child(
                            div()
                                .id("toast-retry")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(SURFACE_HIGH))
                                .text_size(px(11.0))
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

    fn on_search(&mut self, _: &Search, _window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_none() {
            self.search_open = true;
            self.search = Some(SearchState {
                query: String::new(),
                rows: Vec::new(),
                scope: 0,
                scope_cursors: [0; 4],
                generation: 0,
                loading: false,
            });
        }
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

    fn on_key_down(&mut self, event: &gpui::KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.login.is_some() {
            cx.stop_propagation();
            self.login_key(event, cx);
            return;
        }
        if self.search.is_none() || !self.search_open {
            return;
        }
        cx.stop_propagation();
        let Some(search) = self.search.as_mut() else { return };
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform;
        match key {
            "escape" => {
                self.search_open = false;
                cx.notify();
                return;
            }
            "enter" => {
                let row = search.scoped().get(search.cursor()).cloned().cloned();
                if let Some(row) = row {
                    self.search_open = false;
                    self.search = None;
                    self.open_row_page(&row);
                }
                cx.notify();
                return;
            }
            "backspace" => {
                search.query.pop();
            }
            "up" | "ctrl-p" => {
                search.move_cursor(-1);
                cx.notify();
                return;
            }
            "down" | "ctrl-n" => {
                search.move_cursor(1);
                cx.notify();
                return;
            }
            _ if !modified => {
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
        if search.query.is_empty() {
            search.rows.clear();
            search.loading = false;
        } else {
            search.generation = search.generation.wrapping_add(1);
            search.loading = true;
            self.bridge.search(search.query.clone());
        }
        cx.notify();
    }

    fn on_page_down(&mut self, _: &PageDown, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_add(10));
            self.page_scroll.scroll_to_item(page.cursor, ScrollStrategy::Center);
        } else {
            self.select(self.view.cursor.saturating_add(10));
        }
        cx.notify();
    }

    fn on_page_up(&mut self, _: &PageUp, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_sub(10));
            self.page_scroll.scroll_to_item(page.cursor, ScrollStrategy::Center);
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
        self.remember_current();
        self.travel.open(descriptor.clone());
        self.page = Some(PageState::new(descriptor.clone()));
        self.page_generation = self.page_generation.wrapping_add(1);
        self.bridge.load_page(descriptor);
        self.page_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn back_page(&mut self) {
        self.remember_current();
        match self.travel.back() {
            Some(descriptor) => {
                self.page = Some(PageState::new(descriptor.clone()));
                self.page_generation = self.page_generation.wrapping_add(1);
                self.bridge.load_page(descriptor);
                self.page_scroll.scroll_to_item(0, ScrollStrategy::Top);
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
            self.page_scroll.scroll_to_item(0, ScrollStrategy::Top);
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
                            .text_size(px(16.0))
                            .text_color(rgb(ON_VARIANT))
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
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .w_full()
            .h(relative(0.08))
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
            );

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
                    let row_h = 58.0;
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let row = &this.view.rows[index];
                        let title = row.title.clone();
                        let subtitle = row.subtitle.clone();
                        let meta = row.meta.clone();
                        let starred = row.starred;
                        let downloaded = row.downloaded;
                        let cover = cover_path(&row.cover_url);
                        let selected = index == this.view.cursor
                            || this.view.selection.contains(this.view.cursor, index);
                        let striped = index % 2 != 0;
                        let target = if selected { 0x4A4458FF } else if striped { 0x0F0D13FF } else { 0x14121800 };
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
                                .rounded_md()
                                .bg(rgba(color))
                                .cursor_pointer()
                                .hover(|this| this.bg(rgb(SURFACE_HIGH)))
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
                                        .child(
                                            div()
                                                .text_size(px(18.0))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
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
                                                .child(subtitle),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(rgb(ON_VARIANT))
                                                .child(meta),
                                        )
                                        .when(starred, |this| {
                                            this.child(div().text_size(px(12.0)).text_color(rgb(0xFFD54F)).child("★"))
                                        })
                                        .when(downloaded, |this| {
                                            this.child(div().text_size(px(11.0)).text_color(rgb(PRIMARY)).child("↓"))
                                        }),
                                ),
                        );
                    }
                    items
                }))
                .h_full()
                .w_full()
                .track_scroll(scroll),
            )
    }

    fn fullscreen_overlay(&self, progress: f32) -> gpui::Div {
        let track = self.player.current().cloned();
        let (title, artist, cover) = match &track {
            Some(track) => (track.title.clone(), track.artist.clone(), cover_path(&track.cover_url)),
            None => ("Nothing playing".to_string(), String::new(), None),
        };
        let position = self.player.position();
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
        let start = active.saturating_sub(4);
        let end = (active + 8).min(lines.len());
        let visible: Vec<(usize, Option<i64>, String)> = lines
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|(index, (start, text))| (index, *start, text.clone()))
            .collect();
        let lyrics_label = match lyrics {
            Some(lyrics) if lyrics.lines.is_empty() => "No lyrics".to_string(),
            Some(lyrics) => format!("Lyrics · {}", lyrics.source),
            None => "Loading lyrics…".to_string(),
        };
        div()
            .absolute()
            .inset_0()
            .opacity(progress)
            .bg(rgb(BG))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .child(art(cover, 140.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(div().text_size(px(24.0)).child(title))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(artist),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w(px(560.0))
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(lyrics_label),
                    )
                    .children(visible.into_iter().map(|(index, _, text)| {
                        let is_active = index == active;
                        div()
                            .text_size(px(if is_active { 18.0 } else { 14.0 }))
                            .text_color(rgb(if is_active { ON_SURFACE } else { ON_VARIANT }))
                            .when(is_active, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                            .child(text)
                    })),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(ON_VARIANT))
                    .child("Shift-Enter or q closes · Space play/pause · Ctrl-Q queue"),
            )
    }

    fn queue_overlay(&self, cx: &Context<Self>, progress: f32) -> gpui::Div {
        let queue = self.player.queue_snapshot();
        let count = queue.len();
        let current = self.player.index();
        let cursor = self.queue_cursor;
        let scroll = self.queue_scroll.clone();
        div()
            .absolute()
            .top(px(52.0))
            .bottom(px(66.0))
            .right(px(-40.0 * (1.0 - progress)))
            .w(relative(0.36))
            .opacity(progress)
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
                    .text_size(px(13.0))
                    .child("Queue")
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(format!("{} / {}", current.saturating_add(1), count)),
                    ),
            )
            .child(
                uniform_list("queue", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let queue = this.player.queue_snapshot();
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let Some(track) = queue.get(index) else { break };
                        let title = track.title.clone();
                        let artist = track.artist.clone();
                        let active = index == cursor;
                        let playing = index == this.player.index();
                        items.push(
                            div()
                                .id(index)
                                .flex()
                                .items_center()
                                .gap_2()
                                .h(px(40.0))
                                .px_3()
                                .cursor_pointer()
                                .when(active, |this| this.bg(rgb(SURFACE_HIGH)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.queue_cursor = index;
                                    this.player.jump(index);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .w(px(16.0))
                                        .text_size(px(10.0))
                                        .text_color(rgb(if playing { PRIMARY } else { ON_VARIANT }))
                                        .child(if playing { "▶" } else { "" }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .flex_1()
                                        .child(div().text_size(px(12.0)).child(title))
                                        .child(
                                            div()
                                                .text_size(px(10.0))
                                                .text_color(rgb(ON_VARIANT))
                                                .child(artist),
                                        ),
                                ),
                        );
                    }
                    items
                }))
                .h_full()
                .track_scroll(scroll),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(10.0))
                    .text_color(rgb(ON_VARIANT))
                    .child("Enter jumps · d d removes · Ctrl-Shift-J/K reorders · q closes"),
            )
    }


    fn search_overlay(&self, search: &SearchState, progress: f32) -> gpui::Div {
        let query = if search.query.is_empty() {
            "Search artists, albums, songs".to_string()
        } else {
            format!("{}▌", search.query)
        };
        let results = search
            .scoped()
            .into_iter()
            .take(12)
            .enumerate()
            .map(|(index, row)| {
                let selected = index == search.cursor();
                let cover = cover_path(&row.cover_url);
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_5()
                    .h(px(56.0))
                    .when(selected, |this| this.bg(rgb(0x4A4458)))
                    .child(art(cover, 40.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(row.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .whitespace_nowrap()
                                    .text_ellipsis()
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
                                    .text_size(px(20.0))
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
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .text_center()
                            .child("Type to filter · Ctrl-H/L or Tab scope · ↑/↓ rows ·\nEnter open/play · Esc normal mode"),
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
        let cover = cover_path(&page.header.cover_url);

        let header = div()
            .relative()
            .flex_none()
            .w_full()
            .h(px(520.0))
            .px(px(12.0))
            .child(
                div()
                    .absolute()
                    .right(px(16.0))
                    .top(px(48.0))
                    .child(art(cover, 360.0)),
            )
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
                            .child("★ ★ ★ ★ ★"),
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
                    ),
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
            .flex_1()
            .w_full()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(520.0))
                    .child(header),
            )
            .child(
                div()
                    .absolute()
                    .top(px(560.0))
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(
                        uniform_list("page-rows", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            let row_h = 36.0;
                            let mut items = Vec::with_capacity(range.len());
                            for index in range {
                                let Some((title, subtitle, meta, selected, key)) = this
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
                                            format!("page:{}:{}", page.descriptor.id, row.id),
                                        )
                                    })
                                else {
                                    break;
                                };
                                let target = if selected { 0x4A4458FF } else { 0x14121800 };
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
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(page) = this.page.as_mut() {
                                                page.select(index);
                                            }
                                            this.page_scroll.scroll_to_item(index, ScrollStrategy::Center);
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .min_w_0()
                                                .flex_1()
                                                .child(div().text_size(px(15.0)).child(title))
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .text_color(rgb(ON_VARIANT))
                                                        .child(subtitle),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(rgb(ON_VARIANT))
                                                .child(meta),
                                        ),
                                );
                            }
                            items
                        }))
                        .h_full()
                        .w_full()
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
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(if muted {
                                        "muted".to_string()
                                    } else {
                                        format!("vol {:.0}%", volume)
                                    }),
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
        let fullscreen_overlay = (self.fullscreen || fullscreen_progress > 0.001)
            .then(|| self.fullscreen_overlay(fullscreen_progress));
        let exit_overlay = self.exiting.map(|started| {
            let progress = (((self.clock.now - started) / 0.200) as f32).clamp(0.0, 1.0);
            self.exit_overlay(progress)
        });
        if self.row_colors.values().any(|color| !color.settled(&self.clock)) {
            window.request_animation_frame();
        }
        self.toasts.retain(|toast| self.clock.now - toast.created_at < 6.320);
        if self.toasts.iter().any(|toast| self.clock.now - toast.created_at < 6.320) {
            window.request_animation_frame();
        }
        let toast_overlay = (!self.toasts.is_empty()).then(|| self.toast_overlay(&*cx));
        let queue_overlay = (self.queue_open || queue_progress > 0.001)
            .then(|| self.queue_overlay(&*cx, queue_progress));
        let overlay = self.search.as_ref().map(|search| self.search_overlay(search, progress));
        if let Some(started) = self.exiting {
            window.request_animation_frame();
            if self.clock.now - started >= 0.200 {
                self.flush_status();
                window.remove_window();
            }
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
            .font_family("Overused Grotesk")
            .text_color(rgb(ON_SURFACE))
            .child(top)
            .child(body)
            .child(status)
            .child(match queue_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
            .child(match fullscreen_overlay { Some(o) => o.into_any_element(), None => div().into_any_element() })
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

fn cover_path(url: &str) -> Option<std::path::PathBuf> {
    let clean = url.split('#').next()?;
    if clean.is_empty() {
        return None;
    }
    reqwest::Url::parse(clean).ok()?.to_file_path().ok()
}

fn art(path: Option<std::path::PathBuf>, size: f32) -> gpui::AnyElement {
    match path {
        Some(path) => img(path).size(px(size)).rounded_sm().into_any_element(),
        None => div()
            .size(px(size))
            .rounded_md()
            .bg(rgb(SURFACE_HIGH))
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
    match raw.parse::<i64>() {
        Ok(secs) if secs > 0 => {
            let age = crate::consts::now_secs().saturating_sub(secs);
            if age < 60 {
                "Now".to_string()
            } else if age < 3_600 {
                format!("{}m", age / 60)
            } else if age < 86_400 {
                format!("{}h", age / 3600)
            } else if age < 604_800 {
                format!("{}d", age / 86_400)
            } else if age < 2_592_000 {
                format!("{}w", age / 604_800)
            } else {
                format!("{}y", age / 31_536_000)
            }
        }
        _ => "—".to_string(),
    }
}

fn icon(name: &str, size: f32, color: u32) -> gpui::AnyElement {
    let path = format!("{}/src/icons/{}", env!("CARGO_MANIFEST_DIR"), name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .into_any_element()
}
