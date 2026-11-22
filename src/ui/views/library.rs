//! Library list, rows and selected-row rendering.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn list(&self, cx: &Context<Self>, window: &Window) -> impl IntoElement {
        let title = self.view.scope.label();
        let hint = "(Shift-J/K) cycle filter";
        let order = format!("Shift-H / Shift-L  /  {}", self.view.order_label());
        let selection_count = self.view.list.selection.count(self.view.list.cursor);
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .w_full()
            .h(px(72.0))
            .px_2()
            .child(
                div()
                    .text_size(px(26.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(hint),
            )
            .when(self.connection_offline, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .bg(rgb(palette::tertiary_container()))
                        .text_size(px(11.0))
                        .text_color(rgb(palette::on_tertiary_container()))
                        .child("Offline · downloads only"),
                )
            })
            .child(div().flex_1())
            .child(icon(
                "outline-shuffle.svg",
                14.0,
                palette::on_surface_variant(),
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(order),
            )
            .when(selection_count > 0, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_full()
                        .bg(rgb(palette::primary()))
                        .text_size(px(11.0))
                        .text_color(rgb(palette::on_primary()))
                        .child(selection_count.to_string()),
                )
            });

        if self.view.rows.is_empty() {
            if self.view.loading {
                perf::request_frame(window, "library-loading");
                return div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .px(px(12.0))
                    .child(header)
                    .child(loading_skeleton(self.library_scroll.row_h, 8));
            }
            let message = if let Some(error) = &self.error {
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
                        .text_color(rgb(palette::on_surface_variant()))
                        .child(message),
                );
        }

        let count = self.view.rows.len();
        let scroll = self.library_scroll.handle.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .px(px(12.0))
            .child(header)
            .child(
                uniform_list(
                    "library",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                        let row_h = this.library_scroll.row_h;
                        let line_query: gpui::SharedString =
                            this.line_search.query().to_string().into();
                        let mut items = Vec::with_capacity(range.len());
                        for index in range {
                            let selected = index == this.view.list.cursor
                                || this
                                    .view
                                    .list
                                    .selection
                                    .contains(this.view.list.cursor, index);
                            let hovered = this.hovered_row == Some(index);
                            let Some(row) = this.view.rows.get(index) else {
                                // RowWindow blank slot: a waving grey placeholder
                                // until the live window lands.
                                perf::request_frame(window, "library-window-hole");
                                items.push(
                                    div()
                                        .id(index)
                                        .w_full()
                                        .flex()
                                        .items_center()
                                        .h(px(row_h))
                                        .px_1()
                                        .rounded_sm()
                                        .bg(rgb(if selected {
                                            palette::secondary_container()
                                        } else {
                                            palette::surface()
                                        }))
                                        .child(loading_placeholder(index, row_h)),
                                );
                                continue;
                            };
                            let (scale, text_scale) = this.row_emphasis(
                                LineSurface::Library,
                                this.view.scope as usize,
                                index,
                                selected,
                            );
                            let cover_scale = this.visuals.row_cover_scale(scale);
                            let emphasis = this.visuals.row_text_emphasis(text_scale);
                            let hover_fill = row_hover_fill(
                                &this.library_hover_fill,
                                &this.clock,
                                index,
                                selected,
                                hovered,
                            );
                            items.push(
                                div()
                                    .id(gpui::ElementId::named_usize(
                                        if selected {
                                            "library-row-selected"
                                        } else {
                                            "library-row"
                                        },
                                        index,
                                    ))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .h(px(row_h))
                                    .px_1()
                                    .rounded_sm()
                                    .bg(row_bg_color(selected, hover_fill))
                                    .cursor_pointer()
                                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                        target_row_hover_fill(
                                            &mut this.library_hover_fill,
                                            &this.clock,
                                            index,
                                            selected,
                                            *hovered,
                                        );
                                        if *hovered {
                                            this.hovered_row = Some(index);
                                        } else if this.hovered_row == Some(index) {
                                            this.hovered_row = None;
                                        }
                                        cx.notify();
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select(index);
                                        cx.notify();
                                    }))
                                    .child(this.row_content(
                                        row,
                                        this.view.cover(index),
                                        &line_query,
                                        RowPaint {
                                            flags: LIBRARY_ROW_FLAGS,
                                            surface: LineSurface::Library,
                                            index,
                                            selected,
                                            height: row_h,
                                            cover_scale,
                                            emphasis,
                                        },
                                    )),
                            );
                        }
                        items
                    }),
                )
                .pt(px(8.0))
                .pb(px(8.0))
                .h_full()
                .w_full()
                .on_scroll_wheel(cx.listener(|this, _, _, _| {
                    this.library_scroll.on_wheel();
                }))
                .track_scroll(scroll),
            )
    }

    /// One row body for every list surface. The data lives in `Row`; the
    /// surface supplies the flags that decide which cells are materialized.
    /// The cover is resolved by the row window/page payload, and `query` is
    /// hoisted to one `SharedString` per frame (not one per row).
    pub(in crate::ui) fn row_content(
        &self,
        row: &state::Row,
        cover: Option<&std::sync::Arc<CoverSource>>,
        query: &gpui::SharedString,
        paint: RowPaint,
    ) -> gpui::Div {
        let emphasis = paint.emphasis;
        let fonts = self.visuals.fonts;
        let current = self.line_search_current(paint.surface, paint.index);
        let mut body = div().flex().items_center().min_w_0().flex_1();
        if paint.flags.show_cover {
            let id_name = match paint.surface {
                LineSurface::Library => "retained-cover-library",
                LineSurface::Page => "retained-cover-page",
                LineSurface::Search
                | LineSurface::Queue
                | LineSurface::Lyrics
                | LineSurface::Status => "retained-cover",
            };
            let cover_side = paint.height * 0.78;
            body = body.child(div().mr_3().child(CoverImage::new(
                id_name,
                paint.index,
                cover.cloned(),
                self.image_cache.clone(),
                cover_side,
                cover_side,
                paint.cover_scale,
                gpui::ObjectFit::Contain,
                4.0,
            )));
        }
        let id_name = match paint.surface {
            LineSurface::Library => "retained-text-library",
            LineSurface::Page => "retained-text-page",
            LineSurface::Search
            | LineSurface::Queue
            | LineSurface::Lyrics
            | LineSurface::Status => "retained-text",
        };
        let cells = div()
            .flex()
            .flex_col()
            .min_w_0()
            .flex_1()
            .overflow_hidden()
            .child(div().h(px(paint.height)).w_full().child(RetainedText::new(
                id_name,
                paint.index,
                row.title.clone().into(),
                row.subtitle.clone().into(),
                query.clone(),
                current,
                paint.selected,
                fonts.row_title(paint.flags.compact) * emphasis,
                fonts.row_subtitle(paint.flags.compact) * emphasis,
                paint.height,
                palette::on_surface(),
                palette::on_surface_variant(),
                paint.flags.compact,
            )));
        body = body.child(cells);
        let downloaded = self.marks.downloaded(&row.song_id, &row.album_id);
        let downloading = self.marks.pending_row(row);
        let mut trailing = div().flex().items_center().gap_2();
        if paint.flags.show_length && !row.meta.is_empty() {
            trailing = trailing.child(
                div()
                    .text_size(px(fonts.row_meta * emphasis))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(highlighted_text(
                        &row.meta,
                        self.line_search.query(),
                        palette::on_surface_variant(),
                        current,
                    )),
            );
        }
        for mark in row_mark_icons(
            paint.index,
            "retained-download-pending",
            "retained-download-mark",
            "retained-star-mark",
            RowMarks {
                downloaded,
                downloading,
                favorite: row.starred,
            },
        ) {
            trailing = trailing.child(mark);
        }
        if row.rating > 0 {
            trailing = trailing.child(
                div()
                    .text_size(px(fonts.row_rating))
                    .text_color(rgb(palette::primary()))
                    .child(row.rating.to_string()),
            );
        }
        body.child(trailing).into_element()
    }
}

/// Convert a row's `0x00RRGGBB` palette/hover fill into an opaque `gpui::Rgba`.
///
/// Using `rgb(...)` rather than `gpui::rgba(...)` is essential because palette
/// and `ColorBehavior` colors are stored as `0x00RRGGBB` `u32` values (not
/// `0xRRGGBBAA`).
pub(super) fn row_bg_color(selected: bool, hover_fill: u32) -> gpui::Rgba {
    rgb(if selected {
        palette::secondary_container()
    } else {
        hover_fill
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_bg_color_treats_hover_fill_as_opaque_rgb() {
        let mut clock = anim::Clock::new();
        clock.now = 0.0;
        let behavior = anim::ColorBehavior::new(row_base_fill(0));

        // Even non-hovered row uses surface (0x00RRGGBB) as opaque RGB, not 0xRRGGBBAA.
        let even_fill = row_hover_fill(&behavior, &clock, 0, false, false);
        let even_bg = row_bg_color(false, even_fill);
        assert_eq!(even_bg, rgb(palette::surface()));
        assert!((even_bg.a - 1.0).abs() < f32::EPSILON);

        // Odd non-hovered row uses surface_container_lowest as opaque RGB.
        let odd_fill = row_hover_fill(&behavior, &clock, 1, false, false);
        let odd_bg = row_bg_color(false, odd_fill);
        assert_eq!(odd_bg, rgb(palette::surface_container_lowest()));
        assert!((odd_bg.a - 1.0).abs() < f32::EPSILON);
        assert_ne!(even_bg, odd_bg);

        // Hovered unselected row uses the animated hover color as opaque RGB.
        let mut hovered_behavior = anim::ColorBehavior::new(row_base_fill(0));
        target_row_hover_fill(&mut hovered_behavior, &clock, 0, false, true);
        clock.now = 1.0;
        let hover_fill = row_hover_fill(&hovered_behavior, &clock, 0, false, true);
        let hover_bg = row_bg_color(false, hover_fill);
        assert_eq!(hover_bg, rgb(palette::surface_container_high()));
        assert!((hover_bg.a - 1.0).abs() < f32::EPSILON);

        // Selected row always uses secondary_container as opaque RGB.
        let selected_bg = row_bg_color(true, hover_fill);
        assert_eq!(selected_bg, rgb(palette::secondary_container()));
        assert!((selected_bg.a - 1.0).abs() < f32::EPSILON);
    }
}
