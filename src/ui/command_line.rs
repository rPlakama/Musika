//! Neovim-style `:` command line model.
//!
//! Text/cursor/selection live in [`TextModal`] (shared with `/`). This module
//! owns the open state, auto-suggestions, Tab completion cycling, denied-flash
//! timing and the INSERT/VISUAL label derivation.

use super::anim;
use super::ipc_command::{self, Suggestion};
use super::text_modal::{EditCommand, TextModal, apply};

/// Number of suggestion rows painted above the command line.
pub const VISIBLE_SUGGESTIONS: usize = 6;

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
    pub editor: TextModal,
    pub suggestions: Vec<Suggestion>,
    pub selected: usize,
    suggestions_offset: usize,
    context: ipc_command::SuggestionContext,
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
        self.suggestions =
            ipc_command::suggestions_with(&self.editor.text, self.editor.cursor, &self.context);
        self.selected = 0;
        self.suggestions_offset = 0;
    }

    pub fn set_context(&mut self, context: ipc_command::SuggestionContext) {
        self.context = context;
        self.refresh();
    }

    pub fn install_search_suggestions(
        &mut self,
        start: usize,
        end: usize,
        rows: &[(String, String)],
    ) {
        self.suggestions = rows
            .iter()
            .map(|(title, subtitle)| Suggestion {
                start,
                end,
                label: title.clone(),
                description: subtitle.clone(),
                completion: Some(ipc_command::quote_arg(title)),
            })
            .collect();
        self.selected = 0;
        self.suggestions_offset = 0;
    }

    /// Move the highlighted completion. The list keeps the selected row in
    /// view; Up/Down therefore scrolls long suggestion sets.
    pub fn move_suggestion(&mut self, delta: isize) {
        let len = self.suggestions.len();
        if len == 0 {
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.selected = next;
        if next < self.suggestions_offset {
            self.suggestions_offset = next;
        } else if next >= self.suggestions_offset + VISIBLE_SUGGESTIONS {
            self.suggestions_offset = next + 1 - VISIBLE_SUGGESTIONS;
        }
    }

    pub fn visible_suggestions(&self) -> impl Iterator<Item = (usize, &Suggestion)> {
        self.suggestions
            .iter()
            .enumerate()
            .skip(self.suggestions_offset)
            .take(VISIBLE_SUGGESTIONS)
    }

    pub fn reset_completion(&mut self) {
        self.cycle = None;
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the command-line tests exercise this edit helper"
        )
    )]
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

    pub fn apply_edit(&mut self, command: &EditCommand) {
        apply(&mut self.editor, command);
        self.reset_completion();
        self.refresh();
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the command-line tests exercise this edit helper"
        )
    )]
    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text);
        self.reset_completion();
        self.refresh();
    }

    /// Neovim-style Tab/Shift-Tab completion. The first press replaces the
    /// byte span of the selected suggestion (as computed by the suggestion
    /// source, so multi-word arguments such as `playlist add ; "Road T"`
    /// complete as one region); later presses cycle the discovered candidates.
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

        // The suggestion source owns the replaceable byte span: it may cover a
        // multi-word region (the whole typed playlist name) instead of the
        // whitespace-delimited token under the cursor.
        let entries: Vec<Completion> = self
            .suggestions
            .iter()
            .filter_map(|suggestion| {
                suggestion.completion.as_ref().map(|completion| Completion {
                    span: CompletionSpan::clamped(
                        suggestion.start,
                        suggestion.end,
                        &self.editor.text,
                    ),
                    completion: completion.clone(),
                })
            })
            .collect();
        if entries.is_empty() {
            return false;
        }
        let selected_completion = self
            .suggestions
            .get(self.selected)
            .and_then(|suggestion| suggestion.completion.as_deref());
        let index = selected_completion
            .and_then(|selected| {
                entries
                    .iter()
                    .position(|entry| entry.completion == selected)
            })
            .unwrap_or(if backward { entries.len() - 1 } else { 0 });
        let span = entries[index].span;
        // Suggestions may predate the latest edit (an async reply can land
        // after the text moved on). Only trust the source span while it still
        // covers the cursor; otherwise fall back to the token under it.
        let cursor = self.editor.cursor.min(self.editor.text.len());
        let span = if span.start <= cursor && cursor <= span.end {
            span
        } else {
            self.token_span_under_cursor()
        };
        let completion = entries[index].completion.clone();
        let values: Vec<String> = entries.into_iter().map(|entry| entry.completion).collect();
        // A completion of the command token itself starts the argument list, so
        // it appends the space that separates the argument.
        let is_command = self.editor.text[..span.start].trim().is_empty();
        let append_space = is_command && ipc_command::needs_argument(&completion);
        self.editor
            .text
            .replace_range(span.start..span.end, &completion);
        let mut cursor = span.start + completion.len();
        if append_space {
            self.editor.text.insert(cursor, ' ');
            cursor += 1;
        }
        self.editor.cursor = cursor;
        self.editor.anchor = cursor;
        // Refresh for the text left under the cursor: after a command
        // completion that appended a trailing space this is the empty first
        // argument token, so its values are offered immediately.
        self.refresh();
        if append_space {
            self.cycle = None;
        } else {
            self.cycle = Some(CompletionCycle {
                start: span.start,
                end: span.start + completion.len(),
                values,
                index,
            });
            self.selected = index.min(self.suggestions.len().saturating_sub(1));
        }
        true
    }

    /// Fallback replaceable span when the source-provided one is stale: the
    /// whitespace-delimited token around the cursor.
    fn token_span_under_cursor(&self) -> CompletionSpan {
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
        CompletionSpan { start, end }
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

/// The byte region a suggestion asks Tab to replace, with the completion that
/// replaces it.
#[derive(Debug, Clone)]
struct Completion {
    span: CompletionSpan,
    completion: String,
}

/// Byte span of the text a completion replaces, as reported by the suggestion
/// source (multi-word regions included).
#[derive(Debug, Clone, Copy)]
struct CompletionSpan {
    start: usize,
    end: usize,
}

impl CompletionSpan {
    /// Clamps a source-provided span into `text`, snapping both ends to char
    /// boundaries so a stale reply can never split a UTF-8 sequence.
    fn clamped(start: usize, end: usize, text: &str) -> Self {
        let start = floor_char_boundary(text, start.min(text.len()));
        let end = floor_char_boundary(text, end.max(start).min(text.len()));
        Self { start, end }
    }
}

/// Largest char boundary at or below `index`, clamped to `text.len()`.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
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
    fn tab_completes_the_selected_suggestion() {
        let mut state = CommandLineState::new();
        state.open();
        state.set_text("shuffle ");
        let labels: Vec<&str> = state.suggestions.iter().map(|s| s.label.as_str()).collect();
        assert!(labels.contains(&"shuffle album"));
        let album = labels
            .iter()
            .position(|label| *label == "shuffle album")
            .unwrap();
        state.selected = album;
        assert!(state.complete(false));
        assert_eq!(state.query(), "shuffle album");
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

    /// U-18 regression: the completion must replace the byte span the
    /// suggestion source reported, not the whitespace token under the cursor.
    /// A playlist name typed after the `;` cutoff spans several words, so a
    /// token-local replacement would leave the earlier typed words behind.
    #[test]
    fn tab_completion_replaces_suggestion_span() {
        let mut state = CommandLineState::new();
        state.open();
        let query = "playlist add ; Road T";
        state.set_text(query);
        let span_start = query.find("Road T").expect("typed name region");
        state.install_search_suggestions(
            span_start,
            query.len(),
            &[("Road Trip".to_string(), "12 tracks".to_string())],
        );
        assert!(state.complete(false));
        assert_eq!(state.query(), "playlist add ; \"Road Trip\"");
        assert_eq!(state.editor.cursor, state.query().len());

        // Completing a bare command appends the argument space and refreshes
        // the suggestions for the empty argument token in the same press.
        state.set_text("syn");
        assert!(state.complete(false));
        assert_eq!(state.query(), "sync ");
        assert_eq!(
            state
                .suggestions
                .first()
                .and_then(|suggestion| suggestion.completion.as_deref()),
            Some("all"),
        );
    }

    /// A suggestion reply can land after the text moved on; its stale span must
    /// not be used to splice the completion over unrelated text.
    #[test]
    fn stale_suggestion_span_falls_back_to_the_cursor_token() {
        let mut state = CommandLineState::new();
        state.open();
        state.set_text("sync all");
        state.install_search_suggestions(0, 4, &[("repair".to_string(), "rebuild".to_string())]);
        assert!(state.complete(false));
        assert_eq!(state.query(), "sync repair");
    }
}
