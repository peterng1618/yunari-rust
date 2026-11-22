//! What one full frame costs: plan and paint a representative theme.
//!
//! Run with `cargo test --release -p yunari-render-raster --test frame_cost
//! -- --ignored --nocapture`. It prints the figures the spec's performance
//! section reports; it is not a gate.

use std::path::PathBuf;
use std::time::{Duration, Instant};

const THEME: &str = r##"{
  "schemaVersion": "3.0.0",
  "id": "measure",
  "metadata": { "name": "Measure", "themeLanguage": "en" },
  "artboard": { "width": 1920, "height": 1080, "shape": "rect" },
  "background": { "color": { "ref": "palette.bg" } },
  "palette": {
    "bg": { "name": "Background", "value": { "kind": "gradient", "angle": 90, "stops": [
      { "offset": 0, "color": "#0b0b10" },
      { "offset": 1, "color": "#181824" }
    ] } },
    "plate": { "name": "Plate", "value": { "kind": "solid", "color": "#1a1a24" } },
    "ink": { "name": "Ink", "value": { "kind": "solid", "color": "#e8e8ea" } }
  },
  "typeTokens": {
    "heading": { "family": "Segoe UI", "weight": "bold" },
    "body": { "family": "Segoe UI", "weight": "regular" }
  },
  "nodes": [
    { "id": "big", "kind": { "type": "rectangle", "cornerRadius": 24 },
      "transform": { "x": 40, "y": 40, "width": 1180, "height": 620 },
      "style": { "fill": { "ref": "palette.plate" } } },
    { "id": "side", "kind": { "type": "rectangle", "cornerRadius": 24 },
      "transform": { "x": 1240, "y": 40, "width": 640, "height": 300 },
      "style": { "fill": { "ref": "palette.plate" } } },
    { "id": "foot", "kind": { "type": "rectangle", "cornerRadius": 24 },
      "transform": { "x": 40, "y": 680, "width": 1840, "height": 360 },
      "style": { "fill": { "ref": "palette.plate" } } },
    { "id": "dial", "kind": { "type": "chart", "content": { "family": "gauge",
        "settings": { "thickness": 18, "startAngle": 210, "endAngle": -30 } } },
      "transform": { "x": 1400, "y": 380, "width": 360, "height": 360 } },
    { "id": "trend", "kind": { "type": "chart", "content": { "family": "line",
        "settings": { "thickness": 3 } } },
      "transform": { "x": 80, "y": 700, "width": 1100, "height": 320 } },
    { "id": "title", "kind": { "type": "text", "content": { "runs": [
        { "kind": "literal", "text": "THIS PC", "typeToken": "typeTokens.heading",
          "style": {"fontSize": {"value": 56}} } ] } },
      "transform": { "x": 80, "y": 80, "width": 500, "height": 80 },
      "style": { "fill": { "ref": "palette.ink" } } },
    { "id": "sub", "kind": { "type": "text", "content": { "runs": [
        { "kind": "literal", "text": "a representative dashboard at 1920x1080",
          "typeToken": "typeTokens.body", "style": {"fontSize": {"value": 24}} } ] } },
      "transform": { "x": 82, "y": 170, "width": 900, "height": 40 },
      "style": { "fill": { "ref": "palette.ink" } } }
  ]
}"##;

#[test]
#[ignore = "reports timings; run it explicitly"]
fn one_frame_plans_and_paints_a_representative_theme() {
    let document = yunari_model::document::parse_document(THEME).expect("a theme");
    let mut fonts = yunari_render::Fonts::new();
    let mut samples = Vec::new();
    for _ in 0..65 {
        let started = Instant::now();
        let (list, issues) = yunari_render::build(&document, &mut fonts);
        assert!(issues.is_empty(), "{issues:?}");
        let raster = yunari_render_raster::render(&list, 1.0, Some(&mut fonts), None);
        assert_eq!((raster.width, raster.height), (1920, 1080));
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    // The first five warm the font cache and the decoder.
    samples.drain(..5);
    samples.sort_by(|a, b| a.total_cmp(b));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    println!(
        "frame: mean {mean:.2} ms, p95 {p95:.2} ms, min {:.2} ms, max {:.2} ms over {} frames",
        samples[0],
        samples[samples.len() - 1],
        samples.len()
    );
    assert!(mean < 50.0, "a frame took {mean:.2} ms on average");
}

/// Bounded video probe: three seconds of the 4K sunset-drive clip at the
/// Kraken's 9 fps cadence. `load` returns the bytes the panel actually consumes,
/// so the byte count is the decoder's real work. A panel-sized decode must
/// return the same picture at a fraction of the source's size.
#[test]
#[ignore = "requires ffmpeg and an installed theme; reports resource numbers"]
fn video_decode_follows_panel_demand() {
    let dir = PathBuf::from(r"C:\Users\peter\.yunari\themes\f29b228d-b1b3-4109-b07c-6aada468074b");
    let asset = "assets/DeLorean-by-VISUALDON---4k-60fps-Ultra-HD.mp4";
    if !dir.join(asset).is_file() || std::env::var_os("YUNARI_FFMPEG").is_none() {
        eprintln!("skip: theme video or YUNARI_FFMPEG absent");
        return;
    }
    let mut full_frame_bytes = 0_u64;
    let mut panel_frame_bytes = 0_u64;
    let mut panel_pixels = 0_u64;
    let mut frame_changed = false;
    for (label, size) in [("source 4K", (0, 0)), ("panel 800x450", (800, 450))] {
        let mut images = yunari_render_raster::Images::at(&dir);
        images.set_demand_fps(9.0);
        images.set_render_size(size.0, size.1);
        let started = Instant::now();
        let mut reads = 0_u64;
        let mut in_read = Duration::ZERO;
        let mut first_frame = None;
        while started.elapsed() < Duration::from_secs(3) {
            let read_started = Instant::now();
            if let Some(frame) = images.load(asset) {
                if size.0 > 0 {
                    let bytes = frame.rgba.clone();
                    if let Some(first) = &first_frame {
                        frame_changed |= first != &bytes;
                    } else {
                        first_frame = Some(bytes);
                    }
                }
                if reads == 0 {
                    if size.0 == 0 {
                        full_frame_bytes = frame.rgba.len() as u64;
                    } else {
                        panel_frame_bytes = frame.rgba.len() as u64;
                    }
                }
                if size.0 > 0 {
                    assert_eq!((frame.width, frame.height), (800, 450));
                    // A real frame, not a fabricated or blank substitute: the
                    // clip is photographic, so its bytes are not all equal.
                    assert!(frame.rgba.iter().any(|byte| *byte != 0));
                    if panel_pixels == 0 {
                        panel_pixels = frame.rgba.len() as u64 / 4;
                    }
                }
                reads += 1;
            }
            in_read += read_started.elapsed();
            std::thread::sleep(Duration::from_millis(33));
        }
        println!(
            "{label}: {reads} load calls, {:.1} MiB returned in {:.2}s, {:.1}% of wall time in load",
            reads as f64
                * if size.0 == 0 {
                    full_frame_bytes
                } else {
                    panel_frame_bytes
                } as f64
                / (1024.0 * 1024.0),
            started.elapsed().as_secs_f64(),
            100.0 * in_read.as_secs_f64() / started.elapsed().as_secs_f64()
        );
    }
    assert_eq!(panel_pixels, 800 * 450, "the decoded frame is panel-sized");
    assert!(
        frame_changed,
        "the panel receives real advancing video frames"
    );
    assert!(
        panel_frame_bytes * 20 < full_frame_bytes,
        "a panel frame carried {panel_frame_bytes} bytes against {full_frame_bytes} full-size bytes"
    );
}
