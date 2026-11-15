#![forbid(unsafe_code)]
//! Pixel golden-file comparison for render tests.

use std::path::{Path, PathBuf};

use image::{ImageFormat, RgbaImage};

#[derive(Debug, Clone, Copy)]
pub struct GoldenConfig {
    /// A channel delta up to and including this value is not a difference.
    pub channel_tolerance: u8,
    /// Refuse when the share of differing pixels exceeds this.
    pub max_diff_ratio: f64,
}

impl Default for GoldenConfig {
    fn default() -> Self {
        Self {
            channel_tolerance: 2,
            max_diff_ratio: 0.002,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenMismatch {
    pub differing_pixels: usize,
    pub total_pixels: usize,
    pub max_channel_delta: u8,
    message: &'static str,
}

impl GoldenMismatch {
    pub fn message(&self) -> &'static str {
        self.message
    }
}

pub fn golden_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.png"))
}

pub fn compare(expected: &[u8], actual: &[u8], config: GoldenConfig) -> Result<(), GoldenMismatch> {
    if expected.len() != actual.len() {
        return Err(GoldenMismatch {
            differing_pixels: 0,
            total_pixels: expected.len() / 4,
            max_channel_delta: 0,
            message: "expected and actual buffers differ in length",
        });
    }

    let total_pixels = expected.len() / 4;
    if total_pixels == 0 {
        return Ok(());
    }

    let mut differing_pixels = 0usize;
    let mut max_channel_delta = 0u8;
    for (expected, actual) in expected
        .as_chunks::<4>()
        .0
        .iter()
        .zip(actual.as_chunks::<4>().0.iter())
    {
        let delta = expected
            .iter()
            .zip(actual.iter())
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .unwrap_or(0);
        max_channel_delta = max_channel_delta.max(delta);
        if delta > config.channel_tolerance {
            differing_pixels += 1;
        }
    }

    let ratio = differing_pixels as f64 / total_pixels as f64;
    let limit = config.max_diff_ratio.max(0.0);
    if ratio > limit {
        return Err(GoldenMismatch {
            differing_pixels,
            total_pixels,
            max_channel_delta,
            message: "pixel differences exceed the golden budget",
        });
    }
    Ok(())
}

pub fn write_golden(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let image = RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "pixel buffer size")
    })?;
    image
        .save_with_format(golden_path(dir, name), ImageFormat::Png)
        .map_err(std::io::Error::other)
}

pub fn assert_golden(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    config: GoldenConfig,
) {
    if std::env::var_os("YUNARI_UPDATE_GOLDEN").is_some_and(|value| value == "1") {
        write_golden(dir, name, width, height, rgba)
            .unwrap_or_else(|error| panic!("could not write golden {name}: {error}"));
        return;
    }

    let path = golden_path(dir, name);
    let image = image::open(&path)
        .unwrap_or_else(|error| panic!("could not read golden {}: {error}", path.display()))
        .to_rgba8();
    let (golden_width, golden_height) = image.dimensions();
    assert_eq!(
        (golden_width, golden_height),
        (width, height),
        "golden {} has a different size",
        path.display()
    );
    if let Err(mismatch) = compare(image.as_raw(), rgba, config) {
        panic!(
            "golden {} differs: {} of {} pixels, max channel delta {}, ({})",
            path.display(),
            mismatch.differing_pixels,
            mismatch.total_pixels,
            mismatch.max_channel_delta,
            mismatch.message()
        );
    }
}
