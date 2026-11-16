#![forbid(unsafe_code)]
//! The GPUI backend: a display list becomes window primitives.
//!
//! Three cases, in order of how often a theme hits them: an axis-aligned
//! rectangle is a quad, a text segment is a shaped line, and everything else —
//! an arc band, an authored path, a turned rectangle — is rasterised at device
//! scale and painted as an image.
//!
//! GPUI's own path is a triangle fan, which cannot express a concave shape (a
//! gauge ring is one), so the raster carries those shapes instead of drawing
//! them wrongly. The image is produced at device scale by the same geometry the
//! goldens compare against, so what a window shows and what a golden holds are
//! the same picture.

use std::sync::Arc;

use gpui_kit::*;
use yunari_render::{Color, DisplayList, Op, Paint, Shape};
use yunari_render_raster::Images;

/// The artboard fitted into the surface it is painted on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub origin: (f32, f32),
}

impl Fit {
    /// The largest uniform scale that fits the artboard, centred.
    pub fn new(list: &DisplayList, bounds: Bounds<Pixels>) -> Self {
        let width = list.width.max(1.0);
        let height = list.height.max(1.0);
        let surface = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let scale = (surface.0 / width).min(surface.1 / height);
        let origin = (
            f32::from(bounds.origin.x) + (surface.0 - width * scale) / 2.0,
            f32::from(bounds.origin.y) + (surface.1 - height * scale) / 2.0,
        );
        Self { scale, origin }
    }

    pub fn point(&self, x: f32, y: f32) -> Point<Pixels> {
        point(
            px(self.origin.0 + x * self.scale),
            px(self.origin.1 + y * self.scale),
        )
    }

    pub fn rect(&self, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: self.point(0.0, 0.0),
            size: size(px(width * self.scale), px(height * self.scale)),
        }
    }
}

/// Paint a display list into `bounds`, fitted to the artboard.
///
/// `images` resolves picture nodes; a theme whose pictures live beside it
/// passes a store built with [`Images::at`].
pub fn paint(
    list: &DisplayList,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
    images: &mut Images,
) {
    paint_fitted(list, Fit::new(list, bounds), window, cx, images);
}

/// Paint a display list through a caller-chosen fit, which is what an editor's
/// viewport is: the same frame, placed and scaled by the author's camera.
pub fn paint_fitted(
    list: &DisplayList,
    fit: Fit,
    window: &mut Window,
    cx: &mut App,
    images: &mut Images,
) {
    // A page picture is composed against the whole artboard, and a frosted
    // panel samples what is painted beneath it, so either makes the frame one
    // image rather than an op-by-op paint.
    if list.backdrop.is_some() || list.ops.iter().any(|op| matches!(op.shape, Shape::Glass { .. })) {
        paint_whole(list, &fit, window, images);
        return;
    }
    if let Some(paint) = &list.background {
        window.paint_quad(PaintQuad {
            bounds: fit.rect(list.width, list.height),
            corner_radii: Corners::default(),
            background: background_of(paint),
            border_widths: Edges::default(),
            border_color: transparent_black(),
            border_style: BorderStyle::Solid,
        });
    }
    for op in &list.ops {
        paint_op(op, list, &fit, window, cx, images);
    }
}

fn paint_op(
    op: &Op,
    list: &DisplayList,
    fit: &Fit,
    window: &mut Window,
    cx: &mut App,
    images: &mut Images,
) {
    match &op.shape {
        Shape::Text(layout) => paint_text(layout, op, fit, window, cx),
        Shape::Rect {
            width,
            height,
            corner_radius,
        } if axis_aligned(op) => {
            let Some(fill) = &op.fill else {
                paint_rasterised(op, list, fit, window, images);
                return;
            };
            let stroke = op.stroke.as_ref().filter(|stroke| stroke.width > 0.0);
            window.paint_quad(PaintQuad {
                bounds: Bounds {
                    origin: fit.point(op.transform.e, op.transform.f),
                    size: size(
                        px(*width * fit.scale * op.transform.a),
                        px(*height * fit.scale * op.transform.d),
                    ),
                },
                corner_radii: Corners::all(px(*corner_radius * fit.scale)),
                background: background_of(fill),
                border_widths: stroke.map_or(Edges::default(), |stroke| {
                    Edges::all(px(stroke.width * fit.scale))
                }),
                border_color: stroke.map_or(transparent_black(), |stroke| {
                    colour_of(&stroke.paint)
                }),
                border_style: BorderStyle::Solid,
            });
        }
        _ => paint_rasterised(op, list, fit, window, images),
    }
}

/// A rectangle whose transform only moves and scales it: a quad can express
/// that, and nothing else.
fn axis_aligned(op: &Op) -> bool {
    let affine = op.transform;
    affine.b == 0.0 && affine.c == 0.0 && affine.a > 0.0 && affine.d > 0.0
}

fn paint_text(
    layout: &yunari_render::text::TextLayout,
    op: &Op,
    fit: &Fit,
    window: &mut Window,
    cx: &mut App,
) {
    for segment in &layout.segments {
        let origin = fit.point(
            segment.origin.0 * op.transform.a + op.transform.e,
            segment.origin.1 * op.transform.d + op.transform.f,
        );
        let size = px(segment.style.size * fit.scale);
        let run = TextRun {
            len: segment.text.len(),
            font: Font {
                family: segment
                    .style
                    .family
                    .clone()
                    .unwrap_or_else(|| ".SystemUIFont".into())
                    .into(),
                weight: FontWeight(f32::from(segment.style.weight)),
                style: if segment.style.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                ..Font::default()
            },
            color: hsla_of(segment.style.color),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window
            .text_system()
            .shape_line(segment.text.clone().into(), size, &[run], None);
        let _ = line.paint(
            origin,
            px(segment.style.size * 1.2 * fit.scale),
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

/// The whole frame as one image, for a page whose picture spans it.
fn paint_whole(list: &DisplayList, fit: &Fit, window: &mut Window, images: &mut Images) {
    let scale = fit.scale * window.scale_factor();
    let raster = yunari_render_raster::render(list, scale, None, Some(images));
    let Some(buffer) = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba) else {
        return;
    };
    let bounds = fit.rect(list.width, list.height);
    let data = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    let _ = window.paint_image(bounds, bounds, Corners::default(), data, 0, false);
}

/// The op as pixels: the shapes a quad cannot carry, drawn by the same geometry
/// the goldens compare against.
fn paint_rasterised(
    op: &Op,
    list: &DisplayList,
    fit: &Fit,
    window: &mut Window,
    images: &mut Images,
) {
    let one = DisplayList {
        width: list.width,
        height: list.height,
        background: None,
        backdrop: None,
        ops: vec![op.clone()],
    };
    let scale = fit.scale * window.scale_factor();
    let raster = yunari_render_raster::render(&one, scale, None, Some(images));
    let Some(buffer) = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba) else {
        return;
    };
    let bounds = fit.rect(list.width, list.height);
    let data = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    let _ = window.paint_image(bounds, bounds, Corners::default(), data, 0, false);
}

fn background_of(paint: &Paint) -> Background {
    match paint {
        Paint::Solid(color) => Background::from(hsla_of(*color)),
        Paint::Linear { from, to, stops } => {
            // GPUI's gradient carries two stops, so a longer ramp keeps its
            // ends and loses the middle. A theme's gradients are authored in
            // palette pairs, which is what a stop pair expresses.
            let angle = gradient_angle(*from, *to);
            let first = stops.first().copied().unwrap_or(yunari_render::GradientStop {
                offset: 0.0,
                color: Color::BLACK,
            });
            let last = stops.last().copied().unwrap_or(first);
            linear_gradient(
                angle,
                linear_color_stop(hsla_of(first.color), first.offset),
                linear_color_stop(hsla_of(last.color), last.offset),
            )
        }
    }
}

fn colour_of(paint: &Paint) -> Hsla {
    match paint {
        Paint::Solid(color) => hsla_of(*color),
        Paint::Linear { stops, .. } => stops
            .first()
            .map_or(transparent_black(), |stop| hsla_of(stop.color)),
    }
}

/// The CSS angle of a gradient whose axis is `from → to`, in the y-down space
/// the display list is authored in: 0 points up, growing clockwise.
fn gradient_angle(from: (f32, f32), to: (f32, f32)) -> f32 {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let degrees = dx.atan2(-dy).to_degrees();
    (degrees + 360.0) % 360.0
}

fn hsla_of(color: Color) -> Hsla {
    Hsla::from(rgba(
        (u32::from(color.r) << 24)
            | (u32::from(color.g) << 16)
            | (u32::from(color.b) << 8)
            | u32::from(color.a),
    ))
}

#[cfg(test)]
mod tests {
    // Deliberately not `use super::*`: gpui's glob export carries its own `test`
    // attribute macro, which would shadow the built-in one here.
    use super::{Fit, axis_aligned, gradient_angle, hsla_of};
    use gpui_kit::{Bounds, Pixels, point, px, size};
    use yunari_render::{Color, DisplayList, Op, Shape};

    fn list(width: f32, height: f32) -> DisplayList {
        DisplayList {
            width,
            height,
            background: None,
            backdrop: None,
            ops: Vec::new(),
        }
    }

    fn bounds(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(width), px(height)),
        }
    }

    #[test]
    fn a_taller_surface_fits_by_width_and_centres() {
        let fit = Fit::new(&list(100.0, 50.0), bounds(200.0, 200.0));
        assert_eq!(fit.scale, 2.0);
        assert_eq!(fit.origin, (0.0, 50.0));
        assert_eq!(f32::from(fit.point(10.0, 10.0).y), 70.0);
    }

    #[test]
    fn a_caller_chosen_fit_places_the_artboard_where_it_says() {
        // An editor's viewport is just a Fit; painting must not recompute it.
        let fit = Fit {
            scale: 2.0,
            origin: (10.0, 20.0),
        };
        assert_eq!(fit.point(0.0, 0.0), point(px(10.0), px(20.0)));
        assert_eq!(fit.point(5.0, 5.0), point(px(20.0), px(30.0)));
    }

    #[test]
    fn a_wider_surface_fits_by_height() {
        let fit = Fit::new(&list(100.0, 50.0), bounds(400.0, 100.0));
        assert_eq!(fit.scale, 2.0);
        assert_eq!(fit.origin, (100.0, 0.0));
    }

    #[test]
    fn an_empty_artboard_still_yields_a_usable_fit() {
        let fit = Fit::new(&list(0.0, 0.0), bounds(10.0, 10.0));
        assert!(fit.scale.is_finite() && fit.scale > 0.0);
    }

    #[test]
    fn a_gradient_axis_reads_as_a_css_angle() {
        // Straight down the page.
        assert_eq!(gradient_angle((0.0, 0.0), (0.0, 10.0)), 180.0);
        // Straight right.
        assert_eq!(gradient_angle((0.0, 0.0), (10.0, 0.0)), 90.0);
        // Straight up.
        assert!((gradient_angle((0.0, 10.0), (0.0, 0.0)) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn a_solid_paint_is_the_authored_colour() {
        let colour = hsla_of(Color::rgb(255, 0, 0));
        assert!((colour.a - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_rectangle_only_counts_as_axis_aligned_when_it_is() {
        let mut op = Op {
            transform: yunari_render::Affine::IDENTITY,
            shape: Shape::Rect {
                width: 10.0,
                height: 10.0,
                corner_radius: 0.0,
            },
            fill: None,
            stroke: None,
        };
        assert!(axis_aligned(&op));
        op.transform = op
            .transform
            .then(yunari_render::Affine::rotate(45.0));
        assert!(!axis_aligned(&op));
    }
}
