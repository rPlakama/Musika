//! Extracted view methods.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn apply(&mut self, event: Event) {
        match event {
            Event::Command(_) => {}
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
                    self.bridge.restore();
                }
            }
            Event::LibraryCount {
                generation,
                scope,
                order,
                count,
            } => {
                if generation < self.generation || scope != self.view.scope || order != self.view.order {
                    return;
                }
                self.generation = generation;
                self.view.rows.install_count(generation, count);
                self.view.loading = count == 0;
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.cursor = saved as usize;
                }
                self.view.cursor = self.view.cursor.min(count.saturating_sub(1));
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
                if generation < self.generation || scope != self.view.scope || order != self.view.order {
                    return;
                }
                self.view.rows.install_window(start, rows);
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
                if generation < self.generation || scope != self.view.scope || order != self.view.order {
                    return;
                }
                self.generation = generation;
                self.view.rows = RowWindow::full(rows);
                self.view.rows.generation = generation;
                self.view.loading = false;
                self.ensure_marks();
                self.view.error = None;
                let saved = crate::scroll_memory::saved_index(scope.key());
                if saved >= 0 {
                    self.view.cursor = saved as usize;
                }
                self.view.cursor = self.view.cursor.min(self.view.rows.len().saturating_sub(1));
                self.follow_library(false);
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
                if kind == RowKind::Song
                    && let Some(starred) = starred
                    && self
                        .player
                        .current()
                        .is_some_and(|track| track.id == id)
                {
                    self.player.set_track_favorite(&id, starred);
                }
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
                self.marks.requested = None;
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
                remote_error,
            } => {
                if let Some(error) = remote_error {
                    self.push_toast(
                        format!("Saved “{name}” locally; server save failed: {error}"),
                        ToastKind::Error,
                        None,
                    );
                } else {
                    self.push_toast(
                        format!("Saved radio “{name}”"),
                        ToastKind::Notice,
                        None,
                    );
                }
                if self.page.is_none() && self.view.scope == Scope::Playlists {
                    self.reload();
                }
            }
            Event::Wiped { scope } => {
                self.marks.requested = None;
                match scope.as_str() {
                    "covers" => {
                        if let Some(covers) = COVERS.get()
                            && let Ok(mut covers) = covers.lock()
                        {
                            covers.clear();
                        }
                        self.display_covers.clear();
                        self.display_requested.clear();
                        self.display_retry.clear();
                        self.palette_cover = None;
                        self.palette_motion.reset(&self.clock);
                    }
                    "lyrics" => {
                        self.lyrics = None;
                        self.lyrics_requested = self.player.current().map(|track| track.id.clone());
                        self.lyrics_overlay = false;
                        self.lyrics_overlay_wanted = false;
                        self.lyrics_layout = None;
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
            Event::QueueAppend { tracks } => {
                self.player.append(tracks);
            }
            Event::DownloadSettled => {
                self.marks.requested = None;
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
            Event::DisplayCover {
                key,
                cover_art,
                target,
                path,
            } => {
                match path {
                    Some(path) => {
                        self.display_retry.remove(&key);
                        self.display_covers.insert(key, path.clone());
                        if let Some(trace) = self.cover_trace.as_ref()
                            && trace.cover_art == cover_art
                            && trace.bucket == target
                        {
                            crate::debug_log!(
                                "album entry: proper res loaded cover={} bucket={} path={} elapsed_ms={:.1} at={}",
                                cover_art,
                                target,
                                path.display(),
                                trace.started.elapsed().as_secs_f64() * 1000.0,
                                unix_ms()
                            );
                        }
                    }
                    None => {
                        self.display_requested.remove(&key);
                        self.display_retry.insert(key, self.clock.now + 5.0);
                        crate::debug_log!(
                            "album entry: cover load failed cover={} bucket={} path=none",
                            cover_art,
                            target
                        );
                    }
                }
            }
            Event::Preload {
                scope,
                order,
                count,
                rows,
            } => {
                if self.trace {
                    crate::debug_log!("GPUI trace: preload {} order {} count {}", scope.label(), order, count);
                }
                self.warm.insert((scope, order), (count, rows));
            }
            Event::Marks { generation, result } => {
                self.marks.update(generation, result);
                if self.marks.dirty {
                    self.marks.dirty = false;
                    self.ensure_marks();
                }
            }
            Event::Restore {
                volume,
                muted,
                tracks,
                start,
                position,
                page_kind,
                page_id,
            } => {
                self.player.set_volume(volume);
                self.player.set_muted(muted);
                if !tracks.is_empty() {
                    self.player.load_paused(tracks, start);
                    if position > 0.0 {
                        self.player.seek(position);
                    }
                }
                if self.page.is_none() && !page_kind.is_empty() && !page_id.is_empty() {
                    let kind = match page_kind.as_str() {
                        "album" => Some(PageKind::Album),
                        "artist" => Some(PageKind::Artist),
                        "playlist" => Some(PageKind::Playlist),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        self.open_descriptor(PageDescriptor::new(kind, page_id));
                    }
                }
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
                crate::debug_log!("GPUI: page cover={:?} title={}", header.cover_url, header.title);
                page.header = header;
                page.rows = rows;
                page.loading = false;
                page.error = None;
                page.cursor = page.cursor.min(page.rows.len().saturating_sub(1));
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
                        _ => row.id == pending.row_id,
                    })
                });
                if let Some(index) = reveal_index {
                    self.pending_page_reveal = None;
                    page.cursor = index;
                    page.selection.clear();
                }
                self.follow_page(false);
                if let Some(index) = reveal_index {
                    self.land_page(index);
                }
                self.marks.requested = None;
                self.ensure_marks();
                if let Some((kind, id)) = self.page.as_ref().and_then(|page| {
                    matches!(page.descriptor.kind, PageKind::Album | PageKind::Artist)
                        .then(|| (page.descriptor.kind, page.descriptor.id.clone()))
                }) {
                    self.bridge.request_container_state(kind, id);
                }
            }
            Event::LoginRequired { offline_available } => {
                self.login = Some(LoginState {
                    server: String::new(),
                    username: String::new(),
                    password: String::new(),
                    field: 0,
                    busy: false,
                    error: None,
                    offline_available,
                });
            }
            Event::LoggedIn { server_url, username, offline } => {
                self.login = None;
                self.header = Some(Header {
                    server_url: server_url.clone(),
                    username: username.clone(),
                    server_type: String::new(),
                    last_sync: None,
                    counts: self.header.as_ref().map(|header| header.counts).unwrap_or((0, 0, 0)),
                });
                self.error = Some(if offline { "Offline mode".to_string() } else { "Logged in".to_string() });
                self.reload();
                if !offline {
                    self.bridge.refresh_header();
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
                let new_song = self.lyrics.as_ref().map(|lyrics| lyrics.song_id.clone()) != Some(song_id.clone());
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
                    self.lyrics_track_fade.target(0.0, 0.0, anim::Curve::Standard, &self.clock);
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
            Event::Queue { tracks, start } => {
                self.player.load(tracks, start);
                self.playback_error = None;
                self.request_current_lyrics();
                if self.queue_open {
                    self.land_queue(self.player.index());
                }
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
                    search.rows = rows;
                    search.most_played = most_played;
                    search.scope_cursors = [0; 4];
                    search.selection.clear();
                    self.hovered_row = None;
                    search.loading = false;
                    self.search_loading_since = None;
                    if !most_played
                        && !search.editor.text.is_empty()
                        && search.rows.is_empty()
                    {
                        self.search_denied_at = Some(self.clock.now);
                        self.search_denied_query = search.editor.text.clone();
                    }
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
                }
                self.follow_search(false);
            }
            Event::Error(message) => {
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
                if let Some(search) = self.search.as_mut() {
                    search.loading = false;
                }
                self.view.loading = false;
                self.view.error = Some(message.clone());
                self.error = Some(message.clone());
                let lower = message.to_lowercase();
                let (kind, retry) = if lower.contains("playback") {
                    (ToastKind::Error, self.last_play.as_ref().map(|_| RetryAction::Playback))
                } else if lower.contains("library") || lower.contains("search") || lower.contains("sync") {
                    (ToastKind::Sync, Some(RetryAction::ReloadLibrary))
                } else {
                    (ToastKind::Error, None)
                };
                self.push_toast(message, kind, retry);
            }
        }
    }
}
