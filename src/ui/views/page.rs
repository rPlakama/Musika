//! Album/artist/playlist/radio page view.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn page_view(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(page) = self.page.as_ref() else {
            return div().flex().flex_1().into_any_element();
        };
        let profile = page_profile::PageProfile::for_descriptor(&page.descriptor);
        let row_flags = profile.row_flags;
        let window = metrics::WindowLayout::new(self.win_w, self.win_h);
        let layout = window.page();
        // The page hero paints at hundreds of pixels: use the untouched
        // original art, never the row thumbnail or a resized display copy.
        let cover = if profile.collage {
            None
        } else {
            CoverRole::Hero.source(&page.header.cover_url, &page.header.cover_remote)
        };
        // The row thumbnail is already on disk: paint it immediately while
        // the untouched original decodes to the hero size.
        let cover_preview = if profile.collage {
            None
        } else {
            CoverRole::Row.source(&page.header.cover_url, &page.header.cover_remote)
        };

        if page.loading && page.rows.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_size(px(15.0))
                        .text_color(rgb(palette::on_surface_variant()))
                        .child("Loading…"),
                )
                .into_any_element();
        }
        if let Some(error) = &page.error {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.0))
                        .text_color(rgb(palette::on_surface_variant()))
                        .child(error.clone()),
                )
                .into_any_element();
        }

        let count = page.rows.len();
        if count == 0 {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_size(px(15.0))
                        .text_color(rgb(palette::on_surface_variant()))
                        .child(profile.empty_text),
                )
                .into_any_element();
        }

        let render_cover = |side: f32| -> gpui::AnyElement {
            if side <= 0.0 {
                return div().into_any_element();
            }
            if profile.collage {
                // Collage panes sit on the hero and must use the untouched
                // provider original, not the 320 px row thumbnail (which
                // visibly softens at this size).
                let sources = page
                    .header
                    .collage
                    .iter()
                    .enumerate()
                    .map(|(index, url)| {
                        CoverRole::Collage.source(
                            url,
                            page.header
                                .collage_remote
                                .get(index)
                                .map(String::as_str)
                                .unwrap_or_default(),
                        )
                    })
                    .collect();
                collage_art_from_sources(sources, side, self.image_cache.clone())
            } else {
                self.cover_view(
                    "page-hero",
                    0,
                    cover.clone(),
                    cover_preview.clone(),
                    side,
                    side,
                    1.0,
                    gpui::ObjectFit::Contain,
                    4.0,
                )
            }
            .into_any_element()
        };

        match layout.mode {
            metrics::PageMode::Stacked => div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .child(
                    div()
                        .absolute()
                        .top(px(layout.margin_y))
                        .left(px(layout.margin_x))
                        .right(px(layout.margin_x))
                        .h(px(layout.hero_art))
                        .flex()
                        .justify_center()
                        .overflow_hidden()
                        .child(render_cover(layout.hero_art)),
                )
                .child(div().w_full().h(px(layout.header_h)))
                .child(
                    div()
                        .flex_1()
                        .w_full()
                        .min_h_0()
                        .px(px(layout.margin_x))
                        .child(self.page_rows(cx, row_flags, layout.list_pad)),
                )
                .into_any_element(),
            metrics::PageMode::Split => div()
                .flex()
                .flex_row()
                .flex_1()
                .w_full()
                .min_h_0()
                .gap(px(layout.gap))
                .px(px(layout.margin_x))
                .py(px(layout.margin_y))
                .child(
                    div()
                        .w(px(layout.cover_w))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .overflow_hidden()
                        .child(render_cover(layout.cover_art())),
                )
                .child(
                    div()
                        .w(px(layout.list_w))
                        .min_w_0()
                        .h_full()
                        .child(self.page_rows(cx, row_flags, layout.list_pad)),
                )
                .into_any_element(),
            metrics::PageMode::RowsOnly => div()
                .flex_1()
                .w_full()
                .min_h_0()
                .px(px(layout.margin_x))
                .child(self.page_rows(cx, row_flags, layout.list_pad))
                .into_any_element(),
        }
    }

    /// The one page row list. Stacked and split modes only change its parent;
    /// the uniform list id, scroll handle and row builder stay identical.
    fn page_rows(
        &self,
        cx: &Context<Self>,
        row_flags: page_profile::RowFlags,
        list_pad: f32,
    ) -> impl IntoElement {
        let count = self.page.as_ref().map_or(0, |page| page.rows.len());
        let scroll = self.page_scroll_modal.handle.clone();
        uniform_list(
            "page-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let row_h = this.page_scroll_modal.row_h;
                let line_query: gpui::SharedString = this.line_search.query().to_string().into();
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
                            .h(px((row_h - 2.0).max(metrics::MIN_ROW_HEIGHT - 2.0)))
                            .mt(px(1.0))
                            .px_3()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(if selected {
                                rgb(palette::secondary_container())
                            } else {
                                gpui::rgba(hover_fill)
                            })
                            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
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
                            }))
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
            }),
        )
        .pt(px(list_pad))
        .pb(px(list_pad))
        .h_full()
        .w_full()
        .on_scroll_wheel(cx.listener(|this, _, _, _| {
            this.page_scroll_modal.on_wheel();
        }))
        .track_scroll(scroll)
    }
}
