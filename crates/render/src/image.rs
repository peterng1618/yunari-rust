//! Where a picture sits inside the box that holds it.
//!
//! A theme's image node names an asset and a fit; the picture's own size only
//! arrives when the bytes are decoded. The arithmetic that turns those two into
//! a rectangle lives here, once, so every backend places an image the same way
//! and a golden can check it without decoding anything.

use yunari_model::document::ImageFit;

/// The rectangle a picture is drawn into, in the box's own coordinates. A
/// `Cover` placement is larger than the box and overflows equally on two sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Place a `intrinsic`-sized picture in a `box_`-sized box, centred.
///
/// `Contain` fits inside and leaves the box's spare room empty, `Cover` fills
/// the box and overflows, `Stretch` takes the box's shape whatever the
/// picture's. A picture with no size cannot be fitted, so it takes the box and
/// the caller reports it missing.
pub fn placement(intrinsic: (f32, f32), box_: (f32, f32), fit: ImageFit) -> Placement {
    let (box_width, box_height) = (box_.0.max(0.0), box_.1.max(0.0));
    let (width, height) = (intrinsic.0, intrinsic.1);
    if width <= 0.0 || height <= 0.0 {
        return Placement {
            x: 0.0,
            y: 0.0,
            width: box_width,
            height: box_height,
        };
    }
    if fit == ImageFit::Stretch {
        return Placement {
            x: 0.0,
            y: 0.0,
            width: box_width,
            height: box_height,
        };
    }
    let ratio_x = box_width / width;
    let ratio_y = box_height / height;
    let ratio = match fit {
        ImageFit::Cover => ratio_x.max(ratio_y),
        _ => ratio_x.min(ratio_y),
    };
    let drawn = (width * ratio, height * ratio);
    Placement {
        x: (box_width - drawn.0) / 2.0,
        y: (box_height - drawn.1) / 2.0,
        width: drawn.0,
        height: drawn.1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contain_letterboxes_a_square_in_a_wide_box() {
        let placed = placement((50.0, 50.0), (100.0, 50.0), ImageFit::Contain);
        assert_eq!(placed, Placement { x: 25.0, y: 0.0, width: 50.0, height: 50.0 });
    }

    #[test]
    fn contain_pillarboxes_a_wide_picture_in_a_tall_box() {
        let placed = placement((200.0, 100.0), (100.0, 100.0), ImageFit::Contain);
        assert_eq!(placed, Placement { x: 0.0, y: 25.0, width: 100.0, height: 50.0 });
    }

    #[test]
    fn cover_overflows_the_box_equally_on_two_sides() {
        let placed = placement((100.0, 100.0), (200.0, 100.0), ImageFit::Cover);
        assert_eq!(placed, Placement { x: 0.0, y: -50.0, width: 200.0, height: 200.0 });
    }

    #[test]
    fn stretch_takes_the_box_and_ignores_the_picture_shape() {
        let placed = placement((10.0, 40.0), (100.0, 25.0), ImageFit::Stretch);
        assert_eq!(placed, Placement { x: 0.0, y: 0.0, width: 100.0, height: 25.0 });
    }

    #[test]
    fn a_sizeless_picture_falls_back_to_the_box() {
        let placed = placement((0.0, 0.0), (30.0, 20.0), ImageFit::Contain);
        assert_eq!(placed, Placement { x: 0.0, y: 0.0, width: 30.0, height: 20.0 });
    }
}
