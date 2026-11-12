//! Retained SVG mark for list rows.
//!
//! The normal `svg` element rebuilds its layout wrapper each frame. These marks
//! have fixed geometry, so a small custom element paints the cached atlas SVG
//! directly at the row's current origin.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, SharedString, Style, TransformationMatrix, Window, px, rgb,
};

pub(crate) struct RetainedIcon {
    id_name: &'static str,
    index: usize,
    path: SharedString,
    size: f32,
    color: u32,
}

impl RetainedIcon {
    pub(crate) fn new(
        id_name: &'static str,
        index: usize,
        path: impl Into<SharedString>,
        size: f32,
        color: u32,
    ) -> Self {
        Self {
            id_name,
            index,
            path: path.into(),
            size,
            color,
        }
    }
}

impl IntoElement for RetainedIcon {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RetainedIcon {
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
        _window: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        _ = window.paint_svg(
            bounds,
            self.path.clone(),
            TransformationMatrix::unit(),
            rgb(self.color).into(),
            cx,
        );
    }
}
