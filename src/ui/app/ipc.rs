//! IPC command validation and execution.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn handle_command(&mut self, command: &str, cx: &mut Context<Self>) {
        match ipc_command::parse(command) {
            Ok(command) => self.pending_ipc.push(command),
            Err(error) => crate::debug_log!("ipc: ignored {command:?}: {error}"),
        }
        cx.notify();
    }

    /// Execute one shared IPC/`:` command. Runs with a live `Window`, so it can
    /// reuse the existing action handlers instead of duplicating them.
    pub(in crate::ui) fn execute_ipc_command(
        &mut self,
        command: ipc_command::IpcCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use ipc_command::{IpcCommand, QuickDirection, WipeScope};
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
            IpcCommand::Logout => self.bridge.logout(),
            IpcCommand::AddAccount => {
                if self.login.is_none() {
                    self.login = Some(Self::login_state(LoginMode::AddAccount, false));
                }
            }
            IpcCommand::RemoveAccount { keep_data } => self.bridge.remove_account(keep_data),
            IpcCommand::CreatePlaylist { name, offline } => match self.selected_song_ids() {
                Ok(ids) => self.bridge.create_playlist(name, ids, offline),
                Err(error) => self.push_toast(error, ToastKind::Error, None),
            },
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
            IpcCommand::QueueUp => self.on_queue_up(&QueueUp, window, cx),
            IpcCommand::QueueDown => self.on_queue_down(&QueueDown, window, cx),
            IpcCommand::RepeatCycle => self.on_repeat_cycle(&RepeatCycle, window, cx),
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
            IpcCommand::PlayPause => self.on_play_pause(&PlayPause, window, cx),
            IpcCommand::Play => self.player.resume(),
            IpcCommand::Pause => self.player.pause(),
            IpcCommand::Stop => self.player.stop(),
            IpcCommand::NextTrack => self.on_next_track(&NextTrack, window, cx),
            IpcCommand::PrevTrack => self.on_prev_track(&PrevTrack, window, cx),
            IpcCommand::Mute => self.on_mute(&Mute, window, cx),
            IpcCommand::VolumeUp => self.on_volume_up(&VolumeUp, window, cx),
            IpcCommand::VolumeDown => self.on_volume_down(&VolumeDown, window, cx),
            IpcCommand::SetVolume(volume) => self.player.set_volume(volume),
            IpcCommand::SeekBack => self.on_seek_back(&SeekBack, window, cx),
            IpcCommand::SeekForward => self.on_seek_forward(&SeekForward, window, cx),
            IpcCommand::Seek(position) => self.player.seek(position),
            IpcCommand::SeekRelative(offset) => {
                self.player.seek((self.player.position() + offset).max(0.0))
            }
            IpcCommand::PlaySong(id) => {
                self.last_play = Some((id.clone(), self.view.order_label().to_string()));
                self.bridge
                    .play_song(id, self.view.order_label().to_string());
            }
            IpcCommand::PlaySongInAlbum(id) => self.bridge.play_song_in_album(id),
            IpcCommand::OpenSongAlbum(id) => self.bridge.quick_go_album(id),
            IpcCommand::DownloadSong(id) => {
                self.bridge.quick_download("song".to_string(), id, true)
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
                if let Some(menu) = self.alt_menu.clone() {
                    self.alt.cycle(if next { 1 } else { -1 }, &menu);
                }
            }
            IpcCommand::QuickCancel => {
                self.alt.dismiss();
                self.alt_menu = None;
                self.alt_target = None;
                self.animate_alt(0.0);
            }
            IpcCommand::Quit => self.on_quit(&Quit, window, cx),
        }
        cx.notify();
    }
}
