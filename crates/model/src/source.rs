//! Reading a theme envelope written by the TypeScript editor.
//!
//! The source app stores `{schemaVersion, fabricVersion, globals: {palette,
//! typeTokens}, scene: {version, objects}}` and each object is a flat Fabric
//! record (`type`, `left`, `top`, `yunariPaint`, …). The palette, type tokens,
//! assets, bindings and chart settings there are already this model's own
//! shape; what differs is the envelope and the way a scene object is folded
//! into a node. Lifting happens here, once, on the way in — after that a theme
//! lives in one shape only.

use serde_json::{Map, Value, json};

use crate::document::{SCHEMA_VERSION, ThemeDocument};

/// Whether this JSON is the source app's envelope rather than this model's.
pub fn looks_like_source(value: &Value) -> bool {
    value.get("scene").is_some() || value.get("globals").is_some()
}

pub fn from_source_str(text: &str) -> Result<ThemeDocument, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    from_source_value(value)
}

/// Lift one stored envelope into a document. An object kind this port does not
/// draw is dropped rather than failing the theme: one unknown widget must not
/// cost the author their whole scene.
pub fn from_source_value(value: Value) -> Result<ThemeDocument, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "a theme envelope is an object".to_owned())?;
    let mut out = Map::new();
    out.insert("schemaVersion".to_owned(), json!(SCHEMA_VERSION));
    out.insert(
        "id".to_owned(),
        json!(field(object, "id").unwrap_or_default()),
    );
    if let Some(metadata) = object.get("metadata").and_then(Value::as_object) {
        out.insert("metadata".to_owned(), pick(metadata, METADATA_KEYS));
    }
    if let Some(artboard) = object.get("artboard").and_then(Value::as_object) {
        out.insert("artboard".to_owned(), pick(artboard, ARTBOARD_KEYS));
    }
    if let Some(background) = object.get("background").and_then(Value::as_object) {
        let mut lifted = pick(background, BACKGROUND_KEYS);
        if let Some(media) = background.get("media").and_then(Value::as_object) {
            lifted["media"] = pick(media, BACKGROUND_MEDIA_KEYS);
        }
        out.insert("background".to_owned(), lifted);
    }

    let globals = object.get("globals").and_then(Value::as_object);
    let table = |name: &str| {
        globals
            .and_then(|globals| globals.get(name))
            .or_else(|| object.get(name))
            .and_then(Value::as_object)
    };
    if let Some(palette) = table("palette") {
        let mut lifted = Map::new();
        for (id, entry) in palette {
            if let Some(entry) = entry.as_object() {
                lifted.insert(id.clone(), pick(entry, PALETTE_KEYS));
            }
        }
        out.insert("palette".to_owned(), Value::Object(lifted));
    }
    if let Some(tokens) = table("typeTokens") {
        let mut lifted = Map::new();
        for (id, token) in tokens {
            if let Some(token) = token.as_object() {
                lifted.insert(id.clone(), pick(token, TYPE_TOKEN_KEYS));
            }
        }
        out.insert("typeTokens".to_owned(), Value::Object(lifted));
    }
    if let Some(assets) = object.get("assets").and_then(Value::as_array) {
        let mut lifted = Vec::new();
        for asset in assets.iter().filter_map(Value::as_object) {
            let mut asset = pick(asset, ASSET_KEYS);
            if let Some(license) = asset.get("license").and_then(Value::as_object) {
                asset["license"] = pick(license, LICENSE_KEYS);
            }
            lifted.push(asset);
        }
        out.insert("assets".to_owned(), Value::Array(lifted));
    }
    if let Some(bindings) = object.get("bindings").and_then(Value::as_object) {
        let mut lifted = Map::new();
        for (owner, list) in bindings {
            let Some(list) = list.as_array() else {
                continue;
            };
            let list: Vec<Value> = list
                .iter()
                .filter_map(Value::as_object)
                .map(|binding| pick(binding, BINDING_KEYS))
                .collect();
            lifted.insert(owner.clone(), Value::Array(list));
        }
        out.insert("bindings".to_owned(), Value::Object(lifted));
    }

    let scene = object.get("scene").and_then(Value::as_object);
    let nodes = scene
        .and_then(|scene| scene.get("objects"))
        .or_else(|| object.get("nodes"))
        .and_then(Value::as_array);
    let nodes: Vec<Value> = nodes
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(Value::as_object)
                .filter_map(node_of)
                .collect()
        })
        .unwrap_or_default();
    let objects: Vec<Value> = scene
        .and_then(|scene| scene.get("objects"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !nodes.is_empty() {
        let tokens = out
            .entry("typeTokens")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(tokens) = tokens.as_object_mut() {
            declare_tokens(tokens, &nodes, &objects);
        }
    }
    out.insert("nodes".to_owned(), Value::Array(nodes));

    serde_json::from_value(Value::Object(out)).map_err(|error| error.to_string())
}

const METADATA_KEYS: &[&str] = &[
    "name",
    "author",
    "description",
    "version",
    "themeLanguage",
    "tags",
    "remixOf",
    "createdAt",
    "updatedAt",
];
const ARTBOARD_KEYS: &[&str] = &["width", "height", "contentFit", "shape"];
const BACKGROUND_KEYS: &[&str] = &["mode", "color", "media"];
const BACKGROUND_MEDIA_KEYS: &[&str] = &[
    "assetId",
    "color",
    "bounds",
    "offsetX",
    "offsetY",
    "scale",
    "flipX",
    "flipY",
    "fallbackFrame",
    "composition",
];
const PALETTE_KEYS: &[&str] = &["name", "value"];
const TYPE_TOKEN_KEYS: &[&str] = &["family", "weight", "face"];
const ASSET_KEYS: &[&str] = &[
    "id",
    "kind",
    "path",
    "sha256",
    "sourceUrl",
    "license",
    "family",
    "weight",
    "style",
    "format",
    "subset",
];
const LICENSE_KEYS: &[&str] = &["name", "url", "attribution"];
const BINDING_KEYS: &[&str] = &[
    "id",
    "semanticKey",
    "precision",
    "unitDisplay",
    "scale",
    "offset",
    "unit",
    "autoScale",
    "format",
    "timeZone",
];
const RUN_KEYS: &[&str] = &[
    "kind",
    "text",
    "bindingId",
    "precision",
    "unitDisplay",
    "typeToken",
];

fn field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn number(object: &Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key).and_then(|value| match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    })
}

fn pick(object: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut picked = Map::new();
    for key in keys {
        if let Some(value) = object.get(*key) {
            picked.insert((*key).to_owned(), value.clone());
        }
    }
    Value::Object(picked)
}

/// One scene object as a node, or nothing when this port draws no such kind.
fn node_of(object: &Map<String, Value>) -> Option<Value> {
    let id = field(object, "id")?.to_owned();
    let kind = kind_of(object)?;
    let mut node = Map::new();
    node.insert("id".to_owned(), json!(id));
    if let Some(name) = object.get("name") {
        node.insert("name".to_owned(), name.clone());
    }
    node.insert("kind".to_owned(), kind);
    node.insert("transform".to_owned(), transform_of(object));
    for key in ["visible", "locked"] {
        if let Some(value) = object.get(key) {
            node.insert(key.to_owned(), value.clone());
        }
    }
    let style = style_of(object);
    if !style.is_empty() {
        node.insert("style".to_owned(), Value::Object(style));
    }
    if let Some(glass) = object.get("yunariGlass").and_then(Value::as_object) {
        node.insert("glass".to_owned(), pick(glass, &["blurRadius", "sampling"]));
    }
    if let Some(children) = object.get("objects").and_then(Value::as_array) {
        let children: Vec<Value> = children
            .iter()
            .filter_map(Value::as_object)
            .filter_map(node_of)
            .collect();
        node.insert("children".to_owned(), Value::Array(children));
    }
    Some(Value::Object(node))
}

fn transform_of(object: &Map<String, Value>) -> Value {
    let mut transform = Map::new();
    for (from, to) in [
        ("left", "x"),
        ("top", "y"),
        ("width", "width"),
        ("height", "height"),
        ("angle", "rotation"),
        ("scaleX", "scaleX"),
        ("scaleY", "scaleY"),
    ] {
        if let Some(value) = number(object, from) {
            transform.insert(to.to_owned(), json!(value));
        }
    }
    Value::Object(transform)
}

/// A node's style map. The authored paint is a bare `palette.<id>` string; a
/// style slot is either a reference or a literal, so a string is wrapped in the
/// form the model reads.
fn style_of(object: &Map<String, Value>) -> Map<String, Value> {
    let mut style = Map::new();
    if let Some(paint) = object.get("yunariPaint").and_then(Value::as_object) {
        for (key, value) in paint {
            style.insert(key.clone(), style_value(value));
        }
    }
    for key in ["opacity", "charSpacing"] {
        if let Some(value) = number(object, key) {
            style.insert(key.to_owned(), json!({ "value": value }));
        }
    }
    style
}

fn style_map(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), style_value(value)))
        .collect()
}

fn style_value(value: &Value) -> Value {
    match value {
        Value::String(text) if text.starts_with("palette.") => json!({ "ref": text }),
        Value::String(text) => json!({ "value": text }),
        Value::Object(map) if map.contains_key("ref") || map.contains_key("value") => value.clone(),
        other => json!({ "value": other }),
    }
}

fn kind_of(object: &Map<String, Value>) -> Option<Value> {
    let kind = match field(object, "type")? {
        "Rect" => json!({
            "type": "rectangle",
            "cornerRadius": number(object, "rx")
                .or_else(|| number(object, "radius"))
                .unwrap_or(0.0),
        }),
        "Ellipse" => json!({ "type": "ellipse" }),
        "Path" => json!({
            "type": "path",
            "data": path_data(object.get("path")?)?,
            "strokeWidth": number(object, "strokeWidth").unwrap_or(1.0),
            "closed": object
                .get("path")
                .and_then(Value::as_array)
                .is_some_and(|commands| {
                    commands.iter().any(|command| {
                        command
                            .as_array()
                            .and_then(|parts| parts.first())
                            .and_then(Value::as_str)
                            .is_some_and(|head| head.eq_ignore_ascii_case("z"))
                    })
                }),
        }),
        "Group" => json!({ "type": "group" }),
        "Textbox" | "IText" | "Text" => json!({
            "type": "text",
            "content": text_content(object)?,
        }),
        "YunariChart" => json!({
            "type": "chart",
            "content": {
                "family": field(object, "family")?,
                "settings": object.get("settings").cloned().unwrap_or_else(|| json!({})),
            },
        }),
        _ => return None,
    };
    Some(kind)
}

/// A Fabric path is a list of `["M", 14, 2.3]` commands; the model stores one
/// SVG path string.
fn path_data(commands: &Value) -> Option<String> {
    let commands = commands.as_array()?;
    let mut data = String::new();
    for command in commands {
        let parts = command.as_array()?;
        let (head, arguments) = parts.split_first()?;
        if !data.is_empty() {
            data.push(' ');
        }
        data.push_str(head.as_str()?);
        for argument in arguments {
            let value = match argument {
                Value::Number(number) => number.as_f64()?,
                Value::String(text) => text.parse().ok()?,
                _ => return None,
            };
            data.push(' ');
            data.push_str(&format_number(value));
        }
    }
    Some(data)
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

fn text_content(object: &Map<String, Value>) -> Option<Value> {
    let text = object.get("yunariText").and_then(Value::as_object);
    let mut content = Map::new();
    if let Some(text) = text {
        if let Some(box_) = text.get("box").and_then(Value::as_object) {
            content.insert("box".to_owned(), pick(box_, &["width", "height"]));
        }
        for key in ["align", "verticalAlign", "overflow", "vertical"] {
            if let Some(value) = text.get(key) {
                content.insert(key.to_owned(), value.clone());
            }
        }
        if let Some(runs) = text.get("runs").and_then(Value::as_array) {
            let runs: Vec<Value> = runs
                .iter()
                .filter_map(Value::as_object)
                .map(run_of)
                .collect();
            content.insert("runs".to_owned(), Value::Array(runs));
        }
        return Some(Value::Object(content));
    }
    // A text object with no authored payload still carries its rendered string
    // and face, so it becomes one literal run in the body token.
    let body = field(object, "text")?;
    let mut style = Map::new();
    for (from, to) in [
        ("fill", "color"),
        ("fontSize", "fontSize"),
        ("lineHeight", "lineHeight"),
    ] {
        if let Some(value) = object.get(from) {
            style.insert(to.to_owned(), style_value(value));
        }
    }
    content.insert(
        "runs".to_owned(),
        json!([{ "kind": "literal", "text": body, "typeToken": "typeTokens.body", "style": style }]),
    );
    Some(Value::Object(content))
}

/// An older theme names a size-and-weight preset rather than a token, and a
/// run may name no token at all; both become the body token, which `declare_tokens`
/// makes sure the document carries.
fn run_of(run: &Map<String, Value>) -> Value {
    let mut lifted = pick(run, RUN_KEYS);
    let token = lifted
        .get("typeToken")
        .and_then(Value::as_str)
        .filter(|token| token.starts_with("typeTokens."))
        .map(str::to_owned);
    lifted["typeToken"] = json!(token.unwrap_or_else(|| "typeTokens.body".to_owned()));
    if let Some(style) = run.get("style").and_then(Value::as_object) {
        lifted["style"] = Value::Object(style_map(style));
    }
    lifted
}

/// Every token a run names must be declared, or the document is refused. A
/// theme stored before tokens existed names none, so the body token is built
/// from the face its first text object carries and copied for any name used.
fn declare_tokens(tokens: &mut Map<String, Value>, nodes: &[Value], objects: &[Value]) {
    let sample = objects
        .iter()
        .filter_map(Value::as_object)
        .find(|object| object.get("fontFamily").is_some());
    let family = sample
        .and_then(|object| field(object, "fontFamily"))
        .map(|family| family.split(',').next().unwrap_or(family).trim().to_owned())
        .unwrap_or_else(|| "Segoe UI".to_owned());
    let mut body = Map::new();
    body.insert("family".to_owned(), json!(family));
    if let Some(weight) = sample.and_then(|object| object.get("fontWeight")) {
        body.insert("weight".to_owned(), weight.clone());
    }
    let body = Value::Object(body);
    let mut missing = Vec::new();
    collect_tokens(nodes, &mut missing);
    for name in missing {
        tokens.entry(name).or_insert_with(|| body.clone());
    }
}

fn collect_tokens(nodes: &[Value], missing: &mut Vec<String>) {
    for node in nodes {
        let Some(crate::document::NodeKind::Text { content }) = node
            .get("kind")
            .cloned()
            .and_then(|kind| serde_json::from_value(kind).ok())
        else {
            if let Some(children) = node.get("children").and_then(Value::as_array) {
                collect_tokens(children, missing);
            }
            continue;
        };
        for run in content.runs {
            let token = match run {
                crate::document::TextRun::Literal { type_token, .. }
                | crate::document::TextRun::Value { type_token, .. } => type_token,
            };
            if let Some(name) = token.strip_prefix("typeTokens.") {
                let name = name.to_owned();
                if !missing.contains(&name) {
                    missing.push(name);
                }
            }
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            collect_tokens(children, missing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORED: &str = r##"{
        "schemaVersion": "3.0.0",
        "fabricVersion": "7.4.0",
        "id": "0002fd9b-f345-45a9-91bc-8758b767c338",
        "metadata": {"name": "Server Wall", "themeLanguage": "en"},
        "artboard": {"width": 1920, "height": 480},
        "background": {"color": {"ref": "palette.bg"}},
        "globals": {
            "palette": {
                "bg": {"name": "Backdrop", "value": {"kind": "solid", "color": "#0D1114"}},
                "muted": {"name": "Muted", "value": {"kind": "solid", "color": "#8FA0A8"}}
            },
            "typeTokens": {"body": {"family": "Space Grotesk", "weight": 700}}
        },
        "assets": [],
        "bindings": {"object-text-2": [{"id": "b-host", "semanticKey": "system.hostname"}]},
        "scene": {"version": "7.4.0", "objects": [
            {"type": "Rect", "id": "rule", "name": "rule", "left": 48, "top": 88,
             "width": 1824, "height": 1, "rx": 4, "yunariPaint": {"fill": "palette.muted"}},
            {"type": "Textbox", "id": "host", "left": 48, "top": 34, "width": 64,
             "text": "HOST", "fill": "#8FA0A8",
             "yunariPaint": {"fill": "palette.muted"},
             "yunariText": {"box": {"width": 64, "height": 24}, "align": "left",
               "verticalAlign": "middle", "overflow": "clip",
               "runs": [{"kind": "value", "bindingId": "b-host", "typeToken": "typeTokens.body",
                 "style": {"color": {"ref": "palette.muted"}, "fontSize": {"value": 16}},
                 "unitDisplay": "long"}]}},
            {"type": "Path", "id": "icon", "left": 48, "top": 136, "width": 28, "height": 28,
             "strokeWidth": 2, "yunariPaint": {"stroke": "palette.muted"},
             "path": [["M", 14, 2.5], ["L", 14, 25.5], ["Z"]]},
            {"type": "YunariChart", "id": "chart", "family": "gauge", "left": 436, "top": 214,
             "width": 128, "height": 128, "settings": {"min": 0, "max": 100}},
            {"type": "Video", "id": "movie", "left": 0, "top": 0, "width": 10, "height": 10}
        ]}
    }"##;

    #[test]
    fn a_stored_envelope_lifts_into_the_native_shape() {
        let value: Value = serde_json::from_str(STORED).unwrap();
        assert!(looks_like_source(&value));
        let document = from_source_str(STORED).unwrap();
        assert_eq!(document.id, "0002fd9b-f345-45a9-91bc-8758b767c338");
        assert_eq!(document.metadata.name.as_deref(), Some("Server Wall"));
        assert_eq!(document.artboard.width, 1920.0);
        assert_eq!(document.palette["muted"].name, "Muted");
        assert_eq!(document.type_tokens["body"].family, "Space Grotesk");
        assert_eq!(
            document.bindings["object-text-2"][0].semantic_key,
            "system.hostname"
        );
        // Four kinds lift; the video object this port does not draw is dropped.
        assert_eq!(document.nodes.len(), 4);
    }

    #[test]
    fn a_rectangle_keeps_its_radius_and_its_paint() {
        let document = from_source_str(STORED).unwrap();
        let rule = &document.nodes[0];
        assert_eq!(rule.transform.x, 48.0);
        assert_eq!(rule.transform.width, 1824.0);
        assert!(matches!(
            rule.kind,
            crate::document::NodeKind::Rectangle {
                corner_radius
            } if corner_radius == 4.0
        ));
        assert_eq!(
            rule.style["fill"],
            crate::document::StyleValue::Ref(crate::document::StyleRef {
                r#ref: "palette.muted".to_owned()
            })
        );
    }

    #[test]
    fn a_text_box_keeps_its_value_run() {
        let document = from_source_str(STORED).unwrap();
        let crate::document::NodeKind::Text { content } = &document.nodes[1].kind else {
            panic!("expected text");
        };
        assert_eq!(content.align, crate::document::TextAlign::Left);
        assert_eq!(
            content.vertical_align,
            crate::document::VerticalAlign::Middle
        );
        assert_eq!(content.overflow, crate::document::TextOverflow::Clip);
        assert_eq!(content.runs.len(), 1);
        let crate::document::TextRun::Value {
            binding_id,
            unit_display,
            ..
        } = &content.runs[0]
        else {
            panic!("expected a value run");
        };
        assert_eq!(binding_id, "b-host");
        assert_eq!(*unit_display, Some(crate::document::UnitDisplay::Long));
        assert_eq!(content.box_.unwrap().width, 64.0);
    }

    #[test]
    fn fabric_path_commands_become_one_path_string() {
        let document = from_source_str(STORED).unwrap();
        let crate::document::NodeKind::Path {
            data,
            stroke_width,
            closed,
        } = &document.nodes[2].kind
        else {
            panic!("expected a path");
        };
        assert_eq!(data, "M 14 2.5 L 14 25.5 Z");
        assert_eq!(*stroke_width, 2.0);
        assert!(closed);
    }

    #[test]
    fn a_chart_keeps_its_family_and_settings() {
        let document = from_source_str(STORED).unwrap();
        let crate::document::NodeKind::Chart { content } = &document.nodes[3].kind else {
            panic!("expected a chart");
        };
        assert_eq!(content.family, crate::document::ChartFamily::Gauge);
        assert_eq!(content.settings.0["max"], 100.0);
    }
}
