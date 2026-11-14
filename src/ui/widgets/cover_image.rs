//! One retained cover-painting element for every surface.
//!
//! `img()` rebuilds its image request and layout every frame. This element
//! paints the decoded cover directly and keeps using the same
//! [`BoundedImageCache`] instance: the cache is the virtual image store, so one
//! cover file is decoded once and every surface (row, player bar, page hero,
//! fullscreen, collage pane) paints the same `Arc<RenderImage>` / GPU image.
//!
//! The element owns no identity cache itself: callers pass a stable `key`
//! derived from the cover identity (album/cover id, or the resolved source) and
//! the cache de-duplicates by resource. A same-cover change therefore issues no
//! new load; a different cover paints a different cached image.

use std::sync::{Arc, OnceLock};

use gpui::{
    App, Bounds, Corners, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, RenderImage, Style, Window, fill, point, px, rgb,
};

use crate::ui::image_cache::{BoundedImageCache, display_edge_for};
use crate::ui::palette;
use crate::ui::widgets::media::CoverSource;

pub(crate) struct CoverImage {
    id_name: &'static str,
    index: usize,
    source: Option<Arc<CoverSource>>,
    cache: Entity<BoundedImageCache>,
    width: f32,
    height: f32,
    /// Row emphasis growth around the centre (1.0 = exact layout size).
    scale: f32,
    fit: ObjectFit,
    radius: f32,
    image: Option<Arc<RenderImage>>,
    /// Cheap thumbnail painted while `source` (the full-resolution copy)
    /// decodes. A hero appears within a frame instead of staying blank for a
    /// full-size JPEG decode; once the source lands it paints on the next
    /// frame and the preview goes stale.
    preview: Option<Arc<CoverSource>>,
    preview_image: Option<Arc<RenderImage>>,
    /// Keeps this decoded cover in a warm slot across page changes.
    retained: bool,
}

/// Whether cover load tracing (`MUSIKA_LOAD_TRACE`) is enabled. Read once so
/// the per-frame checks cost a relaxed atomic load instead of a getenv scan.
fn load_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MUSIKA_LOAD_TRACE").is_some())
}

/// Wall-clock unix milliseconds; 0 if the system clock is before the epoch.
fn unix_epoch_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

impl CoverImage {
    #[expect(
        clippy::too_many_arguments,
        reason = "the constructor mirrors the paint geometry; a builder would obscure it"
    )]
    pub(crate) fn new(
        id_name: &'static str,
        index: usize,
        source: Option<Arc<CoverSource>>,
        cache: Entity<BoundedImageCache>,
        width: f32,
        height: f32,
        scale: f32,
        fit: ObjectFit,
        radius: f32,
    ) -> Self {
        Self {
            id_name,
            index,
            source,
            cache,
            width,
            height,
            scale,
            fit,
            radius,
            image: None,
            preview: None,
            preview_image: None,
            retained: false,
        }
    }

    /// Paints `preview` (usually the 320 px thumbnail) until the full source
    /// has decoded, so a large cover never leaves a blank hero.
    pub(crate) fn with_preview(mut self, preview: Option<Arc<CoverSource>>) -> Self {
        self.preview = preview;
        self.preview_image = None;
        self
    }

    /// Keeps the decoded cover resident across page changes (hero surfaces
    /// and the playing cover), so album -> artist -> album does not re-decode.
    pub(crate) fn retained(mut self) -> Self {
        self.retained = true;
        self
    }
}

impl IntoElement for CoverImage {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CoverImage {
    type RequestLayoutState = ();
    type PrepaintState = ();

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
        style.size.width = px(self.width).into();
        style.size.height = px(self.height).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(source) = self.source.as_deref() else {
            return;
        };
        let trace = load_trace_enabled();
        let was_none = self.image.is_none();
        let resource = source.resource();
        // Decode at the physical size this cover is painted at. The original
        // cover file is untouched; a 400 pt hero decodes 400 px instead of a
        // 4096 px (64 MiB) texture, which is what used to grow the renderer's
        // texture pool on every album page.
        let edge = display_edge_for(
            self.width.max(self.height),
            window.scale_factor(),
        );
        let retained = self.retained;
        let loaded = self.cache.update(cx, |cache, cx| {
            if retained {
                cache.load_display_retained(&resource, edge, window, cx)
            } else {
                cache.load_display(&resource, edge, window, cx)
            }
        });
        self.image = match loaded {
            Some(Ok(image)) => Some(image),
            Some(Err(_)) => source.remote_resource().and_then(|resource| {
                self.cache
                    .update(cx, |cache, cx| {
                        if retained {
                            cache.load_display_retained(&resource, edge, window, cx)
                        } else {
                            cache.load_display(&resource, edge, window, cx)
                        }
                    })
                    .and_then(Result::ok)
            }),
            None => None,
        };
        // Paint a cheap thumbnail while the source decodes: the thumbnail
        // file is a few tens of KB, so the surface has real pixels within a
        // frame instead of a blank rectangle for the source decode.
        self.preview_image = None;
        if self.image.is_none()
            && let Some(preview) = self.preview.as_deref()
        {
            let preview_edge =
                display_edge_for(self.width.max(self.height), window.scale_factor()).min(384);
            let resource = preview.resource();
            self.preview_image = match self.cache.update(cx, |cache, cx| {
                cache.load_preview(&resource, preview_edge, window, cx)
            }) {
                Some(Ok(image)) => Some(image),
                Some(Err(_)) => preview.remote_resource().and_then(|resource| {
                    self.cache
                        .update(cx, |cache, cx| {
                            cache.load_preview(&resource, preview_edge, window, cx)
                        })
                        .and_then(Result::ok)
                }),
                None => None,
            };
        }
        // Diagnostic only, and gated: one line the first time this element
        // has an image to paint, so an external harness can time startup and
        // album-page hero readiness from the UI log. `state=preview` is the
        // fast placeholder; `state=full` is the sharp source.
        let state = if self.image.is_some() {
            Some("full")
        } else if self.preview_image.is_some() {
            Some("preview")
        } else {
            None
        };
        if trace
            && was_none
            && let Some(state) = state
        {
            eprintln!(
                "[musika/cover] ready id={} index={} epoch_ms={} edge={edge} state={state}",
                self.id_name,
                self.index,
                unix_epoch_ms(),
            );
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let corners = Corners {
            top_left: px(self.radius),
            top_right: px(self.radius),
            bottom_right: px(self.radius),
            bottom_left: px(self.radius),
        };
        window.paint_quad(
            fill(bounds, rgb(palette::surface_container_highest())).corner_radii(corners),
        );
        let Some(image) = self.image.as_ref().or(self.preview_image.as_ref()) else {
            return;
        };
        let scaled = self.scale.max(1.0);
        let width = self.width * scaled;
        let height = self.height * scaled;
        let layout = Bounds {
            origin: point(
                bounds.origin.x + px((self.width - width) * 0.5),
                bounds.origin.y + px((self.height - height) * 0.5),
            ),
            size: gpui::size(px(width), px(height)),
        };
        let image_bounds = self.fit.get_bounds(layout, image.size(0));
        _ = window.paint_image(image_bounds, corners, image.clone(), 0, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_distinguish_different_covers() {
        let a = CoverSource::Path(std::path::PathBuf::from("/a.jpg"));
        let b = CoverSource::Path(std::path::PathBuf::from("/b.jpg"));
        assert_eq!(a.key(), "/a.jpg");
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn identical_sources_share_one_cache_key() {
        let a = CoverSource::Path(std::path::PathBuf::from("/same.jpg"));
        let b = CoverSource::Path(std::path::PathBuf::from("/same.jpg"));
        assert_eq!(a.key(), b.key());
    }
}
