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
/// Raised from the original 18 px floor to 30 px.
pub const MIN_ROW_HEIGHT: f32 = 30.0;
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
    /// integer-based) and never below the raised 30 px floor.
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

/// Responsive geometry for the inferior playback bar.
///
/// Mirrors `InferiorBar`: the bar height is the only reference. Every child
/// size below is the 1200x800 design value multiplied by this bar's own scale,
/// so the transport, cover, metadata, padding and meters all grow and shrink
/// with the bar instead of with fixed pixel floors or the window width. The
/// left/right zones still use equal flex space in the bar itself; these are
/// only the in-zone sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerBarMetrics {
    pub bar_h: f32,
    pub scale: f32,
    pub title_size: f32,
    pub artist_size: f32,
    pub time_size: f32,
    pub cover_side: f32,
    pub cover_radius: f32,
    pub transport_unit: f32,
    pub play_size: f32,
    pub transport_spacing: f32,
    pub volume_bar: f32,
    pub volume_h: f32,
    pub progress_h: f32,
    pub padding_x: f32,
    pub gap: f32,
    pub eq_icon: f32,
}

impl PlayerBarMetrics {
    pub fn new(window_height: f32) -> Self {
        let bar_h = player_bar_height(window_height);
        let scale = bar_h / PLAYER_BAR_REFERENCE_HEIGHT;
        let title_size = 16.0 * scale;
        let time_size = 10.0 * scale;
        let transport_unit = 30.4704 * scale;
        Self {
            bar_h,
            scale,
            title_size,
            artist_size: title_size,
            time_size,
            cover_side: 41.0688 * scale,
            cover_radius: 4.0 * scale,
            transport_unit,
            play_size: transport_unit * 1.22,
            transport_spacing: transport_unit * 0.11,
            volume_bar: 84.0 * scale,
            volume_h: 3.0 * scale,
            progress_h: 3.0 * scale,
            padding_x: 12.0 * scale,
            gap: 12.0 * scale,
            eq_icon: 13.5 * scale,
        }
    }
}

/// Responsive geometry for the top chrome band.
///
/// The same rule as [`PlayerBarMetrics`]: the bar height is the parent scale.
/// Every label, glyph, slot, chip and inset is its 1200x800 reference value
/// multiplied by that scale. The bar height itself is `TOP_BAR_FRACTION` with
/// the shared [`MIN_BAR_SCALE`] clamp, so its children can never outgrow it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopBarMetrics {
    pub bar_h: f32,
    pub scale: f32,
    pub padding_x: f32,
    pub padding_y: f32,
    pub gap: f32,
    pub gap_wide: f32,
    pub title_size: f32,
    pub body_size: f32,
    pub icon_size: f32,
    pub icon_pad: f32,
    pub action_radius: f32,
    pub badge_size: f32,
    pub badge_pad_x: f32,
    pub badge_pad_y: f32,
    pub star_size: f32,
    pub insight_slot: f32,
    pub insight_icon: f32,
    pub insight_text: f32,
    pub insight_small: f32,
    pub chip_label_size: f32,
    pub chip_value_size: f32,
    pub sync_bar_w: f32,
    pub sync_bar_h: f32,
}

impl TopBarMetrics {
    pub fn new(window_height: f32) -> Self {
        let bar_h = top_bar_height(window_height);
        let scale = bar_h / TOP_BAR_REFERENCE_HEIGHT;
        Self {
            bar_h,
            scale,
            padding_x: 16.0 * scale,
            padding_y: 4.0 * scale,
            gap: 8.0 * scale,
            gap_wide: 12.0 * scale,
            title_size: 19.0 * scale,
            body_size: 14.0 * scale,
            icon_size: 18.0 * scale,
            icon_pad: 3.0 * scale,
            action_radius: 6.0 * scale,
            badge_size: 10.0 * scale,
            badge_pad_x: 8.0 * scale,
            badge_pad_y: 4.0 * scale,
            star_size: 13.0 * scale,
            insight_slot: 18.0 * scale,
            insight_icon: 16.0 * scale,
            insight_text: 13.0 * scale,
            insight_small: 11.0 * scale,
            chip_label_size: 9.0 * scale,
            chip_value_size: 12.0 * scale,
            sync_bar_w: 2.0 * scale,
            sync_bar_h: bar_h * 0.16,
        }
    }
}

/// The Alt cross spans five list rows. That row-derived frame is the whole
/// scale; it does not need a window-derived option ramp.
pub const ALT_ARM_ROWS: f32 = 5.0;
/// The Alt board never paints a tile smaller than this.
pub const MIN_ALT_TILE: f32 = 18.0;
/// Soft upper bound for a tile on very tall rows.
pub const MAX_ALT_TILE: f32 = 24.0;
/// Soft upper bound for the whole cross, so 4K rows do not spread it across
/// the screen.
pub const MAX_ALT_FRAME: f32 = 360.0;

/// Geometry for the Alt board's cross of action tiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AltBoardMetrics {
    /// Whole cross side: five list rows.
    pub frame_side: f32,
    /// Fixed arm slot used for centering (does not animate).
    pub slot_side: f32,
    /// Painted tile; the spawn animation scales this around the slot centre.
    pub tile_side: f32,
}

/// Alt-board geometry derived from one list row. The frame is exactly five
/// rows (soft-clamped to a visible minimum and a sane maximum); each arm slot
/// is an eighth of that frame, and the tiles spawn by scaling in place so the
/// cross never drifts off its anchor.
pub fn alt_board_metrics(row_h: f32, progress: f32) -> AltBoardMetrics {
    let row_h = row_h.max(1.0);
    let frame_side = (row_h * ALT_ARM_ROWS).clamp(MIN_ALT_TILE * 6.0, MAX_ALT_FRAME);
    // The tile rectangle and its glyph both use half the previous arm scale.
    let slot_side = (frame_side / 8.0).clamp(MIN_ALT_TILE, MAX_ALT_TILE);
    let spawned = 0.72 + 0.28 * progress;
    AltBoardMetrics {
        frame_side,
        slot_side,
        tile_side: slot_side * spawned,
    }
}

/// The top chrome band as a fraction of the client height.
pub const TOP_BAR_FRACTION: f32 = 0.0648;
/// The inferior playback bar as a fraction of the client height.
pub const PLAYER_BAR_FRACTION: f32 = 0.0828;
/// Page side margins as a fraction of the client width.
pub const PAGE_MARGIN_FRACTION: f32 = 0.02;
/// The stacked hero may use at most this fraction of the page body height.
pub const PAGE_HERO_MAX_FRACTION: f32 = 0.75;
/// Page list padding as a fraction of the body height.
pub const PAGE_LIST_PAD_FRACTION: f32 = 0.012;
/// Width of the reference window used to calibrate the page table.
pub const REFERENCE_WIDTH: f32 = 1200.0;
/// Height of the reference window used to calibrate the page table.
pub const REFERENCE_HEIGHT: f32 = 800.0;
/// Child-size calibration for the top bar at the reference window.
pub const TOP_BAR_REFERENCE_HEIGHT: f32 = REFERENCE_HEIGHT * TOP_BAR_FRACTION;
/// Child-size calibration for the player bar at the reference window.
pub const PLAYER_BAR_REFERENCE_HEIGHT: f32 = REFERENCE_HEIGHT * PLAYER_BAR_FRACTION;
/// The chrome never gets thinner than this share of its reference height. Short
/// windows keep legible bars and every child inherits the same scale; below
/// this the body would be the part that shrinks.
pub const MIN_BAR_SCALE: f32 = 0.9;

/// Top-bar height for a client height: the usual fraction, clamped so a short
/// window cannot make the bar (or its children) arbitrarily thin.
pub fn top_bar_height(window_height: f32) -> f32 {
    (window_height * TOP_BAR_FRACTION).max(TOP_BAR_REFERENCE_HEIGHT * MIN_BAR_SCALE)
}

/// Player-bar height for a client height, with the same minimum as the top bar.
pub fn player_bar_height(window_height: f32) -> f32 {
    (window_height * PLAYER_BAR_FRACTION).max(PLAYER_BAR_REFERENCE_HEIGHT * MIN_BAR_SCALE)
}

/// The window may not shrink below this share of the reference size. At
/// 480x320 the page still has a split cover pane; below it the cover would be
/// the first thing lost.
pub const MIN_WINDOW_FRACTION: f32 = 0.4;
/// Smallest window width that keeps the page cover available.
pub const MIN_WINDOW_WIDTH: f32 = REFERENCE_WIDTH * MIN_WINDOW_FRACTION;
/// Smallest window height that keeps the page cover available.
pub const MIN_WINDOW_HEIGHT: f32 = REFERENCE_HEIGHT * MIN_WINDOW_FRACTION;
/// Below this body height the stacked page switches to the split layout.
pub const SPLIT_BODY_RATIO: f32 = 0.72;
/// Below this body height even the split row pane is discarded.
pub const ROWS_ONLY_BODY_RATIO: f32 = 0.30;
/// Extra horizontal room, as a share of the stacked cover's own width, before
/// the page stops stacking and puts the rows beside the cover. At 130% over
/// the cover there is room for both without shrinking either.
pub const WIDE_SPLIT_HEADROOM: f32 = 1.3;
/// Cover pane share in the split layout.
pub const SPLIT_COVER_FRACTION: f32 = 0.40;
/// Gap between the cover and list panes in the split layout.
pub const SPLIT_GAP_FRACTION: f32 = 0.015;
/// Minimum row-pane width for the optional wide split. A short window always
/// splits instead: it has no vertical room to stack a cover, so preserving the
/// cover and a narrower list beats dropping the cover.
pub const MIN_ROWS_WIDTH: f32 = 280.0;

/// The three vertical bands every shell surface shares: top bar, body and
/// player bar. The body is the remainder, never a separately guessed reserve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowLayout {
    pub width: f32,
    pub height: f32,
    pub top_bar_h: f32,
    pub player_bar_h: f32,
    pub body_h: f32,
}

impl WindowLayout {
    pub fn new(width: f32, height: f32) -> Self {
        let width = width.max(1.0);
        let height = height.max(1.0);
        let top_bar_h = top_bar_height(height);
        let player_bar_h = player_bar_height(height);
        let body_h = (height - top_bar_h - player_bar_h).max(1.0);
        Self {
            width,
            height,
            top_bar_h,
            player_bar_h,
            body_h,
        }
    }

    pub fn reference() -> Self {
        Self::new(REFERENCE_WIDTH, REFERENCE_HEIGHT)
    }

    pub fn page(&self) -> PageLayout {
        PageLayout::new(*self)
    }
}

/// How the page shares its body between the hero and the row list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMode {
    Stacked,
    Split,
    RowsOnly,
}

/// One page geometry snapshot. Every value is derived from the window and the
/// shared fractions above; views paint this, they do not re-derive pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageLayout {
    pub mode: PageMode,
    pub width: f32,
    pub body_h: f32,
    pub margin_x: f32,
    pub margin_y: f32,
    pub gap: f32,
    /// Stacked hero art side.
    pub hero_art: f32,
    /// Stacked cover + bottom margin; also the row-list start.
    pub header_h: f32,
    /// Split cover pane width.
    pub cover_w: f32,
    pub list_w: f32,
    pub list_h: f32,
    pub list_pad: f32,
}

impl PageLayout {
    pub fn reference() -> Self {
        WindowLayout::reference().page()
    }

    pub fn new(window: WindowLayout) -> Self {
        let width = window.width;
        let body_h = window.body_h;
        let margin_x = (width * PAGE_MARGIN_FRACTION).max(1.0);
        let margin_y = margin_x;
        let gap = (width * SPLIT_GAP_FRACTION).max(1.0);
        let list_pad = (body_h * PAGE_LIST_PAD_FRACTION).clamp(4.0, 12.0);

        let max_cover = ((body_h - 2.0 * margin_x) * PAGE_HERO_MAX_FRACTION).max(1.0);
        let cover_side = width.min(body_h).min(max_cover).max(1.0);
        let hero_art = (cover_side - 2.0 * margin_x).max(1.0);
        let header_h = (hero_art + margin_y).max(1.0);
        let stacked_list_h = (body_h - header_h).max(1.0);

        // Scale the cover with the space the body actually leaves for the
        // rows; a hard minimum fraction used to make it jump when short.
        let split_cover_w = (width * SPLIT_COVER_FRACTION)
            .min((body_h - 2.0 * margin_y).max(1.0))
            .max(1.0);
        let split_list_w = (width - split_cover_w - 2.0 * margin_x - gap).max(1.0);
        let split_list_h = (body_h - 2.0 * margin_y).max(1.0);

        let reference_body = WindowLayout::reference().body_h;
        let reference_hero = PageLayout::stacked_hero_art(WindowLayout::reference());
        let split_fits = split_list_w >= MIN_ROWS_WIDTH;
        // Width-only decision: once the client has 130% more lateral room than
        // the stacked cover needs, keep the cover and put the rows beside it.
        // This replaces the old absolute breakpoint, so a wide-but-short window
        // uses the same lateral layout as a narrow one.
        let wide_split = split_fits && width > cover_side * (1.0 + WIDE_SPLIT_HEADROOM);
        let short_split = body_h < SPLIT_BODY_RATIO * reference_body
            || hero_art < 0.5 * reference_hero
            || stacked_list_h < 3.0 * MIN_ROW_HEIGHT;
        // Width never suppresses the cover on its own. A tall, narrow window
        // stacks the cover and accepts fewer visible rows. A short window still
        // splits whenever the row pane has room, because the cover can sit in
        // the otherwise idle horizontal space; only a window that is both short
        // and too narrow for two panes falls back to rows-only.
        let rows_only = body_h < ROWS_ONLY_BODY_RATIO * reference_body && !split_fits;

        let mode = if rows_only {
            PageMode::RowsOnly
        } else if wide_split || short_split {
            PageMode::Split
        } else {
            PageMode::Stacked
        };

        let list_h = match mode {
            PageMode::Stacked => stacked_list_h,
            PageMode::Split => split_list_h,
            PageMode::RowsOnly => body_h,
        };
        let list_w = match mode {
            PageMode::Stacked | PageMode::RowsOnly => width,
            PageMode::Split => split_list_w,
        };

        Self {
            mode,
            width,
            body_h,
            margin_x,
            margin_y,
            gap,
            hero_art,
            header_h,
            cover_w: split_cover_w,
            list_w,
            list_h,
            list_pad,
        }
    }

    fn stacked_hero_art(window: WindowLayout) -> f32 {
        let width = window.width;
        let body_h = window.body_h;
        let margin_x = (width * PAGE_MARGIN_FRACTION).max(1.0);
        let max_cover = ((body_h - 2.0 * margin_x) * PAGE_HERO_MAX_FRACTION).max(1.0);
        let cover_side = width.min(body_h).min(max_cover).max(1.0);
        (cover_side - 2.0 * margin_x).max(1.0)
    }

    /// The art side actually painted: square in stacked, bounded by the pane in
    /// split, zero when the list owns the body.
    pub fn cover_art(&self) -> f32 {
        match self.mode {
            PageMode::Stacked => self.hero_art,
            PageMode::Split => self.cover_w.min(self.list_h).max(1.0),
            PageMode::RowsOnly => 0.0,
        }
    }

    /// Height reference for adaptive page-row sizing.
    ///
    /// Row density follows the page body, not the remaining list pane. The
    /// stacked cover must not make the rows smaller than the reference layout;
    /// it changes how many rows are visible, not their stride.
    pub fn row_reference(&self) -> f32 {
        self.body_h.max(1.0)
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
    fn window_layout_sums_the_three_bands() {
        let layout = WindowLayout::reference();
        assert_close(
            layout.top_bar_h + layout.player_bar_h + layout.body_h,
            layout.height,
        );
        assert_close(layout.top_bar_h, 51.84);
        assert_close(layout.player_bar_h, 66.24);
        assert_close(layout.body_h, 681.92);
    }

    #[test]
    fn reference_page_uses_the_lateral_split() {
        let layout = PageLayout::reference();
        assert_eq!(layout.mode, PageMode::Split);
        assert_close(layout.margin_x, 24.0);
        assert_close(layout.hero_art, 427.44);
        assert_close(layout.header_h, 451.44);
        assert_close(layout.cover_w, 480.0);
        assert_close(layout.list_w, 654.0);
        assert_close(layout.list_h, 633.92);
        assert_close(layout.list_pad, 8.18304);
    }

    #[test]
    fn minimum_window_still_has_a_cover_pane_and_clamped_bars() {
        assert_close(MIN_WINDOW_WIDTH, 480.0);
        assert_close(MIN_WINDOW_HEIGHT, 320.0);
        let window = WindowLayout::new(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT);
        assert_close(window.top_bar_h, TOP_BAR_REFERENCE_HEIGHT * MIN_BAR_SCALE);
        assert_close(
            window.player_bar_h,
            PLAYER_BAR_REFERENCE_HEIGHT * MIN_BAR_SCALE,
        );
        let layout = window.page();
        assert_ne!(layout.mode, PageMode::RowsOnly);
        assert!(layout.cover_art() > 0.0);
        assert!(layout.list_w > 0.0);
    }

    #[test]
    fn page_modes_cover_short_and_wide_windows() {
        assert_eq!(
            WindowLayout::new(1200.0, 800.0).page().mode,
            PageMode::Split
        );
        assert_eq!(WindowLayout::new(912.0, 506.0).page().mode, PageMode::Split);
        assert_eq!(WindowLayout::new(669.0, 332.0).page().mode, PageMode::Split);
        assert_eq!(
            WindowLayout::new(1920.0, 1132.0).page().mode,
            PageMode::Split
        );
        // A tall, narrow page keeps its cover: list width alone must not
        // demote the page to rows-only.
        assert_eq!(
            WindowLayout::new(505.0, 909.0).page().mode,
            PageMode::Stacked
        );
        // The lateral split is width-relative now, not an absolute breakpoint:
        // 800px leaves more than 130% headroom beside a 334px cover, 750px
        // does not.
        assert_eq!(WindowLayout::new(800.0, 600.0).page().mode, PageMode::Split);
        assert_eq!(
            WindowLayout::new(750.0, 600.0).page().mode,
            PageMode::Stacked
        );
        // A short window splits even when the row pane is under the optional
        // wide-split width, preserving the cover instead of hiding it.
        assert_eq!(WindowLayout::new(480.0, 320.0).page().mode, PageMode::Split);
        // A very short window is still wide enough for a cover pane; rows-only
        // is for the corner case where both axes are cramped.
        assert_eq!(WindowLayout::new(737.0, 190.0).page().mode, PageMode::Split);
        assert_eq!(
            WindowLayout::new(320.0, 240.0).page().mode,
            PageMode::RowsOnly
        );
    }

    #[test]
    fn short_wide_page_keeps_a_cover_pane() {
        let layout = WindowLayout::new(737.0, 190.0).page();
        assert_eq!(layout.mode, PageMode::Split);
        assert!(layout.cover_art() > 0.0);
        assert!(layout.list_w >= MIN_ROWS_WIDTH);
        assert!(layout.list_h > 0.0);
    }

    #[test]
    fn tall_narrow_page_keeps_a_usable_stacked_cover() {
        let layout = WindowLayout::new(505.0, 909.0).page();
        assert_eq!(layout.mode, PageMode::Stacked);
        assert!(layout.cover_art() > layout.width * 0.9);
        assert!(layout.list_h >= 3.0 * MIN_ROW_HEIGHT);
        // Row density stays tied to the page body rather than the list pane.
        assert_close(layout.row_reference(), layout.body_h);
        assert_close(
            PageLayout::reference().row_reference(),
            WindowLayout::reference().body_h,
        );
    }

    #[test]
    fn split_geometry_shares_the_row_and_keeps_a_usable_list() {
        for (width, height) in [
            (669.0, 332.0),
            (912.0, 506.0),
            (1440.0, 900.0),
            (1920.0, 1132.0),
        ] {
            let layout = WindowLayout::new(width, height).page();
            assert_eq!(layout.mode, PageMode::Split, "{width}x{height}");
            assert!(layout.cover_w < width * 0.5, "{width}x{height}");
            assert!(layout.list_w >= MIN_ROWS_WIDTH, "{width}x{height}");
            assert!(layout.list_h > 0.0, "{width}x{height}");
            assert!(layout.row_reference() > 0.0, "{width}x{height}");
        }
    }

    #[test]
    fn tiny_windows_do_not_produce_negative_segments() {
        for (width, height) in [(320.0, 240.0), (480.0, 320.0), (900.0, 600.0)] {
            let window = WindowLayout::new(width, height);
            let layout = window.page();
            assert!(window.body_h > 0.0);
            assert!(layout.list_w > 0.0);
            assert!(layout.list_h > 0.0);
            assert!(layout.list_pad >= 4.0);
        }
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
        assert_eq!(pending.row_height(ScrollProfile::PAGE, 0.0), MIN_ROW_HEIGHT);
        assert_eq!(pending.baseline(), 0.0);
        assert_eq!(pending.row_height(ScrollProfile::PAGE, 1000.0), 55.0);
    }

    #[test]
    fn row_height_rounds_to_whole_pixels() {
        assert_close(MIN_ROW_HEIGHT, 30.0);
        // 641 * 0.055 = 35.255 -> 35.
        let mut page = SurfaceMetrics::new();
        assert_eq!(page.row_height(ScrollProfile::PAGE, 641.0), 35.0);

        // Search applies the 1.04 scale after the floor: 120 * 1.04 = 124.8 -> 125.
        let mut search = SurfaceMetrics::new();
        assert_eq!(search.row_height(ScrollProfile::SEARCH, 1000.0), 125.0);
        // 500 px: max(60, 0.82*120) * 1.04 = 102.336 -> 102.
        assert_eq!(search.row_height(ScrollProfile::SEARCH, 500.0), 102.0);

        // The raised 30 px minimum applies after rounding:
        // 100 * 0.12 * 1.04 = 12.48.
        let mut tiny = SurfaceMetrics::new();
        assert_eq!(
            tiny.row_height(ScrollProfile::SEARCH, 100.0),
            MIN_ROW_HEIGHT
        );

        // Integers stay exact (no double rounding).
        assert_eq!(tiny.row_height(ScrollProfile::LIBRARY, 1000.0), 105.0);
    }

    #[test]
    fn alt_board_scales_from_the_row_height() {
        // A normal 57 px row makes the cross exactly five rows wide.
        let board = alt_board_metrics(57.0, 1.0);
        assert_close(board.frame_side, 57.0 * ALT_ARM_ROWS);
        assert!(board.slot_side >= MIN_ALT_TILE);
        assert!(board.slot_side <= MAX_ALT_TILE);
        assert!(board.frame_side >= 3.0 * board.slot_side);
        assert_close(board.tile_side, board.slot_side);

        // The 30 px row floor still yields a visible tile.
        let minimum = alt_board_metrics(MIN_ROW_HEIGHT, 1.0);
        assert_close(minimum.frame_side, MIN_ROW_HEIGHT * ALT_ARM_ROWS);
        assert_close(minimum.slot_side, MIN_ROW_HEIGHT * ALT_ARM_ROWS / 8.0);
        assert!(minimum.slot_side >= MIN_ALT_TILE);
        assert_close(minimum.tile_side, minimum.slot_side);

        // Tall rows grow the cross, but both tile and frame stay soft-clamped.
        let large = alt_board_metrics(100.0, 1.0);
        assert_close(large.frame_side, MAX_ALT_FRAME);
        assert_close(large.slot_side, MAX_ALT_TILE);

        // The spawn animates arm size in place; the frame/slots do not move.
        let spawning = alt_board_metrics(57.0, 0.0);
        assert_close(spawning.frame_side, board.frame_side);
        assert_close(spawning.slot_side, board.slot_side);
        assert!(spawning.tile_side < board.tile_side);
    }

    #[test]
    fn player_bar_children_share_the_bar_scale() {
        let reference = PlayerBarMetrics::new(REFERENCE_HEIGHT);
        assert_close(reference.bar_h, PLAYER_BAR_REFERENCE_HEIGHT);
        assert_close(reference.scale, 1.0);
        assert_close(reference.title_size, 16.0);
        assert_close(reference.time_size, 10.0);
        assert_close(reference.cover_side, 41.0688);
        assert_close(reference.transport_unit, 30.4704);
        assert_close(reference.volume_bar, 84.0);
        assert_close(reference.padding_x, 12.0);

        // Above the clamp every child still tracks the parent bar exactly.
        let tall = PlayerBarMetrics::new(REFERENCE_HEIGHT * 1.25);
        assert_close(tall.scale, 1.25);
        assert_close(tall.transport_unit, reference.transport_unit * 1.25);
        assert_close(tall.title_size, reference.title_size * 1.25);
        assert_close(tall.cover_side, reference.cover_side * 1.25);
        assert_close(tall.volume_bar, reference.volume_bar * 1.25);
        assert_close(tall.eq_icon, reference.eq_icon * 1.25);

        // A short window stops shrinking the bar at 90% of its reference
        // height, and every child inherits that clamp.
        let minimum = PlayerBarMetrics::new(MIN_WINDOW_HEIGHT);
        assert_close(minimum.scale, MIN_BAR_SCALE);
        assert_close(minimum.bar_h, PLAYER_BAR_REFERENCE_HEIGHT * MIN_BAR_SCALE);
        assert_close(
            minimum.transport_unit,
            reference.transport_unit * MIN_BAR_SCALE,
        );
        assert_close(minimum.title_size, reference.title_size * MIN_BAR_SCALE);
        assert_close(minimum.cover_side, reference.cover_side * MIN_BAR_SCALE);
        assert_close(minimum.padding_x, reference.padding_x * MIN_BAR_SCALE);
        assert_close(
            PlayerBarMetrics::new(REFERENCE_HEIGHT * 0.5).scale,
            MIN_BAR_SCALE,
        );
        assert!(tall.play_size > tall.transport_unit);
    }

    #[test]
    fn top_bar_children_share_the_bar_scale() {
        let reference = TopBarMetrics::new(REFERENCE_HEIGHT);
        assert_close(reference.bar_h, TOP_BAR_REFERENCE_HEIGHT);
        assert_close(reference.scale, 1.0);
        assert_close(reference.title_size, 19.0);
        assert_close(reference.body_size, 14.0);
        assert_close(reference.icon_size, 18.0);
        assert_close(reference.icon_pad, 3.0);
        assert_close(reference.padding_x, 16.0);
        assert_close(reference.gap, 8.0);
        assert_close(reference.chip_label_size, 9.0);
        assert_close(reference.chip_value_size, 12.0);
        assert_close(reference.insight_slot, 18.0);
        assert_close(reference.sync_bar_h, TOP_BAR_REFERENCE_HEIGHT * 0.16);

        let tall = TopBarMetrics::new(REFERENCE_HEIGHT * 1.25);
        assert_close(tall.scale, 1.25);
        assert_close(tall.title_size, reference.title_size * 1.25);
        assert_close(tall.icon_size, reference.icon_size * 1.25);
        assert_close(tall.padding_x, reference.padding_x * 1.25);
        assert_close(tall.chip_label_size, reference.chip_label_size * 1.25);
        assert_close(tall.chip_value_size, reference.chip_value_size * 1.25);

        let minimum = TopBarMetrics::new(MIN_WINDOW_HEIGHT);
        assert_close(minimum.scale, MIN_BAR_SCALE);
        assert_close(minimum.bar_h, TOP_BAR_REFERENCE_HEIGHT * MIN_BAR_SCALE);
        assert_close(minimum.icon_size, 18.0 * MIN_BAR_SCALE);
        assert_close(minimum.title_size, 19.0 * MIN_BAR_SCALE);
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
