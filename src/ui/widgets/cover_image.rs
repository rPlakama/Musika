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

use std::sync::Arc;

use gpui::{
    App, Bounds, Corners, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, RenderImage, Style, Window, fill, point, px, rgb,
};

use crate::ui::image_cache::BoundedImageCache;
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
        }
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
        let resource = source.resource();
        let loaded = self
            .cache
            .update(cx, |cache, cx| cache.load(&resource, window, cx));
        self.image = match loaded {
            Some(Ok(image)) => Some(image),
            Some(Err(_)) => source.remote_resource().and_then(|resource| {
                self.cache
                    .update(cx, |cache, cx| cache.load(&resource, window, cx))
                    .and_then(Result::ok)
            }),
            None => None,
        };
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
        let Some(image) = self.image.as_ref() else {
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
