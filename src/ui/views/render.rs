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
        if self.awaiting_core {
            // Startup could not reach the core: keep a minimal retry surface
            // up until the supervisor reports `Connected` (it retries in the
            // background, so the window appears immediately either way).
            self.focus.focus(window);
            return self
                .core_waiting_view(self.image_cache.clone())
                .into_any_element();
        }
        if !self.activation_observed {
            self.activation_observed = true;
            cx.observe_window_activation(window, |this, window, cx| {
                perf::set_window_active(window.is_window_active());
                if !window.is_window_active() {
                    // A keyboard grab can swallow the key-up; drop the pending
                    // hold so it cannot jump on the next tap.
                    this.cancel_playback_hold();
                }
                this.refresh_lyrics_overlay();
                cx.notify();
            })
            .detach();
        }
        self.refresh_lyrics_overlay();
        if self.trace && self.frames.is_multiple_of(120) {
            crate::debug_log!("GPUI trace: frame {}", self.frames);
        }
        self.clock.tick();
        // Cover images are finite: anything nothing asked for in the last two
        // frames (a page we just left, a closed overlay) is unloaded here, so a
        // travel session holds one screen's covers instead of every cover it
        // ever showed.
        self.image_cache
            .update(cx, |cache, cx| cache.begin_frame(window, cx));
        // Page entries are virtual: only the open page exists. Drop the render
        // state keyed by the page just left as soon as the page identity moves,
        // so a travel session keeps one page's worth of row/cover bookkeeping.
        if crate::ui::app::navigation::needs_page_prune(
            self.page_state_marker,
            self.page_generation,
            self.page.is_some(),
        ) {
            self.page_state_marker = Some((self.page_generation, self.page.is_some()));
            self.prune_page_state();
            // The released page's covers are unloaded on the next frame. Ask
            // for that frame explicitly: otherwise the window can go idle right
            // after the navigation and hold the retired page's covers until the
            // user happens to touch the UI again.
            perf::request_frame(window, "cover-release");
        }
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
        // Cover identity first: the palette follows the identity's stable
        // cover-art id, so same-album track changes do not reset it.
        self.refresh_current_cover();
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
                .image_cache(self.image_cache.clone())
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_key_down))
                .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
                .size_full()
                .child(self.login_view(window));
            #[cfg(debug_assertions)]
            let login = if debug::hud_enabled() {
                login.child(self.frame_hud.view())
            } else {
                login
            };
            return login.into_any_element();
        }
        let pending = std::mem::take(&mut self.pending_ipc);
        for pending in pending {
            self.execute_ipc_command(pending.command, pending.announce, window, cx);
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
            .then(|| self.fullscreen_overlay(fullscreen_progress, window, cx));
        let exit_overlay = self.exiting.map(|started| {
            let progress = (((self.clock.now - started) / 0.200) as f32).clamp(0.0, 1.0);
            self.exit_overlay(progress)
        });
        self.toasts
            .retain(|toast| self.clock.now - toast.created_at < anim::TOAST_TTL_S);
        if self
            .toasts
            .iter()
            .any(|toast| self.clock.now - toast.created_at < anim::TOAST_TTL_S)
        {
            perf::request_frame(window, "toast");
        }
        // The held-key counter refreshes itself on every repeat; its TTL is the
        // only authority on when it goes away, so a lost key-up cannot strand it.
        // It runs on `CLOCK_MONOTONIC` like the hold, because `clock.now` only
        // advances while frames render and an idle window would expire it at once.
        let now_ns = crate::linux::monotonic_ns();
        if self
            .hold_toast
            .as_ref()
            .is_some_and(|hold| hold.expired(now_ns))
        {
            self.hold_toast = None;
        }
        if self.hold_toast.is_some() {
            perf::request_frame(window, "hold-toast");
        }
        // Sticky mute bubble: no TTL, so it repaints only while popping in.
        if self.mute_toast.as_ref().is_some_and(|mute| {
            self.clock.now - mute.created_at < f64::from(anim::TOAST_IN_MS / 1000.0)
        }) {
            perf::request_frame(window, "mute-toast");
        }
        let reconnect_banner = self.reconnecting.clone().map(|notice| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .px_3()
                .py_1()
                .bg(rgb(palette::surface_container_high()))
                .text_size(px(11.0))
                .text_color(rgb(palette::on_surface_variant()))
                .child(notice)
                .into_any_element()
        });
        let toast_overlay = (!self.toasts.is_empty()
            || self.hold_toast.is_some()
            || self.mute_toast.is_some())
        .then(|| self.toast_overlay(&*cx));
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
                self.bridge.shutdown_core();
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
                _ = this.update(cx, |this, _| {
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
        self.refresh_topbar_marquee(window);
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
        if self.player.state() == crate::player::State::Playing {
            perf::request_frame(window, "playing");
        }
        if self.player.is_loading() {
            perf::request_frame(window, "loading");
        }
        if self.syncing
            || !self.sync_settle.settled(&self.clock)
            || !self.sync_fill.settled(&self.clock)
        {
            perf::request_frame(window, "sync");
        }
        let status = self.player_bar();
        #[cfg(debug_assertions)]
        let frame_hud = if debug::hud_enabled() {
            self.frame_hud.view()
        } else {
            div().into_any_element()
        };
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
            .on_action(cx.listener(Self::on_delete_row))
            .on_action(cx.listener(Self::on_yank))
            .on_action(cx.listener(Self::on_yank_row))
            .on_action(cx.listener(Self::on_paste_below))
            .on_action(cx.listener(Self::on_paste_above))
            .on_action(cx.listener(Self::on_queue_up))
            .on_action(cx.listener(Self::on_queue_down))
            .on_action(cx.listener(Self::on_mute))
            .on_action(cx.listener(Self::on_next_track))
            .on_action(cx.listener(Self::on_prev_track))
            .on_action(cx.listener(Self::on_search_next))
            .on_action(cx.listener(Self::on_search_prev))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_action(cx.listener(Self::on_visual_all))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_back))
            .on_action(cx.listener(Self::on_prev_place))
            .on_action(cx.listener(Self::on_next_place))
            // One bounded cover cache for the whole tree: every descendant
            // `img()` (page hero, fullscreen art, player-bar cover, collages)
            // resolves through it instead of gpui's retain-forever asset cache.
            .image_cache(self.image_cache.clone())
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
            .child(match reconnect_banner {
                Some(o) => o,
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
        {
            let cache = self.image_cache.read(cx);
            self.image_cache_entries = cache.len();
            self.image_cache_bytes = cache.used_bytes();
            let stats = cache.stats();
            self.image_cache_hits = stats.hits;
            self.image_cache_misses = stats.misses;
        }
        perf::record_frame(frame_started, &*self);
        #[cfg(debug_assertions)]
        let root = root.child(frame_hud);
        root.into_any_element()
    }
}

impl AppRoot {
    /// Minimal full-window surface shown when the core was not reachable at
    /// startup; `UiUpdate::Connected` clears `awaiting_core` and the real UI
    /// takes over.
    fn core_waiting_view(
        &self,
        image_cache: gpui::Entity<image_cache::BoundedImageCache>,
    ) -> gpui::AnyElement {
        let detail = self
            .reconnecting
            .clone()
            .unwrap_or_else(|| "Waiting for the core daemon…".to_string());
        div()
            .image_cache(image_cache)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .bg(rgb(palette::background()))
            .text_color(rgb(palette::on_surface()))
            .child(div().text_size(px(18.0)).child("Waiting for the core daemon"))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(palette::on_surface_variant()))
                    .child(detail),
            )
            .into_any_element()
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
