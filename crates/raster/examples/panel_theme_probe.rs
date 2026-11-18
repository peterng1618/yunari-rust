//! Throwaway feasibility probe, kept only so the result can be reproduced.
//!
//! Feeds one real installed Yunari theme through the native model, render and
//! raster crates. Every stage is an assertion: a failure exits non-zero with
//! the exact error and writes no output. No fallback, no schema conversion.
//!
//! Usage: cargo run -p yunari-render-raster --example panel_theme_probe -- <theme.json>

use std::path::PathBuf;

fn main() {
    if let Err(message) = run() {
        eprintln!("PROBE FAILED: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| "usage: panel_theme_probe <theme.json>".to_owned())?;

    let json = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    println!("theme bytes: {}", json.len());

    let document = yunari_model::document::parse_document(&json)
        .map_err(|error| format!("parse_document rejected the theme: {error}"))?;
    println!(
        "parse: OK — schemaVersion={}, id={}, nodes={}",
        document.schema_version,
        document.id,
        document.nodes.len()
    );

    let base = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;

    let mut fonts = yunari_render::Fonts::new();
    let refusals = fonts.load_faces(&document, |asset| std::fs::read(base.join(asset)).ok());
    println!("font refusals: {refusals:?}");

    let (list, issues) = yunari_render::build(&document, &mut fonts);
    println!(
        "display list: {} ops, {}×{} (artboard {}×{})",
        list.ops.len(),
        list.width,
        list.height,
        document.artboard.width,
        document.artboard.height
    );
    println!("plan issues ({}): {issues:?}", issues.len());
    if list.width != document.artboard.width as f32
        || list.height != document.artboard.height as f32
    {
        return Err(format!(
            "display list {}×{} does not match artboard {}×{}",
            list.width, list.height, document.artboard.width, document.artboard.height
        ));
    }

    let mut images = yunari_render_raster::Images::at(base);
    let raster = yunari_render_raster::render(&list, 1.0, Some(&mut fonts), Some(&mut images));
    println!(
        "raster: {}×{}, RGBA {} bytes ({} pixels)",
        raster.width,
        raster.height,
        raster.rgba.len(),
        raster.width as usize * raster.height as usize
    );
    if raster.width == 0 || raster.height == 0 || raster.rgba.is_empty() {
        return Err("render returned an empty frame".to_owned());
    }
    if raster.rgba.len() != raster.width as usize * raster.height as usize * 4 {
        return Err(format!(
            "RGBA length {} does not match {}×{}×4",
            raster.rgba.len(),
            raster.width,
            raster.height
        ));
    }

    let png = std::env::temp_dir().join(format!(
        "yunari-panel-theme-probe-{}-{}.png",
        std::process::id(),
        document.id
    ));
    image::save_buffer_with_format(
        &png,
        &raster.rgba,
        raster.width,
        raster.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|error| format!("could not write {}: {error}", png.display()))?;
    println!("png: {}", png.display());
    println!("PROBE OK");
    Ok(())
}
