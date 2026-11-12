//! Debug-only frame-time HUD, ported from the retired QML `FpsCounter`.
//!
//! It samples the render-to-render interval from `Clock.dt`, keeps the last
//! 600 intervals and reports the same instantaneous/top/min/1%-low/average
//! numbers as the old overlay. It never requests a frame, so an idle window
//! can settle; the HUD only repaints when the application already renders.

use gpui::{div, prelude::*, px};

const SAMPLE_WINDOW: usize = 600;
const UPDATE_EVERY: u8 = 15;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    pub current: f32,
    pub top: f32,
    pub min: f32,
    pub low_1pct: f32,
    pub avg_fps: f32,
    pub avg_ms: f32,
    pub delay_ms: f32,
}

/// The debug HUD is on in debug builds unless `MUSIKA_FRAME_HUD=0`.
#[expect(
    dead_code,
    reason = "the HUD helper is only instantiated by the debug GPUI build"
)]
pub fn hud_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MUSIKA_FRAME_HUD").map_or(true, |value| value != "0"))
}

pub struct FrameStats {
    ring: [f32; SAMPLE_WINDOW],
    head: usize,
    len: usize,
    since_update: u8,
    stats: Stats,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            ring: [0.0; SAMPLE_WINDOW],
            head: 0,
            len: 0,
            since_update: 0,
            stats: Stats::default(),
        }
    }
}

impl FrameStats {
    pub fn push(&mut self, dt_ms: f32) {
        if !dt_ms.is_finite() || dt_ms <= 0.0 || dt_ms > 1000.0 {
            return;
        }
        self.stats.current = 1000.0 / dt_ms;
        self.stats.delay_ms = dt_ms;
        self.ring[self.head] = dt_ms;
        self.head = (self.head + 1) % SAMPLE_WINDOW;
        self.len = (self.len + 1).min(SAMPLE_WINDOW);
        self.since_update += 1;
        if self.since_update >= UPDATE_EVERY {
            self.since_update = 0;
            self.update_stats();
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    fn sample_count(&self) -> usize {
        self.len
    }

    fn update_stats(&mut self) {
        let n = self.sample_count();
        if n == 0 {
            return;
        }

        let mut sum = 0.0;
        let mut fastest = f32::MAX;
        let mut slowest: f32 = 0.0;
        let mut values = Vec::with_capacity(n);
        for index in 0..n {
            let slot = (self.head + SAMPLE_WINDOW - n + index) % SAMPLE_WINDOW;
            let value = self.ring[slot];
            values.push(value);
            sum += value;
            fastest = fastest.min(value);
            slowest = slowest.max(value);
        }

        let avg_ms = sum / n as f32;
        self.stats.current = 1000.0 / values[n - 1];
        self.stats.delay_ms = values[n - 1];
        self.stats.avg_ms = avg_ms;
        self.stats.avg_fps = 1000.0 / avg_ms;
        self.stats.top = 1000.0 / fastest;
        self.stats.min = 1000.0 / slowest;

        values.sort_by(|a, b| b.total_cmp(a));
        let count = (n / 100).max(1);
        let low_sum: f32 = values.iter().take(count).sum();
        self.stats.low_1pct = 1000.0 / (low_sum / count as f32);
    }
}

#[derive(Default)]
pub struct FrameHud {
    stats: FrameStats,
}

impl FrameHud {
    pub fn observe(&mut self, dt: f32) {
        self.stats.push(dt * 1000.0);
    }

    pub fn view(&self) -> gpui::AnyElement {
        let stats = self.stats.stats();
        let rows = [
            format!("Current: {} FPS", format_value(stats.current, 1)),
            format!("Top: {} FPS", format_value(stats.top, 1)),
            format!("Min: {} FPS", format_value(stats.min, 1)),
            format!("Low 1%: {} FPS", format_value(stats.low_1pct, 1)),
            format!("Average: {} FPS", format_value(stats.avg_fps, 1)),
            format!("AverageMS: {} ms", format_value(stats.avg_ms, 2)),
            format!("Delay: {} ms", format_value(stats.delay_ms, 2)),
        ];
        div()
            .absolute()
            .top(px(8.0))
            .left(px(8.0))
            .w(px(190.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.14))
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.34))
            .p(px(6.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .font_family("monospace")
            .text_size(px(11.0))
            .text_color(gpui::white())
            .children(rows.into_iter().map(|row| div().child(row)))
            .into_any_element()
    }
}

fn format_value(value: f32, digits: usize) -> String {
    let formatted = format!("{value:.digits$}");
    if formatted.contains('.') {
        formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        formatted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_window_and_stats_match_qml() {
        let mut stats = FrameStats::default();
        for _ in 0..SAMPLE_WINDOW + 3 {
            stats.push(16.0);
        }
        assert_eq!(stats.sample_count(), SAMPLE_WINDOW);
        let values = stats.stats();
        assert!((values.current - 62.5).abs() < 0.01);
        assert!((values.top - 62.5).abs() < 0.01);
        assert!((values.min - 62.5).abs() < 0.01);
        assert!((values.avg_fps - 62.5).abs() < 0.01);
        assert!((values.avg_ms - 16.0).abs() < 0.01);
        assert!((values.delay_ms - 16.0).abs() < 0.01);
    }

    #[test]
    fn one_percent_low_uses_slowest_intervals() {
        let mut stats = FrameStats::default();
        for _ in 0..600 {
            stats.push(10.0);
        }
        // Refresh a single slow interval into the ring, then force the
        // 15-frame stats update so the new sample is included.
        stats.push(100.0);
        for _ in 0..15 {
            stats.push(10.0);
        }
        let values = stats.stats();
        assert!(values.top >= 99.0);
        assert!(values.low_1pct < 100.0);
    }

    #[test]
    fn invalid_intervals_are_skipped() {
        let mut stats = FrameStats::default();
        stats.push(0.0);
        stats.push(-1.0);
        stats.push(1000.1);
        assert_eq!(stats.sample_count(), 0);
        assert_eq!(stats.stats(), Stats::default());
    }

    #[test]
    fn formatting_trims_trailing_zeros() {
        assert_eq!(format_value(60.0, 1), "60");
        assert_eq!(format_value(16.70, 2), "16.7");
        assert_eq!(format_value(16.0, 2), "16");
        assert_eq!(format_value(0.0, 1), "0");
    }
}
