//! Line charts fed by the host's per-tick readings.
//!
//! A widget card nests its chart in a scaled Fabric group; a Studio Broadcast
//! chart sits at the scene root. Both must paint a polyline from three real
//! readings, and neither may invent a point when no reading arrives. The
//! goldens record the resulting panel pixels for each shape.

use std::path::Path;

use base64::Engine;
use yunari_golden::{GoldenConfig, assert_golden};
use yunari_render_raster::sidecar::{Sessions, handle_line};

/// A widget card's sparkline: one chart, nested in a 0.4716-scale group — the
/// scale of the installed CPU card — and bound to `cpu.load`. The green stroke
/// is unique to the series so a pixel count is a claim about the polyline, not
/// about card chrome or text.
const WIDGET_THEME: &str = r##"{
  "schemaVersion":"3.0.0","id":"widget-line","artboard":{"width":120,"height":80},
  "background":{"color":{"value":"#000000"}},
  "scene":{"version":"7.4.0","objects":[
    {"id":"card","type":"Group","left":10,"top":10,"width":100,"height":60,
     "originX":"left","originY":"top","scaleX":0.4716,"scaleY":0.4716,"objects":[
       {"id":"spark","type":"YunariChart","family":"line",
        "settings":{"min":0,"max":100,"showAxes":false,
          "windowSeconds":4,"maxPoints":600,"lineWidth":5,
          "stroke":{"kind":"solid","color":"#00ff00"}},
        "left":-40,"top":-20,"width":80,"height":40,
        "originX":"left","originY":"top"}]}]},
  "bindings":{"spark":[{"id":"spark-binding","semanticKey":"cpu.load"}]}
}"##;

/// The Studio Broadcast shape: a root chart bound to two network keys, with
/// the same green series ink so the structural summaries are comparable.
const BROADCAST_THEME: &str = r##"{
  "schemaVersion":"3.0.0","id":"broadcast-line","artboard":{"width":120,"height":80},
  "background":{"color":{"value":"#000000"}},
  "scene":{"version":"7.4.0","objects":[
    {"id":"trend","type":"YunariChart","family":"line",
     "settings":{"lineWidth":3,"min":0,"max":100,"showAxes":false,
       "windowSeconds":4,"maxPoints":600,
       "stroke":{"kind":"solid","color":"#00ff00"}},
     "left":0,"top":0,"width":120,"height":80,
     "originX":"left","originY":"top"}]},
  "bindings":{"trend":[
    {"id":"down-trend","semanticKey":"network.download"},
    {"id":"up-trend","semanticKey":"network.upload"}]}
}"##;

fn reply(state: &mut Sessions, request: serde_json::Value) -> serde_json::Value {
    let line = handle_line(state, &request.to_string()).expect("a response line");
    let line = line.expect("a request always answers");
    serde_json::from_str(&line).expect("a JSON response")
}

fn open(theme: &str) -> Sessions {
    let mut state = Sessions::new();
    let response = reply(
        &mut state,
        serde_json::json!({
            "type":"open","id":"panel","theme_json":theme,
            "width":120,"height":80,"format":"png"
        }),
    );
    assert_eq!(response["ok"], true, "{response}");
    state
}

fn frame(state: &mut Sessions, readings: serde_json::Value) -> image::RgbaImage {
    frame_at(state, readings, "2026-01-01T00:00:00.000Z")
}

fn frame_with_series(
    state: &mut Sessions,
    readings: serde_json::Value,
    series: serde_json::Value,
    sent_at: &str,
) -> image::RgbaImage {
    let response = reply(
        state,
        serde_json::json!({
            "type":"frame","id":"panel","readings":readings,"series":series,
            "sent_at":sent_at
        }),
    );
    assert_eq!(response["ok"], true, "{response}");
    let encoded = response["bytes_base64"].as_str().expect("base64 bytes");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("decodable bytes");
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .expect("a PNG frame")
        .to_rgba8()
}

fn frame_at(state: &mut Sessions, readings: serde_json::Value, sent_at: &str) -> image::RgbaImage {
    frame_with_series(state, readings, serde_json::json!({}), sent_at)
}

/// Columns and rows carrying the unique series ink, so a structural match can
/// be asserted without demanding pixel equality between two different shapes.
#[derive(Debug, PartialEq, Eq)]
struct Structure {
    ink_pixels: usize,
    columns: Vec<u32>,
    rows: Vec<u32>,
}

fn structure(picture: &image::RgbaImage) -> Structure {
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    let mut ink_pixels = 0;
    for (x, y, pixel) in picture.enumerate_pixels() {
        if pixel[1] > 200 && pixel[0] < 50 && pixel[2] < 50 && pixel[3] > 0 {
            ink_pixels += 1;
            if !columns.contains(&x) {
                columns.push(x);
            }
            if !rows.contains(&y) {
                rows.push(y);
            }
        }
    }
    Structure {
        ink_pixels,
        columns,
        rows,
    }
}

fn golden(name: &str, picture: &image::RgbaImage) {
    assert_golden(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens"),
        name,
        picture.width(),
        picture.height(),
        picture.as_raw(),
        // The two frames come from different scene shapes, so each golden is
        // per-shape; the cross-mode claim is the structural summary below.
        GoldenConfig::default(),
    );
}

#[test]
fn a_widget_card_line_chart_paints_a_polyline_from_three_real_readings() {
    let mut state = open(WIDGET_THEME);
    let mut picture = image::RgbaImage::new(1, 1);
    for (tick, value) in [10.0, 40.0, 80.0].into_iter().enumerate() {
        let timestamp = format!("2026-01-01T00:00:0{}.000Z", tick + 1);
        picture = frame_at(
            &mut state,
            serde_json::json!([
                {"semantic_key":"cpu.load","status":"ok","value":value,
                 "unit":"%","timestamp":timestamp}
            ]),
            &timestamp,
        );
    }
    let drawn = structure(&picture);
    assert!(
        drawn.ink_pixels > 30,
        "three readings must paint a polyline: {drawn:?}"
    );
    assert!(
        drawn.columns.len() >= 10 && drawn.rows.len() >= 3,
        "a polyline spans the window, not a single point: {drawn:?}"
    );
    golden("line-series-widget-card", &picture);
}

#[test]
fn the_studio_broadcast_line_chart_still_paints_its_two_series() {
    let mut state = open(BROADCAST_THEME);
    let mut picture = image::RgbaImage::new(1, 1);
    for (tick, (down, up)) in [(10.0, 90.0), (40.0, 60.0), (80.0, 20.0)]
        .into_iter()
        .enumerate()
    {
        let timestamp = format!("2026-01-01T00:00:0{}.000Z", tick + 1);
        picture = frame_at(
            &mut state,
            serde_json::json!([
                {"semantic_key":"network.download","status":"ok","value":down,
                 "unit":"Mb/s","timestamp":timestamp},
                {"semantic_key":"network.upload","status":"ok","value":up,
                 "unit":"Mb/s","timestamp":timestamp}
            ]),
            &timestamp,
        );
    }
    let drawn = structure(&picture);
    assert!(
        drawn.ink_pixels > 60,
        "three readings per binding must paint two polylines: {drawn:?}"
    );
    assert!(
        drawn.columns.len() >= 10 && drawn.rows.len() >= 3,
        "the broadcast series spans the window: {drawn:?}"
    );
    golden("line-series-studio-broadcast", &picture);
}

#[test]
fn the_series_uses_the_host_tick_not_the_frame_count() {
    // One real reading per frame, but the host's clock jumps a whole window.
    // Frame-count pacing (1 s per tick) would place all three points at the
    // right edge; timestamp placement keeps the oldest at the left edge.
    let mut state = open(WIDGET_THEME);
    let mut picture = image::RgbaImage::new(1, 1);
    for (tick, value) in [10.0, 50.0, 90.0].into_iter().enumerate() {
        // The newest reading sits at the right edge. A 4 s window with the
        // two older readings 3 s back puts one at the left edge and one in the
        // middle; frame-count pacing collapses all three into the rightmost
        // columns instead.
        let at = match tick {
            0 => "2026-01-01T00:00:00.000Z",
            1 => "2026-01-01T00:00:01.000Z",
            _ => "2026-01-01T00:00:04.000Z",
        };
        picture = frame_at(
            &mut state,
            serde_json::json!([
                {"semantic_key":"cpu.load","status":"ok","value":value,
                 "unit":"%","timestamp":at}
            ]),
            at,
        );
    }
    let drawn = structure(&picture);
    assert!(
        drawn.columns.len() >= 10,
        "the host's 4 s tick gap must spread three points across the window: {drawn:?}"
    );
}

#[test]
fn a_host_series_uses_each_point_clock_not_its_index() {
    // The authoritative path: the host supplies six points from a 10 s source
    // in a 60 s window. Spacing them by index (1 s) would collapse the whole
    // series into the rightmost tenth of the plot; their real clocks spread it
    // across the window. The stream need only send one frame.
    let theme = WIDGET_THEME.replace("\"windowSeconds\":4", "\"windowSeconds\":60");
    let mut state = open(&theme);
    let series = serde_json::json!({
        "cpu.load": [
            {"at":"2026-01-01T00:00:00.000Z","value":10.0},
            {"at":"2026-01-01T00:00:10.000Z","value":30.0},
            {"at":"2026-01-01T00:00:20.000Z","value":50.0},
            {"at":"2026-01-01T00:00:30.000Z","value":70.0},
            {"at":"2026-01-01T00:00:40.000Z","value":90.0},
            {"at":"2026-01-01T00:00:50.000Z","value":60.0}
        ]
    });
    let picture = frame_with_series(
        &mut state,
        serde_json::json!([
            {"semantic_key":"cpu.load","status":"ok","value":60.0,
             "unit":"%","timestamp":"2026-01-01T00:00:50.000Z"}
        ]),
        series,
        "2026-01-01T00:00:50.000Z",
    );
    let drawn = structure(&picture);
    assert!(
        drawn.columns.len() >= 20,
        "a 10 s cadence must spread six points across a 60 s window: {drawn:?}"
    );
}

#[test]
fn no_reading_never_invents_a_series_point() {
    for theme in [WIDGET_THEME, BROADCAST_THEME] {
        let mut state = open(theme);
        let mut picture = image::RgbaImage::new(1, 1);
        for _ in 0..3 {
            picture = frame(&mut state, serde_json::json!([]));
        }
        let drawn = structure(&picture);
        assert_eq!(
            drawn.ink_pixels, 0,
            "an absent reading must leave the series ink clear: {drawn:?}"
        );
    }
}
