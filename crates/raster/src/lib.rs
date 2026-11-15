#![forbid(unsafe_code)]
//! The raster backend: a display list becomes pixels.
//!
//! Panels receive frames from here, and so does every golden test, so the
//! geometry a theme paints is checked on the path a device actually draws.

use tiny_skia::{
    Color as SkiaColor, FillRule, LineCap, LineJoin, LinearGradient, Paint as SkiaPaint, Path,
    PathBuilder, Pixmap, Point, Rect, Shader, SpreadMode, Stroke as SkiaStroke, Transform,
};

use yunari_render::{Affine, DisplayList, Paint, PathCommand, Shape};

/// A rendered frame in straight (not premultiplied) RGBA8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Draw the list at `scale` device pixels per artboard unit.
pub fn render(list: &DisplayList, scale: f32) -> Raster {
    let width = (list.width * scale).round().max(0.0) as u32;
    let height = (list.height * scale).round().max(0.0) as u32;
    let mut pixmap = match Pixmap::new(width, height) {
        Some(pixmap) => pixmap,
        None => {
            return Raster {
                width: 0,
                height: 0,
                rgba: Vec::new(),
            };
        }
    };

    if let Some(paint) = &list.background {
        pixmap.fill(SkiaColor::TRANSPARENT);
        let board = PathBuilder::from_rect(
            Rect::from_xywh(0.0, 0.0, width as f32, height as f32).expect("a sized board"),
        );
        fill(&mut pixmap, &board, paint, Transform::from_scale(scale, scale));
    } else {
        pixmap.fill(SkiaColor::TRANSPARENT);
    }

    for op in &list.ops {
        let Some(path) = shape_path(&op.shape) else {
            continue;
        };
        let transform = transform_of(op.transform).pre_scale(scale, scale);
        if let Some(paint) = &op.fill {
            fill(&mut pixmap, &path, paint, transform);
        }
        if let Some(stroke) = &op.stroke
            && stroke.width > 0.0
            && let Some(shader) = shader(&stroke.paint)
        {
            let skia_paint = SkiaPaint {
                shader,
                anti_alias: true,
                ..SkiaPaint::default()
            };
            let skia_stroke = SkiaStroke {
                width: stroke.width * scale,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..SkiaStroke::default()
            };
            pixmap.stroke_path(&path, &skia_paint, &skia_stroke, transform, None);
        }
    }

    Raster {
        width,
        height,
        rgba: straight_rgba(&pixmap),
    }
}

fn fill(pixmap: &mut Pixmap, path: &Path, paint: &Paint, transform: Transform) {
    let Some(shader) = shader(paint) else {
        return;
    };
    let skia_paint = SkiaPaint {
        shader,
        anti_alias: true,
        ..SkiaPaint::default()
    };
    pixmap.fill_path(path, &skia_paint, FillRule::Winding, transform, None);
}

fn transform_of(affine: Affine) -> Transform {
    Transform::from_row(affine.a, affine.b, affine.c, affine.d, affine.e, affine.f)
}

fn shader(paint: &Paint) -> Option<Shader<'_>> {
    match paint {
        Paint::Solid(color) => Some(Shader::SolidColor(SkiaColor::from_rgba8(
            color.r, color.g, color.b, color.a,
        ))),
        Paint::Linear { from, to, stops } => {
            let stops: Vec<tiny_skia::GradientStop> = stops
                .iter()
                .map(|stop| {
                    tiny_skia::GradientStop::new(
                        stop.offset,
                        SkiaColor::from_rgba8(
                            stop.color.r,
                            stop.color.g,
                            stop.color.b,
                            stop.color.a,
                        ),
                    )
                })
                .collect();
            LinearGradient::new(
                Point::from_xy(from.0, from.1),
                Point::from_xy(to.0, to.1),
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            )
        }
    }
}

fn shape_path(shape: &Shape) -> Option<Path> {
    match shape {
        Shape::Rect {
            width,
            height,
            corner_radius,
        } => Some(rounded_rect(*width, *height, *corner_radius)),
        Shape::Ellipse { width, height } => {
            PathBuilder::from_oval(Rect::from_xywh(0.0, 0.0, *width, *height)?)
        }
        Shape::Line { length } => {
            let mut builder = PathBuilder::new();
            builder.move_to(0.0, 0.0);
            builder.line_to(*length, 0.0);
            builder.finish()
        }
        Shape::Path { commands, closed } => {
            let mut builder = PathBuilder::new();
            for command in commands {
                match *command {
                    PathCommand::MoveTo(x, y) => builder.move_to(x, y),
                    PathCommand::LineTo(x, y) => builder.line_to(x, y),
                    PathCommand::QuadTo(cx, cy, x, y) => builder.quad_to(cx, cy, x, y),
                    PathCommand::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                        builder.cubic_to(c1x, c1y, c2x, c2y, x, y)
                    }
                    PathCommand::Close => builder.close(),
                };
            }
            if *closed {
                builder.close();
            }
            builder.finish()
        }
    }
}

/// Corners are quarter circles joined by lines, with the usual cubic
/// approximation of a quarter arc. A radius wider than half the box is the
/// widest corner the box can carry, so it is what gets drawn.
fn rounded_rect(width: f32, height: f32, radius: f32) -> Path {
    let radius = radius.clamp(0.0, width.min(height) / 2.0);
    if radius <= 0.0 {
        return PathBuilder::from_rect(
            Rect::from_xywh(0.0, 0.0, width, height).expect("a sized box"),
        );
    }
    let kappa = 0.552_284_8 * radius;
    let mut builder = PathBuilder::new();
    builder.move_to(radius, 0.0);
    builder.line_to(width - radius, 0.0);
    builder.cubic_to(width - radius + kappa, 0.0, width, radius - kappa, width, radius);
    builder.line_to(width, height - radius);
    builder.cubic_to(
        width,
        height - radius + kappa,
        width - radius + kappa,
        height,
        width - radius,
        height,
    );
    builder.line_to(radius, height);
    builder.cubic_to(
        radius - kappa,
        height,
        0.0,
        height - radius + kappa,
        0.0,
        height - radius,
    );
    builder.line_to(0.0, radius);
    builder.cubic_to(0.0, radius - kappa, radius - kappa, 0.0, radius, 0.0);
    builder.close();
    builder.finish().unwrap_or_else(|| {
        PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, width, height).expect("a sized box"))
    })
}

/// tiny-skia paints in premultiplied alpha; a golden file and a device both
/// want the colour the author wrote.
fn straight_rgba(pixmap: &Pixmap) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_render::Color;

    fn list(json: &str) -> DisplayList {
        let document = yunari_model::document::parse_document(json).expect("fixture parses");
        yunari_render::build(&document).0
    }

    fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * raster.width + x) * 4) as usize;
        [
            raster.rgba[index],
            raster.rgba[index + 1],
            raster.rgba[index + 2],
            raster.rgba[index + 3],
        ]
    }

    #[test]
    fn a_filled_rectangle_covers_its_box_only() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":10,"height":10},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"x":2,"y":2,"width":6,"height":6},
                    "style":{"fill":{"value":"#ff0000"}}}]}"##,
            ),
            1.0,
        );
        assert_eq!(raster.width, 10);
        assert_eq!(pixel(&raster, 5, 5), [255, 0, 0, 255]);
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn an_ellipse_leaves_its_corner_unpainted() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":10,"height":10},
                    "nodes":[{"id":"n","kind":{"type":"ellipse"},
                    "transform":{"x":0,"y":0,"width":10,"height":10},
                    "style":{"fill":{"value":"#00ff00"}}}]}"##,
            ),
            1.0,
        );
        assert_eq!(pixel(&raster, 5, 5), [0, 255, 0, 255]);
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn a_background_fills_the_board_behind_the_nodes() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":4,"height":4},
                    "background":{"color":{"value":"#0000ff"}},"nodes":[]}"##,
            ),
            1.0,
        );
        assert_eq!(pixel(&raster, 0, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn a_gradient_runs_along_its_axis() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":8,"height":8},
                    "palette":{"p":{"name":"P","value":{"kind":"gradient","angle":0,
                    "stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}}},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"width":8,"height":8},"style":{"fill":{"ref":"palette.p"}}}]}"##,
            ),
            1.0,
        );
        let left = pixel(&raster, 1, 4);
        let right = pixel(&raster, 6, 4);
        assert!(
            left[0] < right[0],
            "the gradient runs left to right: {left:?} then {right:?}"
        );
    }

    #[test]
    fn scale_doubles_the_frame() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":5,"height":5},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"x":0,"y":0,"width":2,"height":2},
                    "style":{"fill":{"value":"#ffffff"}}}]}"##,
            ),
            2.0,
        );
        assert_eq!((raster.width, raster.height), (10, 10));
        assert_eq!(pixel(&raster, 3, 3), [255, 255, 255, 255]);
        assert_eq!(pixel(&raster, 5, 5), [0, 0, 0, 0]);
    }

    #[test]
    fn a_transparent_paint_leaves_nothing() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":4,"height":4},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"width":4,"height":4},
                    "style":{"fill":{"value":"rgba(255, 0, 0, 0)"}}}]}"##,
            ),
            1.0,
        );
        assert_eq!(pixel(&raster, 2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn straight_alpha_is_the_authored_colour() {
        let raster = render(
            &list(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":2,"height":2},
                    "nodes":[{"id":"n","kind":{"type":"rectangle"},
                    "transform":{"width":2,"height":2},
                    "style":{"fill":{"value":"#ffffff"},"opacity":{"value":0.5}}}]}"##,
            ),
            1.0,
        );
        assert_eq!(pixel(&raster, 1, 1), [255, 255, 255, 128]);
    }

    #[test]
    fn color_type_is_the_render_crates() {
        assert_eq!(Color::rgb(1, 2, 3).channels(), [1, 2, 3, 255]);
    }
}
