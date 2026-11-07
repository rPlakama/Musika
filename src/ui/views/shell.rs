//! Top bar, status bar, player bar and login view.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn topbar_action_chip(
        &self,
        action: TopBarAction,
        page: Option<&PageBarState>,
        anchors: StatusAnchorSlots,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let slot = action.slot();
        let focused = self.status_focus == Some(slot);
        let (icon_el, opacity) = match action {
            TopBarAction::Sync => (
                icon_rotated(
                    "reload-ui-2-svgrepo-com.svg",
                    18.0,
                    if self.syncing {
                        palette::on_surface_variant()
                    } else {
                        palette::on_surface()
                    },
                    self.sync_angle(),
                ),
                if self.syncing { 0.38 } else { 0.85 },
            ),
            TopBarAction::Wipe => (icon("delete-outline.svg", 18.0, palette::on_surface()), 1.0),
            TopBarAction::Download => {
                let cached = page.is_some_and(|page| page.cached);
                let downloading = page.is_some_and(|page| page.downloading);
                (
                    icon(
                        if cached {
                            "baseline-download.svg"
                        } else {
                            "outline-download.svg"
                        },
                        18.0,
                        palette::on_surface_variant(),
                    ),
                    if downloading { 0.38 } else { 1.0 },
                )
            }
            TopBarAction::Favorite => {
                let starred = page.is_some_and(|page| page.starred);
                (
                    icon(
                        if starred {
                            "baseline-favorite.svg"
                        } else {
                            "heart-add-fill.svg"
                        },
                        18.0,
                        if starred {
                            palette::primary()
                        } else {
                            palette::on_surface_variant()
                        },
                    ),
                    1.0,
                )
            }
            TopBarAction::Tool => (
                icon("dots-fill.svg", 18.0, palette::on_surface_variant()),
                1.0,
            ),
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(if focused {
                palette::primary()
            } else {
                palette::surface_container_low()
            }))
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(first) = bounds.first() {
                    let x = f32::from(first.origin.x) + f32::from(first.size.width) * 0.5;
                    let y = f32::from(first.origin.y) + f32::from(first.size.height) * 0.5;
                    anchors.borrow_mut()[slot] = Some((x, y));
                }
            })
            .child(
                div()
                    .id(gpui::ElementId::named_usize("topbar-action", slot))
                    .p(px(3.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate_status(slot);
                        cx.notify();
                    }))
                    .opacity(opacity)
                    .child(icon_el),
            )
            .into_any_element()
    }

    pub(in crate::ui) fn bar_separator(&self) -> gpui::AnyElement {
        div()
            .text_size(px(14.0))
            .text_color(rgb(palette::on_surface_variant()))
            .child("·")
            .into_any_element()
    }

    pub(in crate::ui) fn star_glyphs(rating: u8) -> [bool; 5] {
        std::array::from_fn(|index| index < usize::from(rating.min(5)))
    }

    pub(in crate::ui) fn star_counter(&self, rating: u8) -> gpui::AnyElement {
        let mut row = div().flex_none().flex().items_center().gap_1();
        for filled in Self::star_glyphs(rating) {
            row = row.child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(if filled {
                        palette::primary()
                    } else {
                        palette::on_surface_variant()
                    }))
                    .child(if filled { "★" } else { "☆" }),
            );
        }
        row.into_any_element()
    }

    pub(in crate::ui) fn home_bar(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let (welcome, offline) = match self.header.as_ref() {
            Some(header) => (format!("Welcome, {}", header.username), false),
            None => ("Welcome".to_string(), false),
        };
        let report = match self.header.as_ref() {
            Some(header) => {
                let sync = if self.syncing {
                    "Syncing…".to_string()
                } else {
                    header
                        .last_sync
                        .as_deref()
                        .map(last_sync_text)
                        .unwrap_or_else(|| "—".to_string())
                };
                let provider = if header.server_type.is_empty() {
                    String::new()
                } else {
                    let mut name = header.server_type.clone();
                    if let Some(first) = name.get_mut(0..1) {
                        first.make_ascii_uppercase();
                    }
                    format!(", {name}")
                };
                format!("Last Sync: {sync}{provider}")
            }
            None => String::new(),
        };
        div()
            .flex()
            .items_center()
            .w_full()
            .h_full()
            .px_4()
            .child(
                div().flex().flex_1().min_w_0().items_center().child(
                    div()
                        .text_size(px(19.0))
                        .text_color(rgb(if offline {
                            palette::tertiary()
                        } else {
                            palette::on_surface_variant()
                        }))
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(welcome),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .justify_end()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(14.0))
                            .text_color(rgb(if self.syncing {
                                palette::tertiary()
                            } else {
                                palette::on_surface_variant()
                            }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(report),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(2.0))
                            .h(px((self.win_h * 0.0648 * 0.16).max(2.0)))
                            .rounded_full()
                            .bg(rgb(palette::outline_variant()))
                            .overflow_hidden()
                            .child(
                                div()
                                    .w(px(2.0 * self.sync_fill.value(&self.clock)))
                                    .h_full()
                                    .bg(rgb(if self.syncing {
                                        palette::tertiary()
                                    } else {
                                        palette::primary()
                                    })),
                            ),
                    )
                    .children(
                        self.topbar_actions()
                            .into_iter()
                            .map(|action| {
                                self.topbar_action_chip(
                                    action,
                                    None,
                                    self.status_anchors.clone(),
                                    cx,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
            )
            .into_any_element()
    }

    pub(in crate::ui) fn page_context_bar(
        &self,
        mode: SuperiorMode,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let Some(page) = self.page_bar() else {
            return div().into_any_element();
        };
        let title = if page.title.is_empty() {
            page.kind.label().to_string()
        } else {
            page.title.clone()
        };
        let mut identity = div()
            .flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .text_size(px(19.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette::on_surface()))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            );
        if !page.subtitle.is_empty() {
            identity = identity.child(self.bar_separator()).child(
                div()
                    .flex_none()
                    .text_size(px(14.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(page.subtitle.clone()),
            );
        }
        if matches!(
            mode,
            SuperiorMode::Album | SuperiorMode::Playlist | SuperiorMode::Radio
        ) && !page.meta.is_empty()
        {
            identity = identity.child(self.bar_separator()).child(
                div()
                    .flex_none()
                    .text_size(px(14.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(page.meta.clone()),
            );
        }
        let mut row = div()
            .flex()
            .items_center()
            .w_full()
            .h_full()
            .px_4()
            .gap_2()
            .child(identity);
        match mode {
            SuperiorMode::Album | SuperiorMode::Artist => {
                if page.starred {
                    row = row.child(self.topbar_action_chip(
                        TopBarAction::Favorite,
                        Some(&page),
                        self.status_anchors.clone(),
                        cx,
                    ));
                }
                row = row.child(self.star_counter(page.rating));
                if page.cached || page.downloading {
                    row = row.child(self.topbar_action_chip(
                        TopBarAction::Download,
                        Some(&page),
                        self.status_anchors.clone(),
                        cx,
                    ));
                }
                if mode == SuperiorMode::Artist || page_tool_visible(&page) {
                    row = row.child(self.topbar_action_chip(
                        TopBarAction::Tool,
                        Some(&page),
                        self.status_anchors.clone(),
                        cx,
                    ));
                }
            }
            SuperiorMode::Radio => {
                row = row.child(self.topbar_action_chip(
                    TopBarAction::Tool,
                    Some(&page),
                    self.status_anchors.clone(),
                    cx,
                ));
            }
            _ => {}
        }
        row.into_any_element()
    }

    pub(in crate::ui) fn top_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let mode = self.superior_mode();
        let content = if mode.is_page() {
            self.page_context_bar(mode, cx)
        } else {
            self.home_bar(cx)
        };
        div()
            .relative()
            .w_full()
            .h(relative(0.0648))
            .bg(rgb(palette::surface_container_low()))
            .flex()
            .items_center()
            .child(content)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.0))
                    .bg(rgb(palette::outline_variant())),
            )
    }

    pub(in crate::ui) fn player_bar(&self) -> impl IntoElement {
        let track = self.player.current().cloned();
        let state = self.player.state();
        let position = self.player.position();
        let duration = self.player.duration();
        let volume = self.player.volume();
        let muted = self.player.muted();
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let (title, artist, cover) = match &track {
            Some(track) => (
                track.title.clone(),
                track.artist.clone(),
                cover_source(&track.cover_url),
            ),
            None => ("Nothing playing".to_string(), String::new(), None),
        };
        let m = metrics::PlayerBarMetrics::new(self.win_w, self.win_h);
        div()
            .relative()
            .w_full()
            .h(relative(0.0828))
            .bg(rgb(palette::surface_container()))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(3.0))
                    .bg(rgb(palette::surface_container_high()))
                    .child(
                        div()
                            .h_full()
                            .w(relative(progress))
                            .bg(rgb(palette::primary())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .size_full()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(art(cover, m.cover_side))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .text_size(px(m.title_size))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(title),
                                    )
                                    .when(!artist.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .text_size(px(m.artist_size))
                                                .text_color(rgb(palette::on_surface_variant()))
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .child(artist),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .gap(px(m.transport_spacing))
                            .child(transport_button(
                                "skip-previous-solid-svgrepo-com.svg",
                                m.transport_unit,
                                palette::on_surface(),
                                self.transport_scale(0),
                            ))
                            .child(transport_button(
                                if state == crate::player::State::Playing {
                                    "pause-svgrepo-com.svg"
                                } else {
                                    "play-1001-svgrepo-com.svg"
                                },
                                m.play_size,
                                palette::on_surface(),
                                self.transport_scale(1),
                            ))
                            .child(transport_button(
                                "skip-next-circle-solid-svgrepo-com.svg",
                                m.transport_unit,
                                palette::on_surface(),
                                self.transport_scale(2),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .overflow_hidden()
                            .justify_end()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(m.time_size))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .text_right()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(format!(
                                        "{} / {}",
                                        format_time(position),
                                        format_time(duration)
                                    )),
                            )
                            .when(self.repeat != crate::player::Repeat::Off, |this| {
                                this.child(
                                    div()
                                        .text_size(px(m.time_size))
                                        .text_color(rgb(palette::primary()))
                                        .child(repeat_label(self.repeat)),
                                )
                            })
                            .child(icon(
                                if muted {
                                    "vol-mute-svgrepo-com.svg"
                                } else if volume <= 1.0 {
                                    "vol-zero-svgrepo-com.svg"
                                } else if volume <= 33.0 {
                                    "vol-low-svgrepo-com.svg"
                                } else {
                                    "vol-high-svgrepo-com.svg"
                                },
                                m.volume_icon,
                                palette::on_surface_variant(),
                            ))
                            .child(
                                div()
                                    .w(px(m.volume_bar))
                                    .h(px(3.0))
                                    .rounded_full()
                                    .bg(rgb(palette::surface_container_high()))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative((volume / 100.0) as f32))
                                            .rounded_full()
                                            .bg(rgb(palette::primary())),
                                    ),
                            ),
                    ),
            )
    }

    pub(in crate::ui) fn status_bar(&self) -> impl IntoElement {
        let counts = self
            .header
            .as_ref()
            .map(|header| header.counts)
            .unwrap_or((0, 0, 0));
        div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(66.0))
            .bg(rgb(palette::surface_container()))
            .text_size(px(11.0))
            .text_color(rgb(palette::on_surface_variant()))
            .child({
                let visual = if let Some(page) = self.page.as_ref() {
                    page.selection.count(page.cursor)
                } else {
                    self.view.selection.count(self.view.cursor)
                };
                if visual > 0 {
                    format!(
                        "VISUAL {visual} · {} · {} · {}/{}",
                        self.view.scope.label(),
                        self.view.order_label(),
                        counts.0,
                        counts.1
                    )
                } else {
                    format!(
                        "{} · {} · {} artists / {} albums / {} songs",
                        self.view.scope.label(),
                        self.view.order_label(),
                        counts.0,
                        counts.1,
                        counts.2
                    )
                }
            })
    }
}
