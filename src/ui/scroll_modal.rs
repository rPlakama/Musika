//! One scroll engine for every list surface.
//!
//! legacy UI has a single `ScrollContent` plus `ScrollProfiles`; hosts only supply a
//! model, a row delegate and profile variables. GPUI used to carry a copy of
//! target/follow/metrics per surface. This module is that one engine.

use std::ops::Range;

use gpui::{UniformListScrollHandle, point, px};

use crate::ui::metrics;

#[derive(Debug, Clone, Copy)]
pub struct ScrollProfile {
    pub row_ratio: f32,
    pub nominal_row_h: f32,
    pub min_row_h: f32,
    pub overscan_rows: usize,
    pub follow_ms: f32,
}

pub const LIBRARY: ScrollProfile = ScrollProfile {
    row_ratio: 0.105,
    nominal_row_h: 58.0,
    min_row_h: 18.0,
    overscan_rows: 10,
    follow_ms: 300.0,
};
pub const ALBUM: ScrollProfile = ScrollProfile {
    row_ratio: 0.06,
    nominal_row_h: 36.0,
    min_row_h: 18.0,
    overscan_rows: 10,
    follow_ms: 300.0,
};
pub const SEARCH: ScrollProfile = ScrollProfile {
    row_ratio: 0.12,
    nominal_row_h: 56.0,
    min_row_h: 18.0,
    overscan_rows: 10,
    follow_ms: 300.0,
};
pub const QUEUE: ScrollProfile = ScrollProfile {
    row_ratio: 0.10,
    nominal_row_h: 56.0,
    min_row_h: 18.0,
    overscan_rows: 10,
    follow_ms: 300.0,
};

#[derive(Default, Clone, Copy)]
pub struct ScrollFollow {
    pub from: f32,
    pub to: f32,
    pub started: f64,
    pub duration: f32,
    pub active: bool,
}

impl ScrollFollow {
    pub fn start(&mut self, from: f32, to: f32, now: f64, duration: f32) {
        self.from = from;
        self.to = to;
        self.started = now;
        self.duration = duration;
        self.active = true;
    }
}

pub struct ScrollModal {
    pub handle: UniformListScrollHandle,
    pub follow: ScrollFollow,
    pub row_h: f32,
    pub row_ref: Option<f32>,
    pub profile: ScrollProfile,
}

impl ScrollModal {
    pub fn new(profile: ScrollProfile) -> Self {
        Self {
            handle: UniformListScrollHandle::new(),
            follow: ScrollFollow::default(),
            row_h: profile.nominal_row_h,
            row_ref: None,
            profile,
        }
    }

    pub fn viewport_height(&self) -> f32 {
        let height = self.handle.0.borrow().base_handle.bounds().size.height;
        let height = f32::from(height);
        if height > 0.0 { height } else { 600.0 }
    }

    fn viewport(&self) -> f32 {
        self.viewport_height()
    }

    /// The absolute row band currently painted by the list. legacy UI
    /// `SearchContract.rows()` uses exactly this band: rows outside it are
    /// holes, so `/` can never jump to a row the eye cannot see.
    pub fn visible_range(&self, count: usize) -> Range<usize> {
        let offset = -f32::from(self.handle.0.borrow().base_handle.offset().y);
        visible_index_range(offset, self.viewport_height(), self.row_h, count)
    }

    /// The logical row height taffy asked for is rounded per item by the
    /// uniform list's layout. Read the measured content back so targeting uses
    /// the same stride the rows are painted at.
    fn measured_row_h(&self, count: usize) -> f32 {
        if count == 0 {
            return self.row_h;
        }
        let content_h = self
            .handle
            .0
            .borrow()
            .last_item_size
            .map(|size| f32::from(size.contents.height))
            .unwrap_or(0.0);
        if content_h > 0.0 {
            let measured = content_h / count as f32;
            // The content height is from the previous prepaint. On a scope
            // switch the count can change before the list re-measures; only
            // trust the measurement when it is plausibly this profile's row.
            if (measured / self.row_h - 1.0).abs() <= 0.15 {
                return measured;
            }
        }
        self.row_h
    }

    /// Painted centre of one row, using the same measured stride as follow.
    pub fn row_center(&self, index: usize, count: usize) -> Option<(f32, f32)> {
        let state = self.handle.0.borrow();
        let bounds = state.base_handle.bounds();
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let row_h = self.measured_row_h(count);
        let offset = f32::from(state.base_handle.offset().y);
        Some((
            f32::from(bounds.origin.x) + width * 0.5,
            f32::from(bounds.origin.y) + index as f32 * row_h + row_h * 0.5 + offset,
        ))
    }

    pub fn target_y(&self, count: usize, cursor: usize) -> f32 {
        let viewport = self.viewport();
        let row_h = self.measured_row_h(count);
        let actual_max = f32::from(self.handle.0.borrow().base_handle.max_offset().height);
        if actual_max > 0.0 {
            target_y_max(row_h, cursor, viewport, actual_max)
        } else {
            target_y(row_h, count, cursor, viewport)
        }
    }

    pub fn start(&mut self, count: usize, cursor: usize, animated: bool, now: f64) {
        let target = self.target_y(count, cursor);
        let current = -f32::from(self.handle.0.borrow().base_handle.offset().y);
        let moved = (target - current).abs() > 0.5;
        let duration = if animated && moved {
            self.profile.follow_ms / 1000.0
        } else {
            0.0
        };
        self.follow.start(current, target, now, duration);
    }

    pub fn apply(&mut self, now: f64) -> bool {
        if !self.follow.active {
            return false;
        }
        let y = if self.follow.duration <= 0.0 {
            self.follow.active = false;
            self.follow.to
        } else {
            let progress = (((now - self.follow.started) / self.follow.duration as f64) as f32)
                .clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - progress).powi(3);
            let y = self.follow.from + (self.follow.to - self.follow.from) * eased;
            if progress >= 1.0 {
                self.follow.active = false;
            }
            y
        };
        self.set_offset_now(y);
        self.follow.active
    }

    pub fn set_offset_now(&self, target: f32) {
        self.handle
            .0
            .borrow()
            .base_handle
            .set_offset(point(px(0.0), px(-target)));
    }

    /// Set the cursor and pin the viewport instantly (legacy UI `land()`).
    pub fn land(&mut self, count: usize, cursor: usize) -> usize {
        let cursor = if count == 0 { 0 } else { cursor.min(count - 1) };
        self.follow.active = false;
        let target = self.target_y(count, cursor);
        self.set_offset_now(target);
        self.handle
            .scroll_to_item_strict(cursor, gpui::ScrollStrategy::Center);
        cursor
    }

    /// Capture the first stable viewport once, then use the legacy UI profile ratio.
    /// Returns true when the row height changed.
    pub fn ensure_metrics(&mut self, viewport: f32) -> bool {
        self.ensure_metrics_ref(viewport)
    }

    pub fn ensure_metrics_ref(&mut self, reference: f32) -> bool {
        if reference <= 0.0 {
            return false;
        }
        self.row_ref.get_or_insert(reference);
        let next = metrics::adaptive_row(
            reference,
            self.profile.row_ratio,
            self.profile.nominal_row_h,
        )
        .round()
        .max(self.profile.min_row_h);
        if (next - self.row_h).abs() >= 0.5 {
            self.row_h = next;
            true
        } else {
            false
        }
    }

    pub fn on_wheel(&mut self) {
        self.follow.active = false;
        self.handle.0.borrow_mut().deferred_scroll_to_item = None;
    }
}

/// Row range visible at `offset` for a fixed-height list. A zero viewport
/// means layout is not ready yet: return the whole model (legacy UI's first-open
/// fallback) so the first `/` is not starved to zero candidates.
pub fn visible_index_range(offset: f32, viewport: f32, row_h: f32, count: usize) -> Range<usize> {
    if count == 0 || row_h <= 0.0 {
        return 0..0;
    }
    if viewport <= 0.0 {
        return 0..count;
    }
    let first = (offset.max(0.0) / row_h).floor() as usize;
    let last = ((offset.max(0.0) + viewport) / row_h).ceil() as usize;
    let first = first.min(count.saturating_sub(1));
    let last = last.max(first + 1).min(count);
    first..last
}

pub fn target_y(row_h: f32, count: usize, cursor: usize, viewport: f32) -> f32 {
    let max = (count as f32 * row_h - viewport).max(0.0);
    target_y_max(row_h, cursor, viewport, max)
}

/// Use the scroll handle's real max offset, not a recomputed content height.
/// This is what keeps the cursor pinned at the last row: the centre formula
/// clamps to the actual list end instead of an estimated end.
pub fn target_y_max(row_h: f32, cursor: usize, viewport: f32, max: f32) -> f32 {
    let center = cursor as f32 * row_h + row_h * 0.5 - viewport * 0.5;
    center.clamp(0.0, max.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_target_centers_and_clamps() {
        assert_eq!(target_y(50.0, 100, 50, 500.0), 50.0 * 50.0 + 25.0 - 250.0);
        assert_eq!(target_y(50.0, 10, 0, 500.0), 0.0);
        assert_eq!(target_y(50.0, 10, 9, 500.0), 0.0);
    }

    #[test]
    fn visible_range_tracks_top_middle_and_bottom() {
        assert_eq!(visible_index_range(0.0, 500.0, 50.0, 100), 0..10);
        assert_eq!(visible_index_range(250.0, 500.0, 50.0, 100), 5..15);
        assert_eq!(visible_index_range(4750.0, 500.0, 50.0, 100), 95..100);
        assert_eq!(visible_index_range(0.0, 0.0, 50.0, 100), 0..100);
        assert_eq!(visible_index_range(10.0, 500.0, 50.0, 0), 0..0);
    }

    #[test]
    fn actual_max_keeps_bottom_pinned() {
        // Cursor last two rows with a real max offset: both stay at the end.
        assert_eq!(target_y_max(74.0, 569, 702.0, 41705.6), 41705.6);
        assert_eq!(target_y_max(74.0, 568, 702.0, 41705.6), 41705.6);
    }

    #[test]
    fn profiles_match_legacy_scroll_profiles() {
        assert!((LIBRARY.row_ratio - 0.105).abs() < f32::EPSILON);
        assert!((ALBUM.row_ratio - 0.06).abs() < f32::EPSILON);
        assert!((QUEUE.row_ratio - 0.10).abs() < f32::EPSILON);
        assert_eq!(LIBRARY.follow_ms, 300.0);
    }

    #[test]
    fn metrics_respect_profile_ratio() {
        let mut modal = ScrollModal::new(LIBRARY);
        let changed = modal.ensure_metrics(1000.0);
        assert!(changed);
        assert_eq!(modal.row_ref, Some(1000.0));
        assert!((modal.row_h - 105.0).abs() < 20.0);
    }
}
