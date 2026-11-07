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
        let hero_bucket = self.page_hero_bucket();
        let cover = if profile.collage {
            None
        } else {
            self.display_cover(&page.header.cover_art, hero_bucket)
                .map(CoverSource::Path)
                .or_else(|| self.hero_cover(&page.header.cover_url, hero_bucket))
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
                        collage_art(&page.header.collage, cover_art_side)
                    } else {
                        art(cover, cover_art_side)
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
                                    let page_id = this
                                        .page
                                        .as_ref()
                                        .map(|page| page.descriptor.id.clone())
                                        .unwrap_or_default();
                                    let mut items = Vec::with_capacity(range.len());
                                    for index in range {
                                        let selected_id = this.selected_id.clone();
                                        let Some((row, selected)) =
                                            this.page.as_ref().and_then(|page| {
                                                page.rows.get(index).map(|row| {
                                                    let selected = selected_id.as_deref()
                                                        == Some(row.id.as_str())
                                                        || index == page.cursor
                                                        || page
                                                            .selection
                                                            .contains(page.cursor, index);
                                                    (row.clone(), selected)
                                                })
                                            })
                                        else {
                                            break;
                                        };
                                        let hovered = this.hovered_page == Some(index);
                                        let page_base = if index % 2 != 0 {
                                            palette::surface_container_lowest()
                                        } else {
                                            palette::surface()
                                        };
                                        let hover_fill = this
                                            .row_hover_colors
                                            .entry(format!("page-hover:{index}"))
                                            .or_insert_with(|| anim::ColorBehavior::new(page_base))
                                            .target(
                                                if hovered && !selected {
                                                    palette::surface_container_high()
                                                } else {
                                                    page_base
                                                },
                                                anim::QUICK_MS,
                                                anim::Curve::Standard,
                                                &this.clock,
                                            );
                                        let scale = this.row_scale_factor(
                                            format!("page:{page_id}:{}", row.id),
                                            selected,
                                        );
                                        let cover_scale = 1.0 + scale * 0.20;
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
                                                    &row,
                                                    RowPaint {
                                                        flags: row_flags,
                                                        surface: LineSurface::Page,
                                                        index,
                                                        selected,
                                                        height: row_h,
                                                        cover_scale,
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
