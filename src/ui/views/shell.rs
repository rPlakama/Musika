//! Top bar, status bar, player bar and login view.

use super::super::*;
use gpui::StatefulInteractiveElement;

/// Identity of the currently playing cover. The same album keeps the same
/// thumb and full-resolution sources (and skips every reload/fade) across its
/// tracks, even when per-song cover ids/URLs differ.
#[derive(Default)]
pub(in crate::ui) struct CurrentCover {
    key: Option<String>,
    thumb: Option<CoverSource>,
    full: Option<CoverSource>,
    /// Stable cover-art id used by the palette; filled from the first track of
    /// the identity that carries one.
    cover_art: String,
    changed: bool,
}

impl CurrentCover {
    /// Resolves the current identity. Both resolvers run only when the key
    /// changed; a repeated key reuses both sources and reports `false`.
    ///
    /// Some songs in one album have no `cover_art`; when that happens for a
    /// repeated identity the id is adopted later without invalidating the
    /// sources (the palette updates once, the cover does not reload).
    pub(in crate::ui) fn resolve(
        &mut self,
        key: String,
        cover_art: &str,
        resolve_thumb: impl FnOnce() -> Option<CoverSource>,
        resolve_full: impl FnOnce() -> Option<CoverSource>,
    ) -> bool {
        if self.key.as_deref() == Some(key.as_str()) {
            if self.cover_art.is_empty() && !cover_art.is_empty() {
                self.cover_art = cover_art.to_string();
            }
            self.changed = false;
            return false;
        }
        self.thumb = resolve_thumb();
        self.full = resolve_full();
        self.key = Some(key);
        self.cover_art = cover_art.to_string();
        self.changed = true;
        true
    }

    pub(in crate::ui) fn thumb(&self) -> Option<CoverSource> {
        self.thumb.clone()
    }

    pub(in crate::ui) fn full(&self) -> Option<CoverSource> {
        self.full.clone()
    }

    pub(in crate::ui) fn cover_art(&self) -> &str {
        &self.cover_art
    }

    pub(in crate::ui) fn changed(&self) -> bool {
        self.changed
    }

    pub(in crate::ui) fn clear(&mut self) {
        self.key = None;
        self.thumb = None;
        self.full = None;
        self.cover_art.clear();
        self.changed = false;
    }
}

/// Cover identity for the playing track: the album wins so every track of one
/// album shares one cover; singles fall back to the cover-art id/URL.
pub(in crate::ui) fn current_cover_key(track: &crate::player::Track) -> String {
    if !track.album_id.is_empty() {
        format!("album:{}", track.album_id)
    } else if !track.cover_art.is_empty() {
        format!("cover-art:{}", track.cover_art)
    } else if !track.album.is_empty() {
        format!("album-name:{}|{}", track.album, track.artist)
    } else {
        format!("cover:{}|{}", track.cover_url, track.cover_remote)
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
                        spinning_icon(
                            "topbar-download-spin",
                            slot,
                            "loading-loop.svg",
                            18.0,
                            palette::on_surface_variant(),
                        ),
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
                    div().w_full().overflow_hidden().child(MarqueeText::new(
                        "topbar-welcome",
                        0,
                        welcome,
                        19.0,
                        400.0,
                        if offline {
                            palette::tertiary()
                        } else {
                            palette::on_surface_variant()
                        },
                    )),
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
        // Natural-width identity row inside a clipped horizontal viewport;
        // the marquee offset is driven once per render by
        // `refresh_topbar_marquee`. Action chips below stay pinned right.
        let mut identity = div().flex().flex_none().items_center().gap_2().child(
            div()
                .flex_none()
                .text_size(px(19.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(palette::on_surface()))
                .whitespace_nowrap()
                .child(title),
        );
        if !page.subtitle.is_empty() {
            identity = identity.child(self.bar_separator()).child(
                div()
                    .flex_none()
                    .text_size(px(14.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .whitespace_nowrap()
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
                    .child(page.meta.to_string()),
            );
        }
        let identity_viewport = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(
                div()
                    .id("topbar-identity-scroll")
                    .overflow_x_scroll()
                    .scrollbar_width(px(0.0))
                    .track_scroll(&self.topbar_scroll)
                    .child(identity),
            );
        let mut row = div()
            .flex()
            .items_center()
            .w_full()
            .h_full()
            .px_4()
            .gap_2()
            .child(identity_viewport);
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

    /// Hashes the identity the top bar shows; a change restarts the marquee.
    fn topbar_identity_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(page) = self.page_bar() {
            page.kind.label().hash(&mut hasher);
            page.title.hash(&mut hasher);
            page.subtitle.hash(&mut hasher);
            page.meta.hash(&mut hasher);
        } else {
            self.header
                .as_ref()
                .map(|header| header.username.as_str())
                .unwrap_or("")
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Drives the top-bar identity viewport. Short identities stay at offset
    /// zero; an overflowing one slides exactly like the bottom-bar marquee.
    pub(in crate::ui) fn refresh_topbar_marquee(&mut self, window: &gpui::Window) {
        let signature = self.topbar_identity_signature();
        if signature != self.topbar_identity_signature {
            self.topbar_identity_signature = signature;
            self.topbar_marquee_started = std::time::Instant::now();
            self.topbar_scroll
                .set_offset(gpui::point(px(0.0), px(0.0)));
            // The content width is only known after this frame's layout.
            perf::request_frame(window, "topbar-marquee");
            return;
        }
        let overflow = f32::from(self.topbar_scroll.max_offset().width);
        if overflow <= 0.5 {
            self.topbar_scroll
                .set_offset(gpui::point(px(0.0), px(0.0)));
            return;
        }
        let elapsed = self.topbar_marquee_started.elapsed().as_secs_f32();
        let offset = marquee_offset(overflow, elapsed);
        self.topbar_scroll
            .set_offset(gpui::point(px(-offset), px(0.0)));
        perf::request_frame(window, "topbar-marquee");
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

    /// Refreshes the playing-cover identity once per render, before any
    /// immutable view borrows. Same-album track changes reuse both the thumb
    /// and the full-resolution source; neither surface reloads or refades.
    pub(in crate::ui) fn refresh_current_cover(&mut self) {
        let Some(track) = self.player.current().cloned() else {
            self.current_cover.clear();
            return;
        };
        let key = current_cover_key(&track);
        let url = track.cover_url.clone();
        let remote = track.cover_remote.clone();
        self.current_cover.resolve(
            key,
            &track.cover_art,
            || cover_source_with_remote(&url, &remote),
            || full_cover_source_with_remote(&url, &remote),
        );
    }

    /// One cover-painting element for every surface. The shared image cache is
    /// the instance store: the same cover decodes once and every view paints
    /// the same `Arc<RenderImage>`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the helper bridges the cover geometry to CoverImage; a builder would obscure it"
    )]
    pub(in crate::ui) fn cover_view(
        &self,
        id_name: &'static str,
        index: usize,
        source: Option<CoverSource>,
        width: f32,
        height: f32,
        scale: f32,
        fit: gpui::ObjectFit,
        radius: f32,
    ) -> gpui::AnyElement {
        CoverImage::new(
            id_name,
            index,
            source.map(std::sync::Arc::new),
            self.image_cache.clone(),
            width,
            height,
            scale,
            fit,
            radius,
        )
        .into_any_element()
    }

    /// [`AppRoot::cover_view`] wrapped in a one-shot fade when the cover
    /// identity changed; a same-album track change passes `fade_id: None` and
    /// paints the stable instance without replaying the animation.
    #[expect(
        clippy::too_many_arguments,
        reason = "the helper bridges the cover geometry and optional fade identity"
    )]
    pub(in crate::ui) fn cover_view_fading(
        &self,
        id_name: &'static str,
        index: usize,
        source: Option<CoverSource>,
        width: f32,
        height: f32,
        fit: gpui::ObjectFit,
        radius: f32,
        fade_id: Option<gpui::SharedString>,
    ) -> gpui::AnyElement {
        let image = CoverImage::new(
            id_name,
            index,
            source.map(std::sync::Arc::new),
            self.image_cache.clone(),
            width,
            height,
            1.0,
            fit,
            radius,
        );
        match fade_id {
            Some(id) => div()
                .child(image)
                .with_animation(
                    id,
                    Animation::new(std::time::Duration::from_millis(120)),
                    |this, delta| this.opacity(delta),
                )
                .into_any_element(),
            None => image.into_any_element(),
        }
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
        let cover = self.current_cover.thumb();
        let cover_changed = self.current_cover.changed();
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
                            .child(self.cover_view_fading(
                                "player-cover",
                                0,
                                cover,
                                m.cover_side,
                                m.cover_side,
                                gpui::ObjectFit::Contain,
                                4.0,
                                cover_changed.then(|| {
                                    gpui::SharedString::from(format!(
                                        "cover-fade:player:{}",
                                        self.current_cover
                                            .thumb()
                                            .as_ref()
                                            .map_or_else(|| "none".to_string(), |source| source.key())
                                    ))
                                }),
                            ))
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

    fn track(album_id: &str, cover_art: &str, cover_url: &str) -> crate::player::Track {
        let mut track = crate::player::Track {
            item: crate::model::ItemMeta {
                id: "s".into(),
                title: String::new(),
                cover_url: cover_url.into(),
                cover_remote: String::new(),
                album_id: album_id.into(),
            },
            artist: String::new(),
            album: "Album".into(),
            favorite: false,
            duration: 0.0,
            url: String::new(),
            cover_art: cover_art.into(),
        };
        track.item.cover_url = cover_url.into();
        track
    }

    #[test]
    fn cover_key_prefers_album_then_art_then_name_then_url() {
        assert_eq!(
            current_cover_key(&track("a1", "art", "/song.jpg")),
            "album:a1"
        );
        assert_eq!(
            current_cover_key(&track("", "art", "/song.jpg")),
            "cover-art:art"
        );
        assert_eq!(
            current_cover_key(&track("", "", "/song.jpg")),
            "album-name:Album|"
        );
        let mut no_album = track("", "", "/song.jpg");
        no_album.album.clear();
        assert_eq!(current_cover_key(&no_album), "cover:/song.jpg|");
    }

    #[test]
    fn same_album_reuses_thumb_and_full_without_resolving() {
        let mut cache = CurrentCover::default();
        let mut thumb_calls = 0;
        let mut full_calls = 0;
        assert!(cache.resolve(
            "album:a".into(),
            "art-a",
            || {
                thumb_calls += 1;
                Some(source("/a.thumb.jpg"))
            },
            || {
                full_calls += 1;
                Some(source("/a.jpg"))
            },
        ));
        assert!(cache.changed());
        assert_eq!(cache.thumb().as_ref().map(CoverSource::key), Some("/a.thumb.jpg".into()));
        assert_eq!(cache.full().as_ref().map(CoverSource::key), Some("/a.jpg".into()));

        // A different track of the same album has a different per-song URL:
        // neither source may be re-resolved.
        assert!(!cache.resolve(
            "album:a".into(),
            "art-a",
            || {
                thumb_calls += 1;
                Some(source("/b.thumb.jpg"))
            },
            || {
                full_calls += 1;
                Some(source("/b.jpg"))
            },
        ));
        assert!(!cache.changed());
        assert_eq!(cache.thumb().as_ref().map(CoverSource::key), Some("/a.thumb.jpg".into()));
        assert_eq!(cache.full().as_ref().map(CoverSource::key), Some("/a.jpg".into()));
        assert_eq!((thumb_calls, full_calls), (1, 1));
    }

    #[test]
    fn new_album_resolves_both_sources_and_fades() {
        let mut cache = CurrentCover::default();
        let _ = cache.resolve(
            "album:a".into(),
            "art-a",
            || Some(source("/a.thumb.jpg")),
            || Some(source("/a.jpg")),
        );
        assert!(cache.resolve(
            "album:b".into(),
            "art-b",
            || Some(source("/b.thumb.jpg")),
            || Some(source("/b.jpg")),
        ));
        assert!(cache.changed());
        assert_eq!(cache.thumb().as_ref().map(CoverSource::key), Some("/b.thumb.jpg".into()));
        assert_eq!(cache.full().as_ref().map(CoverSource::key), Some("/b.jpg".into()));
        cache.clear();
        assert!(!cache.changed());
        assert!(cache.thumb().is_none() && cache.full().is_none());
    }

    #[test]
    fn late_cover_art_updates_the_palette_id_without_reloading() {
        let mut cache = CurrentCover::default();
        let _ = cache.resolve(
            "album:a".into(),
            "",
            || Some(source("/a.thumb.jpg")),
            || Some(source("/a.jpg")),
        );
        assert_eq!(cache.cover_art(), "");
        let _ = cache.resolve(
            "album:a".into(),
            "art-a",
            || panic!("same identity must not re-resolve the thumb"),
            || panic!("same identity must not re-resolve the full cover"),
        );
        assert_eq!(cache.cover_art(), "art-a");
        assert!(!cache.changed());
    }
}
