//! Raw keyboard routing, including the login form.

use super::super::text_modal::TextModal;
use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn on_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.login.is_some() {
            cx.stop_propagation();
            self.login_key(event, cx);
            return;
        }
        if self.command_line.is_open() {
            cx.stop_propagation();
            self.command_line_key(event, _window, cx);
            return;
        }
        if self.line_search.open {
            cx.stop_propagation();
            self.line_search_key(event, _window, cx);
            return;
        }
        if self.fullscreen
            && !self.lyrics_overlay
            && !self.alt_held
            && !self.alt.armed
            && !(self.search.is_some() && self.search_open)
        {
            let key = event.keystroke.key.as_str();
            if key == "tab" || key == "backtab" {
                cx.stop_propagation();
                self.cycle_fullscreen_focus(key == "backtab");
                cx.notify();
                return;
            }
        }
        if self.alt_held || self.alt.armed {
            cx.stop_propagation();
            self.on_alt_key(event, cx);
            return;
        }
        if self.search.is_none() || !self.search_open {
            let key = event.keystroke.key.as_str();
            if key == ":" || key == "colon" || (key == ";" && event.keystroke.modifiers.shift) {
                cx.stop_propagation();
                self.open_command_line();
                cx.notify();
                return;
            }
            if key == "/" || key == "?" || key == "slash" {
                cx.stop_propagation();
                if event.keystroke.modifiers.shift || key == "?" {
                    self.on_lyrics_overlay(&LyricsOverlay, _window, cx);
                } else {
                    self.on_line_search(&LineSearch, _window, cx);
                }
                return;
            }
            if key == "escape" {
                self.on_back(&Back, _window, cx);
            }
            return;
        }
        let Some(search) = self.search.as_ref() else {
            return;
        };
        cx.stop_propagation();
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        if !search.typing {
            match key {
                "escape" => {
                    let had_selection = self
                        .search
                        .as_ref()
                        .is_some_and(|search| search.selection.active());
                    if had_selection {
                        if let Some(search) = self.search.as_mut() {
                            search.selection.clear();
                        }
                    } else {
                        self.search_open = false;
                        self.search_denied_at = None;
                    }
                    cx.notify();
                }
                "i" => {
                    if let Some(search) = self.search.as_mut() {
                        search.selection.clear();
                        search.typing = true;
                    }
                    cx.notify();
                }
                "enter" => {
                    if let Some(search) = self.search.as_mut() {
                        search.selection.clear();
                    }
                    self.activate_search_result(cx);
                }
                "tab" | "l" | "right" => {
                    self.on_search_scope_next(&SearchScopeNext, _window, cx);
                }
                "backtab" | "h" | "left" => {
                    self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                }
                "v" | "V" => {
                    if let Some(search) = self.search.as_mut() {
                        let cursor = search.cursor();
                        if key == "V" || shift {
                            search.selection.select_all();
                        } else if search.selection.active() {
                            search.selection.clear();
                        } else {
                            search.selection.start(cursor);
                        }
                    }
                    cx.notify();
                }
                "j" | "down" | "ctrl-n" => {
                    if let Some(search) = self.search.as_mut() {
                        if shift {
                            let cursor = search.cursor();
                            search.selection.start(cursor);
                        }
                        search.move_cursor(1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "k" | "up" | "ctrl-p" => {
                    if let Some(search) = self.search.as_mut() {
                        if shift {
                            let cursor = search.cursor();
                            search.selection.start(cursor);
                        }
                        search.move_cursor(-1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "n" if control => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "p" if control => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(-1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "pagedown" | "ctrl-d" => {
                    if let Some(search) = self.search.as_mut() {
                        for _ in 0..state::SEARCH_VISIBLE_ROWS {
                            search.move_cursor(1);
                        }
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "pageup" | "ctrl-u" => {
                    if let Some(search) = self.search.as_mut() {
                        for _ in 0..state::SEARCH_VISIBLE_ROWS {
                            search.move_cursor(-1);
                        }
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "g" => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] = 0;
                    }
                    self.follow_search(false);
                    cx.notify();
                }
                "G" => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] =
                            search.scope_count(search.scope).saturating_sub(1);
                    }
                    self.follow_search(false);
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        match key {
            "escape" => {
                if let Some(search) = self.search.as_mut() {
                    search.typing = false;
                }
                cx.notify();
                return;
            }
            "enter" => {
                self.activate_search_result(cx);
                return;
            }
            "tab" => {
                self.on_search_scope_next(&SearchScopeNext, _window, cx);
                return;
            }
            "backtab" => {
                self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                return;
            }
            "up" | "ctrl-p" => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(-1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "down" | "ctrl-n" => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "p" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(-1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "n" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "u" if control => {
                if let Some(search) = self.search.as_mut() {
                    for _ in 0..state::SEARCH_VISIBLE_ROWS {
                        search.move_cursor(-1);
                    }
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "d" if control => {
                if let Some(search) = self.search.as_mut() {
                    for _ in 0..state::SEARCH_VISIBLE_ROWS {
                        search.move_cursor(1);
                    }
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "c" if control => {
                if let Some(search) = self.search.as_ref()
                    && let Some(text) = search.editor.selected_text()
                {
                    let text = text.to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                let cut = self
                    .search
                    .as_ref()
                    .and_then(|search| search.editor.selected_text().map(str::to_string));
                if let Some(text) = cut {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    if let Some(search) = self.search.as_mut() {
                        search.editor.replace_selection("");
                    }
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                    && let Some(search) = self.search.as_mut()
                {
                    search.editor.paste(&text);
                }
            }
            "h" if control => {
                self.on_search_scope_prev(&SearchScopePrev, _window, cx);
                return;
            }
            "l" if control => {
                self.on_search_scope_next(&SearchScopeNext, _window, cx);
                return;
            }
            _ => {
                let command = TextModal::command_for_key(
                    key,
                    shift,
                    control,
                    event.keystroke.key_char.as_deref(),
                );
                let Some(command) = command else { return };
                if let Some(search) = self.search.as_mut() {
                    search.editor.apply_command(&command);
                }
            }
        }
        self.search_edited();
        cx.notify();
    }

    pub(in crate::ui) fn login_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let Some(login) = self.login.as_mut() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        let modified =
            control || event.keystroke.modifiers.alt || event.keystroke.modifiers.platform;
        let field = match login.field {
            0 => &mut login.server,
            1 => &mut login.username,
            _ => &mut login.password,
        };
        match key {
            "tab" => login.field = (login.field + 1) % 3,
            "backtab" => login.field = (login.field + 2) % 3,
            "enter" => {
                if login.server.text.trim().is_empty()
                    || login.username.text.trim().is_empty()
                    || login.password.text.is_empty()
                {
                    login.error =
                        Some("Server, username and password are all required.".to_string());
                } else {
                    login.busy = true;
                    login.error = None;
                    let server = login.server.text.trim().to_string();
                    let username = login.username.text.trim().to_string();
                    let password = login.password.text.clone();
                    self.bridge.login(server, username, password);
                }
            }
            "f2" => {
                if login.offline_available {
                    let server = login.server.text.trim().to_string();
                    let username = login.username.text.trim().to_string();
                    self.bridge.offline_login(server, username);
                }
            }
            "c" if control => {
                if let Some(text) = field.copy_selection() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                if let Some(text) = field.cut_selection() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "v" if control => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    field.paste(&text);
                }
            }
            _ => {
                let command = TextModal::command_for_key(
                    key,
                    shift,
                    control,
                    event.keystroke.key_char.as_deref(),
                );
                if let Some(command) = command {
                    field.apply_command(&command);
                } else if !modified {
                    return;
                }
            }
        }
        cx.notify();
    }
}
