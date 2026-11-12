//! List virtualization comparison harness.
//!
//! Run one implementation at a time; the demo HUD is the same QML-derived
//! frame counter used by the app:
//!
//!   cargo run --release --example list_bench -- 100000
//!
//! Keys: 1 plain scroll, 2 uniform_list, 3 ListState, 4 uniform + cached row.

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    AnyView, App, Application, Bounds, Context, FocusHandle, IntoElement, ListAlignment,
    ListOffset, ListState, Render, ScrollHandle, ScrollStrategy, UniformListScrollHandle, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size, uniform_list,
};

#[path = "../src/ui/debug.rs"]
mod fps;

const ROW_H: f32 = 56.0;
const MAX_ROWS: usize = 100_000;

#[derive(Clone)]
struct BenchRow {
    title: Rc<str>,
    subtitle: Rc<str>,
    meta: Rc<str>,
    color: u32,
}

struct BenchRowView {
    row: BenchRow,
    height: f32,
}

impl Render for BenchRowView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        row_element(&self.row, self.height)
    }
}

fn make_rows(count: usize) -> Rc<Vec<BenchRow>> {
    Rc::new(
        (0..count)
            .map(|index| {
                let title: Rc<str> = format!("Track {index:06} — a moderately long title").into();
                let subtitle: Rc<str> =
                    format!("Artist {} · Album {}", index % 9_000, index % 700).into();
                let meta: Rc<str> = format!("{}:{:02}", (index / 60) % 10, index % 60).into();
                BenchRow {
                    title,
                    subtitle,
                    meta,
                    color: 0x202020 + ((index as u32 * 97) & 0x7f7f7f),
                }
            })
            .collect(),
    )
}

fn row_height(index: usize, variable: bool) -> f32 {
    if variable {
        ROW_H + (index % 3) as f32 * 10.0
    } else {
        ROW_H
    }
}

fn row_element(row: &BenchRow, height: f32) -> impl IntoElement {
    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .rounded_sm()
        .child(div().size(px(40.0)).rounded_sm().bg(rgb(row.color)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(14.0))
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(row.title.to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x888888))
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(row.subtitle.to_string()),
                ),
        )
        .child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(0x888888))
                .child(row.meta.to_string()),
        )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Uniform,
    Variable,
    Cached,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Plain => "1 Plain scroll",
            Self::Uniform => "2 uniform_list",
            Self::Variable => "3 ListState",
            Self::Cached => "4 uniform + cached",
        }
    }
}

struct Bench {
    rows: Rc<Vec<BenchRow>>,
    mode: Mode,
    plain_scroll: ScrollHandle,
    uniform_scroll: UniformListScrollHandle,
    variable: ListState,
    focus: FocusHandle,
    auto_scroll: bool,
    scroll_index: usize,
    hud: fps::FrameHud,
    last_frame: Option<Instant>,
}

impl Bench {
    fn new(rows: usize, cx: &Context<Self>) -> Self {
        let rows = make_rows(rows);
        Self {
            mode: Mode::Uniform,
            rows: rows.clone(),
            plain_scroll: ScrollHandle::new(),
            uniform_scroll: UniformListScrollHandle::new(),
            variable: ListState::new(rows.len(), ListAlignment::Top, px(500.0)),
            focus: cx.focus_handle(),
            auto_scroll: false,
            scroll_index: 0,
            hud: fps::FrameHud::default(),
            last_frame: None,
        }
    }
}

impl Render for Bench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if let Some(last) = self.last_frame.replace(now) {
            self.hud.observe(now.duration_since(last).as_secs_f32());
        }
        self.focus.focus(_window);
        if self.auto_scroll {
            _window.request_animation_frame();
            let count = self.rows.len();
            self.scroll_index = (self.scroll_index + 2) % count;
            match self.mode {
                Mode::Plain => self.plain_scroll.scroll_to_item(self.scroll_index),
                Mode::Uniform | Mode::Cached => self
                    .uniform_scroll
                    .scroll_to_item(self.scroll_index, ScrollStrategy::Top),
                Mode::Variable => self.variable.scroll_to(ListOffset {
                    item_ix: self.scroll_index,
                    offset_in_item: px(0.0),
                }),
            }
        }

        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .bg(rgb(0x111111))
            .text_size(px(12.0))
            .child(format!("{} rows", self.rows.len()))
            .child(if self.auto_scroll {
                "auto: on"
            } else {
                "auto: off"
            })
            .child(format!("at {}", self.scroll_index))
            .children(Mode::all().into_iter().map(|mode| {
                div()
                    .id(("mode", mode as usize))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(if mode == self.mode {
                        rgb(0x443366)
                    } else {
                        rgb(0x222222)
                    })
                    .child(mode.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.mode = mode;
                        cx.notify();
                    }))
            }))
            .child(
                div()
                    .id("auto-scroll")
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(if self.auto_scroll {
                        rgb(0x225522)
                    } else {
                        rgb(0x222222)
                    })
                    .child("Auto scroll")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.auto_scroll = !this.auto_scroll;
                        cx.notify();
                    })),
            );

        let rows = self.rows.clone();
        let uniform_scroll = self.uniform_scroll.clone();
        let content = match self.mode {
            Mode::Plain => div()
                .id("plain-list")
                .overflow_y_scroll()
                .track_scroll(&self.plain_scroll)
                .children(rows.iter().map(|row| row_element(row, ROW_H)))
                .into_any_element(),
            Mode::Uniform => uniform_list(
                "bench-uniform",
                self.rows.len(),
                cx.processor(move |_this, range: std::ops::Range<usize>, _, _| {
                    range
                        .filter_map(|index| {
                            rows.get(index)
                                .map(|row| row_element(row, ROW_H).into_any_element())
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .h_full()
            .track_scroll(uniform_scroll)
            .into_any_element(),
            Mode::Variable => {
                let rows = self.rows.clone();
                gpui::list(self.variable.clone(), move |index, _, _| {
                    rows.get(index)
                        .map(|row| row_element(row, row_height(index, true)).into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                })
                .h_full()
                .into_any_element()
            }
            Mode::Cached => {
                // A genuinely cached 100k-row tree would defeat the memory
                // point. This mode caches only the rows materialized by the
                // uniform list's visible range.
                let cache = Rc::new(std::cell::RefCell::new(std::collections::HashMap::<
                    usize,
                    AnyView,
                >::new()));
                let rows = self.rows.clone();
                uniform_list(
                    "bench-cached",
                    self.rows.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                        let mut items = Vec::with_capacity(range.len());
                        for index in range {
                            let mut cache = cache.borrow_mut();
                            let view = cache.entry(index).or_insert_with(|| {
                                let row =
                                    rows.get(index).cloned().unwrap_or_else(|| rows[0].clone());
                                AnyView::from(cx.new(|_| BenchRowView { row, height: ROW_H }))
                                    .cached(Default::default())
                            });
                            items.push(view.clone());
                        }
                        let _ = this;
                        let _ = window;
                        items
                    }),
                )
                .h_full()
                .track_scroll(self.uniform_scroll.clone())
                .into_any_element()
            }
        };

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                let mode = match event.keystroke.key.as_str() {
                    "1" => Mode::Plain,
                    "2" => Mode::Uniform,
                    "3" => Mode::Variable,
                    "4" => Mode::Cached,
                    "a" | "space" => {
                        this.auto_scroll = !this.auto_scroll;
                        cx.notify();
                        return;
                    }
                    _ => return,
                };
                this.mode = mode;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0b0b0b))
            .text_color(gpui::white())
            .font_family("Overused Grotesk Roman")
            .child(header)
            .child(div().flex_1().min_h_0().child(content))
            .child(self.hud.view())
    }
}

impl Mode {
    fn all() -> [Self; 4] {
        [Self::Plain, Self::Uniform, Self::Variable, Self::Cached]
    }
}

fn main() {
    let rows = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(MAX_ROWS);
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| Bench::new(rows, cx)),
        )
        .unwrap_or_else(|error| panic!("cannot open the benchmark window: {error}"));
    });
}
