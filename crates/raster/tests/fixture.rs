//! One fixture theme, painted to pixels and held to a committed golden.
//!
//! This is the phase 1 exit evidence: the same document a window draws also
//! paints here, so a change in geometry shows up as a diff rather than as an
//! opinion.

use std::path::Path;

use yunari_golden::{GoldenConfig, assert_golden};

const FIXTURE: &str = r##"{
  "schemaVersion": "1.0.0",
  "id": "fixture",
  "metadata": { "name": "Fixture", "themeLanguage": "en" },
  "artboard": { "width": 320, "height": 180, "shape": "rect" },
  "background": { "color": { "value": "#101014" } },
  "palette": {
    "plate": {
      "name": "Plate",
      "value": { "kind": "gradient", "angle": 0, "stops": [
        { "offset": 0, "color": "#1f6feb" },
        { "offset": 1, "color": "#0d419d" }
      ] }
    },
    "accent": { "name": "Accent", "value": { "kind": "solid", "color": "#f778ba" } }
  },
  "typeTokens": {
    "heading": { "family": "Segoe UI", "weight": "bold" }
  },
  "nodes": [
    {
      "id": "plate",
      "kind": { "type": "rectangle", "cornerRadius": 12 },
      "transform": { "x": 24, "y": 24, "width": 272, "height": 132 },
      "style": { "fill": { "ref": "palette.plate" } }
    },
    {
      "id": "dot",
      "kind": { "type": "ellipse" },
      "transform": { "x": 48, "y": 48, "width": 40, "height": 40 },
      "style": { "fill": { "ref": "palette.accent" } }
    },
    {
      "id": "rule",
      "kind": { "type": "line" },
      "transform": { "x": 48, "y": 120, "width": 224, "height": 0 },
      "style": { "stroke": { "value": "#e6edf3" }, "strokeWidth": { "value": 2 } }
    },
    {
      "id": "dial",
      "kind": { "type": "chart", "content": { "family": "gauge",
        "settings": { "thickness": 10, "startAngle": 210, "endAngle": -30 } } },
      "transform": { "x": 208, "y": 40, "width": 72, "height": 72 }
    },
    {
      "id": "title",
      "kind": { "type": "text", "content": { "runs": [
        { "kind": "literal", "text": "LOCAL", "typeToken": "typeTokens.heading", "style": {"fontSize": {"value": 22}} }
      ] } },
      "transform": { "x": 100, "y": 56, "width": 120, "height": 40 },
      "style": { "fill": { "value": "#101014" } }
    }
  ]
}"##;

fn golden_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

#[test]
fn the_fixture_theme_matches_its_golden() {
    let document = yunari_model::document::parse_document(FIXTURE).expect("the fixture is a theme");
    let mut fonts = yunari_render::Fonts::new();
    let (list, issues) = yunari_render::build(&document, &mut fonts);
    assert!(issues.is_empty(), "{issues:?}");

    let raster = yunari_render_raster::render(&list, 1.0, &mut fonts);
    assert_eq!((raster.width, raster.height), (320, 180));

    assert_golden(
        &golden_dir(),
        "fixture-theme",
        raster.width,
        raster.height,
        &raster.rgba,
        GoldenConfig::default(),
    );
}
