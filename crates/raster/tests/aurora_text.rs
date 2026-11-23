//! Aurora Signal's headline text on Turzx geometry: real theme, real faces.
//!
//! The author centred the CPU/GPU load readouts on their cards. A native panel
//! must put the same ink in the same place and keep the `%` on the baseline.

use std::path::PathBuf;

use yunari_providers::sample::Reading;
use yunari_render::{Readings, Shape};

fn theme_dir() -> PathBuf {
    PathBuf::from(r"C:\Users\peter\.yunari\themes\e1bdd3c4-ee73-49c0-b717-3a857eb69d8a")
}

fn load_theme() -> yunari_model::document::ThemeDocument {
    let json =
        std::fs::read_to_string(theme_dir().join("theme.json")).expect("Aurora Signal exists");
    yunari_model::document::parse_document(&json).expect("the theme parses")
}

fn readings() -> Readings {
    Readings::new([
        Reading::number("cpu.load", 42.0, "%"),
        Reading::number("gpu.load", 67.0, "%"),
    ])
}

fn opacity_at(raster: &yunari_render_raster::Raster, x: u32, y: u32) -> u8 {
    raster.rgba[((y * raster.width + x) * 4 + 3) as usize]
}

#[test]
fn aurora_load_text_is_centred_on_its_card_with_the_unit_on_the_baseline() {
    let dir = theme_dir();
    if !dir.join("theme.json").exists() {
        eprintln!("SKIP: installed Aurora Signal theme is absent");
        return;
    }
    let document = load_theme();
    let mut fonts = yunari_render::Fonts::new();
    let refusals = fonts.load_faces(&document, |asset| std::fs::read(dir.join(asset)).ok());
    assert!(refusals.is_empty(), "faces: {refusals:?}");
    let mut images = yunari_render_raster::Images::at(&dir);
    let (list, issues) = yunari_render::fabric::build_with_sources(
        &document,
        &mut fonts,
        &readings(),
        &yunari_providers::units::UnitPreferences::default(),
        &std::collections::HashMap::new(),
    );
    // Missing weather/time readings are expected; the CPU/GPU loads supplied
    // above must not be among them.
    assert!(
        !issues
            .iter()
            .any(|issue| issue.detail.contains("cpu.load") || issue.detail.contains("gpu.load")),
        "plan: {issues:?}"
    );

    let raster = yunari_render_raster::render_with_charts(
        &list,
        1.0,
        Some(&mut fonts),
        Some(&mut images),
        Some(&document),
        &yunari_render_raster::ChartSamples::empty(),
    )
    .0;
    assert_eq!((raster.width, raster.height), (480, 800));

    // Ignore the glass card's own pixels: the value run's ivory (#f0f7ff) is
    // brighter than the card fill and the gauges, so threshold on brightness.
    let bright = |r: &yunari_render_raster::Raster, x: u32, y: u32| {
        let i = ((y * r.width + x) * 4) as usize;
        opacity_at(r, x, y) > 200 && r.rgba[i] > 200 && r.rgba[i + 1] > 200 && r.rgba[i + 2] > 200
    };
    let cpu: Vec<u32> = (40_u32..216)
        .filter(|x| (285_u32..350).any(|y| bright(&raster, *x, y)))
        .collect();
    let gpu: Vec<u32> = (264_u32..440)
        .filter(|x| (285_u32..350).any(|y| bright(&raster, *x, y)))
        .collect();
    assert!(!cpu.is_empty(), "CPU load text painted no bright pixels");
    assert!(!gpu.is_empty(), "GPU load text painted no bright pixels");

    // Card centres are 128 and 352. A centred readout must straddle them.
    let cpu_mid = (cpu[0] + *cpu.last().unwrap()) as f32 / 2.0;
    let gpu_mid = (gpu[0] + *gpu.last().unwrap()) as f32 / 2.0;
    assert!(
        (cpu_mid - 128.0).abs() < 22.0,
        "CPU load must centre on its card at x=128, got {cpu_mid}"
    );
    assert!(
        (gpu_mid - 352.0).abs() < 22.0,
        "GPU load must centre on its card at x=352, got {gpu_mid}"
    );

    // The `%` is a 15px body run. It must sit on the same baseline as the
    // 70px value, not one line above it. Find the dim run's own pixels by
    // colour: palette.dim is #9C97A3, distinct from ivory.
    let unit_rows: Vec<u32> = (270_u32..360)
        .filter(|y| {
            (40_u32..216).any(|x| {
                let i = ((*y * raster.width + x) * 4) as usize;
                let (r, g, b, a) = (
                    raster.rgba[i],
                    raster.rgba[i + 1],
                    raster.rgba[i + 2],
                    raster.rgba[i + 3],
                );
                a > 64
                    && (r as i16 - g as i16).abs() < 24
                    && (g as i16 - b as i16).abs() < 30
                    && r > 90
                    && r < 210
            })
        })
        .collect();
    let value_rows: Vec<u32> = (270_u32..360)
        .filter(|y| (40_u32..216).any(|x| bright(&raster, x, *y)))
        .collect();
    assert!(!unit_rows.is_empty(), "unit run painted no pixels");
    assert!(!value_rows.is_empty(), "value run painted no pixels");
    let unit_bottom = *unit_rows.last().unwrap();
    let value_bottom = *value_rows.last().unwrap();
    assert!(
        unit_bottom as i32 >= value_bottom as i32 - 6,
        "the % unit must sit on the value's baseline: unit {unit_bottom}, value {value_bottom}"
    );
}

#[test]
fn aurora_load_text_layout_segments_share_one_baseline() {
    let dir = theme_dir();
    if !dir.join("theme.json").exists() {
        eprintln!("SKIP: installed Aurora Signal theme is absent");
        return;
    }
    let document = load_theme();
    let mut fonts = yunari_render::Fonts::new();
    fonts.load_faces(&document, |asset| std::fs::read(dir.join(asset)).ok());
    let (list, _) = yunari_render::fabric::build_with_sources(
        &document,
        &mut fonts,
        &readings(),
        &yunari_providers::units::UnitPreferences::default(),
        &std::collections::HashMap::new(),
    );
    let text = list
        .ops
        .iter()
        .filter_map(|op| match &op.shape {
            Shape::Text(layout) => Some((op, layout)),
            _ => None,
        })
        .find(|(op, _)| op.transform.e >= 50.0 && op.transform.e < 60.0)
        .map(|(_, layout)| layout)
        .expect("CPU load text op");
    assert_eq!(text.segments.len(), 2, "value and % are two runs");
    let value = &text.segments[0];
    let unit = &text.segments[1];
    assert!(
        (value.origin.1 - unit.origin.1).abs() < 1.0,
        "both runs must share a line top: {} vs {}",
        value.origin.1,
        unit.origin.1
    );
}
