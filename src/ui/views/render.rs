//! Root render composition.

use super::super::*;

impl Render for AppRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame_started = std::time::Instant::now();
        self.frames = self.frames.wrapping_add(1);
        let viewport = window.viewport_size();
        let viewport_w = f32::from(viewport.width);
        let viewport_h = f32::from(viewport.height);
        let previous_size = self.resize_last_size;
        self.win_w = viewport_w;
        self.win_h = viewport_h;
        self.scale_factor = window.scale_factor();
        perf::set_window_active(window.is_window_active());
        self.refresh_lyrics_overlay();
        if self.trace && self.frames.is_multiple_of(120) {
            crate::debug_log!("GPUI trace: frame {}", self.frames);
        }
        self.clock.tick();
        let resized = previous_size.is_some_and(|(width, height)| {
            (width - viewport_w).abs() > 0.5 || (height - viewport_h).abs() > 0.5
        });
        self.resize_last_size = Some((viewport_w, viewport_h));
        let recenter = resize_recenter_now(resized, &mut self.resize_recenter);
        if resized {
            // The list lays the new viewport out during this frame; re-center
            // on the next available frame instead of after a fixed delay.
            perf::request_frame(window, "resize");
        }
        if recenter {
            self.recenter_after_resize();
        }
        #[cfg(debug_assertions)]
        self.frame_hud.observe(self.clock.dt);
        palette::publish(self.palette_motion.value(&self.clock));
        if !self.palette_motion.settled(&self.clock) {
            perf::request_frame(window, "palette");
        }
        self.sync_palette();
        let playback_index = self.player.index();
        if playback_index != self.last_player_index {
            self.last_player_index = playback_index;
            if self.queue_open {
                self.land_queue(playback_index);
            }
        }
        self.refresh_lyrics_emphasis();
        self.sync_lyrics_layout(window);
        if self.exiting.is_none()
            && let Some(ms) = self.quit_after_ms
            && self.clock.now * 1000.0 >= ms as f64
        {
            self.exiting = Some(self.clock.now);
        }
        if self.login.is_some() {
            self.pending_ipc.clear();
            // The login form is keyboard-only. Keep it inside the root key
            // handler: returning the bare view here would bypass both
            // `track_focus` and `on_key_down`, leaving the fields uneditable.
            self.focus.focus(window);
            let login = div()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_key_down))
                .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
                .size_full()
                .child(self.login_view(window));
            #[cfg(debug_assertions)]
            let login = login.child(debug::hud_enabled().then(|| self.frame_hud.view()).unwrap_or_else(|| div().into_any_element()));
            return login.into_any_element();
        }
        let pending = std::mem::take(&mut self.pending_ipc);
        for command in pending {
            self.execute_ipc_command(command, window, cx);
        }
        let progress = self.search_anim.target(
            if self.search_open { 1.0 } else { 0.0 },
            anim::STANDARD_MS,
            if self.search_open {
                anim::Curve::Decelerate
            } else {
                anim::Curve::Accelerate
            },
            &self.clock,
        );
        if self.search.is_some() && !self.search_open && progress <= 0.001 {
            self.search = None;
        }
        if self.search.is_some() && !self.search_anim.settled(&self.clock) {
            perf::request_frame(window, "search-anim");
        }
        if self.search.as_ref().is_some_and(|search| search.loading)
            || self.search_denied_opacity() > 0.0
        {
            perf::request_frame(window, "search-loading");
        }
        let line_progress = self.line_search_anim.target(
            if self.line_search.open { 1.0 } else { 0.0 },
            anim::STANDARD_MS,
            if self.line_search.open {
                anim::Curve::Decelerate
            } else {
                anim::Curve::Accelerate
            },
            &self.clock,
        );
        if !self.line_search_anim.settled(&self.clock) || self.line_search_denied_opacity() > 0.0 {
            perf::request_frame(window, "line-search");
        }
        let line_overlay = (self.line_search.open || line_progress > 0.001)
            .then(|| self.line_search_overlay(window, line_progress));
        let command_progress = self.command_line_anim.target(
            self.command_line.animation_target(),
            anim::STANDARD_MS,
            self.command_line.anim_curve(),
            &self.clock,
        );
        if !self.command_line_anim.settled(&self.clock)
            || self.command_line.denied_opacity(self.clock.now) > 0.0
        {
            perf::request_frame(window, "command-line");
        }
        let command_overlay = (self.command_line.is_open() || command_progress > 0.001)
            .then(|| self.command_line_overlay(window, command_progress));
        self.animate_alt(if self.alt.armed { 1.0 } else { 0.0 });
        let alt_progress = self.alt_spawn.value(&self.clock);
        if !self.alt_spawn.settled(&self.clock) {
            perf::request_frame(window, "alt");
        }
        if !self.alt.armed && alt_progress <= 0.001 {
            self.alt_menu = None;
            self.alt_target = None;
        }
        let alt_overlay =
            (self.alt.armed || alt_progress > 0.001).then(|| self.alt_overlay(alt_progress));
        let queue_progress = self.queue_anim.target(
            if self.queue_open { 1.0 } else { 0.0 },
            anim::EMPHASIZED_MS,
            if self.queue_open {
                anim::Curve::Decelerate
            } else {
                anim::Curve::Accelerate
            },
            &self.clock,
        );
        if !self.queue_anim.settled(&self.clock) {
            perf::request_frame(window, "queue");
        }
        let fullscreen_progress = self.fullscreen_anim.target(
            if self.fullscreen { 1.0 } else { 0.0 },
            anim::EMPHASIZED_MS,
            if self.fullscreen {
                anim::Curve::Decelerate
            } else {
                anim::Curve::Accelerate
            },
            &self.clock,
        );
        if !self.fullscreen_anim.settled(&self.clock) {
            perf::request_frame(window, "fullscreen");
        }
        if self.fullscreen {
            self.request_current_lyrics();
            self.ensure_marks();
            self.refresh_fullscreen_album_state();
        }
        if let Some(page) = self.page.as_ref() {
            let cover_art = page.header.cover_art.clone();
            let bucket = self.page_hero_bucket();
            let changed = self
                .cover_trace
                .as_ref()
                .map(|trace| (trace.cover_art.as_str(), trace.bucket))
                != Some((cover_art.as_str(), bucket));
            if changed {
                let started = std::time::Instant::now();
                crate::debug_log!(
                    "album entry: init album cover load cover={} bucket={} at={}",
                    cover_art,
                    bucket,
                    unix_ms()
                );
                self.cover_trace = Some(CoverLoadTrace {
                    cover_art: cover_art.clone(),
                    bucket,
                    started,
                    thumb_logged: false,
                });
            }
            if self.display_cover(&cover_art, bucket).is_none()
                && let Some(trace) = self.cover_trace.as_mut()
                && !trace.thumb_logged
            {
                trace.thumb_logged = true;
                crate::debug_log!(
                    "album entry: album cover thumb shown cover={} bucket={} elapsed_ms={:.1}",
                    cover_art,
                    bucket,
                    trace.started.elapsed().as_secs_f64() * 1000.0
                );
            }
            self.ensure_display_cover(&cover_art, bucket);
        }
        if let Some(track) = self.player.current().cloned() {
            self.ensure_display_cover(&track.cover_art, self.fullscreen_hero_bucket());
        }
        let lyrics_overlay_progress = self.lyrics_overlay_anim.target(
            if self.lyrics_overlay { 1.0 } else { 0.0 },
            anim::QUICK_MS,
            anim::Curve::Standard,
            &self.clock,
        );
        if !self.lyrics_overlay_anim.settled(&self.clock) {
            perf::request_frame(window, "lyrics-overlay");
        }
        let fullscreen_overlay = (self.fullscreen || fullscreen_progress > 0.001)
            .then(|| self.fullscreen_overlay(fullscreen_progress, cx));
        let exit_overlay = self.exiting.map(|started| {
            let progress = (((self.clock.now - started) / 0.200) as f32).clamp(0.0, 1.0);
            self.exit_overlay(progress)
        });
        self.toasts
            .retain(|toast| self.clock.now - toast.created_at < 6.320);
        if self
            .toasts
            .iter()
            .any(|toast| self.clock.now - toast.created_at < 6.320)
        {
            perf::request_frame(window, "toast");
        }
        let toast_overlay = (!self.toasts.is_empty()).then(|| self.toast_overlay(&*cx));
        let queue_overlay = (self.queue_open || queue_progress > 0.001)
            .then(|| self.queue_overlay(&*cx, queue_progress));
        let overlay = self
            .search
            .as_ref()
            .map(|search| self.search_overlay(window, &*cx, search, progress));
        let description_overlay = self.description_overlay(&*cx);
        if let Some(started) = self.exiting {
            perf::request_frame(window, "exiting");
            if self.clock.now - started >= 0.200 {
                self.flush_status();
                window.remove_window();
            }
        }
        if self.page.is_some() {
            self.ensure_page_metrics();
            self.ensure_marks();
        }
        if self.queue_open {
            self.ensure_queue_metrics();
        }
        if self
            .search
            .as_ref()
            .is_some_and(|search| !search.editor.text.is_empty())
        {
            let height = f32::from(window.viewport_size().height);
            self.ensure_search_metrics(height);
        }
        self.ensure_library_window();
        if !self.preloaded && self.view.rows.count > 0 && self.view.scope != Scope::Playlists {
            self.preloaded = true;
            let start_scope = self.view.scope;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(350))
                    .await;
                let _ = this.update(cx, |this, _| {
                    for scope in Scope::ALL {
                        if scope != start_scope && scope != Scope::Playlists {
                            this.bridge.preload_library(scope, 0);
                        }
                    }
                });
            })
            .detach();
        }
        let following = self.library_scroll.apply(self.clock.now)
            | self.page_scroll_modal.apply(self.clock.now)
            | self.queue_scroll_modal.apply(self.clock.now)
            | self.search_scroll.apply(self.clock.now);
        if following {
            perf::request_frame(window, "follow");
        }
        let top = self.top_bar(&*cx);
        let body = if self.page.is_some() {
            self.page_view(&*cx).into_any_element()
        } else {
            self.list(&*cx, window).into_any_element()
        };
        if self.row_scales.borrow().values().any(|emphasis| {
            !emphasis.cover.settled(&self.clock) || !emphasis.text.settled(&self.clock)
        }) || !self.library_hover_fill.settled(&self.clock)
            || !self.page_hover_fill.settled(&self.clock)
        {
            perf::request_frame(window, "row-emphasis");
        }
        if !self.lyrics_shift.settled(&self.clock)
            || !self.lyrics_track_fade.settled(&self.clock)
            || self
                .lyrics_line_opacity
                .values()
                .any(|value| !value.settled(&self.clock))
            || self
                .lyrics_line_emphasis
                .values()
                .any(|value| !value.settled(&self.clock))
            || self
                .lyrics_line_color
                .values()
                .any(|color| !color.settled(&self.clock))
        {
            perf::request_frame(window, "lyrics-behaviors");
        }
        // Loading and sync spinners are driven by the repaint ticker at 30 fps
        // (perf::Demand); only the short settle/fill animations still need a
        // self-requested frame chain.
        if !self.sync_settle.settled(&self.clock) || !self.sync_fill.settled(&self.clock) {
            perf::request_frame(window, "sync-settle");
        }
        let status = self.player_bar();
        #[cfg(debug_assertions)]
        let frame_hud = debug::hud_enabled()
            .then(|| self.frame_hud.view())
            .unwrap_or_else(|| div().into_any_element());
        let root = div()
            .track_focus(&self.focus)
            .key_context(if self.command_line.is_open() {
                "command-line"
            } else if self.line_search.open {
                "line-search"
            } else if self.search.is_some() && self.search_open {
                "search"
            } else {
                "main"
            })
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_top))
            .on_action(cx.listener(Self::on_bottom))
            .on_action(cx.listener(Self::on_scope_next))
            .on_action(cx.listener(Self::on_scope_prev))
            .on_action(cx.listener(Self::on_order_next))
            .on_action(cx.listener(Self::on_order_prev))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(Self::on_visual))
            .on_action(cx.listener(Self::on_search))
            .on_action(cx.listener(Self::on_insert_mode))
            .on_action(cx.listener(Self::on_line_search))
            .on_action(cx.listener(Self::on_lyrics_overlay))
            .on_action(cx.listener(Self::on_home))
            .on_action(cx.listener(Self::on_search_scope_next))
            .on_action(cx.listener(Self::on_search_scope_prev))
            .on_action(cx.listener(Self::on_play_pause))
            .on_action(cx.listener(Self::on_queue_toggle))
            .on_action(cx.listener(Self::on_repeat_cycle))
            .on_action(cx.listener(Self::on_toggle_fullscreen))
            .on_action(cx.listener(Self::on_queue_remove))
            .on_action(cx.listener(Self::on_queue_up))
            .on_action(cx.listener(Self::on_queue_down))
            .on_action(cx.listener(Self::on_mute))
            .on_action(cx.listener(Self::on_volume_up))
            .on_action(cx.listener(Self::on_volume_down))
            .on_action(cx.listener(Self::on_seek_back))
            .on_action(cx.listener(Self::on_seek_forward))
            .on_action(cx.listener(Self::on_next_track))
            .on_action(cx.listener(Self::on_prev_track))
            .on_action(cx.listener(Self::on_search_next))
            .on_action(cx.listener(Self::on_search_prev))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_action(cx.listener(Self::on_visual_all))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_back))
            .on_action(cx.listener(Self::on_prev_place))
            .on_action(cx.listener(Self::on_next_place))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(palette::background()))
            .font_family("Overused Grotesk Roman")
            .text_color(rgb(palette::on_surface()))
            .child(top)
            .child(body)
            .child(status)
            .child(match fullscreen_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(self.lyrics_overlay_element(lyrics_overlay_progress, cx))
            .child(match queue_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match description_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match line_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match command_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match toast_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match exit_overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match overlay {
                Some(o) => o.into_any_element(),
                None => div().into_any_element(),
            })
            .child(match alt_overlay {
                Some(o) => o,
                None => div().into_any_element(),
            });
        perf::note_notify(perf::now());
        perf::record_frame(frame_started, &*self);
        #[cfg(debug_assertions)]
        let root = root.child(frame_hud);
        root.into_any_element()
    }
}

/// One-frame resize settlement: returns `true` on the first stable frame after
/// a width/height change, so the view can re-center with the new layout.
///
/// `pending` is set on a resize frame (a follow-up frame is requested by the
/// caller) and cleared when that stable frame is rendered. Bursts of resize
/// events coalesce onto the last one.
pub(in crate::ui) fn resize_recenter_now(resized: bool, pending: &mut bool) -> bool {
    if resized {
        *pending = true;
        false
    } else {
        std::mem::take(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::resize_recenter_now;

    #[test]
    fn resize_settles_on_the_next_stable_frame() {
        let mut pending = false;
        // Resize frame: arm, do not re-center yet.
        assert!(!resize_recenter_now(true, &mut pending));
        assert!(pending);
        // Next stable frame: re-center exactly once.
        assert!(resize_recenter_now(false, &mut pending));
        assert!(!pending);
        assert!(!resize_recenter_now(false, &mut pending));
    }

    #[test]
    fn resize_burst_coalesces() {
        let mut pending = false;
        assert!(!resize_recenter_now(true, &mut pending));
        assert!(!resize_recenter_now(true, &mut pending));
        assert!(!resize_recenter_now(true, &mut pending));
        assert!(resize_recenter_now(false, &mut pending));
        assert!(!resize_recenter_now(false, &mut pending));
    }
}
