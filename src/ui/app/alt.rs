//! Alt gesture snapshots, arm/release/aim, and quick-menu dispatch.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn page_alt_menu(
        &self,
        page: &PageBarState,
    ) -> Option<(QuickMenu, AltTarget)> {
        let (menu, kind, radio_seed_kind, radio_seed_id) = match page.kind {
            PageKind::Album => (
                QuickMenu::album_tool(page.cached, page.starred, page.rating),
                RowKind::Album,
                String::new(),
                String::new(),
            ),
            PageKind::Artist => (
                QuickMenu::artist_tool(page.cached, page.starred, page.rating),
                RowKind::Artist,
                String::new(),
                String::new(),
            ),
            PageKind::Radio => (
                QuickMenu::radio_tool(),
                RowKind::Song,
                page.radio_seed_kind
                    .clone()
                    .unwrap_or_else(|| "album".to_string()),
                page.id.clone(),
            ),
            _ => return None,
        };
        Some((
            menu,
            AltTarget {
                kind,
                id: page.id.clone(),
                album_id: if page.kind == PageKind::Album {
                    page.id.clone()
                } else {
                    String::new()
                },
                song_id: String::new(),
                playlist_id: String::new(),
                radio_seed_kind,
                radio_seed_id,
                ids: vec![page.id.clone()],
                selection: false,
            },
        ))
    }

    pub(in crate::ui) fn open_page_menu(&mut self, anchor_slot: usize, sticky: bool) {
        let Some(page) = self.page_bar() else { return };
        let Some((menu, target)) = self.page_alt_menu(&page) else {
            return;
        };
        self.clock.tick();
        self.alt = AltGesture::with_sticky(sticky);
        if !self.alt.arm(&menu) {
            return;
        }
        self.alt_menu = Some(menu);
        self.alt_target = Some(target);
        self.alt_anchor = Some(self.status_anchor(anchor_slot));
        self.animate_alt(1.0);
    }

    pub(in crate::ui) fn open_wipe_menu(&mut self) {
        self.clock.tick();
        let menu = QuickMenu::wipe();
        self.alt = AltGesture::with_sticky(true);
        if !self.alt.arm(&menu) {
            return;
        }
        self.alt_menu = Some(menu);
        self.alt_target = None;
        self.alt_anchor = Some(self.status_anchor(1));
        self.animate_alt(1.0);
    }

    pub(in crate::ui) fn animate_alt(&mut self, target: f32) {
        // Keep the legacy PopFx pop on open; the faster ALT_MS applies to
        // both directions, and the collapse keeps its plain ease-in.
        let curve = if target > 0.5 {
            anim::Curve::OutBack
        } else {
            anim::Curve::Accelerate
        };
        self.alt_spawn
            .target(target, anim::ALT_MS, curve, &self.clock);
    }

    pub(in crate::ui) fn activate_status(&mut self, slot: usize) {
        self.status_focus = Some(slot);
        match slot {
            0 => self.start_sync(SyncMode::Incremental),
            1 => {
                if self.syncing {
                    self.push_toast(
                        "Wait for the sync to finish before wiping.".to_string(),
                        ToastKind::Notice,
                        None,
                    );
                    return;
                }
                self.open_wipe_menu();
            }
            2 => self.activate_container_download(),
            4 => self.activate_container_favorite(),
            5 => self.open_page_menu(5, true),
            _ => {}
        }
    }

    pub(in crate::ui) fn activate_container_download(&mut self) {
        let Some(page) = self.page_bar() else { return };
        if page.downloading {
            return;
        }
        let on = !page.cached;
        if let Some(page) = self.page.as_mut() {
            page.container_cached = on;
            page.container_downloading = true;
        }
        match page.kind {
            PageKind::Album => {
                self.bridge.quick_download("album".to_string(), page.id, on);
            }
            PageKind::Artist => {
                self.bridge.download_artist(page.id, on);
            }
            _ => {}
        }
    }

    pub(in crate::ui) fn activate_container_favorite(&mut self) {
        let Some(page) = self.page_bar() else { return };
        let kind = match page.kind {
            PageKind::Album => "album",
            PageKind::Artist => "artist",
            _ => return,
        };
        if let Some(page) = self.page.as_mut() {
            page.header.starred = !page.header.starred;
        }
        self.bridge
            .quick_favorite(kind.to_string(), page.id, !page.starred);
    }

    pub(in crate::ui) fn alt_menu_for_row(&self, row: &Row) -> QuickMenu {
        match row.kind {
            RowKind::Song => QuickMenu::song_with(row.starred),
            RowKind::Album => QuickMenu::album_with(
                !row.album_id.is_empty() && self.marks.albums_done.contains(&row.album_id),
                row.starred,
                row.rating.clamp(0, 5) as u8,
            ),
            RowKind::Artist => QuickMenu::artist_with(row.starred, row.rating.clamp(0, 5) as u8),
            RowKind::Playlist => QuickMenu::playlist(),
        }
    }

    pub(in crate::ui) fn alt_target_for_row(&self, row: &Row) -> AltTarget {
        AltTarget {
            kind: row.kind,
            id: row.id.clone(),
            album_id: row.album_id.clone(),
            song_id: row.song_id.clone(),
            playlist_id: if row.kind == RowKind::Playlist {
                row.id.clone()
            } else {
                String::new()
            },
            radio_seed_kind: String::new(),
            radio_seed_id: String::new(),
            ids: vec![row.id.clone()],
            selection: false,
        }
    }

    pub(in crate::ui) fn alt_anchor_for(
        &self,
        handle: &UniformListScrollHandle,
        index: usize,
        row_h: f32,
    ) -> (f32, f32) {
        let state = handle.0.borrow();
        let bounds = state.base_handle.bounds();
        let offset = state.base_handle.offset();
        let x = f32::from(bounds.origin.x) + f32::from(bounds.size.width) * 0.5;
        let y =
            f32::from(bounds.origin.y) + index as f32 * row_h + f32::from(offset.y) + row_h * 0.5;
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    pub(in crate::ui) fn fullscreen_anchor(&self, focus: FullscreenFocus) -> (f32, f32) {
        let anchors = self.fullscreen_anchors.borrow();
        let stored = match focus {
            FullscreenFocus::Cover => anchors.cover,
            FullscreenFocus::Title => anchors.title,
            FullscreenFocus::Lyrics => None,
        };
        let (x, y) = stored.unwrap_or(match focus {
            FullscreenFocus::Cover => (self.win_w * 0.5, self.win_h * 0.42),
            FullscreenFocus::Title => (self.win_w * 0.5, self.win_h * 0.86),
            FullscreenFocus::Lyrics => (self.win_w * 0.5, self.win_h * 0.5),
        });
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    pub(in crate::ui) fn alt_scroll_anchor(
        &self,
        modal: &scroll_modal::ScrollModal,
        index: usize,
        count: usize,
    ) -> (f32, f32) {
        modal
            .row_center(index, count)
            .map(|(x, y)| {
                (
                    x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
                    y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
                )
            })
            .unwrap_or_else(|| self.alt_anchor_for(&modal.handle, index, modal.row_h))
    }

    pub(in crate::ui) fn quick_kind(kind: RowKind) -> QuickKind {
        match kind {
            RowKind::Song => QuickKind::Song,
            RowKind::Album => QuickKind::Album,
            RowKind::Artist => QuickKind::Artist,
            RowKind::Playlist => QuickKind::Playlist,
        }
    }

    pub(in crate::ui) fn alt_selection_target(
        &self,
        rows: Vec<Row>,
        cursor: usize,
        anchor: (f32, f32),
    ) -> Option<(QuickMenu, AltTarget, (f32, f32))> {
        if rows.is_empty() {
            return None;
        }
        let kind = rows[0].kind;
        let all_starred = rows.iter().all(|row| row.starred);
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let target = AltTarget {
            kind,
            id: rows[cursor.min(rows.len() - 1)].id.clone(),
            album_id: rows[cursor.min(rows.len() - 1)].album_id.clone(),
            song_id: rows[cursor.min(rows.len() - 1)].song_id.clone(),
            playlist_id: String::new(),
            radio_seed_kind: String::new(),
            radio_seed_id: String::new(),
            ids,
            selection: true,
        };
        Some((
            QuickMenu::selection(Self::quick_kind(kind), all_starred),
            target,
            anchor,
        ))
    }

    pub(in crate::ui) fn search_anchor_for(&self, index: usize, row_h: f32) -> (f32, f32) {
        if let Some(search) = self.search.as_ref() {
            let count = search.scope_count(search.scope);
            if let Some((x, y)) = self.search_scroll.row_center(index, count) {
                return (
                    x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
                    y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
                );
            }
        }
        let state = self.search_scroll.handle.0.borrow();
        let bounds = state.base_handle.bounds();
        let height = f32::from(bounds.size.height);
        let (x, y) = if height > 0.0 {
            (
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) * 0.5,
                f32::from(bounds.origin.y)
                    + index as f32 * row_h
                    + f32::from(state.base_handle.offset().y)
                    + row_h * 0.5,
            )
        } else {
            (
                self.win_w * 0.5,
                self.win_h * 0.14
                    + 16.0
                    + 54.0
                    + 12.0
                    + 30.0
                    + 12.0
                    + index as f32 * row_h
                    + row_h * 0.5,
            )
        };
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    pub(in crate::ui) fn superior_mode(&self) -> SuperiorMode {
        SuperiorMode::from_page(self.page.as_ref())
    }

    pub(in crate::ui) fn page_bar(&self) -> Option<PageBarState> {
        let page = self.page.as_ref()?;
        Some(PageBarState {
            kind: page.descriptor.kind,
            id: page.descriptor.id.clone(),
            title: page.header.title.clone(),
            subtitle: page.header.subtitle.clone(),
            meta: page.header.meta.clone(),
            rating: page.header.rating.clamp(0, 5) as u8,
            starred: page.header.starred,
            cached: page.container_cached,
            downloading: page.container_downloading,
            radio_seed_kind: page.descriptor.radio_seed_kind.clone(),
        })
    }

    pub(in crate::ui) fn topbar_actions(&self) -> Vec<TopBarAction> {
        let page = self.page_bar();
        topbar_actions_for(self.superior_mode(), page.as_ref())
    }

    pub(in crate::ui) fn topbar_tab_order(&self) -> Vec<usize> {
        self.topbar_actions()
            .into_iter()
            .map(TopBarAction::slot)
            .collect()
    }

    pub(in crate::ui) fn status_anchor(&self, scope: usize) -> (f32, f32) {
        let stored = self.status_anchors.borrow()[scope.min(5)];
        let (x, y) = stored.unwrap_or((
            (self.win_w - 42.0).max(30.0),
            (self.win_h * 0.0648 * 0.5).max(30.0),
        ));
        (
            x.clamp(30.0, (self.win_w - 30.0).max(30.0)),
            y.clamp(30.0, (self.win_h - 30.0).max(30.0)),
        )
    }

    pub(in crate::ui) fn alt_snapshot(&self) -> Option<(QuickMenu, AltTarget, (f32, f32))> {
        if self.fullscreen {
            let track = self.player.current()?;
            if self.fullscreen_focus == FullscreenFocus::Lyrics {
                return None;
            }
            if self.fullscreen_focus == FullscreenFocus::Cover && !track.album_id.is_empty() {
                let album_id = track.album_id.clone();
                let (starred, rating) = self
                    .fullscreen_album
                    .as_ref()
                    .filter(|(id, _, _)| *id == album_id)
                    .map(|(_, starred, rating)| (*starred, *rating))
                    .unwrap_or((false, 0));
                let cached = self.marks.albums_done.contains(&album_id);
                let menu = QuickMenu::album_with(cached, starred, rating);
                let target = AltTarget {
                    kind: RowKind::Album,
                    id: album_id.clone(),
                    album_id: album_id.clone(),
                    song_id: String::new(),
                    playlist_id: String::new(),
                    radio_seed_kind: String::new(),
                    radio_seed_id: String::new(),
                    ids: vec![album_id],
                    selection: false,
                };
                return Some((menu, target, self.fullscreen_anchor(FullscreenFocus::Cover)));
            }
            let menu = QuickMenu::song_with(track.favorite);
            let target = AltTarget {
                kind: RowKind::Song,
                id: track.id.clone(),
                album_id: track.album_id.clone(),
                song_id: track.id.clone(),
                playlist_id: String::new(),
                radio_seed_kind: String::new(),
                radio_seed_id: String::new(),
                ids: vec![track.id.clone()],
                selection: false,
            };
            return Some((menu, target, self.fullscreen_anchor(FullscreenFocus::Title)));
        }
        if let Some(focus) = self.status_focus {
            // The scoped status region owns Alt. Wipe opens the maintenance
            // board; Favorite and Tool open the page's contextual actions.
            if focus == TopBarAction::Wipe.slot() {
                if self.syncing {
                    return None;
                }
                return Some((
                    QuickMenu::wipe(),
                    AltTarget {
                        kind: RowKind::Song,
                        id: String::new(),
                        album_id: String::new(),
                        song_id: String::new(),
                        playlist_id: String::new(),
                        radio_seed_kind: String::new(),
                        radio_seed_id: String::new(),
                        ids: Vec::new(),
                        selection: false,
                    },
                    self.status_anchor(focus),
                ));
            }
            if focus == TopBarAction::Favorite.slot() || focus == TopBarAction::Tool.slot() {
                let page = self.page_bar()?;
                let (menu, target) = self.page_alt_menu(&page)?;
                return Some((menu, target, self.status_anchor(focus)));
            }
            return None;
        }
        if let Some(search) = self.search.as_ref().filter(|_| self.search_open) {
            let scoped = search.scoped();
            let cursor = search.cursor();
            let anchor = self.search_anchor_for(cursor, 56.0);
            if search.selection.active() {
                let anchor_index = search.selection.anchor().unwrap_or(cursor);
                let lo = anchor_index.min(cursor);
                let hi = anchor_index.max(cursor);
                let rows = scoped
                    .get(lo..=hi.min(scoped.len().saturating_sub(1)))
                    .unwrap_or_default()
                    .iter()
                    .map(|row| (*row).clone())
                    .collect::<Vec<_>>();
                if let Some(hit) =
                    self.alt_selection_target(rows, cursor.saturating_sub(lo), anchor)
                {
                    return Some(hit);
                }
            }
            let row = scoped.get(cursor)?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                anchor,
            ));
        }

        if self.queue_open {
            let queue = self.player.queue_snapshot();
            let track = queue.get(self.queue_cursor)?;
            let menu = QuickMenu::song_with(track.favorite);
            let target = AltTarget {
                kind: RowKind::Song,
                id: track.id.clone(),
                album_id: track.album_id.clone(),
                song_id: track.id.clone(),
                playlist_id: String::new(),
                radio_seed_kind: String::new(),
                radio_seed_id: String::new(),
                ids: vec![track.id.clone()],
                selection: false,
            };
            return Some((
                menu,
                target,
                self.alt_scroll_anchor(
                    &self.queue_scroll_modal,
                    self.queue_cursor,
                    self.player.len(),
                ),
            ));
        }
        if let Some(page) = self.page.as_ref()
            && page.selection.active()
        {
            let anchor_index = page.selection.anchor().unwrap_or(page.cursor);
            let lo = anchor_index.min(page.cursor);
            let hi = anchor_index.max(page.cursor);
            let rows = page.rows.get(lo..=hi).unwrap_or_default().to_vec();
            if let Some(hit) = self.alt_selection_target(
                rows,
                page.cursor.saturating_sub(lo),
                self.alt_scroll_anchor(&self.page_scroll_modal, page.cursor, page.rows.len()),
            ) {
                return Some(hit);
            }
        }
        if let Some(page) = self.page.as_ref() {
            let row = page.rows.get(page.cursor)?;
            return Some((
                self.alt_menu_for_row(row),
                self.alt_target_for_row(row),
                self.alt_scroll_anchor(&self.page_scroll_modal, page.cursor, page.rows.len()),
            ));
        }
        if self.view.selection.active() {
            let anchor_index = self.view.selection.anchor().unwrap_or(self.view.cursor);
            let lo = anchor_index.min(self.view.cursor);
            let hi = anchor_index.max(self.view.cursor);
            let rows: Vec<Row> = (lo..=hi)
                .filter_map(|index| self.view.rows.get(index).cloned())
                .collect();
            if let Some(hit) = self.alt_selection_target(
                rows,
                self.view.cursor.saturating_sub(lo),
                self.alt_scroll_anchor(
                    &self.library_scroll,
                    self.view.cursor,
                    self.view.rows.count,
                ),
            ) {
                return Some(hit);
            }
        }
        let row = self.view.rows.get(self.view.cursor)?;
        Some((
            self.alt_menu_for_row(row),
            self.alt_target_for_row(row),
            self.alt_scroll_anchor(&self.library_scroll, self.view.cursor, self.view.rows.count),
        ))
    }

    pub(in crate::ui) fn alt_arm(&mut self) {
        self.clock.tick();
        if self.login.is_some() || self.line_search.open {
            return;
        }
        let Some((menu, target, anchor)) = self.alt_snapshot() else {
            return;
        };
        if self.status_focus.is_some() {
            self.alt = AltGesture::new();
        }
        if !self.alt.arm(&menu) {
            return;
        }
        if let Some(search) = self.search.as_mut()
            && search.typing
        {
            self.alt_was_typing = true;
            search.typing = false;
        }
        self.alt_menu = Some(menu);
        self.alt_target = Some(target);
        self.alt_anchor = Some(anchor);
        self.animate_alt(1.0);
    }

    pub(in crate::ui) fn alt_release(&mut self) {
        self.clock.tick();
        let from_search = self.search_open
            && self.search.is_some()
            && self.alt_target.is_some()
            && self.status_focus.is_none();
        let mut acted = false;
        if self.alt_was_typing {
            if let Some(search) = self.search.as_mut() {
                search.selection.clear();
                search.typing = true;
            }
            self.alt_was_typing = false;
        }
        if self.alt.armed {
            let menu = self.alt_menu.clone();
            if let Some(menu) = menu
                && self.alt.release_hold()
            {
                if let Some(verb) = self.alt.confirm_plan(&menu) {
                    self.dispatch_quick(verb);
                    acted = true;
                }
                if let Some(verb) = self.alt.commit_rating(&menu) {
                    self.dispatch_quick(verb);
                    acted = true;
                }
            }
            self.alt.cancel();
        }
        if acted && from_search {
            self.dismiss_search_after_quick();
        }
        self.animate_alt(0.0);
    }

    pub(in crate::ui) fn dispatch_quick(&mut self, verb: QuickVerb) {
        match verb {
            QuickVerb::WipeLibrary => {
                self.bridge.wipe("library".to_string());
                return;
            }
            QuickVerb::WipeIndexes => {
                self.bridge.wipe("indexes".to_string());
                return;
            }
            QuickVerb::WipeCovers => {
                self.bridge.wipe("covers".to_string());
                return;
            }
            _ => {}
        }
        let Some(target) = self.alt_target.clone() else {
            return;
        };
        if !target.radio_seed_id.is_empty() {
            let seed_kind = target.radio_seed_kind.clone();
            let seed_id = target.radio_seed_id.clone();
            match verb {
                QuickVerb::AddToQueue => self.bridge.queue_add_radio(seed_kind, seed_id),
                QuickVerb::SaveRadio => {
                    let title = self
                        .page
                        .as_ref()
                        .map(|page| page.header.title.trim().to_string())
                        .filter(|title| !title.is_empty())
                        .unwrap_or_else(|| "Radio".to_string());
                    self.bridge.save_radio(seed_kind, seed_id, title);
                }
                QuickVerb::ShuffleRadio => self.bridge.shuffle_radio(seed_kind, seed_id),
                QuickVerb::DownloadRadio => self.bridge.download_radio(seed_kind, seed_id),
                _ => {}
            }
            return;
        }
        if target.selection {
            match verb {
                QuickVerb::Favorite { on } => {
                    for id in &target.ids {
                        self.bridge
                            .quick_favorite(target.queue_kind().to_string(), id.clone(), on);
                    }
                }
                QuickVerb::AddToQueue => {
                    for id in &target.ids {
                        self.bridge
                            .quick_queue_add(target.queue_kind().to_string(), id.clone());
                    }
                }
                QuickVerb::Download { on } => {
                    for id in &target.ids {
                        self.bridge
                            .quick_download(target.queue_kind().to_string(), id.clone(), on);
                    }
                }
                _ => {}
            }
            return;
        }
        match verb {
            QuickVerb::AddToQueue => {
                self.bridge
                    .quick_queue_add(target.queue_kind().to_string(), target.id.clone());
            }
            QuickVerb::StartRadio => {
                let seed_id = if !target.song_id.is_empty() {
                    target.song_id.clone()
                } else {
                    target.id.clone()
                };
                self.open_descriptor(PageDescriptor::radio(
                    target.queue_kind().to_string(),
                    seed_id,
                ));
            }
            QuickVerb::Favorite { on } => {
                self.bridge
                    .quick_favorite(target.queue_kind().to_string(), target.id.clone(), on);
            }
            QuickVerb::RateAlbum { level } => {
                self.bridge
                    .quick_rate("album".to_string(), target.id.clone(), level);
            }
            QuickVerb::RateArtist { level } => {
                self.bridge
                    .quick_rate("artist".to_string(), target.id.clone(), level);
            }
            QuickVerb::GoToAlbum => {
                self.bridge.quick_go_album(target.song_id.clone());
            }
            QuickVerb::GoToArtist => {
                self.bridge.quick_go_artist(target.album_id.clone());
            }
            QuickVerb::PlayPlaylist => {
                self.bridge.quick_play_playlist(target.playlist_id.clone());
            }
            QuickVerb::ShuffleArtist => {
                self.bridge.shuffle_artist(target.id.clone());
            }
            QuickVerb::ShuffleAlbum => {
                self.bridge.shuffle_album(target.id.clone());
            }
            QuickVerb::Download { on } => {
                if target.kind == RowKind::Artist {
                    self.bridge.download_artist(target.id.clone(), on);
                } else {
                    self.bridge.quick_download(
                        target.queue_kind().to_string(),
                        target.id.clone(),
                        on,
                    );
                }
            }
            QuickVerb::SaveRadio
            | QuickVerb::ShuffleRadio
            | QuickVerb::DownloadRadio
            | QuickVerb::WipeLibrary
            | QuickVerb::WipeIndexes
            | QuickVerb::WipeCovers => {}
        }
    }

    pub(in crate::ui) fn on_modifiers_changed(
        &mut self,
        event: &gpui::ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let alt = event.modifiers.alt;
        if alt && !self.alt_held {
            self.alt_held = true;
            self.alt_arm();
        } else if !alt && self.alt_held {
            self.alt_held = false;
            self.alt_release();
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_alt_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let control = event.keystroke.modifiers.control;
        match key {
            "escape" => {
                self.alt.dismiss();
                self.alt_menu = None;
                self.animate_alt(0.0);
            }
            "h" if shift => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(-1, &menu);
                }
            }
            "l" if shift => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(1, &menu);
                }
            }
            "h" | "left" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::West, &menu, false);
                }
            }
            "j" if control => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(1, &menu);
                }
            }
            "k" if control => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(-1, &menu);
                }
            }
            "j" | "down" | "ctrl-j" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::South, &menu, false);
                }
            }
            "k" | "up" | "ctrl-k" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::North, &menu, false);
                }
            }
            "l" | "right" => {
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.aim(Direction::East, &menu, false);
                }
            }
            _ => {}
        }
        cx.notify();
    }
}
