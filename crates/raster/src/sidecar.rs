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
use serde_json::{Map, Value, json};
use yunari_model::document::ThemeDocument;
use yunari_providers::sample::{Reading, Status};
use yunari_providers::units::UnitPreferences;
use yunari_render::{DisplayList, Fonts, IssueCode, PlanIssue, Readings, Shape};

use crate::{ChartSamples, Images, fit_raster_into, render_with_charts};

type ReadingsById = HashMap<String, Option<f64>>;
type SeriesById = HashMap<String, Vec<(u64, f64)>>;
/// One frame's raw values and their parsed instants, before chart transforms.
type FrameReadings = (
    ReadingsById,
    HashMap<String, i64>,
    Readings,
    SeriesById,
    Vec<PlanIssue>,
);

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
    /// A video device's glass samples one held poster instead of the live
    /// media underneath it.
    glass_sampling: GlassSampling,
    /// Line charts need more than one point. The host supplies its own bounded
    /// series; when it omits one (older hosts or a frame without `series`), the
    /// session keeps the readings that actually arrived, placed by the host's
    /// tick or the reading's own timestamp. No sample, no fabricated point.
    history: SeriesById,
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

/// The host resolves this from the device and the theme before opening.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GlassSampling {
    Static,
    Dynamic,
}

fn glass_sampling_field(object: &Map<String, Value>) -> Result<GlassSampling, ProtocolError> {
    match object.get("glass_sampling").and_then(Value::as_str) {
        None => Ok(GlassSampling::Dynamic),
        Some("static") => Ok(GlassSampling::Static),
        Some("dynamic") => Ok(GlassSampling::Dynamic),
        Some(other) => Err(fail(format!(
            "A request's glass_sampling must be \"static\" or \"dynamic\", not {other:?}."
        ))),
    }
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

/// The panel's own tick, if it states one. A host that omits it keeps the
/// source's cadence; zero or nonsense is refused rather than turning into an
/// infinite decode rate.
fn frame_interval_field(object: &Map<String, Value>) -> Result<Option<u32>, ProtocolError> {
    match object.get("frame_interval_ms") {
        None => Ok(None),
        Some(value) => {
            let interval = value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| fail("A request's frame_interval_ms must be a positive integer."))?;
            Ok(Some(interval))
        }
    }
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

/// The poster lives in the host's disposable media cache, not in the theme
/// directory. The host names both the cache root and the file; the session
/// accepts the file only when its canonical path is inside the canonical root.
/// A path shape alone cannot prove that: any absolute path to a PNG elsewhere
/// on this PC would otherwise be readable by a theme session.
fn poster_field(object: &Map<String, Value>) -> Result<Option<PathBuf>, ProtocolError> {
    let Some(value) = object.get("poster_path").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(root) = object.get("poster_root").and_then(Value::as_str) else {
        return Err(fail("poster_path needs the host's poster_root."));
    };
    let (path, root) = (Path::new(value), Path::new(root));
    if !path.is_absolute() || !root.is_absolute() || path.file_name().is_none() {
        return Err(fail("poster_path and poster_root must be absolute paths."));
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|_| fail("The host's poster_root could not be read."))?;
    let canonical =
        std::fs::canonicalize(path).map_err(|_| fail("The poster could not be read."))?;
    if !canonical.starts_with(&canonical_root) || canonical == canonical_root {
        return Err(fail("poster_path is outside the host's poster_root."));
    }
    Ok(Some(canonical))
}

fn decode_poster(path: &Path) -> Option<crate::Decoded> {
    let bytes = std::fs::read(path).ok()?;
    let picture = image::load_from_memory(&bytes).ok()?.into_rgba8();
    Some(crate::Decoded {
        width: picture.width(),
        height: picture.height(),
        rgba: picture.into_raw(),
    })
}

fn open(state: &mut Sessions, object: &Map<String, Value>) -> Result<String, ProtocolError> {
    let id = id(object)?;
    let theme_json = field(object, "theme_json")?;
    let width = u32_field(object, "width")?;
    let height = u32_field(object, "height")?;
    let rotation = rotation_field(object)?;
    let frame_interval_ms = frame_interval_field(object)?;
    let glass_sampling = glass_sampling_field(object)?;
    let format = match field(object, "format")? {
        "png" => Format::Png,
        "jpeg" => Format::Jpeg,
        other => return Err(fail(format!("Unsupported still format {other:?}."))),
    };

    let document = yunari_model::document::parse_document(theme_json)
        .map_err(|error| fail(format!("Refused theme: {error}")))?;
    let base = opening_dir(object)?;
    let poster = poster_field(object)?;

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
    if let Some(interval) = frame_interval_ms {
        images.set_demand_fps(1000.0 / interval as f64);
    }
    images.set_render_size(width, height);
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
    if glass_sampling == GlassSampling::Static
        && let (Some(poster), Some(asset)) = (
            poster,
            document.background.media.as_ref().and_then(|media| {
                document
                    .assets
                    .iter()
                    .find(|asset| asset.id == media.asset_id)
            }),
        )
        && let Some(decoded) = decode_poster(&poster)
    {
        let size = (decoded.width as f32, decoded.height as f32);
        images.hold(&asset.path, decoded);
        background_sources.insert(asset.path.clone(), size);
    }

    state.by_id.insert(
        id.clone(),
        Session {
            document,
            fonts,
            images,
            background_sources,
            background_issue,
            glass_sampling,
            history: HashMap::new(),
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

    let (readings, times, wire, mut series, mut issues) = parse_frame(object, &session.document)?;
    // The host owns the series when it sends one: its own window and cap rules
    // already apply. Only a host that omits history needs the session's own
    // fallback, recorded from readings that actually arrived and placed by the
    // host's tick. No sample means no point. Overlays reuse the same state and
    // must not append.
    if !overlay {
        let host_series = object.get("series").and_then(Value::as_object);
        for (key, value) in &readings {
            let host_has_key = host_series.is_some_and(|map| map.contains_key(key));
            if host_has_key {
                continue;
            }
            if let Some(value) = value.filter(|value| value.is_finite()) {
                let at = times
                    .get(key)
                    .copied()
                    .or_else(|| {
                        object
                            .get("sent_at")
                            .and_then(Value::as_str)
                            .and_then(parse_timestamp)
                    })
                    .unwrap_or(0);
                let history = session.history.entry(key.clone()).or_default();
                history.push((at as u64, value));
                history.sort_by_key(|(at, _)| *at);
                if history.len() > 600 {
                    history.remove(0);
                }
            }
        }
        for (key, values) in &session.history {
            series.entry(key.clone()).or_insert_with(|| values.clone());
        }
    }
    let (mut list, build_issues) = yunari_render::fabric::build_with_sources(
        &session.document,
        &mut session.fonts,
        &wire,
        &UnitPreferences::default(),
        &session.background_sources,
    );
    issues.extend(build_issues);
    // Static glass holds one poster for the session. A clip playing under it
    // would make the sample vary frame to frame and spend decode the device
    // does not need, so every video source reads as a still here.
    if session.glass_sampling == GlassSampling::Static {
        if let Some(backdrop) = &mut list.backdrop {
            backdrop.video = false;
        }
        for op in &mut list.ops {
            if let Shape::Image { video, .. } = &mut op.shape {
                *video = false;
            }
        }
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
    // Its backdrop stays as the glass sampling source but is hidden from the
    // visible pixels; the scene objects still paint, so the clip shows through.
    if overlay {
        list.backdrop_visible = false;
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
    let fitted = fit_raster_into(
        &source,
        session.width,
        session.height,
        session.rotation,
        false,
    )
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
) -> Result<FrameReadings, ProtocolError> {
    let wanted = yunari_render::reading::wanted_keys(document);
    let entries = object
        .get("readings")
        .and_then(Value::as_array)
        .map(|values| values.as_slice())
        .unwrap_or(&[]);

    let mut by_key: ReadingsById = HashMap::new();
    let mut times: HashMap<String, i64> = HashMap::new();
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
        let timestamp = map
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp);
        let sent_at = object
            .get("sent_at")
            .and_then(Value::as_str)
            .and_then(parse_timestamp);
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
        by_key.insert(key.clone(), value);
        if value.is_some() {
            let at = timestamp.or(sent_at).unwrap_or_default();
            times.insert(key, at);
        }
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
            // The host's series carries each point's own instant, so a 10 s
            // source stays 10 s apart on the chart. A bare number is accepted
            // for older callers and spaced by the panel tick; a malformed
            // point is refused rather than turned into a fabricated time.
            let values = values
                .as_array()
                .ok_or_else(|| fail(format!("Series {key:?} must be an array.")))?
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let (at, value) = if let Some(number) = value.as_f64() {
                        (index as u64 * 1000, number)
                    } else {
                        let object = value
                            .as_object()
                            .ok_or_else(|| fail(format!("Series {key:?} holds a non-number.")))?;
                        let value = object
                            .get("value")
                            .and_then(Value::as_f64)
                            .filter(|value| value.is_finite())
                            .ok_or_else(|| fail(format!("Series {key:?} holds a non-number.")))?;
                        let at = object
                            .get("at")
                            .and_then(Value::as_str)
                            .and_then(parse_timestamp)
                            .ok_or_else(|| {
                                fail(format!("Series {key:?} point has no usable time."))
                            })?;
                        (at as u64, value)
                    };
                    value
                        .is_finite()
                        .then_some((at, value))
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
    Ok((by_key, times, Readings::new(wire), series, issues))
}

/// Parse the ISO-8601 instants the host sends. A malformed time is not an
/// excuse to invent a wall-clock reading: the frame's `sent_at` is the next
/// honest fallback. A value with neither is placed at the epoch so the series
/// still records that something arrived, at the oldest possible position.
fn parse_timestamp(text: &str) -> Option<i64> {
    // `jiff` is already a workspace dependency and parses the host's ISO-8601
    // form without pulling another crate into this sidecar.
    use jiff::Timestamp;
    text.parse::<Timestamp>().ok().map(|at| at.as_millisecond())
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
