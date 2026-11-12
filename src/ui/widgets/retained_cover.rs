//! Retained cover art for list rows.
//!
//! The normal `img` element rebuilds its layout and image request every frame.
//! This element keeps the decoded image in the app-owned image cache and paints
//! the same cover at the row's current origin.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    App, Bounds, Corners, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, RenderImage, Style, Window, fill, px, rgb,
};

use crate::ui::image_cache::BoundedImageCache;
use crate::ui::widgets::media::CoverSource;

/// Opt-in cover-load latency log (`MUSIKA_COVER_STATS=1`): row decode latency is
/// measured from the first prepaint that requested the image to the frame that
/// first has it, per cover key.
fn cover_stats_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MUSIKA_COVER_STATS").as_deref() == Ok("1"))
}

thread_local! {
    static ROW_REQUESTS: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

pub(crate) struct RetainedCover {
    id_name: &'static str,
    index: usize,
    source: Option<Arc<CoverSource>>,
    cache: Entity<BoundedImageCache>,
    size: f32,
    scale: f32,
    image: Option<Arc<RenderImage>>,
}

impl RetainedCover {
    pub(crate) fn new(
        id_name: &'static str,
        index: usize,
        source: Option<Arc<CoverSource>>,
        cache: Entity<BoundedImageCache>,
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
        let Some(source) = self.source.as_deref() else {
            return;
        };
        let stats = cover_stats_enabled();
        let key = stats.then(|| source.key());
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
        // Arm only while the image is still loading; a cached image logs
        // nothing (it did not cost a load).
        if let (Some(key), true) = (key.as_ref(), stats) {
            ROW_REQUESTS.with(|requests| {
                if self.image.is_some() {
                    if let Some(started) = requests.borrow_mut().remove(key) {
                        eprintln!(
                            "[musika/cover] row {}:{} load_ms={:.2} source={key}",
                            self.id_name,
                            self.index,
                            started.elapsed().as_secs_f64() * 1000.0,
                        );
                    }
                } else {
                    requests
                        .borrow_mut()
                        .entry(key.clone())
                        .or_insert_with(Instant::now);
                }
            });
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
            _ = window.paint_image(image_bounds, corners, image.clone(), 0, false);
        }
    }
}
