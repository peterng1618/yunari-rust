//! Backend-independent paint geometry, in artboard units.
//!
//! Every surface paints the same list: the GPUI window, the raster frames a
//! panel receives, and the canvas the phone player drives. A backend decides
//! how to draw an op, never what to draw.

use serde::Serialize;

use crate::affine::Affine;
use crate::color::Color;
use crate::text::TextLayout;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayList {
    pub width: f32,
    pub height: f32,
    pub background: Option<Paint>,
    /// A picture behind the page, composed and faded into `background`. The
    /// geometry is resolved from the document; a backend adds the picture's own
    /// size and draws it.
    pub backdrop: Option<Backdrop>,
    pub ops: Vec<Op>,
}

impl DisplayList {
    /// True when a backend has to repaint at frame rate: a clip is playing in
    /// the page backdrop or in one of its own nodes.
    pub fn has_video(&self) -> bool {
        self.backdrop
            .as_ref()
            .is_some_and(|backdrop| backdrop.video)
            || self
                .ops
                .iter()
                .any(|op| matches!(&op.shape, Shape::Image { video: true, .. }))
    }
}

/// The page's picture and how it dissolves into the fill.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Backdrop {
    pub asset_id: String,
    pub path: String,
    /// A clip the backend plays rather than a picture it decodes once.
    pub video: bool,
    /// Absent for a simple background, which covers the box.
    pub composition: Option<yunari_model::document::BackgroundComposition>,
    pub geometry: crate::background::Geometry,
    /// The area, ramps and placed picture, resolved once the caller knows the
    /// source's own size. A backend that paints a decoded picture already has
    /// that size; it need not resolve the same arithmetic a second time.
    pub resolved: crate::background::Composition,
    /// A mirror about the placed rect's own centre.
    pub flip_x: bool,
    pub flip_y: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Op {
    pub transform: Affine,
    pub shape: Shape,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Shape {
    Rect {
        width: f32,
        height: f32,
        corner_radius: f32,
    },
    Ellipse {
        width: f32,
        height: f32,
    },
    /// A horizontal run of `length` from the op's origin, stroked.
    Line {
        length: f32,
    },
    Path {
        commands: Vec<PathCommand>,
        closed: bool,
    },
    /// Placed glyph runs. The layout is already wrapped and positioned, so a
    /// backend shapes what it is handed and paints it where it stands.
    Text(TextLayout),
    /// A frosted panel: the ops painted before this one, blurred over the
    /// wrapped shape's interior. It sits in the list at the panel's own place,
    /// so what it samples is exactly what is behind the panel.
    Glass {
        shape: Box<Shape>,
        blur_radius: f32,
    },
    /// A picture from the theme's assets, fitted into the op's box. The node
    /// names the asset; the backend decodes it and asks
    /// [`crate::image::placement`] where the pixels go. `monochrome` draws the
    /// picture's alpha as a mask in that colour, which is how an icon takes a
    /// palette colour.
    Image {
        asset_id: String,
        /// The asset's path in the theme, which is where its bytes live.
        path: String,
        width: f32,
        height: f32,
        fit: yunari_model::document::ImageFit,
        monochrome: Option<Color>,
        /// A clip the backend plays rather than a picture it decodes once.
        video: bool,
        /// Whether a clip starts again at its end; a background always does.
        video_loop: bool,
    },
    /// A chart a backend draws from its own settings and readings. The settings
    /// are the authored JSON bag; the renderer's chart rules decide which keys
    /// are meaningful, so no backend gets a second chart vocabulary. `id` is
    /// the scene object's id, which owns its bindings and names its issues.
    /// `width`/`height` are the chart's own box in artboard units and `opacity`
    /// is the composed authored opacity, so geometry is never sized from the
    /// whole artboard or drawn at full strength by mistake.
    Chart {
        id: String,
        family: ChartFamily,
        settings: serde_json::Value,
        bindings: Vec<String>,
        width: f32,
        height: f32,
        opacity: f32,
    },
}

/// The six chart families a theme authors. Names match the source app's
/// family names so a Fabric object needs no translation table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartFamily {
    Gauge,
    Line,
    Bar,
    Pie,
    Radar,
    PolarBar,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Paint {
    Solid(Color),
    Linear {
        from: (f32, f32),
        to: (f32, f32),
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}
