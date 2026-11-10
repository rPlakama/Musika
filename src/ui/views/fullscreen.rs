//! Fullscreen view, lyrics pane and transport controls.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn lyric_line_elements(
        &self,
        layout: &lyrics_layout::LyricsLayout,
        lines: &[(Option<i64>, String)],
        synced: bool,
    ) -> Vec<gpui::AnyElement> {
        layout
            .rows
            .iter()
            .map(|row| {
                let is_active = synced && row.active;
                let emphasis = self
                    .lyrics_line_emphasis
                    .get(&row.index)
                    .map(|anim| anim.value(&self.clock))
                    .unwrap_or(1.0);
                let color = self
                    .lyrics_line_color
                    .get(&row.index)
                    .map(|anim| anim.value(&self.clock))
                    .unwrap_or_else(|| {
                        if is_active {
                            palette::on_surface()
                        } else {
                            palette::on_surface_variant()
                        }
                    });
                let opacity = self
                    .lyrics_line_opacity
                    .get(&row.index)
                    .map(|anim| anim.value(&self.clock))
                    .unwrap_or(1.0);
                let text = lines
                    .get(row.index)
                    .map(|(_, text)| text.clone())
                    .unwrap_or_default();
                div()
                    .w(px(layout.width))
                    .min_w_0()
                    .h(px(row.height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(layout.base_size * emphasis))
                    .line_height(px(layout.text_height * emphasis))
                    .text_color(rgb(color))
                    .text_center()
                    .whitespace_normal()
                    .opacity(opacity)
                    .child(div().w_full().text_center().whitespace_normal().child(text))
                    .into_any_element()
            })
            .collect()
    }

    pub(in crate::ui) fn lyrics_overlay_element(
        &self,
        progress: f32,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        if !self.lyrics_overlay && progress <= 0.001 {
            return div().into_any_element();
        }
        let Some(lyrics) = self.lyrics.as_ref() else {
            return div().into_any_element();
        };
        let Some(layout) = self.lyrics_layout.as_ref() else {
            return div().into_any_element();
        };
        div()
            .absolute()
            .inset_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(self.visuals.motion.lyrics_scrim * progress))
            .opacity(progress.clamp(0.0, 1.0))
            .on_scroll_wheel(cx.listener(Self::on_lyrics_wheel))
            .child(
                div()
                    .w(relative(0.72))
                    .max_w(px(720.0))
                    .h_full()
                    .px_4()
                    .py_3()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(layout.gap))
                    .opacity(self.lyrics_track_fade.value(&self.clock))
                    .relative()
                    .top(px(self.lyrics_shift.value(&self.clock)))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .child(format!(
                                "Lyrics · {} · j/k scroll · ? closes",
                                lyrics.source
                            )),
                    )
                    .children(self.lyric_line_elements(layout, &lyrics.lines, lyrics.synced)),
            )
            .into_any_element()
    }

    pub(in crate::ui) fn fullscreen_overlay(&self, progress: f32, cx: &Context<Self>) -> gpui::Div {
        let track = self.player.current().cloned();
        let (title, artist, cover) = match &track {
            Some(track) => {
                let local = self
                    .display_cover(&track.cover_art, self.fullscreen_hero_bucket())
                    .map(CoverSource::Path)
                    .or_else(|| self.hero_cover(&track.cover_url, self.fullscreen_hero_bucket()));
                let cover = match local {
                    Some(source) => Some(source.with_remote_fallback(&track.cover_remote)),
                    None => remote_cover(&track.cover_remote).map(CoverSource::Remote),
                };
                (track.title.clone(), track.artist.clone(), cover)
            }
            None => ("No track playing".to_string(), String::new(), None),
        };
        let title_size = (metrics::font_size("headline", self.win_h)
            * metrics::width_scale(self.win_w))
        .clamp(18.0, 32.0);
        let favorite = track.as_ref().is_some_and(|track| track.favorite);
        let pending = track
            .as_ref()
            .is_some_and(|track| self.marks.pending(&track.id, &track.album_id));
        let downloaded = track
            .as_ref()
            .is_some_and(|track| self.marks.downloaded(&track.id, &track.album_id));
        let cover_focus = self.fullscreen_focus == FullscreenFocus::Cover;
        let title_focus = self.fullscreen_focus == FullscreenFocus::Title;
        let lyrics_focus = self.fullscreen_focus == FullscreenFocus::Lyrics;
        let cover_anchor_slot = self.fullscreen_anchors.clone();
        let title_anchor_slot = self.fullscreen_anchors.clone();
        let position = self.player.position();
        let duration = self.player.duration();
        let progress_ratio = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let lyrics = self.lyrics.as_ref();
        let lines = lyrics
            .map(|lyrics| lyrics.lines.clone())
            .unwrap_or_default();
        let lyrics_label = match lyrics {
            Some(lyrics) if lyrics.lines.is_empty() => "No lyrics".to_string(),
            Some(lyrics) if lyrics_focus => {
                format!("Lyrics · {} · j/k scroll", lyrics.source)
            }
            Some(lyrics) => format!("Lyrics · {}", lyrics.source),
            None => "Loading lyrics…".to_string(),
        };
        let lyrics_inline = self.lyrics_inline();
        let lyrics_synced = lyrics.is_some_and(|lyrics| lyrics.synced);
        let lyric_gap = self
            .lyrics_layout
            .as_ref()
            .map(|layout| layout.gap)
            .unwrap_or(12.0);
        let lyric_children = self
            .lyrics_layout
            .as_ref()
            .map(|layout| self.lyric_line_elements(layout, &lines, lyrics_synced))
            .unwrap_or_default();
        // The overlay is a centered lyrics panel over the full player: keep the
        // cover as the backdrop instead of hiding it and splitting the pane.
        let split_lyrics = lyrics_inline && !self.lyrics_overlay;
        let cover_side = if split_lyrics {
            ((self.win_w * 0.95 - 24.0) * 0.5).min(self.win_h * 0.62)
        } else {
            (self.win_w * 0.95).min(self.win_h * 0.62)
        };
        div()
            .absolute()
            .inset_0()
            .size_full()
            .opacity(progress)
            .bg(rgb(palette::background()))
            .flex()
            .flex_col()
            .p(px(16.0))
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .w_full()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if cover_focus {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .on_children_prepainted(move |bounds, _, _| {
                                if let Some(bounds) = bounds.first() {
                                    let x = f32::from(bounds.origin.x)
                                        + f32::from(bounds.size.width) * 0.5;
                                    let y = f32::from(bounds.origin.y)
                                        + f32::from(bounds.size.height) * 0.5;
                                    cover_anchor_slot.borrow_mut().cover = Some((x, y));
                                }
                            })
                            .child(art(cover, cover_side)),
                    )
                    .child(
                        div()
                            .flex()
                            .when(split_lyrics, |this| this.flex_1())
                            .when(!split_lyrics, |this| this.hidden())
                            .h_full()
                            .opacity(if split_lyrics {
                                self.lyrics_track_fade.value(&self.clock)
                            } else {
                                0.0
                            })
                            .min_w_0()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if lyrics_focus {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(lyric_gap))
                            .relative()
                            .top(px(self.lyrics_shift.value(&self.clock)))
                            .on_scroll_wheel(cx.listener(Self::on_lyrics_wheel))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(lyrics_label),
                            )
                            .children(lyric_children),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .w_full()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if title_focus {
                                palette::primary()
                            } else {
                                palette::outline_variant()
                            }))
                            .px_2()
                            .py_1()
                            .on_children_prepainted(move |bounds, _, _| {
                                if let Some(first) = bounds.first() {
                                    let mut left = f32::from(first.origin.x);
                                    let mut right = left + f32::from(first.size.width);
                                    let mut top = f32::from(first.origin.y);
                                    let mut bottom = top + f32::from(first.size.height);
                                    for bounds in bounds.iter().skip(1) {
                                        left = left.min(f32::from(bounds.origin.x));
                                        right = right.max(
                                            f32::from(bounds.origin.x)
                                                + f32::from(bounds.size.width),
                                        );
                                        top = top.min(f32::from(bounds.origin.y));
                                        bottom = bottom.max(
                                            f32::from(bounds.origin.y)
                                                + f32::from(bounds.size.height),
                                        );
                                    }
                                    title_anchor_slot
                                        .borrow_mut()
                                        .title = Some(((left + right) * 0.5, (top + bottom) * 0.5));
                                }
                            })
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div().flex_1().min_w_0().overflow_hidden().child(
                                            MarqueeText::new(
                                                "fullscreen-title",
                                                0,
                                                title,
                                                title_size,
                                                700.0,
                                                palette::on_surface(),
                                            ),
                                        ),
                                    )
                                    .when(favorite, |this| {
                                        this.child(icon(
                                            "baseline-favorite.svg",
                                            (title_size * 0.62).clamp(13.0, 19.0),
                                            0xFFD54F,
                                        ))
                                    })
                                    .when(pending, |this| {
                                        this.child(icon(
                                            "loading-loop.svg",
                                            (title_size * 0.58).clamp(12.0, 18.0),
                                            palette::primary(),
                                        ))
                                    })
                                    .when(downloaded && !pending, |this| {
                                        this.child(icon(
                                            "outline-download.svg",
                                            (title_size * 0.58).clamp(12.0, 18.0),
                                            palette::primary(),
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        div().w_full().overflow_hidden().child(MarqueeText::new(
                            "fullscreen-artist",
                            0,
                            artist,
                            13.0,
                            400.0,
                            palette::on_surface_variant(),
                        )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(format_time(position)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(rgb(palette::surface_container_high()))
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(progress_ratio))
                                            .rounded_full()
                                            .bg(rgb(palette::primary())),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(palette::on_surface_variant()))
                                    .child(format_time(duration)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_5()
                            .child(transport_button("skip-previous-solid-svgrepo-com.svg", 22.0, palette::on_surface(), self.transport_scale(0)))
                            .child(if self.player.is_loading() {
                                transport_loading_button(
                                    "loading-loop.svg",
                                    28.0,
                                    palette::on_surface(),
                                    self.transport_scale(1),
                                    self.stream_loading_angle(),
                                )
                            } else {
                                transport_button(
                                    if self.player.state() == crate::player::State::Playing {
                                        "pause-svgrepo-com.svg"
                                    } else {
                                        "play-1001-svgrepo-com.svg"
                                    },
                                    28.0,
                                    palette::on_surface(),
                                    self.transport_scale(1),
                                )
                            })
                            .child(transport_button("skip-next-circle-solid-svgrepo-com.svg", 22.0, palette::on_surface(), self.transport_scale(2))),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette::on_surface_variant()))
                            .text_center()
                            .child("Tab cover/title/lyrics · Alt actions · Shift-Enter/q closes · Space play/pause · Ctrl-Q queue"),
                    ),
            )
    }
}
