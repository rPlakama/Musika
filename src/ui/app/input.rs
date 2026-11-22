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
        // The Alt board is a surface above every overlay and keeps its raw
        // modifier+key path while a modal is open; modal contexts only replace
        // the `main` key map, so this must stay in front of them.
        if self.alt_held || self.alt.armed {
            cx.stop_propagation();
            self.on_alt_key(event, cx);
            return;
        }
        // U-24: an open settings/description modal owns the keyboard. Its key
        // context (set in `render.rs`) stops the `main` bindings from
        // dispatching underneath it, so the modal's own keys are re-driven
        // from the raw key stream here and everything else is swallowed.
        if self.settings_open {
            cx.stop_propagation();
            self.settings_key(event, _window, cx);
            return;
        }
        if self.description.is_some() {
            cx.stop_propagation();
            self.description_key(event, _window, cx);
            return;
        }
        if self.fullscreen && !self.lyrics_overlay && !(self.search.is_some() && self.search_open) {
            let key = event.keystroke.key.as_str();
            if key == "tab" || key == "backtab" {
                cx.stop_propagation();
                self.cycle_fullscreen_focus(key == "backtab");
                cx.notify();
                return;
            }
        }
        let search_typing =
            self.search_open && self.search.as_ref().is_some_and(|search| search.typing);
        if !search_typing
            && let Some(key) = batchable_playback_key(
                event.keystroke.key.as_str(),
                event.keystroke.modifiers.control,
                event.keystroke.modifiers.alt,
                event.keystroke.modifiers.platform,
            )
        {
            // Repeats fold into one hold: the bubble counts them and the single
            // absolute op is sent on release. `event.is_held` is not usable here
            // because X11 auto-repeat arrives as a bare key-down (`is_held` is
            // always false on that backend), so `KeyHold` times the repeats.
            self.hold_playback_key(key);
            cx.stop_propagation();
            cx.notify();
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
                "j" if control => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
                "k" if control => {
                    if let Some(search) = self.search.as_mut() {
                        search.move_cursor(-1);
                    }
                    self.follow_search(true);
                    cx.notify();
                }
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
                // VI-8a: normal mode yanks the current scope; typing keeps y
                // as ordinary text input.
                "y" => {
                    self.yank_search(shift);
                    cx.notify();
                }
                "enter" => {
                    if let Some(search) = self.search.as_mut() {
                        search.selection.clear();
                    }
                    self.activate_search_result(cx);
                }
                // One h/l-style scope step; Shift-Tab reverses. The uppercase
                // spellings cover the backends/IMEs that report the typed
                // character in `key` (Shift-L arrives as `"L"` there; on
                // X11/Wayland it is `"l"` plus the shift modifier).
                "tab" | "backtab" | "l" | "right" | "L" | "h" | "left" | "H" => {
                    match search_scope_step(key, shift) {
                        Some(1) => self.on_search_scope_next(&SearchScopeNext, _window, cx),
                        Some(-1) => self.on_search_scope_prev(&SearchScopePrev, _window, cx),
                        Some(_) | None => {}
                    }
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
                "j" | "down" => {
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
                "k" | "up" => {
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
                "pagedown" => {
                    self.search_move_page(1, state::SEARCH_VISIBLE_ROWS);
                    cx.notify();
                }
                "pageup" => {
                    self.search_move_page(-1, state::SEARCH_VISIBLE_ROWS);
                    cx.notify();
                }
                // Vim half-page motions: GPUI spells a bare Ctrl chord as the
                // letter plus the `control` modifier, so `"ctrl-d"` never
                // matched before.
                "d" if control => {
                    self.search_move_page(1, half_page_rows());
                    cx.notify();
                }
                "u" if control => {
                    self.search_move_page(-1, half_page_rows());
                    cx.notify();
                }
                // Vim `G`: jump to the last row of the scope. `Shift-G` is
                // `"g"`+shift on X11/Wayland and `"G"` on the IME path.
                "g" if shift => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] =
                            search.scope_count(search.scope).saturating_sub(1);
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
                "g" => {
                    if let Some(search) = self.search.as_mut() {
                        search.scope_cursors[search.scope] = 0;
                    }
                    self.follow_search(false);
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        match key {
            "j" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "k" if control => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(-1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
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
            "tab" | "backtab" => {
                match search_scope_step(key, shift) {
                    Some(1) => self.on_search_scope_next(&SearchScopeNext, _window, cx),
                    Some(-1) => self.on_search_scope_prev(&SearchScopePrev, _window, cx),
                    Some(_) | None => {}
                }
                return;
            }
            "up" => {
                if let Some(search) = self.search.as_mut() {
                    search.move_cursor(-1);
                }
                self.follow_search(true);
                cx.notify();
                return;
            }
            "down" => {
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
                self.search_move_page(-1, state::SEARCH_VISIBLE_ROWS);
                cx.notify();
                return;
            }
            "d" if control => {
                self.search_move_page(1, state::SEARCH_VISIBLE_ROWS);
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

    /// Keys of the isolated settings-modal context (U-24).
    ///
    /// The modal's own key context (set in `render.rs`) disables the `main`
    /// key map, so the handlers those bindings used to call are re-driven from
    /// the raw key stream here. Every other `main` chord is swallowed: it must
    /// not move the list, toggle playback or walk the top bar behind the panel.
    fn settings_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        // Alt is dispatched to the Alt board before this handler runs; Alt
        // without an armed board and the Super/Command chords belonged to the
        // `main` map, so they stay inert under the modal.
        if modifiers.alt || modifiers.platform {
            return;
        }
        if modifiers.control {
            if modifiers.shift {
                // Ctrl-Shift-* shortcuts of the main map (queue reorder, EQ,
                // normalization...) are not settings chords.
                return;
            }
            match key {
                "j" => self.on_move_down(&MoveDown, window, cx),
                "k" => self.on_move_up(&MoveUp, window, cx),
                // `Ctrl-,` opened the panel: keep the same chord closing it.
                "," | "comma" => self.on_settings_overlay(&SettingsOverlay, window, cx),
                _ => {}
            }
            return;
        }
        if modifiers.shift {
            // Shifted letters were main-mode bindings (Shift-J/K, Shift-Q
            // home, Shift-Enter fullscreen...): swallow them.
            return;
        }
        match key {
            "j" | "down" => self.on_move_down(&MoveDown, window, cx),
            "k" | "up" => self.on_move_up(&MoveUp, window, cx),
            "h" | "left" => {
                self.adjust_settings_row(-1);
                cx.notify();
            }
            "l" | "right" => {
                self.adjust_settings_row(1);
                cx.notify();
            }
            "enter" => self.on_enter(&Enter, window, cx),
            "escape" | "q" => self.on_back(&Back, window, cx),
            _ => {}
        }
    }

    /// Keys of the isolated description-reader context (U-24).
    ///
    /// Mirrors the `main` chords the reader used to receive (j/k, Ctrl-D/U,
    /// PageUp/PageDown, `g g`, `G`, Esc/q) and swallows the rest.
    fn description_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        let armed_g = DESCRIPTION_PENDING_G.with(std::cell::Cell::get);
        let (stroke, next_armed) = description_stroke(
            event.keystroke.key.as_str(),
            modifiers.control,
            modifiers.shift,
            modifiers.alt,
            modifiers.platform,
            armed_g,
        );
        DESCRIPTION_PENDING_G.with(|slot| slot.set(next_armed));
        match stroke {
            DescriptionStroke::ScrollDown => self.on_move_down(&MoveDown, window, cx),
            DescriptionStroke::ScrollUp => self.on_move_up(&MoveUp, window, cx),
            DescriptionStroke::PageDown => self.on_page_down(&PageDown, window, cx),
            DescriptionStroke::PageUp => self.on_page_up(&PageUp, window, cx),
            DescriptionStroke::Top => self.on_top(&Top, window, cx),
            DescriptionStroke::Bottom => self.on_bottom(&Bottom, window, cx),
            DescriptionStroke::Close => self.on_back(&Back, window, cx),
            DescriptionStroke::ArmedG | DescriptionStroke::Ignored => {}
        }
    }

    pub(in crate::ui) fn on_key_up(
        &mut self,
        event: &gpui::KeyUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = batched_playback_key(event.keystroke.key.as_str()) else {
            return;
        };
        let Some(hold) = self.playback_key_hold else {
            return;
        };
        // Either key of a family releases the whole fold: `,.` spammed together
        // is one accumulator, so the first release commits the net delta.
        if HoldKind::of(key) != Some(hold.kind) {
            return;
        }
        self.playback_key_hold = None;
        self.apply_playback_hold(hold);
        cx.stop_propagation();
        cx.notify();
    }

    /// Applies one held-key group as one relative op. Relative seeks/volume
    /// steps never read a stale local anchor: the core resolves them against
    /// its own playhead/volume, and the outbound queue sums pending
    /// `SeekRelative` deltas, so rapid alternating taps cannot drop steps while
    /// a round-trip is in flight.
    pub(in crate::ui) fn apply_playback_hold(&self, hold: KeyHold) {
        match hold.kind {
            HoldKind::Seek => self.player.seek_relative(hold.delta),
            HoldKind::Volume => {
                let steps = (hold.delta / HOLD_STEP).round() as i64;
                // The core clamps to 0..=100, so more than a full range of
                // steps in either direction cannot change the result.
                let steps = steps.clamp(-20, 20);
                let repeats = steps.unsigned_abs();
                if steps > 0 {
                    for _ in 0..repeats {
                        self.player.volume_up();
                    }
                } else {
                    for _ in 0..repeats {
                        self.player.volume_down();
                    }
                }
            }
        }
    }

    /// Drops a pending hold without applying it and clears its counter bubble.
    /// Runs when the window loses focus mid-hold, since the key-up then never
    /// arrives.
    pub(in crate::ui) fn cancel_playback_hold(&mut self) {
        self.playback_key_hold = None;
        self.hold_toast = None;
    }

    /// Moves the search cursor by `rows` rows in one step (negative = up) and
    /// snaps the viewport to the new cursor row. Used by PageUp/PageDown and
    /// the Ctrl-D/Ctrl-U half-page motions.
    pub(in crate::ui) fn search_move_page(&mut self, direction: i8, rows: usize) {
        if let Some(search) = self.search.as_mut() {
            for _ in 0..rows {
                search.move_cursor(i32::from(direction));
            }
        }
        self.follow_search(true);
    }

    /// Validates the login form and, when complete, sends the credentials.
    ///
    /// Shared by the keyboard Enter path and the panel's submit button (U-29
    /// mouse parity) so the busy guard and the field validation cannot drift.
    pub(in crate::ui) fn login_submit(&mut self, cx: &mut Context<Self>) {
        let Some(login) = self.login.as_mut() else {
            return;
        };
        // A request is already in flight; a second submit must not issue a
        // duplicate login.
        if login.busy {
            return;
        }
        if login.server.text.trim().is_empty()
            || login.username.text.trim().is_empty()
            || login.password.text.is_empty()
        {
            login.error = Some("Server, username and password are all required.".to_string());
            cx.notify();
            return;
        }
        login.busy = true;
        login.error = None;
        let server = login.server.text.trim().to_string();
        let username = login.username.text.trim().to_string();
        let password = login.password.text.clone();
        self.bridge.login(server, username, password);
        cx.notify();
    }

    pub(in crate::ui) fn login_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        // Enter submits from any field through the same path the submit button
        // uses; handling it before the field borrow keeps `login_submit`
        // callable here.
        if event.keystroke.key == "enter" {
            self.login_submit(cx);
            return;
        }
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
        // The password input is always painted masked; copying its selection
        // would put the plaintext on the clipboard, so both clipboard writes
        // are refused on that field.
        let masked = login.field == 2;
        match key {
            "escape" if login.mode == LoginMode::AddAccount => {
                self.login = None;
                cx.notify();
                return;
            }
            "tab" => login.field = (login.field + 1) % 3,
            "backtab" => login.field = (login.field + 2) % 3,
            "f2" => {
                if login.offline_available {
                    let server = login.server.text.trim().to_string();
                    let username = login.username.text.trim().to_string();
                    self.bridge.offline_login(server, username);
                }
            }
            "c" if control => {
                if masked {
                    return;
                }
                if let Some(text) = field.copy_selection() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            "x" if control => {
                if masked {
                    return;
                }
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

/// Playback key families must not steal modified shortcuts: `Ctrl-,` opens
/// settings and `Ctrl-.` is a future surface, while bare `,`/`.` seek.
fn batchable_playback_key(key: &str, control: bool, alt: bool, platform: bool) -> Option<char> {
    if control || alt || platform {
        return None;
    }
    batched_playback_key(key)
}

fn batched_playback_key(key: &str) -> Option<char> {
    match key {
        "," | "comma" => Some(','),
        "." | "period" => Some('.'),
        "-" | "minus" => Some('-'),
        "+" | "=" | "equal" => Some('+'),
        _ => None,
    }
}

/// Half-page row count for the Ctrl-D/Ctrl-U motions (at least one row, so a
/// list shorter than the visible window still moves).
fn half_page_rows() -> usize {
    (state::SEARCH_VISIBLE_ROWS / 2).max(1)
}

/// The search-scope step a key names: `h`/`left`/Shift-Tab move backward,
/// `l`/`right`/Tab forward. Shift-Tab may arrive as `backtab` or as `tab` with
/// the shift modifier depending on the platform; both reverse the cycle.
/// Uppercase letters are the IME-commit spelling of the shifted letter.
fn search_scope_step(key: &str, shift: bool) -> Option<i8> {
    match key {
        "backtab" | "h" | "left" | "H" => Some(-1),
        "tab" if shift => Some(-1),
        "tab" | "l" | "right" | "L" => Some(1),
        _ => None,
    }
}

thread_local! {
    /// First-`g` latch of the description reader's two-stroke `g g` motion.
    /// The reader's isolated key context cannot use the keymap's multi-key
    /// bindings, so the sequence is resolved in [`description_stroke`].
    static DESCRIPTION_PENDING_G: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One resolved key stroke of the description reader's isolated key context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptionStroke {
    /// One text line down (`j`, Down).
    ScrollDown,
    /// One text line up (`k`, Up).
    ScrollUp,
    /// One viewport down (Ctrl-D, PageDown).
    PageDown,
    /// One viewport up (Ctrl-U, PageUp).
    PageUp,
    /// Reader start (second stroke of `g g`).
    Top,
    /// Reader end (`G`).
    Bottom,
    /// Dismiss the reader (Esc, q).
    Close,
    /// First stroke of `g g`: armed, no motion yet.
    ArmedG,
    /// A `main`-context chord: swallow it while the reader owns the keyboard.
    Ignored,
}

/// Resolves one description-reader key event, returning the stroke and the new
/// `g g` armed state. Every non-`g` key disarms the sequence.
fn description_stroke(
    key: &str,
    control: bool,
    shift: bool,
    alt: bool,
    platform: bool,
    armed_g: bool,
) -> (DescriptionStroke, bool) {
    // Alt is dispatched to the Alt board before this resolver runs; the
    // remaining modifier chords belonged to the `main` map.
    if alt || platform {
        return (DescriptionStroke::Ignored, false);
    }
    if control {
        if shift {
            return (DescriptionStroke::Ignored, false);
        }
        let stroke = match key {
            "d" => DescriptionStroke::PageDown,
            "u" => DescriptionStroke::PageUp,
            "j" => DescriptionStroke::ScrollDown,
            "k" => DescriptionStroke::ScrollUp,
            _ => DescriptionStroke::Ignored,
        };
        return (stroke, false);
    }
    if shift {
        // `G` (arriving as `"G"` or as `"g"` plus shift) ends the reader;
        // the other shifted chords were main-mode bindings.
        let stroke = match key {
            "G" | "g" => DescriptionStroke::Bottom,
            _ => DescriptionStroke::Ignored,
        };
        return (stroke, false);
    }
    match key {
        "escape" | "q" => (DescriptionStroke::Close, false),
        "j" | "down" => (DescriptionStroke::ScrollDown, false),
        "k" | "up" => (DescriptionStroke::ScrollUp, false),
        "pagedown" => (DescriptionStroke::PageDown, false),
        "pageup" => (DescriptionStroke::PageUp, false),
        // IME backends report the shifted character without the modifier.
        "G" => (DescriptionStroke::Bottom, false),
        "g" if armed_g => (DescriptionStroke::Top, false),
        "g" => (DescriptionStroke::ArmedG, true),
        _ => (DescriptionStroke::Ignored, false),
    }
}

#[cfg(test)]
mod search_scope_tests {
    use super::{batchable_playback_key, search_scope_step};

    #[test]
    fn modified_playback_punctuation_is_not_swallowed() {
        assert_eq!(batchable_playback_key(",", true, false, false), None);
        assert_eq!(batchable_playback_key(".", false, true, false), None);
        assert_eq!(batchable_playback_key("-", false, false, true), None);
        assert_eq!(batchable_playback_key(",", false, false, false), Some(','));
    }

    /// U-16: `Shift-L` must move the search scope forward (it used to be
    /// routed through the Shift-Tab reversal), and both the printed-key and
    /// typed-character spellings are recognized.
    #[test]
    fn h_l_and_tab_move_the_scope_in_the_right_direction() {
        assert_eq!(search_scope_step("l", false), Some(1));
        assert_eq!(search_scope_step("l", true), Some(1));
        assert_eq!(search_scope_step("L", false), Some(1));
        assert_eq!(search_scope_step("right", true), Some(1));
        assert_eq!(search_scope_step("tab", false), Some(1));
        assert_eq!(search_scope_step("tab", true), Some(-1));
        assert_eq!(search_scope_step("h", false), Some(-1));
        assert_eq!(search_scope_step("H", false), Some(-1));
        assert_eq!(search_scope_step("backtab", false), Some(-1));
        assert_eq!(search_scope_step("j", true), None);
    }
}

#[cfg(test)]
mod isolated_modal_key_tests {
    use super::{DescriptionStroke, description_stroke};

    #[test]
    fn description_reader_keeps_its_main_bindings() {
        let plain = |key| description_stroke(key, false, false, false, false, false).0;
        assert_eq!(plain("j"), DescriptionStroke::ScrollDown);
        assert_eq!(plain("down"), DescriptionStroke::ScrollDown);
        assert_eq!(plain("k"), DescriptionStroke::ScrollUp);
        assert_eq!(plain("up"), DescriptionStroke::ScrollUp);
        assert_eq!(plain("pagedown"), DescriptionStroke::PageDown);
        assert_eq!(plain("pageup"), DescriptionStroke::PageUp);
        assert_eq!(plain("escape"), DescriptionStroke::Close);
        assert_eq!(plain("q"), DescriptionStroke::Close);
        assert_eq!(
            description_stroke("d", true, false, false, false, false).0,
            DescriptionStroke::PageDown
        );
        assert_eq!(
            description_stroke("u", true, false, false, false, false).0,
            DescriptionStroke::PageUp
        );
        assert_eq!(plain("G"), DescriptionStroke::Bottom);
        assert_eq!(
            description_stroke("g", false, true, false, false, false).0,
            DescriptionStroke::Bottom
        );
    }

    #[test]
    fn description_reader_swallows_main_mode_chords() {
        let stroke =
            |key, control, shift| description_stroke(key, control, shift, false, false, false).0;
        // Keys the `main` map would have acted on underneath the reader.
        assert_eq!(stroke("space", false, false), DescriptionStroke::Ignored);
        assert_eq!(stroke("m", false, false), DescriptionStroke::Ignored);
        assert_eq!(stroke("tab", false, false), DescriptionStroke::Ignored);
        assert_eq!(stroke("q", false, true), DescriptionStroke::Ignored);
        assert_eq!(stroke("enter", false, false), DescriptionStroke::Ignored);
        assert_eq!(stroke("f", true, false), DescriptionStroke::Ignored);
        assert_eq!(stroke("j", true, true), DescriptionStroke::Ignored);
        assert_eq!(
            description_stroke("j", false, false, false, true, false).0,
            DescriptionStroke::Ignored
        );
    }

    #[test]
    fn description_reader_gg_needs_two_strokes_and_other_keys_disarm_it() {
        let (first, armed) = description_stroke("g", false, false, false, false, false);
        assert_eq!(first, DescriptionStroke::ArmedG);
        assert!(armed);
        let (second, armed) = description_stroke("g", false, false, false, false, true);
        assert_eq!(second, DescriptionStroke::Top);
        assert!(!armed);
        // A different stroke after the first `g` drops the sequence.
        let (other, armed) = description_stroke("j", false, false, false, false, true);
        assert_eq!(other, DescriptionStroke::ScrollDown);
        assert!(!armed);
        // `G` after an armed `g` is the end jump, not a second `g`.
        assert_eq!(
            description_stroke("G", false, false, false, false, true),
            (DescriptionStroke::Bottom, false)
        );
    }
}
