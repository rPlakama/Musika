//! Key action handlers: list/page/queue navigation and playback.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn on_quit(
        &mut self,
        _: &Quit,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.remove_window();
    }

    pub(in crate::ui) fn on_move_down(
        &mut self,
        _: &MoveDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_focus = None;
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(1);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_cursor = self
                .queue_cursor
                .saturating_add(1)
                .min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            if !self.visual_mode {
                page.selection.clear();
            }
            page.select(page.cursor.saturating_add(1));
            self.hovered_page = None;
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_add(1));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_move_up(
        &mut self,
        _: &MoveUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_focus = None;
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(-1);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_cursor = self.queue_cursor.saturating_sub(1);
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            if !self.visual_mode {
                page.selection.clear();
            }
            page.select(page.cursor.saturating_sub(1));
            self.hovered_page = None;
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_sub(1));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_top(&mut self, _: &Top, _window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_open {
            self.land_queue(0);
        } else if self.page.is_some() {
            self.land_page(0);
        } else {
            self.land_library(0);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_bottom(
        &mut self,
        _: &Bottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            let last = self.player.len().saturating_sub(1);
            self.land_queue(last);
        } else if let Some(page) = self.page.as_ref() {
            let last = page.rows.len().saturating_sub(1);
            self.land_page(last);
        } else {
            let last = self.view.rows.len().saturating_sub(1);
            self.land_library(last);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_scope_next(
        &mut self,
        _: &ScopeNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(self.view.scope.next());
        cx.notify();
    }

    pub(in crate::ui) fn on_scope_prev(
        &mut self,
        _: &ScopePrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_scope(self.view.scope.prev());
        cx.notify();
    }

    pub(in crate::ui) fn on_order_next(
        &mut self,
        _: &OrderNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search.is_some() {
            self.move_search_scope(1);
            cx.notify();
            return;
        }
        self.cycle_order(true);
        cx.notify();
    }

    pub(in crate::ui) fn on_order_prev(
        &mut self,
        _: &OrderPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search.is_some() {
            self.move_search_scope(-1);
            cx.notify();
            return;
        }
        self.cycle_order(false);
        cx.notify();
    }

    pub(in crate::ui) fn on_refresh(
        &mut self,
        _: &Refresh,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_sync(SyncMode::Incremental);
        cx.notify();
    }

    pub(in crate::ui) fn cycle_fullscreen_focus(&mut self, backward: bool) {
        let mut order = vec![FullscreenFocus::Cover, FullscreenFocus::Title];
        if self.lyrics_inline() && !self.lyrics_overlay {
            order.push(FullscreenFocus::Lyrics);
        }
        let position = order
            .iter()
            .position(|focus| *focus == self.fullscreen_focus)
            .unwrap_or(0);
        let next = if backward {
            (position + order.len() - 1) % order.len()
        } else {
            (position + 1) % order.len()
        };
        self.fullscreen_focus = order[next];
    }

    pub(in crate::ui) fn on_toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            self.request_current_lyrics();
            self.ensure_marks();
            self.refresh_fullscreen_album_state();
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_repeat_cycle(
        &mut self,
        _: &RepeatCycle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.repeat = self.player.cycle_repeat();
        cx.notify();
    }

    pub(in crate::ui) fn on_queue_toggle(
        &mut self,
        _: &QueueToggle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_focus = None;
        self.queue_open = !self.queue_open;
        if self.queue_open {
            self.land_queue(self.player.index());
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_queue_remove(
        &mut self,
        _: &QueueRemove,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            self.player.remove(self.queue_cursor);
            self.queue_cursor = self.queue_cursor.min(self.player.len().saturating_sub(1));
            self.follow_queue(true);
            cx.notify();
        }
    }

    pub(in crate::ui) fn on_queue_up(
        &mut self,
        _: &QueueUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open && self.queue_cursor > 0 {
            self.player
                .move_item(self.queue_cursor, self.queue_cursor - 1);
            self.queue_cursor -= 1;
            self.follow_queue(true);
            cx.notify();
        }
    }

    pub(in crate::ui) fn on_queue_down(
        &mut self,
        _: &QueueDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open && self.queue_cursor + 1 < self.player.len() {
            self.player
                .move_item(self.queue_cursor, self.queue_cursor + 1);
            self.queue_cursor += 1;
            self.follow_queue(true);
            cx.notify();
        }
    }

    pub(in crate::ui) fn on_play_pause(
        &mut self,
        _: &PlayPause,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.press_transport(1);
        self.player.toggle();
        cx.notify();
    }

    pub(in crate::ui) fn on_mute(
        &mut self,
        _: &Mute,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.toggle_mute();
        cx.notify();
    }

    pub(in crate::ui) fn on_volume_up(
        &mut self,
        _: &VolumeUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player
            .set_volume((self.player.volume() + 5.0).min(100.0));
        cx.notify();
    }

    pub(in crate::ui) fn on_volume_down(
        &mut self,
        _: &VolumeDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player
            .set_volume((self.player.volume() - 5.0).max(0.0));
        cx.notify();
    }

    pub(in crate::ui) fn on_seek_back(
        &mut self,
        _: &SeekBack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.seek((self.player.position() - 5.0).max(0.0));
        cx.notify();
    }

    pub(in crate::ui) fn on_seek_forward(
        &mut self,
        _: &SeekForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.seek(self.player.position() + 5.0);
        cx.notify();
    }

    pub(in crate::ui) fn on_next_track(
        &mut self,
        _: &NextTrack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.press_transport(2);
        self.player.next();
        if self.queue_open {
            self.land_queue(self.player.index());
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_prev_track(
        &mut self,
        _: &PrevTrack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.press_transport(0);
        self.player.previous();
        if self.queue_open {
            self.queue_cursor = self.player.index();
            self.follow_queue(true);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_search_scope_next(
        &mut self,
        _: &SearchScopeNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_search.open {
            self.line_search_step(1);
            cx.notify();
            return;
        }
        if self.search.is_some() && self.search_open {
            self.move_search_scope(1);
            cx.notify();
            return;
        }
        self.on_tab(false, cx);
    }

    pub(in crate::ui) fn on_search_scope_prev(
        &mut self,
        _: &SearchScopePrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_search.open {
            self.line_search_step(-1);
            cx.notify();
            return;
        }
        if self.search.is_some() && self.search_open {
            self.move_search_scope(-1);
            cx.notify();
            return;
        }
        self.on_tab(true, cx);
    }

    /// Tab's contextual walk. The superior bar exposes the current page's
    /// chips (the artist entry adds Download/Shuffle/Favorite) followed by
    /// the status actions; leaving the end drops back to the content.
    pub(in crate::ui) fn on_tab(&mut self, backward: bool, cx: &mut Context<Self>) {
        if self.fullscreen {
            if !self.lyrics_overlay {
                self.cycle_fullscreen_focus(backward);
            }
            cx.notify();
            return;
        }
        let order = self.topbar_tab_order();
        self.status_focus = next_tab_slot(&order, self.status_focus, backward);
        cx.notify();
    }

    pub(in crate::ui) fn on_page_down(
        &mut self,
        _: &PageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_add(10));
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_add(10));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_page_up(
        &mut self,
        _: &PageUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = self.page.as_mut() {
            page.select(page.cursor.saturating_sub(10));
            self.follow_page(true);
        } else {
            self.select(self.view.cursor.saturating_sub(10));
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_visual(
        &mut self,
        _: &Visual,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            let active = self.queue_selection.active();
            self.visual_mode = !active;
            if active {
                self.queue_selection.clear();
            } else {
                self.queue_selection.start(self.queue_cursor);
            }
            self.hovered_row = None;
            cx.notify();
            return;
        }
        let (active, cursor) = if let Some(page) = self.page.as_ref() {
            (page.selection.active(), page.cursor)
        } else {
            (self.view.selection.active(), self.view.cursor)
        };
        self.visual_mode = !active;
        if active {
            if let Some(page) = self.page.as_mut() {
                page.selection.clear();
            } else {
                self.view.selection.clear();
            }
        } else if let Some(page) = self.page.as_mut() {
            page.selection.start(cursor);
        } else {
            self.view.selection.start(cursor);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_visual_all(
        &mut self,
        _: &VisualAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.visual_mode = true;
        if self.queue_open {
            self.queue_selection.select_all();
            self.queue_cursor = self.player.len().saturating_sub(1);
            self.follow_queue(false);
            cx.notify();
            return;
        }
        if let Some(page) = self.page.as_mut() {
            page.selection.select_all();
            let last = page.rows.len().saturating_sub(1);
            page.select(last);
        } else {
            self.view.selection.select_all();
            let last = self.view.rows.len().saturating_sub(1);
            self.select(last);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_enter(
        &mut self,
        _: &Enter,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_search.open {
            self.line_search_commit(_window, cx);
            return;
        }
        if self.alt.armed && self.alt.sticky {
            let menu = self.alt_menu.clone();
            if let Some(menu) = menu
                && let Some(verb) = self.alt.confirm_plan(&menu)
            {
                self.dispatch_quick(verb);
            }
            self.alt.cancel();
            self.alt_menu = None;
            self.alt_target = None;
            self.animate_alt(0.0);
            cx.notify();
            return;
        }
        if let Some(slot) = self.status_focus {
            self.activate_status(slot);
            cx.notify();
            return;
        }
        if self.search.is_some() {
            self.activate_search_result(cx);
            return;
        }
        if self.queue_open {
            let index = self.queue_cursor;
            self.player.jump(index);
            self.queue_open = false;
            cx.notify();
            return;
        }
        if self.page.is_some() {
            self.activate_page_cursor();
        } else {
            let row = self.view.rows.get(self.view.cursor).cloned();
            if let Some(row) = row {
                self.activate_row(&row);
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_back(
        &mut self,
        _: &Back,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.line_search.open {
            self.close_line_search();
            cx.notify();
            return;
        }
        if self.alt.armed || self.alt_menu.is_some() {
            self.alt.cancel();
            self.alt_menu = None;
            self.alt_target = None;
            self.animate_alt(0.0);
            cx.notify();
            return;
        }
        if self.status_focus.is_some() {
            self.status_focus = None;
            cx.notify();
            return;
        }
        if self.description.is_some() {
            self.description = None;
            cx.notify();
            return;
        }
        if self.lyrics_overlay {
            self.lyrics_overlay = false;
            self.lyrics_overlay_wanted = false;
            cx.notify();
            return;
        }
        if self.fullscreen || self.fullscreen_anim.value(&self.clock) > 0.01 {
            self.fullscreen = false;
            cx.notify();
            return;
        }
        if self.queue_open && self.queue_selection.active() {
            self.queue_selection.clear();
            self.visual_mode = false;
            cx.notify();
            return;
        }
        if self.queue_open || self.queue_anim.value(&self.clock) > 0.01 {
            self.queue_open = false;
            cx.notify();
            return;
        }
        if self.search.is_some() {
            self.search_open = false;
            cx.notify();
            return;
        }
        if self
            .page
            .as_ref()
            .is_some_and(|page| page.selection.active())
        {
            if let Some(page) = self.page.as_mut() {
                page.selection.clear();
            }
            cx.notify();
            return;
        }
        if self.view.selection.active() && self.page.is_none() {
            self.view.selection.clear();
            cx.notify();
            return;
        }
        if self.page.is_some() {
            self.back_page();
            cx.notify();
            return;
        }
        let now = self.clock.now;
        if self.quit_armed_at.is_some_and(|at| now - at <= 0.25) {
            self.quit_armed_at = None;
            self.exiting = Some(now);
            cx.notify();
            return;
        }
        self.quit_armed_at = Some(now);
        cx.notify();
    }

    pub(in crate::ui) fn on_prev_place(
        &mut self,
        _: &PrevPlace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_travel(-1);
        cx.notify();
    }

    pub(in crate::ui) fn on_next_place(
        &mut self,
        _: &NextPlace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_travel(1);
        cx.notify();
    }

    pub(in crate::ui) fn on_home(
        &mut self,
        _: &Home,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_line_search();
        self.alt.cancel();
        self.alt_menu = None;
        self.alt_target = None;
        self.animate_alt(0.0);
        self.search_open = false;
        self.search = None;
        self.queue_open = false;
        self.fullscreen = false;
        self.lyrics_overlay = false;
        self.lyrics_overlay_wanted = false;
        self.description = None;
        self.page = None;
        self.travel = Travel::default();
        self.view.selection.clear();
        cx.notify();
    }
}
