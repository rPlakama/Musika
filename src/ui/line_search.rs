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

//! The `/` in-surface command line, as a pure model (no GPUI types).
//!
//! This mirrors `LineSearch.qml` + `SearchMatch.js`: the whole window is one
//! search buffer, made of the surfaces' visible rows in window order. Rows are
//! sparse — a surface only contributes the rows that are actually on screen —
//! and each target keeps its own row indices, so a hit is a
//! `(target, row)` pair and `searchJump` needs no remapping.
//!
//! Matching is literal smartcase, as in `SearchMatch.js`: a query with any
//! ASCII uppercase character is case-sensitive, otherwise it folds. Ranges are
//! byte ranges into the original text, always on `char` boundaries, in
//! left-to-right, non-overlapping order — ready for a GPUI `TextRun` loop.
//! `LineRow::search_text` joins the three rendered cells with
//! [`CELL_SEPARATOR`] (`'\n'`), so an ordinary single-line query can never
//! match across a cell boundary; `LineRow::cell_ranges` is the per-cell form
//! for widgets that paint title/subtitle/meta separately.
//!
//! The model has no window, focus or target-snapshot ownership. The caller
//! snapshots `&[LineTarget]` once when `/` opens and reuses it for every query
//! edit, exactly like QML `LineSearch.open()` captures the visible row buffer.

use std::ops::Range;

/// The separator used by [`LineRow::search_text`]. A newline keeps the
/// joined text stable while still preventing a query typed into the
/// single-line bar from spanning two cells.
pub const CELL_SEPARATOR: char = '\n';

/// The rendered cells of one visible row plus search-only extras.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineRow {
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    /// Search-only cells that are not painted as title/subtitle/meta (album
    /// track number, queue duration, …).
    pub extra: Vec<String>,
    /// Cell indices that participate in candidate matching. Indices 0..=2 map
    /// to title/subtitle/meta; indices >=3 map to `extra`. QML's
    /// `SearchContract.fields` selects these per surface; painting still uses
    /// all three rendered cells.
    pub search_fields: Vec<usize>,
}

impl Default for LineRow {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            meta: String::new(),
            extra: Vec::new(),
            search_fields: vec![0, 1, 2],
        }
    }
}

impl LineRow {
    pub fn new(
        title: impl Into<String>,
        subtitle: impl Into<String>,
        meta: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            meta: meta.into(),
            extra: Vec::new(),
            search_fields: vec![0, 1, 2],
        }
    }

    /// Adds search-only cells after the painted three.
    pub fn with_extra(
        title: impl Into<String>,
        subtitle: impl Into<String>,
        meta: impl Into<String>,
        extra: Vec<String>,
    ) -> Self {
        let mut search_fields = vec![0, 1, 2];
        search_fields.extend((0..extra.len()).map(|index| index + 3));
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            meta: meta.into(),
            extra,
            search_fields,
        }
    }

    /// Restricts candidate matching to the given cells (QML per-surface
    /// `searchFields`). Out-of-range indices are ignored.
    pub fn with_search_fields(mut self, fields: impl IntoIterator<Item = usize>) -> Self {
        self.search_fields = fields
            .into_iter()
            .filter(|index| *index < 3 + self.extra.len())
            .collect();
        self
    }

    /// The three cells, in render order.
    pub fn cells(&self) -> [&str; 3] {
        [&self.title, &self.subtitle, &self.meta]
    }

    fn cell(&self, index: usize) -> Option<&str> {
        match index {
            0 => Some(&self.title),
            1 => Some(&self.subtitle),
            2 => Some(&self.meta),
            _ => self.extra.get(index - 3).map(String::as_str),
        }
    }

    /// The cells joined by [`CELL_SEPARATOR`] — the stable text a highlight
    /// layout can run over. Use [`Self::cell_ranges`] when each cell is painted
    /// as its own text element.
    pub fn search_text(&self) -> String {
        let mut text = String::new();
        for (index, cell) in self.cells().iter().enumerate() {
            if index > 0 {
                text.push(CELL_SEPARATOR);
            }
            text.push_str(cell);
        }
        for cell in &self.extra {
            text.push(CELL_SEPARATOR);
            text.push_str(cell);
        }
        text
    }

    /// Whether any selected search field contains the query under smartcase.
    /// This is `SearchMatch.candidates`; cells are independent, so a query
    /// never matches across a cell boundary.
    pub fn matches(&self, query: &str) -> bool {
        !query.is_empty()
            && self.search_fields.iter().any(|index| {
                self.cell(*index)
                    .is_some_and(|cell| !match_ranges(query, cell).is_empty())
            })
    }

    /// Highlight ranges per painted cell, in `[title, subtitle, meta]` order.
    pub fn cell_ranges(&self, query: &str) -> [Vec<Range<usize>>; 3] {
        [
            match_ranges(query, &self.title),
            match_ranges(query, &self.subtitle),
            match_ranges(query, &self.meta),
        ]
    }
}

/// One searchable surface: its rows in display order. Sparse: `None` means
/// "row not on screen", so the index stays the surface's absolute row index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineTarget {
    pub rows: Vec<Option<LineRow>>,
}

impl LineTarget {
    pub fn new(rows: Vec<Option<LineRow>>) -> Self {
        Self { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, row: usize) -> Option<&LineRow> {
        self.rows.get(row).and_then(Option::as_ref)
    }
}

/// One matching row: which target, and the target-local row index.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LineHit {
    pub target: usize,
    pub row: usize,
}

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

/// The `/` command-line session. `target` tracks the target that owns the
/// currently previewed candidate (0 when there is none), for callers that
/// publish the preview to one surface at a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineSearchState {
    pub open: bool,
    pub editor: LineEditor,
    pub candidates: Vec<LineHit>,
    pub index: Option<usize>,
    pub target: usize,
}

impl LineSearchState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn query(&self) -> &str {
        &self.editor.text
    }

    pub fn editor_mut(&mut self) -> &mut LineEditor {
        &mut self.editor
    }

    /// Open a fresh command line: clear the previous query, ring and preview
    /// (QML `LineSearch.open` assigns `opened = true` and empties the field).
    pub fn open(&mut self) {
        self.clear_session();
        self.open = true;
    }

    /// Close and clear the session.
    pub fn close(&mut self) {
        self.open = false;
        self.clear_session();
    }

    /// Return to the closed, empty state. Like closing, this discards the
    /// query and ring; callers resetting *inside* an open session should call
    /// [`Self::set_query`] with the new query instead.
    pub fn reset(&mut self) {
        self.open = false;
        self.clear_session();
    }

    fn clear_session(&mut self) {
        self.editor.clear();
        self.candidates.clear();
        self.index = None;
        self.target = 0;
    }

    /// Replace the query, rediscover candidates over the current visible
    /// targets, and preview the first hit. An empty query matches nothing.
    pub fn set_query(&mut self, query: &str, targets: &[LineTarget]) {
        self.editor.set_text(query);
        self.refresh(targets);
    }

    /// Rediscover candidates for the existing query (for example when the
    /// visible band or the target list changed) and preview the first hit.
    pub fn refresh(&mut self, targets: &[LineTarget]) {
        self.candidates = find_candidates(targets, &self.editor.text);
        self.index = if self.candidates.is_empty() {
            None
        } else {
            Some(0)
        };
        self.sync_target();
    }

    /// Cycle the candidate ring with wrap-around: `step(1)` is Tab,
    /// `step(-1)` is Shift-Tab. Keeps `target` in sync with the preview.
    pub fn step(&mut self, delta: isize) {
        let len = self.candidates.len();
        if len == 0 {
            self.index = None;
            self.target = 0;
            return;
        }
        let current = self.index.unwrap_or(0) as isize;
        self.index = Some((current + delta).rem_euclid(len as isize) as usize);
        self.sync_target();
    }

    /// The candidate currently previewed, if any.
    pub fn current_hit(&self) -> Option<LineHit> {
        self.index
            .and_then(|index| self.candidates.get(index).copied())
    }

    /// Act on the current candidate: return it and make it the active target.
    /// `None` when the query has no matches (or no query is set).
    pub fn jump(&mut self) -> Option<LineHit> {
        let hit = self.current_hit();
        if let Some(hit) = hit {
            self.target = hit.target;
        }
        hit
    }

    fn sync_target(&mut self) {
        self.target = self.current_hit().map_or(0, |hit| hit.target);
    }
}

/// Candidate discovery: every row containing the query, in target order and
/// ascending row order within a target. Sparse `None` rows are skipped.
pub fn find_candidates(targets: &[LineTarget], query: &str) -> Vec<LineHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (target, surface) in targets.iter().enumerate() {
        for (row, line) in surface.rows.iter().enumerate() {
            if let Some(line) = line
                && line.matches(query)
            {
                hits.push(LineHit { target, row });
            }
        }
    }
    hits
}

/// Every non-overlapping occurrence of `query` inside `text`, as byte ranges
/// into `text`.
///
/// Smartcase, exactly like `SearchMatch.js`: any ASCII uppercase character in
/// the query (`[A-Z]`) makes the match case-sensitive; otherwise it folds.
/// An empty query or text matches nothing.
pub fn match_ranges(query: &str, text: &str) -> Vec<Range<usize>> {
    if query.is_empty() || text.is_empty() {
        return Vec::new();
    }

    if query.chars().any(|ch| ch.is_ascii_uppercase()) {
        return text
            .match_indices(query)
            .map(|(start, found)| start..start + found.len())
            .collect();
    }

    // Fold the haystack character by character, remembering which original
    // character each folded character came from. That keeps the returned
    // ranges as valid byte slices of `text` even when a character folds to
    // several lowercase characters (e.g. `İ` -> `i` + combining dot).
    let mut hay = Vec::with_capacity(text.len());
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        for folded in ch.to_lowercase() {
            hay.push(Folded {
                ch: folded,
                start,
                end,
            });
        }
    }
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        let hit = hay[at..at + needle.len()]
            .iter()
            .map(|folded| folded.ch)
            .eq(needle.iter().copied());
        if hit {
            ranges.push(hay[at].start..hay[at + needle.len() - 1].end);
            at += needle.len();
        } else {
            at += 1;
        }
    }
    ranges
}

/// One character of the folded haystack and the original character's byte span.
#[derive(Debug, Clone, Copy)]
struct Folded {
    ch: char,
    start: usize,
    end: usize,
}

/// Ctrl-W in the command line: drop the trailing whitespace run and then the
/// word before the cursor. The cursor is modelled at the end of `text`, which
/// is the only position the `/` bar's word-delete exposes. Mirrors
/// `SearchInput.deleteWordBack` (which also keeps text after a mid-string
/// cursor; `&mut String` alone cannot express that case).
pub fn delete_word_back(text: &mut String) {
    let mut start = text.len();

    while let Some((index, ch)) = text[..start].char_indices().next_back() {
        if ch.is_whitespace() {
            start = index;
        } else {
            break;
        }
    }

    while let Some((index, ch)) = text[..start].char_indices().next_back() {
        if ch.is_whitespace() {
            break;
        }
        start = index;
    }

    if start < text.len() {
        text.truncate(start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(rows: Vec<Option<LineRow>>) -> LineTarget {
        LineTarget::new(rows)
    }

    fn row(title: &str, subtitle: &str, meta: &str) -> Option<LineRow> {
        Some(LineRow::new(title, subtitle, meta))
    }

    fn ring_targets() -> Vec<LineTarget> {
        vec![
            target(vec![row("hit one", "", ""), row("miss", "", "")]),
            target(vec![row("", "hit two", "")]),
            target(vec![row("", "", "hit three"), None]),
        ]
    }

    #[test]
    fn smartcase_is_decided_by_ascii_uppercase() {
        // No ASCII uppercase: fold.
        assert_eq!(match_ranges("hello", "Hello hello"), vec![0..5, 6..11]);
        // ASCII uppercase anywhere: case-sensitive.
        assert_eq!(match_ranges("Hello", "Hello hello"), vec![0..5]);
        assert_eq!(match_ranges("hEllo", "hello"), Vec::<Range<usize>>::new());
        assert!(match_ranges("A", "a").is_empty());
        // Non-ASCII uppercase does not trip the QML `[A-Z]` test.
        assert_eq!(match_ranges("É", "é"), vec![0..2]);
        assert_eq!(match_ranges("é", "É"), vec![0..2]);
    }

    #[test]
    fn ranges_are_byte_offsets_non_overlapping_and_boundary_safe() {
        assert_eq!(match_ranges("an", "banana"), vec![1..3, 3..5]);
        assert_eq!(match_ranges("aa", "aaaa"), vec![0..2, 2..4]);
        assert_eq!(match_ranges("aa", "aaa"), vec![0..2]);
        assert_eq!(match_ranges("", "abc"), Vec::<Range<usize>>::new());
        assert_eq!(match_ranges("a", ""), Vec::<Range<usize>>::new());

        let text = "Éclair éclair";
        let found = match_ranges("éclair", text);
        assert_eq!(found, vec![0..7, 8..15]);
        for range in found {
            assert!(text.is_char_boundary(range.start));
            assert!(text.is_char_boundary(range.end));
        }
    }

    #[test]
    fn folded_ranges_expand_to_the_whole_original_character() {
        // `İ` lowercases to `i` + combining dot; highlighting `i` must still
        // return a range that slices the original two-byte character.
        let text = "İstanbul";
        assert_eq!(match_ranges("i", text), vec![0..2]);
        assert!(text.is_char_boundary(2));
    }

    #[test]
    fn candidates_follow_target_and_row_order_and_skip_sparse_rows() {
        let targets = vec![
            target(vec![row("Alpha", "", "one"), None, row("beta", "two", "")]),
            target(vec![row("gamma", "alphabet", ""), row("ALPHA", "", "")]),
        ];

        assert_eq!(
            find_candidates(&targets, "alpha"),
            vec![
                LineHit { target: 0, row: 0 },
                LineHit { target: 1, row: 0 },
                LineHit { target: 1, row: 1 },
            ]
        );
        // Smartcase: only the exact `Alpha` row.
        assert_eq!(
            find_candidates(&targets, "Alpha"),
            vec![LineHit { target: 0, row: 0 }]
        );
        // Subtitle and meta are searched like the title.
        assert_eq!(
            find_candidates(&targets, "two"),
            vec![LineHit { target: 0, row: 2 }]
        );
        assert_eq!(
            find_candidates(&targets, "one"),
            vec![LineHit { target: 0, row: 0 }]
        );
        assert!(find_candidates(&targets, "").is_empty());
    }

    #[test]
    fn preview_steps_with_wrap_and_keeps_target_in_sync() {
        let targets = ring_targets();
        let mut state = LineSearchState::new();
        state.open();
        state.set_query("hit", &targets);

        assert_eq!(state.index, Some(0));
        assert_eq!(state.target, 0);
        assert_eq!(state.jump(), Some(LineHit { target: 0, row: 0 }));

        state.step(-1);
        assert_eq!(state.index, Some(2));
        assert_eq!(state.target, 2);
        assert_eq!(state.jump(), Some(LineHit { target: 2, row: 0 }));

        state.step(1);
        assert_eq!(state.index, Some(0));
        assert_eq!(state.target, 0);

        state.step(5); // (0 + 5) % 3 == 2
        assert_eq!(state.index, Some(2));
        assert_eq!(state.jump(), Some(LineHit { target: 2, row: 0 }));

        state.step(-2); // (2 - 2) % 3 == 0
        assert_eq!(state.index, Some(0));
        assert_eq!(state.jump(), Some(LineHit { target: 0, row: 0 }));
    }

    #[test]
    fn no_match_clears_the_preview_and_step_is_inert() {
        let targets = ring_targets();
        let mut state = LineSearchState::new();
        state.open();
        state.set_query("nothing", &targets);
        assert!(state.candidates.is_empty());
        assert_eq!(state.index, None);
        assert_eq!(state.target, 0);
        assert_eq!(state.jump(), None);

        state.step(1);
        assert_eq!(state.index, None);
        assert_eq!(state.jump(), None);
    }

    #[test]
    fn delete_word_back_matches_the_qml_editor() {
        let mut text = String::from("hello world  ");
        delete_word_back(&mut text);
        assert_eq!(text, "hello ");

        let mut text = String::from("hello   ");
        delete_word_back(&mut text);
        assert_eq!(text, "");

        let mut text = String::from("hello");
        delete_word_back(&mut text);
        assert_eq!(text, "");

        let mut text = String::from("   ");
        delete_word_back(&mut text);
        assert_eq!(text, "");

        let mut text = String::from("");
        delete_word_back(&mut text);
        assert_eq!(text, "");

        let mut text = String::from("héllo wörld ");
        delete_word_back(&mut text);
        assert_eq!(text, "héllo ");

        let mut text = String::from("foo\tbar\n ");
        delete_word_back(&mut text);
        assert_eq!(text, "foo\t");
    }

    #[test]
    fn close_and_reset_clear_the_whole_session() {
        let targets = ring_targets();
        let mut state = LineSearchState::new();
        state.open();
        state.set_query("hit", &targets);
        assert!(state.open);
        assert!(!state.candidates.is_empty());

        state.close();
        assert!(!state.open);
        assert!(state.query().is_empty());
        assert!(state.candidates.is_empty());
        assert_eq!(state.index, None);
        assert_eq!(state.target, 0);

        // `open` always starts a fresh session, even after a previous one.
        state.open();
        state.set_query("hit", &targets);
        state.open();
        assert!(state.open);
        assert!(state.query().is_empty());
        assert!(state.candidates.is_empty());
        assert_eq!(state.index, None);
        assert_eq!(state.target, 0);

        state.set_query("hit", &targets);
        state.reset();
        assert!(!state.open);
        assert!(state.query().is_empty());
        assert!(state.candidates.is_empty());
        assert_eq!(state.index, None);
        assert_eq!(state.target, 0);
    }

    #[test]
    fn row_text_and_cell_ranges_share_the_match_model() {
        let line = LineRow::new("Alpha", "Beta", "1999");
        assert_eq!(line.search_text(), "Alpha\nBeta\n1999");
        assert_eq!(line.cells(), ["Alpha", "Beta", "1999"]);
        let mut expected: [Vec<Range<usize>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        expected[1].push(0..4);
        assert_eq!(line.cell_ranges("beta"), expected);
        assert!(line.matches("alpha"));
        assert!(line.matches("1999"));
        assert!(!line.matches("ALPHA"));
        assert!(!line.matches(""));
    }

    #[test]
    fn search_fields_control_matching_not_painted_cells() {
        let row = LineRow::new("visible title", "hidden subtitle", "")
            .with_search_fields([0]);
        assert!(row.matches("visible"));
        assert!(!row.matches("hidden"));
        assert_eq!(row.cells(), ["visible title", "hidden subtitle", ""]);

        let album = LineRow::with_extra("Song", "Artist", "3:20", vec!["7".to_string()])
            .with_search_fields([0, 1, 3]);
        assert!(album.matches("song"));
        assert!(album.matches("7"));
        assert!(!album.matches("3:20"));
    }

    #[test]
    fn editor_selection_and_word_delete_respect_char_boundaries() {
        let mut editor = LineEditor::new();
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

}
