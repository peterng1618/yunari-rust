//! Backend-independent paint geometry, in artboard units.
//!
//! Every surface paints the same list: the GPUI window, the raster frames a
//! panel receives, and the canvas the phone player drives. A backend decides
//! how to draw an op, never what to draw.

use crate::affine::Affine;
use crate::color::Color;
use crate::text::TextLayout;

#[derive(Debug, Clone, PartialEq)]
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

/// The page's picture and how it dissolves into the fill.
#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    pub asset_id: String,
    pub path: String,
    /// Absent for a simple background, which covers the box.
    pub composition: Option<yunari_model::document::BackgroundComposition>,
    pub geometry: crate::background::Geometry,
    /// A mirror about the placed rect's own centre.
    pub flip_x: bool,
    pub flip_y: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub transform: Affine,
    pub shape: Shape,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
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
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Color),
    Linear {
        from: (f32, f32),
        to: (f32, f32),
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}
