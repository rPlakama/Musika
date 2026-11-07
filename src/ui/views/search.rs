//! Floating search overlay and fields.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn search_loading_visible(&self, search: &SearchState) -> bool {
        search.loading
            && self
                .search_loading_since
                .is_some_and(|started| self.clock.now - started >= 0.06)
    }

    pub(in crate::ui) fn search_field(
        &self,
        search: &SearchState,
        height: f32,
        denied: f32,
    ) -> gpui::AnyElement {
        let field = div()
            .h(px(height))
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(rgb(if denied > 0.0 {
                palette::error()
            } else if search.typing {
                palette::primary()
            } else {
                palette::outline_variant()
            }))
            .bg(rgb(palette::surface_container_highest()))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .overflow_hidden()
            .child(icon(
                "search-right-1505-svgrepo-com.svg",
                height * 0.40,
                if search.typing {
                    palette::primary()
                } else {
                    palette::on_surface_variant()
                },
            ))
            .child(text_edit::editor_content(
                &search.editor,
                height * 0.32,
                height * 0.42,
                "Search artists, albums, songs",
                search.typing,
            ));
        field.into_any_element()
    }

    pub(in crate::ui) fn search_result_row(
        &self,
        cx: &Context<Self>,
        row: &Row,
        paint: SearchRowPaint,
    ) -> gpui::AnyElement {
        let SearchRowPaint {
            scope,
            index,
            row_h,
            selected,
            hovered,
            scale,
            text_scale,
        } = paint;
        let fonts = self.visuals.fonts;
        let emphasis = self.visuals.search_text_emphasis(text_scale);
        let cover_scale = self.visuals.search_cover_scale(scale);
        let cover = cover_source(&row.cover_url);
        div()
            .id(("search-row", index))
            .flex()
            .w_full()
            .items_center()
            .gap_3()
            .h(px(row_h))
            .px_3()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(rgb(palette::primary_container())))
            .when(hovered && !selected, |this| {
                this.bg(rgb(palette::surface_container_high()))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_row = Some(index);
                } else if this.hovered_row == Some(index) {
                    this.hovered_row = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(search) = this.search.as_mut() {
                    search.scope = scope;
                    search.scope_cursors[scope] = index;
                }
                this.activate_search_result(cx);
            }))
            .child(art_scaled(cover, row_h * 0.714, cover_scale))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(fonts.search_title * emphasis))
                            .font_weight(if selected {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(if selected {
                                palette::on_primary_container()
                            } else {
                                palette::on_surface()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(fonts.search_subtitle * emphasis))
                            .text_color(rgb(if selected {
                                palette::on_primary_container()
                            } else {
                                palette::on_surface_variant()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(row.subtitle.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(fonts.search_meta * emphasis))
                    .text_color(rgb(if selected {
                        palette::on_primary_container()
                    } else {
                        palette::on_surface_variant()
                    }))
                    .whitespace_nowrap()
                    .child(row.meta.clone()),
            )
            .into_any_element()
    }

    pub(in crate::ui) fn search_overlay(
        &self,
        cx: &Context<Self>,
        search: &SearchState,
        progress: f32,
    ) -> gpui::AnyElement {
        let row_h = 56.0;
        let field_h = 54.0;
        let panel_w = (self.win_w * 0.60).clamp(360.0, 680.0);
        let panel_h = field_h
            + 12.0
            + 34.0
            + 10.0
            + 24.0
            + row_h * state::SEARCH_VISIBLE_ROWS as f32
            + 36.0
            + 32.0;
        let denied = self.search_denied_opacity();
        let loading = self.search_loading_visible(search);
        let query_empty = search.editor.text.is_empty();
        let scoped = search.scoped();
        // Keep settled rows visible while the next payload is in flight; the
        // skeleton is only for the first empty search, not every keystroke.
        let show_skeleton = loading && search.rows.is_empty();
        let nothing = !show_skeleton && !query_empty && scoped.is_empty();
        let selection_count = search.selection.count(search.cursor());
        let footer = if selection_count > 0 {
            format!(
                "{} selected · Shift-J/K extend · v clear · Alt actions · ↵ open",
                selection_count
            )
        } else if search.typing {
            "Type to filter · Ctrl-N/P rows · ⇥ scope · ↵ open · esc normal".to_string()
        } else {
            "j/k rows · Ctrl-N/P scroll · v visual · shift-J/K extend · i type · esc close"
                .to_string()
        };

        let result_area: gpui::AnyElement = if show_skeleton {
            loading_skeleton(row_h, state::SEARCH_VISIBLE_ROWS)
        } else if nothing {
            div()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(rgb(palette::on_surface_variant()))
                .child(if query_empty {
                    "Nothing played yet"
                } else {
                    "No Results"
                })
                .into_any_element()
        } else {
            let scroll = self.search_scroll.handle.clone();
            let scope = search.scope;
            uniform_list(
                "search-results",
                search.scope_count(scope),
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let mut items = Vec::with_capacity(range.len());
                    let Some(search) = this.search.as_ref() else {
                        return items;
                    };
                    let rows = search.scoped();
                    let cursor = search.cursor();
                    for index in range {
                        let Some(row) = rows.get(index) else { break };
                        let selected = index == cursor || search.selection.contains(cursor, index);
                        let hovered = this.hovered_row == Some(index);
                        let key = format!("search:{scope}:{}", row.id);
                        let active = selected || hovered;
                        let scale = this.row_scale_factor(&key, active);
                        let text_scale = this.row_text_factor(&key, active);
                        items.push(this.search_result_row(
                            cx,
                            row,
                            SearchRowPaint {
                                scope,
                                index,
                                row_h,
                                selected,
                                hovered,
                                scale,
                                text_scale,
                            },
                        ));
                    }
                    items
                }),
            )
            .h(px(row_h * state::SEARCH_VISIBLE_ROWS as f32))
            .w_full()
            .track_scroll(scroll)
            .into_any_element()
        };

        let scopes = (0..4)
            .map(|scope| {
                let active = scope == search.scope;
                let count = search.scope_count(scope);
                div()
                    .id(("search-scope", scope))
                    .h(px(28.0))
                    .px_3()
                    .rounded_full()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .text_size(px(11.0))
                    .bg(rgb(if active {
                        palette::primary()
                    } else {
                        palette::surface_container_high()
                    }))
                    .text_color(rgb(if active {
                        palette::on_primary()
                    } else {
                        palette::on_surface_variant()
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(search) = this.search.as_mut() {
                            search.scope_cursors[search.scope] = search.cursor();
                            search.scope = scope;
                            search.scope_cursors[scope] = search.scope_cursors[scope]
                                .min(search.scope_count(scope).saturating_sub(1));
                            search.selection.clear();
                        }
                        this.follow_search(false);
                        cx.notify();
                    }))
                    .child(format!("{} {count}", search_scope_label(scope)))
            })
            .collect::<Vec<_>>();

        div()
            .id("search-backdrop")
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .justify_center()
            .bg(gpui::black().opacity(0.50 * progress))
            .opacity(progress)
            .on_click(cx.listener(|this, _, _, cx| {
                this.search_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("search-panel")
                    .absolute()
                    .top(px(self.win_h * 0.14))
                    .w(px(panel_w))
                    .h(px(panel_h))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(palette::outline_variant()))
                    .bg(rgb(palette::surface_container()))
                    .shadow_lg()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                    .child(self.search_field(search, field_h, denied))
                    .child(
                        div()
                            .h(px(30.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .children(scopes),
                    )
                    .when(query_empty, |this| {
                        this.child(
                            div()
                                .h(px(22.0))
                                .px_1()
                                .flex()
                                .items_center()
                                .text_size(px(11.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette::on_surface_variant()))
                                .child("Most Played"),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(result_area),
                    )
                    .child(
                        div()
                            .h(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child(footer),
                    ),
            )
            .into_any_element()
    }
}
