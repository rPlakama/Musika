//! Floating search state, scope changes and result activation.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn move_search_scope(&mut self, delta: i32) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let current = search.scope;
        search.scope_cursors[current] = search.cursor();
        let next = (current as i32 + delta).rem_euclid(4) as usize;
        search.scope = next;
        search.scope_cursors[next] =
            search.scope_cursors[next].min(search.scope_count(next).saturating_sub(1));
        search.selection.clear();
        self.follow_search(false);
    }

    pub(in crate::ui) fn open_search(&mut self, typing: bool) {
        self.status_focus = None;
        self.search_open = true;
        self.search = Some(SearchState {
            editor: TextModal::new(),
            rows: Vec::new(),
            most_played: false,
            scope: 0,
            scope_cursors: [0; 4],
            selection: state::Selection::default(),
            generation: 0,
            loading: true,
            typing,
        });
        self.clock.tick();
        self.search_loading_since = Some(self.clock.now);
        self.search_denied_at = None;
        self.bridge.search(String::new());
    }

    pub(in crate::ui) fn on_search(
        &mut self,
        _: &Search,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_search(true);
        cx.notify();
    }

    pub(in crate::ui) fn on_insert_mode(
        &mut self,
        _: &InsertMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(search) = self.search.as_mut() {
            search.typing = true;
            self.search_open = true;
        } else {
            self.open_search(true);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_search_next(
        &mut self,
        _: &SearchNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(1);
        }
        cx.notify();
    }

    pub(in crate::ui) fn on_search_prev(
        &mut self,
        _: &SearchPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(search) = self.search.as_mut() {
            search.move_cursor(-1);
        }
        cx.notify();
    }

    /// Fully disable the floating search overlay: no animation tail, no
    /// stale loading/denied state and no way for a late payload to resurrect it.
    /// A successful quick action launched from the Search overlay is a
    /// search success: close the overlay exactly like Enter on a result.
    pub(in crate::ui) fn dismiss_search_after_quick(&mut self) {
        if self.search_open && self.search.is_some() {
            self.dismiss_search_overlay();
        }
    }

    pub(in crate::ui) fn dismiss_search_overlay(&mut self) {
        self.search_open = false;
        self.search = None;
        self.search_loading_since = None;
        self.search_denied_at = None;
        self.search_denied_query.clear();
        self.search_anim
            .target(0.0, anim::STANDARD_MS, anim::Curve::Accelerate, &self.clock);
    }

    pub(in crate::ui) fn activate_search_result(&mut self, cx: &mut Context<Self>) {
        let row = self
            .search
            .as_ref()
            .and_then(|search| search.scoped().get(search.cursor()).cloned().cloned());
        let Some(row) = row else {
            cx.notify();
            return;
        };
        self.dismiss_search_overlay();
        // A chosen result is a navigation: dismiss the surfaces the search was
        // floating over so the destination is visible.
        self.fullscreen = false;
        self.lyrics_overlay = false;
        self.lyrics_overlay_wanted = false;
        self.queue_open = false;
        self.description = None;
        match row.kind {
            RowKind::Song => {
                if row.album_id.is_empty() {
                    self.activate_row(&row);
                } else {
                    let reveal = PendingReveal {
                        page_kind: PageKind::Album,
                        page_id: row.album_id.clone(),
                        row_kind: RowKind::Song,
                        row_id: row.song_id.clone(),
                    };
                    self.open_descriptor_reveal(
                        PageDescriptor::new(PageKind::Album, row.album_id.clone()),
                        Some(reveal),
                    );
                }
            }
            RowKind::Album => {
                self.bridge.open_album_artist_reveal(row.id.clone());
            }
            RowKind::Artist => {
                self.open_descriptor(PageDescriptor::new(PageKind::Artist, row.id.clone()));
            }
            RowKind::Playlist => {
                self.open_descriptor(PageDescriptor::new(PageKind::Playlist, row.id.clone()));
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn search_edited(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let query = search.editor.text.clone();
        let empty = query.is_empty();
        if let Some(search) = self.search.as_mut() {
            search.loading = true;
            if empty {
                search.rows.clear();
                search.most_played = true;
            }
        }
        self.clock.tick();
        self.search_loading_since = Some(self.clock.now);
        self.search_denied_at = None;
        self.bridge.search(query);
    }

    pub(in crate::ui) fn toggle_description(&mut self) {
        if self.description.is_some() {
            self.description = None;
            return;
        }
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let notes = self
            .notes
            .as_ref()
            .filter(|(id, _)| *id == page.descriptor.id)
            .map(|(_, notes)| notes.clone())
            .unwrap_or_default();
        if notes.is_empty() {
            return;
        }
        self.description = Some(DescriptionState {
            title: page.header.title.clone(),
            subtitle: page.header.subtitle.clone(),
            body: notes,
        });
    }

    pub(in crate::ui) fn search_denied_opacity(&self) -> f32 {
        anim::denied_opacity(self.search_denied_at, self.clock.now)
    }
}
