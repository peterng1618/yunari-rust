//! JSON-lines panel sidecar: the protocol a host drives over stdin/stdout.
//!
//! One request per line in, one response per line out. The binary owns nothing
//! but framing; all behaviour lives here in a pure handler so the wire shape is
//! testable without spawning a process, opening a socket or touching a device.
//!
//! No credentials, no sockets, no writes to anything but the response stream:
//! a session holds an in-memory theme and its caches. A refused theme produces
//! an error response, never a blank frame, so the host can fall back to
//! Chromium without showing a broken panel.

use std::collections::HashMap;
use std::time::Instant;

use base64::Engine as _;
use serde_json::{Map, Value, json};
use yunari_model::document::ThemeDocument;
use yunari_providers::sample::{Reading, Status};
use yunari_providers::units::UnitPreferences;
use yunari_render::{Fonts, IssueCode, PlanIssue, Readings};

use crate::{ChartSamples, Images, render_with_charts};

type ReadingsById = HashMap<String, Option<f64>>;
type SeriesById = HashMap<String, Vec<(u64, f64)>>;

/// A theme and the caches that make its frames cheap to rebuild.
struct Session {
    document: ThemeDocument,
    fonts: Fonts,
    images: Images,
    width: u32,
    height: u32,
    format: Format,
}

#[derive(Clone, Copy)]
enum Format {
    Png,
    Jpeg,
}

/// Every open session, keyed by the host's display id.
#[derive(Default)]
pub struct Sessions {
    by_id: HashMap<String, Session>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }
}

/// An error that must reach the host as `{ok:false,error}` on its own line.
#[derive(Debug)]
pub struct ProtocolError(String);

impl ProtocolError {
    /// The wire response for a refused request.
    pub fn to_response_line(&self) -> String {
        json!({"ok": false, "error": self.0}).to_string()
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolError {}

fn fail(message: impl Into<String>) -> ProtocolError {
    ProtocolError(message.into())
}

/// Handle one JSON-lines request. `Ok(None)` is an orderly `shutdown`: the loop
/// stops without writing a response line. Every other request answers once.
pub fn handle_line(state: &mut Sessions, line: &str) -> Result<Option<String>, ProtocolError> {
    let request: Value =
        serde_json::from_str(line).map_err(|error| fail(format!("Malformed request: {error}")))?;
    let object = request
        .as_object()
        .ok_or_else(|| fail("A request must be a JSON object."))?;
    match field(object, "type")? {
        "open" => Ok(Some(open(state, object)?)),
        "frame" => Ok(Some(frame(state, object, false)?)),
        "overlay" => Ok(Some(frame(state, object, true)?)),
        "close" => Ok(Some(close(state, object)?)),
        "shutdown" => Ok(None),
        other => Err(fail(format!("Unknown request type {other:?}."))),
    }
}

fn field<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, ProtocolError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| fail(format!("A request needs a string {name:?}.")))
}

fn id(object: &Map<String, Value>) -> Result<String, ProtocolError> {
    Ok(field(object, "id")?.to_owned())
}

fn u32_field(object: &Map<String, Value>, name: &str) -> Result<u32, ProtocolError> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| fail(format!("A request needs an unsigned integer {name:?}.")))
}

fn open(state: &mut Sessions, object: &Map<String, Value>) -> Result<String, ProtocolError> {
    let id = id(object)?;
    let theme_json = field(object, "theme_json")?;
    let width = u32_field(object, "width")?;
    let height = u32_field(object, "height")?;
    let format = match field(object, "format")? {
        "png" => Format::Png,
        "jpeg" => Format::Jpeg,
        other => return Err(fail(format!("Unsupported still format {other:?}."))),
    };

    let document = yunari_model::document::parse_document(theme_json)
        .map_err(|error| fail(format!("Refused theme: {error}")))?;
    let mut fonts = Fonts::new();
    // Theme faces are read from the theme's own directory. The host names that
    // directory; every other path stays closed to a session.
    let base = object.get("theme_dir").and_then(Value::as_str);
    fonts.load_faces(&document, |asset| {
        base.and_then(|dir| std::fs::read(std::path::Path::new(dir).join(asset)).ok())
    });
    let images = match base {
        Some(dir) => Images::at(dir),
        None => Images::empty(),
    };
    state.by_id.insert(
        id.clone(),
        Session {
            document,
            fonts,
            images,
            width,
            height,
            format,
        },
    );
    // Open answers with a first, empty-input frame. The host can refuse a
    // refused theme here (no blank fallback) and already has bytes to present.
    frame(state, object, false)
}

fn frame(
    state: &mut Sessions,
    object: &Map<String, Value>,
    overlay: bool,
) -> Result<String, ProtocolError> {
    let id = id(object)?;
    let session = state
        .by_id
        .get_mut(&id)
        .ok_or_else(|| fail(format!("No open session for {id:?}.")))?;

    let (readings, series, mut issues) = parse_frame(object, &session.document)?;
    let (mut list, build_issues) = yunari_render::fabric::build(
        &session.document,
        &mut session.fonts,
        &Readings::new(readings.iter().map(|(key, value)| Reading {
            semantic_key: key.clone(),
            status: if value.is_some() {
                Status::Ok
            } else {
                Status::Missing
            },
            value: *value,
            ..Reading::missing(key, "no sample this frame")
        })),
        &UnitPreferences::default(),
    );
    issues.extend(build_issues);

    // The overlay is the UI alone, composited by the device over its own clip.
    // Hiding the board's paint and its backdrop is the whole difference; the
    // scene's own objects still paint, with alpha, so the clip shows through.
    if overlay {
        list.background = None;
        list.backdrop = None;
    }

    let samples = ChartSamples {
        readings: &readings,
        series: &series,
    };
    let started = Instant::now();
    let (raster, chart_issues) = render_with_charts(
        &list,
        1.0,
        Some(&mut session.fonts),
        Some(&mut session.images),
        Some(&session.document),
        &samples,
    );
    issues.extend(chart_issues);
    let raster_ms = started.elapsed().as_secs_f64() * 1000.0;

    let picture = image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba)
        .ok_or_else(|| fail("The frame does not fill the panel."))?;
    let mut bytes = Vec::new();
    match session.format {
        Format::Png => picture
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .map_err(|error| fail(format!("Could not encode PNG: {error}")))?,
        Format::Jpeg => {
            let rgb = image::DynamicImage::ImageRgba8(picture).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut std::io::Cursor::new(&mut bytes),
                85,
            )
            .encode_image(&rgb)
            .map_err(|error| fail(format!("Could not encode JPEG: {error}")))?;
        }
    }
    Ok(response(
        &id,
        session.width,
        session.height,
        base64::engine::general_purpose::STANDARD.encode(bytes),
        raster_ms,
        &issues,
    ))
}

/// Build the frame's reading and series maps from the wire snapshot. Values are
/// already scaled and offset by the host: the sidecar applies none of its own.
/// A value no binding names becomes an `UnmappedKey` issue, never a fabricated
/// zero; a bound key with no sample stays absent with a `MissingReading` reason.
fn parse_frame(
    object: &Map<String, Value>,
    document: &ThemeDocument,
) -> Result<(ReadingsById, SeriesById, Vec<PlanIssue>), ProtocolError> {
    let wanted = yunari_render::reading::wanted_keys(document);
    let entries = object
        .get("readings")
        .and_then(Value::as_array)
        .map(|values| values.as_slice())
        .unwrap_or(&[]);

    let mut by_key: ReadingsById = HashMap::new();
    let mut issues: Vec<PlanIssue> = Vec::new();
    for entry in entries {
        let map = entry
            .as_object()
            .ok_or_else(|| fail("A reading must be an object."))?;
        let key = field(map, "semantic_key")?.to_owned();
        if !wanted.contains(&key) {
            issues.push(PlanIssue {
                code: IssueCode::UnmappedKey,
                node_id: None,
                detail: format!("{key} is not bound by this theme."),
            });
            continue;
        }
        let status = match field(map, "status")? {
            "ok" => Status::Ok,
            "missing" => Status::Missing,
            "error" => Status::Error,
            other => return Err(fail(format!("Unknown reading status {other:?}."))),
        };
        let value = map.get("value").and_then(Value::as_f64);
        let value = (status == Status::Ok)
            .then_some(value)
            .flatten()
            .filter(|value| value.is_finite());
        by_key.insert(key, value);
    }

    let mut series: SeriesById = HashMap::new();
    if let Some(objects) = object.get("series").and_then(Value::as_object) {
        for (key, values) in objects {
            if !wanted.contains(key) {
                issues.push(PlanIssue {
                    code: IssueCode::UnmappedKey,
                    node_id: None,
                    detail: format!("{key} is not bound by this theme."),
                });
                continue;
            }
            // The wire sends values in order, not timestamps. A chart's window
            // is measured in seconds; spacing samples one second apart places a
            // rolling window correctly without inventing a wall-clock reading.
            let values = values
                .as_array()
                .ok_or_else(|| fail(format!("Series {key:?} must be an array.")))?
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value
                        .as_f64()
                        .filter(|value| value.is_finite())
                        .map(|value| (index as u64 * 1000, value))
                        .ok_or_else(|| fail(format!("Series {key:?} holds a non-number.")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            series.insert(key.clone(), values);
        }
    }

    for key in &wanted {
        if !by_key.contains_key(key) {
            issues.push(PlanIssue {
                code: IssueCode::MissingReading,
                node_id: None,
                detail: format!("No reading for {key}."),
            });
            by_key.insert(key.clone(), None);
        }
    }
    Ok((by_key, series, issues))
}

fn close(state: &mut Sessions, object: &Map<String, Value>) -> Result<String, ProtocolError> {
    let id = id(object)?;
    match state.by_id.remove(&id) {
        Some(_) => Ok(json!({"ok": true, "id": id, "closed": true}).to_string()),
        None => Err(fail(format!("No open session for {id:?}."))),
    }
}

fn response(
    id: &str,
    width: u32,
    height: u32,
    bytes_base64: String,
    raster_ms: f64,
    issues: &[PlanIssue],
) -> String {
    let issues: Vec<Value> = issues
        .iter()
        .map(|issue| {
            json!({
                "code": format!("{:?}", issue.code),
                "node_id": issue.node_id,
                "detail": issue.detail,
            })
        })
        .collect();
    json!({
        "ok": true,
        "id": id,
        "bytes_base64": bytes_base64,
        "width": width,
        "height": height,
        "raster_ms": raster_ms,
        "issues": issues,
    })
    .to_string()
}
