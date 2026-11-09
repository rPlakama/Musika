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
        // A manual scroll holds the window on its own anchor; the playback
        // highlight (and its emphasis) still tracks the played line.
        let center = self
            .lyrics_hold
            .unwrap_or(active)
            .min(lyrics.lines.len() - 1);
        let mut base = (height * 0.055).clamp(13.0, 22.0);
        let mut layout = lyrics_layout::layout_windowed(
            &lyrics.lines,
            active,
            center,
            width,
            height,
            base,
            |text, size| Self::lyric_wrap_count(window, text, size, width),
        );
        // A single very long line can still be taller than the pane at the
        // nominal size. Step the type down until it fits, then keep it.
        while base > 13.0 && layout.rows.iter().any(|row| row.height > height) {
            base -= 1.0;
            layout = lyrics_layout::layout_windowed(
                &lyrics.lines,
                active,
                center,
                width,
                height,
                base,
                |text, size| Self::lyric_wrap_count(window, text, size, width),
            );
        }
        let old_start = self.lyrics_window_start;
        if layout.start != old_start {
            let unit = (layout.text_height + layout.gap).max(1.0);
            self.lyrics_shift.target(
                (layout.start as f32 - old_start as f32) * unit,
                0.0,
                anim::Curve::Standard,
                &self.clock,
            );
            // Softer than the phrase transition: a long, calm settle instead
            // of a quick recenter.
            self.lyrics_shift.target(
                0.0,
                self.visuals.motion.lyrics_scroll_ms,
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
            } else {
                self.push_toast("There is no lyrics.".to_string(), ToastKind::Error, None);
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

    /// True when j/k (and the page motions) should scroll the lyrics body
    /// rather than the surface behind the fullscreen player.
    pub(in crate::ui) fn lyrics_scroll_focused(&self) -> bool {
        self.fullscreen
            && self
                .lyrics
                .as_ref()
                .is_some_and(|lyrics| !lyrics.lines.is_empty())
            && (self.lyrics_overlay
                || (self.fullscreen_focus == FullscreenFocus::Lyrics && self.lyrics_inline()))
    }

    /// The line the pane is currently centred on.
    fn lyrics_center_line(&self) -> Option<usize> {
        let count = self.lyrics.as_ref()?.lines.len();
        if count == 0 {
            return None;
        }
        if let Some(anchor) = self.lyrics_hold {
            return Some(anchor.min(count - 1));
        }
        if let Some(layout) = self.lyrics_layout.as_ref() {
            return Some(layout.center.min(count - 1));
        }
        Some(self.lyrics_active_line.unwrap_or(0).min(count - 1))
    }

    /// One `j`/`k` step: 30% of the visible rows, the legacy UI lyric-scroll
    /// grammar (`scrollLyricsBy` in the old NowPlaying pane).
    pub(in crate::ui) fn lyrics_line_step(&self) -> i32 {
        self.lyrics_layout
            .as_ref()
            .map(|layout| ((layout.rows.len() as f32 * 0.30).round() as i32).max(1))
            .unwrap_or(1)
    }

    /// One page step: half the visible rows, like the legacy `page_down`.
    pub(in crate::ui) fn lyrics_page_step(&self) -> i32 {
        self.lyrics_layout
            .as_ref()
            .map(|layout| ((layout.rows.len() as f32 * 0.50).round() as i32).max(1))
            .unwrap_or(1)
    }

    /// Release a manual lyric scroll: the next layout follows the played line
    /// again. Superseded resume timers are retired by the generation bump.
    pub(in crate::ui) fn release_lyrics_hold(&mut self) {
        self.lyrics_hold = None;
        self.lyrics_hold_generation = self.lyrics_hold_generation.wrapping_add(1);
    }

    /// Anchor the pane on `line` for a manual scroll and arm the idle resume
    /// (LYR-8): the follow returns after 5 s with the pane focused, 2 s when
    /// focus is elsewhere.
    pub(in crate::ui) fn hold_lyrics_at(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(count) = self
            .lyrics
            .as_ref()
            .map(|lyrics| lyrics.lines.len())
            .filter(|count| *count > 0)
        else {
            return;
        };
        let anchor = line.min(count - 1);
        self.lyrics_hold = Some(anchor);
        self.lyrics_hold_generation = self.lyrics_hold_generation.wrapping_add(1);
        let generation = self.lyrics_hold_generation;
        let focused = self.lyrics_overlay
            || (self.fullscreen_focus == FullscreenFocus::Lyrics && self.lyrics_inline());
        let resume_ms = if focused {
            anim::LYRICS_RESUME_MS
        } else {
            anim::LYRICS_RESUME_UNFOCUSED_MS
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(resume_ms))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.lyrics_hold_generation == generation {
                    this.lyrics_hold = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Scroll the lyric body by `lines` (positive = down).
    pub(in crate::ui) fn scroll_lyrics_by(&mut self, lines: i32, cx: &mut Context<Self>) {
        let Some(current) = self.lyrics_center_line() else {
            return;
        };
        let Some(count) = self.lyrics.as_ref().map(|lyrics| lyrics.lines.len()) else {
            return;
        };
        let next = (current as i64 + i64::from(lines)).clamp(0, count as i64 - 1) as usize;
        self.hold_lyrics_at(next, cx);
    }

    /// `g g` in the lyric body: hold the first line.
    pub(in crate::ui) fn lyrics_top(&mut self, cx: &mut Context<Self>) {
        self.hold_lyrics_at(0, cx);
    }

    /// `G` in the lyric body: hold the last line.
    pub(in crate::ui) fn lyrics_bottom(&mut self, cx: &mut Context<Self>) {
        let last = self
            .lyrics
            .as_ref()
            .map(|lyrics| lyrics.lines.len().saturating_sub(1))
            .unwrap_or(0);
        self.hold_lyrics_at(last, cx);
    }

    /// Mouse wheel over the lyrics body: convert the pixel delta into whole
    /// rows and stop the event so the list behind the fullscreen player does
    /// not scroll as well.
    pub(in crate::ui) fn on_lyrics_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(layout) = self.lyrics_layout.as_ref() else {
            return;
        };
        let unit = (layout.text_height + layout.gap).max(1.0);
        let delta = f32::from(event.delta.pixel_delta(window.line_height()).y);
        if delta == 0.0 {
            return;
        }
        // Wayland/X11 wheel-down is a negative pixel delta; lyric lines grow
        // downward, so invert it.
        let mut steps = (-delta / unit).round() as i32;
        if steps == 0 {
            steps = -delta.signum() as i32;
        }
        self.scroll_lyrics_by(steps, cx);
        cx.notify();
    }
}
