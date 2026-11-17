//! A theme's catalogue picture: the frame the raster backend draws, encoded as
//! the PNG a store listing shows.
//!
//! The size is the artboard's aspect within the listing's pixel budget, and a
//! circle theme is captured as a square because the disc is inscribed in it.

use std::collections::BTreeMap;

use image::ImageEncoder;
use yunari_model::document::ArtboardShape;

pub const CAPTURE_PIXELS: f64 = 819_200.0;
pub const MAX_THUMBNAIL_EDGE: u32 = 4096;
pub const MAX_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;

/// The size a host-side capture uses: the artboard's aspect within the
/// picture's pixel budget, and a square for a circle.
pub fn capture_size(width: f64, height: f64, shape: &ArtboardShape) -> (u32, u32) {
    let long = width.max(height);
    if matches!(shape, ArtboardShape::Circle) {
        let side = CAPTURE_PIXELS.sqrt().min(long).round().max(1.0);
        return (side as u32, side as u32);
    }
    let scale = if width * height > 0.0 {
        (CAPTURE_PIXELS / (width * height)).sqrt().min(1.0)
    } else {
        1.0
    };
    (
        (width * scale).round().max(1.0) as u32,
        (height * scale).round().max(1.0) as u32,
    )
}

/// Draws the document at its capture size and answers the PNG with the
/// picture's mean relative luminance.
pub fn of(
    document: &yunari_model::document::ThemeDocument,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<u8>, f64), String> {
    let (width, height) = capture_size(
        document.artboard.width,
        document.artboard.height,
        &document.artboard.shape,
    );
    if width > MAX_THUMBNAIL_EDGE || height > MAX_THUMBNAIL_EDGE {
        return Err(format!(
            "That picture would be {width}×{height}; a thumbnail may be at most {MAX_THUMBNAIL_EDGE} pixels on a side."
        ));
    }
    let mut fonts = yunari_render::text::Fonts::new();
    for refusal in fonts.load_faces(document, |path| assets.get(path).cloned()) {
        eprintln!("{refusal}");
    }
    let (list, _issues) = yunari_render::plan::build(document, &mut fonts);
    let mut images = yunari_render_raster::Images::empty();
    for (path, bytes) in assets {
        images.put(path, bytes);
    }
    let scale = if list.width > 0.0 {
        width as f32 / list.width
    } else {
        1.0
    };
    let raster = yunari_render_raster::render(&list, scale, Some(&mut fonts), Some(&mut images));
    if raster.width == 0 || raster.height == 0 {
        return Err("That theme could not be drawn.".to_owned());
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &raster.rgba,
            raster.width,
            raster.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
    if png.len() > MAX_THUMBNAIL_BYTES {
        return Err("That picture is larger than a thumbnail may be.".to_owned());
    }
    Ok((png, luminance(&raster.rgba)))
}

/// The mean relative luminance of a picture's pixels, 0 to 1.
fn luminance(rgba: &[u8]) -> f64 {
    let pixels = rgba.len() / 4;
    if pixels == 0 {
        return 0.0;
    }
    let total: f64 = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            0.2126 * linear(pixel[0]) + 0.7152 * linear(pixel[1]) + 0.0722 * linear(pixel[2])
        })
        .sum();
    total / pixels as f64
}

fn linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.039_28 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_is_the_artboards_aspect_within_the_budget() {
        assert_eq!(
            capture_size(1920.0, 1080.0, &ArtboardShape::Rect),
            (1207, 679)
        );
        assert_eq!(capture_size(400.0, 300.0, &ArtboardShape::Rect), (400, 300));
        assert_eq!(
            capture_size(1920.0, 1080.0, &ArtboardShape::Circle),
            (905, 905)
        );
        assert_eq!(capture_size(0.0, 0.0, &ArtboardShape::Rect), (1, 1));
    }

    #[test]
    fn a_drawn_theme_becomes_a_png_and_a_luminance() {
        let document = crate::store_install::test_document("aurora");
        let (png, luminance) = of(&document, &BTreeMap::new()).expect("a picture");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        // The starter ground is dark, and a theme with nothing drawn can still
        // be submitted because the picture exists.
        assert!(luminance < 0.15, "{luminance}");
    }

    #[test]
    fn luminance_runs_from_dark_to_light() {
        assert!(luminance(&[0, 0, 0, 255]) < 0.001);
        assert!(luminance(&[255, 255, 255, 255]) > 0.99);
        assert!(luminance(&[255, 0, 0, 255]) < luminance(&[0, 255, 0, 255]));
    }
}
