//! Library sync lifecycle and palette rebuilds.

use super::super::*;

impl AppRoot {
    pub(in crate::ui) fn sync_angle(&self) -> f32 {
        if self.syncing {
            let elapsed =
                ((self.clock.now - self.sync_spin_started).max(0.0) / Self::SYNC_TURN_SECS) as f32;
            (self.sync_spin_offset + elapsed * 360.0).rem_euclid(360.0)
        } else {
            self.sync_settle.value(&self.clock)
        }
    }

    pub(in crate::ui) fn start_sync(&mut self, mode: SyncMode) {
        if self.syncing {
            self.push_toast(
                "A sync is already running.".to_string(),
                ToastKind::Notice,
                None,
            );
            return;
        }
        self.clock.tick();
        self.syncing = true;
        self.sync_progress = 0.0;
        self.sync_fill = anim::Behavior::new(0.0);
        self.sync_spin_started = self.clock.now;
        self.sync_spin_offset = self.sync_settle.value(&self.clock);
        self.bridge.sync(mode);
    }

    pub(in crate::ui) fn finish_sync(&mut self) {
        if !self.syncing {
            return;
        }
        self.clock.tick();
        let angle = self.sync_angle();
        self.syncing = false;
        let target = ((angle / 360.0).ceil() * 360.0).max(360.0);
        self.sync_fill
            .target(1.0, anim::QUICK_MS, anim::Curve::Linear, &self.clock);
        self.sync_settle = anim::Behavior::new(angle);
        self.sync_settle
            .target(target, anim::STANDARD_MS, anim::Curve::Bounce, &self.clock);
    }

    pub(in crate::ui) fn mark_sync_progress(&mut self, done: usize, total: usize) {
        self.clock.tick();
        if !self.syncing {
            self.syncing = true;
            self.sync_spin_started = self.clock.now;
            self.sync_spin_offset = self.sync_settle.value(&self.clock);
        }
        self.sync_progress = (done as f32 / total.max(1) as f32).clamp(0.0, 1.0);
        self.sync_fill.target(
            self.sync_progress,
            anim::QUICK_MS,
            anim::Curve::Linear,
            &self.clock,
        );
    }

    pub(in crate::ui) fn sync_palette(&mut self) {
        let cover = self
            .player
            .current()
            .map(|track| track.cover_art.clone())
            .unwrap_or_default();
        if self.palette_cover.as_deref() == Some(cover.as_str()) {
            return;
        }
        if cover.is_empty() {
            self.palette_motion.reset(&self.clock);
            self.palette_cover = None;
        } else {
            self.palette_cover = Some(cover.clone());
            self.bridge.request_colors(cover);
        }
    }
}
