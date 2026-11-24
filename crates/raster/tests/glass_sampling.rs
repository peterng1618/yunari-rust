//! The native sidecar's glass sampling rule: an explicit `glass_sampling` and
//! a held poster for video devices. The whole-frame fit preserves the authored
//! artboard composition, so the card frosts the same media on a panel whose
//! pixel box differs from the artboard.
//!
//! Every frame is decoded from the wire response, so the assertions are about
//! the pixels a panel receives, not about a private renderer helper.

use std::path::PathBuf;

use base64::Engine;
use yunari_render_raster::sidecar::{Sessions, handle_line};

/// A theme whose artboard is twice the panel's width. Two colour fields sit
/// where the glass card is; contain fitting scales the whole board to 20×20,
/// so sampling artboard coordinates instead of panel coordinates reads the
/// wrong field.
const WIDE_THEME: &str = r##"{
  "schemaVersion":"3.0.0",
  "id":"fit-glass",
  "artboard":{"width":40,"height":20},
  "background":{"media":{"assetId":"poster","bounds":"theme",
    "color":{"value":"#000000"}}},
  "assets":[{"id":"poster","kind":"image","path":"assets/poster.png"}],
  "scene":{"version":"7.4.0","objects":[
    {"id":"card","type":"Rect","left":16,"top":4,"width":8,"height":12,
     "originX":"left","originY":"top","fill":"rgba(0,0,0,0)",
     "yunariGlass":{"blurRadius":3}}
  ]}
}"##;

fn picture_dir(tag: &str, left: [u8; 3], right: [u8; 3]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yunari-poster-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).expect("a theme with an assets dir");
    let mut picture = image::RgbaImage::new(20, 20);
    for y in 0..20 {
        for x in 0..20 {
            let color = if x < 10 { left } else { right };
            picture.put_pixel(x, y, image::Rgba([color[0], color[1], color[2], 255]));
        }
    }
    picture
        .save(dir.join("assets/poster.png"))
        .expect("the poster is written");
    dir
}

fn reply(state: &mut Sessions, request: serde_json::Value) -> serde_json::Value {
    let line = handle_line(state, &request.to_string()).expect("a response line");
    let line = line.expect("a request always answers");
    serde_json::from_str(&line).expect("the response is JSON")
}

fn open(theme: &str, width: u32, height: u32, root: Option<&PathBuf>) -> serde_json::Value {
    serde_json::json!({
        "type": "open",
        "id": "panel",
        "theme_json": theme,
        "width": width,
        "height": height,
        "format": "png",
        "glass_sampling": "static",
        "poster_root": root.map(|path| path.to_string_lossy().to_string()),
        "poster_path": root.map(|path| path.join("assets/poster.png").to_string_lossy().to_string()),
    })
}

fn frame(state: &mut Sessions) -> image::RgbaImage {
    let response = reply(
        state,
        serde_json::json!({"type":"frame","id":"panel","readings":[],"series":{}}),
    );
    assert_eq!(response["ok"], true, "{response}");
    let encoded = response["bytes_base64"].as_str().expect("a base64 frame");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("decodable bytes");
    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .expect("a PNG frame")
        .to_rgba8()
}

#[test]
fn a_poster_without_a_named_cache_root_is_refused() {
    let mut state = Sessions::new();
    let mut request = open(WIDE_THEME, 20, 20, None);
    request["poster_path"] = serde_json::json!("C:/themes/any/poster.png");
    let error = handle_line(&mut state, &request.to_string())
        .expect_err("a poster needs its allowed root")
        .to_string();
    assert!(error.contains("poster_root"), "{error}");
}

#[test]
fn an_unknown_glass_sampling_is_refused_at_the_boundary() {
    let mut state = Sessions::new();
    let mut request = open(WIDE_THEME, 20, 20, None);
    request["glass_sampling"] = serde_json::json!("live");
    let error = handle_line(&mut state, &request.to_string())
        .expect_err("an unknown rule is not a sampling mode")
        .to_string();
    assert!(error.contains("glass_sampling"), "{error}");
}

#[test]
fn a_static_session_uses_the_poster_and_holds_it_across_ticks() {
    let dir = picture_dir("static", [240, 40, 40], [40, 40, 240]);
    let mut state = Sessions::new();
    reply(&mut state, open(WIDE_THEME, 20, 20, Some(&dir)));
    let first = frame(&mut state);
    let second = frame(&mut state);
    let inside = first.get_pixel(16, 10).0;
    assert!(
        i32::from(inside[2]) > i32::from(inside[0]) + 80,
        "glass must sample the poster's blue half: {inside:?}"
    );
    assert_eq!(
        first.get_pixel(8, 10).0,
        second.get_pixel(8, 10).0,
        "a static session holds one poster across ticks"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn glass_sampling_maps_the_card_through_the_inverse_panel_fit() {
    let dir = picture_dir("fit", [240, 40, 40], [40, 40, 240]);
    let mut state = Sessions::new();
    reply(&mut state, open(WIDE_THEME, 20, 20, Some(&dir)));
    let picture = frame(&mut state);
    // The artboard is 40×20 in a 20×20 panel: contain fits at 0.5 and centres.
    // The card at artboard x 16..24 lands at device x 8..12, straddling the
    // poster's red/blue edge at device x 10. A blur there must mix both sides;
    // a card left in artboard coordinates samples outside the panel and never
    // frosts the edge.
    let edge = picture.get_pixel(10, 10).0;
    assert!(
        i32::from(edge[0]) > 30 && i32::from(edge[2]) > 30,
        "the fitted card must frost the poster's edge: {edge:?}"
    );
    let outside = picture.get_pixel(4, 10).0;
    assert!(
        i32::from(outside[0]) > i32::from(outside[2]) + 80,
        "outside the card the panel still shows the red field: {outside:?}"
    );

    // The same composition authored at twice the artboard size must land on
    // the same device pixels: the whole frame fit preserves the authored
    // relationship between the card and its media.
    let doubled = WIDE_THEME
        .replace("\"width\":40,\"height\":20", "\"width\":80,\"height\":40")
        .replace(
            "\"left\":16,\"top\":4,\"width\":8,\"height\":12",
            "\"left\":32,\"top\":8,\"width\":16,\"height\":24",
        );
    let mut other = Sessions::new();
    reply(&mut other, open(&doubled, 20, 20, Some(&dir)));
    let other_picture = frame(&mut other);
    for (x, y) in [(10, 10), (4, 10), (16, 10)] {
        let a = picture.get_pixel(x, y).0;
        let b = other_picture.get_pixel(x, y).0;
        // The blur is authored in artboard units, so doubling the artboard
        // halves the device-space radius and the two resamplings are not
        // pixel-identical. The fitted region and its colour must still agree.
        for channel in 0..3 {
            assert!(
                a[channel].abs_diff(b[channel]) <= 35,
                "both artboard derivations must fit to the same panel region ({x},{y}): {a:?} vs {b:?}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_absolute_poster_outside_the_hosts_cache_is_refused() {
    let dir = picture_dir("escape", [240, 40, 40], [40, 40, 240]);
    let cache = std::env::temp_dir().join(format!(
        "yunari-poster-cache-{}-{}",
        "escape",
        std::process::id()
    ));
    let outside = std::env::temp_dir().join(format!(
        "yunari-poster-outside-{}-{}",
        "escape",
        std::process::id()
    ));
    for path in [&cache, &outside] {
        let _ = std::fs::remove_dir_all(path);
        std::fs::create_dir_all(path).expect("a directory");
    }
    std::fs::copy(dir.join("assets/poster.png"), outside.join("poster.png"))
        .expect("a readable picture outside the host's cache");

    let mut state = Sessions::new();
    let mut request = open(WIDE_THEME, 20, 20, Some(&cache));
    request["poster_path"] =
        serde_json::json!(outside.join("poster.png").to_string_lossy().to_string());
    let error = handle_line(&mut state, &request.to_string())
        .expect_err("an absolute path outside the named cache is not readable")
        .to_string();
    assert!(error.contains("poster_path"), "{error}");

    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(&cache);
    let _ = std::fs::remove_dir_all(&outside);
}
