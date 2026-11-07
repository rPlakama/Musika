//! Queue drawer view.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn queue_overlay(&self, cx: &Context<Self>, progress: f32) -> gpui::Div {
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
                        uniform_list(
                            "queue",
                            count,
                            cx.processor(
                                move |this, range: std::ops::Range<usize>, _window, cx| {
                                    let queue = this.player.queue_snapshot();
                                    let mut items = Vec::with_capacity(range.len());
                                    for index in range {
                                        let Some(track) = queue.get(index) else { break };
                                        let title = track.title.clone();
                                        let artist = track.artist.clone();
                                        let cover = cover_source(&track.cover_url);
                                        let active = index == cursor;
                                        let selected = this.selected_id.as_deref()
                                            == Some(track.id.as_str())
                                            || active
                                            || this.queue_selection.contains(cursor, index)
                                            || this
                                                .queue_selection
                                                .contains(this.queue_cursor, index);
                                        let playing = index == this.player.index();
                                        let hovered = this.hovered_row == Some(index);
                                        let active = selected || hovered;
                                        let (scale, text_scale) =
                                            this.row_emphasis(LineSurface::Queue, 0, index, active);
                                        let emphasis = this.visuals.row_text_emphasis(text_scale);
                                        let cover_scale = this.visuals.row_cover_scale(scale);
                                        let fonts = this.visuals.fonts;
                                        items.push(
                                            div()
                                                .id(gpui::ElementId::named_usize(
                                                    if selected {
                                                        "queue-row-selected"
                                                    } else {
                                                        "queue-row"
                                                    },
                                                    index,
                                                ))
                                                .flex()
                                                .w_full()
                                                .items_center()
                                                .gap_2()
                                                .h(px(this.queue_scroll_modal.row_h))
                                                .px_3()
                                                .cursor_pointer()
                                                .when(selected, |this| {
                                                    this.bg(rgb(palette::secondary_container()))
                                                })
                                                .when(hovered && !selected, |this| {
                                                    this.bg(rgb(palette::surface_container_high()))
                                                })
                                                .on_hover(cx.listener(
                                                    move |this, hovered: &bool, _, cx| {
                                                        if *hovered {
                                                            this.hovered_row = Some(index);
                                                        } else if this.hovered_row == Some(index) {
                                                            this.hovered_row = None;
                                                        }
                                                        cx.notify();
                                                    },
                                                ))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if !this.visual_mode {
                                                        this.queue_selection.clear();
                                                    }
                                                    this.queue_cursor = index;
                                                    this.player.jump(index);
                                                    cx.notify();
                                                }))
                                                .child(RetainedCover::new(
                                                    "retained-cover-queue",
                                                    index,
                                                    cover,
                                                    this.image_cache.clone(),
                                                    this.queue_scroll_modal.row_h * 0.714,
                                                    cover_scale,
                                                ))
                                                .child(
                                                    div()
                                                        .flex()
                                                        .flex_col()
                                                        .min_w_0()
                                                        .flex_1()
                                                        .h(px(this.queue_scroll_modal.row_h))
                                                        .child(RetainedText::new(
                                                            "retained-text-queue",
                                                            index,
                                                            title.clone().into(),
                                                            artist.clone().into(),
                                                            this.line_search
                                                                .query()
                                                                .to_string()
                                                                .into(),
                                                            this.line_search_current(
                                                                LineSurface::Queue,
                                                                index,
                                                            ),
                                                            active,
                                                            fonts.queue_title * emphasis,
                                                            fonts.queue_meta * emphasis,
                                                            this.queue_scroll_modal.row_h,
                                                            if playing {
                                                                palette::primary()
                                                            } else {
                                                                palette::on_surface()
                                                            },
                                                            palette::on_surface_variant(),
                                                            true,
                                                        )),
                                                ),
                                        );
                                    }
                                    items
                                },
                            ),
                        )
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
                            .child(
                                "Enter jumps · d d removes · Ctrl-Shift-J/K reorders · q closes",
                            ),
                    ),
            )
    }
}
