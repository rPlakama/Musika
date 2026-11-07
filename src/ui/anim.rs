// Musika — GPUI animation primitives (behaviour parity, legacy UI Anim/ColorAnim).
//!
//! The GPUI element tree is retained; these are our logical behaviour clocks,
//! keyed by stable ids so scrolled/recycled rows keep their state. Durations and
//! curves come from the legacy UI `Theme.js` tokens.

use std::time::Instant;

pub const REDUCE_MOTION: bool = false;

pub const QUICK_MS: f32 = 120.0;
pub const STANDARD_MS: f32 = 160.0;
pub const EMPHASIZED_MS: f32 = 240.0;
pub const MICRO_MS: f32 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Linear,
    Standard,
    Decelerate,
    Accelerate,
    /// Qt-clamped `[0.34,1.56,0.64,1]` (Theme.js bounce).
    Bounce,
    /// legacy UI `Easing.OutBack` with overshoot 1.7 (PopFx).
    OutBack,
    /// `Easing.OutCubic`: fast start, gentle settle (lyrics follow).
    OutCubic,
}

impl Curve {
    pub fn sample(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::Standard => cubic(0.2, 0.0, 0.0, 1.0, t),
            Curve::Decelerate => cubic(0.05, 0.7, 0.1, 1.0, t),
            Curve::Accelerate => cubic(0.3, 0.0, 0.8, 0.15, t),
            Curve::Bounce => cubic(0.34, 1.56, 0.64, 1.0, t).min(1.0),
            Curve::OutBack => {
                let c1 = 1.7;
                let c3 = c1 + 1.0;
                let u = t - 1.0;
                1.0 + c3 * u * u * u + c1 * u * u
            }
            Curve::OutCubic => 1.0 - (1.0 - t).powi(3),
        }
    }
}

pub struct Clock {
    start: Instant,
    pub now: f64,
    pub dt: f32,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            now: 0.0,
            dt: 0.0,
        }
    }

    pub fn tick(&mut self) -> f32 {
        let now = self.start.elapsed().as_secs_f64();
        let dt = (now - self.now) as f32;
        self.now = now;
        self.dt = dt.clamp(0.0, 0.1);
        self.dt
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

/// legacy UI `Behavior on <prop> { Anim {} }`: first call snaps to target; a target
/// change restarts from the currently painted value.
pub struct Behavior {
    from: f32,
    to: f32,
    start: f64,
    duration: f32,
    curve: Curve,
    initialized: bool,
}

impl Behavior {
    pub fn new(initial: f32) -> Self {
        Self {
            from: initial,
            to: initial,
            start: 0.0,
            duration: 0.0,
            curve: Curve::Standard,
            initialized: true,
        }
    }

    pub fn target(&mut self, to: f32, duration: f32, curve: Curve, clock: &Clock) -> f32 {
        if !self.initialized {
            self.from = to;
            self.to = to;
            self.start = clock.now;
            self.duration = duration;
            self.curve = curve;
            self.initialized = true;
            return to;
        }
        if (to - self.to).abs() > f32::EPSILON {
            self.from = self.value(clock);
            self.to = to;
            self.start = clock.now;
            self.duration = duration.max(0.0) / 1000.0;
            self.curve = curve;
        }
        self.value(clock)
    }

    pub fn value(&self, clock: &Clock) -> f32 {
        if REDUCE_MOTION || self.duration <= 0.0 {
            return self.to;
        }
        let t = ((clock.now - self.start) / self.duration as f64) as f32;
        if t >= 1.0 {
            return self.to;
        }
        let eased = self.curve.sample(t.clamp(0.0, 1.0));
        self.from + (self.to - self.from) * eased
    }

    pub fn settled(&self, clock: &Clock) -> bool {
        REDUCE_MOTION || clock.now - self.start >= self.duration as f64
    }
}

fn cubic(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let sample = |u: f32, a: f32, b: f32| {
        let v = 1.0 - u;
        3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
    };
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..18 {
        let mid = (lo + hi) * 0.5;
        if sample(mid, x1, x2) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    sample((lo + hi) * 0.5, y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_are_pinned() {
        for curve in [
            Curve::Linear,
            Curve::Standard,
            Curve::Decelerate,
            Curve::Accelerate,
            Curve::Bounce,
            Curve::OutCubic,
        ] {
            assert!((curve.sample(0.0) - 0.0).abs() < 1e-5);
            assert!((curve.sample(1.0) - 1.0).abs() < 1e-5);
        }
        let out_back = Curve::OutBack.sample(0.6);
        assert!(out_back > 1.0, "OutBack overshoot at the crest: {out_back}");
        assert!((Curve::OutCubic.sample(0.5) - 0.875).abs() < 1e-5);
    }

    #[test]
    fn behavior_retargets_from_current_value() {
        let mut clock = Clock::new();
        clock.now = 0.0;
        let mut behavior = Behavior::new(0.0);
        behavior.target(1.0, 1000.0, Curve::Linear, &clock);
        clock.now = 0.5;
        let mid = behavior.value(&clock);
        assert!((mid - 0.5).abs() < 1e-4);
        behavior.target(0.0, 1000.0, Curve::Linear, &clock);
        assert!((behavior.value(&clock) - mid).abs() < 1e-4);
    }
}

/// legacy UI `ColorAnim` semantics: straight-alpha component-wise sRGB interpolation,
/// retargeting from the currently painted colour.
pub struct ColorBehavior {
    from: u32,
    to: u32,
    start: f64,
    duration: f32,
    curve: Curve,
}

impl ColorBehavior {
    pub fn new(color: u32) -> Self {
        Self {
            from: color,
            to: color,
            start: 0.0,
            duration: 0.0,
            curve: Curve::Standard,
        }
    }

    pub fn target(&mut self, to: u32, duration: f32, curve: Curve, clock: &Clock) -> u32 {
        if to != self.to {
            self.from = self.value(clock);
            self.to = to;
            self.start = clock.now;
            self.duration = duration.max(0.0) / 1000.0;
            self.curve = curve;
        }
        self.value(clock)
    }

    pub fn value(&self, clock: &Clock) -> u32 {
        if REDUCE_MOTION || self.duration <= 0.0 {
            return self.to;
        }
        let t = ((clock.now - self.start) / self.duration as f64) as f32;
        lerp_srgb(self.from, self.to, self.curve.sample(t.clamp(0.0, 1.0)))
    }

    pub fn settled(&self, clock: &Clock) -> bool {
        REDUCE_MOTION || clock.now - self.start >= self.duration as f64
    }
}

/// Straight-alpha component-wise sRGB (Qt colour animation look).
pub fn lerp_srgb(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let av = ((a >> shift) & 0xff) as f32;
        let bv = ((b >> shift) & 0xff) as f32;
        (av + (bv - av) * t).round() as u32
    };
    (channel(24) << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn color_lerp_is_component_wise_srgb() {
        assert_eq!(lerp_srgb(0x00000000, 0xFFFFFFFF, 0.5), 0x80808080);
        assert_eq!(lerp_srgb(0xFF0000FF, 0x0000FFFF, 0.5), 0x800080FF);
    }

    #[test]
    fn color_behavior_retargets_from_current() {
        let mut clock = Clock::new();
        clock.now = 0.0;
        let mut color = ColorBehavior::new(0x00000000);
        color.target(0xFFFFFFFF, 1000.0, Curve::Linear, &clock);
        clock.now = 0.5;
        let mid = color.value(&clock);
        color.target(0x00000000, 1000.0, Curve::Linear, &clock);
        assert_eq!(color.value(&clock), mid);
    }
}
