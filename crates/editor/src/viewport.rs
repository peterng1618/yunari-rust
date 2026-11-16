//! The camera: what part of the artboard the canvas shows, and at what scale.
//!
//! The numbers are the source editor's, so a theme framed there frames the same
//! here: a scale ladder of 1.1, a zoom range of 2 % to 6400 %, and panning that
//! may overscroll the artboard by a fixed margin but no further.

use gpui_kit::{Bounds, Pixels, Point, point, px};

pub const MIN_ZOOM: f32 = 0.02;
pub const MAX_ZOOM: f32 = 64.0;
/// One press of `+` or `-`.
pub const ZOOM_STEP: f32 = 1.1;
/// A ctrl+wheel delta is divided by this before becoming a zoom exponent, so
/// the zoom follows the wheel as a continuous curve rather than a ladder.
pub const WHEEL_ZOOM_DIVISOR: f32 = 1000.0;
/// How far the artboard may be pushed past the canvas edge.
pub const PAN_OVERSCROLL_MARGIN: f32 = 48.0;
/// One line of a line-based wheel delta, in pixels.
pub const LINE_HEIGHT: f32 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub scale: f32,
    /// Where the artboard's top-left lands, in canvas pixels.
    pub offset: (f32, f32),
}

impl Viewport {
    /// The whole artboard, centred, as large as fits.
    pub fn contain(artboard: (f64, f64), bounds: Bounds<Pixels>) -> Self {
        let (width, height) = surface(bounds);
        let board_width = artboard.0 as f32;
        let board_height = artboard.1 as f32;
        let scale = (width / board_width.max(1.0))
            .min(height / board_height.max(1.0))
            .clamp(MIN_ZOOM, MAX_ZOOM);
        Self {
            scale,
            offset: (
                (width - board_width * scale) / 2.0,
                (height - board_height * scale) / 2.0,
            ),
        }
    }

    /// The artboard at 100 %, centred.
    pub fn reset(bounds: Bounds<Pixels>, artboard: (f64, f64)) -> Self {
        let (width, height) = surface(bounds);
        Self {
            scale: 1.0,
            offset: (
                (width - artboard.0 as f32) / 2.0,
                (height - artboard.1 as f32) / 2.0,
            ),
        }
    }

    /// The artboard point under a canvas pixel.
    pub fn to_scene(&self, point: Point<Pixels>) -> (f64, f64) {
        let scale = self.scale.max(MIN_ZOOM);
        (
            f64::from((f32::from(point.x) - self.offset.0) / scale),
            f64::from((f32::from(point.y) - self.offset.1) / scale),
        )
    }

    /// Where an artboard point lands, in canvas pixels.
    pub fn to_screen(&self, x: f64, y: f64) -> Point<Pixels> {
        point(
            px(self.offset.0 + x as f32 * self.scale),
            px(self.offset.1 + y as f32 * self.scale),
        )
    }

    /// Zooms about the canvas centre, which is what `+` and `-` do.
    pub fn zoom_by(&mut self, factor: f32, bounds: Bounds<Pixels>, artboard: (f64, f64)) {
        let (width, height) = surface(bounds);
        self.zoom_at(
            factor,
            point(px(width / 2.0), px(height / 2.0)),
            bounds,
            artboard,
        );
    }

    /// Zooms about a canvas pixel, so the artboard point under the cursor stays
    /// under the cursor.
    pub fn zoom_at(
        &mut self,
        factor: f32,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        artboard: (f64, f64),
    ) {
        let (scene_x, scene_y) = self.to_scene(cursor);
        let next = (self.scale * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.scale = next;
        self.offset = (
            f32::from(cursor.x) - scene_x as f32 * next,
            f32::from(cursor.y) - scene_y as f32 * next,
        );
        self.clamp(bounds, artboard);
    }

    /// Moves the camera by a canvas-pixel delta.
    pub fn pan_by(&mut self, dx: f32, dy: f32, bounds: Bounds<Pixels>, artboard: (f64, f64)) {
        self.offset = (self.offset.0 + dx, self.offset.1 + dy);
        self.clamp(bounds, artboard);
    }

    /// The whole artboard, centred again.
    pub fn fit(&mut self, bounds: Bounds<Pixels>, artboard: (f64, f64)) {
        *self = Self::contain(artboard, bounds);
    }

    /// Keeps the artboard from being pushed off the canvas: its left edge may
    /// reach `margin - width` and no further, its right edge `canvas - margin`.
    fn clamp(&mut self, bounds: Bounds<Pixels>, artboard: (f64, f64)) {
        let (width, height) = surface(bounds);
        let scaled_width = artboard.0 as f32 * self.scale;
        let scaled_height = artboard.1 as f32 * self.scale;
        self.offset.0 = clamp_axis(
            self.offset.0,
            PAN_OVERSCROLL_MARGIN - scaled_width,
            width - PAN_OVERSCROLL_MARGIN,
        );
        self.offset.1 = clamp_axis(
            self.offset.1,
            PAN_OVERSCROLL_MARGIN - scaled_height,
            height - PAN_OVERSCROLL_MARGIN,
        );
    }
}

/// The zoom a wheel delta asks for under ctrl, as a factor.
pub fn wheel_factor(delta: f32) -> f32 {
    (-delta / WHEEL_ZOOM_DIVISOR).exp()
}

fn surface(bounds: Bounds<Pixels>) -> (f32, f32) {
    (f32::from(bounds.size.width), f32::from(bounds.size.height))
}

/// A canvas smaller than the margin pair would invert the range, and clamping
/// to an inverted range panics, so the ends are ordered once here.
fn clamp_axis(value: f32, low: f32, high: f32) -> f32 {
    value.clamp(low.min(high), high.max(low))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: (f64, f64) = (450.0, 675.0);

    fn canvas(width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size_of(width, height),
        }
    }

    fn size_of(width: f32, height: f32) -> gpui_kit::Size<Pixels> {
        gpui_kit::size(px(width), px(height))
    }

    #[test]
    fn contain_fits_the_whole_artboard_and_centres_it() {
        // 450x675 artboard in a 900x675 canvas -> scale 1, centred at x=225.
        let view = Viewport::contain(BOARD, canvas(900.0, 675.0));
        assert_eq!(view.scale, 1.0);
        assert_eq!(view.offset, (225.0, 0.0));
    }

    #[test]
    fn zooming_at_a_cursor_keeps_the_scene_point_under_it() {
        let mut view = Viewport::contain(BOARD, canvas(900.0, 675.0));
        let cursor = point(px(300.0), px(300.0));
        let before = view.to_scene(cursor);
        view.zoom_at(1.1, cursor, canvas(900.0, 675.0), BOARD);
        let after = view.to_scene(cursor);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
    }

    #[test]
    fn the_scale_is_clamped_to_the_source_limits() {
        let mut view = Viewport::contain(BOARD, canvas(900.0, 675.0));
        for _ in 0..200 {
            view.zoom_by(ZOOM_STEP, canvas(900.0, 675.0), BOARD);
        }
        assert_eq!(view.scale, MAX_ZOOM);
        for _ in 0..400 {
            view.zoom_by(1.0 / ZOOM_STEP, canvas(900.0, 675.0), BOARD);
        }
        assert_eq!(view.scale, MIN_ZOOM);
    }

    #[test]
    fn panning_stops_at_the_overscroll_margin() {
        let mut view = Viewport::contain(BOARD, canvas(900.0, 675.0));
        view.pan_by(10_000.0, 10_000.0, canvas(900.0, 675.0), BOARD);
        assert_eq!(view.offset.0, 900.0 - PAN_OVERSCROLL_MARGIN);
        assert_eq!(view.offset.1, 675.0 - PAN_OVERSCROLL_MARGIN);
        view.pan_by(-10_000.0, -10_000.0, canvas(900.0, 675.0), BOARD);
        assert_eq!(view.offset.0, PAN_OVERSCROLL_MARGIN - 450.0);
        assert_eq!(view.offset.1, PAN_OVERSCROLL_MARGIN - 675.0);
    }

    #[test]
    fn a_wheel_zoom_follows_the_exponential_the_source_uses() {
        assert!((wheel_factor(100.0) - (-0.1f32).exp()).abs() < 1e-6);
        assert!(wheel_factor(100.0) < 1.0);
        assert!(wheel_factor(-100.0) > 1.0);
        assert_eq!(wheel_factor(0.0), 1.0);
    }

    #[test]
    fn reset_centres_the_artboard_at_100_percent() {
        let view = Viewport::reset(canvas(900.0, 675.0), BOARD);
        assert_eq!(view.scale, 1.0);
        assert_eq!(view.offset, (225.0, 0.0));
    }

    #[test]
    fn a_canvas_round_trip_returns_the_same_scene_point() {
        let view = Viewport::contain(BOARD, canvas(900.0, 675.0));
        let screen = view.to_screen(120.0, 340.0);
        let scene = view.to_scene(screen);
        assert!((scene.0 - 120.0).abs() < 1e-3 && (scene.1 - 340.0).abs() < 1e-3);
    }
}
