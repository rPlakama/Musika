//! Retained cover art for list rows.
//!
//! The normal `img` element rebuilds its layout and image request every frame.
//! This element keeps the decoded image in the app-owned image cache and paints
//! the same cover at the row's current origin.

use std::sync::Arc;

use gpui::{
    App, Bounds, Corners, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, RenderImage, RetainAllImageCache, Style, Window, fill, px, rgb,
};

use crate::ui::widgets::media::CoverSource;

pub(crate) struct RetainedCover {
    id_name: &'static str,
    index: usize,
    source: Option<CoverSource>,
    cache: Entity<RetainAllImageCache>,
    size: f32,
    scale: f32,
    image: Option<Arc<RenderImage>>,
}

impl RetainedCover {
    pub(crate) fn new(
        id_name: &'static str,
        index: usize,
        source: Option<CoverSource>,
        cache: Entity<RetainAllImageCache>,
        size: f32,
        scale: f32,
    ) -> Self {
        Self {
            id_name,
            index,
            source,
            cache,
            size,
            scale,
            image: None,
        }
    }
}

impl IntoElement for RetainedCover {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RetainedCover {
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
        style.size.width = px(self.size).into();
        style.size.height = px(self.size).into();
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
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let resource = source.resource();
        self.image = self
            .cache
            .update(cx, |cache, cx| cache.load(&resource, window, cx))
            .and_then(Result::ok);
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
        let grow = (self.size * self.scale - self.size) * 0.5;
        let image_bounds = Bounds {
            origin: gpui::point(bounds.origin.x - px(grow), bounds.origin.y - px(grow)),
            size: gpui::size(px(self.size * self.scale), px(self.size * self.scale)),
        };
        let corners = Corners {
            top_left: px(4.0),
            top_right: px(4.0),
            bottom_right: px(4.0),
            bottom_left: px(4.0),
        };
        window.paint_quad(fill(bounds, rgb(0x303030)).corner_radii(Corners {
            top_left: px(4.0),
            top_right: px(4.0),
            bottom_right: px(4.0),
            bottom_left: px(4.0),
        }));
        if let Some(image) = self.image.as_ref() {
            window
                .paint_image(image_bounds, corners, image.clone(), 0, false)
                .ok();
        }
    }
}
