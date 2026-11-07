//! One editable-text engine for every text box.
//!
//! This mirrors [`super::scroll_modal::ScrollModal`]: surfaces keep a
//! `TextModal` instance instead of carrying their own text/caret/selection
//! plumbing. The modal paints the text box content and owns horizontal
//! caret-follow scrolling; the host only supplies its frame and width.

use std::ops::Range;

use gpui::{HighlightStyle, StyledText, TextRun, Window, div, font, prelude::*, px, rgb};

use crate::ui::palette;

const FONT_FAMILY: &str = "Overused Grotesk Roman";
const CARET_WIDTH: f32 = 2.0;

/// Editable command-line text with a caret and selection.
///
/// Positions are byte offsets kept on `char` boundaries. `anchor` is where a
/// selection started; `cursor` is the moving end. When they are equal there is
/// no selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextModal {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

fn single_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

impl TextModal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        let mut editor = Self::new();
        editor.set_text(text);
        editor
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = single_line(text);
        self.cursor = self.text.len();
        self.anchor = self.cursor;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = 0;
    }

    pub fn selection(&self) -> Option<Range<usize>> {
        if self.cursor == self.anchor {
            None
        } else {
            Some(self.anchor.min(self.cursor)..self.anchor.max(self.cursor))
        }
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|range| &self.text[range])
    }

    fn clamp_boundary(&self, mut index: usize) -> usize {
        index = index.min(self.text.len());
        while index > 0 && !self.text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    pub fn set_cursor(&mut self, index: usize, extend: bool) {
        self.cursor = self.clamp_boundary(index);
        if !extend {
            self.anchor = self.cursor;
        }
    }

    fn remove_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            return false;
        };
        self.text.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.anchor = range.start;
        true
    }

    pub fn insert(&mut self, text: &str) {
        self.remove_selection();
        let text = single_line(text);
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
        self.anchor = self.cursor;
    }

    pub fn replace_selection(&mut self, text: &str) {
        self.remove_selection();
        self.insert(text);
    }

    pub fn backspace(&mut self) {
        if self.remove_selection() {
            return;
        }
        if let Some((index, _)) = self.text[..self.cursor].char_indices().next_back() {
            self.text.replace_range(index..self.cursor, "");
            self.cursor = index;
            self.anchor = index;
        }
    }

    pub fn delete_forward(&mut self) {
        if self.remove_selection() {
            return;
        }
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.text
                .replace_range(self.cursor..self.cursor + ch.len_utf8(), "");
        }
    }

    pub fn move_left(&mut self, extend: bool) {
        let index = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.set_cursor(index, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        let index = self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |ch| self.cursor + ch.len_utf8());
        self.set_cursor(index, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.set_cursor(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.set_cursor(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    pub fn delete_word_back(&mut self) {
        if self.remove_selection() {
            return;
        }
        let mut start = self.cursor;
        while let Some((index, ch)) = self.text[..start].char_indices().next_back() {
            if ch.is_whitespace() {
                start = index;
            } else {
                break;
            }
        }
        while let Some((index, ch)) = self.text[..start].char_indices().next_back() {
            if ch.is_whitespace() {
                break;
            }
            start = index;
        }
        if start < self.cursor {
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.anchor = start;
        }
    }
}

/// One editable key action, independent of GPUI events and clipboard state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditCommand {
    MoveLeft { extend: bool },
    MoveRight { extend: bool },
    Home { extend: bool },
    End { extend: bool },
    Backspace,
    Delete,
    SelectAll,
    DeleteWordBack,
    Insert(String),
}

/// Maps the shared subset of text-editing keys. Clipboard keys and modal keys
/// deliberately return `None` and stay with their surface.
pub fn command_for_key(
    key: &str,
    shift: bool,
    control: bool,
    key_char: Option<&str>,
) -> Option<EditCommand> {
    match key {
        "left" => Some(EditCommand::MoveLeft { extend: shift }),
        "right" => Some(EditCommand::MoveRight { extend: shift }),
        "home" => Some(EditCommand::Home { extend: shift }),
        "end" => Some(EditCommand::End { extend: shift }),
        "backspace" => Some(EditCommand::Backspace),
        "delete" => Some(EditCommand::Delete),
        "a" if control => Some(EditCommand::SelectAll),
        "w" if control => Some(EditCommand::DeleteWordBack),
        _ if !control => {
            if let Some(text) = key_char
                && !text.chars().any(char::is_control)
            {
                Some(EditCommand::Insert(text.to_string()))
            } else if key == "space" {
                Some(EditCommand::Insert(" ".to_string()))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn apply(editor: &mut TextModal, command: &EditCommand) {
    match command {
        EditCommand::MoveLeft { extend } => editor.move_left(*extend),
        EditCommand::MoveRight { extend } => editor.move_right(*extend),
        EditCommand::Home { extend } => editor.home(*extend),
        EditCommand::End { extend } => editor.end(*extend),
        EditCommand::Backspace => editor.backspace(),
        EditCommand::Delete => editor.delete_forward(),
        EditCommand::SelectAll => editor.select_all(),
        EditCommand::DeleteWordBack => editor.delete_word_back(),
        EditCommand::Insert(text) => editor.insert(text),
    }
}

/// Painting options supplied by a host surface.
pub struct TextBoxStyle<'a> {
    pub placeholder: &'a str,
    pub text_size: f32,
    pub caret_h: f32,
    pub max_width: f32,
    pub caret: bool,
    pub mask: bool,
}

struct DisplayText {
    text: String,
    cursor: usize,
    selection: Option<Range<usize>>,
}

fn display_text(editor: &TextModal, mask: bool) -> DisplayText {
    let selection = editor.selection();
    if !mask {
        return DisplayText {
            text: editor.text.clone(),
            cursor: editor.cursor,
            selection,
        };
    }

    let mut text = String::with_capacity(editor.text.chars().count() * '•'.len_utf8());
    let mut cursor = editor.text.len();
    let mut anchor = editor.anchor;
    for (index, _) in editor.text.char_indices() {
        if index == editor.cursor {
            cursor = text.len();
        }
        if index == editor.anchor {
            anchor = text.len();
        }
        text.push('•');
    }
    if editor.cursor == editor.text.len() {
        cursor = text.len();
    }
    if editor.anchor == editor.text.len() {
        anchor = text.len();
    }
    let selection = if cursor == anchor {
        None
    } else {
        Some(anchor.min(cursor)..anchor.max(cursor))
    };
    DisplayText {
        text,
        cursor,
        selection,
    }
}

/// Horizontal viewport offset that keeps the caret visible.
pub fn auto_scroll_offset(
    text_width: f32,
    caret_x: f32,
    caret_width: f32,
    viewport_width: f32,
    margin: f32,
) -> f32 {
    let content_width = text_width + caret_width + margin;
    if viewport_width <= 0.0 || content_width <= viewport_width {
        return 0.0;
    }
    (caret_x + caret_width + margin - viewport_width).clamp(0.0, content_width - viewport_width)
}

impl TextModal {
    /// Paint this modal inside the host's chrome.
    pub fn render(&self, window: &Window, style: TextBoxStyle<'_>) -> gpui::AnyElement {
        if style.max_width <= 0.0 {
            return div().into_any_element();
        }
        let display = display_text(self, style.mask);
        let size = px(style.text_size);

        if display.text.is_empty() {
            return div()
                .w(px(style.max_width))
                .min_w(px(style.max_width))
                .overflow_hidden()
                .flex()
                .items_center()
                .text_size(size)
                .text_color(rgb(palette::on_surface_variant()))
                .child(
                    div()
                        .whitespace_nowrap()
                        .child(style.placeholder.to_string()),
                )
                .into_any_element();
        }

        let shaped = window.text_system().shape_line(
            display.text.clone().into(),
            size,
            &[TextRun {
                len: display.text.len(),
                font: font(FONT_FAMILY),
                color: rgb(palette::on_surface()).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let text_width = f32::from(shaped.width);
        let caret_x = f32::from(shaped.x_for_index(display.cursor.min(display.text.len())));
        let offset = if style.caret {
            auto_scroll_offset(
                text_width,
                caret_x,
                CARET_WIDTH,
                style.max_width,
                style.text_size * 0.25,
            )
        } else {
            0.0
        };

        let content: gpui::AnyElement = if let Some(range) = display.selection {
            let highlight = HighlightStyle {
                color: Some(rgb(palette::on_primary_container()).into()),
                background_color: Some(rgb(palette::primary_container()).into()),
                ..Default::default()
            };
            StyledText::new(display.text.clone())
                .with_highlights([(range, highlight)])
                .into_any_element()
        } else if style.caret {
            let cursor = display.cursor.min(display.text.len());
            let (before, after) = display.text.split_at(cursor);
            div()
                .flex()
                .items_center()
                .child(before.to_string())
                .child(
                    div()
                        .w(px(CARET_WIDTH))
                        .h(px(style.caret_h))
                        .bg(rgb(palette::primary())),
                )
                .child(after.to_string())
                .into_any_element()
        } else {
            display.text.clone().into_any_element()
        };

        let text_color = rgb(palette::on_surface());
        div()
            .w(px(style.max_width))
            .min_w(px(style.max_width))
            .overflow_hidden()
            .flex()
            .items_center()
            .text_size(size)
            .text_color(text_color)
            .child(
                div()
                    .ml(px(-offset))
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .child(content),
            )
            .into_any_element()
    }

    /// Shared key mapping for the modal-owned editing vocabulary.
    pub fn command_for_key(
        key: &str,
        shift: bool,
        control: bool,
        key_char: Option<&str>,
    ) -> Option<EditCommand> {
        command_for_key(key, shift, control, key_char)
    }

    pub fn apply_command(&mut self, command: &EditCommand) {
        apply(self, command);
    }

    pub fn copy_selection(&self) -> Option<String> {
        self.selected_text().map(str::to_string)
    }

    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.copy_selection()?;
        self.replace_selection("");
        Some(text)
    }

    pub fn paste(&mut self, text: &str) {
        self.insert(text);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_selection_and_word_delete_respect_char_boundaries() {
        let mut editor = TextModal::new();
        editor.insert("héllo wörld");
        assert_eq!(editor.cursor, editor.text.len());
        editor.delete_word_back();
        assert_eq!(editor.text, "héllo ");
        editor.select_all();
        assert_eq!(editor.selected_text(), Some("héllo "));
        editor.insert("x");
        assert_eq!(editor.text, "x");
        assert_eq!(editor.cursor, 1);

        editor.set_text("abc");
        editor.home(false);
        editor.move_right(true);
        assert_eq!(editor.selection(), Some(0..1));
        editor.insert("Z");
        assert_eq!(editor.text, "Zbc");
    }

    #[test]
    fn shared_command_maps_motion_and_deletion() {
        assert_eq!(
            command_for_key("left", true, false, None),
            Some(EditCommand::MoveLeft { extend: true })
        );
        assert_eq!(
            command_for_key("a", true, true, None),
            Some(EditCommand::SelectAll)
        );
        assert_eq!(command_for_key("c", false, true, None), None);
        assert_eq!(
            command_for_key("x", false, false, Some("x")),
            Some(EditCommand::Insert("x".to_string()))
        );

        let mut editor = TextModal::from_text("abc");
        apply(&mut editor, &EditCommand::Home { extend: false });
        apply(&mut editor, &EditCommand::MoveRight { extend: true });
        assert_eq!(editor.selection(), Some(0..1));
        apply(&mut editor, &EditCommand::Insert("Z".to_string()));
        assert_eq!(editor.text, "Zbc");
    }

    #[test]
    fn auto_scroll_keeps_caret_visible() {
        assert_eq!(auto_scroll_offset(50.0, 40.0, 2.0, 100.0, 4.0), 0.0);
        assert_eq!(auto_scroll_offset(200.0, 190.0, 2.0, 100.0, 4.0), 96.0);
        assert_eq!(auto_scroll_offset(200.0, 100.0, 2.0, 100.0, 4.0), 6.0);
        assert_eq!(auto_scroll_offset(200.0, 0.0, 2.0, 100.0, 4.0), 0.0);
        assert_eq!(auto_scroll_offset(100.0, 20.0, 2.0, 0.0, 4.0), 0.0);
        assert_eq!(auto_scroll_offset(100.0, 100.0, 2.0, 100.0, 4.0), 6.0);
    }

    #[test]
    fn pasted_text_stays_single_line() {
        let mut editor = TextModal::new();
        editor.insert("one\r\ntwo\tthree");
        assert_eq!(editor.text, "one two three");
        assert_eq!(editor.cursor, editor.text.len());
    }

    #[test]
    fn masked_display_maps_cursor_and_selection() {
        let mut editor = TextModal::from_text("héllo");
        editor.set_cursor("héllo".len(), false);
        let display = display_text(&editor, true);
        assert_eq!(display.text, "•••••");
        assert_eq!(display.cursor, display.text.len());

        editor.set_cursor("hé".len(), false);
        let display = display_text(&editor, true);
        assert_eq!(display.cursor, 2 * '•'.len_utf8());

        editor.anchor = 0;
        editor.cursor = "hé".len();
        let display = display_text(&editor, true);
        assert_eq!(display.selection, Some(0..2 * '•'.len_utf8()));
    }
}
