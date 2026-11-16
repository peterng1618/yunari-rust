//! A 2×3 affine transform, in the order a backend already speaks:
//! `a b c d e f`, mapping `(x, y)` to `(a x + c y + e, b x + d y + f)`.

use serde::Serialize;
use yunari_model::document::Transform;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    pub fn scale(x: f32, y: f32) -> Self {
        Self {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    /// `degrees` turns clockwise on screen, where `y` grows downwards.
    pub fn rotate(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// The transform that applies `self` and then `next`.
    pub fn then(self, next: Self) -> Self {
        Self {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            e: next.a * self.e + next.c * self.f + next.e,
            f: next.b * self.e + next.d * self.f + next.f,
        }
    }

    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The transform that undoes this one, or `None` when it flattens the
    /// plane and nothing can be mapped back.
    pub fn inverse(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        if determinant.abs() < 1e-9 {
            return None;
        }
        Some(Self {
            a: self.d / determinant,
            b: -self.b / determinant,
            c: -self.c / determinant,
            d: self.a / determinant,
            e: (self.c * self.f - self.d * self.e) / determinant,
            f: (self.b * self.e - self.a * self.f) / determinant,
        })
    }

    /// A node's authored transform, in artboard units.
    ///
    /// Rotation and scale turn about the box's centre, which is the origin the
    /// authored documents carry from the editor they were drawn in — a
    /// quarter-turned panel keeps the place its author gave it.
    pub fn from_node(transform: &Transform) -> Self {
        let (x, y) = (transform.x as f32, transform.y as f32);
        let (width, height) = (transform.width as f32, transform.height as f32);
        let (half_width, half_height) = (width / 2.0, height / 2.0);
        Self::translate(-half_width, -half_height)
            .then(Self::rotate(transform.rotation as f32))
            .then(Self::scale(
                transform.scale_x as f32,
                transform.scale_y as f32,
            ))
            .then(Self::translate(half_width, half_height))
            .then(Self::translate(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: (f32, f32), expected: (f32, f32)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-4 && (actual.1 - expected.1).abs() < 1e-4,
            "{actual:?} != {expected:?}"
        );
    }

    fn node(x: f64, y: f64, width: f64, height: f64, rotation: f64, scale: f64) -> Transform {
        Transform {
            x,
            y,
            width,
            height,
            rotation,
            scale_x: scale,
            scale_y: scale,
        }
    }

    #[test]
    fn identity_keeps_a_point() {
        assert_close(Affine::IDENTITY.apply(3.0, 4.0), (3.0, 4.0));
    }

    #[test]
    fn a_translation_moves_without_turning() {
        assert_close(Affine::translate(10.0, 20.0).apply(1.0, 2.0), (11.0, 22.0));
    }

    #[test]
    fn a_quarter_turn_about_the_centre_keeps_the_box() {
        let transform = node(0.0, 0.0, 10.0, 10.0, 90.0, 1.0);
        let affine = Affine::from_node(&transform);
        assert_close(affine.apply(0.0, 0.0), (10.0, 0.0));
        assert_close(affine.apply(10.0, 10.0), (0.0, 10.0));
    }

    #[test]
    fn a_scale_grows_about_the_centre() {
        let transform = node(0.0, 0.0, 10.0, 10.0, 0.0, 2.0);
        let affine = Affine::from_node(&transform);
        assert_close(affine.apply(5.0, 5.0), (5.0, 5.0));
        assert_close(affine.apply(0.0, 0.0), (-5.0, -5.0));
    }

    #[test]
    fn a_parent_transform_composes_over_its_child() {
        let parent = Affine::from_node(&node(10.0, 0.0, 4.0, 4.0, 0.0, 1.0));
        let child = Affine::from_node(&node(5.0, 0.0, 2.0, 2.0, 0.0, 1.0));
        assert_close(parent.then(child).apply(0.0, 0.0), (15.0, 0.0));
    }

    #[test]
    fn an_inverse_lands_back_on_the_point_it_moved() {
        let affine = Affine::from_node(&node(3.0, 7.0, 20.0, 20.0, 30.0, 2.0));
        let inverse = affine.inverse().expect("a turned, scaled box inverts");
        assert_close(inverse.apply(affine.apply(11.0, 4.0).0, affine.apply(11.0, 4.0).1), (11.0, 4.0));
    }

    #[test]
    fn a_flattened_transform_has_no_inverse() {
        assert!(Affine::scale(0.0, 1.0).inverse().is_none());
    }
}
