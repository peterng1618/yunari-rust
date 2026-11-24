//! Kraken text and video fidelity, measured on the pixels a panel receives.
//!
//! The clock group scales its text down to 0.7399; nearest-resampled glyph
//! planes turn the shaper's antialiased edges into hard steps. Video from a
//! 16:9 clip is decoded to the panel box and then sampled into a taller
//! themed band, so the upscale must keep an edge instead of smoothing it flat.

use std::path::PathBuf;

use yunari_providers::units::UnitPreferences;
use yunari_render::{Affine, DisplayList, Fonts, Op, Readings};
use yunari_render_raster::render;

const TEXT_THEME: &str = r##"{
  "schemaVersion":"3.0.0","id":"kraken-text","artboard":{"width":200,"height":80},
  "background":{"color":{"value":"#000000"}},
  "globals":{"palette":{"none":{"name":"None","value":{"kind":"solid","color":"transparent"}}},
    "typeTokens":{"heading":{"family":"serif","weight":"bold"},
    "body":{"family":"serif","weight":"regular"},
    "mono":{"family":"monospace","weight":"regular"}}},
  "scene":{"version":"7.4.0","objects":[
    {"id":"clock","type":"Textbox","left":0,"top":0,"width":200,"height":null,
     "text":"00:00","fontSize":70,"fill":"#ffffff",
     "yunariText":{"box":{"width":200,"height":80},"runs":[
       {"kind":"literal","text":"00:00","typeToken":"typeTokens.heading",
        "style":{"fontSize":{"value":70},"color":{"value":"#ffffff"}}}]}}]}}"##;

fn text_list() -> DisplayList {
    let document = yunari_model::document::parse_document(TEXT_THEME).expect("a theme");
    let (list, issues) = yunari_render::fabric::build(
        &document,
        &mut Fonts::new(),
        &Readings::default(),
        &UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "text plan: {issues:?}");
    assert!(
        list.ops
            .iter()
            .any(|op| matches!(op.shape, yunari_render::Shape::Text(_))),
        "a text op must be planned"
    );
    list
}

#[test]
fn kraken_clock_text_keeps_antialiased_edges_under_group_scale() {
    let list = text_list();
    let scaled = DisplayList {
        ops: list
            .ops
            .iter()
            .map(|op| Op {
                transform: op.transform.then(Affine::scale(0.7399, 0.7399)),
                ..op.clone()
            })
            .collect(),
        ..list.clone()
    };
    let raster = render(&scaled, 1.0, Some(&mut Fonts::new()), None);
    // Luminance on a black fill carries glyph coverage. Nearest sampling
    // stretches one value across many pixels; bilinear keeps the shaper's
    // intermediate coverage in a short ramp at each edge.
    let luma = |x: u32, y: u32| raster.rgba[((y * raster.width + x) * 4) as usize];
    let mut edge_pixels = 0;
    for y in 12..58 {
        for x in 5..(raster.width - 1) {
            if luma(x, y) != luma(x + 1, y) {
                edge_pixels += 1;
            }
        }
    }
    eprintln!("scaled clock glyphs: edge_pixels={edge_pixels}");
    assert!(
        edge_pixels > 600,
        "downscaled clock edges must keep intermediate coverage: edge_pixels={edge_pixels}"
    );
}

fn picture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yunari-fidelity-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("an assets dir");
    // A single hard vertical edge. The panel samples this 16-pixel picture into
    // a 64-pixel box, the same fourfold upscale a short video band asks for.
    let mut picture = image::RgbaImage::new(16, 8);
    for (x, _, pixel) in picture.enumerate_pixels_mut() {
        let value = if x < 8 { 0 } else { 255 };
        *pixel = image::Rgba([value, value, value, 255]);
    }
    picture
        .save(dir.join("assets/edge.png"))
        .expect("the edge picture is written");
    dir
}

#[test]
fn an_upscaled_video_edge_keeps_its_gradient() {
    let dir = picture_dir("edge");
    let document = yunari_model::document::parse_document(
        r##"{"schemaVersion":"3.0.0","id":"edge","artboard":{"width":64,"height":32},
          "background":{"media":{"assetId":"edge","bounds":"theme","color":{"value":"#000000"}}},
          "assets":[{"id":"edge","kind":"image","path":"assets/edge.png"}],
          "nodes":[{"id":"n","kind":{"type":"image",
            "content":{"assetId":"edge","fit":"stretch"}},
            "transform":{"x":0,"y":0,"width":64,"height":32}}]}"##,
    )
    .expect("a theme");
    // The sidecar hands the planner the decoded clip size; without it a simple
    // background has no crop and a sampler change cannot reach the pixels.
    let list = yunari_render::fabric::build_with_sources(
        &document,
        &mut Fonts::new(),
        &Readings::default(),
        &UnitPreferences::default(),
        &std::collections::HashMap::from([("assets/edge.png".to_owned(), (16.0_f32, 8.0_f32))]),
    )
    .0;
    let raster = render(
        &yunari_render::DisplayList {
            ops: Vec::new(),
            ..list
        },
        1.0,
        None,
        Some(&mut yunari_render_raster::Images::at(&dir)),
    );
    assert_eq!((raster.width, raster.height), (64, 32));
    let row = |x: u32| raster.rgba[((16 * 64 + x) * 4) as usize];
    // A fourfold bilinear ramp spreads the edge over four pixels at ~64 per
    // step. Cubic interpolation steepens the middle steps and keeps the two
    // fields flat; the test reads the finished frame, not the sampler.
    let ramp: Vec<u8> = (24..40).map(row).collect();
    let steps: Vec<u32> = ramp
        .windows(2)
        .map(|pair| (pair[1] as i32 - pair[0] as i32).unsigned_abs())
        .collect();
    let middle = steps[4] + steps[5] + steps[6] + steps[7];
    eprintln!("upscaled edge ramp: {ramp:?}, steps={steps:?}, middle={middle}");
    assert!(
        row(4) < 16 && row(60) > 240,
        "both fields must survive the fit"
    );
    assert!(
        middle > 170,
        "a fourfold video upscale must keep a steep central edge: {ramp:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The fidelity changes must not buy sharpness with another full-resolution
/// decode: one panel-sized frame is still the only input the sampler gets.
#[test]
fn sharpening_does_not_add_a_second_decode() {
    let dir = picture_dir("budget");
    let document = yunari_model::document::parse_document(
        r##"{"schemaVersion":"3.0.0","id":"budget","artboard":{"width":64,"height":32},
          "background":{"media":{"assetId":"edge","bounds":"theme","color":{"value":"#000000"}}},
          "assets":[{"id":"edge","kind":"image","path":"assets/edge.png"}],
          "scene":{"version":"7.4.0","objects":[]}}"##,
    )
    .expect("a theme");
    let sources =
        std::collections::HashMap::from([("assets/edge.png".to_owned(), (16.0_f32, 8.0_f32))]);
    let list = yunari_render::fabric::build_with_sources(
        &document,
        &mut Fonts::new(),
        &Readings::default(),
        &UnitPreferences::default(),
        &sources,
    )
    .0;
    let started = std::time::Instant::now();
    let mut frames = 0;
    while started.elapsed() < std::time::Duration::from_millis(200) {
        let mut images = yunari_render_raster::Images::at(&dir);
        let raster = render(
            &yunari_render::DisplayList {
                ops: Vec::new(),
                ..list.clone()
            },
            1.0,
            None,
            Some(&mut images),
        );
        assert_eq!(raster.rgba.len() as u32, 64 * 32 * 4);
        frames += 1;
    }
    let per_frame = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    eprintln!("{frames} upscaled frames, {per_frame:.2} ms per frame (debug build)");
    assert!(
        per_frame < 20.0,
        "sharpening must not cost another decode: {per_frame:.2} ms per frame"
    );
    let _ = std::fs::remove_dir_all(dir);
}
