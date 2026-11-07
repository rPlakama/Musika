//! Neovim-style `:` command line model.
//!
//! Text/cursor/selection live in [`LineEditor`] (shared with `/`). This module
//! owns the open state, auto-suggestions, Tab completion cycling, denied-flash
//! timing and the INSERT/VISUAL label derivation.

use super::anim;
use super::ipc_command::{self, Suggestion};
use super::line_search::LineEditor;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletionCycle {
    start: usize,
    end: usize,
    values: Vec<String>,
    index: usize,
}

#[derive(Debug, Default)]
pub struct CommandLineState {
    open: bool,
    pub editor: LineEditor,
    pub suggestions: Vec<Suggestion>,
    pub selected: usize,
    cycle: Option<CompletionCycle>,
    denied_at: Option<f64>,
    pub denied_text: String,
}

impl CommandLineState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) {
        self.open = true;
        self.editor.clear();
        self.suggestions.clear();
        self.selected = 0;
        self.cycle = None;
        self.denied_at = None;
        self.denied_text.clear();
        self.refresh();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.editor.clear();
        self.suggestions.clear();
        self.cycle = None;
        self.denied_at = None;
        self.denied_text.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn query(&self) -> &str {
        &self.editor.text
    }

    pub fn mode_label(&self) -> &'static str {
        if self.editor.selection().is_some() {
            "VISUAL"
        } else {
            "INSERT"
        }
    }

    pub fn refresh(&mut self) {
        self.suggestions = ipc_command::suggestions(&self.editor.text, self.editor.cursor);
        self.selected = 0;
    }

    pub fn reset_completion(&mut self) {
        self.cycle = None;
    }

    pub fn move_left(&mut self, extend: bool) {
        self.editor.move_left(extend);
        self.reset_completion();
        self.refresh();
    }

    pub fn move_right(&mut self, extend: bool) {
        self.editor.move_right(extend);
        self.reset_completion();
        self.refresh();
    }

    pub fn home(&mut self, extend: bool) {
        self.editor.home(extend);
        self.reset_completion();
        self.refresh();
    }

    pub fn end(&mut self, extend: bool) {
        self.editor.end(extend);
        self.reset_completion();
        self.refresh();
    }

    pub fn select_all(&mut self) {
        self.editor.select_all();
        self.reset_completion();
        self.refresh();
    }

    pub fn insert(&mut self, text: &str) {
        self.editor.insert(text);
        self.reset_completion();
        self.refresh();
    }

    pub fn backspace(&mut self) {
        self.editor.backspace();
        self.reset_completion();
        self.refresh();
    }

    pub fn delete_forward(&mut self) {
        self.editor.delete_forward();
        self.reset_completion();
        self.refresh();
    }

    pub fn delete_word_back(&mut self) {
        self.editor.delete_word_back();
        self.reset_completion();
        self.refresh();
    }

    pub fn apply_edit(&mut self, command: &super::widgets::text_edit::EditCommand) {
        super::widgets::text_edit::apply(&mut self.editor, command);
        self.reset_completion();
        self.refresh();
    }

    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
        self.reset_completion();
        self.refresh();
    }

    /// Neovim-style Tab/Shift-Tab completion. The first press completes the
    /// token under the cursor; later presses cycle the discovered candidates.
    pub fn complete(&mut self, backward: bool) -> bool {
        if let Some(cycle) = self.cycle.clone() {
            if !cycle.values.is_empty() {
                let index = if backward {
                    (cycle.index + cycle.values.len() - 1) % cycle.values.len()
                } else {
                    (cycle.index + 1) % cycle.values.len()
                };
                let value = cycle.values[index].clone();
                self.editor
                    .text
                    .replace_range(cycle.start..cycle.end, &value);
                self.editor.cursor = cycle.start + value.len();
                self.editor.anchor = self.editor.cursor;
                self.cycle = Some(CompletionCycle {
                    start: cycle.start,
                    end: cycle.start + value.len(),
                    values: cycle.values,
                    index,
                });
                self.refresh();
                self.selected = index.min(self.suggestions.len().saturating_sub(1));
                return true;
            }
            self.cycle = None;
        }

        let token = self.token_under_cursor();
        let values: Vec<(String, String)> = self
            .suggestions
            .iter()
            .filter_map(|suggestion| {
                suggestion
                    .completion
                    .clone()
                    .map(|completion| (completion, suggestion.label.clone()))
            })
            .collect();
        if values.is_empty() {
            return false;
        }
        let index = if backward { values.len() - 1 } else { 0 };
        let completion = values[index].0.clone();
        let is_command = token.index == 0;
        let append_space = is_command && ipc_command::needs_argument(&completion);
        self.editor
            .text
            .replace_range(token.start..token.end, &completion);
        let cursor = token.start + completion.len() + usize::from(append_space);
        if append_space {
            self.editor.text.insert(token.start + completion.len(), ' ');
        }
        self.editor.cursor = cursor;
        self.editor.anchor = cursor;
        self.refresh();
        if append_space {
            self.cycle = None;
        } else {
            let values: Vec<String> = values.into_iter().map(|(value, _)| value).collect();
            self.cycle = Some(CompletionCycle {
                start: token.start,
                end: token.start + completion.len(),
                values,
                index,
            });
            self.selected = index.min(self.suggestions.len().saturating_sub(1));
        }
        true
    }

    fn token_under_cursor(&self) -> Token {
        let input = &self.editor.text;
        let mut cursor = self.editor.cursor.min(input.len());
        while cursor > 0 && !input.is_char_boundary(cursor) {
            cursor -= 1;
        }
        let mut start = cursor;
        while start > 0 {
            let prev = input[..start]
                .char_indices()
                .next_back()
                .map(|(index, _)| index);
            let Some(prev) = prev else { break };
            if input[prev..start].trim().is_empty() {
                break;
            }
            start = prev;
        }
        let mut end = cursor;
        while end < input.len() {
            let Some(ch) = input[end..].chars().next() else {
                break;
            };
            if ch.is_whitespace() {
                break;
            }
            end += ch.len_utf8();
        }
        Token {
            start,
            end: end.max(cursor),
            index: input[..start].split_whitespace().count(),
        }
    }

    pub fn deny(&mut self, now: f64, error: String) {
        self.denied_at = Some(now);
        self.denied_text = error;
    }

    pub fn clear_denied(&mut self) {
        self.denied_at = None;
        self.denied_text.clear();
    }

    pub fn denied_opacity(&self, now: f64) -> f32 {
        anim::denied_opacity(self.denied_at, now)
    }

    pub fn animation_target(&self) -> f32 {
        if self.open { 1.0 } else { 0.0 }
    }

    pub fn anim_curve(&self) -> anim::Curve {
        if self.open {
            anim::Curve::Decelerate
        } else {
            anim::Curve::Accelerate
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Token {
    start: usize,
    end: usize,
    index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_in_insert_and_derives_visual() {
        let mut state = CommandLineState::new();
        state.open();
        assert!(state.is_open());
        assert_eq!(state.mode_label(), "INSERT");
        state.set_text("sync");
        state.select_all();
        assert_eq!(state.mode_label(), "VISUAL");
    }

    #[test]
    fn tab_completes_and_cycles_arguments() {
        let mut state = CommandLineState::new();
        state.open();
        state.set_text("syn");
        assert_eq!(
            state
                .suggestions
                .first()
                .and_then(|s| s.completion.as_deref()),
            Some("sync")
        );
        assert!(state.complete(false));
        assert_eq!(state.query(), "sync ");
        assert_eq!(
            state
                .suggestions
                .first()
                .and_then(|s| s.completion.as_deref()),
            Some("all")
        );
        assert!(state.complete(false));
        assert_eq!(state.query(), "sync all");
        assert!(state.complete(false));
        assert_eq!(state.query(), "sync repair");
        assert!(state.complete(true));
        assert_eq!(state.query(), "sync all");
    }

    #[test]
    fn typing_replaces_visual_selection() {
        let mut state = CommandLineState::new();
        state.open();
        state.set_text("wipe");
        state.select_all();
        assert_eq!(state.mode_label(), "VISUAL");
        state.insert("sync");
        assert_eq!(state.query(), "sync");
        assert_eq!(state.mode_label(), "INSERT");
    }

    #[test]
    fn empty_input_suggests_the_whole_registry() {
        let mut state = CommandLineState::new();
        state.open();
        assert_eq!(state.suggestions.len(), ipc_command::COMMANDS.len());
        assert_eq!(state.mode_label(), "INSERT");
    }

    #[test]
    fn invalid_text_keeps_no_completion() {
        let mut state = CommandLineState::new();
        state.open();
        state.set_text("nonsense");
        assert!(!state.complete(false));
    }
}
