//! Applies backend `Event`s to the root state.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn apply(&mut self, event: Event) {
        match event {
            Event::Command(_) => {}
            Event::IpcRan(command) => self.toast_ipc(&command),
            Event::Header {
                server_url,
                username,
                server_type,
                last_sync,
                counts,
            } => {
                self.header = Some(Header {
                    server_url,
                    username,
                    server_type,
                    last_sync,
                    counts,
                });
                self.error = None;
                if !self.restored {
                    self.restored = true;
                }
            }
            Event::LibraryCount {
                generation,
                scope,
                order,
                count,
            } => {
                if generation < self.generation
                    || scope != self.view.scope
                    || order != self.view.order
                {
                    return;
                }
                self.generation = generation;
                self.view.rows.install_count(generation, count);
                self.view.loading = count == 0;
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.list.cursor = saved as usize;
                }
                self.view.list.cursor = self.view.list.cursor.min(count.saturating_sub(1));
                self.follow_library(false);
                self.ensure_library_window();
            }
            Event::LibraryWindow {
                generation,
                scope,
                order,
                start,
                rows,
            } => {
                if generation < self.generation
                    || scope != self.view.scope
                    || order != self.view.order
                {
                    return;
                }
                self.view.rows.install_window(start, rows);
                self.view.rebuild_covers();
                self.view.loading = false;
                self.view.error = None;
                self.ensure_marks();
            }
            Event::Library {
                generation,
                scope,
                order,
                rows,
            } => {
                if generation < self.generation
                    || scope != self.view.scope
                    || order != self.view.order
                {
                    return;
                }
                self.generation = generation;
                self.view.rows = RowWindow::full(rows);
                self.view.rows.generation = generation;
                self.view.rebuild_covers();
                self.view.loading = false;
                self.ensure_marks();
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.list.cursor = saved as usize;
                }
                self.view.list.cursor = self.view.list.cursor.min(self.view.rows.len().saturating_sub(1));
                self.follow_library(false);
            }
            Event::CommandSuggestions {
                generation,
                start,
                end,
                rows,
            } => {
                if generation != self.command_suggestions_generation || !self.command_line.is_open()
                {
                    return;
                }
                self.command_line
                    .install_search_suggestions(start, end, &rows);
            }
            Event::Notes { album_id, notes } => {
                self.notes = Some((album_id, notes));
            }
            Event::RowsPatched {
                kind,
                id,
                starred,
                rating,
            } => {
                let patch = |row: &mut Row| {
                    if row.id == id && row.kind == kind {
                        if let Some(starred) = starred {
                            row.starred = starred;
                        }
                        if let Some(rating) = rating {
                            row.rating = i64::from(rating);
                        }
                    }
                };
                for row in &mut self.view.rows.live {
                    patch(row);
                }
                if let Some(page) = self.page.as_mut() {
                    for row in &mut page.rows {
                        patch(row);
                    }
                }
                if let Some(search) = self.search.as_mut() {
                    for row in &mut search.rows {
                        patch(row);
                    }
                }
                if matches!(kind, RowKind::Album | RowKind::Artist)
                    && let Some(page) = self.page.as_mut().filter(|page| {
                        page.descriptor.kind != PageKind::Playlist
                            && page.descriptor.kind != PageKind::Radio
                            && page.descriptor.id == id
                    })
                {
                    if let Some(starred) = starred {
                        page.header.starred = starred;
                    }
                    if let Some(rating) = rating {
                        page.header.rating = i64::from(rating);
                    }
                }
                // The queue's favourite mark is owned by the core: it arrives
                // as a `QueueDelta::Patch`, so echoing a `SetTrackFavorite`
                // here would feed the mutation back into the core.
                if kind == RowKind::Album
                    && let Some((album_id, album_starred, album_rating)) =
                        self.fullscreen_album.as_mut()
                    && *album_id == id
                {
                    if let Some(starred) = starred {
                        *album_starred = starred;
                    }
                    if let Some(rating) = rating {
                        *album_rating = rating;
                    }
                }
            }
            Event::AlbumState {
                id,
                starred,
                rating,
            } => {
                if self
                    .player
                    .current()
                    .is_some_and(|track| track.album_id == id)
                {
                    self.fullscreen_album = Some((id, starred, rating));
                }
            }
            Event::SyncProgress {
                message: _,
                done,
                total,
            } => {
                self.mark_sync_progress(done, total);
            }
            Event::LibrarySynced => {
                self.sync_progress = 1.0;
                self.finish_sync();
                self.marks.signature = None;
                self.bridge.refresh_header();
                self.reload();
                if let Some(page) = self.page.as_ref() {
                    let descriptor = page.descriptor.clone();
                    self.bridge.load_page(descriptor);
                }
                self.push_toast("Library synced".to_string(), ToastKind::Notice, None);
            }
            Event::RadioSaved {
                name,
                offline,
                remote_error,
            } => {
                if let Some(error) = remote_error {
                    self.push_toast(
                        format!("Saved “{name}” locally; server save failed: {error}"),
                        ToastKind::Error,
                        None,
                    );
                } else if offline {
                    self.push_toast(
                        format!("Saved radio “{name}” locally"),
                        ToastKind::Notice,
                        None,
                    );
                } else {
                    self.push_toast(format!("Saved radio “{name}”"), ToastKind::Notice, None);
                }
                if self.page.is_none() && self.view.scope == Scope::Playlists {
                    self.reload();
                }
            }
            Event::Wiped { scope } => {
                self.marks.signature = None;
                match scope.as_str() {
                    "covers" => {
                        if let Some(covers) = COVERS.get()
                            && let Ok(mut covers) = covers.lock()
                        {
                            covers.clear();
                        }
                        self.palette_cover = None;
                        self.palette_motion.reset(&self.clock);
                    }
                    "lyrics" => {
                        self.lyrics = None;
                        self.lyrics_requested = self.player.current().map(|track| track.id.clone());
                        self.lyrics_overlay = false;
                        self.lyrics_overlay_wanted = false;
                        self.lyrics_layout = None;
                        self.release_lyrics_hold();
                    }
                    "album" => {
                        if let Some(page) = self.page.as_ref()
                            && matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        {
                            let kind = page.descriptor.kind;
                            let id = page.descriptor.id.clone();
                            self.bridge.request_container_state(kind, id);
                        }
                    }
                    _ => {}
                }
                self.bridge.refresh_header();
                self.reload();
                if let Some(page) = self.page.as_ref() {
                    let descriptor = page.descriptor.clone();
                    self.bridge.load_page(descriptor);
                }
                self.push_toast(format!("Wiped {scope}"), ToastKind::Notice, None);
            }

            Event::PageContainerState {
                kind,
                id,
                cached,
                starred,
                rating,
            } => {
                if let Some(page) = self
                    .page
                    .as_mut()
                    .filter(|page| page.descriptor.kind == kind && page.descriptor.id == id)
                {
                    page.container_cached = cached;
                    page.header.starred = starred;
                    page.header.rating = i64::from(rating);
                    if !cached {
                        page.container_downloading = false;
                    }
                }
            }
            Event::QueueAppend { tracks: _ } => {
                // Retired in IPC v2: the core streams `QueueDelta::Insert`.
            }
            Event::QueuePaste { tracks: _, at: _ } => {
                // Retired in IPC v2: the core streams `QueueDelta::Insert` and
                // the rows themselves; only the local clipboard bookkeeping
                // stays here.
                self.paste_backup = None;
                self.pending_playlist_cursor = None;
            }
            Event::DownloadStarted { kind, id, on } => {
                self.apply_download_started(&kind, &id, on);
            }
            Event::DownloadSettled => {
                self.push_toast("Download updated.".to_string(), ToastKind::Notice, None);
                self.marks.signature = None;
                self.ensure_marks();
                if let Some((kind, id)) = self.page.as_ref().and_then(|page| {
                    matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        .then(|| (page.descriptor.kind, page.descriptor.id.clone()))
                }) {
                    if let Some(page) = self.page.as_mut() {
                        page.container_downloading = false;
                    }
                    self.bridge.request_container_state(kind, id);
                }
            }
            Event::OpenPage { descriptor } => {
                self.open_descriptor(descriptor);
            }
            Event::OpenPageReveal {
                descriptor,
                row_kind,
                row_id,
            } => {
                let reveal = PendingReveal {
                    page_kind: descriptor.kind,
                    page_id: descriptor.id.clone(),
                    row_kind,
                    row_id,
                };
                self.open_descriptor_reveal(descriptor, Some(reveal));
            }
            Event::Preload {
                scope,
                order,
                count,
                rows,
            } => {
                if self.trace {
                    crate::debug_log!(
                        "GPUI trace: preload {} order {} count {}",
                        scope.label(),
                        order,
                        count
                    );
                }
                self.warm.insert((scope, order), (count, rows));
            }
            Event::Marks { generation, result } => {
                // The next render re-checks the window signature; while the
                // player/fullscreen surfaces render every frame this is enough.
                self.marks.update(generation, result);
            }
            Event::Restore { .. } => {
                // Retired in IPC v2: the core replays `PlaybackState` and the
                // queue (Reset + rows) on connect, and sends `OpenPage` for the
                // restored page itself.
            }
            Event::Page {
                generation,
                descriptor,
                header,
                rows,
            } => {
                if generation < self.page_generation {
                    return;
                }
                self.page_generation = generation;
                let page = self
                    .page
                    .get_or_insert_with(|| PageState::new(descriptor.clone()));
                if page.descriptor != descriptor {
                    *page = PageState::new(descriptor.clone());
                }
                crate::debug_log!(
                    "GPUI: page cover={:?} title={}",
                    header.cover_url,
                    header.title
                );
                page.header = header;
                page.rows = rows;
                page.rebuild_covers();
                page.loading = false;
                page.error = None;
                if let Some((id, row, keep_selection)) = self.pending_playlist_cursor.as_ref()
                    && page.descriptor.kind == PageKind::Playlist
                    && &page.descriptor.id == id
                {
                    let row = *row;
                    let keep_selection = *keep_selection;
                    self.pending_playlist_cursor = None;
                    page.list.cursor = row.min(page.rows.len().saturating_sub(1));
                    if !keep_selection {
                        page.list.selection.clear();
                        self.visual_mode = false;
                    }
                }
                page.list.cursor = page.list.cursor.min(page.rows.len().saturating_sub(1));
                let reveal_index = self.pending_page_reveal.as_ref().and_then(|pending| {
                    if page.descriptor.kind != pending.page_kind
                        || page.descriptor.id != pending.page_id
                    {
                        return None;
                    }
                    page.rows.iter().position(|row| match pending.row_kind {
                        state::RowKind::Song => {
                            row.song_id == pending.row_id || row.id == pending.row_id
                        }
                        state::RowKind::Album
                        | state::RowKind::Artist
                        | state::RowKind::Playlist => row.id == pending.row_id,
                    })
                });
                if let Some(index) = reveal_index {
                    self.pending_page_reveal = None;
                    page.list.cursor = index;
                    page.list.selection.clear();
                }
                self.follow_page(false);
                if let Some(index) = reveal_index {
                    self.land_page(index);
                }
                self.marks.signature = None;
                self.ensure_marks();
                if let Some((kind, id)) = self.page.as_ref().and_then(|page| {
                    matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        .then(|| (page.descriptor.kind, page.descriptor.id.clone()))
                }) {
                    self.bridge.request_container_state(kind, id);
                }
            }
            Event::LoginRequired { offline_available } => {
                self.login = Some(Self::login_state(LoginMode::SignIn, offline_available));
            }
            Event::LoggedIn {
                server_url,
                username,
                offline,
            } => {
                self.reset_for_account();
                self.login = None;
                self.header = Some(Header {
                    server_url,
                    username,
                    server_type: String::new(),
                    last_sync: None,
                    counts: (0, 0, 0),
                });
                self.error = Some(if offline {
                    "Offline mode".to_string()
                } else {
                    "Logged in".to_string()
                });
                self.bridge.load_library(self.view.scope, self.view.order);
                if !offline {
                    self.bridge.refresh_header();
                }
            }
            Event::LoggedOut => {
                self.reset_for_account();
                self.login = Some(Self::login_state(LoginMode::SignIn, false));
                self.header = None;
                self.error = Some("Logged out".to_string());
            }
            Event::PlaylistSaved {
                name,
                offline,
                created,
                removed,
                moved,
                rows,
                playlist_id,
                cursor,
                error,
            } => {
                let failed = error.is_some();
                if !failed && !removed && rows > 0 {
                    self.paste_backup = None;
        self.pending_playlist_cursor = None;
                }
                if failed
                    && self.clipboard_kind.is_none()
                    && self.clipboard_ids.is_empty()
                    && let Some((kind, ids)) = self.paste_backup.take()
                {
                    self.clipboard_kind = Some(kind);
                    self.clipboard_ids = ids;
                }
                let verb = if removed {
                    "remove from"
                } else if created {
                    "create"
                } else {
                    "update"
                };
                if let Some(error) = error {
                    self.push_toast(
                        format!("Could not {verb} “{name}”: {error}"),
                        ToastKind::Error,
                        None,
                    );
                } else if offline {
                    self.push_toast(format!("Saved “{name}” locally"), ToastKind::Notice, None);
                } else if created {
                    self.push_toast(format!("Created “{name}”"), ToastKind::Notice, None);
                } else if removed && rows > 0 {
                    self.push_toast(
                        format!(
                            "Deleted {rows} {} from “{name}”",
                            if rows == 1 { "row" } else { "rows" }
                        ),
                        ToastKind::Notice,
                        None,
                    );
                } else if removed {
                    self.push_toast(format!("Removed from “{name}”"), ToastKind::Notice, None);
                } else if moved {
                    // Reordering is visible in the list; no toast to spam.
                } else if rows > 0 {
                    self.push_toast(
                        format!(
                            "Added {rows} {} to “{name}”",
                            if rows == 1 { "track" } else { "tracks" }
                        ),
                        ToastKind::Notice,
                        None,
                    );
                } else {
                    self.push_toast(format!("Added to “{name}”"), ToastKind::Notice, None);
                }
                if failed {
                    return;
                }
                if self.page.is_none() && self.view.scope == Scope::Playlists {
                    self.reload();
                }
                // The backend refreshed the playlist's cached entries before
                // sending this event; reload an open playlist page so the rows
                // and count match what was just added or removed, and park the
                // cursor on the edited row for the reload.
                if let (Some(id), Some(row)) = (playlist_id, cursor)
                    && self.page.as_ref().is_some_and(|page| {
                        page.descriptor.kind == PageKind::Playlist
                            && page.descriptor.id == id
                    })
                {
                    self.pending_playlist_cursor = Some((id, row, moved));
                }
                if let Some(descriptor) = self
                    .page
                    .as_ref()
                    .filter(|page| page.descriptor.kind == PageKind::Playlist)
                    .map(|page| page.descriptor.clone())
                {
                    self.bridge.load_page(descriptor);
                }
            }
            Event::PlaylistDeleted { id, name, error } => {
                if let Some(error) = error {
                    self.push_toast(
                        format!("Could not delete playlist “{name}”: {error}"),
                        ToastKind::Error,
                        None,
                    );
                } else {
                    self.push_toast(
                        format!("Deleted playlist “{name}”"),
                        ToastKind::Notice,
                        None,
                    );
                    if self
                        .page
                        .as_ref()
                        .is_some_and(|page| {
                            page.descriptor.kind == PageKind::Playlist
                                && page.descriptor.id == id
                        })
                    {
                        self.page = None;
                        self.travel.clear();
                    }
                    if self.view.scope == Scope::Playlists {
                        self.reload();
                    }
                }
            }
            Event::AccountRemoved { label, next } => {
                if next.is_some() {
                    self.reset_for_account();
                    self.login = None;
                    self.error = Some(format!("Removed {label}"));
                    self.bridge.refresh_header();
                    self.bridge.load_library(self.view.scope, self.view.order);
                } else {
                    self.reset_for_account();
                    self.login = Some(Self::login_state(LoginMode::SignIn, false));
                    self.header = None;
                    self.error = Some(format!("Removed {label}"));
                }
            }
            Event::AuthError(message) => {
                if let Some(login) = self.login.as_mut() {
                    login.busy = false;
                    login.error = Some(message.clone());
                }
                self.error = Some(message);
            }
            Event::Lyrics { song_id, document } => {
                crate::debug_log!(
                    "GPUI: lyrics {} lines synced={} source={}",
                    song_id,
                    document.lines.len(),
                    document.source
                );
                let new_song = self.lyrics.as_ref().map(|lyrics| lyrics.song_id.clone())
                    != Some(song_id.clone());
                self.lyrics = Some(LyricsState {
                    song_id,
                    synced: document.synced,
                    source: document.source,
                    lines: document
                        .lines
                        .into_iter()
                        .map(|line| (line.start_ms, line.value))
                        .collect(),
                });
                if new_song {
                    self.lyrics_window_start = 0;
                    self.lyrics_line_opacity.clear();
                    self.lyrics_line_emphasis.clear();
                    self.lyrics_line_color.clear();
                    self.lyrics_layout = None;
                    self.release_lyrics_hold();
                    self.lyrics_track_fade
                        .target(0.0, 0.0, anim::Curve::Standard, &self.clock);
                    self.lyrics_track_fade
                        .target(1.0, 300.0, anim::Curve::Decelerate, &self.clock);
                }
            }
            Event::Palette { cover_art, roles } => {
                let current = self
                    .player
                    .current()
                    .map(|track| track.cover_art.as_str())
                    .unwrap_or_default();
                if current == cover_art {
                    let next = palette::Palette::from_roles(&roles);
                    self.clock.tick();
                    self.palette_motion.set(next, &self.clock, true);
                }
            }
            Event::PaletteReset { cover_art } => {
                let current = self
                    .player
                    .current()
                    .map(|track| track.cover_art.as_str())
                    .unwrap_or_default();
                if current == cover_art {
                    self.clock.tick();
                    self.palette_motion.reset(&self.clock);
                }
            }
            Event::Queue { tracks: _, start: _ } => {
                // Retired in IPC v2: `QueueDelta::Reset` + `QueueRows` replace
                // the full queue payload.
                self.playback_error = None;
            }
            Event::Search {
                generation,
                query,
                rows,
                most_played,
            } => {
                if let Some(search) = self.search.as_mut()
                    && generation >= search.generation
                {
                    if search.editor.text != query {
                        return;
                    }
                    search.generation = generation;
                    let rows_changed = search.merge_rows(rows, most_played);
                    search.loading = false;
                    self.search_loading_since = None;
                    if !most_played && !search.editor.text.is_empty() && search.rows.is_empty() {
                        self.search_denied_at = Some(self.clock.now);
                        self.search_denied_query = search.editor.text.clone();
                    }
                    if rows_changed {
                        search.scope_cursors = [0; 4];
                        search.selection.clear();
                        self.hovered_row = None;
                        crate::debug_log!(
                            "GPUI: search scopes songs={} albums={} artists={} playlists={}",
                            search.scope_count(0),
                            search.scope_count(1),
                            search.scope_count(2),
                            search.scope_count(3)
                        );
                        if search.scope_count(search.scope) == 0 {
                            for step in 0..4 {
                                let candidate = (search.scope + step) % 4;
                                if search.scope_count(candidate) > 0 {
                                    search.scope = candidate;
                                    break;
                                }
                            }
                        }
                        self.follow_search(false);
                    }
                }
            }
            Event::Error(message) => {
                if self.clipboard_kind.is_none()
                    && self.clipboard_ids.is_empty()
                    && let Some((kind, ids)) = self.paste_backup.take()
                {
                    self.clipboard_kind = Some(kind);
                    self.clipboard_ids = ids;
                }
                self.finish_sync();
                let reconcile = self
                    .page
                    .as_mut()
                    .filter(|page| page.container_downloading)
                    .map(|page| {
                        page.container_downloading = false;
                        (page.descriptor.kind, page.descriptor.id.clone())
                    });
                if let Some((kind, id)) = reconcile
                    && matches!(kind, PageKind::Album | PageKind::Artist)
                {
                    self.bridge.request_container_state(kind, id);
                }
                // A failed download/removal never sends `DownloadSettled`, so
                // roll the optimistic row marks back from the ledger.
                self.marks.signature = None;
                self.ensure_marks();
                if let Some(search) = self.search.as_mut() {
                    search.loading = false;
                }
                self.view.loading = false;
                self.view.error = Some(message.clone());
                self.error = Some(message.clone());
                let lower = message.to_lowercase();
                let (kind, retry) = if lower.contains("playback") {
                    (
                        ToastKind::Error,
                        self.last_play.as_ref().map(|_| RetryAction::Playback),
                    )
                } else if lower.contains("library")
                    || lower.contains("search")
                    || lower.contains("sync")
                {
                    (ToastKind::Sync, Some(RetryAction::ReloadLibrary))
                } else {
                    (ToastKind::Error, None)
                };
                self.push_toast(message, kind, retry);
            }
        }
    }
}

impl AppRoot {
    pub(in crate::ui) fn login_state(mode: LoginMode, offline_available: bool) -> LoginState {
        LoginState {
            mode,
            server: TextModal::new(),
            username: TextModal::new(),
            password: TextModal::new(),
            field: 0,
            busy: false,
            error: None,
            offline_available,
        }
    }

    fn reset_for_account(&mut self) {
        // The queue and transport belong to the core; an account switch makes
        // it replay a fresh state/queue rather than mutating the view here.
        self.player_bar_cover.clear();
        self.playback_error = None;
        self.view = ViewState::default();
        self.generation = 0;
        self.page = None;
        self.page_generation = 0;
        self.travel.clear();
        self.search = None;
        self.search_open = false;
        self.search_loading_since = None;
        self.queue_open = false;
        self.queue_list.cursor = 0;
        self.queue_list.selection.clear();
        self.clipboard_kind = None;
        self.clipboard_ids.clear();
        self.paste_backup = None;
        self.pending_playlist_cursor = None;
        self.visual_mode = false;
        self.fullscreen = false;
        self.fullscreen_anim = anim::Behavior::new(0.0);
        self.fullscreen_album = None;
        self.fullscreen_album_requested = None;
        self.lyrics = None;
        self.lyrics_requested = None;
        self.lyrics_overlay = false;
        self.lyrics_overlay_wanted = false;
        self.lyrics_layout = None;
        self.lyrics_active_line = None;
        self.release_lyrics_hold();
        self.line_search.close();
        self.command_line.close();
        self.pending_ipc.clear();
        self.alt.dismiss();
        self.alt_menu = None;
        self.alt_target = None;
        self.alt_held = false;
        self.alt_was_typing = false;
        self.marks = MarksState::default();
        self.warm.clear();
        self.restored = false;
        self.header = None;
        self.notes = None;
        self.status_focus = None;
        self.library_scroll = scroll_modal::ScrollModal::new(scroll_modal::LIBRARY);
        self.page_scroll_modal = scroll_modal::ScrollModal::new(scroll_modal::ALBUM);
        self.queue_scroll_modal = scroll_modal::ScrollModal::new(scroll_modal::QUEUE);
        self.search_scroll = scroll_modal::ScrollModal::new(scroll_modal::SEARCH);
    }
}
