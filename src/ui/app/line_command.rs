//! The `/` line-search and `:` command-line sessions.

use super::super::*;

use super::super::text_modal::{apply as apply_text, command_for_key};

impl AppRoot {
    pub(in crate::ui) fn apply_line_hit(&mut self, hit: line_search::LineHit) {
        let Some(surface) = self.line_search_surfaces.get(hit.target).copied() else {
            return;
        };
        match surface {
            LineSurface::Search => {
                if let Some(search) = self.search.as_mut() {
                    search.scope_cursors[search.scope] = hit.row;
                }
            }
            LineSurface::Queue => {
                self.queue_cursor = hit.row.min(self.player.len().saturating_sub(1));
                self.follow_queue(true);
            }
            LineSurface::Page => {
                if let Some(page) = self.page.as_mut() {
                    page.select(hit.row);
                }
                self.follow_page(true);
            }
            LineSurface::Library => {
                self.select(hit.row);
            }
            LineSurface::Lyrics => {
                self.lyrics_window_start = hit.row.saturating_sub(2);
            }
            LineSurface::Status => {
                if let Some(slot) = self.topbar_tab_order().get(hit.row) {
                    self.status_focus = Some(*slot);
                }
            }
        }
    }

    pub(in crate::ui) fn line_search_step(&mut self, delta: isize) {
        self.line_search.step(delta);
        if let Some(hit) = self.line_search.current_hit() {
            self.apply_line_hit(hit);
        }
    }

    pub(in crate::ui) fn line_search_commit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A typo after a valid prefix must not make Enter a no-op: fall back
        // to the last candidate that actually matched in this session.
        if let Some(hit) = self.line_search.latest_hit() {
            let status = self.line_search_surfaces.get(hit.target) == Some(&LineSurface::Status);
            let status_slot = self.topbar_tab_order().get(hit.row).copied();
            self.apply_line_hit(hit);
            self.close_line_search();
            if status {
                if let Some(slot) = status_slot {
                    self.activate_status(slot);
                }
            } else {
                self.on_enter(&Enter, window, cx);
            }
        } else if self.line_search.query().is_empty() {
            self.close_line_search();
        } else {
            self.flash_line_search_denied();
        }
        cx.notify();
    }

    pub(in crate::ui) fn line_search_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        match key {
            "escape" => {
                self.close_line_search();
                cx.notify();
                return;
            }
            "enter" => {
                self.line_search_commit(window, cx);
                return;
            }
            "tab" => {
                self.line_search_step(1);
                cx.notify();
                return;
            }
            "backtab" => {
                self.line_search_step(-1);
                cx.notify();
                return;
            }
            "c" if control => {
                if let Some(text) = self.line_search.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                if let Some(text) = self.line_search.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.line_search.editor_mut().replace_selection("");
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.line_search.editor_mut().insert(&text);
                }
            }
            _ => {
                let command =
                    command_for_key(key, shift, control, event.keystroke.key_char.as_deref());
                let Some(command) = command else { return };
                apply_text(self.line_search.editor_mut(), &command);
            }
        }
        self.refresh_line_search();
        cx.notify();
    }

    pub(in crate::ui) fn clear_line_search_selection(&mut self) {
        self.visual_mode = false;
        self.queue_selection.clear();
        self.view.selection.clear();
        if let Some(page) = self.page.as_mut() {
            page.selection.clear();
        }
    }

    pub(in crate::ui) fn line_search_targets(
        &self,
    ) -> (Vec<line_search::LineTarget>, Vec<LineSurface>) {
        use line_search::{LineRow, LineTarget};
        let mut targets = Vec::new();
        let mut surfaces = Vec::new();

        if self.search_open
            && let Some(search) = self.search.as_ref()
        {
            let rows = search.scoped();
            let visible = 12usize;
            let mut lines = vec![None; rows.len()];
            for (index, row) in rows.iter().take(visible).enumerate() {
                lines[index] = Some(LineRow::new(
                    row.title.clone(),
                    row.subtitle.clone(),
                    row.meta.clone(),
                ));
            }
            targets.push(LineTarget::new(lines));
            surfaces.push(LineSurface::Search);
            return (targets, surfaces);
        }

        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let range = self.queue_scroll_modal.visible_range(queue.len());
            let mut lines = vec![None; queue.len()];
            for (index, track) in queue.iter().enumerate() {
                if range.contains(&index) {
                    let duration =
                        crate::browse::rows::format_duration(Some(track.duration as i32));
                    lines[index] = Some(
                        LineRow::with_extra(
                            track.title.clone(),
                            track.artist.clone(),
                            "",
                            vec![duration],
                        )
                        .with_search_fields([0, 1, 3]),
                    );
                }
            }
            targets.push(LineTarget::new(lines));
            surfaces.push(LineSurface::Queue);
            return (targets, surfaces);
        }

        if self.fullscreen {
            if let Some(lyrics) = self.lyrics.as_ref() {
                let count = lyrics.lines.len();
                let visible = 15usize;
                let start = self.lyrics_window_start.min(count);
                let end = (start + visible).min(count);
                let mut lines = vec![None; count];
                for (index, (_, text)) in lyrics.lines.iter().enumerate() {
                    if (start..end).contains(&index) {
                        lines[index] =
                            Some(LineRow::new(text.clone(), "", "").with_search_fields([0]));
                    }
                }
                targets.push(LineTarget::new(lines));
                surfaces.push(LineSurface::Lyrics);
            }
            return (targets, surfaces);
        }

        // The status bar's two objects are part of the plain-window buffer,
        // before the active list, exactly like the legacy `/` contract.
        let mode = self.superior_mode();
        let status_rows = self
            .topbar_actions()
            .into_iter()
            .map(|action| Some(LineRow::new(action.label(mode), "Superior bar", "")))
            .collect();
        targets.push(LineTarget::new(status_rows));
        surfaces.push(LineSurface::Status);

        if let Some(page) = self.page.as_ref() {
            let range = self.page_scroll_modal.visible_range(page.rows.len());
            let artist = matches!(page.descriptor.kind, state::PageKind::Artist);
            let mut lines = vec![None; page.rows.len()];
            for (index, row) in page.rows.iter().enumerate() {
                if !range.contains(&index) {
                    continue;
                }
                let line = if artist {
                    LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone())
                } else {
                    LineRow::with_extra(
                        row.title.clone(),
                        row.subtitle.clone(),
                        row.meta.clone(),
                        vec![row.track.clone()],
                    )
                    .with_search_fields([0, 1, 2, 3])
                };
                lines[index] = Some(line);
            }
            targets.push(LineTarget::new(lines));
            surfaces.push(LineSurface::Page);
            return (targets, surfaces);
        }

        let count = self.view.rows.count;
        let range = self.library_scroll.visible_range(count);
        let mut lines = vec![None; count];
        for (offset, row) in self.view.rows.live.iter().enumerate() {
            let index = self.view.rows.live_start + offset;
            if index < count && range.contains(&index) {
                lines[index] = Some(
                    LineRow::new(row.title.clone(), row.subtitle.clone(), row.meta.clone())
                        .with_search_fields([0]),
                );
            }
        }
        targets.push(LineTarget::new(lines));
        surfaces.push(LineSurface::Library);
        (targets, surfaces)
    }

    pub(in crate::ui) fn line_search_current(&self, surface: LineSurface, row: usize) -> bool {
        self.line_search.current_hit().is_some_and(|hit| {
            self.line_search_surfaces.get(hit.target) == Some(&surface) && hit.row == row
        })
    }

    pub(in crate::ui) fn flash_line_search_denied(&mut self) {
        if self.line_search.query().is_empty() {
            return;
        }
        self.line_search_denied_at = Some(self.clock.now);
        self.line_search_denied_query = self.line_search.query().to_string();
    }

    pub(in crate::ui) fn refresh_line_search(&mut self) {
        let query = self.line_search.query().to_string();
        self.line_search.refresh(&self.line_search_snapshot);
        if let Some(hit) = self.line_search.current_hit() {
            self.apply_line_hit(hit);
        }
        if query.is_empty() {
            self.line_search_denied_query.clear();
        } else if self.line_search.candidates.is_empty()
            && self.line_search.last_hit.is_none()
            && query != self.line_search_denied_query
        {
            self.flash_line_search_denied();
        }
    }

    pub(in crate::ui) fn close_line_search(&mut self) {
        self.line_search.close();
        self.line_search_snapshot.clear();
        self.line_search_surfaces.clear();
        self.line_search_denied_at = None;
        self.line_search_denied_query.clear();
        self.line_search_anim
            .target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
    }

    pub(in crate::ui) fn on_line_search(
        &mut self,
        _: &LineSearch,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_line_search_selection();
        self.line_search.open();
        let (targets, surfaces) = self.line_search_targets();
        self.line_search_snapshot = targets;
        self.line_search_surfaces = surfaces;
        self.line_search.set_query("", &self.line_search_snapshot);
        self.line_search_denied_at = None;
        self.line_search_denied_query.clear();
        self.line_search_anim
            .target(1.0, anim::STANDARD_MS, anim::Curve::Decelerate, &self.clock);
        cx.notify();
    }

    pub(in crate::ui) fn open_command_line(&mut self) {
        self.command_line.open();
        self.command_line
            .set_context(self.command_suggestion_context());
        self.clock.tick();
        self.command_line_anim
            .target(1.0, anim::STANDARD_MS, anim::Curve::Decelerate, &self.clock);
    }

    fn command_suggestion_context(&self) -> ipc_command::SuggestionContext {
        use ipc_command::{ContextRow, CurrentTrack, SuggestionContext};
        let current = self.player.current().map(|track| CurrentTrack {
            id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            album_id: track.album_id.clone(),
        });
        let kind_name = |kind: RowKind| match kind {
            RowKind::Song => "track",
            RowKind::Album => "album",
            RowKind::Artist => "artist",
            RowKind::Playlist => "playlist",
        };
        let mut rows = Vec::new();
        if let Some(page) = self.page.as_ref() {
            for row in page.rows.iter().take(32) {
                rows.push(ContextRow {
                    kind: kind_name(row.kind),
                    id: row.id.clone(),
                    title: row.title.clone(),
                    subtitle: row.subtitle.clone(),
                });
            }
        } else {
            for index in self.view.rows.live_start
                ..self
                    .view
                    .rows
                    .live_start
                    .saturating_add(self.view.rows.live.len())
            {
                if let Some(row) = self.view.rows.row(index) {
                    rows.push(ContextRow {
                        kind: kind_name(row.kind),
                        id: row.id.clone(),
                        title: row.title.clone(),
                        subtitle: row.subtitle.clone(),
                    });
                }
            }
        }
        SuggestionContext { current, rows }
    }

    pub(in crate::ui) fn close_command_line(&mut self) {
        self.command_line.close();
        self.command_line_anim
            .target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
    }

    pub(in crate::ui) fn command_line_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        match key {
            "escape" => {
                self.close_command_line();
                cx.notify();
                return;
            }
            "enter" => {
                let text = self.command_line.query().to_string();
                if text.trim().is_empty() {
                    self.close_command_line();
                    cx.notify();
                    return;
                }
                match ipc_command::parse(&text) {
                    Ok(command) => {
                        self.close_command_line();
                        self.dismiss_search_overlay();
                        self.pending_ipc.push(command);
                    }
                    Err(error) => {
                        let now = self.clock.now;
                        self.command_line.deny(now, error.to_string());
                    }
                }
                cx.notify();
                return;
            }
            "tab" => {
                self.command_line.complete(false);
                self.command_line.clear_denied();
                cx.notify();
                return;
            }
            "backtab" => {
                self.command_line.complete(true);
                self.command_line.clear_denied();
                cx.notify();
                return;
            }
            "c" if control => {
                if let Some(text) = self.command_line.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                if let Some(text) = self.command_line.editor.selected_text() {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.command_line.editor.replace_selection("");
                    self.command_line.reset_completion();
                    self.command_line.refresh();
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.command_line.insert(&text);
                }
            }
            _ => {
                let command =
                    command_for_key(key, shift, control, event.keystroke.key_char.as_deref());
                let Some(command) = command else { return };
                self.command_line.apply_edit(&command);
            }
        }
        self.command_line.clear_denied();
        cx.notify();
    }
}
