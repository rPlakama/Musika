//! List/page/queue navigation, windowing, scroll follow and cover preload.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn reload(&mut self) {
        if let Some(covers) = COVERS.get()
            && let Ok(mut covers) = covers.lock()
        {
            covers.clear();
        }
        self.view.loading = true;
        self.view.rows.clear();
        self.view.covers.clear();
        self.marks.signature = None;
        self.generation = self.bridge.load_library(self.view.scope, self.view.order);
    }

    pub(in crate::ui) fn follow_library(&mut self, animated: bool) {
        self.clock.tick();
        self.library_scroll.start(
            self.view.rows.count,
            self.view.cursor,
            animated,
            self.clock.now,
        );
    }

    pub(in crate::ui) fn follow_page(&mut self, animated: bool) {
        self.status_focus = None;
        self.clock.tick();
        let Some(page) = self.page.as_ref() else {
            return;
        };
        self.selected_id = page.rows.get(page.cursor).map(|row| row.id.clone());
        self.page_scroll_modal
            .start(page.rows.len(), page.cursor, animated, self.clock.now);
    }

    pub(in crate::ui) fn follow_search(&mut self, _animated: bool) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let scope = search.scope;
        let count = search.scope_count(scope);
        let cursor = search.cursor();
        self.clock.tick();
        // Search navigation is keyboard-repeat heavy: snap the viewport to
        // the cursor so it can never lag behind and lose the active row.
        self.search_scroll
            .start(count, cursor, false, self.clock.now);
    }

    pub(in crate::ui) fn follow_queue(&mut self, animated: bool) {
        self.status_focus = None;
        self.clock.tick();
        self.selected_id = self
            .player
            .queue_snapshot()
            .get(self.queue_cursor)
            .map(|track| track.id.clone());
        self.queue_scroll_modal.start(
            self.player.len(),
            self.queue_cursor,
            animated,
            self.clock.now,
        );
    }

    /// Re-centre the active list after a window resize. The list surfaces
    /// normally only run this when a keyboard/scroll action happens; a
    /// resize changes the viewport under the cursor without any such action.
    pub(in crate::ui) fn recenter_after_resize(&mut self) {
        if self.queue_open {
            self.follow_queue(false);
        } else if self.page.is_some() {
            self.follow_page(false);
        } else if self.search_open && self.search.is_some() {
            self.follow_search(false);
        } else {
            self.follow_library(false);
        }
    }

    pub(in crate::ui) fn land_library(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        let cursor = self.library_scroll.land(self.view.rows.count, index);
        self.view.select(cursor);
        self.selected_id = self.view.rows.get(cursor).map(|row| row.id.clone());
        self.view.rows.requested = None;
        self.ensure_library_window();
        self.remember_current();
    }

    pub(in crate::ui) fn land_page(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        let Some(page) = self.page.as_mut() else {
            return;
        };
        let cursor = self.page_scroll_modal.land(page.rows.len(), index);
        page.select(cursor);
        self.selected_id = page.rows.get(cursor).map(|row| row.id.clone());
        self.hovered_page = None;
    }

    pub(in crate::ui) fn land_queue(&mut self, index: usize) {
        self.status_focus = None;
        self.reset_row_scales();
        let cursor = self.queue_scroll_modal.land(self.player.len(), index);
        self.queue_cursor = cursor;
        self.selected_id = self
            .player
            .queue_snapshot()
            .get(cursor)
            .map(|track| track.id.clone());
    }

    pub(in crate::ui) fn ensure_library_window(&mut self) {
        if self.page.is_some() || self.view.scope == Scope::Playlists {
            return;
        }
        let count = self.view.rows.count;
        if count == 0 || self.view.rows.generation == 0 {
            return;
        }
        let (top, measured) = {
            let state = self.library_scroll.handle.0.borrow();
            let top = state.base_handle.logical_scroll_top().0;
            let height = f32::from(state.base_handle.bounds().size.height);
            (top, (height > 0.0).then_some(height))
        };
        if let Some(viewport) = measured
            && self.library_scroll.ensure_metrics(viewport)
        {
            self.view.rows.requested = None;
            self.follow_library(false);
        }
        let viewport = measured.unwrap_or(600.0);
        let row_h = self.library_scroll.row_h;
        let visible = ((viewport / row_h).ceil() as usize).max(1);
        let overscan = 10;
        let cursor = self.view.cursor.min(count - 1);
        let mut first = top.saturating_sub(overscan);
        let mut last = (top + visible + overscan).min(count - 1);
        first = first.min(cursor);
        last = last.max(cursor);
        let start = first.saturating_sub(20);
        let end = (last + 20).min(count - 1);
        if self.view.rows.covers(start, end) {
            return;
        }
        self.view.rows.set_requested(start, end);
        if self.trace {
            crate::debug_log!(
                "GPUI trace: row window {}..{} cursor {} count {} row_h {:.1}",
                start,
                end,
                self.view.cursor,
                count,
                row_h
            );
        }
        self.bridge.request_library_window(
            self.view.rows.generation,
            self.view.scope,
            self.view.order,
            start,
            end - start + 1,
        );
    }

    pub(in crate::ui) fn ensure_queue_metrics(&mut self) {
        let measured = {
            let height = f32::from(
                self.queue_scroll_modal
                    .handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds()
                    .size
                    .height,
            );
            (height > 0.0).then_some(height)
        };
        let Some(viewport) = measured else { return };
        if self.queue_scroll_modal.ensure_metrics(viewport) {
            self.follow_queue(false);
        }
    }

    pub(in crate::ui) fn ensure_search_metrics(&mut self, window_height: f32) {
        let measured = f32::from(
            self.search_scroll
                .handle
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .height,
        );
        let viewport = if measured > 0.0 {
            measured
        } else {
            (window_height * 0.72).max(1.0)
        };
        if self.search_scroll.ensure_metrics(viewport) {
            self.follow_search(false);
        }
    }

    pub(in crate::ui) fn ensure_page_metrics(&mut self) {
        let measured = {
            let height = f32::from(
                self.page_scroll_modal
                    .handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds()
                    .size
                    .height,
            );
            (height > 0.0).then_some(height)
        };
        let Some(_viewport) = measured else { return };
        let reference = (self.win_h - 100.0).max(1.0);
        if self.page_scroll_modal.ensure_metrics_ref(reference) {
            self.marks.signature = None;
            self.follow_page(false);
        }
    }

    /// Stable hash of the marks window: the current track ids plus the ids of
    /// the visible rows. An unchanged window therefore costs a hash per frame
    /// and no allocation; a scroll, page change, track change or explicit
    /// invalidation (`signature = None`) triggers one request.
    fn marks_signature(&self) -> u64 {
        let player = self
            .player
            .current()
            .map(|track| (track.id.as_str(), track.album_id.as_str()));
        if let Some(page) = self.page.as_ref() {
            let (top, viewport) = {
                let state = self.page_scroll_modal.handle.0.borrow();
                let top = state.base_handle.logical_scroll_top().0;
                let viewport = state
                    .last_item_size
                    .map(|size| f32::from(size.item.height))
                    .unwrap_or(600.0);
                (top, viewport)
            };
            let visible = ((viewport / self.page_scroll_modal.row_h).ceil() as usize).max(1);
            let lo = top.saturating_sub(10);
            let hi = (top + visible + 10).min(page.rows.len().saturating_sub(1));
            let rows = page
                .rows
                .get(lo..=hi)
                .unwrap_or_default()
                .iter()
                .map(|row| (row.song_id.as_str(), row.album_id.as_str()));
            marks_signature(1, player, rows)
        } else {
            marks_signature(
                0,
                player,
                self.view
                    .rows
                    .live
                    .iter()
                    .map(|row| (row.song_id.as_str(), row.album_id.as_str())),
            )
        }
    }

    pub(in crate::ui) fn ensure_marks(&mut self) {
        let signature = self.marks_signature();
        if self.marks.signature == Some(signature) || self.marks.inflight {
            return;
        }
        let mut songs: Vec<String> = Vec::new();
        let mut albums: Vec<String> = Vec::new();
        // The fullscreen title shows the current track's marks even when it is
        // not part of the visible list window.
        if let Some(track) = self.player.current() {
            if !track.id.is_empty() {
                songs.push(track.id.clone());
            }
            if !track.album_id.is_empty() {
                albums.push(track.album_id.clone());
            }
        }
        let push = |row: &Row, songs: &mut Vec<String>, albums: &mut Vec<String>| {
            if !row.song_id.is_empty() {
                songs.push(row.song_id.clone());
            }
            if !row.album_id.is_empty() {
                albums.push(row.album_id.clone());
            }
        };
        if let Some(page) = self.page.as_ref() {
            let (top, viewport) = {
                let state = self.page_scroll_modal.handle.0.borrow();
                let top = state.base_handle.logical_scroll_top().0;
                let viewport = state
                    .last_item_size
                    .map(|size| f32::from(size.item.height))
                    .unwrap_or(600.0);
                (top, viewport)
            };
            let visible = ((viewport / self.page_scroll_modal.row_h).ceil() as usize).max(1);
            let lo = top.saturating_sub(10);
            let hi = (top + visible + 10).min(page.rows.len().saturating_sub(1));
            for row in page.rows.get(lo..=hi).unwrap_or_default() {
                push(row, &mut songs, &mut albums);
            }
        } else {
            for row in &self.view.rows.live {
                push(row, &mut songs, &mut albums);
            }
        }
        songs.sort_unstable();
        songs.dedup();
        albums.sort_unstable();
        albums.dedup();
        if songs.is_empty() && albums.is_empty() {
            return;
        }
        self.marks.signature = Some(signature);
        self.marks.inflight = true;
        if self.trace {
            crate::debug_log!(
                "GPUI trace: marks window {} songs, {} albums",
                songs.len(),
                albums.len()
            );
        }
        self.bridge.request_marks(songs, albums);
    }

    pub(in crate::ui) fn refresh_fullscreen_album_state(&mut self) {
        let Some(track) = self.player.current() else {
            return;
        };
        let album_id = track.album_id.clone();
        if album_id.is_empty() {
            return;
        }
        if self
            .fullscreen_album
            .as_ref()
            .is_some_and(|(id, _, _)| *id == album_id)
        {
            return;
        }
        if self.fullscreen_album_requested.as_deref() == Some(album_id.as_str()) {
            return;
        }
        self.fullscreen_album_requested = Some(album_id.clone());
        self.bridge.request_album_state(album_id);
    }

    pub(in crate::ui) fn page_hero_bucket(&self) -> u32 {
        covers::bucket_for(PAGE_HERO_PX, self.scale_factor)
    }

    pub(in crate::ui) fn fullscreen_hero_bucket(&self) -> u32 {
        covers::bucket_for(FULLSCREEN_HERO_PX, self.scale_factor)
    }

    pub(in crate::ui) fn hero_cover(&self, url: &str, target: u32) -> Option<CoverSource> {
        let local = COVERS
            .get_or_init(|| std::sync::Mutex::new(covers::CoverStore::new(768)))
            .lock()
            .ok()?
            .hero_url(url, target);
        local
            .map(CoverSource::Path)
            .or_else(|| remote_cover(url).map(CoverSource::Remote))
    }

    pub(in crate::ui) fn ensure_display_cover(&mut self, cover_art: &str, target: u32) {
        if cover_art.is_empty() {
            return;
        }
        let key = format!("{cover_art}:{target}");
        if let Some(retry_at) = self.display_retry.get(&key)
            && *retry_at > self.clock.now
        {
            return;
        }
        self.display_retry.remove(&key);
        if !self.display_requested.insert(key.clone()) {
            return;
        }
        self.bridge
            .request_display_cover(key, cover_art.to_string(), target);
    }

    pub(in crate::ui) fn display_cover(
        &self,
        cover_art: &str,
        target: u32,
    ) -> Option<std::path::PathBuf> {
        self.display_covers
            .get(&format!("{cover_art}:{target}"))
            .cloned()
    }

    pub(in crate::ui) fn set_scope(&mut self, scope: Scope) {
        if scope == self.view.scope {
            return;
        }
        self.remember_current();
        self.hovered_row = None;
        let wanted = (scope, 0);
        if let Some((count, rows)) = self.warm.remove(&wanted) {
            self.view.clear_for(scope);
            self.view.rows = RowWindow::full(rows);
            self.view.rows.count = count;
            self.view.rebuild_covers();
            self.view.loading = false;
            self.generation = self.bridge.load_library(scope, 0);
        } else {
            self.view.clear_for(scope);
            self.reload();
        }
    }

    pub(in crate::ui) fn reset_row_scales(&mut self) {
        self.clock.tick();
        let curve = self.visuals.motion.row_curve;
        for emphasis in self.row_scales.borrow_mut().values_mut() {
            emphasis.cover.target(0.0, 0.0, curve, &self.clock);
            emphasis.text.target(0.0, 0.0, curve, &self.clock);
        }
    }

    /// Row selection emphasis: 0 at rest, 1 when selected or hovered.
    ///
    /// Cover and text use separate factors because the text needs a little
    /// longer to settle. Both are linear; covers stay brief, text is 80%
    /// slower so the glyphs do not feel like they snapped into place. Keys are
    /// numeric `(surface, scope, index)` keys: the list paint path never
    /// formats or allocates a string per visible row.
    pub(in crate::ui) fn row_emphasis(
        &self,
        surface: LineSurface,
        scope: usize,
        index: usize,
        active: bool,
    ) -> (f32, f32) {
        let target = if active { 1.0 } else { 0.0 };
        let motion = self.visuals.motion;
        let curve = motion.row_curve;
        let mut scales = self.row_scales.borrow_mut();
        let emphasis = scales
            .entry((surface, scope, index))
            .or_insert_with(|| RowEmphasis::new(active));
        let cover = emphasis
            .cover
            .target(target, motion.row_cover_ms, curve, &self.clock);
        let text = emphasis
            .text
            .target(target, motion.row_text_ms, curve, &self.clock);
        (cover, text)
    }

    pub(in crate::ui) fn select(&mut self, index: usize) {
        self.status_focus = None;
        if !self.visual_mode {
            self.view.selection.clear();
        }
        self.view.select(index);
        self.selected_id = self.view.rows.get(index).map(|row| row.id.clone());
        self.hovered_row = None;
        self.follow_library(true);
        self.remember_current();
    }

    pub(in crate::ui) fn open_row_page(&mut self, row: &Row) {
        let kind = match row.kind {
            state::RowKind::Album => PageKind::Album,
            state::RowKind::Artist => PageKind::Artist,
            state::RowKind::Playlist => PageKind::Playlist,
            state::RowKind::Song => return,
        };
        self.open_descriptor(PageDescriptor::new(kind, row.id.clone()));
    }

    pub(in crate::ui) fn open_descriptor(&mut self, descriptor: PageDescriptor) {
        self.status_focus = None;
        self.pending_page_reveal = None;
        let profile = page_profile::PageProfile::for_descriptor(&descriptor);
        if profile.notes {
            if descriptor.kind == PageKind::Artist {
                self.bridge.request_artist_notes(descriptor.id.clone());
            } else {
                self.bridge.request_notes(descriptor.id.clone());
            }
        }
        self.remember_current();
        self.hovered_page = None;
        self.travel.open(descriptor.clone());
        self.page = Some(PageState::new(descriptor.clone()));
        self.page_generation = self.page_generation.wrapping_add(1);
        self.bridge.load_page(descriptor);
        self.follow_page(false);
    }

    pub(in crate::ui) fn open_descriptor_reveal(
        &mut self,
        descriptor: PageDescriptor,
        reveal: Option<PendingReveal>,
    ) {
        self.open_descriptor(descriptor);
        self.pending_page_reveal = reveal;
    }

    pub(in crate::ui) fn back_page(&mut self) {
        self.status_focus = None;
        self.remember_current();
        match self.travel.back() {
            Some(descriptor) => {
                self.hovered_page = None;
                self.page = Some(PageState::new(descriptor.clone()));
                self.page_generation = self.page_generation.wrapping_add(1);
                self.bridge.load_page(descriptor);
                self.follow_page(false);
            }
            None => self.page = None,
        }
    }

    pub(in crate::ui) fn step_travel(&mut self, delta: i32) {
        self.status_focus = None;
        self.remember_current();
        if let Some(descriptor) = self.travel.step(delta) {
            self.page = Some(PageState::new(descriptor.clone()));
            self.page_generation = self.page_generation.wrapping_add(1);
            self.bridge.load_page(descriptor);
            self.follow_page(false);
        }
    }

    pub(in crate::ui) fn cycle_order(&mut self, next: bool) {
        let len = self.view.order_len();
        if len <= 1 {
            return;
        }
        self.view.order = if next {
            (self.view.order + 1) % len
        } else {
            (self.view.order + len - 1) % len
        };
        self.reload();
    }

    pub(in crate::ui) fn remember_current(&self) {
        if let Some(page) = self.page.as_ref() {
            let key = page_key(&page.descriptor);
            crate::scroll_memory::remember(
                &key,
                page.cursor as i64,
                0.0,
                page.descriptor.kind as i64,
                "list",
            );
        } else {
            crate::scroll_memory::remember(
                self.view.scope.key(),
                self.view.cursor as i64,
                0.0,
                self.view.scope as i64,
                "list",
            );
        }
    }

    pub(in crate::ui) fn activate_page_cursor(&mut self) {
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let Some(row) = page.rows.get(page.cursor) else {
            return;
        };
        let profile = page_profile::PageProfile::for_descriptor(&page.descriptor);
        let action = profile.row_action(row);
        let index = page.cursor;
        let descriptor = page.descriptor.clone();
        let song_id = row.song_id.clone();
        let album_id = row.album_id.clone();
        match action {
            page_profile::PageAction::OpenAlbum => {
                if !album_id.is_empty() {
                    self.open_descriptor(PageDescriptor::new(PageKind::Album, album_id));
                }
            }
            page_profile::PageAction::PlayAlbum => {
                if let Some(page) = self.page.as_ref() {
                    self.bridge
                        .play_album(page.descriptor.id.clone(), page.cursor);
                }
            }
            page_profile::PageAction::PlayArtist => {
                if !song_id.is_empty() {
                    self.last_play = Some((song_id.clone(), self.view.order_label().to_string()));
                    self.bridge
                        .play_song(song_id, self.view.order_label().to_string());
                }
            }
            page_profile::PageAction::PlayPlaylist => {
                self.bridge.quick_play_playlist(descriptor.id);
            }
            page_profile::PageAction::PlayRadio => {
                let seed_kind = descriptor
                    .radio_seed_kind
                    .clone()
                    .unwrap_or_else(|| "album".to_string());
                self.bridge.play_radio(seed_kind, descriptor.id, index);
            }
        }
    }
    pub(in crate::ui) fn activate_row(&mut self, row: &Row) {
        if row.song_id.is_empty() && matches!(row.kind, state::RowKind::Song) {
            return;
        }
        if matches!(row.kind, state::RowKind::Song) {
            self.last_play = Some((row.song_id.clone(), self.view.order_label().to_string()));
            self.bridge
                .play_song(row.song_id.clone(), self.view.order_label().to_string());
            return;
        }
        self.open_row_page(row);
    }
}

/// Hash of one marks window. Split out so the "unchanged window is a no-op"
/// contract has a unit test without a window.
fn marks_signature<'a, I>(domain: u8, player: Option<(&'a str, &'a str)>, rows: I) -> u64
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    domain.hash(&mut hasher);
    match player {
        Some((song, album)) => {
            song.hash(&mut hasher);
            album.hash(&mut hasher);
        }
        None => u8::MAX.hash(&mut hasher),
    }
    for (song, album) in rows {
        song.hash(&mut hasher);
        album.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::marks_signature;

    fn rows<'a>(ids: &'a [(&'a str, &'a str)]) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        ids.iter().copied()
    }

    #[test]
    fn identical_windows_hash_equal() {
        let a = [("s1", "a1"), ("s2", "a2")];
        let b = [("s1", "a1"), ("s2", "a2")];
        assert_eq!(
            marks_signature(0, Some(("p1", "pa")), rows(&a)),
            marks_signature(0, Some(("p1", "pa")), rows(&b)),
        );
    }

    #[test]
    fn changed_rows_player_or_domain_hash_differently() {
        let window = [("s1", "a1"), ("s2", "a2")];
        let scrolled = [("s2", "a2"), ("s3", "a3")];
        let base = marks_signature(0, Some(("p1", "pa")), rows(&window));
        assert_ne!(base, marks_signature(0, Some(("p1", "pa")), rows(&scrolled)));
        assert_ne!(base, marks_signature(0, Some(("p2", "pa")), rows(&window)));
        assert_ne!(base, marks_signature(0, None, rows(&window)));
        assert_ne!(base, marks_signature(1, Some(("p1", "pa")), rows(&window)));
    }
}
