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
    pub ops: Vec<Op>,
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
