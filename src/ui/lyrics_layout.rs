//! Variable-height lyric layout: fit the visible lines into the pane.

pub const EMPHASIS: f32 = 1.12;
pub const LINE_HEIGHT_SCALE: f32 = 1.25;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricRow {
    pub index: usize,
    pub height: f32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricsLayout {
    pub start: usize,
    /// The line the pane is centred on: the played line while the follow is
    /// live, or the anchor of a manual scroll.
    pub center: usize,
    pub rows: Vec<LyricRow>,
    pub gap: f32,
    pub base_size: f32,
    pub text_height: f32,
    pub width: f32,
}

impl LyricsLayout {
    pub fn empty(base_size: f32) -> Self {
        Self {
            start: 0,
            center: 0,
            rows: Vec::new(),
            gap: base_size * 1.2,
            base_size,
            text_height: base_size * LINE_HEIGHT_SCALE,
            width: 0.0,
        }
    }

    /// The full height of the windowed column: every row plus the gaps.
    pub fn content_height(&self) -> f32 {
        self.rows.iter().map(|row| row.height).sum::<f32>()
            + self.gap * self.rows.len().saturating_sub(1) as f32
    }

    /// Distance from the column's top edge to the row `index`, when that line
    /// is inside this window.
    fn row_top(&self, index: usize) -> Option<f32> {
        let position = self.rows.iter().position(|row| row.index == index)?;
        let mut y = 0.0;
        for row in &self.rows[..position] {
            y += row.height + self.gap;
        }
        Some(y)
    }

    /// The shift to paint this layout with so a line shared with `previous`
    /// stays exactly where `previous` painted it: the lyric follow glide.
    /// The column is centred in the pane, so the total height change moves it
    /// by half; using the real row heights keeps wrapped lines from jumping
    /// when the window scrolls (a uniform row step cannot, because lyric rows
    /// differ in height). `None` when the anchor is not inside both windows
    /// (a far seek), where no meaningful glide exists.
    pub fn glide_shift(&self, previous: &Self, anchor: usize) -> Option<f32> {
        let before = previous.row_top(anchor)?;
        let after = self.row_top(anchor)?;
        let height_delta = self.content_height() - previous.content_height();
        Some(height_delta * 0.5 + before - after)
    }
}

/// Per-line row heights: the wrapped line count measured at the emphasis size,
/// scaled by the line height. The active emphasis is reserved on every row, so
/// moving the highlight can never change the column's total height (QML
/// reserves `emphasisPad` likewise).
fn row_heights<F>(lines: &[(Option<i64>, String)], base: f32, mut measure: F) -> Vec<f32>
where
    F: FnMut(&str, f32) -> usize,
{
    let text_height = base * LINE_HEIGHT_SCALE;
    lines
        .iter()
        .map(|(_, text)| {
            let wrapped = measure(text, base * EMPHASIS).max(1) as f32;
            (wrapped * text_height * EMPHASIS).max(text_height)
        })
        .collect()
}

/// Exact revision of the lyrics text content. It is the memoization identity of
/// one lyrics document: a reloaded or re-parsed document always changes it, so
/// the wrapped-line cache cannot serve stale heights. Cheap enough to recompute
/// on every frame that lays lyrics out (a few KiB of hashing versus shaping
/// every line).
pub fn lines_revision(lines: &[(Option<i64>, String)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.len().hash(&mut hasher);
    for (start, text) in lines {
        start.hash(&mut hasher);
        text.hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Debug, PartialEq, Eq)]
struct MeasureKey {
    track: String,
    revision: u64,
    width: u32,
}

/// Memoizes the wrapped-line heights of one lyrics document.
///
/// The expensive part of a lyrics layout is measuring (shaping and wrapping)
/// every line; the window geometry arithmetic that follows is trivial. This
/// cache is keyed by `(track_id, lyrics_rev, width_px)` and holds one height
/// vector per requested font size, because the host steps the type size down
/// until a too-tall line fits. A same-key frame reuses the measured heights and
/// never calls the text system again.
#[derive(Default)]
pub struct LyricsMeasureCache {
    key: Option<MeasureKey>,
    /// `(font size bits, row heights)` for the sizes the stepping loop asked
    /// for under the current key. The number of sizes is tiny (the host only
    /// steps down from the nominal size).
    sizes: Vec<(u32, Vec<f32>)>,
}

impl LyricsMeasureCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Row heights for every line, wrapped at `size` in a `width` pane.
    ///
    /// `track`/`revision` identify the lyrics document (see [`lines_revision`]),
    /// `measure` is the host's real text measurement and is only called on a
    /// miss. `lines` must stay the document identified by `revision`.
    pub fn heights<F>(
        &mut self,
        track: &str,
        revision: u64,
        width: f32,
        size: f32,
        lines: &[(Option<i64>, String)],
        mut measure: F,
    ) -> &[f32]
    where
        F: FnMut(&str, f32) -> usize,
    {
        let width = width.max(0.0).to_bits();
        let same_document = self.key.as_ref().is_some_and(|key| {
            key.track == track && key.revision == revision && key.width == width
        });
        if !same_document {
            self.key = Some(MeasureKey {
                track: track.to_string(),
                revision,
                width,
            });
            self.sizes.clear();
        }
        let size = size.max(1.0);
        let size_bits = size.to_bits();
        if let Some(index) = self.sizes.iter().position(|(bits, _)| *bits == size_bits) {
            return &self.sizes[index].1;
        }
        let heights = row_heights(lines, size, &mut measure);
        self.sizes.push((size_bits, heights));
        match self.sizes.last() {
            Some((_, heights)) => heights.as_slice(),
            None => &[],
        }
    }
}

/// [`layout_windowed`] with the wrapped-line measurements memoized in a
/// process-local cache keyed by `(track_id, lyrics_rev, width_px, font_size)`.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors layout_windowed plus the cache identity; a parameter struct would only add indirection at the one call site"
)]
pub fn layout_windowed_memoized<F>(
    track: &str,
    revision: u64,
    lines: &[(Option<i64>, String)],
    active: usize,
    center: usize,
    width: f32,
    height: f32,
    base_size: f32,
    measure: F,
) -> LyricsLayout
where
    F: FnMut(&str, f32) -> usize,
{
    thread_local! {
        static CACHE: std::cell::RefCell<LyricsMeasureCache> =
            std::cell::RefCell::new(LyricsMeasureCache::new());
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let heights = cache.heights(track, revision, width, base_size, lines, measure);
        layout_windowed_with_heights(lines, active, center, width, height, base_size, heights)
    })
}

/// Measure the number of wrapped lines for one lyric at `size` and `width`.
/// The callback is the host's real text measurement (tests pass a cheap stub).
///
/// `active` is the line the playback highlight sits on; `center` is the line
/// the pane is centred on (the follow target). They differ while a manual
/// scroll holds the pane away from the played line.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the GPUI unit tests exercise this helper")
)]
pub fn layout_windowed<F>(
    lines: &[(Option<i64>, String)],
    active: usize,
    center: usize,
    width: f32,
    height: f32,
    base_size: f32,
    measure: F,
) -> LyricsLayout
where
    F: FnMut(&str, f32) -> usize,
{
    let base = base_size.max(1.0);
    if lines.is_empty() || width <= 0.0 || height <= 0.0 {
        return LyricsLayout::empty(base);
    }
    let heights = row_heights(lines, base, measure);
    layout_windowed_with_heights(lines, active, center, width, height, base, &heights)
}

/// [`layout_windowed`] over row heights the caller already measured (the cached
/// path). `heights` must have one entry per line.
pub fn layout_windowed_with_heights(
    lines: &[(Option<i64>, String)],
    active: usize,
    center: usize,
    width: f32,
    height: f32,
    base_size: f32,
    heights: &[f32],
) -> LyricsLayout {
    let base = base_size.max(1.0);
    let gap = base * 1.2;
    let text_height = base * LINE_HEIGHT_SCALE;
    if lines.is_empty() || width <= 0.0 || height <= 0.0 || heights.len() != lines.len() {
        return LyricsLayout::empty(base);
    }
    let active = active.min(lines.len() - 1);
    let center = center.min(lines.len() - 1);

    if heights[center] > height {
        // A single line cannot fit even at the minimum size; the host shrinks
        // the base size and calls again. Keep the centred line visible in the
        // fallback.
        return LyricsLayout {
            start: center,
            center,
            rows: vec![LyricRow {
                index: center,
                height: heights[center],
                active: center == active,
            }],
            gap,
            base_size: base,
            text_height,
            width,
        };
    }

    // Grow outward from the centred line, alternating sides so it stays
    // centred when possible. Both ends are clamped to the available height.
    let mut start = center;
    let mut end = center + 1;
    let mut used = heights[center];
    let mut grow_forward = true;
    loop {
        if grow_forward {
            grow_forward = false;
            if end < heights.len() {
                let next = heights[end];
                if used + gap + next <= height {
                    used += gap + next;
                    end += 1;
                    continue;
                }
            }
        } else {
            grow_forward = true;
            if start > 0 {
                let prev = heights[start - 1];
                if used + gap + prev <= height {
                    used += gap + prev;
                    start -= 1;
                    continue;
                }
            }
        }
        // One side is exhausted; give the other side a chance before stopping.
        let forward_open = end < heights.len() && used + gap + heights[end] <= height;
        let backward_open = start > 0 && used + gap + heights[start - 1] <= height;
        if !forward_open && !backward_open {
            break;
        }
    }

    let rows = (start..end)
        .map(|index| LyricRow {
            index,
            height: heights[index],
            active: index == active,
        })
        .collect();
    LyricsLayout {
        start,
        center,
        rows,
        gap,
        base_size: base,
        text_height,
        width,
    }
}

/// The follow layout: centre the played line (the ordinary synced-lyrics mode).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the GPUI unit tests exercise this helper")
)]
pub fn layout<F>(
    lines: &[(Option<i64>, String)],
    active: usize,
    width: f32,
    height: f32,
    base_size: f32,
    measure: F,
) -> LyricsLayout
where
    F: Fn(&str, f32) -> usize,
{
    layout_windowed(lines, active, active, width, height, base_size, measure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn char_lines(text: &str, size: f32) -> usize {
        let per_line = (48.0 / (size * 0.55)).max(1.0) as usize;
        text.chars().count().div_ceil(per_line).max(1)
    }

    fn lines(count: usize) -> Vec<(Option<i64>, String)> {
        (0..count)
            .map(|index| (Some(index as i64 * 1000), format!("line {index}")))
            .collect()
    }

    #[test]
    fn active_line_is_always_visible_and_fits() {
        let lines = lines(100);
        let layout = layout(&lines, 50, 500.0, 300.0, 18.0, char_lines);
        assert!(layout.rows.iter().any(|row| row.active));
        let total: f32 = layout.rows.iter().map(|row| row.height).sum::<f32>()
            + layout.gap * layout.rows.len().saturating_sub(1) as f32;
        assert!(total <= 300.0 + 0.01, "layout overflowed: {total}");
    }

    #[test]
    fn long_wrapped_line_reserves_emphasis_height() {
        let lines = vec![
            (Some(0), "short".to_string()),
            (
                Some(1000),
                "a very very very long lyric that has to wrap several times".to_string(),
            ),
        ];
        let layout = layout(&lines, 1, 120.0, 500.0, 20.0, char_lines);
        let active = layout.rows.iter().find(|row| row.active).unwrap();
        assert!(active.height > 20.0 * LINE_HEIGHT_SCALE);
        assert!(layout.gap > 0.0);
    }

    #[test]
    fn too_tall_active_line_keeps_a_row() {
        let lines = lines(4);
        let layout = layout(&lines, 1, 40.0, 10.0, 20.0, char_lines);
        assert_eq!(layout.rows.len(), 1);
        assert!(layout.rows[0].active);
    }

    #[test]
    fn gap_follows_type_size() {
        let lines = lines(10);
        let small = layout(&lines, 4, 500.0, 300.0, 13.0, char_lines);
        let large = layout(&lines, 4, 500.0, 300.0, 22.0, char_lines);
        assert!((small.gap - 13.0 * 1.2).abs() < 0.01);
        assert!((large.gap - 22.0 * 1.2).abs() < 0.01);
        assert!(small.rows.len() >= large.rows.len());
    }

    #[test]
    fn follow_layout_centres_the_active_line() {
        let layout = layout(&lines(100), 50, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(layout.center, 50);
        assert!(layout.rows.iter().any(|row| row.active && row.index == 50));
    }

    #[test]
    fn manual_window_centres_its_anchor_and_keeps_the_played_highlight_off_pane() {
        let lines = lines(100);
        // The played line stays 10 while a manual scroll holds the pane at 50.
        let layout = layout_windowed(&lines, 10, 50, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(layout.center, 50);
        assert!(layout.rows.iter().any(|row| row.index == 50));
        assert!(!layout.rows.iter().any(|row| row.index == 10));
        assert!(
            !layout.rows.iter().any(|row| row.active),
            "the played highlight is outside the held window"
        );
    }

    #[test]
    fn manual_window_clamps_the_anchor_at_both_ends() {
        let lines = lines(20);
        let top = layout_windowed(&lines, 10, 0, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(top.start, 0);
        assert_eq!(top.center, 0);
        let bottom = layout_windowed(&lines, 10, 19, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(bottom.center, 19);
        assert!(bottom.rows.iter().any(|row| row.index == 19));
        // Out-of-range anchors clamp instead of panicking.
        let clamped = layout_windowed(&lines, 0, 999, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(clamped.center, 19);
    }

    /// Painted y of a line relative to the pane centre, without the constant
    /// pane height (it cancels in the glide difference).
    fn painted_y(layout: &LyricsLayout, index: usize) -> f32 {
        -layout.content_height() * 0.5 + layout.row_top(index).unwrap()
    }

    #[test]
    fn glide_shift_keeps_the_anchor_line_in_place() {
        let lines = lines(100);
        let old = layout(&lines, 50, 500.0, 300.0, 18.0, char_lines);
        let new = layout(&lines, 51, 500.0, 300.0, 18.0, char_lines);
        let anchor = old.center;
        let shift = new.glide_shift(&old, anchor).expect("anchor stays visible");
        assert!(
            (painted_y(&old, anchor) - (painted_y(&new, anchor) + shift)).abs() < 0.01,
            "the previous phrase must keep its painted position"
        );
        // With uniform rows and the same window size the exact glide is one
        // row: the next phrase starts below the centre and rises into it.
        let row_step = new.rows[0].height + new.gap;
        assert!(
            (shift - row_step).abs() < 0.01,
            "uniform rows glide exactly one row: {shift} vs {row_step}"
        );
    }

    #[test]
    fn glide_shift_uses_the_real_row_heights() {
        // A wrapped neighbour changes the column height, so a uniform row step
        // is wrong; the exact glide keeps the painted line in place.
        let mut lines: Vec<(Option<i64>, String)> = (0..40)
            .map(|index| (Some(index as i64 * 1000), format!("line {index}")))
            .collect();
        lines[19].1 = "a longer line".to_string();
        let old = layout(&lines, 20, 500.0, 300.0, 18.0, char_lines);
        let new = layout(&lines, 21, 500.0, 300.0, 18.0, char_lines);
        let anchor = old.center;
        let shift = new.glide_shift(&old, anchor).expect("anchor stays visible");
        assert!(
            (painted_y(&old, anchor) - (painted_y(&new, anchor) + shift)).abs() < 0.01,
            "wrapped rows must not jump the column"
        );
        let uniform_step = new.rows[0].height + new.gap;
        assert!(
            (shift - uniform_step).abs() > 0.5,
            "the wrapped row makes the exact glide differ from a row step: {shift}"
        );
    }

    #[test]
    fn glide_shift_is_none_without_a_shared_line() {
        let lines = lines(100);
        let old = layout(&lines, 10, 500.0, 300.0, 18.0, char_lines);
        let far = layout(&lines, 80, 500.0, 300.0, 18.0, char_lines);
        assert!(far.glide_shift(&old, old.center).is_none());
    }

    #[test]
    fn cache_reuses_wrapped_lines_for_same_width() {
        let lines = lines(12);
        let mut cache = LyricsMeasureCache::new();
        let calls = std::cell::Cell::new(0usize);
        let measure = |text: &str, size: f32| {
            calls.set(calls.get() + 1);
            char_lines(text, size)
        };
        let first = cache
            .heights("song-1", 7, 500.0, 18.0, &lines, measure)
            .to_vec();
        assert_eq!(
            calls.get(),
            lines.len(),
            "the whole document is measured once"
        );
        let second = cache
            .heights("song-1", 7, 500.0, 18.0, &lines, measure)
            .to_vec();
        assert_eq!(first, second);
        assert_eq!(
            calls.get(),
            lines.len(),
            "a same-width frame must reuse the wrapped lines"
        );

        // A different pane width re-measures: wrapping depends on it.
        let _ = cache
            .heights("song-1", 7, 360.0, 18.0, &lines, measure)
            .to_vec();
        assert_eq!(calls.get(), lines.len() * 2);

        // A new lyrics document (revision) invalidates even at the same width.
        let _ = cache
            .heights("song-1", 8, 360.0, 18.0, &lines, measure)
            .to_vec();
        assert_eq!(calls.get(), lines.len() * 3);
    }

    #[test]
    fn cached_heights_layout_matches_the_measured_layout() {
        let lines = lines(30);
        let mut cache = LyricsMeasureCache::new();
        let heights = cache
            .heights("song", 1, 500.0, 18.0, &lines, char_lines)
            .to_vec();
        let cached = layout_windowed_with_heights(&lines, 10, 10, 500.0, 300.0, 18.0, &heights);
        let direct = layout_windowed(&lines, 10, 10, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(cached, direct);
    }

    #[test]
    fn memoized_layout_reuses_heights_across_panes() {
        let lines = lines(20);
        let calls = std::cell::Cell::new(0usize);
        let measure = |text: &str, size: f32| {
            calls.set(calls.get() + 1);
            char_lines(text, size)
        };
        // Same document and width, different playback windows: only the window
        // arithmetic reruns, the wrapped heights come from the cache.
        let first =
            layout_windowed_memoized("memo-track", 5, &lines, 8, 8, 500.0, 300.0, 18.0, measure);
        assert_eq!(calls.get(), lines.len());
        let second =
            layout_windowed_memoized("memo-track", 5, &lines, 9, 9, 500.0, 300.0, 18.0, measure);
        assert_eq!(calls.get(), lines.len(), "second pane reuses the heights");
        let direct = layout_windowed(&lines, 9, 9, 500.0, 300.0, 18.0, char_lines);
        assert_eq!(second, direct);
        assert_ne!(first.center, second.center);
    }

    #[test]
    fn lines_revision_tracks_text_changes() {
        let document = lines(4);
        let revision = lines_revision(&document);
        assert_eq!(revision, lines_revision(&document));
        let mut changed = document;
        changed[2].1.push('!');
        assert_ne!(revision, lines_revision(&changed));
        assert_ne!(revision, lines_revision(&lines(5)));
    }
}
