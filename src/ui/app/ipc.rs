//! IPC command validation and execution.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn handle_command(&mut self, command: &str, cx: &mut Context<Self>) {
        match ipc_command::parse(command) {
            // Forwarded socket commands were announced by the core already;
            // this path also serves internal key helpers (track next/prev).
            Ok(command) => self.pending_ipc.push(PendingIpc {
                command,
                announce: false,
            }),
            Err(error) => crate::debug_log!("ipc: ignored {command:?}: {error}"),
        }
        cx.notify();
    }

    /// Execute one shared IPC/`:` command. Runs with a live `Window`, so it can
    /// reuse the existing action handlers instead of duplicating them.
    pub(in crate::ui) fn execute_ipc_command(
        &mut self,
        command: ipc_command::IpcCommand,
        announce: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use ipc_command::{IpcCommand, QuickDirection, WipeScope};
        use crate::ipc_protocol::PlayerOp;
        // Commands typed on the `:` line are echoed here (socket commands were
        // announced by the core). Announce before dispatch so an early return
        // cannot swallow it.
        if announce {
            self.toast_ipc(&ipc_command::canonical_string(&command));
        }
        // The core resolves relative volume/seek and the queue itself; the UI
        // only forwards the typed op (never a locally mutated value).
        if let Some(op) = command.player_op() {
            match op {
                PlayerOp::PlayPause => {
                    self.press_transport(1);
                    self.player.toggle();
                }
                PlayerOp::Play => self.player.resume(),
                PlayerOp::Pause => self.player.pause(),
                PlayerOp::Stop => self.player.stop(),
                PlayerOp::Next => self.execute_track_next(cx),
                PlayerOp::Previous => self.execute_track_previous(cx),
                PlayerOp::ToggleMute => self.player.toggle_mute(),
                PlayerOp::VolumeUp => self.player.volume_up(),
                PlayerOp::VolumeDown => self.player.volume_down(),
                PlayerOp::SetVolume(volume) => self.player.set_volume(volume),
                PlayerOp::Seek(seconds) => self.player.seek(seconds),
                PlayerOp::SeekRelative(offset) => self.player.seek_relative(offset),
                PlayerOp::CycleRepeat => self.player.cycle_repeat(),
                PlayerOp::ShuffleQueue => self.player.shuffle_queue(),
                PlayerOp::SetMuted(_)
                | PlayerOp::Jump(_)
                | PlayerOp::Remove(_)
                | PlayerOp::RemoveMany(_)
                | PlayerOp::Move { .. }
                | PlayerOp::MoveBlock { .. }
                | PlayerOp::SetTrackFavorite { .. }
                | PlayerOp::Resync => {
                    unreachable!("IPC playback mapping returned a non-playback op")
                }
            }
            cx.notify();
            return;
        }
        match command {
            IpcCommand::Sync(mode) => self.start_sync(mode),
            IpcCommand::Wipe(scope) => {
                if self.syncing {
                    self.push_toast(
                        "Wait for the sync to finish before wiping.".to_string(),
                        ToastKind::Notice,
                        None,
                    );
                } else {
                    let scope = match scope {
                        WipeScope::Album => "album",
                        WipeScope::Lyrics => "lyrics",
                        WipeScope::Covers => "covers",
                    };
                    self.bridge.wipe(scope.to_string());
                }
            }
            IpcCommand::WipeDatabases => self.open_wipe_menu(),
            IpcCommand::Logout => {
                self.bridge.logout();
            }
            IpcCommand::AddAccount => {
                if self.login.is_none() {
                    self.login = Some(Self::login_state(LoginMode::AddAccount, false));
                }
            }
            IpcCommand::RemoveAccount { keep_data } => {
                self.bridge.remove_account(keep_data);
            }
            IpcCommand::PlaylistCreate { name, offline } => {
                match self.player.current().map(|track| track.id.clone()) {
                    Some(id) => {
                        self.bridge.create_playlist(name, vec![id], offline);
                    }
                    None => self.push_toast(
                        "Nothing is playing; playlist create needs a current track.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
            IpcCommand::PlaylistAdd { targets, playlist } => {
                let current = self.player.current().map(|track| track.id.clone());
                self.bridge.playlist_add(targets, playlist, current);
            }
            IpcCommand::PlaylistAddSelected { playlist } => match self.selected_playlist_ids() {
                Some((kind, ids)) => {
                    self.bridge
                        .playlist_add_selected(kind.to_string(), ids, playlist);
                }
                None => self.push_toast(
                    "Nothing selected to add.".to_string(),
                    ToastKind::Notice,
                    None,
                ),
            },
            IpcCommand::PlaylistAddQueue { playlist } => {
                let rows = self.player.queue();
                let ids: Vec<String> = rows
                    .iter()
                    .map(|track| track.id.clone())
                    .filter(|id| !id.is_empty())
                    .collect();
                if rows.is_empty() {
                    self.push_toast(
                        "The queue is empty; nothing to save.".to_string(),
                        ToastKind::Notice,
                        None,
                    );
                } else if ids.len() != rows.len() {
                    // Queue bodies arrive in bounded chunks; a placeholder row
                    // would silently drop tracks from the saved playlist.
                    self.push_toast(
                        "The queue is still loading; try again.".to_string(),
                        ToastKind::Notice,
                        None,
                    );
                } else {
                    self.bridge.playlist_add_ids(ids, playlist);
                }
            }
            IpcCommand::PlaylistRemove { targets, playlist } => {
                let current = self.player.current().map(|track| track.id.clone());
                self.bridge.playlist_remove(targets, playlist, current);
            }
            IpcCommand::PlaylistDelete { playlist } => {
                self.bridge.delete_playlist(playlist);
            }
            IpcCommand::RadioSave { offline } => {
                let radio = self.page.as_ref().and_then(|page| {
                    (page.descriptor.kind == state::PageKind::Radio).then(|| {
                        (
                            page.descriptor.radio_seed_kind.clone().unwrap_or_default(),
                            page.descriptor.id.clone(),
                            page.header.title.trim().to_string(),
                        )
                    })
                });
                match radio {
                    Some((kind, id, name)) if !kind.is_empty() => {
                        if offline {
                            self.bridge.save_radio_local(kind, id, name);
                        } else {
                            self.bridge.save_radio(kind, id, name);
                        }
                    }
                    _ => self.push_toast(
                        "Open a radio page before saving it.".to_string(),
                        ToastKind::Error,
                        None,
                    ),
                }
            }
            IpcCommand::Shuffle { target } => self.execute_shuffle(target),
            IpcCommand::Radio { seed } => self.execute_radio(seed),
            IpcCommand::Download { request } => self.execute_download(request),
            IpcCommand::Refresh => self.on_refresh(&Refresh, window, cx),
            IpcCommand::Top => self.on_top(&Top, window, cx),
            IpcCommand::Bottom => self.on_bottom(&Bottom, window, cx),
            IpcCommand::MoveUp => self.on_move_up(&MoveUp, window, cx),
            IpcCommand::MoveDown => self.on_move_down(&MoveDown, window, cx),
            IpcCommand::ScrollUp | IpcCommand::ScrollDown => {
                let down = matches!(command, IpcCommand::ScrollDown);
                let modal = if self.queue_open {
                    &mut self.queue_scroll_modal
                } else if self.page.is_some() {
                    &mut self.page_scroll_modal
                } else {
                    &mut self.library_scroll
                };
                let current = -f32::from(modal.handle.0.borrow().base_handle.offset().y);
                let next = current + if down { modal.row_h } else { -modal.row_h };
                modal.on_wheel();
                modal.set_offset_now(next.max(0.0));
            }
            IpcCommand::PageUp => self.on_page_up(&PageUp, window, cx),
            IpcCommand::PageDown => self.on_page_down(&PageDown, window, cx),
            IpcCommand::MoveLeft | IpcCommand::MoveRight => {
                let left = matches!(command, IpcCommand::MoveLeft);
                if self.search.is_some() && self.search_open {
                    if left {
                        self.on_search_scope_prev(&SearchScopePrev, window, cx);
                    } else {
                        self.on_search_scope_next(&SearchScopeNext, window, cx);
                    }
                } else if self.fullscreen {
                    self.cycle_fullscreen_focus(left);
                }
            }
            IpcCommand::PrevPlace => self.step_travel(-1),
            IpcCommand::NextPlace => self.step_travel(1),
            IpcCommand::Home => self.on_home(&Home, window, cx),
            IpcCommand::Back => self.on_back(&Back, window, cx),
            IpcCommand::Enter => self.on_enter(&Enter, window, cx),
            IpcCommand::ScopeNext => self.on_scope_next(&ScopeNext, window, cx),
            IpcCommand::ScopePrev => self.on_scope_prev(&ScopePrev, window, cx),
            IpcCommand::OrderNext => self.on_order_next(&OrderNext, window, cx),
            IpcCommand::OrderPrev => self.on_order_prev(&OrderPrev, window, cx),
            IpcCommand::Visual => self.on_visual(&Visual, window, cx),
            IpcCommand::VisualAll => self.on_visual_all(&VisualAll, window, cx),
            IpcCommand::QueueToggle => self.on_queue_toggle(&QueueToggle, window, cx),
            IpcCommand::QueueRemove => self.on_queue_remove(&QueueRemove, window, cx),
            IpcCommand::Yank => self.on_yank(&Yank, window, cx),
            IpcCommand::YankRow => self.on_yank_row(&YankRow, window, cx),
            IpcCommand::PasteBelow => self.on_paste_below(&PasteBelow, window, cx),
            IpcCommand::PasteAbove => self.on_paste_above(&PasteAbove, window, cx),
            IpcCommand::DeleteRow => self.on_delete_row(&DeleteRow, window, cx),
            IpcCommand::QueueUp => self.on_queue_up(&QueueUp, window, cx),
            IpcCommand::QueueDown => self.on_queue_down(&QueueDown, window, cx),
            IpcCommand::RepeatCycle => unreachable!("mapped above"),
            IpcCommand::ToggleFullscreen => {
                self.on_toggle_fullscreen(&ToggleFullscreen, window, cx)
            }
            IpcCommand::LyricsOverlay => self.on_lyrics_overlay(&LyricsOverlay, window, cx),
            IpcCommand::PaneCycle { backward } => self.on_tab(backward, cx),
            IpcCommand::Search { query } => {
                self.open_search(true);
                if !query.is_empty() {
                    if let Some(search) = self.search.as_mut() {
                        search.editor.set_text(&query);
                    }
                    self.search_edited();
                }
            }
            IpcCommand::InsertMode { query } => {
                self.open_search(true);
                if !query.is_empty() {
                    if let Some(search) = self.search.as_mut() {
                        search.editor.set_text(&query);
                        search.typing = true;
                    }
                    self.search_edited();
                }
            }
            IpcCommand::SearchNext => self.on_search_next(&SearchNext, window, cx),
            IpcCommand::SearchPrev => self.on_search_prev(&SearchPrev, window, cx),
            IpcCommand::SearchScopeNext => self.on_search_scope_next(&SearchScopeNext, window, cx),
            IpcCommand::SearchScopePrev => self.on_search_scope_prev(&SearchScopePrev, window, cx),
            IpcCommand::LineSearch => self.on_line_search(&LineSearch, window, cx),
            IpcCommand::PlayPause
            | IpcCommand::Play
            | IpcCommand::Pause
            | IpcCommand::Stop
            | IpcCommand::Track(_)
            | IpcCommand::Mute
            | IpcCommand::VolumeUp
            | IpcCommand::VolumeDown
            | IpcCommand::SetVolume(_)
            | IpcCommand::SeekBack
            | IpcCommand::SeekForward
            | IpcCommand::Seek(_)
            | IpcCommand::SeekRelative(_) => unreachable!("mapped above"),
            // Read-only core queries: ask the core over its text socket (the
            // UI thread stays free) and echo the reply in a toast.
            IpcCommand::Status => self.query_ipc("status", cx),
            IpcCommand::QueueQuery => self.query_ipc("queue", cx),
            IpcCommand::PlaySong(id) => {
                self.last_play = Some((id.clone(), self.view.order_label().to_string()));
                self.bridge
                    .play_song(id, self.view.order_label().to_string());
            }
            IpcCommand::PlaySongInAlbum(id) => {
                self.bridge.play_song_in_album(id);
            }
            IpcCommand::OpenSongAlbum(id) => {
                self.bridge.quick_go_album(id);
            }
            IpcCommand::DownloadSong(id) => {
                self.bridge.quick_download("song".to_string(), id, true);
            }
            IpcCommand::QuickHold(true) => {
                self.alt_held = true;
                self.alt_arm();
            }
            IpcCommand::QuickHold(false) => {
                self.alt_held = false;
                self.alt_release();
            }
            IpcCommand::QuickAim(direction) => {
                if let Some(menu) = self.alt_menu.clone() {
                    let direction = match direction {
                        QuickDirection::North => Direction::North,
                        QuickDirection::South => Direction::South,
                        QuickDirection::East => Direction::East,
                        QuickDirection::West => Direction::West,
                    };
                    self.alt.aim(direction, &menu, false);
                }
            }
            IpcCommand::QuickCycle { next } => {
                self.alt_cycle(if next { 1 } else { -1 });
            }
            IpcCommand::QuickCancel => {
                self.alt.dismiss();
                self.alt_menu = None;
                self.alt_target = None;
                self.animate_alt(0.0);
            }
            IpcCommand::Show => window.activate_window(),
            IpcCommand::RandomAlbum => {
                self.bridge.play_random_album();
            }
            IpcCommand::Quit => self.on_quit(&Quit, window, cx),
        }
        cx.notify();
    }

    /// Runs a read-only text query (`status`/`queue`) against the core socket
    /// off the UI thread and echoes the reply in a toast. The core does not
    /// forward these commands, so `:` is the only UI surface that runs them.
    fn query_ipc(&self, line: &'static str, cx: &Context<Self>) {
        cx.spawn(async move |this, cx| {
            let reply = cx
                .background_executor()
                .spawn(async move { crate::ipc::send_query(line) })
                .await;
            _ = this.update(cx, |this, cx| {
                match reply {
                    Some(text) => {
                        this.push_toast(format!("{line} · {text}"), ToastKind::Notice, None);
                    }
                    None => this.push_toast(
                        format!("{line}: the core did not answer."),
                        ToastKind::Error,
                        None,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
