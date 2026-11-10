//! Key action handlers: list/page/queue navigation and playback.

use super::super::*;

/// Description reader steps (legacy QML `DescriptionOverlay` grammar): `j`/`k`
/// move one text line, the page motions move 90% of the viewport.
const DESCRIPTION_LINE_STEP: f32 = 22.0;
const DESCRIPTION_PAGE_RATIO: f32 = 0.9;

/// The next description reader offset, clamped into `0..=max`.
pub(in crate::ui) fn description_offset(current: f32, delta: f32, max: f32) -> f32 {
    (current + delta).clamp(0.0, max.max(0.0))
}

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
        if self.description.is_some() {
            self.scroll_description(DESCRIPTION_LINE_STEP);
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
        if self.lyrics_scroll_focused() {
            self.scroll_lyrics_by(self.lyrics_line_step(), cx);
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
        if self.description.is_some() {
            self.scroll_description(-DESCRIPTION_LINE_STEP);
            cx.notify();
            return;
        }
        if self.queue_open {
            self.queue_cursor = self.queue_cursor.saturating_sub(1);
            self.follow_queue(true);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() {
            self.scroll_lyrics_by(-self.lyrics_line_step(), cx);
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
        if self.description.is_some() {
            self.scroll_description(-f32::MAX);
        } else if self.lyrics_scroll_focused() && !self.queue_open {
            self.lyrics_top(cx);
        } else if self.queue_open {
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
        if self.description.is_some() {
            self.scroll_description(f32::MAX);
        } else if self.lyrics_scroll_focused() && !self.queue_open {
            self.lyrics_bottom(cx);
        } else if self.queue_open {
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
        } else {
            self.release_lyrics_hold();
        }
        cx.notify();
    }

    /// Any navigation to another place retracts the fullscreen playback view
    /// (and releases its lyrics hold).
    pub(in crate::ui) fn retract_playback(&mut self) {
        if self.fullscreen {
            self.fullscreen = false;
            self.release_lyrics_hold();
        }
    }

    /// Optimistically apply the start of a download toggle so the row marks
    /// react at once: a start shows the loading glyph, a removal drops the
    /// downloaded glyph before the ledger read settles.
    pub(in crate::ui) fn apply_download_started(&mut self, kind: &str, id: &str, on: bool) {
        if id.is_empty() {
            return;
        }
        let row_kind = match kind {
            "song" => RowKind::Song,
            "album" => RowKind::Album,
            "artist" => RowKind::Artist,
            _ => return,
        };
        let mut songs = std::collections::HashSet::new();
        let mut albums = std::collections::HashSet::new();
        let mut row_ids = std::collections::HashSet::new();
        let mut patch = |row: &mut Row, container: bool| {
            let hit = container
                || row.id == id
                || row.song_id == id
                || (!row.album_id.is_empty() && row.album_id == id);
            if !hit {
                return;
            }
            if !on {
                row.downloaded = false;
            }
            if !row.id.is_empty() {
                row_ids.insert(row.id.clone());
            }
            if !row.song_id.is_empty() {
                songs.insert(row.song_id.clone());
            }
            if !row.album_id.is_empty() {
                albums.insert(row.album_id.clone());
            }
        };
        for row in &mut self.view.rows.live {
            patch(row, false);
        }
        if let Some(search) = self.search.as_mut() {
            for row in &mut search.rows {
                patch(row, false);
            }
        }
        if let Some(page) = self.page.as_mut() {
            let container = page.descriptor.id == id
                && matches!(
                    (page.descriptor.kind, row_kind),
                    (PageKind::Album, RowKind::Album) | (PageKind::Artist, RowKind::Artist)
                );
            if container {
                page.container_cached = on;
                page.container_downloading = true;
            }
            for row in &mut page.rows {
                patch(row, container);
            }
        }
        drop(patch);
        if on {
            self.marks.pending.insert(id.to_string());
            self.marks.pending.extend(songs);
            self.marks.pending.extend(albums);
            self.marks.pending.extend(row_ids);
            return;
        }
        self.marks.songs_done.remove(id);
        self.marks.albums_done.remove(id);
        self.marks.pending.remove(id);
        for song in &songs {
            self.marks.songs_done.remove(song);
            self.marks.pending.remove(song);
        }
        for album in &albums {
            self.marks.albums_done.remove(album);
            self.marks.pending.remove(album);
        }
        for row_id in &row_ids {
            self.marks.pending.remove(row_id);
        }
    }

    pub(in crate::ui) fn on_repeat_cycle(
        &mut self,
        _: &RepeatCycle,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The core owns the repeat mode; the next playback state updates the
        // status bar badge.
        self.player.cycle_repeat();
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
            self.delete_queue_range(cx);
        }
    }

    fn rows_word(count: usize) -> &'static str {
        if count == 1 { "row" } else { "rows" }
    }

    /// Replace the row clipboard from a row list, preserving order and
    /// duplicates. Returns the number of ids stored; an unsupported row kind
    /// (playlists) or an empty id skips that row.
    fn clipboard_from_rows(&mut self, rows: Vec<Row>) -> usize {
        let Some((kind, ids)) = clipboard_ids_from_rows(rows) else {
            return 0;
        };
        let count = ids.len();
        self.clipboard_kind = Some(kind);
        self.clipboard_ids = ids;
        count
    }

    fn clipboard_from_song_ids(&mut self, ids: Vec<String>) -> usize {
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|id| !id.is_empty())
            .collect();
        if ids.is_empty() {
            return 0;
        }
        let count = ids.len();
        self.clipboard_kind = Some(RowKind::Song);
        self.clipboard_ids = ids;
        count
    }

    fn announce_yank(&mut self, count: usize, missing: usize) {
        if count == 0 {
            self.push_toast(
                "Nothing to yank".to_string(),
                ToastKind::Notice,
                None,
            );
            return;
        }
        let suffix = if missing > 0 {
            format!(" ({missing} not loaded)")
        } else {
            String::new()
        };
        self.push_toast(
            format!("Yanked {count} {}{suffix}", Self::rows_word(count)),
            ToastKind::Notice,
            None,
        );
    }

    /// `y` (range or cursor) / `Y` (cursor only) against the active surface.
    fn yank_surface(&mut self, row_only: bool) -> usize {
        if self.fullscreen || self.login.is_some() || self.command_line.is_open() {
            return 0;
        }
        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let indices = self.queue_selection.indices(self.queue_cursor, row_only);
            let ids: Vec<String> = indices
                .iter()
                .filter_map(|index| queue.get(*index).map(|track| track.id.clone()))
                .collect();
            let count = self.clipboard_from_song_ids(ids);
            self.queue_selection.clear();
            self.visual_mode = false;
            self.announce_yank(count, 0);
            return count;
        }
        let page_rows = self.page.as_ref().map(|page| {
            let indices = page.selection.indices(page.cursor, row_only);
            indices
                .iter()
                .filter_map(|index| page.rows.get(*index).cloned())
                .collect::<Vec<Row>>()
        });
        if let Some(rows) = page_rows {
            let count = self.clipboard_from_rows(rows);
            if let Some(page) = self.page.as_mut() {
                page.selection.clear();
            }
            self.visual_mode = false;
            self.announce_yank(count, 0);
            return count;
        }
        let indices = self.view.selection.indices(self.view.cursor, row_only);
        let selected = indices.len();
        let rows: Vec<Row> = indices
            .iter()
            .filter_map(|index| self.view.rows.get(*index).cloned())
            .collect();
        let missing = selected.saturating_sub(rows.len());
        let count = self.clipboard_from_rows(rows);
        self.view.selection.clear();
        self.visual_mode = false;
        self.announce_yank(count, missing);
        count
    }

    pub(in crate::ui) fn on_yank(
        &mut self,
        _: &Yank,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.yank_surface(false);
        cx.notify();
    }

    pub(in crate::ui) fn on_yank_row(
        &mut self,
        _: &YankRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.yank_surface(true);
        cx.notify();
    }

    /// Yank from the floating search's current scope (normal mode only; the
    /// typing mode keeps `y` as text input).
    pub(in crate::ui) fn yank_search(&mut self, row_only: bool) -> usize {
        let Some(search) = self.search.as_ref() else {
            return 0;
        };
        let rows = search.scoped().into_iter().cloned().collect::<Vec<Row>>();
        let indices = search.selection.indices(search.cursor(), row_only);
        let selected = indices.len();
        let picked: Vec<Row> = indices
            .iter()
            .filter_map(|index| rows.get(*index).cloned())
            .collect();
        let missing = selected.saturating_sub(picked.len());
        let count = self.clipboard_from_rows(picked);
        if let Some(search) = self.search.as_mut() {
            search.selection.clear();
        }
        self.visual_mode = false;
        self.announce_yank(count, missing);
        count
    }

    fn paste_clipboard(&mut self, above: bool, cx: &mut Context<Self>) {
        if self.fullscreen || self.login.is_some() {
            return;
        }
        let Some(kind) = self.clipboard_kind else {
            return;
        };
        let Some(kind_str) = kind.queue_kind() else {
            return;
        };
        let ids = std::mem::take(&mut self.clipboard_ids);
        if ids.is_empty() {
            self.clipboard_kind = None;
            return;
        }
        let target = if self.queue_open {
            Some(PasteTarget::Queue(paste_at(self.queue_cursor, above)))
        } else {
            self.page
                .as_ref()
                .filter(|page| page.descriptor.kind == PageKind::Playlist)
                .map(|page| {
                    PasteTarget::Playlist(
                        page.descriptor.id.clone(),
                        paste_at(page.cursor, above),
                    )
                })
        };
        let Some(target) = target else {
            // No valid target: keep the clipboard for the next attempt.
            self.clipboard_ids = ids;
            self.push_toast(
                "Open the queue or a playlist page to paste".to_string(),
                ToastKind::Notice,
                None,
            );
            cx.notify();
            return;
        };
        // One-shot clipboard: the same yanked rows cannot be pasted twice.
        // Keep a backup so a failed paste can restore it (Event::Error).
        self.clipboard_kind = None;
        self.paste_backup = Some((kind, ids.clone()));
        match target {
            PasteTarget::Queue(at) => {
                self.queue_selection.clear();
                self.visual_mode = false;
                self.bridge.paste_queue(kind_str.to_string(), ids, at);
            }
            PasteTarget::Playlist(playlist_id, position) => {
                self.bridge.paste_playlist(
                    playlist_id,
                    kind_str.to_string(),
                    ids,
                    position,
                );
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_paste_below(
        &mut self,
        _: &PasteBelow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_clipboard(false, cx);
    }

    pub(in crate::ui) fn on_paste_above(
        &mut self,
        _: &PasteAbove,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_clipboard(true, cx);
    }

    fn delete_queue_range(&mut self, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .queue_selection
            .indices(self.queue_cursor, false)
            .into_iter()
            .filter(|index| *index < self.player.len())
            .collect();
        if indices.is_empty() {
            return;
        }
        let first = indices[0];
        let count = self.player.remove_many(&indices);
        // The core is the authority: it shrinks `len()` when the delta lands.
        // Clamp against the expected remaining length so the cursor never
        // parks past the end in the meantime.
        let remaining = self.player.len().saturating_sub(count);
        self.queue_cursor = first.min(remaining.saturating_sub(1));
        self.queue_selection.clear();
        self.visual_mode = false;
        self.follow_queue(true);
        self.push_toast(
            format!("Deleted {count} {}", Self::rows_word(count)),
            ToastKind::Notice,
            None,
        );
        cx.notify();
    }

    fn delete_playlist_range(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let indices: Vec<usize> = page
            .selection
            .indices(page.cursor, false)
            .into_iter()
            .filter(|index| *index < page.rows.len())
            .collect();
        if indices.is_empty() {
            return;
        }
        let first = indices[0];
        page.cursor = first;
        page.selection.clear();
        let playlist_id = page.descriptor.id.clone();
        self.visual_mode = false;
        self.bridge
            .delete_playlist_positions(playlist_id, indices);
        cx.notify();
    }

    /// `d d`: the queue range wins when the drawer is open, otherwise the
    /// playlist page edits its entries; other surfaces are inert.
    pub(in crate::ui) fn on_delete_row(
        &mut self,
        _: &DeleteRow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fullscreen || self.login.is_some() {
            return;
        }
        if self.queue_open {
            self.delete_queue_range(cx);
            return;
        }
        if self
            .page
            .as_ref()
            .is_some_and(|page| page.descriptor.kind == PageKind::Playlist)
        {
            self.delete_playlist_range(cx);
        }
    }

    pub(in crate::ui) fn on_queue_up(
        &mut self,
        _: &QueueUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            if self.queue_selection.active() {
                self.move_queue_block(-1, cx);
            } else if self.queue_cursor > 0 {
                self.player
                    .move_item(self.queue_cursor, self.queue_cursor - 1);
                self.queue_cursor -= 1;
                self.follow_queue(true);
                cx.notify();
            }
            return;
        }
        self.move_playlist_rows(-1, cx);
    }

    /// Queue `Ctrl-Shift-K/J` with a visual selection: move the whole block.
    fn move_queue_block(&mut self, delta: i32, cx: &mut Context<Self>) {
        let anchor = self.queue_selection.anchor().unwrap_or(self.queue_cursor);
        let start = anchor.min(self.queue_cursor);
        let end = anchor.max(self.queue_cursor);
        let len = end - start + 1;
        if !self.player.move_block(start, len, delta) {
            return;
        }
        self.queue_cursor = (self.queue_cursor as i64 + i64::from(delta)) as usize;
        self.queue_selection.shift(delta);
        self.follow_queue(true);
        cx.notify();
    }

    /// Playlist-page `Ctrl-Shift-K/J`: move the cursor entry or the whole
    /// selected block one position.
    fn move_playlist_rows(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut() else {
            return;
        };
        if page.descriptor.kind != PageKind::Playlist {
            return;
        }
        let indices = page.selection.indices(page.cursor, false);
        let Some(&start) = indices.first() else {
            return;
        };
        let len = indices.len();
        let new_start = start as i64 + i64::from(delta);
        if new_start < 0 || new_start as usize + len > page.rows.len() || new_start as usize == start
        {
            return;
        }
        let cursor = (page.cursor as i64 + i64::from(delta)) as usize;
        if page.selection.active() {
            page.selection.shift(delta);
        }
        page.cursor = cursor;
        let playlist_id = page.descriptor.id.clone();
        self.visual_mode = page.selection.active();
        self.bridge
            .move_playlist_block(playlist_id, start, len, delta, cursor);
        cx.notify();
    }

    pub(in crate::ui) fn on_queue_down(
        &mut self,
        _: &QueueDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.queue_open {
            if self.queue_selection.active() {
                self.move_queue_block(1, cx);
            } else if self.queue_cursor + 1 < self.player.len() {
                self.player
                    .move_item(self.queue_cursor, self.queue_cursor + 1);
                self.queue_cursor += 1;
                self.follow_queue(true);
                cx.notify();
            }
            return;
        }
        self.move_playlist_rows(1, cx);
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
        // Show the sticky bubble on the expected state immediately; the next
        // state frame re-applies `sync_mute_toast` with the core's truth.
        let muted = !self.player.muted();
        self.player.toggle_mute();
        self.sync_mute_toast(muted);
        cx.notify();
    }

    pub(in crate::ui) fn on_volume_up(
        &mut self,
        _: &VolumeUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.volume_up();
        cx.notify();
    }

    pub(in crate::ui) fn on_volume_down(
        &mut self,
        _: &VolumeDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.player.volume_down();
        cx.notify();
    }

    pub(in crate::ui) fn on_seek_back(
        &mut self,
        _: &SeekBack,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.player.seek_relative(-5.0);
    }

    pub(in crate::ui) fn on_seek_forward(
        &mut self,
        _: &SeekForward,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.player.seek_relative(5.0);
    }

    pub(in crate::ui) fn on_next_track(
        &mut self,
        _: &NextTrack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_command("track next", cx);
    }

    pub(in crate::ui) fn execute_track_next(&mut self, cx: &mut Context<Self>) {
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
        self.handle_command("track previous", cx);
    }

    pub(in crate::ui) fn execute_track_previous(&mut self, cx: &mut Context<Self>) {
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
        if self.description.is_some() {
            self.page_description(true);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() && !self.queue_open {
            self.scroll_lyrics_by(self.lyrics_page_step(), cx);
            cx.notify();
            return;
        }
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
        if self.description.is_some() {
            self.page_description(false);
            cx.notify();
            return;
        }
        if self.lyrics_scroll_focused() && !self.queue_open {
            self.scroll_lyrics_by(-self.lyrics_page_step(), cx);
            cx.notify();
            return;
        }
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
            let from_search = self.search_open
                && self.search.is_some()
                && self.alt_target.is_some()
                && self.status_focus.is_none();
            let menu = self.alt_menu.clone();
            let mut acted = false;
            if let Some(menu) = menu
                && let Some(verb) = self.alt.confirm_plan(&menu)
            {
                self.dispatch_quick(verb);
                acted = true;
            }
            if acted && from_search {
                self.dismiss_search_after_quick();
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
            self.release_lyrics_hold();
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
        self.release_lyrics_hold();
        self.description = None;
        self.page = None;
        self.travel = Travel::default();
        self.view.selection.clear();
        cx.notify();
    }

    /// The painted height of the description reader's scroll body.
    fn description_viewport(&self) -> f32 {
        let height = f32::from(self.description_scroll.bounds().size.height);
        if height > 1.0 {
            height
        } else {
            (self.win_h * 0.6).max(1.0)
        }
    }

    /// Scroll the description reader by `delta` pixels (positive = down),
    /// clamped to the real content range tracked by the scroll handle.
    fn scroll_description(&mut self, delta: f32) {
        // The handle stores the offset negatively (content moved up); the
        // tracked `max_offset` is the positive scroll range.
        let current = -f32::from(self.description_scroll.offset().y);
        let max = f32::from(self.description_scroll.max_offset().height);
        let next = description_offset(current, delta, max);
        self.description_scroll
            .set_offset(gpui::point(px(0.0), px(-next)));
    }

    /// Page the description reader: 90% of the visible height per motion.
    fn page_description(&mut self, down: bool) {
        let step = self.description_viewport() * DESCRIPTION_PAGE_RATIO;
        self.scroll_description(if down { step } else { -step });
    }
}

enum PasteTarget {
    Queue(usize),
    Playlist(String, usize),
}

/// Collect clipboard ids from already-materialized rows. The first supported
/// row fixes the kind; ids keep row order and duplicates. Playlist rows and
/// empty ids are skipped.
fn clipboard_ids_from_rows(rows: Vec<Row>) -> Option<(RowKind, Vec<String>)> {
    let mut kind = None;
    let mut ids = Vec::new();
    for row in rows {
        if row.kind.queue_kind().is_none() {
            continue;
        }
        let id = if row.kind == RowKind::Song {
            if row.song_id.is_empty() {
                row.id
            } else {
                row.song_id
            }
        } else {
            row.id
        };
        if id.is_empty() {
            continue;
        }
        match kind {
            None => {
                kind = Some(row.kind);
                ids.push(id);
            }
            Some(seen) if seen == row.kind => ids.push(id),
            _ => {}
        }
    }
    kind.map(|kind| (kind, ids))
}

/// `p` below / `P` above the queue cursor.
fn paste_at(cursor: usize, above: bool) -> usize {
    cursor.saturating_add(usize::from(!above))
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    fn row(kind: RowKind, id: &str, song_id: &str) -> Row {
        Row {
            id: id.to_string(),
            kind,
            title: String::new(),
            subtitle: String::new(),
            meta: String::new(),
            track: String::new(),
            cover_url: String::new(),
            cover_remote: String::new(),
            song_id: song_id.to_string(),
            album_id: String::new(),
            starred: false,
            rating: 0,
            downloaded: false,
        }
    }

    #[test]
    fn clipboard_keeps_order_duplicates_and_song_ids() {
        let rows = vec![
            row(RowKind::Song, "a", "s1"),
            row(RowKind::Song, "b", "s1"),
            row(RowKind::Song, "c", ""),
        ];
        assert_eq!(
            clipboard_ids_from_rows(rows),
            Some((RowKind::Song, vec!["s1".to_string(), "s1".to_string(), "c".to_string()]))
        );
    }

    #[test]
    fn clipboard_uses_album_ids_and_rejects_playlists() {
        let rows = vec![
            row(RowKind::Album, "al1", ""),
            row(RowKind::Album, "al2", ""),
        ];
        assert_eq!(
            clipboard_ids_from_rows(rows),
            Some((RowKind::Album, vec!["al1".to_string(), "al2".to_string()]))
        );
        assert_eq!(
            clipboard_ids_from_rows(vec![row(RowKind::Playlist, "p1", "")]),
            None
        );
        assert_eq!(clipboard_ids_from_rows(Vec::new()), None);
    }

    #[test]
    fn paste_at_is_below_or_above_the_cursor() {
        assert_eq!(paste_at(0, false), 1);
        assert_eq!(paste_at(0, true), 0);
        assert_eq!(paste_at(3, false), 4);
        assert_eq!(paste_at(3, true), 3);
        assert_eq!(paste_at(usize::MAX, false), usize::MAX);
    }
}
