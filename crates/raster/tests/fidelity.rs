//! Kraken text and video fidelity, measured on the pixels a panel receives.
//!
//! The clock group scales its text down to 0.7399; nearest-resampled glyph
//! planes turn the shaper's antialiased edges into hard steps. Video decoded
//! to the panel box and sampled into a taller band cannot recover detail the
//! decoder discarded; the tap only mitigates the added bilinear softness.
//! Human review on the real panel is still required.

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
        "a fourfold video upscale must mitigate bilinear softness: {ramp:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The backdrop sampler must read the one picture the session already holds.
/// A second decode inside the sampler would fail this count even if the frame
/// still looked right; a timing proxy cannot prove that.
#[test]
fn sharpening_uses_the_one_decoded_picture() {
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
    let mut images = yunari_render_raster::Images::at(&dir);
    let list = yunari_render::DisplayList {
        ops: Vec::new(),
        ..list
    };
    let first = render(&list, 1.0, None, Some(&mut images));
    let after_first = images.decode_count();
    for _ in 0..20 {
        let later = render(&list, 1.0, None, Some(&mut images));
        assert_eq!(later.rgba, first.rgba, "the held picture stays stable");
    }
    eprintln!(
        "decodes: first={after_first}, after 21 frames={}",
        images.decode_count()
    );
    assert_eq!(after_first, 1, "one picture must be decoded once");
    assert_eq!(
        images.decode_count(),
        after_first,
        "sharpening must not decode a second frame per paint"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The sharpen trigger compares artboard cover size with decoded pixels. When
/// the artboard and panel differ, that only works after converting the cover
/// to device pixels. A fourfold device upscale must sharpen; a fourfold board
/// that is downscaled to the decoded size must not.
#[test]
fn sharpen_trigger_uses_device_pixels_not_artboard_units() {
    let dir = picture_dir("geometry");
    let theme = |width: u32, height: u32| {
        let json = format!(
            r##"{{"schemaVersion":"3.0.0","id":"geometry","artboard":{{"width":{width},"height":{height}}},
              "background":{{"media":{{"assetId":"edge","bounds":"theme","color":{{"value":"#000000"}}}}}},
              "assets":[{{"id":"edge","kind":"image","path":"assets/edge.png"}}],
              "scene":{{"version":"7.4.0","objects":[]}}}}"##
        );
        let document = yunari_model::document::parse_document(&json).expect("a theme");
        yunari_render::fabric::build_with_sources(
            &document,
            &mut Fonts::new(),
            &Readings::default(),
            &UnitPreferences::default(),
            &std::collections::HashMap::from([("assets/edge.png".to_owned(), (16.0_f32, 8.0_f32))]),
        )
        .0
    };

    fn middle_step(raster: &yunari_render_raster::Raster, y: u32) -> u32 {
        let row = |x: u32| raster.rgba[((y * raster.width + x) * 4) as usize];
        let ramp: Vec<u8> = (24..40).map(row).collect();
        let steps: Vec<u32> = ramp
            .windows(2)
            .map(|pair| (pair[1] as i32 - pair[0] as i32).unsigned_abs())
            .collect();
        steps[4] + steps[5] + steps[6] + steps[7]
    }

    // 16x8 board on a 64x32 panel: the sampler reads the 16x8 picture four
    // times larger, so the central edge must be steeper than plain bilinear.
    let enlarged = render(
        &yunari_render::DisplayList {
            ops: Vec::new(),
            ..theme(16, 8)
        },
        4.0,
        None,
        Some(&mut yunari_render_raster::Images::at(&dir)),
    );
    let enlarged_middle = middle_step(&enlarged, 16);
    eprintln!("device upscale middle={enlarged_middle}");
    assert!(enlarged.width == 64 && enlarged.height == 32);
    assert!(
        enlarged_middle > 170,
        "a device upscale must sharpen: middle={enlarged_middle}"
    );

    // 64x32 board painted at 0.25 into a 16x8 panel: the artboard cover is
    // four times the source, but the device cover is 1:1 and needs no tap.
    let reduced = render(
        &yunari_render::DisplayList {
            ops: Vec::new(),
            ..theme(64, 32)
        },
        0.25,
        None,
        Some(&mut yunari_render_raster::Images::at(&dir)),
    );
    assert!(reduced.width == 16 && reduced.height == 8);
    let reduced_row = |x: u32| reduced.rgba[((4 * 16 + x) * 4) as usize];
    let reduced_edge = (0..16).map(reduced_row).collect::<Vec<_>>();
    let reduced_peak = reduced_edge
        .windows(2)
        .map(|pair| (pair[1] as i32 - pair[0] as i32).unsigned_abs())
        .max()
        .unwrap_or(0);
    eprintln!("device 1:1 edge peak={reduced_peak}, row={reduced_edge:?}");
    assert!(
        reduced_edge
            .iter()
            .all(|value| *value == 0 || *value == 255),
        "a 1:1 device read must keep its source values: {reduced_edge:?}"
    );
    assert_eq!(reduced_peak, 255, "a 1:1 edge stays hard: {reduced_edge:?}");
    let _ = std::fs::remove_dir_all(dir);
}
