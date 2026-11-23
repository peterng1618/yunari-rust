//! Chart ops painted by the raster backend.
//!
//! One golden per family pins the geometry the renderer's chart rules own;
//! the malformed cases pin the "issue, no invented mark" contract. The test
//! names name the formula each golden covers.

use std::collections::HashMap;
use std::path::Path;

type Readings = HashMap<String, Option<f64>>;
type Series = HashMap<String, Vec<(u64, f64)>>;

use yunari_golden::{GoldenConfig, assert_golden};
use yunari_model::document::{ThemeDocument, parse_document};
use yunari_render::{Affine, ChartFamily, DisplayList, Fonts, Op, Shape};
use yunari_render_raster::{ChartSamples, Raster, render_with_charts};

fn chart_list(
    family: ChartFamily,
    settings: serde_json::Value,
    bindings: &[&str],
) -> DisplayList {
    DisplayList {
        width: 120.0,
        height: 120.0,
        background: None,
        backdrop: None,
        backdrop_visible: true,
        ops: vec![Op {
            transform: Affine::IDENTITY,
            shape: Shape::Chart {
                id: "chart".to_owned(),
                family,
                settings,
                bindings: bindings
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect(),
                width: 120.0,
                height: 120.0,
                opacity: 1.0,
            },
            fill: None,
            stroke: None,
        }],
    }
}

fn chart_list_on_artboard(
    family: ChartFamily,
    settings: serde_json::Value,
    bindings: &[&str],
    id: &str,
    size: (f32, f32),
    opacity: f32,
) -> DisplayList {
    DisplayList {
        width: 480.0,
        height: 270.0,
        background: None,
        backdrop: None,
        backdrop_visible: true,
        ops: vec![Op {
            transform: Affine::translate(20.0, 30.0),
            shape: Shape::Chart {
                id: id.to_owned(),
                family,
                settings,
                bindings: bindings
                    .iter()
                    .map(|key| (*key).to_owned())
                    .collect(),
                width: size.0,
                height: size.1,
                opacity,
            },
            fill: None,
            stroke: None,
        }],
    }
}

/// The theme document a chart's paint refs and radar names resolve against.
fn document() -> ThemeDocument {
    parse_document(
        r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":120,"height":120},
            "scene":{"version":"7.4.0","objects":[
              {"id":"chart","type":"YunariChart","family":"gauge","settings":{},
               "left":0,"top":0,"width":120,"height":120}
            ]},
            "palette":{
              "track":{"name":"Track","value":{"kind":"solid","color":"#223047"}},
              "ink":{"name":"Ink","value":{"kind":"solid","color":"#00b8d9"}}
            },
            "bindings":{"chart":[
              {"id":"a","semanticKey":"cpu.load"},
              {"id":"b","semanticKey":"gpu.load"},
              {"id":"c","semanticKey":"ram.used"},
              {"id":"d","semanticKey":"disk.used"}
            ]}}"##,
    )
    .expect("the palette fixture parses")
}

/// Fixed readings and window samples, keyed by the chart op's binding names.
fn fixed_samples(
    keys: &[&str],
    values: &[Option<f64>],
    history: &[Vec<(u64, f64)>],
) -> (Readings, Series) {
    let mut readings = HashMap::new();
    let mut series = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        readings.insert((*key).to_owned(), values[index]);
        series.insert((*key).to_owned(), history.get(index).cloned().unwrap_or_default());
    }
    (readings, series)
}

fn draw(
    list: &DisplayList,
    readings: &Readings,
    series: &Series,
) -> (Raster, Vec<yunari_render::PlanIssue>) {
    let samples = ChartSamples { readings, series };
    render_with_charts(list, 1.0, Some(&mut Fonts::new()), None, Some(&document()), &samples)
}

fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * raster.width + x) * 4) as usize;
    [
        raster.rgba[index],
        raster.rgba[index + 1],
        raster.rgba[index + 2],
        raster.rgba[index + 3],
    ]
}

fn painted(raster: &Raster) -> usize {
    (0..raster.height)
        .flat_map(|y| (0..raster.width).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(raster, *x, *y)[3] > 0)
        .count()
}

fn golden(name: &str, raster: &Raster) {
    assert_golden(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens"),
        name,
        raster.width,
        raster.height,
        &raster.rgba,
        GoldenConfig::default(),
    );
}

fn assert_all_clear(raster: &Raster) {
    assert!(
        raster
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0),
        "an unreadable chart must not paint marks"
    );
}

#[test]
fn gauge_sweep_and_progress_position_follow_the_chart_authority() {
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "startAngle": 225, "endAngle": -45, "min": 0, "max": 100,
            "thickness": 12, "roundCap": true, "segmentDivisor": 100,
            "track": {"kind": "solid", "color": "#223047"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 400, "a ring is painted");
    // The ring is centred; its hole stays clear.
    assert_eq!(pixel(&raster, 60, 60)[3], 0);
    golden("chart-gauge", &raster);
}

#[test]
fn line_window_position_and_area_gradient_follow_the_chart_authority() {
    let list = chart_list(
        ChartFamily::Line,
        serde_json::json!({
            "lineWidth": 3, "interpolation": "linear", "min": 0, "max": 100,
            "showAxes": true, "showMarkers": true, "markerSize": 4,
            "windowSeconds": 60, "maxPoints": 600,
            "stroke": {"kind": "solid", "color": "#00b8d9"},
            "area": {"kind": "solid", "color": "#00b8d933"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(
        &["cpu.load"],
        &[Some(42.0)],
        &[vec![
            (60_000, 10.0),
            (62_000, 30.0),
            (64_000, 20.0),
            (66_000, 60.0),
            (68_000, 42.0),
        ]],
    );
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 400, "axes and a series are painted");
    golden("chart-line", &raster);
}

#[test]
fn bar_category_slot_and_segment_gap_follow_the_chart_authority() {
    let list = chart_list(
        ChartFamily::Bar,
        serde_json::json!({
            "orientation": "horizontal", "min": 0, "max": 100, "barWidth": 14,
            "categoryGapPercent": 40, "cornerRadius": 4, "trackCornerRadius": 4,
            "showAxes": false, "segmentDivisor": 5, "segmentFill": "partial",
            "fill": {"kind": "solid", "color": "#00b8d9"},
            "track": {"kind": "solid", "color": "#223047"}
        }),
        &["cpu.load", "gpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load", "gpu.load"], &[Some(80.0), Some(35.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 200, "two segmented tracks are painted");
    golden("chart-bar", &raster);
}

#[test]
fn pie_sector_share_and_pad_angle_follow_the_chart_authority() {
    let list = chart_list(
        ChartFamily::Pie,
        serde_json::json!({
            "startAngle": 90, "innerRadiusPercent": 30, "outerRadiusPercent": 90,
            "padAngle": 3,
            "palette": [
                {"kind": "solid", "color": "#00b8d9"},
                {"kind": "solid", "color": "#6554c0"},
                {"kind": "solid", "color": "#36b37e"}
            ]
        }),
        &["cpu.load", "gpu.load", "ram.used"],
    );
    let (readings, series) = fixed_samples(
        &["cpu.load", "gpu.load", "ram.used"],
        &[Some(40.0), Some(30.0), Some(20.0)],
        &[],
    );
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 1000, "three donut sectors are painted");
    assert_eq!(pixel(&raster, 60, 60)[3], 0, "the donut has a hole");
    golden("chart-pie", &raster);
}

#[test]
fn radar_spoke_angles_and_polygon_position_follow_the_chart_authority() {
    let list = chart_list(
        ChartFamily::Radar,
        serde_json::json!({
            "min": 0, "max": 100, "splitNumber": 3, "showAxes": true,
            "shape": "polygon", "showMarkers": true,
            "stroke": {"kind": "solid", "color": "#00b8d9"},
            "area": {"kind": "solid", "color": "#00b8d933"}
        }),
        &["cpu.load", "gpu.load", "ram.used", "disk.used"],
    );
    let (readings, series) = fixed_samples(
        &["cpu.load", "gpu.load", "ram.used", "disk.used"],
        &[Some(80.0), Some(60.0), Some(40.0), Some(20.0)],
        &[],
    );
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 500, "rings, spokes and a polygon are painted");
    golden("chart-radar", &raster);
}

#[test]
fn polar_bar_sector_radii_and_category_slot_follow_the_chart_authority() {
    let list = chart_list(
        ChartFamily::PolarBar,
        serde_json::json!({
            "startAngle": 90, "innerRadiusPercent": 20, "outerRadiusPercent": 90,
            "showAxes": true, "categoryGapPercent": 30, "min": 0, "max": 100,
            "fill": {"kind": "solid", "color": "#00b8d9"},
            "track": {"kind": "solid", "color": "#223047"}
        }),
        &["cpu.load", "gpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load", "gpu.load"], &[Some(70.0), Some(40.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 400, "two annular sectors are painted");
    golden("chart-polar-bar", &raster);
}

#[test]
fn a_chart_whose_settings_are_not_an_object_reports_an_issue_and_draws_nothing() {
    let list = chart_list(ChartFamily::Gauge, serde_json::json!(["not", "settings"]), &["cpu.load"]);
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, yunari_render::IssueCode::UnsupportedContent);
    assert_all_clear(&raster);
}

#[test]
fn an_unresolvable_chart_paint_reports_an_issue_and_draws_no_invented_mark() {
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "track": {"ref": "palette.absent"},
            "progress": {"ref": "palette.absent"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(!issues.is_empty(), "the unresolved ref is reported");
    assert!(
        issues
            .iter()
            .all(|issue| issue.code == yunari_render::IssueCode::UnresolvedGlobal),
        "{issues:?}"
    );
    assert_all_clear(&raster);
}

#[test]
fn a_chart_with_no_reading_draws_no_value_mark() {
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "thickness": 12,
            "track": {"kind": "solid", "color": "#223047"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[None], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, yunari_render::IssueCode::MissingReading);
    assert!(painted(&raster) > 400, "the track is still drawn");
    assert!(
        !raster
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] == 0 && pixel[1] > 150 && pixel[2] > 150 && pixel[3] > 0),
        "no progress colour appears without a reading"
    );
}

#[test]
fn a_chart_op_without_a_document_still_draws_its_track() {
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "thickness": 12,
            "track": {"kind": "solid", "color": "#223047"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
    );
    let samples = ChartSamples::empty();
    let (raster, issues) = render_with_charts(
        &list,
        1.0,
        Some(&mut Fonts::new()),
        None,
        None,
        &samples,
    );
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert_eq!(issues[0].code, yunari_render::IssueCode::MissingReading);
    assert!(painted(&raster) > 400, "the renderer's own default track is drawn");
}

#[test]
fn a_chart_uses_its_own_box_and_object_id_not_the_artboard() {
    let list = chart_list_on_artboard(
        ChartFamily::Gauge,
        serde_json::json!({
            "thickness": 6,
            "track": {"kind": "solid", "color": "#223047"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
        "object-chart",
        (60.0, 60.0),
        1.0,
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.iter().all(|issue| issue.node_id.as_deref() != Some("chart")), "{issues:?}");
    // The ring fits in the 60×60 box at (20,30); nothing reaches the far side
    // of the 480×270 artboard, which a wrongly artboard-sized ring would do.
    assert_eq!(pixel(&raster, 460, 250)[3], 0);
    assert!(painted(&raster) < 60 * 60, "the ring stays inside its own box");
    assert!(painted(&raster) > 200, "a ring is painted");
}

#[test]
fn authored_chart_opacity_fades_the_marks() {
    let settings = serde_json::json!({
        "thickness": 12,
        "track": {"kind": "solid", "color": "#223047"},
        "progress": {"kind": "solid", "color": "#00b8d9"}
    });
    let opaque = chart_list(ChartFamily::Gauge, settings.clone(), &["cpu.load"]);
    let faint = chart_list_on_artboard(
        ChartFamily::Gauge,
        settings,
        &["cpu.load"],
        "chart",
        (120.0, 120.0),
        0.5,
    );
    // Put the faint chart at the same place so pixels compare directly.
    let mut faint = faint;
    faint.ops[0].transform = Affine::IDENTITY;
    faint.width = 120.0;
    faint.height = 120.0;
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (full, _) = draw(&opaque, &readings, &series);
    let (half, _) = draw(&faint, &readings, &series);
    let full_opaque = full.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count();
    let half_opaque = half.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count();
    assert!(half_opaque < full_opaque / 2, "half opacity leaves few fully solid pixels: {half_opaque} vs {full_opaque}");
}

#[test]
fn a_radar_chart_uses_its_real_object_id_for_binding_names() {
    // The document names a radar id that is not the synthetic "chart" used by
    // the other fixtures. If the request still used "chart", radar::build
    // would find no binding names and draw no frame at all.
    let doc = parse_document(
        r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":120,"height":120},
            "scene":{"version":"7.4.0","objects":[
              {"id":"radar-7","type":"YunariChart","family":"radar","settings":{},
               "left":0,"top":0,"width":120,"height":120}]},
            "bindings":{"radar-7":[
              {"id":"a","semanticKey":"cpu.load"},
              {"id":"b","semanticKey":"gpu.load"},
              {"id":"c","semanticKey":"ram.used"},
              {"id":"d","semanticKey":"disk.used"}
            ]}}"##,
    )
    .expect("radar fixture parses");
    let list = chart_list_on_artboard(
        ChartFamily::Radar,
        serde_json::json!({
            "min": 0, "max": 100, "splitNumber": 3, "showAxes": true,
            "stroke": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load", "gpu.load", "ram.used", "disk.used"],
        "radar-7",
        (120.0, 120.0),
        1.0,
    );
    let (readings, series) = fixed_samples(
        &["cpu.load", "gpu.load", "ram.used", "disk.used"],
        &[Some(80.0), Some(60.0), Some(40.0), Some(20.0)],
        &[],
    );
    let samples = ChartSamples {
        readings: &readings,
        series: &series,
    };
    let (raster, issues) = render_with_charts(
        &list,
        1.0,
        Some(&mut Fonts::new()),
        None,
        Some(&doc),
        &samples,
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert!(painted(&raster) > 500, "a frame is drawn for the real id");
}

#[test]
fn a_wrongly_typed_required_setting_reports_an_issue_and_draws_nothing() {
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "thickness": "thick",
            "track": {"kind": "solid", "color": "#223047"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.iter().any(|issue| {
        issue.code == yunari_render::IssueCode::UnsupportedContent
            && issue.detail.contains("thickness")
    }), "{issues:?}");
    assert_all_clear(&raster);
}

#[test]
fn a_nonfinite_required_setting_reports_an_issue_and_draws_nothing() {
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "startAngle": null,
            "track": {"kind": "solid", "color": "#223047"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let (raster, issues) = draw(&list, &readings, &series);
    assert!(issues.iter().any(|issue| issue.detail.contains("startAngle")), "{issues:?}");
    assert_all_clear(&raster);
}

#[test]
fn a_chart_paint_ref_resolves_against_globals_not_only_the_legacy_palette() {
    let doc = parse_document(
        r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":120,"height":120},
            "scene":{"version":"7.4.0","objects":[
              {"id":"chart","type":"YunariChart","family":"gauge","settings":{},
               "left":0,"top":0,"width":120,"height":120}]},
            "globals":{"palette":{"none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
              "track":{"name":"Track","value":{"kind":"solid","color":"#223047"}}},
              "typeTokens":{
                "heading":{"family":"Segoe UI"},
                "body":{"family":"Segoe UI"},
                "mono":{"family":"Consolas"}}},
            "bindings":{"chart":[{"id":"a","semanticKey":"cpu.load"}]}}"##,
    )
    .expect("schema-3 globals fixture parses");
    let list = chart_list(
        ChartFamily::Gauge,
        serde_json::json!({
            "thickness": 12,
            "track": {"ref": "palette.track"},
            "progress": {"kind": "solid", "color": "#00b8d9"}
        }),
        &["cpu.load"],
    );
    let (readings, series) = fixed_samples(&["cpu.load"], &[Some(42.0)], &[]);
    let samples = ChartSamples {
        readings: &readings,
        series: &series,
    };
    let (raster, issues) = render_with_charts(
        &list,
        1.0,
        Some(&mut Fonts::new()),
        None,
        Some(&doc),
        &samples,
    );
    assert!(
        !issues.iter().any(|issue| issue.code == yunari_render::IssueCode::UnresolvedGlobal),
        "{issues:?}"
    );
    assert!(painted(&raster) > 400);
}
