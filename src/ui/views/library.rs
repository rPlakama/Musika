//! Library list, rows and selected-row rendering.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn list(&self, cx: &Context<Self>, window: &mut Window) -> impl IntoElement {
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
                        let line_query: gpui::SharedString = this.line_search.query().to_string().into();
                        let mut items = Vec::with_capacity(range.len());
                        for index in range {
                            let selected = index == this.view.list.cursor
                                || this.view.list.selection.contains(this.view.list.cursor, index);
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
                                    .bg(if selected {
                                        rgb(palette::secondary_container())
                                    } else {
                                        gpui::rgba(hover_fill)
                                    })
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
                _ => "retained-cover",
            };
            body = body.child(div().mr_3().child(RetainedCover::new(
                id_name,
                paint.index,
                cover.cloned(),
                self.image_cache.clone(),
                paint.height * 0.78,
                paint.cover_scale,
            )));
        }
        let id_name = match paint.surface {
            LineSurface::Library => "retained-text-library",
            LineSurface::Page => "retained-text-page",
            _ => "retained-text",
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
        let downloaded = self.marks.downloaded(&row.song_id, &row.album_id) || row.downloaded;
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
