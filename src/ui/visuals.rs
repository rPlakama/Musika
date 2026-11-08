//! Central visual tokens: typography and motion.
//!
//! Views should read sizes, durations and easing from here instead of carrying
//! their own literals. Changing the feel of every row then means editing this
//! file, not each surface.

use crate::ui::anim::Curve;

#[derive(Debug, Clone, Copy, Default)]
pub struct Visuals {
    pub fonts: Fonts,
    pub motion: Motion,
}

impl Visuals {
    /// Cover multiplier for a row emphasis factor (0 = resting, 1 = active).
    pub fn row_cover_scale(&self, factor: f32) -> f32 {
        1.0 + factor * self.motion.row_cover_selected
    }

    /// Text multiplier for a row emphasis factor (0 = resting, 1 = active).
    pub fn row_text_emphasis(&self, factor: f32) -> f32 {
        1.0 + factor * self.motion.row_text_selected
    }

    pub fn search_cover_scale(&self, factor: f32) -> f32 {
        1.0 + factor * self.motion.search_cover_selected
    }

    pub fn search_text_emphasis(&self, factor: f32) -> f32 {
        1.0 + factor * self.motion.search_text_selected
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Fonts {
    pub row_title: f32,
    pub row_title_compact: f32,
    pub row_subtitle: f32,
    pub row_subtitle_compact: f32,
    pub row_meta: f32,
    pub row_rating: f32,
    pub queue_title: f32,
    pub queue_meta: f32,
    pub search_title: f32,
    pub search_subtitle: f32,
    pub search_meta: f32,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            row_title: 18.0,
            row_title_compact: 15.0,
            row_subtitle: 12.0,
            row_subtitle_compact: 11.0,
            row_meta: 11.0,
            row_rating: 10.0,
            queue_title: 14.0,
            queue_meta: 10.0,
            search_title: 15.0,
            search_subtitle: 11.0,
            search_meta: 11.0,
        }
    }
}

impl Fonts {
    pub fn row_title(&self, compact: bool) -> f32 {
        if compact {
            self.row_title_compact
        } else {
            self.row_title
        }
    }

    pub fn row_subtitle(&self, compact: bool) -> f32 {
        if compact {
            self.row_subtitle_compact
        } else {
            self.row_subtitle
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Motion {
    /// Cover scaling is immediate; text gets more time to settle.
    pub row_cover_ms: f32,
    pub row_text_ms: f32,
    pub row_curve: Curve,
    pub row_cover_selected: f32,
    pub row_text_selected: f32,
    pub search_cover_selected: f32,
    pub search_text_selected: f32,
    /// Fullscreen lyrics overlay backdrop. 50% stronger than the old 0.35.
    pub lyrics_scrim: f32,
    /// Spotify-like active-line size/gap settle: very quick.
    pub lyrics_line_ms: f32,
    /// Softer fade for unsung/neighbouring lines.
    pub lyrics_fade_ms: f32,
    /// Lyric-centering glide when the active line changes.
    pub lyrics_scroll_ms: f32,
}

impl Default for Motion {
    fn default() -> Self {
        let row_cover_ms = 80.0;
        Self {
            row_cover_ms,
            // 80% slower than the cover so glyphs do not pop ahead of art.
            row_text_ms: row_cover_ms * 1.8,
            row_curve: Curve::Linear,
            row_cover_selected: 0.20,
            row_text_selected: 0.08,
            search_cover_selected: 0.08,
            search_text_selected: 0.08,
            lyrics_scrim: 0.525,
            lyrics_line_ms: 400.0,
            lyrics_fade_ms: 400.0,
            lyrics_scroll_ms: 300.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_snappy_and_centralized() {
        let visuals = Visuals::default();
        assert_eq!(visuals.motion.row_cover_ms, 80.0);
        assert_eq!(visuals.motion.row_text_ms, 144.0);
        assert_eq!(visuals.motion.row_curve, Curve::Linear);
        assert_eq!(visuals.fonts.row_title(true), 15.0);
        assert_eq!(visuals.fonts.row_title(false), 18.0);
        assert_eq!(visuals.fonts.row_subtitle(true), 11.0);
        assert_eq!(visuals.motion.lyrics_scrim, 0.525);
        assert_eq!(visuals.motion.lyrics_line_ms, 400.0);
        assert_eq!(visuals.motion.lyrics_scroll_ms, 300.0);
    }
}
