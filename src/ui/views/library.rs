//! Library list, rows and selected-row rendering.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn list(&self, cx: &Context<Self>, window: &mut Window) -> impl IntoElement {
        let title = self.view.scope.label();
        let hint = "(Shift-J/K) cycle filter";
        let order = format!("Shift-H / Shift-L  /  {}", self.view.order_label());
        let selection_count = self.view.selection.count(self.view.cursor);
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
                window.request_animation_frame();
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
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                        let row_h = this.library_scroll.row_h;
                        let mut items = Vec::with_capacity(range.len());
                        for index in range {
                            let selected = this.view.rows.get(index).is_some_and(|row| {
                                this.selected_id.as_deref() == Some(row.id.as_str())
                            }) || index == this.view.cursor
                                || this.view.selection.contains(this.view.cursor, index);
                            let hovered = this.hovered_row == Some(index);
                            let striped = index % 2 != 0;
                            let Some(row) = this.view.rows.get(index) else {
                                // RowWindow blank slot: a waving grey placeholder
                                // until the live window lands.
                                _window.request_animation_frame();
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
                            let cover_scale = if selected { 1.20 } else { 1.0 };
                            let base_fill = if striped {
                                palette::surface_container_lowest()
                            } else {
                                palette::surface()
                            };
                            let hover_fill = this
                                .row_hover_colors
                                .entry(format!("library-hover:{}", row.id))
                                .or_insert_with(|| anim::ColorBehavior::new(base_fill))
                                .target(
                                    if hovered && !selected {
                                        palette::surface_container_high()
                                    } else {
                                        base_fill
                                    },
                                    anim::QUICK_MS,
                                    anim::Curve::Standard,
                                    &this.clock,
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
                                        RowPaint {
                                            flags: LIBRARY_ROW_FLAGS,
                                            surface: LineSurface::Library,
                                            index,
                                            selected,
                                            height: row_h,
                                            cover_scale,
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
    pub(in crate::ui) fn row_content(&self, row: &state::Row, paint: RowPaint) -> gpui::Div {
        let emphasis = if paint.selected { 1.08 } else { 1.0 };
        let current = self.line_search_current(paint.surface, paint.index);
        let mut body = div().flex().items_center().min_w_0().flex_1();
        if paint.flags.show_cover {
            let cover = cover_source(&row.cover_url);
            body = body.child(div().mr_3().child(art_scaled(
                cover,
                paint.height * 0.78,
                paint.cover_scale,
            )));
        }
        let mut cells = div().flex().flex_col().min_w_0().flex_1().overflow_hidden();
        let title_size = if paint.flags.compact { 15.0 } else { 18.0 } * emphasis;
        let title = div()
            .text_size(px(title_size))
            .font_weight(if paint.selected {
                gpui::FontWeight::BOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .text_color(rgb(palette::on_surface()))
            .overflow_hidden()
            .child(highlighted_text(
                &row.title,
                self.line_search.query(),
                palette::on_surface(),
                current,
            ));
        let title = if paint.flags.compact {
            title.whitespace_nowrap().text_ellipsis()
        } else {
            title.whitespace_normal().max_h(px(34.0))
        };
        cells = cells.child(title);
        if paint.flags.show_subtitle && !row.subtitle.is_empty() {
            let subtitle = div()
                .text_size(px(if paint.flags.compact { 11.0 } else { 12.0 } * emphasis))
                .text_color(rgb(palette::on_surface_variant()))
                .overflow_hidden()
                .child(highlighted_text(
                    &row.subtitle,
                    self.line_search.query(),
                    palette::on_surface_variant(),
                    current,
                ));
            let subtitle = if paint.flags.compact {
                subtitle.whitespace_nowrap().text_ellipsis()
            } else {
                subtitle.whitespace_normal().max_h(px(22.0))
            };
            cells = cells.child(subtitle);
        }
        body = body.child(cells);
        let downloaded = self.marks.downloaded(&row.song_id, &row.album_id) || row.downloaded;
        let mut trailing = div().flex().items_center().gap_2();
        if paint.flags.show_length && !row.meta.is_empty() {
            trailing = trailing.child(
                div()
                    .text_size(px(11.0 * emphasis))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(highlighted_text(
                        &row.meta,
                        self.line_search.query(),
                        palette::on_surface_variant(),
                        current,
                    )),
            );
        }
        if downloaded {
            trailing = trailing.child(icon("outline-download.svg", 12.0, palette::primary()));
        }
        if row.starred {
            trailing = trailing.child(icon("baseline-favorite.svg", 11.0, 0xFFD54F));
        }
        if row.rating > 0 {
            trailing = trailing.child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgb(palette::primary()))
                    .child(row.rating.to_string()),
            );
        }
        body.child(trailing).into_element()
    }
}
