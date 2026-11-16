//! The geometry of a selected node's control chrome: where the handles sit and
//! what dragging one does to the box.
//!
//! The maths is unit-tested here, away from gpui, so a rotated node, a side
//! that would invert and a uniform drag can be checked without a window.

use yunari_render::Affine;

/// A whole unit, which is what a free move lands on.
pub const MOVE_SNAP_STEP: f64 = 1.0;
/// The smallest a side may be dragged to, in artboard units.
pub const MIN_SIZE: f64 = 1.0;
/// How far above the top edge the rotation handle sits, in artboard units.
pub const ROTATE_OFFSET: f64 = 32.0;
/// A handle is grabbed within this many screen pixels.
pub const HANDLE_REACH: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Rotate,
}

impl Handle {
    pub const ALL: [Handle; 9] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Right,
        Handle::BottomRight,
        Handle::Bottom,
        Handle::BottomLeft,
        Handle::Left,
        Handle::Rotate,
    ];

    /// The handle's artboard position, turned with the box.
    pub fn position(self, rect: (f64, f64, f64, f64), rotation: f64) -> (f64, f64) {
        let (x, y, width, height) = rect;
        let (half_width, half_height) = (width / 2.0, height / 2.0);
        let local = match self {
            Handle::TopLeft => (-half_width, -half_height),
            Handle::Top => (0.0, -half_height),
            Handle::TopRight => (half_width, -half_height),
            Handle::Right => (half_width, 0.0),
            Handle::BottomRight => (half_width, half_height),
            Handle::Bottom => (0.0, half_height),
            Handle::BottomLeft => (-half_width, half_height),
            Handle::Left => (-half_width, 0.0),
            Handle::Rotate => (0.0, -half_height - ROTATE_OFFSET),
        };
        let (dx, dy) = Affine::rotate(rotation as f32).apply(local.0 as f32, local.1 as f32);
        (
            x + half_width + f64::from(dx),
            y + half_height + f64::from(dy),
        )
    }

    /// Every handle with the point it is drawn at.
    pub fn at(rect: (f64, f64, f64, f64), rotation: f64) -> Vec<(Self, (f64, f64))> {
        Self::ALL
            .into_iter()
            .map(|handle| (handle, handle.position(rect, rotation)))
            .collect()
    }
}

/// The handle under a pointer, if the pointer is near one.
pub fn handle_at(
    rect: (f64, f64, f64, f64),
    rotation: f64,
    x: f64,
    y: f64,
    zoom: f32,
) -> Option<Handle> {
    let reach = HANDLE_REACH / f64::from(zoom.max(f32::MIN_POSITIVE));
    Handle::at(rect, rotation)
        .into_iter()
        .find(|(_, (hx, hy))| (hx - x).abs() <= reach && (hy - y).abs() <= reach)
        .map(|(handle, _)| handle)
}

/// A free move lands on whole units unless Ctrl asks for no rounding at all.
pub fn move_to(_origin: (f64, f64), proposed: (f64, f64), snap: bool) -> (f64, f64) {
    if !snap {
        return proposed;
    }
    (
        (proposed.0 / MOVE_SNAP_STEP).round() * MOVE_SNAP_STEP,
        (proposed.1 / MOVE_SNAP_STEP).round() * MOVE_SNAP_STEP,
    )
}

/// The box a resize drag produces. Corner drags keep the aspect ratio unless
/// `uniform` is false; `from_centre` fixes the centre instead of the opposite
/// corner; a side never inverts and never shrinks below [`MIN_SIZE`].
pub fn resize(
    rect: (f64, f64, f64, f64),
    handle: Handle,
    pointer: (f64, f64),
    uniform: bool,
    from_centre: bool,
) -> (f64, f64, f64, f64) {
    let (x, y, width, height) = rect;
    let left = x;
    let top = y;
    let right = x + width;
    let bottom = y + height;
    let (pointer_x, pointer_y) = pointer;

    let corner = matches!(
        handle,
        Handle::TopLeft | Handle::TopRight | Handle::BottomLeft | Handle::BottomRight
    );
    let moves_x = matches!(
        handle,
        Handle::Left
            | Handle::Right
            | Handle::TopLeft
            | Handle::TopRight
            | Handle::BottomLeft
            | Handle::BottomRight
    );
    let moves_y = matches!(
        handle,
        Handle::Top
            | Handle::Bottom
            | Handle::TopLeft
            | Handle::TopRight
            | Handle::BottomLeft
            | Handle::BottomRight
    );

    if from_centre {
        let cx = x + width / 2.0;
        let cy = y + height / 2.0;
        let mut half_width = if moves_x {
            (pointer_x - cx).abs()
        } else {
            width / 2.0
        };
        let mut half_height = if moves_y {
            (pointer_y - cy).abs()
        } else {
            height / 2.0
        };
        if uniform && corner {
            let ratio = (half_width / (width / 2.0).max(MIN_SIZE))
                .max(half_height / (height / 2.0).max(MIN_SIZE));
            half_width = (width / 2.0) * ratio;
            half_height = (height / 2.0) * ratio;
        }
        half_width = half_width.max(MIN_SIZE / 2.0);
        half_height = half_height.max(MIN_SIZE / 2.0);
        return (
            cx - half_width,
            cy - half_height,
            half_width * 2.0,
            half_height * 2.0,
        );
    }

    let (mut new_left, mut new_right) = (left, right);
    let (mut new_top, mut new_bottom) = (top, bottom);
    if moves_x {
        match handle {
            Handle::Left | Handle::TopLeft | Handle::BottomLeft => new_left = pointer_x,
            _ => new_right = pointer_x,
        }
    }
    if moves_y {
        match handle {
            Handle::Top | Handle::TopLeft | Handle::TopRight => new_top = pointer_y,
            _ => new_bottom = pointer_y,
        }
    }
    let mut new_width = (new_right - new_left).max(MIN_SIZE);
    let mut new_height = (new_bottom - new_top).max(MIN_SIZE);
    if uniform && corner && width > 0.0 && height > 0.0 {
        let ratio = (new_width / width).max(new_height / height);
        new_width = (width * ratio).max(MIN_SIZE);
        new_height = (height * ratio).max(MIN_SIZE);
    }
    let anchor_x = match handle {
        Handle::Left | Handle::TopLeft | Handle::BottomLeft => right,
        _ => left,
    };
    let anchor_y = match handle {
        Handle::Top | Handle::TopLeft | Handle::TopRight => bottom,
        _ => top,
    };
    (
        anchor_x
            - if matches!(handle, Handle::Left | Handle::TopLeft | Handle::BottomLeft) {
                new_width
            } else {
                0.0
            },
        anchor_y
            - if matches!(handle, Handle::Top | Handle::TopLeft | Handle::TopRight) {
                new_height
            } else {
                0.0
            },
        new_width,
        new_height,
    )
}

/// The angle the pointer asks for, clockwise from the top, in whole degrees.
pub fn rotate(centre: (f64, f64), pointer: (f64, f64)) -> f64 {
    let dx = pointer.0 - centre.0;
    let dy = pointer.1 - centre.1;
    let degrees = dy.atan2(dx).to_degrees() + 90.0;
    degrees.rem_euclid(360.0).round().rem_euclid(360.0)
}

/// Arrow keys move a box by whole units.
pub fn nudge(rect: (f64, f64, f64, f64), dx: f64, dy: f64) -> (f64, f64) {
    (rect.0 + dx, rect.1 + dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_axes_land_on_whole_units() {
        assert_eq!(move_to((0.0, 0.0), (10.4, 10.6), true), (10.0, 11.0));
        assert_eq!(move_to((0.0, 0.0), (10.4, 10.6), false), (10.4, 10.6));
    }

    #[test]
    fn a_corner_drag_scales_uniformly_and_shift_frees_it() {
        let rect = (0.0, 0.0, 100.0, 50.0);
        let free = resize(rect, Handle::BottomRight, (200.0, 75.0), false, false);
        assert_eq!(free, (0.0, 0.0, 200.0, 75.0));
        let uniform = resize(rect, Handle::BottomRight, (200.0, 75.0), true, false);
        assert_eq!(uniform, (0.0, 0.0, 200.0, 100.0));
    }

    #[test]
    fn a_side_handle_moves_one_edge_and_never_inverts() {
        let rect = (0.0, 0.0, 100.0, 50.0);
        let dragged = resize(rect, Handle::Left, (200.0, 10.0), false, false);
        assert_eq!(dragged.0, 99.0, "the right edge stays put");
        assert_eq!(dragged.2, MIN_SIZE);
        assert_eq!(dragged.3, 50.0, "a left drag leaves the height alone");
    }

    #[test]
    fn resizing_from_the_centre_keeps_the_centre_fixed() {
        let rect = (0.0, 0.0, 100.0, 50.0);
        let resized = resize(rect, Handle::Right, (100.0, 25.0), false, true);
        assert_eq!(resized, (0.0, 0.0, 100.0, 50.0));
        let wider = resize(rect, Handle::Right, (150.0, 25.0), false, true);
        assert_eq!(wider, (-50.0, 0.0, 200.0, 50.0));
        let centre = |r: (f64, f64, f64, f64)| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        assert_eq!(centre(wider), centre(rect));
    }

    #[test]
    fn rotation_snaps_to_whole_degrees() {
        let centre = (0.0, 0.0);
        let angle = 44.6_f64.to_radians() - std::f64::consts::FRAC_PI_2;
        let pointer = (10.0 * angle.cos(), 10.0 * angle.sin());
        assert_eq!(rotate(centre, pointer), 45.0);
    }

    #[test]
    fn the_rotation_handle_sits_above_the_top_edge() {
        let rect = (0.0, 0.0, 100.0, 50.0);
        let (x, y) = Handle::Rotate.position(rect, 0.0);
        assert!((x - 50.0).abs() < 1e-9);
        assert!((y - (-ROTATE_OFFSET)).abs() < 1e-9);
    }

    #[test]
    fn a_handle_is_grabbed_within_six_screen_pixels() {
        let rect = (0.0, 0.0, 100.0, 50.0);
        assert_eq!(handle_at(rect, 0.0, 100.0, 25.0, 1.0), Some(Handle::Right));
        assert_eq!(handle_at(rect, 0.0, 100.0, 32.0, 1.0), None);
        // Zooming in shrinks six pixels' worth of artboard, so a point four
        // units away is reachable at 100 % and not at 200 %.
        assert_eq!(handle_at(rect, 0.0, 100.0, 29.0, 1.0), Some(Handle::Right));
        assert_eq!(handle_at(rect, 0.0, 100.0, 29.0, 2.0), None);
    }
}
