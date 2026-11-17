//! Headless observations for the coverage-gap audit. Each mode builds one
//! small theme through the port's public crates and prints what it painted.

use std::path::PathBuf;
use std::time::Duration;

use yunari_model::document::{parse_document, ThemeDocument};
use yunari_render::text::{
    self, TextAlign, TextOverflow, TextRequest, TextRunPlan, TextStyle, VerticalAlign,
};
use yunari_render::{DisplayList, Fonts, Shape};
use yunari_render_raster::{Images, Raster, render};

const FAVICON: &[u8] = include_bytes!("../../../../../crates/host/assets/favicon.svg");
const MOTION: &[u8] = include_bytes!("../fixtures/motion.mp4");

fn rgba(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * raster.width + x) * 4) as usize;
    [
        raster.rgba[at],
        raster.rgba[at + 1],
        raster.rgba[at + 2],
        raster.rgba[at + 3],
    ]
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yunari-coverage-gaps-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("scratch dir");
    dir
}

fn theme(json: &str) -> (ThemeDocument, DisplayList) {
    let document = parse_document(json).unwrap_or_else(|error| panic!("theme: {error}"));
    let (list, issues) = yunari_render::build(&document, &mut Fonts::new());
    assert!(issues.is_empty(), "{issues:?}");
    (document, list)
}

fn list(json: &str) -> DisplayList {
    theme(json).1
}

fn ffmpeg_available() -> bool {
    let named = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(path) = std::env::var_os("YUNARI_FFMPEG")
        && PathBuf::from(path).is_file()
    {
        return true;
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(named))
            .any(|candidate| candidate.is_file())
    })
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    match mode.as_str() {
        "svg" => svg(),
        "avif" => avif(),
        "vertical" => vertical(),
        "letter-spacing" => letter_spacing(),
        "overflow" => overflow(),
        "short-box" => short_box(),
        "glass-grain" => glass_grain(),
        "glass-sampling" => glass_sampling(),
        "content-fit" => content_fit(),
        "video" => video(),
        other => panic!("unknown mode {other:?}"),
    }
}

/// Source renders an SVG through Fabric's `<img>`, rasterised into the image
/// object (`scene-fabric/fabric-image.ts:52-63`). The port's image store only
/// asks `image::load_from_memory`, which has no SVG codec.
fn svg() {
    let dir = scratch("svg");
    std::fs::write(dir.join("assets/logo.svg"), FAVICON).expect("write svg");
    let list = list(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":40,"height":40},
            "background":{},
            "assets":[{"id":"logo","kind":"svg","path":"assets/logo.svg"}],
            "nodes":[{"id":"i","transform":{"x":0,"y":0,"width":40,"height":40},
            "kind":{"type":"image","content":{"assetId":"logo","fit":"stretch"}}}]}"##,
    );
    let mut images = Images::at(dir);
    let raster = render(&list, 1.0, None, Some(&mut images));
    println!("svg center pixel {:?}", rgba(&raster, 20, 20));
    println!(
        "svg max alpha {}",
        raster.rgba.chunks(4).map(|pixel| pixel[3]).max().unwrap()
    );
}

/// Source admits `avif` as an image extension and serves `image/avif`
/// (`renderer-core/theme/asset-media-type.ts:18,35`).
fn avif() {
    let result = parse_document(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":40,"height":40},
            "assets":[{"id":"pic","kind":"image","path":"assets/pic.avif"}],
            "nodes":[]}"##,
    );
    match result {
        Ok(_) => println!("avif accepted"),
        Err(error) => println!("avif refused: {error}"),
    }
}

/// Source stacks grapheme clusters and drops authored line breaks
/// (`scene-fabric/text-runs.ts:39-54`).
fn vertical() {
    let runs = vec![TextRunPlan {
        text: "👨‍👩‍👧‍👦A\r\nB".to_owned(),
        style: TextStyle {
            size: 20.0,
            ..TextStyle::default()
        },
    }];
    let request = TextRequest {
        runs: &runs,
        box_: None,
        align: TextAlign::Left,
        vertical_align: VerticalAlign::Top,
        overflow: TextOverflow::Visible,
        vertical: true,
        line_height: 1.2,
    };
    let laid = text::layout(&mut Fonts::new(), &request);
    println!("vertical segment count {}", laid.segments.len());
    for segment in &laid.segments {
        println!("segment {:?}", segment.text);
    }
}

/// Source applies a run's `letterSpacing` as Fabric object `charSpacing`
/// (`scene-fabric/paint.ts:67-88`).
fn letter_spacing() {
    let with = list(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":200,"height":100},
            "typeTokens":{"body":{"family":"Arial"}},
            "nodes":[{"id":"t","transform":{"x":0,"y":0,"width":200,"height":100},
            "kind":{"type":"text","content":{"runs":[
            {"kind":"literal","text":"AB","typeToken":"typeTokens.body",
             "style":{"fontSize":{"value":20},"letterSpacing":{"value":10}}}]}}}]}"##,
    );
    let without = list(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":200,"height":100},
            "typeTokens":{"body":{"family":"Arial"}},
            "nodes":[{"id":"t","transform":{"x":0,"y":0,"width":200,"height":100},
            "kind":{"type":"text","content":{"runs":[
            {"kind":"literal","text":"AB","typeToken":"typeTokens.body",
             "style":{"fontSize":{"value":20}}}]}}}]}"##,
    );
    println!(
        "display lists equal with and without letterSpacing: {}",
        with == without
    );
    for (name, list) in [("with", &with), ("without", &without)] {
        if let Some(Shape::Text(layout)) = list
            .ops
            .iter()
            .map(|op| &op.shape)
            .find(|shape| matches!(shape, Shape::Text(_)))
        {
            println!("{name}: {} segments", layout.segments.len());
            for segment in &layout.segments {
                println!(
                    "  {:?} at {:?} style {:?}",
                    segment.text, segment.origin, segment.style
                );
            }
        }
    }
}

/// Source honours `overflow: visible` by deleting the clip path and painting
/// every line (`scene-fabric/authored-box.ts:320-328`). The port drops every
/// line past the box height for every overflow mode
/// (`crates/render/src/text.rs:241-247`).
fn overflow() {
    let json = r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":200,"height":200},
        "typeTokens":{"body":{"family":"Arial"}},
        "nodes":[{"id":"t","transform":{"x":0,"y":0,"width":100,"height":48},
        "kind":{"type":"text","content":{"box":{"width":100,"height":48},"overflow":"visible","runs":[
        {"kind":"literal","text":"one two three four five six seven eight nine ten","typeToken":"typeTokens.body",
         "style":{"fontSize":{"value":20}}}]}}}]}"##;
    let (document, list) = theme(json);
    let Some(Shape::Text(layout)) = list
        .ops
        .iter()
        .map(|op| &op.shape)
        .find(|shape| matches!(shape, Shape::Text(_)))
    else {
        panic!("text op");
    };
    let runs = vec![TextRunPlan {
        text: "one two three four five six seven eight nine ten".to_owned(),
        style: TextStyle {
            size: 20.0,
            family: Some("Arial".to_owned()),
            ..TextStyle::default()
        },
    }];
    let auto = text::layout(
        &mut Fonts::new(),
        &TextRequest {
            runs: &runs,
            box_: Some((100.0, f32::INFINITY)),
            align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            overflow: TextOverflow::Visible,
            vertical: false,
            line_height: 1.2,
        },
    );
    println!("parsed overflow {:?}", document.nodes[0].kind);
    println!(
        "layout clip {:?}, {} lines kept of {}",
        layout.clip,
        layout.segments.len(),
        auto.segments.len()
    );
}

/// Source clamps a fixed box to at least one line
/// (`scene-fabric/authored-box.ts:298-316`). The port drops the first run as
/// soon as its line height exceeds the box (`crates/render/src/text.rs:241-247`).
fn short_box() {
    let json = r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":200,"height":200},
        "typeTokens":{"body":{"family":"Arial"}},
        "nodes":[{"id":"t","transform":{"x":0,"y":0,"width":100,"height":20},
        "kind":{"type":"text","content":{"box":{"width":100,"height":20},"runs":[
        {"kind":"literal","text":"STATUS","typeToken":"typeTokens.body",
         "style":{"fontSize":{"value":20}}}]}}}]}"##;
    let (_, list) = theme(json);
    let Some(Shape::Text(layout)) = list
        .ops
        .iter()
        .map(|op| &op.shape)
        .find(|shape| matches!(shape, Shape::Text(_)))
    else {
        panic!("text op");
    };
    println!(
        "20-unit box: {} segments, clip {:?}",
        layout.segments.len(),
        layout.clip
    );
    for segment in &layout.segments {
        println!("  {:?} at {:?}", segment.text, segment.origin);
    }
}

/// Source composites a 1.5 % overlay grain and `saturate(1.6)` with the blur
/// (`scene-fabric/glass.ts:175,190,558-584`). The port's raster glass only
/// blurs (`crates/raster/src/lib.rs:269-320`).
fn glass_grain() {
    let list = list(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":40,"height":40},
            "background":{"color":{"value":"#0000ff"}},
            "nodes":[{"id":"panel","kind":{"type":"rectangle"},
            "transform":{"x":10,"y":10,"width":20,"height":20},
            "glass":{"blurRadius":12}}]}"##,
    );
    let raster = render(&list, 1.0, None, None);
    let mut reds = Vec::new();
    for y in 12..28 {
        for x in 12..28 {
            reds.push(rgba(&raster, x, y)[0]);
        }
    }
    let min = *reds.iter().min().unwrap();
    let max = *reds.iter().max().unwrap();
    let distinct: std::collections::BTreeSet<u8> = reds.iter().copied().collect();
    println!(
        "flat-blue glass interior red min {min} max {max}, distinct {}",
        distinct.len()
    );
    println!("panel pixel {:?}", rgba(&raster, 20, 20));
}

/// Source gives a static panel the backdrop's declared still and never the live
/// video (`scene-fabric/glass.ts:500-501,696-700`); the host panel page defaults
/// to `static` (`scene-fabric/glass.ts:118-121`).
fn glass_sampling() {
    if !ffmpeg_available() {
        println!("skipped: no ffmpeg");
        return;
    }
    let dir = scratch("sampling");
    std::fs::write(dir.join("assets/clip.mp4"), MOTION).expect("write video");
    let (_, list) = theme(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":40,"height":40},
            "background":{"media":{"assetId":"clip","bounds":"theme","color":{"value":"#ff0000"}}},
            "assets":[{"id":"clip","kind":"video","path":"assets/clip.mp4"}],
            "nodes":[{"id":"panel","kind":{"type":"rectangle"},
            "transform":{"x":10,"y":10,"width":20,"height":20},
            "glass":{"blurRadius":4,"sampling":"static"}}]}"##,
    );
    let mut images = Images::at(dir);
    // Start the decoder and let it settle, so the measurement is not the
    // ffmpeg cold start.
    let _ = render(&list, 1.0, None, Some(&mut images));
    std::thread::sleep(Duration::from_millis(700));
    let _ = render(&list, 1.0, None, Some(&mut images));
    let first = render(&list, 1.0, None, Some(&mut images));
    std::thread::sleep(Duration::from_millis(500));
    let second = render(&list, 1.0, None, Some(&mut images));
    let differing = first
        .rgba
        .chunks(4)
        .zip(second.rgba.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "static panel first {:?} second {:?}",
        rgba(&first, 20, 20),
        rgba(&second, 20, 20)
    );
    println!(
        "static panel differing pixels {differing} of {}",
        first.rgba.len() / 4
    );
}

/// Source fits a rect artboard with its own `contentFit`
/// (`renderer-core/artboard.ts:107-120,158-169`); a circle is always contain.
/// The panel raster is always contain (`crates/raster/src/lib.rs:800-804`).
fn content_fit() {
    let list = list(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":50,"contentFit":"cover"},
            "background":{"color":{"value":"#ff0000"}},
            "nodes":[]}"##,
    );
    let raster =
        yunari_render_raster::fit_into(&list, 100, 100, 0, false, None, None).expect("fit");
    println!(
        "cover top {:?} middle {:?} bottom {:?}",
        rgba(&raster, 50, 10),
        rgba(&raster, 50, 50),
        rgba(&raster, 50, 90)
    );
}

/// Source starts a background video muted and looping
/// (`scene-fabric/background-media.ts:412-415`). The port's raster store reads
/// frames against the wall clock and reopens at its own duration
/// (`crates/raster/src/images.rs:124-178`).
fn video() {
    if !ffmpeg_available() {
        println!("skipped: no ffmpeg");
        return;
    }
    let dir = scratch("video");
    std::fs::write(dir.join("assets/clip.mp4"), MOTION).expect("write video");
    let facts = yunari_model::video::read(MOTION);
    println!("video facts {facts:?}");
    let (_, list) = theme(
        r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":40,"height":40},
            "background":{"media":{"assetId":"clip","bounds":"theme"}},
            "assets":[{"id":"clip","kind":"video","path":"assets/clip.mp4"}],
            "nodes":[]}"##,
    );
    let mut images = Images::at(dir);
    // Start the decoder and let it settle, so the measurement is not the
    // ffmpeg cold start.
    let _ = render(&list, 1.0, None, Some(&mut images));
    std::thread::sleep(Duration::from_millis(700));
    let mut previous = render(&list, 1.0, None, Some(&mut images));
    println!("sample 0 ms center {:?}", rgba(&previous, 20, 20));
    // Duration is 1 s: samples past it show the loop keeps painting frames
    // instead of holding the last one.
    for step in 1..=6 {
        std::thread::sleep(Duration::from_millis(300));
        let sample = render(&list, 1.0, None, Some(&mut images));
        let differing = sample
            .rgba
            .chunks(4)
            .zip(previous.rgba.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        println!(
            "sample {} ms center {:?} differing from previous {differing} of {}",
            step * 300,
            rgba(&sample, 20, 20),
            sample.rgba.len() / 4
        );
        previous = sample;
    }
}
