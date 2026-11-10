//! Description, toast, exit, Alt-board and prompt-bar overlays.

use super::super::text_modal::TextBoxStyle;
use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn description_overlay(
        &self,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
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
                // The reader owns the wheel: the page behind it must not
                // scroll while the note body is being read.
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.stop_propagation()))
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
                                .min_h_0()
                                .w_full()
                                .overflow_y_scroll()
                                .track_scroll(&self.description_scroll)
                                .text_size(px(14.0))
                                .text_color(rgb(palette::on_surface()))
                                .whitespace_normal()
                                .child(description.body.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(rgb(palette::on_surface_variant()))
                                .child("j/k scroll · Ctrl-D/U page · gg/G ends · Esc/Q close"),
                        ),
                ),
        )
        .map(|overlay| overlay.into_any_element())
    }

    pub(in crate::ui) fn alt_overlay(&self, progress: f32) -> gpui::AnyElement {
        let (Some(menu), Some((ax, ay))) = (self.alt_menu.as_ref(), self.alt_anchor) else {
            return div().into_any_element();
        };
        let page_h = (self.win_h - 120.0).max(1.0);
        let option_scale =
            (0.816 * ((self.win_w - 20.0) / 1280.0).min((page_h - 20.0) / 720.0)).clamp(0.5, 0.816);
        let spawned = 0.72 + 0.28 * progress;
        let box_side = page_h * 0.07 * option_scale * spawned;
        let frame_side = (box_side * 8.0).min(self.win_w - 20.0).min(page_h - 20.0);
        // The board is anchored at the focused item but may slide inside the
        // safe window area: topbar/status anchors often sit too close to an
        // edge for a full-size board to stay centred there.
        let wall_gap = 10.0;
        let board_x = (ax - frame_side * 0.5)
            .clamp(wall_gap, (self.win_w - frame_side - wall_gap).max(wall_gap));
        let board_y =
            (ay - frame_side * 0.5).clamp(wall_gap, (page_h - frame_side + wall_gap).max(wall_gap));
        let mut board = div()
            .absolute()
            .left(px(board_x))
            .top(px(board_y))
            .w(px(frame_side))
            .h(px(frame_side))
            .opacity(progress);
        let center = (frame_side - box_side) * 0.5;
        for direction in Direction::ALL {
            let Some(slot) = menu.slot(direction) else {
                continue;
            };
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
                .border_color(rgb(if aimed {
                    palette::primary()
                } else {
                    palette::outline_variant()
                }))
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

    pub(in crate::ui) fn line_search_denied_opacity(&self) -> f32 {
        anim::denied_opacity(self.line_search_denied_at, self.clock.now)
    }

    pub(in crate::ui) fn line_search_field(
        &self,
        window: &gpui::Window,
        bar_h: f32,
        width: f32,
    ) -> gpui::AnyElement {
        self.line_search.editor.render(
            window,
            TextBoxStyle {
                placeholder: "Search this surface",
                text_size: bar_h * 0.18,
                caret_h: bar_h * 0.28,
                max_width: width,
                caret: true,
                mask: false,
            },
        )
    }

    pub(in crate::ui) fn line_search_overlay(
        &self,
        window: &gpui::Window,
        progress: f32,
    ) -> gpui::Div {
        let counter = match (
            self.line_search.current_hit(),
            self.line_search.candidates.len(),
        ) {
            (Some(_), count) if count > 0 => {
                format!("[{}/{}]", self.line_search.index.unwrap_or(0) + 1, count)
            }
            _ => String::new(),
        };
        let bar_h = (self.win_h * 0.07).max(32.0);
        let field_w = (self.win_w * 0.738).max(1.0);
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
                    .child(self.line_search_field(window, bar_h, field_w))
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

    pub(in crate::ui) fn command_line_field(
        &self,
        window: &gpui::Window,
        bar_h: f32,
        width: f32,
    ) -> gpui::AnyElement {
        self.command_line.editor.render(
            window,
            TextBoxStyle {
                placeholder: "Type a command…",
                text_size: bar_h * 0.18,
                caret_h: bar_h * 0.28,
                max_width: width,
                caret: true,
                mask: false,
            },
        )
    }

    pub(in crate::ui) fn command_line_overlay(
        &self,
        window: &gpui::Window,
        progress: f32,
    ) -> gpui::Div {
        let bar_h = (self.win_h * 0.07).max(32.0);
        let field_w = (self.win_w * 0.55).max(1.0);
        let row_h = (bar_h * 0.44).max(18.0);
        let suggestions: Vec<_> = self
            .command_line
            .visible_suggestions()
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
                    .child(self.command_line_field(window, bar_h, field_w))
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

    pub(in crate::ui) fn exit_overlay(&self, progress: f32) -> gpui::Div {
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

    pub(in crate::ui) fn toast_overlay(&self, cx: &Context<Self>) -> gpui::Div {
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
                self.toast_bubble(&toast.message, opacity, toast.retry, cx)
            }))
            .when_some(self.hold_toast.as_ref(), |this, hold| {
                this.child(self.toast_bubble(&hold.message, 1.0, None, cx))
            })
    }

    /// One toast card. Shared by the queued toasts and the held-key counter so
    /// both read as the same surface.
    fn toast_bubble(
        &self,
        message: &str,
        opacity: f32,
        retry: Option<RetryAction>,
        cx: &Context<Self>,
    ) -> gpui::Div {
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
                    .child(message.to_string()),
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
    }

    pub(in crate::ui) fn login_view(&self, window: &gpui::Window) -> gpui::Div {
        let login = self.login.as_ref().expect("login view");
        let fields = [
            ("Server URL", &login.server, 0, false),
            ("Username", &login.username, 1, false),
            ("Password", &login.password, 2, true),
        ];
        let field_w = (self.win_w * 0.224 - 26.0).max(1.0);
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
                    .children(fields.into_iter().map(|(label, input, index, mask)| {
                        let active = login.field == index;
                        div()
                            .h(px(52.0))
                            .px_3()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .overflow_hidden()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if active {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .bg(rgb(palette::surface_container_highest()))
                            .child(input.render(
                                window,
                                TextBoxStyle {
                                    placeholder: label,
                                    text_size: 14.0,
                                    caret_h: 20.0,
                                    max_width: field_w,
                                    caret: active,
                                    mask,
                                },
                            ))
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
                            .child(if login.busy {
                                "Working…"
                            } else if login.mode == LoginMode::AddAccount {
                                "Add account"
                            } else {
                                "Log in"
                            }),
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
                            .when(login.error.is_some(), |this| {
                                this.border_1()
                                    .border_color(rgb(palette::error()))
                                    .bg(rgb(palette::error_container()))
                            })
                            .text_size(px(11.0))
                            .text_color(rgb(if login.error.is_some() {
                                palette::on_error_container()
                            } else {
                                palette::on_surface_variant()
                            }))
                            .text_center()
                            .whitespace_normal()
                            .child(login.error.clone().unwrap_or_else(|| {
                                if login.mode == LoginMode::AddAccount {
                                    "Tab moves · Enter adds account · Esc cancels".to_string()
                                } else if login.offline_available {
                                    "Tab moves · Enter logs in · F2 offline".to_string()
                                } else {
                                    "Tab moves · Enter logs in".to_string()
                                }
                            })),
                    ),
            )
    }
}
