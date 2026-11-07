//! Shared icon, cover and text/presentation helpers.

use super::super::*;

pub(crate) fn transport_button(
    icon_name: &str,
    base: f32,
    color: u32,
    scale: f32,
) -> gpui::AnyElement {
    div()
        .size(px(base * 1.3))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(icon_name, base * scale, color))
        .into_any_element()
}

pub(crate) fn highlighted_text(
    text: &str,
    query: &str,
    color: u32,
    current: bool,
) -> gpui::AnyElement {
    let ranges = line_search::match_ranges(query, text);
    if ranges.is_empty() {
        return div()
            .text_color(rgb(color))
            .child(text.to_string())
            .into_any_element();
    }
    let (foreground, background) = if current {
        (palette::on_primary(), palette::primary())
    } else {
        (
            palette::on_primary_container(),
            palette::primary_container(),
        )
    };
    let highlight = HighlightStyle {
        color: Some(rgb(foreground).into()),
        background_color: Some(rgb(background).into()),
        ..Default::default()
    };
    StyledText::new(text.to_string())
        .with_highlights(ranges.into_iter().map(|range| (range, highlight)))
        .into_any_element()
}

pub(crate) fn loading_placeholder(index: usize, row_h: f32) -> gpui::AnyElement {
    let shade = if index.is_multiple_of(2) {
        palette::surface_container_high()
    } else {
        palette::surface_container_highest()
    };
    let muted = palette::surface_container_highest();
    div()
        .w_full()
        .h(px(row_h))
        .flex()
        .items_center()
        .px_1()
        .child(div().size(px(row_h * 0.78)).rounded_sm().bg(rgb(shade)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .ml_3()
                .gap_2()
                .child(
                    div()
                        .w(relative(0.44))
                        .h(px((row_h * 0.18).max(6.0)))
                        .rounded_sm()
                        .bg(rgb(muted)),
                )
                .child(
                    div()
                        .w(relative(0.26))
                        .h(px((row_h * 0.12).max(4.0)))
                        .rounded_sm()
                        .bg(rgb(muted)),
                ),
        )
        .into_any_element()
}

pub(crate) fn loading_skeleton(row_h: f32, count: usize) -> gpui::AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .children((0..count).map(|index| loading_placeholder(index, row_h)))
        .with_animation(
            "loading-skeleton",
            Animation::new(std::time::Duration::from_millis(1200))
                .repeat()
                .with_easing(gpui::ease_in_out),
            |this, delta| {
                let pulse = 0.72 + (1.0 - (delta * 2.0 - 1.0).abs()) * 0.28;
                this.opacity(pulse)
            },
        )
        .into_any_element()
}

pub(crate) fn cover_path(url: &str) -> Option<std::path::PathBuf> {
    COVERS
        .get_or_init(|| std::sync::Mutex::new(covers::CoverStore::new(768)))
        .lock()
        .ok()?
        .row_path(url)
}

/// A cover that may still be only on the provider's stream. Local cache hits
/// use a resolved `file://` path; cold caches fall back to the authenticated
/// `getCoverArt` URL, which GPUI can stream through [`remote_images`].
#[derive(Clone, Debug)]
pub(crate) enum CoverSource {
    Path(std::path::PathBuf),
    Remote(String),
}

impl CoverSource {
    fn key(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Remote(url) => url.clone(),
        }
    }

    fn image(&self) -> gpui::ImageSource {
        match self {
            Self::Path(path) => path.clone().into(),
            Self::Remote(url) => url.clone().into(),
        }
    }

    pub(crate) fn resource(&self) -> gpui::Resource {
        match self {
            Self::Path(path) => path.clone().into(),
            Self::Remote(url) => gpui::Resource::Uri(gpui::SharedUri::from(url.clone())),
        }
    }
}

pub(crate) fn remote_cover(url: &str) -> Option<String> {
    let url = url.trim();
    (url.starts_with("http://") || url.starts_with("https://")).then(|| url.to_string())
}

pub(crate) fn cover_source(url: &str) -> Option<CoverSource> {
    if let Some(remote) = remote_cover(url) {
        return Some(CoverSource::Remote(remote));
    }
    cover_path(url).map(CoverSource::Path)
}

/// Playlist/Radio mosaic: golden-ratio panes for any number of distinct
/// covers. The first split mirrors `PlaylistCollage.qml`; deeper splits keep
/// subdividing the remaining tail, so there is no fixed cover cap.
pub(crate) fn collage_art(urls: &[String], side: f32) -> gpui::AnyElement {
    let sources: Vec<Option<CoverSource>> = urls
        .iter()
        .filter(|url| !url.trim().is_empty())
        .map(|url| cover_source(url))
        .collect();

    let mut mosaic = div()
        .relative()
        .size(px(side))
        .rounded_sm()
        .overflow_hidden()
        .bg(rgb(palette::surface_container_high()));
    if sources.is_empty() {
        mosaic = mosaic.child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px((side * 0.16).max(28.0)))
                .text_color(rgb(palette::on_surface_variant()))
                .child("♪"),
        );
        return mosaic.into_any_element();
    }

    for (pane, source) in collage::panes(side, sources.len()).into_iter().zip(sources) {
        let content = match source {
            Some(source) => img(source.image())
                .size_full()
                .object_fit(gpui::ObjectFit::Cover)
                .into_any_element(),
            None => div()
                .size_full()
                .bg(rgb(palette::surface_container_highest()))
                .into_any_element(),
        };
        mosaic = mosaic.child(
            div()
                .absolute()
                .left(px(pane.x))
                .top(px(pane.y))
                .w(px(pane.w))
                .h(px(pane.h))
                .overflow_hidden()
                .child(content),
        );
    }
    mosaic.into_any_element()
}

pub(crate) fn art_scaled(source: Option<CoverSource>, base: f32, scale: f32) -> gpui::AnyElement {
    let size = base * scale;
    let offset = (size - base) * 0.5;
    div()
        .w(px(size))
        .h(px(size))
        .ml(px(-offset))
        .mr(px(-offset))
        .mt(px(-offset))
        .mb(px(-offset))
        .child(cover_element(source, size, false))
        .into_any_element()
}

pub(crate) fn art(source: Option<CoverSource>, size: f32) -> gpui::AnyElement {
    cover_element(source, size, true)
}

pub(crate) fn cover_element(
    source: Option<CoverSource>,
    size: f32,
    fade: bool,
) -> gpui::AnyElement {
    match source {
        Some(source) => {
            let image = img(source.image())
                .size(px(size))
                .rounded_sm()
                .bg(rgb(palette::surface_container_highest()));
            if fade {
                let id = gpui::SharedString::from(format!("cover-fade:{}:{size:.0}", source.key()));
                image
                    .with_animation(
                        id,
                        Animation::new(std::time::Duration::from_millis(120)),
                        |this, delta| this.opacity(delta),
                    )
                    .into_any_element()
            } else {
                image.into_any_element()
            }
        }
        None => div()
            .size(px(size))
            .rounded_md()
            .bg(rgb(palette::surface_container_highest()))
            .into_any_element(),
    }
}

pub(crate) fn page_key(descriptor: &PageDescriptor) -> String {
    let kind = match descriptor.kind {
        PageKind::Album => "album",
        PageKind::Artist => "artist",
        PageKind::Playlist => "playlist",
        PageKind::Radio => "radio",
    };
    format!("{kind}:{}", descriptor.id)
}

pub(crate) fn trace_scroll() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("MUSIKA_GPUUI_TRACE").is_some())
}

pub(crate) fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub(crate) fn repeat_label(repeat: crate::player::Repeat) -> &'static str {
    match repeat {
        crate::player::Repeat::Off => "off",
        crate::player::Repeat::One => "one",
        crate::player::Repeat::All => "all",
    }
}

pub(crate) fn asset_root() -> &'static str {
    static ROOT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::var("MUSIKA_ASSET_DIR")
            .ok()
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| format!("{}/src", env!("CARGO_MANIFEST_DIR")))
    })
}

pub(crate) fn icon(name: &str, size: f32, color: u32) -> gpui::AnyElement {
    let path = format!("{}/icons/{}", asset_root(), name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .into_any_element()
}

pub(crate) fn icon_rotated(name: &str, size: f32, color: u32, degrees: f32) -> gpui::AnyElement {
    let path = format!("{}/icons/{}", asset_root(), name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .with_transformation(gpui::Transformation::rotate(gpui::radians(
            degrees.to_radians(),
        )))
        .into_any_element()
}
