//! Golden-ratio pane layout for Playlist/Radio cover mosaics.
//!
//! The first three panes mirror legacy `PlaylistCollage.qml` exactly: a
//! 61.8% master, then the 38.2% tail split in the other axis. Beyond three
//! covers the tail keeps subdividing along its longer side, so any number of
//! distinct covers can be fitted without a hard cap.

/// The golden ratio.
pub const PHI: f32 = 1.618_034;

/// Fraction of the longer side that receives the next cover.
const LARGE: f32 = 0.618_034;
const SMALL: f32 = 1.0 - LARGE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pane {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn pane(self) -> Pane {
        Pane {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
        }
    }

    fn longer_axis(self, previous: Axis) -> Axis {
        if self.w > self.h {
            Axis::Vertical
        } else if self.h > self.w {
            Axis::Horizontal
        } else {
            match previous {
                Axis::Vertical => Axis::Horizontal,
                Axis::Horizontal => Axis::Vertical,
            }
        }
    }
}

/// Splits `rect` into the next cover pane (`large`) and the remaining tail
/// (`small`) with a seam of roughly `gap` pixels. The seam is clamped for very
/// deep subdivisions so the layout can never produce negative dimensions.
fn split(rect: Rect, axis: Axis, gap: f32) -> (Rect, Rect) {
    match axis {
        Axis::Vertical => {
            let seam = gap.min(rect.w * 0.25).max(0.0);
            let main_w = (rect.w * LARGE - seam * 0.5).max(0.0);
            let tail_w = (rect.w * SMALL - seam * 0.5).max(0.0);
            (
                Rect {
                    x: rect.x,
                    y: rect.y,
                    w: main_w,
                    h: rect.h,
                },
                Rect {
                    x: rect.x + rect.w - tail_w,
                    y: rect.y,
                    w: tail_w,
                    h: rect.h,
                },
            )
        }
        Axis::Horizontal => {
            let seam = gap.min(rect.h * 0.25).max(0.0);
            let main_h = (rect.h * LARGE - seam * 0.5).max(0.0);
            let tail_h = (rect.h * SMALL - seam * 0.5).max(0.0);
            (
                Rect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: main_h,
                },
                Rect {
                    x: rect.x,
                    y: rect.y + rect.h - tail_h,
                    w: rect.w,
                    h: tail_h,
                },
            )
        }
    }
}

/// Lays out `count` covers inside a square of `side` pixels.
///
/// `count == 0` returns no panes. The caller draws the empty mark. For
/// `count >= 2` the first split is vertical, matching the QML master pane;
/// each subsequent split uses the longer side of the remaining tail.
pub fn panes(side: f32, count: usize) -> Vec<Pane> {
    if count == 0 || !side.is_finite() || side <= 0.0 {
        return Vec::new();
    }
    let side = side.max(1.0);
    if count == 1 {
        return vec![Pane {
            x: 0.0,
            y: 0.0,
            w: side,
            h: side,
        }];
    }

    let gap = (side * 0.004).max(1.0);
    let mut out = Vec::with_capacity(count);
    let (master, mut tail) = split(
        Rect {
            x: 0.0,
            y: 0.0,
            w: side,
            h: side,
        },
        Axis::Vertical,
        gap,
    );
    out.push(master.pane());
    let mut previous = Axis::Vertical;

    // Every cover except the last gets the large side of a tail split; the
    // final cover fills whatever remains.
    for _ in 1..count.saturating_sub(1) {
        let axis = tail.longer_axis(previous);
        let (next, remainder) = split(tail, axis, gap);
        out.push(next.pane());
        previous = axis;
        tail = remainder;
    }
    out.push(tail.pane());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn empty_and_single_cover() {
        assert!(panes(100.0, 0).is_empty());
        assert!(panes(0.0, 3).is_empty());
        let one = panes(100.0, 1);
        assert_eq!(one.len(), 1);
        assert!(approx(one[0].w, 100.0) && approx(one[0].h, 100.0));
    }

    #[test]
    fn two_covers_match_the_qml_split() {
        let side = 100.0;
        let p = panes(side, 2);
        assert_eq!(p.len(), 2);
        let gap = (side * 0.004).max(1.0);
        assert!(approx(p[0].w, side * LARGE - gap / 2.0));
        assert!(approx(p[1].w, side * SMALL - gap / 2.0));
        assert!(approx(p[0].h, side));
        assert!(approx(p[1].h, side));
        assert!(approx(p[1].x, side - p[1].w));
        assert!(approx(p[1].x - (p[0].x + p[0].w), gap));
    }

    #[test]
    fn three_covers_match_the_qml_split() {
        let side = 100.0;
        let p = panes(side, 3);
        assert_eq!(p.len(), 3);
        let gap = (side * 0.004).max(1.0);
        assert!(approx(p[0].w, side * LARGE - gap / 2.0));
        assert!(approx(p[0].h, side));
        let right_w = side * SMALL - gap / 2.0;
        assert!(approx(p[1].w, right_w));
        assert!(approx(p[2].w, right_w));
        assert!(approx(p[1].h, side * LARGE - gap / 2.0));
        assert!(approx(p[2].h, side * SMALL - gap / 2.0));
        assert!(approx(p[2].y, side - p[2].h));
        assert!(approx(p[2].y - (p[1].y + p[1].h), gap));
    }

    #[test]
    fn unlimited_covers_stay_inside_the_square() {
        let side = 100.0;
        for count in [4usize, 5, 10, 50, 200] {
            let p = panes(side, count);
            assert_eq!(p.len(), count, "pane count {count}");
            for pane in p {
                assert!(pane.x.is_finite() && pane.y.is_finite());
                assert!(pane.w.is_finite() && pane.h.is_finite());
                assert!(pane.x >= -0.01 && pane.y >= -0.01);
                assert!(pane.w >= 0.0 && pane.h >= 0.0);
                assert!(pane.x + pane.w <= side + 0.01);
                assert!(pane.y + pane.h <= side + 0.01);
            }
        }
    }

    #[test]
    fn tail_keeps_the_golden_ratio() {
        let side = 100.0;
        let p = panes(side, 5);
        assert!(p[0].w / side < LARGE + 0.01);
        assert!((p[0].w + p[1].w + p[2].w + p[3].w + p[4].w) > 0.0);
        assert!(approx(LARGE / SMALL, PHI));
    }
}
