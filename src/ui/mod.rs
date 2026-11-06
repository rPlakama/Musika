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

//! GPUI front end (M0 retarget).
//!
//! Target is [GPUI](https://gpui.rs), not egui: a hybrid immediate/retained,
//! CSS-like element tree with entities and actions. This M0 slice proves the
//! application/window, font fallback, shell layout and clean shutdown; the
//! QML abstractions in `docs/mds/gpuui/` are retargeted in `09-gpui-retarget.md`.

use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Render, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size,
};

actions!(musika, [Quit]);

pub fn run() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("q", Quit, None),
            KeyBinding::new("escape", Quit, None),
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
        .expect("GPUUI: could not open the application window");
    });
}

struct AppRoot {
    focus: FocusHandle,
}

impl AppRoot {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        Self { focus }
    }
}

impl Render for AppRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_action(|_: &Quit, window, _cx| window.remove_window())
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x141218))
            .font_family("Overused Grotesk")
            .text_color(rgb(0xE6E0E9))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(52.0))
                    .px_5()
                    .bg(rgb(0x211F26))
                    .child(div().text_size(px(18.0)).child("Musika"))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xCAC4D0))
                            .child("GPUI M0 · sign-in/list bridge next"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(div().text_size(px(20.0)).child("GPUI port initialized"))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(rgb(0xCAC4D0))
                                    .child("q / Esc quits · shell/list/pages next"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(66.0))
                    .bg(rgb(0x211F26))
                    .text_size(px(11.0))
                    .text_color(rgb(0xCAC4D0))
                    .child("GPUI · frame/effects/motion port follows"),
            )
    }
}
