//! Editable command-line text, shared by the `/`, `:` and floating-search
//! surfaces.

use std::ops::Range;

use gpui::{div, prelude::*, px, rgb, HighlightStyle, StyledText};

use crate::ui::palette;

/// Editable command-line text with a caret and selection.
///
/// Positions are byte offsets kept on `char` boundaries. `anchor` is where a
/// selection started; `cursor` is the moving end. When they are equal there is
/// no selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineEditor {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

impl LineEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        let mut editor = Self::new();
        editor.set_text(text);
        editor
    }

    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
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
        self.text.insert_str(self.cursor, text);
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

pub fn apply(editor: &mut LineEditor, command: &EditCommand) {
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

/// Paints the shared editor content: placeholder, highlighted selection, plain
/// text, or text plus caret. Chrome (bar/panel/border) stays at the call site.
pub fn editor_content(
    editor: &LineEditor,
    text_size: f32,
    caret_h: f32,
    placeholder: &str,
    caret: bool,
) -> gpui::AnyElement {
    let size = px(text_size);
    if editor.text.is_empty() {
        return div()
            .flex_1()
            .min_w_0()
            .text_size(size)
            .text_color(rgb(palette::on_surface_variant()))
            .child(placeholder.to_string())
            .into_any_element();
    }
    if let Some(range) = editor.selection() {
        let highlight = HighlightStyle {
            color: Some(rgb(palette::on_primary_container()).into()),
            background_color: Some(rgb(palette::primary_container()).into()),
            ..Default::default()
        };
        return div()
            .flex_1()
            .min_w_0()
            .text_size(size)
            .text_color(rgb(palette::on_surface()))
            .child(
                StyledText::new(editor.text.clone()).with_highlights([(range, highlight)]),
            )
            .into_any_element();
    }
    if !caret {
        return div()
            .flex_1()
            .min_w_0()
            .text_size(size)
            .text_color(rgb(palette::on_surface()))
            .child(editor.text.clone())
            .into_any_element();
    }
    let cursor = editor.cursor.min(editor.text.len());
    let (before, after) = editor.text.split_at(cursor);
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .text_size(size)
        .text_color(rgb(palette::on_surface()))
        .child(before.to_string())
        .child(
            div()
                .w(px(2.0))
                .h(px(caret_h))
                .bg(rgb(palette::primary())),
        )
        .child(after.to_string())
        .into_any_element()
}
