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

//! Pure layout metrics ported from the legacy UI front end.
//!
//! Every formula here mirrors `the retired reference UI`, ``
//! and the row-metric block of `src/legacy/ScrollContent.legacy`/`SearchColumn.legacy`.
//! Nothing in this module touches GPUI: it is plain `f32` geometry so the
//! port's type ramp and row sizing can be tested without a window. Font sizes
//! are logical pixels; callers convert them with `gpui::px(...)`.
//!
//! The legacy UI side is the source of truth; when a fraction changes there, change
//! it here in the same commit.

/// `Theme.breakpoints.narrow`: below this width the chrome type ramp starts
/// shrinking.
pub const NARROW_WIDTH: f32 = 640.0;
/// `Theme.breakpoints.compact`: at or above this width the ramp is 1.0.
pub const COMPACT_WIDTH: f32 = 900.0;
/// `Theme.widthScale` default floor (UI-57).
pub const WIDTH_SCALE_FLOOR: f32 = 0.7;
/// `ScrollContent.minRowHeight`: the smallest whole row the lists ever get.
pub const MIN_ROW_HEIGHT: f32 = 18.0;
/// A row may lose at most 18% of its captured baseline (UI-58).
pub const BASELINE_FLOOR: f32 = 0.82;

/// `Theme.clamp`: the legacy UI order `Math.max(min, Math.min(max, value))`, kept
/// exactly so NaN and inverted bounds behave like the legacy UI source.
#[inline]
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() || min.is_nan() || max.is_nan() {
        return f32::NAN;
    }
    value.min(max).max(min)
}

/// UI-57 width ramp: 1.0 at the `compact` breakpoint (900), tapering linearly
/// to the 0.7 floor at `narrow` (640), clamped on both sides so resizing or
/// crossing a breakpoint never jumps. Continuous by construction.
#[inline]
pub fn width_scale(width: f32) -> f32 {
    let ramp = WIDTH_SCALE_FLOOR
        + (1.0 - WIDTH_SCALE_FLOOR) * ((width - NARROW_WIDTH) / (COMPACT_WIDTH - NARROW_WIDTH));
    clamp(ramp, WIDTH_SCALE_FLOOR, 1.0)
}

/// `Theme.fontSize(role, referenceHeight)`: a proportional fraction of the
/// reference height with the role's pixel clamps (R7).
///
/// Roles match `Theme.type`: `display`, `headline`, `title`, `body`, `label`,
/// `mono`. An unknown role falls back to the legacy UI behaviour, `reference * 0.02`
/// with no clamp.
pub fn font_size(role: &str, reference_height: f32) -> f32 {
    let (fraction, min, max) = match role {
        "display" => (0.055, 22.0, 44.0),
        "headline" => (0.04, 18.0, 32.0),
        "title" => (0.032, 14.0, 24.0),
        "body" => (0.024, 12.0, 18.0),
        "label" => (0.02, 10.0, 14.0),
        "mono" => (0.02, 10.0, 14.0),
        _ => return reference_height * 0.02,
    };
    clamp(reference_height * fraction, min, max)
}

/// `Theme.rowFont(role, width, height)`: one row type ramp for every row body
/// (library, queue, search, album tracklists). Height sets the base; width
/// caps it so narrow search columns and the queue do not inherit the wide
/// list size (UI-58).
///
/// Roles: `title` (.28h/.05w, 10..24), `subtitle` (.20h/.032w, 9..17),
/// `meta` (.20h/.028w, 9..15). An unknown role mirrors legacy UI:
/// `clamp(height * 0.20, 9, 18)`.
pub fn row_font(role: &str, view_width: f32, row_height: f32) -> f32 {
    let (height_fraction, width_fraction, min, max) = match role {
        "title" => (0.28, 0.05, 10.0, 24.0),
        "subtitle" => (0.20, 0.032, 9.0, 17.0),
        "meta" => (0.20, 0.028, 9.0, 15.0),
        _ => return clamp(row_height * 0.20, 9.0, 18.0),
    };
    clamp(
        (row_height * height_fraction).min(view_width * width_fraction),
        min,
        max,
    )
}

/// UI-58 adaptive rows: a row may lose at most 18% of its baseline
/// (default-window) height; past that it stops shrinking instead of trading a
/// whole row. `ratio` is the host's nominal row fraction of its reference;
/// `baseline` is the row height captured on the first real measure (0 means
/// "not captured yet", which disables the floor).
///
/// Degenerate, non-positive viewports return the nominal value untouched,
/// exactly like the legacy UI helper.
#[inline]
pub fn adaptive_row(view_height: f32, ratio: f32, baseline: f32) -> f32 {
    let nominal = view_height * ratio;
    if nominal <= 0.0 || view_height <= 0.0 {
        return nominal;
    }
    let floor = if baseline > 0.0 {
        BASELINE_FLOOR * baseline
    } else {
        0.0
    };
    nominal.max(floor)
}

/// One context profile's row scaling policy (`ScrollProfiles.js`).
///
/// Only the metrics fields live here; keyboard flags stay with the GPUI
/// surface. `row_height_scale` is applied after the adaptive floor, matching
/// `ScrollContent.adaptiveRowFor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollProfile {
    /// Nominal row fraction of the profile's reference height.
    pub row_ratio: f32,
    /// Multiplier applied after the adaptive row (search rows get 1.04).
    pub row_height_scale: f32,
}

impl ScrollProfile {
    /// Library lists: 10.5% of the panel body.
    pub const LIBRARY: Self = Self {
        row_ratio: 0.105,
        row_height_scale: 1.0,
    };
    /// Album/artist/playlist page body: 5.5% of the page height.
    pub const PAGE: Self = Self {
        row_ratio: 0.055,
        row_height_scale: 1.0,
    };
    /// Page tracklists use the denser 6% step (`PageEntry.legacy`).
    pub const PAGE_TRACKLIST: Self = Self {
        row_ratio: 0.06,
        row_height_scale: 1.0,
    };
    /// Search overlay: 12% of the panel, then +4% (UI-58's `rowHeightScale`).
    pub const SEARCH: Self = Self {
        row_ratio: 0.12,
        row_height_scale: 1.04,
    };
    /// Queue drawer: 10% of the drawer height.
    pub const QUEUE: Self = Self {
        row_ratio: 0.10,
        row_height_scale: 1.0,
    };

    /// Build a profile for ratios the named constants do not cover.
    pub const fn new(row_ratio: f32, row_height_scale: f32) -> Self {
        Self {
            row_ratio,
            row_height_scale,
        }
    }

    /// The page profile, switched to the denser tracklist step.
    pub const fn page(tracklist: bool) -> Self {
        if tracklist {
            Self::PAGE_TRACKLIST
        } else {
            Self::PAGE
        }
    }
}

/// Per-surface row metric with a baseline captured once (UI-58).
///
/// `ScrollContent._rowBase` and `SearchColumn._rowSizingBase` are metrics, not
/// interactive state: they are captured from the first stable reference and a
/// resize never recaptures them, so every row keeps the same 18%-shrink floor
/// for the life of the surface.
#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceMetrics {
    baseline: f32,
}

impl SurfaceMetrics {
    pub const fn new() -> Self {
        Self { baseline: 0.0 }
    }

    /// The captured baseline row height (0 before the first real measure).
    pub const fn baseline(&self) -> f32 {
        self.baseline
    }

    /// Captures the baseline on the first real measure only. A zero/negative
    /// reference is not a real measure and leaves the metric uncaptured.
    ///
    /// Returns the baseline in force after the call, which is the newly
    /// captured one or the pre-existing one.
    pub fn capture(&mut self, profile: ScrollProfile, reference: f32) -> f32 {
        if self.baseline <= 0.0 && reference > 0.0 {
            self.baseline = reference * profile.row_ratio;
        }
        self.baseline
    }

    /// `ScrollContent.adaptiveRowHeight`: the adaptive row scaled by the
    /// profile, rounded to a whole pixel (Qt's content coordinate is
    /// integer-based) and never below the 18 px floor.
    ///
    /// The first call captures the baseline; later calls ignore the reference
    /// for the floor and only use it for the nominal height.
    pub fn row_height(&mut self, profile: ScrollProfile, reference: f32) -> f32 {
        self.capture(profile, reference);
        let height =
            adaptive_row(reference, profile.row_ratio, self.baseline) * profile.row_height_scale;
        height.round().max(MIN_ROW_HEIGHT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn clamp_keeps_the_legacy_order() {
        assert_eq!(clamp(5.0, 0.0, 10.0), 5.0);
        assert_eq!(clamp(-3.0, 0.0, 10.0), 0.0);
        assert_eq!(clamp(13.0, 0.0, 10.0), 10.0);
        assert_eq!(clamp(0.0, 0.0, 0.0), 0.0);
        assert!(clamp(f32::NAN, 0.0, 10.0).is_nan());
        assert!(clamp(5.0, f32::NAN, 10.0).is_nan());
        // Inverted bounds follow Math.max(min, Math.min(max, value)).
        assert_eq!(clamp(7.0, 10.0, 5.0), 10.0);
        assert_eq!(clamp(20.0, 10.0, 5.0), 10.0);
    }

    #[test]
    fn width_scale_ramps_640_to_900() {
        assert_close(width_scale(NARROW_WIDTH), 0.7);
        assert_close(width_scale(COMPACT_WIDTH), 1.0);
        // Midpoint of the ramp.
        assert_close(width_scale(770.0), 0.85);
        // Clamped both sides.
        assert_close(width_scale(300.0), 0.7);
        assert_close(width_scale(1600.0), 1.0);
        // Continuous across the upper breakpoint (never jumps past 1.0).
        assert_close(width_scale(899.0), 0.7 + 0.3 * (259.0 / 260.0));
        assert!(width_scale(899.0) < width_scale(900.0));
    }

    #[test]
    fn font_size_fractions_and_clamps() {
        // display: 0.055 / 22..44
        assert_close(font_size("display", 1000.0), 44.0);
        assert_close(font_size("display", 500.0), 27.5);
        assert_close(font_size("display", 100.0), 22.0);
        // headline: 0.04 / 18..32
        assert_close(font_size("headline", 1000.0), 32.0);
        assert_close(font_size("headline", 100.0), 18.0);
        // title: 0.032 / 14..24
        assert_close(font_size("title", 1000.0), 24.0);
        assert_close(font_size("title", 300.0), 14.0);
        // body: 0.024 / 12..18
        assert_close(font_size("body", 600.0), 14.4);
        assert_close(font_size("body", 1000.0), 18.0);
        // label + mono: 0.02 / 10..14
        assert_close(font_size("label", 1000.0), 14.0);
        assert_close(font_size("label", 200.0), 10.0);
        assert_close(font_size("mono", 500.0), 10.0);
        // Unknown role keeps the legacy UI fallback (unclamped 0.02).
        assert_close(font_size("unknown", 400.0), 8.0);
    }

    #[test]
    fn row_font_height_base_width_cap_and_clamps() {
        // title: min(.28h, .05w), 10..24
        assert_close(row_font("title", 1000.0, 1000.0), 24.0);
        assert_close(row_font("title", 100.0, 100.0), 10.0);
        assert_close(row_font("title", 1000.0, 30.0), 10.0);
        // subtitle: min(.20h, .032w), 9..17
        assert_close(row_font("subtitle", 1000.0, 1000.0), 17.0);
        assert_close(row_font("subtitle", 100.0, 100.0), 9.0);
        // meta: min(.20h, .028w), 9..15
        assert_close(row_font("meta", 1000.0, 1000.0), 15.0);
        assert_close(row_font("meta", 100.0, 100.0), 9.0);
        // Hierarchy holds inside one row box.
        let (width, height) = (1000.0, 600.0);
        assert!(row_font("title", width, height) > row_font("subtitle", width, height));
        assert!(row_font("subtitle", width, height) > row_font("meta", width, height));
        // Unknown role: clamp(height * 0.20, 9, 18).
        assert_close(row_font("unknown", 1000.0, 100.0), 18.0);
        assert_close(row_font("unknown", 1000.0, 10.0), 9.0);
    }

    #[test]
    fn adaptive_row_floor_is_82_percent_of_baseline() {
        assert_close(adaptive_row(1000.0, 0.105, 100.0), 105.0);
        assert_close(adaptive_row(500.0, 0.105, 100.0), 82.0);
        // Exactly at the floor, then below it: the floor holds.
        assert_close(adaptive_row(1000.0, 0.082, 100.0), 82.0);
        assert_close(adaptive_row(300.0, 0.105, 100.0), 82.0);
        // No baseline captured yet: pure nominal.
        assert_close(adaptive_row(500.0, 0.105, 0.0), 52.5);
        // Degenerate viewport returns the nominal untouched (legacy UI parity).
        assert_close(adaptive_row(0.0, 0.105, 100.0), 0.0);
    }

    #[test]
    fn surface_baseline_is_captured_once_and_never_recaptured() {
        let mut metrics = SurfaceMetrics::new();
        assert_eq!(metrics.baseline(), 0.0);
        // First real measure: library rows are 10.5% of an 800 px body.
        assert_eq!(metrics.row_height(ScrollProfile::LIBRARY, 800.0), 84.0);
        assert_close(metrics.baseline(), 84.0);

        // Shrinking the window uses the floor from the original baseline:
        // max(42, 0.82 * 84) = 68.88 -> 69, not 42.
        assert_eq!(metrics.row_height(ScrollProfile::LIBRARY, 400.0), 69.0);
        assert_close(metrics.baseline(), 84.0);

        // A later explicit capture (e.g. a resize callback) is a no-op.
        assert_close(metrics.capture(ScrollProfile::LIBRARY, 1200.0), 84.0);
        // Growing back restores the plain nominal row.
        assert_eq!(metrics.row_height(ScrollProfile::LIBRARY, 1200.0), 126.0);

        // A zero reference is not a real measure: it must not capture.
        let mut pending = SurfaceMetrics::new();
        assert_eq!(pending.row_height(ScrollProfile::PAGE, 0.0), 18.0);
        assert_eq!(pending.baseline(), 0.0);
        assert_eq!(pending.row_height(ScrollProfile::PAGE, 1000.0), 55.0);
    }

    #[test]
    fn row_height_rounds_to_whole_pixels() {
        // 641 * 0.055 = 35.255 -> 35.
        let mut page = SurfaceMetrics::new();
        assert_eq!(page.row_height(ScrollProfile::PAGE, 641.0), 35.0);

        // Search applies the 1.04 scale after the floor: 120 * 1.04 = 124.8 -> 125.
        let mut search = SurfaceMetrics::new();
        assert_eq!(search.row_height(ScrollProfile::SEARCH, 1000.0), 125.0);
        // 500 px: max(60, 0.82*120) * 1.04 = 102.336 -> 102.
        assert_eq!(search.row_height(ScrollProfile::SEARCH, 500.0), 102.0);

        // The 18 px minimum applies after rounding: 100 * 0.12 * 1.04 = 12.48.
        let mut tiny = SurfaceMetrics::new();
        assert_eq!(tiny.row_height(ScrollProfile::SEARCH, 100.0), 18.0);

        // Integers stay exact (no double rounding).
        assert_eq!(tiny.row_height(ScrollProfile::LIBRARY, 1000.0), 105.0);
    }

    #[test]
    fn scroll_profiles_match_the_legacy_table() {
        assert_close(ScrollProfile::LIBRARY.row_ratio, 0.105);
        assert_close(ScrollProfile::LIBRARY.row_height_scale, 1.0);

        assert_close(ScrollProfile::PAGE.row_ratio, 0.055);
        assert_close(ScrollProfile::PAGE_TRACKLIST.row_ratio, 0.06);
        assert_eq!(ScrollProfile::page(false), ScrollProfile::PAGE);
        assert_eq!(ScrollProfile::page(true), ScrollProfile::PAGE_TRACKLIST);
        assert_close(ScrollProfile::page(false).row_ratio, 0.055);

        assert_close(ScrollProfile::SEARCH.row_ratio, 0.12);
        assert_close(ScrollProfile::SEARCH.row_height_scale, 1.04);

        assert_close(ScrollProfile::QUEUE.row_ratio, 0.10);
        assert_close(ScrollProfile::QUEUE.row_height_scale, 1.0);

        assert_eq!(
            ScrollProfile::new(0.2, 1.1),
            ScrollProfile {
                row_ratio: 0.2,
                row_height_scale: 1.1
            }
        );
    }
}
