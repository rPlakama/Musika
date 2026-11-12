//! Shared icon, cover and text/presentation helpers.

use super::super::*;
use gpui::{SharedString, StyledImage as _};

fn transport_button(
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

fn transport_loading_button(
    icon_name: &str,
    base: f32,
    color: u32,
    scale: f32,
    degrees: f32,
) -> gpui::AnyElement {
    div()
        .size(px(base * 1.3))
        .flex()
        .items_center()
        .justify_center()
        .child(icon_rotated(icon_name, base * scale, color, degrees))
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
    /// A cached file with a provider URL to try when it does not load.
    Fallback {
        path: std::path::PathBuf,
        remote: String,
    },
}

impl CoverSource {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Path(path) => path.display().to_string(),
            Self::Remote(url) => url.clone(),
            Self::Fallback { path, remote } => format!("{}|{remote}", path.display()),
        }
    }

    fn image(&self) -> gpui::ImageSource {
        match self {
            Self::Path(path) | Self::Fallback { path, .. } => path.clone().into(),
            Self::Remote(url) => url.clone().into(),
        }
    }

    pub(crate) fn resource(&self) -> gpui::Resource {
        match self {
            Self::Path(path) | Self::Fallback { path, .. } => path.clone().into(),
            Self::Remote(url) => gpui::Resource::Uri(gpui::SharedUri::from(url.clone())),
        }
    }

    /// The provider URL to try when a cached file cannot be decoded.
    pub(crate) fn remote_resource(&self) -> Option<gpui::Resource> {
        let url = match self {
            Self::Remote(url) => url,
            Self::Fallback { remote, .. } => remote,
            Self::Path(_) => return None,
        };
        Some(gpui::Resource::Uri(gpui::SharedUri::from(url.clone())))
    }

    /// Attaches the provider URL as a load-time fallback for a cached path.
    ///
    /// A source that is already remote keeps its URL: the fallback is only
    /// meaningful for a local file that can fail to decode.
    pub(crate) fn with_remote_fallback(self, remote: &str) -> Self {
        let Some(remote) = remote_cover(remote) else {
            return self;
        };
        match self {
            Self::Path(path) => Self::Fallback { path, remote },
            other => other,
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

/// Resolves a cover URL with a provider URL as the fallback for a local file
/// that is missing or fails to load.
pub(crate) fn cover_source_with_remote(url: &str, remote: &str) -> Option<CoverSource> {
    match cover_source(url) {
        Some(source) => Some(source.with_remote_fallback(remote)),
        None => remote_cover(remote).map(CoverSource::Remote),
    }
}

/// Resolve a batch of row covers while holding the shared cover store once.
///
/// `Row` values keep their provider URLs; the resolved [`CoverSource`] is built
/// when the row window/page payload is installed, not per frame.
pub(crate) fn cover_sources<'a, I>(rows: I) -> Vec<Option<std::sync::Arc<CoverSource>>>
where
    I: Iterator<Item = (&'a str, &'a str)>,
{
    let mut store = COVERS
        .get_or_init(|| std::sync::Mutex::new(covers::CoverStore::new(768)))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    rows.map(|(url, remote)| {
        let source = match store.row_path(url) {
            Some(path) => Some(CoverSource::Path(path).with_remote_fallback(remote)),
            None => remote_cover(remote).map(CoverSource::Remote),
        };
        source.map(std::sync::Arc::new)
    })
    .collect()
}

fn cached_icon(
    cell: &'static std::sync::OnceLock<SharedString>,
    name: &'static str,
) -> SharedString {
    cell.get_or_init(|| crate::assets::icon(name).into())
        .clone()
}

/// Row mark: a download is in flight.
pub(crate) fn icon_download_pending() -> SharedString {
    static CELL: std::sync::OnceLock<SharedString> = std::sync::OnceLock::new();
    cached_icon(&CELL, "loading-loop.svg")
}

/// Row mark: downloaded.
pub(crate) fn icon_downloaded() -> SharedString {
    static CELL: std::sync::OnceLock<SharedString> = std::sync::OnceLock::new();
    cached_icon(&CELL, "outline-download.svg")
}

/// Row mark: favourite.
pub(crate) fn icon_favorite() -> SharedString {
    static CELL: std::sync::OnceLock<SharedString> = std::sync::OnceLock::new();
    cached_icon(&CELL, "baseline-favorite.svg")
}

/// Playlist/Radio mosaic: golden-ratio panes for any number of distinct
/// covers. The first split mirrors `PlaylistCollage.qml`; deeper splits keep
/// subdividing the remaining tail, so there is no fixed cover cap.
pub(crate) fn collage_art(urls: &[String], remotes: &[String], side: f32) -> gpui::AnyElement {
    let sources: Vec<Option<CoverSource>> = urls
        .iter()
        .enumerate()
        .filter(|(_, url)| !url.trim().is_empty())
        .map(|(index, url)| {
            cover_source_with_remote(
                url,
                remotes.get(index).map(String::as_str).unwrap_or_default(),
            )
        })
        .collect();
    collage_art_from_sources(sources, side)
}

pub(crate) fn collage_art_from_sources(
    sources: Vec<Option<CoverSource>>,
    side: f32,
) -> gpui::AnyElement {
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
            Some(CoverSource::Fallback { path, remote }) => {
                let fallback = remote.clone();
                img(path)
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover)
                    .with_fallback(move || {
                        img(fallback.clone())
                            .size_full()
                            .object_fit(gpui::ObjectFit::Cover)
                            .into_any_element()
                    })
                    .into_any_element()
            }
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
        Some(CoverSource::Fallback { path, remote }) => {
            let id = gpui::SharedString::from(format!(
                "cover-fade:{}|{remote}:{size:.0}",
                path.display()
            ));
            let fallback_url = remote.clone();
            let image = img(path)
                .size(px(size))
                .rounded_sm()
                .bg(rgb(palette::surface_container_highest()))
                .with_fallback(move || {
                    img(fallback_url.clone())
                        .size(px(size))
                        .rounded_sm()
                        .bg(rgb(palette::surface_container_highest()))
                        .into_any_element()
                });
            if fade {
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
    format!("{}:{}", descriptor.kind.api_kind(), descriptor.id)
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

pub(crate) fn icon(name: &str, size: f32, color: u32) -> gpui::AnyElement {
    let path = crate::assets::icon(name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .into_any_element()
}

pub(crate) fn icon_rotated(name: &str, size: f32, color: u32, degrees: f32) -> gpui::AnyElement {
    let path = crate::assets::icon(name);
    svg()
        .path(path)
        .size(px(size))
        .text_color(rgb(color))
        .with_transformation(gpui::Transformation::rotate(gpui::radians(
            degrees.to_radians(),
        )))
        .into_any_element()
}

/// The three transport buttons every player surface shows. The play/pause
/// branch, the loading spinner and the shared glyph names live here once
/// instead of in each view; `scales` are the press animations.
pub(crate) fn transport_controls(
    state: crate::player::State,
    loading: bool,
    loading_angle: f32,
    scales: [f32; 3],
    unit: f32,
    play: f32,
    spacing: f32,
) -> gpui::AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(spacing))
        .child(transport_button(
            crate::assets::ICON_PREVIOUS,
            unit,
            palette::on_surface(),
            scales[0],
        ))
        .child(if loading {
            transport_loading_button(
                "loading-loop.svg",
                play,
                palette::on_surface(),
                scales[1],
                loading_angle,
            )
        } else {
            transport_button(
                if state == crate::player::State::Playing {
                    crate::assets::ICON_PAUSE
                } else {
                    crate::assets::ICON_PLAY
                },
                play,
                palette::on_surface(),
                scales[1],
            )
        })
        .child(transport_button(
            crate::assets::ICON_NEXT,
            unit,
            palette::on_surface(),
            scales[2],
        ))
        .into_any_element()
}

/// The playback progress bar, including the held-seek ghost band. The caller
/// positions/sizes the returned bar; `rounded` keeps the fullscreen strip's
/// pill shape while the bottom thin strip stays square.
pub(crate) fn progress_bar(
    progress: f32,
    ghost: Option<(f32, f32)>,
    height: f32,
    rounded: bool,
) -> gpui::Div {
    div()
        .relative()
        .w_full()
        .h(px(height))
        .bg(rgb(palette::surface_container_high()))
        .when(rounded, |this| this.rounded_full())
        .child(
            div()
                .h_full()
                .w(relative(progress))
                .bg(rgb(palette::primary()))
                .when(rounded, |this| this.rounded_full()),
        )
        .when_some(ghost, |this, (start, length)| {
            this.child(
                div()
                    .absolute()
                    .top_0()
                    .left(relative(start))
                    .h_full()
                    .w(relative(length))
                    .bg(rgb(palette::primary()))
                    .opacity(0.30)
                    .when(rounded, |this| this.rounded_full()),
            )
        })
}

/// The flat volume meter shared by the player bar and the fullscreen transport.
/// The caller sizes the returned bar (`.flex_1()` or a fixed width); the fill
/// keeps the current level visible in grey while the output is muted.
pub(crate) fn volume_meter(volume: f64, muted: bool) -> gpui::Div {
    div()
        .h(px(3.0))
        .min_w_0()
        .rounded_full()
        .overflow_hidden()
        .bg(rgb(palette::surface_container_high()))
        .child(
            div()
                .h_full()
                .w(relative((volume / 100.0).clamp(0.0, 1.0) as f32))
                .rounded_full()
                .bg(rgb(volume_fill_color(muted))),
        )
}

/// Fill colour of a volume meter: primary normally, dimmed grey while muted.
pub(crate) fn volume_fill_color(muted: bool) -> u32 {
    if muted {
        palette::outline()
    } else {
        palette::primary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_meter_greys_out_while_muted() {
        assert_eq!(volume_fill_color(false), palette::primary());
        assert_eq!(volume_fill_color(true), palette::outline());
        assert_ne!(volume_fill_color(true), volume_fill_color(false));
    }
}
