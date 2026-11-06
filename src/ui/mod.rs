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

mod bridge;
mod state;

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Render, ScrollStrategy, Window,
    WindowBounds, WindowOptions, actions, div, img, prelude::*, px, rgb, size, uniform_list,
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
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("q", Back, None),
            KeyBinding::new("escape", Back, None),
            KeyBinding::new("ctrl-q", Quit, None),
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
            KeyBinding::new("ctrl-n", SearchNext, None),
            KeyBinding::new("ctrl-p", SearchPrev, None),
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
    last_sync: Option<String>,
    counts: (i64, i64, i64),
}

struct SearchState {
    query: String,
    rows: Vec<Row>,
    cursor: usize,
    generation: u64,
    loading: bool,
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
        };
        if let Ok(query) = std::env::var("MUSIKA_GPUUI_SEARCH")
            && !query.is_empty()
        {
            app.search = Some(SearchState {
                query: query.clone(),
                rows: Vec::new(),
                cursor: 0,
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
                last_sync,
                counts,
            } => {
                self.header = Some(Header {
                    server_url,
                    username,
                    last_sync,
                    counts,
                });
                self.error = None;
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
                page.header = header;
                page.rows = rows;
                page.loading = false;
                page.error = None;
                page.cursor = page.cursor.min(page.rows.len().saturating_sub(1));
                self.page_scroll
                    .scroll_to_item(page.cursor, ScrollStrategy::Center);
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
                    search.cursor = 0;
                    search.loading = false;
                }
            }
            Event::Error(message) => {
                self.view.loading = false;
                self.view.error = Some(message.clone());
                self.error = Some(message);
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
            search.cursor = search.cursor.saturating_add(1).min(search.rows.len().saturating_sub(1));
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
            search.cursor = search.cursor.saturating_sub(1);
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
        self.cycle_order(true);
        cx.notify();
    }

    fn on_order_prev(&mut self, _: &OrderPrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_order(false);
        cx.notify();
    }

    fn on_refresh(&mut self, _: &Refresh, _window: &mut Window, cx: &mut Context<Self>) {
        self.reload();
        cx.notify();
    }

    fn on_search(&mut self, _: &Search, _window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_none() {
            self.search = Some(SearchState {
                query: String::new(),
                rows: Vec::new(),
                cursor: 0,
                generation: 0,
                loading: false,
            });
        }
        cx.notify();
    }

    fn on_search_next(&mut self, _: &SearchNext, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.cursor = search.cursor.saturating_add(1).min(search.rows.len().saturating_sub(1));
        }
        cx.notify();
    }

    fn on_search_prev(&mut self, _: &SearchPrev, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = self.search.as_mut() {
            search.cursor = search.cursor.saturating_sub(1);
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &gpui::KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform;
        if key == "backspace" {
            search.query.pop();
        } else if key == "escape" {
            self.search = None;
            cx.notify();
            return;
        } else if !modified {
            if let Some(text) = event.keystroke.key_char.as_ref()
                && !text.chars().any(char::is_control)
            {
                search.query.push_str(text);
            } else if key == "space" {
                search.query.push(' ');
            }
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
        if let Some(search) = self.search.as_ref() {
            let row = search.rows.get(search.cursor).cloned();
            if let Some(row) = row {
                self.search = None;
                self.open_row_page(&row);
            }
            cx.notify();
            return;
        }
        if self.page.is_some() {
            let row = self
                .page
                .as_ref()
                .and_then(|page| page.rows.get(page.cursor))
                .cloned();
            if let Some(row) = row {
                self.open_row_page(&row);
            }
        } else if let Some(row) = self.view.rows.get(self.view.cursor).cloned() {
            self.open_row_page(&row);
        }
        cx.notify();
    }

    fn on_back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            self.search = None;
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
        } else {
            window.remove_window();
        }
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

    fn top_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let account = self
            .header
            .as_ref()
            .map(|header| {
                format!(
                    "{} · {}",
                    header.username, header.server_url
                )
            })
            .unwrap_or_else(|| "loading account…".to_string());

        div()
            .flex()
            .items_center()
            .h(px(52.0))
            .px_5()
            .bg(rgb(SURFACE))
            .child(div().text_size(px(18.0)).child("Musika"))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .justify_center()
                    .gap_1()
                    .children(Scope::ALL.into_iter().enumerate().map(|(index, scope)| {
                        let active = scope == self.view.scope;
                        div()
                            .id(("scope", index))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(rgb(if active { PRIMARY } else { ON_VARIANT }))
                            .when(active, |this| this.bg(rgb(SURFACE_HIGH)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_scope(scope);
                                cx.notify();
                            }))
                            .child(scope.label())
                    })),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(ON_VARIANT))
                    .child(account),
            )
    }

    fn list(&self, cx: &Context<Self>) -> impl IntoElement {
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
                .flex_1()
                .items_center()
                .justify_center()
                .text_size(px(15.0))
                .text_color(rgb(ON_VARIANT))
                .child(message);
        }

        let count = self.view.rows.len();
        let scroll = self.scroll.clone();
        div().flex().flex_1().child(
            uniform_list("library", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
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
                    items.push(
                        div()
                            .id(index)
                            .flex()
                            .items_center()
                            .h(px(48.0))
                            .px_4()
                            .gap_3()
                            .cursor_pointer()
                            .when(selected, |this| this.bg(rgb(SURFACE_HIGH)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select(index);
                                cx.notify();
                            }))
                            .child(art(cover, 36.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(14.0))
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(ON_VARIANT))
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .child(subtitle),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
                                    .child(meta),
                            )
                            .when(starred, |this| {
                                this.child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(0xFFD54F))
                                        .child("★"),
                                )
                            })
                            .when(downloaded, |this| {
                                this.child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(PRIMARY))
                                        .child("↓"),
                                )
                            }),
                    );
                }
                items
            }))
            .h_full()
            .track_scroll(scroll),
        )
    }

    fn search_overlay(&self, search: &SearchState) -> gpui::Div {
        let query = if search.query.is_empty() {
            "Search…".to_string()
        } else {
            format!("{}▌", search.query)
        };
        let results = search
            .rows
            .iter()
            .take(12)
            .enumerate()
            .map(|(index, row)| {
                let selected = index == search.cursor;
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .when(selected, |this| this.bg(rgb(SURFACE_HIGH)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .flex_1()
                            .child(div().text_size(px(14.0)).child(row.title.clone()))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(ON_VARIANT))
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
            .flex()
            .justify_center()
            .items_start()
            .pt(px(72.0))
            .child(
                div()
                    .w(px(720.0))
                    .max_h(px(460.0))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(OUTLINE))
                    .bg(rgb(SURFACE))
                    .shadow_lg()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .text_size(px(18.0))
                            .text_color(if search.query.is_empty() { rgb(ON_VARIANT) } else { rgb(ON_SURFACE) })
                            .child(query),
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
                            .px_4()
                            .py_2()
                            .text_size(px(10.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(if search.loading {
                                "searching…"
                            } else {
                                "Enter opens · type to refine · Esc closes"
                            }),
                    ),
            )
    }

    fn page_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(page) = self.page.as_ref() else {
            return div().flex().flex_1();
        };
        let title = format!("{} · {}", page.descriptor.kind.label(), page.header.title);
        let subtitle = page.header.subtitle.clone();
        let meta = page.header.meta.clone();
        let place = format!(
            "{} / {} · q back · [ ] travel",
            self.travel.position.saturating_add(1),
            self.travel.places.len().max(1)
        );

        let hero = div()
            .flex()
            .items_center()
            .gap_5()
            .h(px(180.0))
            .px_6()
            .bg(rgb(SURFACE))
            .child(art(cover_path(&page.header.cover_url), 116.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .flex_1()
                    .child(div().text_size(px(20.0)).child(title))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(subtitle),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(ON_VARIANT))
                            .child(meta),
                    ),
            )
            .child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(ON_VARIANT))
                    .child(place),
            );

        if page.loading && page.rows.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .child(hero)
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
                .child(hero)
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
            .flex()
            .flex_col()
            .flex_1()
            .child(hero)
            .child(
                uniform_list("page-rows", count, cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    for index in range {
                        let Some(page) = this.page.as_ref() else { break };
                        let row = &page.rows[index];
                        let title = row.title.clone();
                        let subtitle = row.subtitle.clone();
                        let meta = row.meta.clone();
                        let downloaded = row.downloaded;
                        let selected = index == page.cursor
                            || page.selection.contains(page.cursor, index);
                        items.push(
                            div()
                                .id(index)
                                .flex()
                                .items_center()
                                .h(px(44.0))
                                .px_4()
                                .gap_3()
                                .cursor_pointer()
                                .when(selected, |this| this.bg(rgb(SURFACE_HIGH)))
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
                                        .child(div().text_size(px(14.0)).child(title))
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
                                )
                                .when(downloaded, |this| {
                                    this.child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(PRIMARY))
                                            .child("↓"),
                                    )
                                }),
                        );
                    }
                    items
                }))
                .h_full()
                .track_scroll(scroll),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = self.search.as_ref().map(|search| self.search_overlay(search));
        let top = self.top_bar(&*cx);
        let body = if self.page.is_some() {
            self.page_view(&*cx).into_any_element()
        } else {
            self.list(&*cx).into_any_element()
        };
        let status = self.status_bar();
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
            .child(div().when_some(overlay, |this, overlay| this.child(overlay)))
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
        Some(path) => img(path).size(px(size)).rounded_md().into_any_element(),
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
