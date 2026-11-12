//! Top bar, status bar, player bar and login view.

use super::super::*;

/// Bottom-bar cover identity cache: the same album must keep the same cover
/// source (and skip the fade/reload) across its tracks.
#[derive(Default)]
pub(in crate::ui) struct PlayerBarCover {
    key: Option<String>,
    source: Option<CoverSource>,
    changed: bool,
}

impl PlayerBarCover {
    /// Returns the cover and whether the album/cover identity changed. A
    /// repeated key reuses the source and never re-runs the resolver.
    pub(in crate::ui) fn resolve(
        &mut self,
        key: String,
        resolve: impl FnOnce() -> Option<CoverSource>,
    ) -> Option<CoverSource> {
        if self.key.as_deref() == Some(key.as_str()) {
            self.changed = false;
            return self.source.clone();
        }
        let source = resolve();
        self.key = Some(key);
        self.source = source.clone();
        self.changed = true;
        source
    }

    pub(in crate::ui) fn source(&self) -> Option<CoverSource> {
        self.source.clone()
    }

    pub(in crate::ui) fn changed(&self) -> bool {
        self.changed
    }

    pub(in crate::ui) fn clear(&mut self) {
        self.key = None;
        self.source = None;
        self.changed = false;
    }
}

impl AppRoot {
    pub(in crate::ui) fn topbar_action_chip(
        &self,
        action: TopBarAction,
        page: Option<&PageBarState<'_>>,
        anchors: StatusAnchorSlots,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let slot = action.slot();
        let focused = self.status_focus == Some(slot);
        let (icon_el, opacity) = match action {
            TopBarAction::Sync => {
                // The spinner is a sync-in-progress affordance only: it spins
                // while the sync runs (and while the settle bounce finishes),
                // then retracts to the static sync glyph.
                let color = if self.syncing {
                    palette::on_surface_variant()
                } else {
                    palette::on_surface()
                };
                let opacity = if self.syncing { 0.38 } else { 0.85 };
                if self.syncing || !self.sync_settle.settled(&self.clock) {
                    (
                        icon_rotated("loading-loop.svg", 18.0, color, self.sync_angle()),
                        opacity,
                    )
                } else {
                    (icon("reload-ui-2-svgrepo-com.svg", 18.0, color), opacity)
                }
            }
            TopBarAction::Wipe => (icon("delete-outline.svg", 18.0, palette::on_surface()), 1.0),
            TopBarAction::Download => {
                let cached = page.is_some_and(|page| page.cached);
                let downloading = page.is_some_and(|page| page.downloading);
                if downloading {
                    (
                        icon("loading-loop.svg", 18.0, palette::on_surface_variant()),
                        0.85,
                    )
                } else {
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
                        1.0,
                    )
                }
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
        let sync_label = self
            .header
            .as_ref()
            .and_then(|header| header.last_sync.as_deref())
            .map(last_sync_text)
            .unwrap_or_else(|| "—".to_string());
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
                    .children({
                        let mut children = Vec::new();
                        for action in self.topbar_actions() {
                            children.push(self.topbar_action_chip(
                                action,
                                None,
                                self.status_anchors.clone(),
                                cx,
                            ));
                            if action == TopBarAction::Sync {
                                children.push(
                                    div()
                                        .flex_none()
                                        .text_size(px(14.0))
                                        .text_color(rgb(if self.syncing {
                                            palette::tertiary()
                                        } else {
                                            palette::on_surface_variant()
                                        }))
                                        .whitespace_nowrap()
                                        .child(sync_label.clone())
                                        .into_any_element(),
                                );
                            }
                        }
                        children
                    }),
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
            page.title.to_string()
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
                    .child(page.subtitle.to_string()),
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
                    .child(page.meta.to_string()),
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
            SuperiorMode::Home | SuperiorMode::Playlist => {}
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

    /// Refreshes the bottom-bar cover identity once per render, before any
    /// immutable view borrows. Same-album skips reuse the previous source and
    /// do not replay the fade animation.
    pub(in crate::ui) fn refresh_player_bar_cover(&mut self) {
        let Some(track) = self.player.current().cloned() else {
            self.player_bar_cover.clear();
            return;
        };
        let key = if !track.album_id.is_empty() {
            format!("album:{}", track.album_id)
        } else if !track.album.is_empty() {
            format!("album-name:{}|{}", track.album, track.artist)
        } else {
            format!("cover:{}|{}", track.cover_url, track.cover_remote)
        };
        let url = track.cover_url.clone();
        let remote = track.cover_remote.clone();
        self.player_bar_cover
            .resolve(key, || cover_source_with_remote(&url, &remote));
    }

    pub(in crate::ui) fn player_bar(&self) -> impl IntoElement {
        let track = self.player.current().cloned();
        let state = self.player.state();
        let position = self.player.position_now();
        let duration = self.player.duration();
        let volume = self.player.volume();
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let cover = self.player_bar_cover.source();
        let cover_changed = self.player_bar_cover.changed();
        let (title, artist) = match &track {
            Some(track) => (track.title.clone(), track.artist.clone()),
            None => ("Nothing playing".to_string(), String::new()),
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
                    .child(progress_bar(progress, self.seek_preview(), 3.0, false)),
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
                            .child(cover_element(cover, m.cover_side, cover_changed))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(div().w_full().overflow_hidden().child(
                                        MarqueeText::new(
                                            "player-title",
                                            0,
                                            title,
                                            m.title_size,
                                            500.0,
                                            palette::on_surface(),
                                        ),
                                    ))
                                    .when(!artist.is_empty(), |this| {
                                        this.child(div().w_full().overflow_hidden().child(
                                            MarqueeText::new(
                                                "player-artist",
                                                0,
                                                artist,
                                                m.artist_size,
                                                400.0,
                                                palette::on_surface_variant(),
                                            ),
                                        ))
                                    }),
                            ),
                    )
                    .child(transport_controls(
                        state,
                        self.player.is_loading(),
                        self.stream_loading_angle(),
                        [
                            self.transport_scale(0),
                            self.transport_scale(1),
                            self.transport_scale(2),
                        ],
                        m.transport_unit,
                        m.play_size,
                        m.transport_spacing,
                    ))
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
                    page.list.selection.count(page.list.cursor)
                } else {
                    self.view.list.selection.count(self.view.list.cursor)
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

#[cfg(test)]
mod cover_cache_tests {
    use super::*;

    fn source(path: &str) -> CoverSource {
        CoverSource::Path(std::path::PathBuf::from(path))
    }

    #[test]
    fn same_album_reuses_cover_without_resolving() {
        let mut cache = PlayerBarCover::default();
        let mut calls = 0;
        let first = cache.resolve("album:a".into(), || {
            calls += 1;
            Some(source("/a.jpg"))
        });
        assert!(cache.changed());
        assert_eq!(first.as_ref().map(CoverSource::key), Some("/a.jpg".into()));
        let second = cache.resolve("album:a".into(), || {
            calls += 1;
            Some(source("/different.jpg"))
        });
        assert!(!cache.changed());
        assert_eq!(second.as_ref().map(CoverSource::key), Some("/a.jpg".into()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn new_album_fades_and_clears() {
        let mut cache = PlayerBarCover::default();
        let _ = cache.resolve("album:a".into(), || Some(source("/a.jpg")));
        let b = cache.resolve("album:b".into(), || Some(source("/b.jpg")));
        assert!(cache.changed());
        assert_eq!(b.as_ref().map(CoverSource::key), Some("/b.jpg".into()));
        cache.clear();
        assert!(!cache.changed());
        let _ = cache.resolve("album:b".into(), || Some(source("/b.jpg")));
        assert!(cache.changed());
    }
}
