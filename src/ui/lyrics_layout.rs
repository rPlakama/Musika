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

/// Measure the number of wrapped lines for one lyric at `size` and `width`.
/// The callback is the host's real text measurement (tests pass a cheap stub).
///
/// `active` is the line the playback highlight sits on; `center` is the line
/// the pane is centred on (the follow target). They differ while a manual
/// scroll holds the pane away from the played line.
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
    F: Fn(&str, f32) -> usize,
{
    let base = base_size.max(1.0);
    let gap = base * 1.2;
    let text_height = base * LINE_HEIGHT_SCALE;
    if lines.is_empty() || width <= 0.0 || height <= 0.0 {
        return LyricsLayout::empty(base);
    }
    let active = active.min(lines.len() - 1);
    let center = center.min(lines.len() - 1);

    // Reserve the active emphasis on every row: moving the highlight can never
    // change the column's total height (QML reserves `emphasisPad` likewise).
    let heights: Vec<f32> = lines
        .iter()
        .map(|(_, text)| {
            let wrapped = measure(text, base * EMPHASIS).max(1) as f32;
            (wrapped * text_height * EMPHASIS).max(text_height)
        })
        .collect();

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
}
