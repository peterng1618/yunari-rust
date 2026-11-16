//! One library theme as the bytes a store submission sends.
//!
//! The Worker builds the item JSON and computes the authoritative version; this
//! only does what needs the local library: read the document, run the ladder
//! against the authenticated baseline, write the version into the document,
//! build the package and its picture, upload, and finalize.
//!
//! The library write that records the version happens only after the store
//! accepted it, so a refused submission never leaves the library ahead of the
//! store.

use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use yunari_store::{ClientError, CreateInput};

use crate::http::Shared;

pub const NOT_JSON_MESSAGE: &str = "That submission request is not JSON.";
pub const NO_ID_MESSAGE: &str = "A submission needs the id of a library theme.";
pub const NO_STORE_MESSAGE: &str = "This host has no store to submit to.";
pub const NO_SESSION_MESSAGE: &str = "Sign in to the store before submitting.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Initial,
    None,
    Patch,
    Minor,
    Major,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Initial => "initial",
            Level::None => "none",
            Level::Patch => "patch",
            Level::Minor => "minor",
            Level::Major => "major",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Level::Initial | Level::None => 0,
            Level::Patch => 1,
            Level::Minor => 2,
            Level::Major => 3,
        }
    }
}

fn higher(left: Level, right: Level) -> Level {
    if right.rank() > left.rank() {
        right
    } else {
        left
    }
}

/// Three integers, with no prefix, prerelease or build.
pub fn is_semantic_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts.iter().all(|part| {
            part.parse::<u64>()
                .is_ok_and(|value| value <= 9_007_199_254_740_991)
        })
}

/// The next version by `level`, or nothing when `previous` is not a version.
pub fn bump(previous: &str, level: Level) -> Option<String> {
    if !is_semantic_version(previous) {
        return None;
    }
    let parts: Vec<u64> = previous
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    Some(match level {
        Level::Major => format!("{}.0.0", parts[0] + 1),
        Level::Minor => format!("{}.{}.0", parts[0], parts[1] + 1),
        _ => format!("{}.{}.{}", parts[0], parts[1], parts[2] + 1),
    })
}

/// The version a submission carries.
pub fn version_for(previous: Option<&str>, level: Level) -> Option<String> {
    match level {
        Level::Initial => previous.is_none().then(|| "1.0.0".to_owned()),
        Level::None => previous
            .filter(|previous| is_semantic_version(previous))
            .map(str::to_owned),
        Level::Patch | Level::Minor | Level::Major => {
            previous.and_then(|previous| bump(previous, level))
        }
    }
}

const IGNORED_TOP: [&str; 2] = ["editorMetadata", "id"];
const IGNORED_METADATA: [&str; 3] = ["version", "createdAt", "updatedAt"];

/// How a stored package and the library's document differ.
pub fn classify(previous: Option<&Value>, next: &Value) -> Level {
    let Some(previous) = previous else {
        return Level::Initial;
    };
    let (Some(before), Some(after)) = (previous.as_object(), next.as_object()) else {
        return Level::Major;
    };
    let mut level = Level::None;
    for key in before.keys().chain(after.keys()) {
        if IGNORED_TOP.contains(&key.as_str()) {
            continue;
        }
        let change = match key.as_str() {
            "schemaVersion" => (before.get(key) != after.get(key)).then_some(Level::Major),
            "artboard" => artboard_change(before.get(key), after.get(key)),
            "background" => background_change(before.get(key), after.get(key)),
            "metadata" => metadata_change(before.get(key), after.get(key)),
            "palette" | "typeTokens" => token_group_change(before.get(key), after.get(key)),
            "bindings" => bindings_change(before.get(key), after.get(key)),
            "assets" => asset_change(before.get(key), after.get(key)),
            "nodes" => node_list_change(before.get(key), after.get(key)),
            // A member this build does not know is a format change.
            _ => Some(match (before.get(key), after.get(key)) {
                (None, None) => Level::None,
                (Some(left), Some(right)) if left == right => Level::None,
                (None, Some(_)) | (Some(_), None) => Level::Major,
                (Some(left), Some(right)) => diff(left, right, false),
            }),
        };
        if let Some(change) = change {
            level = higher(level, change);
        }
    }
    level
}

fn artboard_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    let (Some(before), Some(after)) = (before, after) else {
        return Some(Level::Patch);
    };
    if before == after {
        return None;
    }
    let canvas = ["width", "height", "shape"]
        .iter()
        .any(|key| before.get(key) != after.get(key));
    Some(if canvas { Level::Major } else { Level::Patch })
}

fn background_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    let (Some(before), Some(after)) = (before, after) else {
        return Some(Level::Patch);
    };
    if before == after {
        return None;
    }
    Some(if before.get("media") != after.get("media") {
        Level::Minor
    } else {
        Level::Patch
    })
}

fn metadata_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    let without = |value: Option<&Value>| -> Value {
        let Some(object) = value.and_then(Value::as_object) else {
            return Value::Object(Default::default());
        };
        Value::Object(
            object
                .iter()
                .filter(|(key, _)| !IGNORED_METADATA.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    };
    let before = without(before);
    let after = without(after);
    if before == after {
        return None;
    }
    // Tags are a set: reordering them is no change, joining or leaving is minor.
    let tags = |value: &Value| -> BTreeSet<String> {
        value
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let membership = tags(&before) != tags(&after);
    Some(if membership {
        Level::Minor
    } else {
        Level::Patch
    })
}

fn token_group_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    let keys = |value: Option<&Value>| -> BTreeSet<String> {
        value
            .and_then(Value::as_object)
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default()
    };
    let before_keys = keys(before);
    let after_keys = keys(after);
    if before == after {
        return None;
    }
    Some(if before_keys != after_keys {
        Level::Minor
    } else {
        Level::Patch
    })
}

fn bindings_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    if before == after {
        return None;
    }
    let sensors = |value: Option<&Value>| -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let Some(object) = value.and_then(Value::as_object) else {
            return found;
        };
        for list in object.values() {
            for binding in list.as_array().into_iter().flatten() {
                if let Some(key) = binding.get("semanticKey").and_then(Value::as_str) {
                    found.insert(key.to_owned());
                }
            }
        }
        found
    };
    let before_sensors = sensors(before);
    let after_sensors = sensors(after);
    let removed = before_sensors
        .iter()
        .any(|key| !after_sensors.contains(key));
    let added = after_sensors
        .iter()
        .any(|key| !before_sensors.contains(key));
    Some(if removed {
        Level::Major
    } else if added {
        Level::Minor
    } else {
        Level::Patch
    })
}

fn asset_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    if before == after {
        return None;
    }
    let ids = |value: Option<&Value>| -> BTreeSet<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect()
    };
    Some(if ids(before) != ids(after) {
        Level::Minor
    } else {
        Level::Patch
    })
}

/// Nodes are matched by id, at any depth, so a re-parent keeps the same
/// identity; entering or leaving the document is structure.
fn node_list_change(before: Option<&Value>, after: Option<&Value>) -> Option<Level> {
    if before == after {
        return None;
    }
    collect_nodes(before)
        .key_compare(&collect_nodes(after))
        .or(Some(Level::Patch))
}

#[derive(Default)]
struct Nodes {
    by_id: BTreeMap<String, (Option<String>, Value)>,
    order: Vec<String>,
}

fn collect_nodes(value: Option<&Value>) -> Nodes {
    let mut nodes = Nodes::default();
    fn walk(value: &Value, parent: Option<&str>, nodes: &mut Nodes) {
        let Some(list) = value.as_array() else {
            return;
        };
        for entry in list {
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            nodes
                .by_id
                .insert(id.to_owned(), (parent.map(str::to_owned), entry.clone()));
            nodes.order.push(id.to_owned());
            if let Some(children) = entry.get("children") {
                walk(children, Some(id), nodes);
            }
        }
    }
    if let Some(value) = value {
        walk(value, None, &mut nodes);
    }
    nodes
}

impl Nodes {
    fn key_compare(&self, after: &Nodes) -> Option<Level> {
        if self.by_id.keys().ne(after.by_id.keys()) {
            return Some(Level::Minor);
        }
        let mut level = Level::None;
        for (id, (parent, value)) in &self.by_id {
            let Some((next_parent, next)) = after.by_id.get(id) else {
                continue;
            };
            if parent != next_parent {
                level = higher(level, Level::Patch);
            }
            if value != next {
                // Children are compared as their own identities, so only this
                // node's other members are a content change.
                let mut left = value.clone();
                let mut right = next.clone();
                if let (Some(left), Some(right)) = (left.as_object_mut(), right.as_object_mut()) {
                    left.remove("children");
                    right.remove("children");
                }
                if left != right {
                    level = higher(level, Level::Patch);
                }
            }
        }
        // Z-order among the ids that stayed in this document.
        let before_order: Vec<&String> = self
            .order
            .iter()
            .filter(|id| after.by_id.contains_key(*id))
            .collect();
        let after_order: Vec<&String> = after
            .order
            .iter()
            .filter(|id| self.by_id.contains_key(*id))
            .collect();
        if before_order != after_order {
            level = higher(level, Level::Patch);
        }
        (level != Level::None).then_some(level)
    }
}

/// A generic comparison for a member this build does not rule over.
fn diff(before: &Value, after: &Value, order_matters: bool) -> Level {
    if before == after {
        return Level::None;
    }
    match (before, after) {
        (Value::Array(left), Value::Array(right)) => {
            let identified = left
                .iter()
                .chain(right)
                .all(|entry| entry.get("id").and_then(Value::as_str).is_some());
            if identified {
                let ids = |list: &Vec<Value>| -> BTreeSet<String> {
                    list.iter()
                        .filter_map(|entry| {
                            entry.get("id").and_then(Value::as_str).map(str::to_owned)
                        })
                        .collect()
                };
                if ids(left) != ids(right) {
                    return Level::Minor;
                }
                if order_matters && left != right {
                    return Level::Patch;
                }
                let mut level = Level::None;
                for (index, entry) in left.iter().enumerate() {
                    if let Some(next) = right.get(index) {
                        level = higher(level, diff(entry, next, false));
                    }
                }
                level
            } else {
                Level::Patch
            }
        }
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
            let mut level = Level::None;
            for key in keys {
                match (left.get(key), right.get(key)) {
                    (Some(a), Some(b)) => level = higher(level, diff(a, b, false)),
                    _ => level = higher(level, Level::Patch),
                }
            }
            level
        }
        _ => Level::Patch,
    }
}

/// One submission's answer, as the Store surface draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submitted {
    pub status: String,
    pub from: Option<String>,
    pub to: String,
    pub level: Level,
}

/// The refusals a submission can end in, and the HTTP status each earns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub message: String,
}

fn refuse(status: u16, message: impl Into<String>) -> Refusal {
    Refusal {
        status,
        message: message.into(),
    }
}

fn client_refusal(app: &Shared, error: ClientError) -> Refusal {
    if error.status == 401 {
        let _ = app.account.sessions.clear();
    }
    let status = if error.status == 401 || error.status == 403 {
        401
    } else if (400..500).contains(&error.status) {
        400
    } else {
        502
    };
    refuse(status, error.message)
}

/// Reads the library theme, builds the package and puts it through the Worker's
/// authenticated upload, finalizing with the picture.
pub fn submit(app: &Shared, theme_id: &str) -> Result<Submitted, Refusal> {
    if !yunari_model::validate::is_stable_id(theme_id) {
        return Err(refuse(
            400,
            format!("\"{theme_id}\" is not a theme id this library can hold."),
        ));
    }
    let Some(client) = app.account.client() else {
        return Err(refuse(401, NO_SESSION_MESSAGE));
    };
    let theme = app.library.read(theme_id).map_err(|_| {
        refuse(
            400,
            format!("This PC's library could not read theme \"{theme_id}\"."),
        )
    })?;
    let assets = app.library.assets(theme_id, &theme.document);
    let baseline = client
        .baseline(theme_id)
        .map_err(|error| client_refusal(app, error))?;
    let previous = baseline.as_ref().map(|baseline| &baseline.envelope);
    let next =
        serde_json::to_value(&theme.document).map_err(|error| refuse(400, error.to_string()))?;
    let level = classify(previous, &next);
    if let (Some(baseline), Level::None) = (&baseline, level) {
        return Err(refuse(
            400,
            format!(
                "Version {} is published and this theme has not changed since it.",
                baseline.version
            ),
        ));
    }
    let Some(version) = version_for(baseline.as_ref().map(|b| b.version.as_str()), level) else {
        return Err(refuse(
            400,
            match baseline {
                Some(baseline) => format!(
                    "The store's copy of this theme records version \"{}\", which cannot be bumped.",
                    baseline.version
                ),
                None => "This theme could not be given a version.".to_owned(),
            },
        ));
    };

    let mut document = theme.document.clone();
    document.metadata.version = Some(version.clone());
    let (thumbnail, luminance) =
        crate::thumbnail::of(&document, &assets).map_err(|message| refuse(400, message))?;
    let package = yunari_library::package::write_package(&document, &assets, Some(&thumbnail))
        .map_err(|message| refuse(400, message))?;
    let envelope =
        serde_json::to_value(&document).map_err(|error| refuse(400, error.to_string()))?;
    let plan = client
        .create_version(
            theme_id,
            &CreateInput {
                envelope,
                name: document
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| theme.entry.name.clone()),
                credits: None,
            },
        )
        .map_err(|error| client_refusal(app, error))?;

    // The store accepted this version, so the library may record it. A stale
    // base is the library's own sentence; another refusal names no path.
    if let Err(error) = app.library.save(&yunari_library::SaveRequest {
        id: theme_id,
        document: &document,
        assets: &assets,
        base: Some(&theme.base),
        overwrite: false,
    }) {
        return Err(refuse(
            400,
            match error {
                yunari_library::LibraryError::Conflict { .. } => {
                    "This theme changed in this PC's library while it was being submitted."
                        .to_owned()
                }
                _ => "This theme's new version could not be saved in this PC's library.".to_owned(),
            },
        ));
    }

    client
        .upload(&plan.upload_url, &package)
        .map_err(|error| client_refusal(app, error))?;
    let finalized = client
        .finalize(theme_id, &plan.version, &thumbnail, Some(luminance))
        .map_err(|error| client_refusal(app, error))?;
    Ok(Submitted {
        status: finalized.status,
        from: baseline.map(|baseline| baseline.version),
        to: plan.version,
        level,
    })
}

/// `POST /api/themes/submit`, body `{"id": "<library theme>"}`.
pub async fn route(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    body: axum::body::Bytes,
) -> Response {
    if !crate::http::is_loopback(peer.ip()) {
        return (
            StatusCode::FORBIDDEN,
            crate::store_routes::NOT_LOCAL_THEME_MESSAGE,
        )
            .into_response();
    }
    if app.account.origin.is_none() {
        return (StatusCode::SERVICE_UNAVAILABLE, NO_STORE_MESSAGE).into_response();
    }
    if app.account.sessions.read().is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "message": NO_SESSION_MESSAGE })),
        )
            .into_response();
    }
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, NOT_JSON_MESSAGE).into_response(),
    };
    let Some(id) = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
    else {
        return (StatusCode::BAD_REQUEST, NO_ID_MESSAGE).into_response();
    };
    let app = std::sync::Arc::clone(&app);
    let outcome = tokio::task::spawn_blocking(move || submit(&app, &id))
        .await
        .unwrap_or_else(|error| Err(refuse(500, error.to_string())));
    match outcome {
        Ok(submitted) => Json(json!({
            "ok": true,
            "status": submitted.status,
            "version": {
                "from": submitted.from,
                "to": submitted.to,
                "level": submitted.level.as_str(),
            },
        }))
        .into_response(),
        Err(refusal) => {
            if refusal.status == 401 {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "ok": false, "message": refusal.message })),
                )
                    .into_response();
            }
            let status = StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::BAD_GATEWAY);
            (
                status,
                Json(json!({ "ok": false, "message": refusal.message })),
            )
                .into_response()
        }
    }
}

/// The package's own theme document, for the version ladder's tests.
#[cfg(test)]
fn sample_document() -> yunari_model::document::ThemeDocument {
    crate::store_install::test_document("aurora")
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_model::document::ThemeDocument;

    fn value(document: &ThemeDocument) -> Value {
        serde_json::to_value(document).unwrap()
    }

    #[test]
    fn a_first_submission_is_initial_and_mints_one_point_oh_oh() {
        let document = sample_document();
        assert_eq!(classify(None, &value(&document)), Level::Initial);
        assert_eq!(version_for(None, Level::Initial).as_deref(), Some("1.0.0"));
        // A previous alongside `initial` is a contradiction.
        assert_eq!(version_for(Some("1.0.0"), Level::Initial), None);
    }

    #[test]
    fn an_unchanged_document_is_none_and_keeps_its_version() {
        let document = sample_document();
        let baseline = value(&document);
        assert_eq!(classify(Some(&baseline), &baseline), Level::None);
        assert_eq!(
            version_for(Some("1.4.0"), Level::None).as_deref(),
            Some("1.4.0")
        );
    }

    #[test]
    fn the_ignored_members_move_without_a_release() {
        let document = sample_document();
        let mut next = document.clone();
        next.editor_metadata
            .insert("zoom".to_owned(), serde_json::json!(2));
        next.metadata.version = Some("9.9.9".to_owned());
        assert_eq!(
            classify(Some(&value(&document)), &value(&next)),
            Level::None
        );
    }

    #[test]
    fn canvas_schema_and_sensor_loss_are_major_changes() {
        let document = sample_document();
        let baseline = value(&document);

        let mut next = document.clone();
        next.artboard.width = 2560.0;
        assert_eq!(classify(Some(&baseline), &value(&next)), Level::Major);

        let mut next = document.clone();
        next.schema_version = "2.0.0".to_owned();
        assert_eq!(classify(Some(&baseline), &value(&next)), Level::Major);

        // Losing the only sensor in a binding map is major.
        let binding = yunari_model::document::Binding {
            id: "clock-value".to_owned(),
            semantic_key: "time.now".to_owned(),
            precision: None,
            unit_display: None,
            scale: None,
            offset: None,
            unit: None,
            auto_scale: None,
            format: None,
            time_zone: None,
        };
        let mut bound = document.clone();
        bound
            .bindings
            .insert("clock".to_owned(), vec![binding.clone()]);
        let before = value(&bound);
        let mut after = bound.clone();
        after.bindings.clear();
        assert_eq!(classify(Some(&before), &value(&after)), Level::Major);

        // Gaining one is minor; editing a kept binding's precision is a patch.
        assert_eq!(classify(Some(&value(&after)), &before), Level::Minor);
        let mut edited = bound.clone();
        edited.bindings.get_mut("clock").unwrap()[0].precision = Some(2);
        assert_eq!(classify(Some(&before), &value(&edited)), Level::Patch);
        // A second sensor joining a bound document is minor.
        let mut added = bound.clone();
        added.bindings.insert(
            "cpu".to_owned(),
            vec![{
                let mut extra = binding;
                extra.id = "cpu-value".to_owned();
                extra.semantic_key = "cpu.load".to_owned();
                extra
            }],
        );
        assert_eq!(classify(Some(&before), &value(&added)), Level::Minor);
    }

    #[test]
    fn structural_and_content_changes_land_on_different_rungs() {
        let document = sample_document();
        let baseline = value(&document);

        // A metadata-only change is a patch.
        let mut next = document.clone();
        next.metadata.description = Some("new".to_owned());
        assert_eq!(classify(Some(&baseline), &value(&next)), Level::Patch);

        // A tag joining is minor.
        let mut next = document.clone();
        next.metadata.tags = vec!["dark".to_owned()];
        assert_eq!(classify(Some(&baseline), &value(&next)), Level::Minor);

        // A palette token joining is minor; recolouring one is a patch.
        let mut next = document.clone();
        next.palette.insert(
            "accent".to_owned(),
            yunari_model::document::PaletteEntry {
                name: "Accent".to_owned(),
                value: yunari_model::document::Paint::Solid {
                    color: "#ff0000".to_owned(),
                },
            },
        );
        assert_eq!(classify(Some(&baseline), &value(&next)), Level::Minor);

        let mut next = document.clone();
        if let Some(entry) = next.palette.get_mut("chartTrack") {
            entry.name = "Track".to_owned();
        }
        assert_eq!(classify(Some(&baseline), &value(&next)), Level::Patch);
    }

    #[test]
    fn a_version_is_three_safe_integers_and_the_bump_keeps_them() {
        assert!(is_semantic_version("0.0.0"));
        assert!(is_semantic_version("10.20.30"));
        assert!(!is_semantic_version("v1.4.0"));
        assert!(!is_semantic_version("1.4.0-beta.1"));
        assert_eq!(bump("1.4.0", Level::Patch).as_deref(), Some("1.4.1"));
        assert_eq!(bump("1.9.9", Level::Minor).as_deref(), Some("1.10.0"));
        assert_eq!(bump("9.9.9", Level::Major).as_deref(), Some("10.0.0"));
        assert_eq!(bump("v1.4", Level::Patch), None);
        assert_eq!(bump("9007199254740992.0.0", Level::Patch), None);
    }
}
