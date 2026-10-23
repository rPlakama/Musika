// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Keyboard binding engine (Neovim-flavoured).
//!
//! Bindings are sequences of tokens separated by spaces (for example
//! `"space f"` or `"escape escape"`). A [`Keymap`] is fed one token at a time
//! and reports whether a binding fired, a longer sequence is still being typed,
//! or nothing matched.
//!
//! [`token_from_qt`] turns a Qt key event (`key` + `modifiers`) into a token,
//! so the UI only has to forward raw values.

/// A resolved user intention. Behaviour is wired to these as the movementation
/// lands; for now the engine only classifies input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    MoveLeft,
    MoveUp,
    MoveDown,
    MoveRight,
    Top,
    Bottom,
    PageUp,
    PageDown,
    ExtendUp,
    ExtendDown,
    ExtendLeft,
    ExtendRight,
    Search,
    SearchAction,
    VisualSelect,
    VisualAll,
    Enter,
    Back,
    ToggleFullscreen,
    PlayPause,
    Mute,
    VolumeUp,
    VolumeDown,
    OrderPrev,
    OrderNext,
    ScopePrev,
    ScopeNext,
    LayoutToggle,
    InsertMode,
    NextTrack,
    PrevTrack,
    SeekBack,
    SeekForward,
    Refresh,
    PaneCycle,
    PaneCycleBack,
    Yank,
    Paste,
    QueueToggle,
    QueueUp,
    QueueDown,
    QueueRemove,
    PrevPlace,
    NextPlace,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Self::MoveLeft => "move_left",
            Self::MoveUp => "move_up",
            Self::MoveDown => "move_down",
            Self::MoveRight => "move_right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::PageUp => "page_up",
            Self::PageDown => "page_down",
            Self::ExtendUp => "extend_up",
            Self::ExtendDown => "extend_down",
            Self::ExtendLeft => "extend_left",
            Self::ExtendRight => "extend_right",
            Self::Search => "search",
            Self::SearchAction => "search_action",
            Self::VisualSelect => "visual",
            Self::VisualAll => "visual_all",
            Self::Enter => "enter",
            Self::Back => "back",
            Self::ToggleFullscreen => "toggle_fullscreen",
            Self::PlayPause => "play_pause",
            Self::Mute => "mute",
            Self::VolumeUp => "volume_up",
            Self::VolumeDown => "volume_down",
            Self::OrderPrev => "order_prev",
            Self::OrderNext => "order_next",
            Self::ScopePrev => "scope_prev",
            Self::ScopeNext => "scope_next",
            Self::LayoutToggle => "layout_toggle",
            Self::InsertMode => "insert_mode",
            Self::NextTrack => "next_track",
            Self::PrevTrack => "prev_track",
            Self::SeekBack => "seek_back",
            Self::SeekForward => "seek_forward",
            Self::Refresh => "refresh",
            Self::PaneCycle => "pane_cycle",
            Self::PaneCycleBack => "pane_cycle_back",
            Self::Yank => "yank",
            Self::Paste => "paste",
            Self::QueueToggle => "queue_toggle",
            Self::QueueUp => "queue_up",
            Self::QueueDown => "queue_down",
            Self::QueueRemove => "queue_remove",
            Self::PrevPlace => "prev_place",
            Self::NextPlace => "next_place",
        }
    }
}

/// Result of feeding a token to a [`Keymap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A complete binding fired.
    Action(Action),
    /// The tokens so far are the start of at least one binding.
    Pending,
    /// No binding matches.
    None,
}

/// The default binding table.
/// The default binding table (VI-10/VI-16 semantics):
///
/// - `j`/`k` keep the list cursor (and scrolling); `n`/`p` and `Ctrl-N`/
///   `Ctrl-P` skip tracks wherever a list owns focus (VI-10).
/// - `Shift-J`/`Shift-K` move the selected queue track down/up (bare `Alt`
///   opens the quick-action menu, ARC-43; its chords stay out of the keymap).
/// - `i` starts typing: it opens the floating search in INSERT mode.
/// - `Enter` is context-confirm: open/play a row, jump in the queue, go back
///   on the album toolbar, sync in the status bar, commit in visual mode.
/// - `Esc`, `q` and `Ctrl-[` are the same Back action; Esc additionally steps
///   out of typing/insert before leaving (the invariant cancel key).
/// - `[b`/`]b` walk the session's previous/next place travel log (ARC-45).
/// - `Space` is play/pause globally; `Shift-Enter` toggles fullscreen.
/// - `,`/`.` seek back/forward 5 s globally (the transport is decoration:
///   it never owns a focus region, so seek cannot live inside it).
/// - `r` refreshes the library (the same action as the sync button).
fn defaults() -> Vec<(&'static str, Action)> {
    vec![
        ("ctrl-n", Action::NextTrack),
        ("ctrl-p", Action::PrevTrack),
        ("space", Action::PlayPause),
        ("m", Action::Mute),
        (",", Action::SeekBack),
        (".", Action::SeekForward),
        ("r", Action::Refresh),
        ("-", Action::VolumeDown),
        ("=", Action::VolumeUp),
        ("shift-=", Action::VolumeUp),
        ("+", Action::VolumeUp),
        ("shift-h", Action::OrderPrev),
        ("shift-l", Action::OrderNext),
        ("shift-c", Action::LayoutToggle),
        ("shift-space f", Action::Search),
        ("shift-space shift-f", Action::Search),
        ("/", Action::SearchAction),
        ("h", Action::MoveLeft),
        ("j", Action::MoveDown),
        ("k", Action::MoveUp),
        ("l", Action::MoveRight),
        ("g g", Action::Top),
        ("shift-g", Action::Bottom),
        // VI/Neovim half-page motions.
        ("ctrl-d", Action::PageDown),
        ("ctrl-u", Action::PageUp),
        ("pagedown", Action::PageDown),
        ("pageup", Action::PageUp),
        // VI-4/ARC-UI-7.9: Shift+Arrow extends the visual range.
        ("shift-up", Action::ExtendUp),
        ("shift-down", Action::ExtendDown),
        ("shift-left", Action::ExtendLeft),
        ("shift-right", Action::ExtendRight),
        ("v", Action::VisualSelect),
        ("shift-v", Action::VisualAll),
        ("enter", Action::Enter),
        ("shift-enter", Action::ToggleFullscreen),
        ("i", Action::InsertMode),
        ("n", Action::NextTrack),
        ("p", Action::PrevTrack),
        ("tab", Action::PaneCycle),
        ("shift-tab", Action::PaneCycleBack),
        ("y", Action::Yank),
        ("shift-p", Action::Paste),
        ("ctrl-v", Action::Paste),
        ("escape", Action::Back),
        ("ctrl-[", Action::Back),
        ("q", Action::Back),
        ("ctrl-q", Action::QueueToggle),
        ("shift-k", Action::ScopePrev),
        ("shift-j", Action::ScopeNext),
        ("ctrl-shift-k", Action::QueueUp),
        ("ctrl-shift-j", Action::QueueDown),
        ("d d", Action::QueueRemove),
        // ARC-45: Vim's previous/next buffer motions over the travel log.
        ("[ b", Action::PrevPlace),
        ("] b", Action::NextPlace),
    ]
}

/// VI-7: the count register's ceiling (a held digit cannot overflow it).
const MAX_COUNT: u32 = 999_999;

/// VI-7: a token that is one bare digit, as its value.
fn digit_value(token: &str) -> Option<u32> {
    let bytes = token.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_digit() {
        Some((bytes[0] - b'0') as u32)
    } else {
        None
    }
}

/// A stateful, sequence-aware keymap.
pub struct Keymap {
    bindings: Vec<(&'static str, Action)>,
    pending: Vec<String>,
    /// VI-7: the count being typed (`3j`, `10l`, `5G`). It rides the next
    /// resolved action and clears with it.
    count: u32,
    /// The count attached to the last fired action, read once by the
    /// consumer (`take_count`).
    fired_count: u32,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

impl Keymap {
    pub fn new() -> Self {
        Self {
            bindings: defaults(),
            pending: Vec::new(),
            count: 0,
            fired_count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.count = 0;
        self.fired_count = 0;
    }

    /// VI-7: the count mid-typing (the chip's number; 0 = none).
    pub fn count(&self) -> u32 {
        self.count
    }

    /// VI-7: the count the last fired action carries; read once (a plain
    /// action reports 0).
    pub fn take_count(&mut self) -> u32 {
        std::mem::take(&mut self.fired_count)
    }

    /// Captures the typed count onto the firing action.
    fn fire(&mut self, action: Action) -> Outcome {
        self.fired_count = std::mem::take(&mut self.count);
        Outcome::Action(action)
    }

    #[allow(dead_code)]
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Feeds one token; see [`Outcome`].
    ///
    /// If the tokens so far are also the exact start of a longer binding (e.g.
    /// `g` for `g g`), the match is deferred to [`Self::feed_timeout`] rather
    /// than firing immediately.
    pub fn feed(&mut self, token: &str) -> Outcome {
        // VI-7: bare digits accumulate the count and never join the pending
        // sequence. A lone leading zero is not a count here (there is no
        // start-of-line motion to mean).
        if let Some(digit) = digit_value(token) {
            if self.count == 0 && digit == 0 && self.pending.is_empty() {
                return Outcome::None;
            }
            self.count = (self.count.saturating_mul(10).saturating_add(digit)).min(MAX_COUNT);
            return Outcome::Pending;
        }

        self.pending.push(token.to_string());
        let sequence = self.pending.join(" ");

        if self.has_prefix(&sequence) {
            return Outcome::Pending;
        }
        if let Some(action) = self.exact(&sequence) {
            self.pending.clear();
            return self.fire(action);
        }

        // The accumulated tokens dead-end: drop them and retry this token on
        // its own (covers, e.g., a stray token before a valid sequence).
        self.pending.clear();
        if self.has_prefix(token) {
            self.pending.push(token.to_string());
            return Outcome::Pending;
        }
        if let Some(action) = self.exact(token) {
            return self.fire(action);
        }
        self.count = 0;
        Outcome::None
    }

    /// Resolves a deferred sequence once the inter-key timeout elapses.
    pub fn feed_timeout(&mut self) -> Outcome {
        if self.pending.is_empty() {
            // A lone count times out silently: the chip clears, no motion.
            self.count = 0;
            return Outcome::None;
        }
        let sequence = self.pending.join(" ");
        self.pending.clear();
        match self.exact(&sequence) {
            Some(action) => self.fire(action),
            None => {
                self.count = 0;
                Outcome::None
            }
        }
    }

    fn exact(&self, sequence: &str) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(keys, _)| *keys == sequence)
            .map(|(_, action)| *action)
    }

    fn has_prefix(&self, sequence: &str) -> bool {
        let prefix = format!("{sequence} ");
        self.bindings
            .iter()
            .any(|(keys, _)| keys.starts_with(&prefix))
    }
}

// Qt key / modifier constants (qnamespace.h), kept here so the module has no
// Qt dependency of its own.
const QT_KEY_SPACE: i32 = 0x20;
const QT_KEY_ESCAPE: i32 = 0x0100_0000;
const QT_KEY_TAB: i32 = 0x0100_0001;
const QT_KEY_BACKTAB: i32 = 0x0100_0002;
const QT_KEY_BACKSPACE: i32 = 0x0100_0003;
const QT_KEY_ENTER: i32 = 0x0100_0004;
const QT_KEY_HOME: i32 = 0x0100_0010;
const QT_KEY_END: i32 = 0x0100_0011;
const QT_KEY_LEFT: i32 = 0x0100_0012;
const QT_KEY_UP: i32 = 0x0100_0013;
const QT_KEY_RIGHT: i32 = 0x0100_0014;
const QT_KEY_DOWN: i32 = 0x0100_0015;
const QT_KEY_PAGEUP: i32 = 0x0100_0016;
const QT_KEY_PAGEDOWN: i32 = 0x0100_0017;

const QT_SHIFT: i32 = 0x0200_0000;
const QT_CTRL: i32 = 0x0400_0000;
const QT_ALT: i32 = 0x0800_0000;

/// Converts a Qt key event into a canonical token, or `None` for keys we do
/// not bind.
pub fn token_from_qt(key: i32, modifiers: i32) -> Option<String> {
    // Qt hands Shift+Tab as Backtab (0x01000002), not Tab+shiftmodifier.
    // Remember it so both shapes canonicalise to "shift-tab".
    let mut backtab = false;
    let base = match key {
        QT_KEY_SPACE => "space".to_string(),
        QT_KEY_ESCAPE => "escape".to_string(),
        QT_KEY_TAB => "tab".to_string(),
        QT_KEY_BACKTAB => {
            backtab = true;
            "tab".to_string()
        }
        QT_KEY_BACKSPACE => "backspace".to_string(),
        QT_KEY_ENTER | 0x0100_0005 => "enter".to_string(), // Return or Enter
        QT_KEY_HOME => "home".to_string(),
        QT_KEY_END => "end".to_string(),
        QT_KEY_LEFT => "left".to_string(),
        QT_KEY_UP => "up".to_string(),
        QT_KEY_RIGHT => "right".to_string(),
        QT_KEY_DOWN => "down".to_string(),
        QT_KEY_PAGEUP => "pageup".to_string(),
        QT_KEY_PAGEDOWN => "pagedown".to_string(),
        0x41..=0x5A => ((key as u8 + 32) as char).to_string(), // A-Z
        0x61..=0x7A => (key as u8 as char).to_string(),        // a-z
        0x30..=0x39 => (key as u8 as char).to_string(),        // 0-9
        0x2C => ",".to_string(),                               // comma
        0x2D => "-".to_string(),                               // minus / keypad minus
        0x2E => ".".to_string(),                               // period
        0x2F => "/".to_string(),                               // slash (ARC-UI-2)
        0x2B => "+".to_string(),                               // keypad plus
        0x3D => "=".to_string(),                               // '+' on the main row (shifted)
        0x5B => "[".to_string(),                               // Ctrl-[ == Escape, `[b` history
        0x5D => "]".to_string(),                               // `]b` history (ARC-45)
        _ => return None,
    };

    let token = if modifiers & QT_CTRL != 0 {
        if modifiers & QT_SHIFT != 0 || backtab {
            format!("ctrl-shift-{base}")
        } else {
            format!("ctrl-{base}")
        }
    } else if modifiers & QT_ALT != 0 {
        format!("alt-{base}")
    } else if modifiers & QT_SHIFT != 0 || backtab {
        format!("shift-{base}")
    } else {
        base
    };
    Some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_token_binding() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-n"), Outcome::Action(Action::NextTrack));
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
    }

    #[test]
    fn sequences_need_all_tokens() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        assert_eq!(keymap.feed("g"), Outcome::Action(Action::Top));

        assert_eq!(keymap.feed("shift-space"), Outcome::Pending);
        assert_eq!(keymap.feed("f"), Outcome::Action(Action::Search));
    }

    #[test]
    fn dead_end_resets_and_retries() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        // Second token does not continue `g g`; the single `j` still fires.
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
        assert!(!keymap.is_pending());
    }

    #[test]
    fn unknown_key_is_none() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("f13"), Outcome::None);
    }

    #[test]
    fn qt_tokens() {
        assert_eq!(token_from_qt(0x4A, 0).as_deref(), Some("j")); // Key_J
        assert_eq!(token_from_qt(0x4E, QT_CTRL).as_deref(), Some("ctrl-n"));
        assert_eq!(token_from_qt(0x57, QT_CTRL).as_deref(), Some("ctrl-w"));
        assert_eq!(token_from_qt(0x77, QT_CTRL).as_deref(), Some("ctrl-w"));
        assert_eq!(token_from_qt(0x47, QT_SHIFT).as_deref(), Some("shift-g"));
        assert_eq!(token_from_qt(QT_KEY_SPACE, 0).as_deref(), Some("space"));
        assert_eq!(token_from_qt(QT_KEY_ESCAPE, 0).as_deref(), Some("escape"));
        assert_eq!(token_from_qt(QT_KEY_DOWN, 0).as_deref(), Some("down"));
        assert_eq!(
            token_from_qt(QT_KEY_ENTER, QT_SHIFT).as_deref(),
            Some("shift-enter")
        );
        assert_eq!(token_from_qt(0x0100_0009, 0), None); // unbound special key
        assert_eq!(token_from_qt(0x2F, 0).as_deref(), Some("/")); // Key_Slash
        assert_eq!(token_from_qt(0x5B, 0).as_deref(), Some("[")); // Key_BracketLeft
        assert_eq!(token_from_qt(0x5D, 0).as_deref(), Some("]")); // Key_BracketRight
        assert_eq!(token_from_qt(0x3F, 0), None); // `?` stays unbound until M4
    }

    #[test]
    fn place_history_bindings() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("["), Outcome::Pending);
        assert_eq!(keymap.feed("b"), Outcome::Action(Action::PrevPlace));
        assert_eq!(keymap.feed("]"), Outcome::Pending);
        assert_eq!(keymap.feed("b"), Outcome::Action(Action::NextPlace));

        // A lone prefix times out silently; Ctrl-[ keeps hierarchical Back.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("["), Outcome::Pending);
        assert_eq!(keymap.feed_timeout(), Outcome::None);
        assert_eq!(keymap.feed("ctrl-["), Outcome::Action(Action::Back));
    }

    #[test]
    fn slash_opens_the_surface_search() {
        let mut keymap = Keymap::new();
        assert_eq!(
            keymap.feed("/"),
            Outcome::Action(Action::SearchAction)
        );
    }

    #[test]
    fn shift_enter_toggles_fullscreen() {
        let mut keymap = Keymap::new();
        assert_eq!(
            keymap.feed("shift-enter"),
            Outcome::Action(Action::ToggleFullscreen)
        );
    }

    #[test]
    fn space_plays_and_shift_space_f_searches() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("space"), Outcome::Action(Action::PlayPause));

        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-space"), Outcome::Pending);
        assert_eq!(keymap.feed("f"), Outcome::Action(Action::Search));

        // Shift held through to F is the natural way to type Shift+Space+F.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-space"), Outcome::Pending);
        assert_eq!(keymap.feed("shift-f"), Outcome::Action(Action::Search));
    }

    #[test]
    fn escape_and_insert_mode() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("escape"), Outcome::Action(Action::Back));
        assert_eq!(keymap.feed("i"), Outcome::Action(Action::InsertMode));
    }

    #[test]
    fn ctrl_bracket_is_back_and_cancels_pending() {
        assert_eq!(token_from_qt(0x5B, QT_CTRL).as_deref(), Some("ctrl-["));

        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-["), Outcome::Action(Action::Back));

        // A half-typed sequence is cleared by the escape alias.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        assert_eq!(keymap.feed("escape"), Outcome::Action(Action::Back));
        assert!(!keymap.is_pending());
    }

    #[test]
    fn visual_selection_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("v"), Outcome::Action(Action::VisualSelect));
        assert_eq!(keymap.feed("shift-v"), Outcome::Action(Action::VisualAll));
        assert_eq!(token_from_qt(0x56, QT_SHIFT).as_deref(), Some("shift-v"));
        // Ctrl-W is retired: `/` owns cross-region reach (ARC-UI-2), so the
        // token stays a plain no-binding now.
        assert_eq!(keymap.feed("ctrl-w"), Outcome::None);
    }

    #[test]
    fn queue_bindings() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-q"), Outcome::Action(Action::QueueToggle));
        assert_eq!(keymap.feed("ctrl-shift-j"), Outcome::Action(Action::QueueDown));
        assert_eq!(keymap.feed("ctrl-shift-k"), Outcome::Action(Action::QueueUp));

        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("d"), Outcome::Pending);
        assert_eq!(keymap.feed("d"), Outcome::Action(Action::QueueRemove));
    }

    #[test]
    fn backtab_canonicalises_to_shift_tab() {
        assert_eq!(token_from_qt(0x0100_0002, 0).as_deref(), Some("shift-tab"));
        assert_eq!(token_from_qt(0x0100_0001, 0).as_deref(), Some("tab"));
        assert_eq!(
            token_from_qt(0x0100_0001, QT_SHIFT).as_deref(),
            Some("shift-tab")
        );
    }

    #[test]
    fn transport_and_pane_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("n"), Outcome::Action(Action::NextTrack));
        assert_eq!(keymap.feed("p"), Outcome::Action(Action::PrevTrack));
        assert_eq!(keymap.feed("tab"), Outcome::Action(Action::PaneCycle));
        assert_eq!(
            keymap.feed("shift-tab"),
            Outcome::Action(Action::PaneCycleBack)
        );
    }

    #[test]
    fn seek_keys_are_global_bindings() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed(","), Outcome::Action(Action::SeekBack));
        assert_eq!(keymap.feed("."), Outcome::Action(Action::SeekForward));
        assert_eq!(token_from_qt(0x2C, 0).as_deref(), Some(","));
        assert_eq!(token_from_qt(0x2E, 0).as_deref(), Some("."));
    }

    #[test]
    fn refresh_key_syncs_the_library() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("r"), Outcome::Action(Action::Refresh));
        assert_eq!(token_from_qt(0x52, 0).as_deref(), Some("r"));
    }

    #[test]
    fn clipboard_verbs() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("y"), Outcome::Action(Action::Yank));
        assert_eq!(keymap.feed("shift-p"), Outcome::Action(Action::Paste));
        assert_eq!(keymap.feed("ctrl-v"), Outcome::Action(Action::Paste));
        assert_eq!(
            token_from_qt(0x56, QT_CTRL).as_deref(),
            Some("ctrl-v"),
            "system paste (Ctrl-V) must map to the clipboard action"
        );
    }

    #[test]
    fn volume_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("m"), Outcome::Action(Action::Mute));
        assert_eq!(keymap.feed("-"), Outcome::Action(Action::VolumeDown));
        assert_eq!(keymap.feed("="), Outcome::Action(Action::VolumeUp));
        assert_eq!(keymap.feed("shift-="), Outcome::Action(Action::VolumeUp));
        assert_eq!(keymap.feed("+"), Outcome::Action(Action::VolumeUp));

        assert_eq!(token_from_qt(0x4A, QT_CTRL | QT_SHIFT).as_deref(), Some("ctrl-shift-j"));
        assert_eq!(token_from_qt(0x2D, 0).as_deref(), Some("-"));
        assert_eq!(token_from_qt(0x3D, QT_SHIFT).as_deref(), Some("shift-="));
        assert_eq!(token_from_qt(0x2B, 0).as_deref(), Some("+"));
    }

    #[test]
    fn queue_reorder_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-shift-j"), Outcome::Action(Action::QueueDown));
        assert_eq!(keymap.feed("ctrl-shift-k"), Outcome::Action(Action::QueueUp));
        assert_eq!(keymap.feed("shift-j"), Outcome::Action(Action::ScopeNext));
        assert_eq!(keymap.feed("shift-k"), Outcome::Action(Action::ScopePrev));
        // ARC-43: Alt chords belong to the quick-action overlay; the keymap
        // must not claim them.
        assert_eq!(keymap.feed("alt-j"), Outcome::None);
        assert_eq!(keymap.feed("alt-k"), Outcome::None);
    }

    #[test]
    fn album_order_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-l"), Outcome::Action(Action::OrderNext));
        assert_eq!(keymap.feed("shift-h"), Outcome::Action(Action::OrderPrev));
    }

    #[test]
    fn layout_toggle_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-c"), Outcome::Action(Action::LayoutToggle));
        assert_eq!(token_from_qt(0x43, QT_SHIFT).as_deref(), Some("shift-c"));
    }

    #[test]
    fn vi7_counts_accumulate_and_ride_the_action() {
        // `3 j`.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.count(), 3);
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
        assert_eq!(keymap.take_count(), 3);
        assert_eq!(keymap.count(), 0, "the count clears with the action");
        assert_eq!(keymap.take_count(), 0, "read once");

        // `10 l` — multi-digit.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("1"), Outcome::Pending);
        assert_eq!(keymap.feed("0"), Outcome::Pending);
        assert_eq!(keymap.count(), 10);
        assert_eq!(keymap.feed("l"), Outcome::Action(Action::MoveRight));
        assert_eq!(keymap.take_count(), 10);

        // `5 G` — the count rides a shift binding too.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("5"), Outcome::Pending);
        assert_eq!(keymap.feed("shift-g"), Outcome::Action(Action::Bottom));
        assert_eq!(keymap.take_count(), 5);

        // A lone leading zero has no motion to mean and starts nothing.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("0"), Outcome::None);
        assert_eq!(keymap.count(), 0);

        // A plain action reports no count.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
        assert_eq!(keymap.take_count(), 0);
    }

    #[test]
    fn vi7_counts_clear_on_timeout_and_reset() {
        // A lone count times out silently.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.feed_timeout(), Outcome::None);
        assert_eq!(keymap.count(), 0);

        // Esc: the count dies with the sequence it was waiting on.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.feed("escape"), Outcome::Action(Action::Back));
        assert_eq!(keymap.take_count(), 3, "the count rides the fired action");
        assert_eq!(keymap.count(), 0);

        // reset() drops everything (context switches).
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("2"), Outcome::Pending);
        keymap.reset();
        assert_eq!(keymap.count(), 0);
        assert_eq!(keymap.take_count(), 0);
    }

    #[test]
    fn vi7_digits_do_not_join_sequences() {
        // `3 g g` works as count + the gg sequence.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        assert_eq!(keymap.feed("g"), Outcome::Action(Action::Top));
        assert_eq!(keymap.take_count(), 3);

        // A shifted digit is a symbol, not a count (`#` etc. stay unbound).
        assert_eq!(token_from_qt(0x31, QT_SHIFT).as_deref(), Some("shift-1"));
    }

    #[test]
    fn page_and_extend_motions() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-d"), Outcome::Action(Action::PageDown));
        assert_eq!(keymap.feed("ctrl-u"), Outcome::Action(Action::PageUp));
        assert_eq!(
            keymap.feed("shift-down"),
            Outcome::Action(Action::ExtendDown)
        );
        assert_eq!(keymap.feed("shift-up"), Outcome::Action(Action::ExtendUp));
        assert_eq!(
            keymap.feed("shift-left"),
            Outcome::Action(Action::ExtendLeft)
        );
        assert_eq!(
            keymap.feed("shift-right"),
            Outcome::Action(Action::ExtendRight)
        );
        assert_eq!(
            token_from_qt(QT_KEY_DOWN, QT_SHIFT).as_deref(),
            Some("shift-down")
        );
    }
}
