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
            rows: Vec::new(),
            gap: base_size * 0.5,
            base_size,
            text_height: base_size * LINE_HEIGHT_SCALE,
            width: 0.0,
        }
    }
}

/// Measure the number of wrapped lines for one lyric at `size` and `width`.
/// The callback is the host's real text measurement (tests pass a cheap stub).
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
    let base = base_size.max(1.0);
    let gap = base * 0.5;
    let text_height = base * LINE_HEIGHT_SCALE;
    if lines.is_empty() || width <= 0.0 || height <= 0.0 {
        return LyricsLayout::empty(base);
    }
    let active = active.min(lines.len() - 1);

    // Reserve the active emphasis on every row: moving the highlight can never
    // change the column's total height (QML reserves `emphasisPad` likewise).
    let heights: Vec<f32> = lines
        .iter()
        .map(|(_, text)| {
            let wrapped = measure(text, base * EMPHASIS).max(1) as f32;
            (wrapped * text_height * EMPHASIS).max(text_height)
        })
        .collect();

    if heights[active] > height {
        // A single line cannot fit even at the minimum size; the host shrinks
        // the base size and calls again. Keep it visible in the fallback.
        return LyricsLayout {
            start: active,
            rows: vec![LyricRow {
                index: active,
                height: heights[active],
                active: true,
            }],
            gap,
            base_size: base,
            text_height,
            width,
        };
    }

    // Grow outward from the active line, alternating sides so it stays
    // centred when possible. Both ends are clamped to the available height.
    let mut start = active;
    let mut end = active + 1;
    let mut used = heights[active];
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
        rows,
        gap,
        base_size: base,
        text_height,
        width,
    }
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
        assert!((small.gap - 13.0 * 0.5).abs() < 0.01);
        assert!((large.gap - 22.0 * 0.5).abs() < 0.01);
        assert!(small.rows.len() >= large.rows.len());
    }
}
