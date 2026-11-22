//! Absolute placement of a scaled Fabric group, pinned to the Kraken capture.
//!
//! The clock group carries its children at negative centre-local coordinates.
//! The rule is a rect, so a golden can pin its pixel position without depending
//! on installed fonts; the text transform is pinned in `render/tests/fabric.rs`.

use std::path::Path;

use yunari_golden::{GoldenConfig, assert_golden};
use yunari_model::document::parse_document;
use yunari_render::{Fonts, Readings};
use yunari_render_raster::{Raster, fit_raster_into, render};

const THEME: &str = r##"{
  "schemaVersion":"3.0.0","id":"kraken-clock","artboard":{"width":320,"height":320},
  "background":{"color":{"value":"#081d1d"}},
  "globals":{
    "palette":{
      "none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
      "brass":{"name":"Brass","value":{"kind":"solid","color":"#c9a227"}}
    },
    "typeTokens":{
      "heading":{"family":"serif"},"body":{"family":"serif"},
      "mono":{"family":"monospace"}
    }
  },
  "scene":{"version":"7.4.0","objects":[
    {"type":"Group","id":"clock","left":160,"top":243,
     "width":354.3895,"height":128,"scaleX":0.7399,"scaleY":0.7399,
     "objects":[
       {"type":"Rect","id":"rule","left":-24.5,"top":27.6009,
        "width":48,"height":1,"fill":"#c9a227"}
     ]}
  ]}
}"##;

fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * raster.width + x) * 4) as usize;
    [
        raster.rgba[index],
        raster.rgba[index + 1],
        raster.rgba[index + 2],
        raster.rgba[index + 3],
    ]
}

/// Bounding box of the brass rule's pixels.
fn brass_bounds(raster: &Raster) -> (u32, u32, u32, u32) {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..raster.height {
        for x in 0..raster.width {
            let [r, g, b, _] = pixel(raster, x, y);
            if r > 100 && g > 80 && b < 80 {
                (x0, y0) = (x0.min(x), y0.min(y));
                (x1, y1) = (x1.max(x), y1.max(y));
            }
        }
    }
    (x0, y0, x1, y1)
}

#[test]
fn the_kraken_clock_group_lands_where_chromium_paints_it() {
    let document = parse_document(THEME).expect("the fixture is a theme");
    let list = yunari_render::fabric::build(
        &document,
        &mut Fonts::new(),
        &Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    )
    .0;
    let source = render(&list, 1.0, None, None);
    assert_eq!((source.width, source.height), (320, 320));

    // Fabric's child matrix puts the rule's centre at (141.87, 263.42); the
    // backend's rect path starts at its leading edge, half a scaled width back.
    let (x0, y0, x1, y1) = brass_bounds(&source);
    assert!(
        (120..128).contains(&x0) && (262..266).contains(&y0) && (155..164).contains(&x1),
        "brass rule at ({x0}, {y0})..({x1}, {y1}) must match Fabric's group child placement"
    );

    let fitted = fit_raster_into(&source, 320, 320, 90, false)
        .expect("a 320×320 panel frame");
    let (fx0, fy0, fx1, fy1) = brass_bounds(&fitted);
    // After the quarter turn the rule becomes a short vertical stroke at
    // x≈56, y≈124..159 — inside the clock band Chromium shows at x≈66..226.
    assert!(
        (52..62).contains(&fx0) && (118..130).contains(&fy0) && (154..166).contains(&fy1),
        "turned rule at ({fx0}, {fy0})..({fx1}, {fy1}) must land in the clock band"
    );

    assert_golden(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens"),
        "kraken-clock-group",
        fitted.width,
        fitted.height,
        &fitted.rgba,
        GoldenConfig::default(),
    );
}
