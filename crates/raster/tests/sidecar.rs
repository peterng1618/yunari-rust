//! JSON-lines panel sidecar, driven through its pure handler.
//!
//! The tests send the exact wire shapes the host will send and read the exact
//! JSON responses back. No process, socket or filesystem frame is involved: a
//! response that encodes pixels is decoded and inspected here.

use base64::Engine;
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

fn reply(state: &mut Sessions, request: serde_json::Value) -> serde_json::Value {
    let line = handle_line(state, &request.to_string()).expect("a response line");
    let line = line.expect("a request always answers");
    serde_json::from_str(&line).expect("the response is JSON")
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
    assert!(codes(&empty).contains(&"MissingReading"), "{empty}");

    let sampled = reply(
        &mut state,
        serde_json::json!({
            "type":"frame","id":"d1",
            "readings":[{"semantic_key":"cpu.load","value":90.0,"status":"ok"}],
            "series":{"cpu.load":[10.0,20.0,30.0]}
        }),
    );
    assert!(!codes(&sampled).contains(&"MissingReading"), "{sampled}");
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
        codes(&with_unmapped).contains(&"UnmappedKey"),
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
