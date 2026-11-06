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
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size, uniform_list,
    UniformListScrollHandle,
};

use bridge::Event;
use state::{Scope, ViewState};

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
            KeyBinding::new("q", Quit, None),
            KeyBinding::new("escape", Quit, None),
            KeyBinding::new("j", MoveDown, None),
            KeyBinding::new("k", MoveUp, None),
            KeyBinding::new("g", Top, None),
            KeyBinding::new("shift-g", Bottom, None),
            KeyBinding::new("shift-j", ScopeNext, None),
            KeyBinding::new("shift-k", ScopePrev, None),
            KeyBinding::new("shift-l", OrderNext, None),
            KeyBinding::new("shift-h", OrderPrev, None),
            KeyBinding::new("r", Refresh, None),
            KeyBinding::new("enter", Enter, None),
        ]);
        cx.on_window_closed(|cx| {
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

struct AppRoot {
    focus: FocusHandle,
    bridge: bridge::Bridge,
    view: ViewState,
    header: Option<Header>,
    error: Option<String>,
    generation: u64,
    scroll: UniformListScrollHandle,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);

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

        Self {
            focus,
            bridge,
            view,
            header: None,
            error: None,
            generation: 0,
            scroll: UniformListScrollHandle::new(),
        }
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
                self.view.cursor = self.view.cursor.min(self.view.rows.len().saturating_sub(1));
                self.scroll
                    .scroll_to_item(self.view.cursor, ScrollStrategy::Center);
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
        self.view.clear_for(scope);
        self.reload();
    }

    fn select(&mut self, index: usize) {
        self.view.select(index);
        self.scroll
            .scroll_to_item(self.view.cursor, ScrollStrategy::Center);
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, _cx: &mut Context<Self>) {
        window.remove_window();
    }

    fn on_move_down(&mut self, _: &MoveDown, _window: &mut Window, cx: &mut Context<Self>) {
        self.select(self.view.cursor.saturating_add(1));
        cx.notify();
    }

    fn on_move_up(&mut self, _: &MoveUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.select(self.view.cursor.saturating_sub(1));
        cx.notify();
    }

    fn on_top(&mut self, _: &Top, _window: &mut Window, cx: &mut Context<Self>) {
        self.select(0);
        cx.notify();
    }

    fn on_bottom(&mut self, _: &Bottom, _window: &mut Window, cx: &mut Context<Self>) {
        self.select(self.view.rows.len().saturating_sub(1));
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

    fn on_enter(&mut self, _: &Enter, _window: &mut Window, _cx: &mut Context<Self>) {
        // Pages land in G1's next increment.
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
                    let selected = index == this.view.cursor;
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
            .child(format!(
                "{} · {} · {} artists / {} albums / {} songs",
                self.view.scope.label(),
                self.view.order_label(),
                counts.0,
                counts.1,
                counts.2
            ))
    }
}

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top = self.top_bar(&*cx);
        let list = self.list(&*cx);
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
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .font_family("Overused Grotesk")
            .text_color(rgb(ON_SURFACE))
            .child(top)
            .child(list)
            .child(status)
    }
}
