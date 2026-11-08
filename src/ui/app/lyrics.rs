//! Lyrics layout, emphasis and transport animation state.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn request_current_lyrics(&mut self) {
        let Some(track) = self.player.current() else {
            return;
        };
        let id = track.id.clone();
        if self
            .lyrics
            .as_ref()
            .is_some_and(|lyrics| lyrics.song_id == id)
        {
            return;
        }
        if self.lyrics_requested.as_deref() == Some(id.as_str()) {
            return;
        }
        self.lyrics_requested = Some(id.clone());
        self.bridge.request_lyrics(id);
    }

    pub(in crate::ui) fn lyrics_inline(&self) -> bool {
        let inline_width = (self.win_w * 0.95 - self.win_w * 0.012) / 2.0;
        self.lyrics
            .as_ref()
            .is_some_and(|lyrics| !lyrics.lines.is_empty())
            && inline_width >= 300.0
    }

    pub(in crate::ui) fn press_transport(&mut self, index: usize) {
        self.clock.tick();
        if let Some(slot) = self.transport_press.get_mut(index) {
            *slot = Some(self.clock.now);
        }
    }

    pub(in crate::ui) fn transport_scale(&self, index: usize) -> f32 {
        let Some(start) = self.transport_press.get(index).copied().flatten() else {
            return 1.0;
        };
        let elapsed = (self.clock.now - start) as f32;
        if elapsed < 0.0 || elapsed >= 0.32 {
            return 1.0;
        }
        if elapsed < 0.08 {
            1.0 - 0.14 * (elapsed / 0.08)
        } else {
            let progress = (elapsed - 0.08) / 0.24;
            0.86 + 0.14 * anim::Curve::Bounce.sample(progress)
        }
    }

    pub(in crate::ui) fn transport_press_live(&self) -> bool {
        self.transport_press.iter().flatten().any(|start| {
            let elapsed = (self.clock.now - start) as f32;
            (0.0..0.32).contains(&elapsed)
        })
    }

    pub(in crate::ui) fn refresh_lyrics_emphasis(&mut self) {
        let Some(lyrics) = self.lyrics.as_ref() else {
            self.lyrics_active_line = None;
            return;
        };
        if lyrics.lines.is_empty() {
            return;
        }
        let active = if lyrics.synced {
            let ms = (self.player.position() * 1000.0) as i64;
            lyrics
                .lines
                .iter()
                .rposition(|(start, _)| start.is_some_and(|start| start <= ms))
                .unwrap_or(0)
        } else {
            0
        };
        self.lyrics_active_line = Some(active);
        for (index, _) in lyrics.lines.iter().enumerate() {
            let opacity = if !lyrics.synced {
                0.8
            } else {
                match index.abs_diff(active) {
                    0 => 1.0,
                    1 => 0.8,
                    _ => 0.5,
                }
            };
            self.lyrics_line_opacity
                .entry(index)
                .or_insert_with(|| anim::Behavior::new(opacity))
                .target(
                    opacity,
                    self.visuals.motion.lyrics_fade_ms,
                    anim::Curve::Standard,
                    &self.clock,
                );

            let emphasis = if lyrics.synced && index == active {
                lyrics_layout::EMPHASIS
            } else {
                1.0
            };
            self.lyrics_line_emphasis
                .entry(index)
                .or_insert_with(|| anim::Behavior::new(1.0))
                .target(
                    emphasis,
                    self.visuals.motion.lyrics_line_ms,
                    anim::Curve::Standard,
                    &self.clock,
                );

            let color = if lyrics.synced && index == active {
                palette::on_surface()
            } else {
                palette::on_surface_variant()
            };
            self.lyrics_line_color
                .entry(index)
                .or_insert_with(|| anim::ColorBehavior::new(color))
                .target(
                    color,
                    self.visuals.motion.lyrics_fade_ms,
                    anim::Curve::Standard,
                    &self.clock,
                );
        }
    }

    pub(in crate::ui) fn lyric_wrap_count(
        window: &Window,
        text: &str,
        size: f32,
        width: f32,
    ) -> usize {
        if width <= 0.0 || text.is_empty() {
            return 1;
        }
        let mut wrapper = window
            .text_system()
            .line_wrapper(gpui::font("Overused Grotesk Roman"), px(size));
        wrapper
            .wrap_line(&[gpui::LineFragment::text(text)], px(width))
            .count()
            .max(1)
    }

    pub(in crate::ui) fn sync_lyrics_layout(&mut self, window: &Window) {
        let Some(lyrics) = self.lyrics.as_ref() else {
            self.lyrics_layout = None;
            return;
        };
        if lyrics.lines.is_empty() {
            self.lyrics_layout = None;
            return;
        }
        let overlay = self.lyrics_overlay;
        let (width, height) = if overlay {
            (
                ((self.win_w * 0.72).min(720.0) - 32.0).max(1.0),
                (self.win_h * 0.80).max(1.0),
            )
        } else if self.fullscreen {
            (
                ((self.win_w * 0.95 - 24.0) * 0.5 - 24.0).max(1.0),
                (self.win_h * 0.55).max(1.0),
            )
        } else {
            self.lyrics_layout = None;
            return;
        };
        let active = self
            .lyrics_active_line
            .unwrap_or(0)
            .min(lyrics.lines.len() - 1);
        let mut base = (height * 0.055).clamp(13.0, 22.0);
        let mut layout =
            lyrics_layout::layout(&lyrics.lines, active, width, height, base, |text, size| {
                Self::lyric_wrap_count(window, text, size, width)
            });
        // A single very long line can still be taller than the pane at the
        // nominal size. Step the type down until it fits, then keep it.
        while base > 13.0 && layout.rows.iter().any(|row| row.height > height) {
            base -= 1.0;
            layout =
                lyrics_layout::layout(&lyrics.lines, active, width, height, base, |text, size| {
                    Self::lyric_wrap_count(window, text, size, width)
                });
        }
        let old_start = self.lyrics_window_start;
        if layout.start != old_start {
            let unit = (layout.text_height + layout.gap).max(1.0);
            self.lyrics_shift.target(
                (old_start as f32 - layout.start as f32) * unit,
                0.0,
                anim::Curve::Standard,
                &self.clock,
            );
            // The phrase transition owns the rhythm: centering glides with
            // the same duration/easing instead of front-loading the motion.
            self.lyrics_shift.target(
                0.0,
                self.visuals.motion.lyrics_line_ms,
                anim::Curve::Standard,
                &self.clock,
            );
        } else {
            self.lyrics_shift
                .target(0.0, 0.0, anim::Curve::Standard, &self.clock);
        }
        self.lyrics_window_start = layout.start;
        self.lyrics_layout = Some(layout);
    }

    pub(in crate::ui) fn refresh_lyrics_overlay(&mut self) {
        if !self.fullscreen {
            self.lyrics_overlay = false;
            self.lyrics_overlay_wanted = false;
            return;
        }
        let has_lyrics = self
            .lyrics
            .as_ref()
            .is_some_and(|lyrics| !lyrics.lines.is_empty());
        let inline = self.lyrics_inline();
        if !has_lyrics {
            self.lyrics_overlay = false;
            return;
        }
        self.lyrics_overlay = self.lyrics_overlay_wanted && !inline;
    }

    pub(in crate::ui) fn on_lyrics_overlay(
        &mut self,
        _: &LyricsOverlay,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fullscreen {
            if self
                .lyrics
                .as_ref()
                .is_some_and(|lyrics| !lyrics.lines.is_empty())
            {
                self.lyrics_overlay_wanted = !self.lyrics_overlay_wanted;
                self.refresh_lyrics_overlay();
            }
        } else if self.page.as_ref().is_some_and(|page| {
            self.notes
                .as_ref()
                .is_some_and(|(id, notes)| *id == page.descriptor.id && !notes.is_empty())
        }) {
            self.toggle_description();
        }
        cx.notify();
    }
}
