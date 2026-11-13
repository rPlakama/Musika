//! One-line horizontal marquee for now-playing names.
//!
//! The normal labels use `text_ellipsis`. Spotify-style names keep moving when
//! they do not fit, so this small custom element measures the shaped line once
//! and slides it inside its clipped parent.

use std::time::Instant;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, ShapedLine, SharedString, Style, TextRun, Window, point, px,
};

const INITIAL_PAUSE: f32 = 0.8;
const END_PAUSE: f32 = 1.0;
const SPEED: f32 = 36.0;

/// Ping-pong scroll offset for a line overflowing its viewport by `overflow`
/// pixels after `elapsed` seconds. Shared by the bottom-bar `MarqueeText` and
/// the top-bar identity viewport so both surfaces move identically.
pub(crate) fn marquee_offset(overflow: f32, elapsed: f32) -> f32 {
    let overflow = overflow.max(0.0);
    if overflow <= 0.0 {
        return 0.0;
    }
    let travel = overflow / SPEED;
    let cycle = INITIAL_PAUSE + travel + END_PAUSE + travel;
    let mut phase = elapsed.max(0.0) % cycle;
    if phase < INITIAL_PAUSE {
        0.0
    } else if phase < INITIAL_PAUSE + travel {
        phase -= INITIAL_PAUSE;
        phase * SPEED
    } else if phase < INITIAL_PAUSE + travel + END_PAUSE {
        overflow
    } else {
        phase -= INITIAL_PAUSE + travel + END_PAUSE;
        overflow - phase * SPEED
    }
}

#[derive(Clone, PartialEq)]
struct MarqueeKey {
    text: SharedString,
    size: u32,
    weight: u32,
    color: u32,
    width: u32,
}

impl MarqueeKey {
    fn new(text: &SharedString, size: f32, weight: f32, color: u32, width: Pixels) -> Self {
        Self {
            text: text.clone(),
            size: size.to_bits(),
            weight: weight.to_bits(),
            color,
            width: f32::from(width).to_bits(),
        }
    }
}

struct MarqueeCache {
    key: MarqueeKey,
    line: ShapedLine,
    overflow: f32,
    started: Instant,
}

impl MarqueeCache {
    fn shape(key: MarqueeKey, window: &Window) -> Self {
        let weight = gpui::FontWeight(f32::from_bits(key.weight));
        let mut font = gpui::font("Overused Grotesk Roman");
        font.weight = weight;
        let color = gpui::rgb(key.color).into();
        let run = TextRun {
            len: key.text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            key.text.clone(),
            px(f32::from_bits(key.size)),
            &[run],
            None,
        );
        let available = f32::from_bits(key.width);
        let overflow = (f32::from(line.width) - available).max(0.0);
        Self {
            key,
            line,
            overflow,
            started: Instant::now(),
        }
    }

    fn offset_at(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        marquee_offset(self.overflow, elapsed)
    }
}

pub(crate) struct MarqueeText {
    id_name: &'static str,
    index: usize,
    text: SharedString,
    size: f32,
    weight: f32,
    color: u32,
}

pub struct MarqueePaint {
    line: ShapedLine,
    offset: f32,
}

/// Natural pixel width of `text` shaped at `size`/`weight`.
///
/// The marquee element itself stretches to its parent, so a caller that needs
/// to place something immediately after the text (the fullscreen artist line)
/// measures the shaped line here.
pub(crate) fn line_width(window: &Window, text: &str, size: f32, weight: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let mut font = gpui::font("Overused Grotesk Roman");
    font.weight = gpui::FontWeight(weight);
    let color = gpui::rgb(0).into();
    let run = TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped_text: gpui::SharedString = text.to_owned().into();
    let line = window
        .text_system()
        .shape_line(shaped_text, px(size), &[run], None);
    f32::from(line.width)
}

impl MarqueeText {
    pub(crate) fn new(
        id_name: &'static str,
        index: usize,
        text: impl Into<SharedString>,
        size: f32,
        weight: f32,
        color: u32,
    ) -> Self {
        Self {
            id_name,
            index,
            text: text.into(),
            size,
            weight,
            color,
        }
    }
}

impl IntoElement for MarqueeText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MarqueeText {
    type RequestLayoutState = ();
    type PrepaintState = MarqueePaint;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::named_usize(self.id_name, self.index))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.height = px(self.size * 1.35).into();
        style.align_self = Some(gpui::AlignSelf::Stretch);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let width = bounds.size.width.max(px(1.0));
        let key = MarqueeKey::new(&self.text, self.size, self.weight, self.color, width);
        let now = Instant::now();
        let Some(id) = id else {
            let cache = MarqueeCache::shape(key, window);
            return MarqueePaint {
                line: cache.line,
                offset: 0.0,
            };
        };
        window.with_element_state::<MarqueeCache, _>(id, |cache, window| {
            let cache = match cache {
                Some(cache) if cache.key == key => cache,
                _ => MarqueeCache::shape(key, window),
            };
            if cache.overflow > 0.0 {
                window.request_animation_frame();
            }
            (
                MarqueePaint {
                    line: cache.line.clone(),
                    offset: cache.offset_at(now),
                },
                cache,
            )
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        paint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = px(self.size * 1.35);
        let origin = point(
            bounds.origin.x - px(paint.offset),
            bounds.origin.y + (bounds.size.height - line_height) * 0.5,
        );
        _ = paint.line.paint(origin, line_height, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_offset_starts_paused_then_travels_and_returns() {
        assert_eq!(marquee_offset(0.0, 0.0), 0.0);
        assert_eq!(marquee_offset(0.0, 10.0), 0.0);
        // Initial pause keeps the line at rest.
        assert_eq!(marquee_offset(72.0, 0.4), 0.0);
        // 72 px at 36 px/s: 2 s forward, 1 s end pause, 2 s back, 0.8 s start.
        assert!((marquee_offset(72.0, INITIAL_PAUSE + 1.0) - 36.0).abs() < 0.01);
        assert!((marquee_offset(72.0, INITIAL_PAUSE + 2.0) - 72.0).abs() < 0.01);
        assert!((marquee_offset(72.0, INITIAL_PAUSE + 2.7) - 72.0).abs() < 0.01);
        assert!((marquee_offset(72.0, INITIAL_PAUSE + 3.5) - 54.0).abs() < 0.01);
        // After the full cycle the line is resting again.
        let cycle = INITIAL_PAUSE + 2.0 + END_PAUSE + 2.0;
        assert_eq!(marquee_offset(72.0, cycle), marquee_offset(72.0, 0.0));
    }

    #[test]
    fn marquee_offset_is_bounded() {
        for step in 0..200 {
            let value = marquee_offset(120.0, step as f32 * 0.1);
            assert!((0.0..=120.0 + f32::EPSILON).contains(&value), "{value}");
        }
    }
}
