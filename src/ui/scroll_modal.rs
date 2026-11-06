//! One scroll engine for every list surface.
//!
//! QML has a single `ScrollContent` plus `ScrollProfiles`; hosts only supply a
//! model, a row delegate and profile variables. GPUI used to carry a copy of
//! target/follow/metrics per surface. This module is that one engine.

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

    pub fn target_y(&self, count: usize, cursor: usize) -> f32 {
        let viewport = self.viewport();
        target_y(self.row_h, count, cursor, viewport)
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

    /// Set the cursor and pin the viewport instantly (QML `land()`).
    pub fn land(&mut self, count: usize, cursor: usize) -> usize {
        let cursor = if count == 0 {
            0
        } else {
            cursor.min(count - 1)
        };
        self.follow.active = false;
        let target = self.target_y(count, cursor);
        self.set_offset_now(target);
        self.handle
            .scroll_to_item_strict(cursor, gpui::ScrollStrategy::Center);
        cursor
    }

    /// Capture the first stable viewport once, then use the QML profile ratio.
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

pub fn target_y(row_h: f32, count: usize, cursor: usize, viewport: f32) -> f32 {
    let max = (count as f32 * row_h - viewport).max(0.0);
    let center = cursor as f32 * row_h + row_h * 0.5 - viewport * 0.5;
    center.clamp(0.0, max)
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
    fn profiles_match_qml_scroll_profiles() {
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
