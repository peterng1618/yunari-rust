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
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use base64::Engine as _;
use serde_json::{json, Map, Value};
use yunari_model::document::ThemeDocument;
use yunari_providers::sample::{Reading, Status};
use yunari_providers::units::UnitPreferences;
use yunari_render::{DisplayList, Fonts, IssueCode, PlanIssue, Readings, Shape};

use crate::{fit_raster_into, render_with_charts, ChartSamples, Images};

type ReadingsById = HashMap<String, Option<f64>>;
type SeriesById = HashMap<String, Vec<(u64, f64)>>;

/// A theme and the caches that make its frames cheap to rebuild.
struct Session {
    document: ThemeDocument,
    fonts: Fonts,
    images: Images,
    /// Decoded sizes of the theme's background pictures, keyed by asset path.
    /// Measured at open so the planner can place the page picture before the
    /// first frame is painted.
    background_sources: HashMap<String, (f32, f32)>,
    /// Why the declared background picture could not be measured, if so.
    background_issue: Option<String>,
    width: u32,
    height: u32,
    rotation: u16,
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

/// A display's quarter turn, defaulting to zero so hosts that predate the
/// field keep the same frames. An out-of-range value is refused at the trust
/// boundary rather than truncated into a different rotation.
fn rotation_field(object: &Map<String, Value>) -> Result<u16, ProtocolError> {
    let Some(value) = object.get("rotation") else {
        return Ok(0);
    };
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| fail("A request's rotation must be an unsigned 16-bit integer."))
}

fn opening_dir(object: &Map<String, Value>) -> Result<Option<PathBuf>, ProtocolError> {
    let Some(dir) = object.get("theme_dir").and_then(Value::as_str) else {
        return Ok(None);
    };
    let path = Path::new(dir);
    if !path.is_absolute() {
        return Err(fail("theme_dir must be an absolute path."));
    }
    Ok(Some(path.to_path_buf()))
}

/// A resolved asset path must still live under the theme's own directory. The
/// model already refuses literal `..` segments; this guards the join at the
/// trust boundary against a link inside the theme that points back out, so a
/// session can never read bytes the host did not hand it.
fn safe_asset_path(base: Option<&Path>, asset: &str) -> Option<PathBuf> {
    let base = base?;
    let resolved = base.join(asset);
    let canonical_base = std::fs::canonicalize(base).ok()?;
    let canonical = std::fs::canonicalize(&resolved).ok()?;
    canonical
        .strip_prefix(canonical_base)
        .ok()
        .filter(|rest| {
            !rest.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        })
        .map(|_| resolved)
}

fn open(state: &mut Sessions, object: &Map<String, Value>) -> Result<String, ProtocolError> {
    let id = id(object)?;
    let theme_json = field(object, "theme_json")?;
    let width = u32_field(object, "width")?;
    let height = u32_field(object, "height")?;
    let rotation = rotation_field(object)?;
    let format = match field(object, "format")? {
        "png" => Format::Png,
        "jpeg" => Format::Jpeg,
        other => return Err(fail(format!("Unsupported still format {other:?}."))),
    };

    let document = yunari_model::document::parse_document(theme_json)
        .map_err(|error| fail(format!("Refused theme: {error}")))?;
    let base = opening_dir(object)?;

    // Refuse any asset that resolves outside the theme dir before a frame is
    // ever painted: a portable theme must not be able to read its way out.
    if let Some(dir) = base.as_deref() {
        for asset in &document.assets {
            if safe_asset_path(Some(dir), &asset.path).is_none() {
                return Err(fail(format!(
                    "Asset {:?} escapes the theme directory.",
                    asset.path
                )));
            }
        }
    }

    let mut fonts = Fonts::new();
    // Theme faces are read from the theme's own directory. The host names that
    // directory; every other path stays closed to a session.
    fonts.load_faces(&document, |asset| {
        safe_asset_path(base.as_deref(), asset).and_then(|path| std::fs::read(path).ok())
    });
    let mut images = match base.as_deref() {
        // Images::at resolves paths against `base`; the guard above proved each
        // declared asset stays inside it.
        Some(dir) => Images::at(dir),
        None => Images::empty(),
    };
    // Measuring the background asset is the only way to place it: the
    // composition's area and ramps are known from the theme, but its crop needs
    // the source's own size. Unreadable bytes are reported at `open`, never
    // replaced by a guessed size or a fabricated picture.
    let mut background_sources = HashMap::new();
    let mut background_issue = None;
    if let (Some(media), Some(_)) = (document.background.media.as_ref(), base.as_deref())
        && let Some(asset) = document
            .assets
            .iter()
            .find(|asset| asset.id == media.asset_id)
    {
        match images.load(&asset.path) {
            Some(decoded) => {
                background_sources.insert(
                    asset.path.clone(),
                    (decoded.width as f32, decoded.height as f32),
                );
            }
            None => {
                background_issue = Some(format!(
                    "Background picture {:?} could not be read; only the fill paints.",
                    asset.path
                ));
            }
        }
    }
    state.by_id.insert(
        id.clone(),
        Session {
            document,
            fonts,
            images,
            background_sources,
            background_issue,
            width,
            height,
            rotation,
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

    let (readings, wire, series, mut issues) = parse_frame(object, &session.document)?;
    let (mut list, build_issues) = yunari_render::fabric::build_with_sources(
        &session.document,
        &mut session.fonts,
        &wire,
        &UnitPreferences::default(),
        &session.background_sources,
    );
    issues.extend(build_issues);
    // The planner resolves the backdrop in artboard units, but the final fit
    // turns the whole frame afterwards. An author's `top` band belongs to the
    // panel, so resolve it against the panel box first and map the area, crop
    // and ramps back through the fit's inverse. The scene itself stays in
    // artboard coordinates, so text and charts still turn with the panel.
    if let Some(backdrop) = list.backdrop.as_mut()
        && session.rotation != 0
    {
        let panel_composition = yunari_render::background::resolve(
            (session.width as f32, session.height as f32),
            backdrop.composition.as_ref(),
            session.background_sources.get(&backdrop.path).copied(),
            backdrop.geometry,
        );
        backdrop.resolved = yunari_render::background::from_panel(
            panel_composition,
            (list.width, list.height),
            (session.width, session.height),
            session.rotation,
        );
    }
    // The refusal is a fact about the theme, not a per-frame sensor gap:
    // report it on the first frame only, so a running panel does not flood the
    // host's notice surface at display rate.
    if let Some(detail) = session.background_issue.take() {
        issues.push(PlanIssue {
            code: IssueCode::UnreadablePaint,
            node_id: None,
            detail,
        });
    }

    // The overlay is the UI alone, composited by the device over its own clip.
    // Hiding the board's paint and its backdrop is the whole difference; the
    // scene's own objects still paint, with alpha, so the clip shows through.
    if overlay {
        list.background = None;
        list.backdrop = None;
    }

    // Charts read the same binding scale/offset as text. `fabric::build`
    // applies it to value runs itself; the chart backend must see its own
    // object's transform, not the first binding that shares a semantic key.
    let samples_by_key = transform_samples(&session.document, &mut list, &readings, &series);
    let samples = ChartSamples {
        readings: &samples_by_key.0,
        series: &samples_by_key.1,
    };
    let started = Instant::now();
    let (source, chart_issues) = render_with_charts(
        &list,
        1.0,
        Some(&mut session.fonts),
        Some(&mut session.images),
        Some(&session.document),
        &samples,
    );
    issues.extend(chart_issues);
    let raster_ms = started.elapsed().as_secs_f64() * 1000.0;

    // The device consumes a frame at its own geometry. Fit the painted artboard
    // into it; the frame we report is the frame we encoded.
    let fitted = fit_raster_into(&source, session.width, session.height, session.rotation, false)
        .map_err(|error| fail(format!("Could not fit the frame: {error}")))?;
    let picture = image::RgbaImage::from_raw(fitted.width, fitted.height, fitted.rgba)
        .ok_or_else(|| fail("The frame does not fill the panel."))?;
    let mut bytes = Vec::new();
    // An overlay must keep its alpha: a JPEG has none, so a video panel would
    // composite a black rectangle. The session's format applies only to stills.
    let format = if overlay { Format::Png } else { session.format };
    match format {
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
        fitted.width,
        fitted.height,
        base64::engine::general_purpose::STANDARD.encode(bytes),
        raster_ms,
        &issues,
    ))
}

/// Build the frame's reading and series maps from the wire snapshot. Values are
/// raw measurements; the document's bindings own scale/offset (applied to text
/// by `fabric::build` and to chart samples by `transform_samples`). A value no
/// binding names becomes an `UnmappedKey` issue, never a fabricated zero.
fn parse_frame(
    object: &Map<String, Value>,
    document: &ThemeDocument,
) -> Result<(ReadingsById, Readings, SeriesById, Vec<PlanIssue>), ProtocolError> {
    let wanted = yunari_render::reading::wanted_keys(document);
    let entries = object
        .get("readings")
        .and_then(Value::as_array)
        .map(|values| values.as_slice())
        .unwrap_or(&[]);

    let mut by_key: ReadingsById = HashMap::new();
    let mut wire: Vec<Reading> = Vec::new();
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
        let reason = map
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned);
        wire.push(Reading {
            semantic_key: key.clone(),
            status,
            value,
            unit: map.get("unit").and_then(Value::as_str).map(str::to_owned),
            text: map.get("text").and_then(Value::as_str).map(str::to_owned),
            instant: map
                .get("instant")
                .and_then(Value::as_str)
                .map(str::to_owned),
            message: reason,
        });
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
            wire.push(Reading::missing(key, "no reading this frame"));
            by_key.insert(key.clone(), None);
        }
    }
    Ok((by_key, Readings::new(wire), series, issues))
}

/// Apply each chart object's own binding transform to the samples the chart
/// backend consumes. `fabric::build` already transforms text value runs using
/// the binding owned by each run's object; a global lookup by semantic key
/// would give every chart the first owner's scale/offset and disagree with the
/// text on the same key. The chart op carries its object id, so its bindings
/// are looked up under that id and its sample keys are made unique per owner.
fn transform_samples(
    document: &ThemeDocument,
    list: &mut DisplayList,
    readings: &ReadingsById,
    series: &SeriesById,
) -> (ReadingsById, SeriesById) {
    let mut scaled_readings: ReadingsById = HashMap::new();
    let mut scaled_series: SeriesById = HashMap::new();

    for op in &mut list.ops {
        let Shape::Chart { id, bindings, .. } = &mut op.shape else {
            continue;
        };
        let owners = document.bindings.get(id);
        for key in bindings.iter_mut() {
            let binding = owners
                .and_then(|bindings| bindings.iter().find(|binding| binding.semantic_key == *key));
            let (scale, offset) = binding
                .map(|binding| (binding.scale.unwrap_or(1.0), binding.offset.unwrap_or(0.0)))
                .unwrap_or((1.0, 0.0));
            let transform = |value: f64| value * scale + offset;
            // A key unique to this owner lets two charts on the same semantic
            // key carry different transforms without one overwriting the other.
            let unique = format!("{id}\u{0}{key}");
            if let Some(value) = readings.get(key).copied().flatten() {
                scaled_readings.insert(unique.clone(), Some(transform(value)));
            } else {
                scaled_readings.insert(unique.clone(), None);
            }
            if let Some(values) = series.get(key) {
                scaled_series.insert(
                    unique.clone(),
                    values
                        .iter()
                        .map(|(at, value)| (*at, transform(*value)))
                        .collect(),
                );
            }
            *key = unique;
        }
    }
    (scaled_readings, scaled_series)
}

/// The renderer-core issue vocabulary is kebab-case; a Rust debug name is not
/// a wire value the host can switch on.
fn issue_code(code: IssueCode) -> &'static str {
    match code {
        IssueCode::UnmappedKey => "unmapped-key",
        IssueCode::UnresolvedGlobal => "unresolved-global",
        IssueCode::UnsupportedUnit => "unsupported-unit",
        IssueCode::MissingReading => "missing-reading",
        IssueCode::UnreadablePaint => "unreadable-paint",
        IssueCode::UnsupportedContent => "unsupported-content",
    }
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
                "code": issue_code(issue.code),
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
