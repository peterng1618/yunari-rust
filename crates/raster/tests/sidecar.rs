//! JSON-lines panel sidecar, driven through its pure handler.
//!
//! The tests send the exact wire shapes the host will send and read the exact
//! JSON responses back. No process, socket or filesystem frame is involved: a
//! response that encodes pixels is decoded and inspected here.

use base64::Engine;
use image::GenericImageView;
use yunari_render_raster::sidecar::{Sessions, handle_line};

const THEME: &str = r##"{
  "schemaVersion":"3.0.0",
  "id":"sidecar",
  "artboard":{"width":40,"height":20},
  "background":{"color":{"value":"#0000ff"}},
  "scene":{"version":"7.4.0","objects":[
    {"id":"plate","type":"Rect","left":0,"top":0,"width":20,"height":20,
     "originX":"left","originY":"top","fill":"#ff0000"}
  ]}
}"##;

const CHART_THEME: &str = r##"{
  "schemaVersion":"3.0.0",
  "id":"chart",
  "artboard":{"width":40,"height":40},
  "palette":{
    "track":{"name":"Track","value":{"kind":"solid","color":"#223047"}},
    "ink":{"name":"Ink","value":{"kind":"solid","color":"#00b8d9"}}
  },
  "scene":{"version":"7.4.0","objects":[
    {"id":"gauge","type":"YunariChart","family":"gauge",
     "settings":{"track":{"kind":"solid","color":"#223047"},
                 "progress":{"kind":"solid","color":"#00b8d9"}},
     "left":0,"top":0,"width":40,"height":40,"originX":"left","originY":"top"}
  ]},
  "bindings":{"gauge":[{"id":"a","semanticKey":"cpu.load"}]}
}"##;

/// A gauge and a text value bound to the same key with `scale:2,offset:1`, so
/// text and chart marks must both show the transformed value.
const SCALED_THEME: &str = r##"{
  "schemaVersion":"3.0.0",
  "id":"scaled",
  "artboard":{"width":120,"height":120},
  "palette":{"ink":{"name":"Ink","value":{"kind":"solid","color":"#ffffff"}}},
  "typeTokens":{"body":{"family":"Inter","weight":400}},
  "scene":{"version":"7.4.0","objects":[
    {"id":"label","type":"Textbox","left":0,"top":0,"width":120,"height":30,
     "originX":"left","originY":"top","fontSize":20,"fill":"#ffffff",
     "yunariText":{"runs":[{"kind":"value","bindingId":"b",
        "typeToken":"typeTokens.body","style":{"fontSize":{"value":20}}}]}},
    {"id":"gauge","type":"YunariChart","family":"gauge",
     "settings":{"track":{"kind":"solid","color":"#223047"},
                 "progress":{"kind":"solid","color":"#00b8d9"}},
     "left":0,"top":30,"width":120,"height":90,"originX":"left","originY":"top"}
  ]},
  "bindings":{
    "label":[{"id":"b","semanticKey":"cpu.load","scale":2,"offset":1}],
    "gauge":[{"id":"b","semanticKey":"cpu.load","scale":2,"offset":1}]
  }
}"##;

/// A line chart on its own, with a bright stroke on a dark board so a painted
/// polyline can be counted in the decoded frame.
const LINE_THEME: &str = r##"{
  "schemaVersion":"3.0.0",
  "id":"line",
  "artboard":{"width":120,"height":80},
  "background":{"color":{"value":"#000000"}},
  "scene":{"version":"7.4.0","objects":[
    {"id":"trend","type":"YunariChart","family":"line",
     "settings":{"lineWidth":4,"min":0,"max":100,"showAxes":false,
       "windowSeconds":60,"maxPoints":10,
       "stroke":{"kind":"solid","color":"#00ff00"}},
     "left":0,"top":0,"width":120,"height":80,
     "originX":"left","originY":"top"}
  ]},
  "bindings":{"trend":[{"id":"a","semanticKey":"cpu.load"}]}
}"##;

/// Two gauges share `cpu.load` but own different transforms: `left` uses
/// scale:1,offset:0 and `right` uses scale:2,offset:1. Each chart must use its
/// own object's binding, not the first one found for the key.
const CONFLICTING_THEME: &str = r##"{
  "schemaVersion":"3.0.0",
  "id":"conflict",
  "artboard":{"width":120,"height":40},
  "scene":{"version":"7.4.0","objects":[
    {"id":"left","type":"YunariChart","family":"gauge",
     "settings":{"track":{"kind":"solid","color":"#223047"},
                 "progress":{"kind":"solid","color":"#00b8d9"}},
     "left":0,"top":0,"width":40,"height":40,"originX":"left","originY":"top"},
    {"id":"right","type":"YunariChart","family":"gauge",
     "settings":{"track":{"kind":"solid","color":"#223047"},
                 "progress":{"kind":"solid","color":"#ff0000"}},
     "left":60,"top":0,"width":40,"height":40,"originX":"left","originY":"top"}
  ]},
  "bindings":{
    "left":[{"id":"a","semanticKey":"cpu.load"}],
    "right":[{"id":"a","semanticKey":"cpu.load","scale":2,"offset":1}]
  }
}"##;

/// One gauge at a known transform and colour, used as the reference half of
/// the conflicting theme. `id`, `left`, `scale`/`offset` and `colour` are the
/// only differences.
fn single_gauge_theme(id: &str, scale_offset: &str, color: &str, left: u32) -> String {
    format!(
        r##"{{"schemaVersion":"3.0.0","id":"conflict","artboard":{{"width":120,"height":40}},
          "scene":{{"version":"7.4.0","objects":[
            {{"id":"{id}","type":"YunariChart","family":"gauge",
             "settings":{{"track":{{"kind":"solid","color":"#223047"}},
                         "progress":{{"kind":"solid","color":"{color}"}}}},
             "left":{left},"top":0,"width":40,"height":40,
             "originX":"left","originY":"top"}}]}},
          "bindings":{{"{id}":[{{"id":"a","semanticKey":"cpu.load"{scale_offset}}}]}}}}"##
    )
}

fn open(id: &str, theme: &str, format: &str) -> String {
    serde_json::json!({
        "type": "open",
        "id": id,
        "theme_json": theme,
        "width": 40,
        "height": if theme == CHART_THEME { 40 } else { 20 },
        "format": format
    })
    .to_string()
}

fn open_with_geometry(id: &str, theme: &str, format: &str, width: u32, height: u32) -> String {
    serde_json::json!({
        "type": "open",
        "id": id,
        "theme_json": theme,
        "width": width,
        "height": height,
        "format": format
    })
    .to_string()
}

fn open_with_interval(
    id: &str,
    theme: &str,
    format: &str,
    width: u32,
    height: u32,
    interval_ms: u32,
) -> String {
    serde_json::json!({
        "type": "open",
        "id": id,
        "theme_json": theme,
        "width": width,
        "height": height,
        "format": format,
        "frame_interval_ms": interval_ms
    })
    .to_string()
}

fn reply(state: &mut Sessions, request: serde_json::Value) -> serde_json::Value {
    let line = handle_line(state, &request.to_string()).expect("a response line");
    let line = line.expect("a request always answers");
    serde_json::from_str(&line).expect("the response is JSON")
}

#[test]
fn a_line_chart_accumulates_samples_from_frames_without_a_series_field() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open_with_geometry("d1", LINE_THEME, "png", 120, 80)).unwrap(),
    );
    let mut response = serde_json::Value::Null;
    for value in [10.0, 40.0, 80.0] {
        response = reply(
            &mut state,
            serde_json::json!({
                "type": "frame",
                "id": "d1",
                "readings": [
                    {"semantic_key": "cpu.load", "value": value, "status": "ok"}
                ]
            }),
        );
    }
    assert_eq!(response["ok"], true, "{response}");
    let picture = image::load_from_memory_with_format(&bytes(&response), image::ImageFormat::Png)
        .expect("a PNG frame");
    let green = picture
        .pixels()
        .filter(|(_, _, pixel)| pixel[1] > 200 && pixel[2] < 50 && pixel[0] < 50)
        .count();
    assert!(
        green > 30,
        "three readings should paint a polyline, got {green} green pixels"
    );
}

#[test]
fn an_open_with_a_panel_interval_still_answers_with_a_frame() {
    let mut state = Sessions::new();
    let response = reply(
        &mut state,
        serde_json::from_str(&open_with_interval("d1", THEME, "png", 40, 20, 110)).unwrap(),
    );
    assert_eq!(response["ok"], true);
    assert!(!bytes(&response).is_empty());
}

#[test]
fn a_zero_panel_interval_is_refused_at_the_boundary() {
    let mut state = Sessions::new();
    let request = serde_json::json!({
        "type": "open",
        "id": "d1",
        "theme_json": THEME,
        "width": 40,
        "height": 20,
        "format": "png",
        "frame_interval_ms": 0
    });
    let error = handle_line(&mut state, &request.to_string())
        .expect_err("a zero interval is not a cadence");
    assert!(error.to_string().contains("frame_interval_ms"));
}

fn bytes(response: &serde_json::Value) -> Vec<u8> {
    let encoded = response["bytes_base64"].as_str().expect("a base64 frame");
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("decodable base64")
}

fn codes(response: &serde_json::Value) -> Vec<&str> {
    response["issues"]
        .as_array()
        .expect("an issues array")
        .iter()
        .map(|issue| issue["code"].as_str().expect("a code"))
        .collect()
}

#[test]
fn open_returns_ok_with_the_declared_geometry() {
    let mut state = Sessions::new();
    let response = reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "png")).unwrap(),
    );
    assert_eq!(response["ok"], true);
    assert_eq!(response["id"], "d1");
    assert_eq!(response["width"], 40);
    assert_eq!(response["height"], 20);
    assert!(!bytes(&response).is_empty());
    assert!(response["raster_ms"].as_f64().is_some_and(|ms| ms >= 0.0));
}

/// A panel turn belongs to the session, not to the frame request: the host
/// opens a session per display and every frame must carry that display's
/// rotation. Without it a 90-degree panel paints an unturned picture.
#[test]
fn an_open_rotation_turns_every_frame() {
    fn frame_pixels(rotation: Option<u16>) -> Vec<u8> {
        let mut state = Sessions::new();
        let mut request: serde_json::Value =
            serde_json::from_str(&open("d1", THEME, "png")).unwrap();
        if let Some(degrees) = rotation {
            request["rotation"] = serde_json::json!(degrees);
        }
        reply(&mut state, request);
        bytes(&reply(
            &mut state,
            serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
        ))
    }

    let upright = frame_pixels(None);
    let turned = frame_pixels(Some(90));
    assert_ne!(upright, turned, "a 90-degree session must turn its frame");
}

#[test]
fn an_out_of_range_rotation_is_refused() {
    let mut state = Sessions::new();
    let mut request: serde_json::Value = serde_json::from_str(&open("d1", THEME, "png")).unwrap();
    request["rotation"] = serde_json::json!(70_000);
    let response = handle_line(&mut state, &request.to_string())
        .expect_err("an out-of-range turn is refused")
        .to_response_line();
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["ok"], false);
    assert!(response["error"].as_str().unwrap().contains("rotation"));
}

#[test]
fn frame_with_a_known_reading_decodes_to_a_non_empty_picture() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", CHART_THEME, "png")).unwrap(),
    );
    let response = reply(
        &mut state,
        serde_json::json!({
            "type": "frame",
            "id": "d1",
            "readings": [{"semantic_key":"cpu.load","value":42.5,"status":"ok"}],
            "series": {}
        }),
    );
    assert_eq!(response["ok"], true);
    assert_eq!(response["width"], 40);
    assert_eq!(response["height"], 40);
    let picture = bytes(&response);
    assert!(!picture.is_empty());
    let decoded = image::load_from_memory_with_format(&picture, image::ImageFormat::Png)
        .expect("a PNG frame");
    assert_eq!(decoded.width(), 40);
    assert_eq!(decoded.height(), 40);
}

#[test]
fn chart_samples_change_the_picture_and_a_missing_reading_is_an_issue() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", CHART_THEME, "png")).unwrap(),
    );

    let empty = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    assert!(codes(&empty).contains(&"missing-reading"), "{empty}");

    let sampled = reply(
        &mut state,
        serde_json::json!({
            "type":"frame","id":"d1",
            "readings":[{"semantic_key":"cpu.load","value":90.0,"status":"ok"}],
            "series":{"cpu.load":[10.0,20.0,30.0]}
        }),
    );
    assert!(!codes(&sampled).contains(&"missing-reading"), "{sampled}");
    assert_ne!(bytes(&sampled), bytes(&empty), "a value mark was painted");
}

#[test]
fn an_unmapped_key_stays_absent_without_a_fabricated_zero() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "png")).unwrap(),
    );

    let plain = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    let with_unmapped = reply(
        &mut state,
        serde_json::json!({
            "type":"frame","id":"d1",
            "readings":[{"semantic_key":"nobody.listens","value":0.0,"status":"ok"}],
            "series":{"nobody.listens":[1.0,2.0]}
        }),
    );
    assert!(
        codes(&with_unmapped).contains(&"unmapped-key"),
        "{with_unmapped}"
    );
    assert_eq!(
        bytes(&with_unmapped),
        bytes(&plain),
        "an unbound key must not change a single pixel"
    );
}

#[test]
fn overlay_hides_the_background_layer() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "png")).unwrap(),
    );

    let full = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    let overlay = reply(
        &mut state,
        serde_json::json!({"type":"overlay","id":"d1","readings":[],"series":{}}),
    );
    assert_eq!(overlay["ok"], true);

    let pixel = |response: &serde_json::Value, x, y| -> [u8; 4] {
        let picture =
            image::load_from_memory_with_format(&bytes(response), image::ImageFormat::Png)
                .expect("a PNG frame")
                .to_rgba8();
        let at = (y as usize * picture.width() as usize + x as usize) * 4;
        let data = picture.as_raw();
        [data[at], data[at + 1], data[at + 2], data[at + 3]]
    };
    // Outside the red plate the page's blue fill shows in a frame.
    assert_eq!(pixel(&full, 35, 10), [0, 0, 255, 255]);
    // The overlay carries only the scene: the same spot is transparent.
    assert_eq!(pixel(&overlay, 35, 10)[3], 0);
    // The plate itself still paints.
    assert_eq!(pixel(&overlay, 10, 10), [255, 0, 0, 255]);
}

#[test]
fn jpeg_is_a_supported_still_format() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "jpeg")).unwrap(),
    );
    let response = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    let picture = bytes(&response);
    let decoded = image::load_from_memory_with_format(&picture, image::ImageFormat::Jpeg)
        .expect("a JPEG frame");
    assert_eq!((decoded.width(), decoded.height()), (40, 20));
}

#[test]
fn close_then_frame_is_an_error_and_does_not_panic() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "png")).unwrap(),
    );
    reply(&mut state, serde_json::json!({"type":"close","id":"d1"}));
    let error = handle_line(
        &mut state,
        &serde_json::json!({"type":"frame","id":"d1","readings":[]}).to_string(),
    )
    .expect_err("a closed session has no frame");
    let response: serde_json::Value =
        serde_json::from_str(&error.to_response_line()).expect("an error response");
    assert_eq!(response["ok"], false);
    assert!(response["error"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn malformed_json_is_an_error_and_the_loop_continues() {
    let mut state = Sessions::new();
    let error = handle_line(&mut state, "{not json").expect_err("malformed JSON refused");
    let response: serde_json::Value =
        serde_json::from_str(&error.to_response_line()).expect("an error response");
    assert_eq!(response["ok"], false);

    // The same session still accepts the next line: a bad line never poisons it.
    let next = reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "png")).unwrap(),
    );
    assert_eq!(next["ok"], true);
}

#[test]
fn an_unknown_request_type_is_refused() {
    let mut state = Sessions::new();
    assert!(handle_line(&mut state, r#"{"type":"explode"}"#).is_err());
}

#[test]
fn shutdown_ends_the_loop_without_a_response_line() {
    let mut state = Sessions::new();
    assert_eq!(
        handle_line(&mut state, r#"{"type":"shutdown"}"#).expect("shutdown is valid"),
        None
    );
}

// ---- Review fixes -------------------------------------------------------

#[test]
fn overlay_is_always_png_even_for_a_jpeg_session() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "jpeg")).unwrap(),
    );
    let overlay = reply(
        &mut state,
        serde_json::json!({"type":"overlay","id":"d1","readings":[],"series":{}}),
    );
    assert_eq!(overlay["ok"], true);
    // A JPEG has no alpha: decoding the overlay bytes as a PNG must work.
    let picture = image::load_from_memory_with_format(&bytes(&overlay), image::ImageFormat::Png)
        .expect("overlay bytes are a PNG even on a jpeg session");
    let rgba = picture.to_rgba8();
    let at = (10 * rgba.width() as usize + 35) * 4;
    assert_eq!(rgba.as_raw()[at + 3], 0, "the board's paint is gone");
}

#[test]
fn response_geometry_matches_the_encoded_pixels() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open_with_geometry("d1", THEME, "png", 80, 40)).unwrap(),
    );
    let response = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    assert_eq!(response["width"], 80);
    assert_eq!(response["height"], 40);
    let picture = image::load_from_memory_with_format(&bytes(&response), image::ImageFormat::Png)
        .expect("a PNG frame");
    assert_eq!(
        (picture.width(), picture.height()),
        (80, 40),
        "the declared geometry is the encoded geometry"
    );
}

/// The raw value 10 with scale:2, offset:1 must display as 21 (text) and drive
/// the gauge as 21 of 100. A binding is the single owner of that transform.
#[test]
fn binding_scale_and_offset_apply_to_both_text_and_chart_marks() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open_with_geometry("d1", SCALED_THEME, "png", 120, 120)).unwrap(),
    );

    // The same document without the binding transform: a raw 21 there must
    // paint what a raw 10 paints under scale:2, offset:1.
    let plain_theme = SCALED_THEME.replace(",\"scale\":2,\"offset\":1", "");
    let frame = |theme: &str, value: f64, series: serde_json::Value| {
        let mut state = Sessions::new();
        reply(
            &mut state,
            serde_json::from_str(&open_with_geometry("d", theme, "png", 120, 120)).unwrap(),
        );
        reply(
            &mut state,
            serde_json::json!({
                "type":"frame","id":"d",
                "readings":[{"semantic_key":"cpu.load","value":value,"status":"ok"}],
                "series":series
            }),
        )
    };

    let scaled = frame(
        SCALED_THEME,
        10.0,
        serde_json::json!({"cpu.load":[10.0,11.0]}),
    );
    let expected = frame(
        &plain_theme,
        21.0,
        serde_json::json!({"cpu.load":[21.0,23.0]}),
    );
    // Both the text run and the chart read the one binding transform, so the
    // raw value under scale/offset equals the pre-scaled value without it.
    assert_eq!(
        bytes(&scaled),
        bytes(&expected),
        "raw 10 with scale 2/offset 1 must paint like pre-scaled 21"
    );
}

#[test]
fn issue_codes_use_renderer_core_kebab_case() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open("d1", THEME, "png")).unwrap(),
    );
    let response = reply(
        &mut state,
        serde_json::json!({
            "type":"frame","id":"d1",
            "readings":[{"semantic_key":"nobody.listens","value":0.0,"status":"ok"}],
            "series":{}
        }),
    );
    assert!(codes(&response).contains(&"unmapped-key"), "{response}");
    assert!(
        response["issues"].as_array().unwrap().iter().all(|issue| {
            let code = issue["code"].as_str().unwrap();
            !code.contains(char::is_uppercase)
        }),
        "all codes are kebab-case"
    );
}

#[test]
fn reading_status_and_reason_are_preserved() {
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open_with_geometry("d1", SCALED_THEME, "png", 120, 120)).unwrap(),
    );
    let response = reply(
        &mut state,
        serde_json::json!({
            "type":"frame","id":"d1",
            "readings":[{"semantic_key":"cpu.load","value":42.0,"status":"error",
                         "message":"the sensor is asleep"}],
            "series":{}
        }),
    );
    assert!(
        response["issues"].as_array().unwrap().iter().any(|issue| {
            issue["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("the sensor is asleep"))
        }),
        "the incoming reason is carried, not replaced: {response}"
    );
}

#[test]
fn asset_paths_cannot_escape_the_theme_dir() {
    let mut state = Sessions::new();
    let dir = std::env::temp_dir().join(format!("yunari-sidecar-assets-{}", std::process::id()));
    let theme_dir = dir.join("theme");
    let assets_dir = theme_dir.join("assets");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&assets_dir).expect("a theme dir with assets");

    // The path string passes the model's own `assets/` rule, but the file it
    // names is a link pointing outside the theme dir. A resolved path must stay
    // under theme_dir or the session must refuse it.
    let outside = dir.join("secret.png");
    std::fs::write(&outside, b"outside").expect("a file outside the theme dir");
    let link = assets_dir.join("pic.png");
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside, &link).expect("a file link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).expect("a file link");

    let theme = r##"{"schemaVersion":"3.0.0","id":"esc","artboard":{"width":10,"height":10},
        "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
        "scene":{"version":"7.4.0","objects":[
          {"id":"mark","type":"Rect","left":2,"top":2,"width":6,"height":4,
           "originX":"left","originY":"top","fill":"#ff00ff"}]}}"##;
    let request = serde_json::json!({
        "type":"open","id":"d1","theme_json":theme,
        "width":10,"height":10,"format":"png",
        "theme_dir": theme_dir.to_string_lossy()
    });
    let error = handle_line(&mut state, &request.to_string())
        .expect_err("an asset that resolves outside the theme dir is refused");
    let response: serde_json::Value =
        serde_json::from_str(&error.to_response_line()).expect("an error response");
    assert_eq!(response["ok"], false);
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|detail| detail.contains("escap") || detail.contains("outside")),
        "the refusal names the reason: {response}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An advanced background with a real picture: the sidecar measures the source
/// at open, so the frame shows the picture in its band and the fill below it.
#[test]
fn an_advanced_background_composites_the_picture_into_the_frame() {
    let dir = std::env::temp_dir().join(format!("yunari-backdrop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("a theme dir with assets");
    let mut picture = image::RgbaImage::new(2, 2);
    for pixel in picture.pixels_mut() {
        *pixel = image::Rgba([0, 0, 255, 255]);
    }
    picture
        .save(dir.join("assets/pic.png"))
        .expect("the picture is written");

    let theme = r##"{"schemaVersion":"3.0.0","id":"backdrop","artboard":{"width":20,"height":20},
        "background":{"mode":"advanced","media":{"assetId":"pic","bounds":"display",
          "color":{"value":"#00ff00"},
          "composition":{"anchor":"top","extent":0.5,"fade":1.0}}},
        "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
        "scene":{"version":"7.4.0","objects":[
          {"id":"mark","type":"Rect","left":2,"top":2,"width":6,"height":4,
           "originX":"left","originY":"top","fill":"#ff00ff"}]}}"##;
    let request = serde_json::json!({
        "type":"open","id":"d1","theme_json":theme,
        "width":20,"height":20,"format":"png",
        "theme_dir": dir.to_string_lossy()
    });
    let mut state = Sessions::new();
    let opened = reply(&mut state, request);
    assert_eq!(opened["ok"], true, "{opened}");
    let response = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    let rgba = image::load_from_memory_with_format(&bytes(&response), image::ImageFormat::Png)
        .expect("a PNG frame")
        .to_rgba8()
        .as_raw()
        .to_vec();
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let at = (y * 20 + x) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    };
    let band = pixel(10, 1);
    assert!(
        band[2] > 240 && band[1] < 16,
        "the picture fills the band: {band:?}"
    );
    let seam = pixel(10, 5);
    assert!(
        seam[2] > 40 && seam[1] > 40,
        "the fade blends picture and fill: {seam:?}"
    );
    assert_eq!(
        pixel(10, 15),
        [0, 255, 0, 255],
        "the fill stays below the band"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Unreadable bytes are not a picture. The session still paints the fill and
/// says why, so no fabricated image reaches the panel.
#[test]
fn an_unreadable_background_asset_reports_an_issue_without_a_picture() {
    let dir = std::env::temp_dir().join(format!("yunari-bad-backdrop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("a theme dir with assets");
    std::fs::write(dir.join("assets/pic.png"), b"not a picture").expect("unreadable bytes");

    let theme = r##"{"schemaVersion":"3.0.0","id":"bad","artboard":{"width":10,"height":10},
        "background":{"mode":"advanced","media":{"assetId":"pic","bounds":"display",
          "color":{"value":"#ff0000"},
          "composition":{"anchor":"top","extent":0.5,"fade":0.1}}},
        "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
        "scene":{"version":"7.4.0","objects":[
          {"id":"mark","type":"Rect","left":2,"top":2,"width":6,"height":4,
           "originX":"left","originY":"top","fill":"#ff00ff"}]}}"##;
    let request = serde_json::json!({
        "type":"open","id":"d1","theme_json":theme,
        "width":10,"height":10,"format":"png",
        "theme_dir": dir.to_string_lossy()
    });
    let mut state = Sessions::new();
    // `open` paints its first frame and carries the refusal there.
    let response = reply(&mut state, request);
    assert_eq!(response["ok"], true, "{response}");
    assert!(
        codes(&response).contains(&"unreadable-paint"),
        "the unreadable asset is reported at open: {response}"
    );
    // A fact about the theme is not a per-frame sensor gap: repeating it at
    // display rate floods the host's notice surface. Later frames stay quiet.
    let later = reply(
        &mut state,
        serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
    );
    assert!(
        !codes(&later).contains(&"unreadable-paint"),
        "the refusal is reported once, not on every frame: {later}"
    );
    let rgba = image::load_from_memory_with_format(&bytes(&response), image::ImageFormat::Png)
        .expect("a PNG frame")
        .to_rgba8()
        .as_raw()
        .to_vec();
    assert_eq!(&rgba[..4], &[255, 0, 0, 255], "only the fill paints");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn charts_with_the_same_key_use_their_own_binding_transform() {
    // Raw 10: the left gauge (no transform) shows 10/100; the right gauge
    // (scale:2, offset:1) shows 21/100. First-match-wins would paint both as 10.
    let mut state = Sessions::new();
    reply(
        &mut state,
        serde_json::from_str(&open_with_geometry("d", CONFLICTING_THEME, "png", 120, 40)).unwrap(),
    );
    let both = reply(
        &mut state,
        serde_json::json!({
            "type":"frame","id":"d",
            "readings":[{"semantic_key":"cpu.load","value":10.0,"status":"ok"}],
            "series":{}
        }),
    );
    let both_pixels = image::load_from_memory_with_format(&bytes(&both), image::ImageFormat::Png)
        .expect("a PNG frame")
        .to_rgba8()
        .as_raw()
        .to_vec();

    let reference = |theme: &str| -> Vec<u8> {
        let mut state = Sessions::new();
        reply(
            &mut state,
            serde_json::from_str(&open_with_geometry("r", theme, "png", 120, 40)).unwrap(),
        );
        let response = reply(
            &mut state,
            serde_json::json!({
                "type":"frame","id":"r",
                "readings":[{"semantic_key":"cpu.load","value":10.0,"status":"ok"}],
                "series":{}
            }),
        );
        image::load_from_memory_with_format(&bytes(&response), image::ImageFormat::Png)
            .expect("a PNG frame")
            .to_rgba8()
            .as_raw()
            .to_vec()
    };

    let left = reference(&single_gauge_theme("left", "", "#00b8d9", 0));
    let right = reference(&single_gauge_theme(
        "right",
        ",\"scale\":2,\"offset\":1",
        "#ff0000",
        60,
    ));

    // Each 40x40 half must equal its own reference. A global first-match
    // transform makes the right half equal a raw-10 gauge instead.
    let width = 120usize * 4;
    let left_half = |pixels: &[u8]| -> Vec<u8> {
        (0..40)
            .flat_map(|y| pixels[y * width..y * width + 40 * 4].iter().copied())
            .collect()
    };
    let right_half = |pixels: &[u8]| -> Vec<u8> {
        (0..40)
            .flat_map(|y| pixels[y * width + 60 * 4..(y + 1) * width].iter().copied())
            .collect()
    };
    assert_eq!(
        left_half(&both_pixels),
        left_half(&left),
        "left gauge owns scale 1"
    );
    assert_eq!(
        right_half(&both_pixels),
        right_half(&right),
        "right gauge owns scale 2, offset 1"
    );
}

/// A panel is the Chromium frame turned by the device. With an upright square
/// artboard the turned frame must be exactly the upright frame rotated a
/// quarter turn: every scene op and the backdrop have to land in the same
/// place they did before the device turned it. Inverse-mapping the backdrop
/// into artboard coordinates rotates it twice and moves the picture away from
/// the scene it belongs to.
#[test]
fn a_turned_panel_is_the_upright_frame_turned_by_the_device() {
    let dir = std::env::temp_dir().join(format!("yunari-turn-parity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("a theme dir with assets");
    // A source whose two halves differ: a wrong window or a second turn puts
    // the marker on the opposite side of the panel.
    let mut picture = image::RgbaImage::new(8, 4);
    for y in 0..4 {
        for x in 0..8 {
            picture.put_pixel(
                x,
                y,
                image::Rgba([
                    if x < 4 { 240 } else { 20 },
                    if y < 2 { 200 } else { 30 },
                    40,
                    255,
                ]),
            );
        }
    }
    picture
        .save(dir.join("assets/pic.png"))
        .expect("the picture is written");
    let theme = r##"{"schemaVersion":"3.0.0","id":"parity","artboard":{"width":20,"height":20},
        "background":{"mode":"advanced","media":{"assetId":"pic","bounds":"display",
          "color":{"value":"#111111"},
          "composition":{"anchor":"top","extent":0.75,"fade":0.01}}},
        "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
        "scene":{"version":"7.4.0","objects":[
          {"id":"mark","type":"Rect","left":2,"top":2,"width":6,"height":4,
           "originX":"left","originY":"top","fill":"#ff00ff"}]}}"##;
    let capture = |rotation: u16| -> Vec<u8> {
        let mut state = Sessions::new();
        let mut request: serde_json::Value =
            serde_json::from_str(&open("d1", theme, "png")).unwrap();
        request["theme_dir"] = serde_json::json!(dir.to_string_lossy());
        request["width"] = serde_json::json!(20);
        request["height"] = serde_json::json!(20);
        request["rotation"] = serde_json::json!(rotation);
        reply(&mut state, request);
        let response = reply(
            &mut state,
            serde_json::json!({"type":"frame","id":"d1","readings":[],"series":{}}),
        );
        image::load_from_memory_with_format(&bytes(&response), image::ImageFormat::Png)
            .expect("a PNG frame")
            .to_rgba8()
            .as_raw()
            .to_vec()
    };
    let upright = capture(0);
    let pixel = |frame: &[u8], x: usize, y: usize| -> [u8; 4] {
        let at = (y * 20 + x) * 4;
        [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
    };
    for rotation in [90, 180, 270] {
        let turned = capture(rotation);
        let mismatches = (0..20)
            .flat_map(|y| (0..20).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                // The same inverse map fit_raster_into uses to sample source.
                let angle = f32::from(rotation) * std::f32::consts::PI / 180.0;
                let (sin, cos) = angle.sin_cos();
                let dx = x as f32 + 0.5 - 10.0;
                let dy = y as f32 + 0.5 - 10.0;
                let sx = (dx * cos + dy * sin + 10.0) as usize;
                let sy = (-dx * sin + dy * cos + 10.0) as usize;
                let expected = pixel(&upright, sx.min(19), sy.min(19));
                let actual = pixel(&turned, x, y);
                expected.iter().zip(actual).any(|(a, b)| a.abs_diff(b) > 12)
            })
            .count();
        assert_eq!(
            mismatches, 0,
            "the {rotation}-degree panel must show the upright frame turned once"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}
