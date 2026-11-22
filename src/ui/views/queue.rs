//! Queue drawer view.

use super::super::*;

/// `AppRoot::hovered_row` is one plain `Option<usize>` shared by every list
/// surface, so raw paint indices bleed between queue/search/library. Queue
/// indices are tagged into the top bits; library rows keep plain indices and
/// can therefore never equal a queue hover (and vice versa). The tag is local
/// to this module: every comparison against it happens here.
const QUEUE_HOVER_TAG: usize = 1 << (usize::BITS - 4);

fn queue_hover_id(index: usize) -> usize {
    QUEUE_HOVER_TAG | index
}

/// Drawer header position label. An empty queue must read `0 / 0`; the plain
/// `current + 1` formula painted the off-by-one `1 / 0`.
fn queue_position_label(current: usize, count: usize) -> String {
    let position = if count == 0 {
        0
    } else {
        current.saturating_add(1)
    };
    format!("{position} / {count}")
}

impl AppRoot {
    pub(in crate::ui) fn queue_overlay(
        &self,
        cx: &Context<Self>,
        progress: f32,
    ) -> gpui::AnyElement {
        let count = self.player.len();
        let current = self.player.index();
        let cursor = self.queue_list.cursor;
        let scroll = self.queue_scroll_modal.handle.clone();
        let position = queue_position_label(current, count);
        div()
            .id("queue-backdrop")
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .justify_end()
            .bg(gpui::black().opacity(0.35 * progress))
            .opacity(progress)
            // The drawer is modal: neither clicks nor wheel deltas may reach
            // the page under the scrim.
            .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.stop_propagation()))
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
                                    .child(format!("{position} / {count}")),
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
                                        let cover = cover_source_with_remote(
                                            &track.cover_url,
                                            &track.cover_remote,
                                        )
                                        .map(std::sync::Arc::new);
                                        let active = index == cursor;
                                        let selected = active
                                            || this.queue_list.selection.contains(cursor, index)
                                            || this
                                                .queue_list
                                                .selection
                                                .contains(this.queue_list.cursor, index);
                                        let playing = index == this.player.index();
                                        let pending =
                                            this.marks.pending(&track.id, &track.album_id);
                                        let downloaded =
                                            this.marks.downloaded(&track.id, &track.album_id);
                                        let hovered =
                                            this.hovered_row == Some(queue_hover_id(index));
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
                                                            this.hovered_row =
                                                                Some(queue_hover_id(index));
                                                        } else if this.hovered_row
                                                            == Some(queue_hover_id(index))
                                                        {
                                                            this.hovered_row = None;
                                                        }
                                                        cx.notify();
                                                    },
                                                ))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if !this.visual_mode {
                                                        this.queue_list.selection.clear();
                                                    }
                                                    this.queue_list.cursor = index;
                                                    this.player.jump(index);
                                                    cx.notify();
                                                }))
                                                .child(CoverImage::new(
                                                    "retained-cover-queue",
                                                    index,
                                                    cover,
                                                    this.image_cache.clone(),
                                                    this.queue_scroll_modal.row_h
                                                        * OVERLAY_ROW_COVER_RATIO,
                                                    this.queue_scroll_modal.row_h
                                                        * OVERLAY_ROW_COVER_RATIO,
                                                    cover_scale,
                                                    gpui::ObjectFit::Contain,
                                                    4.0,
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
                                                )
                                                .children(row_mark_icons(
                                                    index,
                                                    "retained-queue-download-pending",
                                                    "retained-queue-download",
                                                    "retained-queue-favorite",
                                                    RowMarks {
                                                        downloaded,
                                                        downloading: pending,
                                                        favorite: track.favorite,
                                                    },
                                                )),
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
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{QUEUE_HOVER_TAG, queue_hover_id, queue_position_label};

    #[test]
    fn empty_queue_header_reads_zero_of_zero() {
        assert_eq!(queue_position_label(0, 0), "0 / 0");
    }

    #[test]
    fn queue_header_is_one_based_for_a_playing_row() {
        assert_eq!(queue_position_label(0, 3), "1 / 3");
        assert_eq!(queue_position_label(2, 3), "3 / 3");
    }

    #[test]
    fn queue_hover_ids_cannot_collide_with_plain_paint_indices() {
        for index in [0, 1, 17, 9999] {
            assert_ne!(queue_hover_id(index), index);
        }
        assert_eq!(queue_hover_id(0), QUEUE_HOVER_TAG);
        assert_ne!(queue_hover_id(0), queue_hover_id(1));
    }
}
