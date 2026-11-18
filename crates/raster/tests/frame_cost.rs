//! What one full frame costs: plan and paint a representative theme.
//!
//! Run with `cargo test --release -p yunari-render-raster --test frame_cost
//! -- --ignored --nocapture`. It prints the figures the spec's performance
//! section reports; it is not a gate.

use std::time::Instant;

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
