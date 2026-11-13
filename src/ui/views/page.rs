//! Album/artist/playlist/radio page view.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn page_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(page) = self.page.as_ref() else {
            return div().flex().flex_1();
        };
        let profile = page_profile::PageProfile::for_descriptor(&page.descriptor);
        let row_flags = profile.row_flags;
        let page_h = (self.win_h - 120.0).max(1.0);
        let page_w = self.win_w.max(1.0);
        let max_cover = (page_h - page_w * 0.04) * 0.75;
        let cover_side = page_w.min(page_h).min(max_cover).max(1.0);
        let cover_art_side = (cover_side - page_w * 0.04).max(1.0);
        let cover_bottom = cover_art_side + page_w * 0.02;
        // The page hero paints at hundreds of pixels: use the untouched
        // original art, never the row thumbnail or a resized display copy.
        let cover = if profile.collage {
            None
        } else {
            full_cover_source_with_remote(&page.header.cover_url, &page.header.cover_remote)
        };
        let header_h = cover_bottom.max(240.0);

        if page.loading && page.rows.is_empty() {
            return div().flex().flex_col().flex_1().child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_size(px(15.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child("Loading…"),
            );
        }
        if let Some(error) = &page.error {
            return div().flex().flex_col().flex_1().child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(error.clone()),
            );
        }

        let count = page.rows.len();
        if count == 0 {
            return div().flex().flex_col().flex_1().child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_size(px(15.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(profile.empty_text),
            );
        }
        let scroll = self.page_scroll_modal.handle.clone();
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .child(
                div()
                    .absolute()
                    .top(px(page_w * 0.02))
                    .left(px(page_w * 0.02))
                    .right(px(page_w * 0.02))
                    .h(px(cover_art_side))
                    .flex()
                    .justify_center()
                    .overflow_hidden()
                    .child(if profile.collage {
                        // Collage panes are hero-sized too: each pane paints the
                        // original cover source, falling back to the provider
                        // URL when the local original is missing.
                        let sources = page
                            .header
                            .collage
                            .iter()
                            .enumerate()
                            .map(|(index, url)| {
                                full_cover_source_with_remote(
                                    url,
                                    page.header
                                        .collage_remote
                                        .get(index)
                                        .map(String::as_str)
                                        .unwrap_or_default(),
                                )
                            })
                            .collect();
                        collage_art_from_sources(
                            sources,
                            cover_art_side,
                            self.image_cache.clone(),
                        )
                    } else {
                        self.cover_view(
                            "page-hero",
                            0,
                            cover,
                            cover_art_side,
                            cover_art_side,
                            1.0,
                            gpui::ObjectFit::Contain,
                            4.0,
                        )
                    }),
            )
            .child(div().w_full().h(px(header_h)))
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .px(px(page_w * 0.02))
                    .pt(px(page_w * 0.02))
                    .child(
                        uniform_list(
                            "page-rows",
                            count,
                            cx.processor(
                                move |this, range: std::ops::Range<usize>, _window, cx| {
                                    let row_h = this.page_scroll_modal.row_h;
                                    let line_query: gpui::SharedString =
                                        this.line_search.query().to_string().into();
                                    let mut items = Vec::with_capacity(range.len());
                                    for index in range {
                                        let Some(page) = this.page.as_ref() else {
                                            break;
                                        };
                                        let Some(row) = page.rows.get(index) else {
                                            break;
                                        };
                                        let selected = index == page.list.cursor
                                            || page.list.selection.contains(page.list.cursor, index);
                                        let cover = page.cover(index);
                                        let hovered = this.hovered_page == Some(index);
                                        let hover_fill = row_hover_fill(
                                            &this.page_hover_fill,
                                            &this.clock,
                                            index,
                                            selected,
                                            hovered,
                                        );
                                        let (scale, text_scale) = this.row_emphasis(
                                            LineSurface::Page,
                                            this.page_generation as usize,
                                            index,
                                            selected,
                                        );
                                        let cover_scale = this.visuals.row_cover_scale(scale);
                                        let emphasis = this.visuals.row_text_emphasis(text_scale);
                                        items.push(
                                            div()
                                                .id(gpui::ElementId::named_usize(
                                                    if selected {
                                                        "page-row-selected"
                                                    } else {
                                                        "page-row"
                                                    },
                                                    index,
                                                ))
                                                .w_full()
                                                .flex()
                                                .items_center()
                                                .h(px((row_h - 2.0).max(18.0)))
                                                .mt(px(1.0))
                                                .px_3()
                                                .rounded_sm()
                                                .cursor_pointer()
                                                .bg(if selected {
                                                    rgb(palette::secondary_container())
                                                } else {
                                                    gpui::rgba(hover_fill)
                                                })
                                                .on_hover(cx.listener(
                                                    move |this, hovered: &bool, _, cx| {
                                                        target_row_hover_fill(
                                                            &mut this.page_hover_fill,
                                                            &this.clock,
                                                            index,
                                                            selected,
                                                            *hovered,
                                                        );
                                                        if *hovered {
                                                            this.hovered_page = Some(index);
                                                        } else if this.hovered_page == Some(index) {
                                                            this.hovered_page = None;
                                                        }
                                                        cx.notify();
                                                    },
                                                ))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.status_focus = None;
                                                    if let Some(page) = this.page.as_mut() {
                                                        page.select(index);
                                                    }
                                                    this.follow_page(true);
                                                    this.activate_page_cursor();
                                                    cx.notify();
                                                }))
                                                .child(this.row_content(
                                                    row,
                                                    cover,
                                                    &line_query,
                                                    RowPaint {
                                                        flags: row_flags,
                                                        surface: LineSurface::Page,
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
                                },
                            ),
                        )
                        .pt(px(8.0))
                        .pb(px(8.0))
                        .h_full()
                        .w_full()
                        .on_scroll_wheel(cx.listener(|this, _, _, _| {
                            this.page_scroll_modal.on_wheel();
                        }))
                        .track_scroll(scroll),
                    ),
            )
    }
}
