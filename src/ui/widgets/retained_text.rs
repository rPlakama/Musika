//! Retained text block for list rows.
//!
//! GPUI clears its Taffy tree every frame, so a normal row rebuilds and
//! measures its text on every scroll frame. This custom element keeps the
//! shaped lines in GPUI element state and only re-shapes when the text, width,
//! selection, highlight or palette changes. Painting reuses those lines at
//! the row's current origin.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId, Pixels,
    ShapedLine, SharedString, Style, TextAlign, TextRun, Window, point, px, rgb, size,
};
use gpui::{IntoElement, WrappedLine};

use crate::ui::{line_search, palette};

#[derive(Clone)]
struct TextKey {
    title: SharedString,
    subtitle: SharedString,
    query: SharedString,
    current: bool,
    selected: bool,
    title_size: u32,
    subtitle_size: u32,
    width: u32,
    height: u32,
    title_color: u32,
    subtitle_color: u32,
    compact: bool,
    palette: palette::Palette,
}

impl TextKey {
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor mirrors the cached text inputs; a builder would obscure the cache key"
    )]
    fn new(
        title: SharedString,
        subtitle: SharedString,
        query: SharedString,
        current: bool,
        selected: bool,
        title_size: f32,
        subtitle_size: f32,
        width: f32,
        height: f32,
        title_color: u32,
        subtitle_color: u32,
        compact: bool,
    ) -> Self {
        Self {
            title,
            subtitle,
            query,
            current,
            selected,
            title_size: title_size.to_bits(),
            subtitle_size: subtitle_size.to_bits(),
            width: width.to_bits(),
            height: height.to_bits(),
            title_color,
            subtitle_color,
            compact,
            palette: palette::current(),
        }
    }
}

impl PartialEq for TextKey {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title
            && self.subtitle == other.subtitle
            && self.query == other.query
            && self.current == other.current
            && self.selected == other.selected
            && self.title_size == other.title_size
            && self.subtitle_size == other.subtitle_size
            && self.width == other.width
            && self.height == other.height
            && self.title_color == other.title_color
            && self.subtitle_color == other.subtitle_color
            && self.compact == other.compact
            && self.palette == other.palette
    }
}

impl Eq for TextKey {}

#[derive(Clone)]
enum CachedLine {
    Wrapped(WrappedLine),
    Plain(ShapedLine),
}

impl CachedLine {
    fn height(&self, line_height: Pixels) -> Pixels {
        match self {
            Self::Wrapped(line) => line.size(line_height).height,
            Self::Plain(_) => line_height,
        }
    }

    fn paint(
        &self,
        origin: gpui::Point<Pixels>,
        line_height: Pixels,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Wrapped(line) => {
                _ = line.paint_background(
                    origin,
                    line_height,
                    TextAlign::Left,
                    Some(bounds),
                    window,
                    cx,
                );
                _ = line.paint(
                    origin,
                    line_height,
                    TextAlign::Left,
                    Some(bounds),
                    window,
                    cx,
                );
            }
            Self::Plain(line) => {
                _ = line.paint_background(origin, line_height, window, cx);
                _ = line.paint(origin, line_height, window, cx);
            }
        }
    }
}

#[derive(Clone)]
pub struct CachedText {
    key: TextKey,
    title: Option<CachedLine>,
    subtitle: Option<CachedLine>,
    title_line_height: Pixels,
    subtitle_line_height: Pixels,
}

impl CachedText {
    fn title_height(&self) -> Pixels {
        self.title
            .as_ref()
            .map_or(Pixels::ZERO, |line| line.height(self.title_line_height))
    }

    fn subtitle_height(&self) -> Pixels {
        self.subtitle
            .as_ref()
            .map_or(Pixels::ZERO, |line| line.height(self.subtitle_line_height))
    }
}

/// A title/subtitle block that keeps its shaped lines across frames.
pub(crate) struct RetainedText {
    id_name: &'static str,
    index: usize,
    title: SharedString,
    subtitle: SharedString,
    query: SharedString,
    current: bool,
    selected: bool,
    title_size: f32,
    subtitle_size: f32,
    height: f32,
    title_color: u32,
    subtitle_color: u32,
    compact: bool,
}

impl RetainedText {
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor mirrors the layout/text inputs; grouping them would add a throwaway struct"
    )]
    pub(crate) fn new(
        id_name: &'static str,
        index: usize,
        title: SharedString,
        subtitle: SharedString,
        query: SharedString,
        current: bool,
        selected: bool,
        title_size: f32,
        subtitle_size: f32,
        height: f32,
        title_color: u32,
        subtitle_color: u32,
        compact: bool,
    ) -> Self {
        Self {
            id_name,
            index,
            title,
            subtitle,
            query,
            current,
            selected,
            title_size,
            subtitle_size,
            height,
            title_color,
            subtitle_color,
            compact,
        }
    }

    fn key(&self, width: Pixels) -> TextKey {
        TextKey::new(
            self.title.clone(),
            self.subtitle.clone(),
            self.query.clone(),
            self.current,
            self.selected,
            self.title_size,
            self.subtitle_size,
            f32::from(width),
            self.height,
            self.title_color,
            self.subtitle_color,
            self.compact,
        )
    }

    fn shape(&self, width: Pixels, window: &mut Window) -> CachedText {
        let title_font = if self.selected {
            gpui::font("Overused Grotesk Roman").bold()
        } else {
            gpui::font("Overused Grotesk Roman")
        };
        let subtitle_font = gpui::font("Overused Grotesk Roman");
        let title_runs = runs(
            &self.title,
            &self.query,
            self.current,
            title_font,
            rgb(self.title_color).into(),
        );
        let subtitle_runs = runs(
            &self.subtitle,
            &self.query,
            self.current,
            subtitle_font,
            rgb(self.subtitle_color).into(),
        );
        let title_line_height = (self.title_size * 1.2).max(1.0);
        let subtitle_line_height = (self.subtitle_size * 1.2).max(1.0);
        let key = self.key(width);
        let title = shape_text(
            self.title.clone(),
            self.title_size,
            title_runs,
            width,
            2,
            self.compact,
            window,
        );
        let subtitle = shape_text(
            self.subtitle.clone(),
            self.subtitle_size,
            subtitle_runs,
            width,
            2,
            self.compact,
            window,
        );
        CachedText {
            key,
            title,
            subtitle,
            title_line_height: px(title_line_height),
            subtitle_line_height: px(subtitle_line_height),
        }
    }
}

static SHAPES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Total calls into `shape_text` since process start (diagnostics).
pub(crate) fn shape_count() -> u64 {
    SHAPES.load(std::sync::atomic::Ordering::Relaxed)
}

fn text_trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MUSIKA_TEXT_TRACE").is_some())
}

fn shape_text(
    text: SharedString,
    font_size: f32,
    runs: Vec<TextRun>,
    width: Pixels,
    line_clamp: usize,
    compact: bool,
    window: &mut Window,
) -> Option<CachedLine> {
    if text.is_empty() {
        return None;
    }
    let count = SHAPES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if count.is_multiple_of(100) && text_trace() {
        eprintln!("retained-text: shape calls={count}");
    }
    if compact {
        let font = runs.first()?.font.clone();
        let mut wrapper = window.text_system().line_wrapper(font, px(font_size));
        let mut runs = runs;
        let text = wrapper.truncate_line(text, width, "…", &mut runs);
        return Some(CachedLine::Plain(window.text_system().shape_line(
            text,
            px(font_size),
            &runs,
            None,
        )));
    }
    window
        .text_system()
        .shape_text(text, px(font_size), &runs, Some(width), Some(line_clamp))
        .ok()
        .and_then(|lines| lines.into_iter().next())
        .map(CachedLine::Wrapped)
}

fn runs(
    text: &str,
    query: &str,
    current: bool,
    font: gpui::Font,
    color: gpui::Hsla,
) -> Vec<TextRun> {
    let matches = line_search::match_ranges(query, text);
    if matches.is_empty() {
        return vec![TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
    }
    let (highlight_fg, highlight_bg) = if current {
        (palette::on_primary(), palette::primary())
    } else {
        (
            palette::on_primary_container(),
            palette::primary_container(),
        )
    };
    let mut runs = Vec::new();
    let mut pos = 0;
    for range in matches {
        if range.start > pos {
            runs.push(TextRun {
                len: range.start - pos,
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        runs.push(TextRun {
            len: range.end.saturating_sub(range.start),
            font: font.clone(),
            color: rgb(highlight_fg).into(),
            background_color: Some(rgb(highlight_bg).into()),
            underline: None,
            strikethrough: None,
        });
        pos = range.end;
    }
    if pos < text.len() {
        runs.push(TextRun {
            len: text.len() - pos,
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

impl IntoElement for RetainedText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RetainedText {
    type RequestLayoutState = ();
    type PrepaintState = CachedText;

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
        style.size.height = px(self.height).into();
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
        let key = self.key(width);
        let Some(global_id) = id else {
            return self.shape(width, window);
        };
        window.with_element_state::<CachedText, _>(global_id, |cached, window| {
            let next = match cached.as_ref() {
                Some(cached) if cached.key == key => cached.clone(),
                _ => self.shape(width, window),
            };
            (next.clone(), next)
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        cached: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let total = cached.title_height() + cached.subtitle_height();
        let mut origin = point(
            bounds.origin.x,
            bounds.origin.y + ((bounds.size.height - total) / 2.0).max(px(0.0)),
        );
        if let Some(line) = cached.title.as_ref() {
            let text_bounds = Bounds {
                origin,
                size: size(bounds.size.width, cached.title_height()),
            };
            line.paint(origin, cached.title_line_height, text_bounds, window, cx);
            origin.y += cached.title_height();
        }
        if let Some(line) = cached.subtitle.as_ref() {
            let text_bounds = Bounds {
                origin,
                size: size(bounds.size.width, cached.subtitle_height()),
            };
            line.paint(origin, cached.subtitle_line_height, text_bounds, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_runs_preserve_the_whole_text_length() {
        let runs = runs(
            "abcdef",
            "cd",
            false,
            gpui::font("monospace"),
            rgb(0xFFFFFFFF).into(),
        );
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), 6);
        assert!(
            runs.iter()
                .any(|run| run.background_color.is_some() && run.len == 2)
        );
    }

    #[test]
    fn empty_query_uses_one_run() {
        let runs = runs(
            "abcdef",
            "",
            false,
            gpui::font("monospace"),
            rgb(0xFFFFFFFF).into(),
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, 6);
        assert!(runs[0].background_color.is_none());
    }
}
