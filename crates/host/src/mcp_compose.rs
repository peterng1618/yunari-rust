//! The theme-composition tools of the agent surface: a draft in memory, the
//! objects a brief becomes, and one save that puts it in the library.
//!
//! Every argument is validated here, not by the tool's JSON schema alone: a
//! schema a client ignores is still a boundary. A tool that cannot apply its
//! call says why in a sentence and leaves the draft as it was.

use serde_json::{Value, json};
use yunari_model::document::{
    Binding, ChartContent, ChartFamily, ChartSettings, ImageContent, ImageFit, Node, NodeKind,
    TextAlign, TextContent, TextOverflow, TextRun, ThemeDocument, Transform, UnitDisplay,
    VerticalAlign,
};
use yunari_model::validate::validate_document;

use crate::drafts::Draft;
use crate::http::Shared;

const NO_DRAFT: &str = "There is no draft. Call create_theme first, or open_theme by id.";

pub fn tools() -> Value {
    let theme_id = || json!({"type": "string", "minLength": 1});
    let box_schema = json!({
        "type": "object",
        "properties": {
            "left": {"type": "number"},
            "top": {"type": "number"},
            "width": {"type": "number"},
            "height": {"type": "number"}
        },
        "required": ["left", "top", "width", "height"],
        "additionalProperties": false
    });
    let binding = json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "minLength": 1},
            "semanticKey": {"type": "string", "minLength": 1},
            "precision": {"type": "number"},
            "unitDisplay": {"enum": ["none", "short", "long"]},
            "scale": {"type": "number"},
            "offset": {"type": "number"},
            "unit": {"type": "string"},
            "autoScale": {"type": "boolean"},
            "format": {"type": "string"},
            "timeZone": {"type": "string"}
        },
        "required": ["id", "semanticKey"],
        "additionalProperties": false
    });
    json!([
        {
            "name": "create_theme",
            "description": "Start a theme draft and return its id. The starter carries a title and a live clock; a blank draft carries only its tokens and artboard.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "starter": {"type": "boolean"}
                },
                "additionalProperties": false
            }
        },
        {
            "name": "open_theme",
            "description": "Open a stored theme as a draft, so compose tools can change a copy before a save.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string", "minLength": 1}},
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "save_theme",
            "description": "Validate the draft and write it to this host's library. Returns the stored id and its base.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "themeId": theme_id(),
                    "id": {"type": "string"},
                    "overwrite": {"type": "boolean"}
                },
                "additionalProperties": false
            }
        },
        {
            "name": "discard_draft",
            "description": "Drop a draft without writing anything.",
            "inputSchema": {
                "type": "object",
                "properties": {"themeId": theme_id()},
                "additionalProperties": false
            }
        },
        {
            "name": "list_drafts",
            "description": "List the drafts this host holds in memory.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "get_draft",
            "description": "Read one draft's whole document.",
            "inputSchema": {
                "type": "object",
                "properties": {"themeId": theme_id()},
                "additionalProperties": false
            }
        },
        {
            "name": "validate_theme",
            "description": "Validate a draft and report every issue without saving.",
            "inputSchema": {
                "type": "object",
                "properties": {"themeId": theme_id()},
                "additionalProperties": false
            }
        },
        {
            "name": "list_objects",
            "description": "List a draft's scene objects, bindings and declared assets.",
            "inputSchema": {
                "type": "object",
                "properties": {"themeId": theme_id()},
                "additionalProperties": false
            }
        },
        {
            "name": "add_text",
            "description": "Add a text object with literal and value runs. A value run names a binding supplied in bindings. A run may name a type token (heading, body or mono).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "box": box_schema,
                    "name": {"type": "string"},
                    "runs": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": {"enum": ["literal", "value"]},
                                "bindingId": {"type": "string"},
                                "text": {"type": "string"},
                                "token": {"type": "string"},
                                "size": {"type": "number", "exclusiveMinimum": 0},
                                "typeToken": {"enum": ["heading", "body", "mono"]},
                                "precision": {"type": "integer"},
                                "unitDisplay": {"enum": ["none", "short", "long"]}
                            },
                            "required": ["size"],
                            "additionalProperties": false
                        }
                    },
                    "align": {"enum": ["left", "center", "right"]},
                    "verticalAlign": {"enum": ["top", "middle", "bottom"]},
                    "overflow": {"enum": ["clip", "ellipsis", "visible"]},
                    "vertical": {"type": "boolean"},
                    "bindings": {"type": "array", "items": binding},
                    "themeId": theme_id()
                },
                "required": ["box", "runs"],
                "additionalProperties": false
            }
        },
        {
            "name": "add_shape",
            "description": "Add a rect, ellipse or path shape with a palette token. A path supplies its own commands and stroke width; a rect takes a radius.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": {"enum": ["rect", "ellipse", "path"]},
                    "box": box_schema,
                    "name": {"type": "string"},
                    "token": {"type": "string"},
                    "radius": {"type": "number"},
                    "points": {"type": "array", "items": {"type": "array", "items": {"type": ["string", "number"]}}},
                    "strokeWidth": {"type": "number", "exclusiveMinimum": 0},
                    "themeId": theme_id()
                },
                "required": ["kind", "box"],
                "additionalProperties": false
            }
        },
        {
            "name": "import_asset",
            "description": "Read a picture from this PC into the draft and declare it. PNG, JPEG, GIF, WebP and a plain SVG are accepted; a scripted SVG is refused.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "themeId": theme_id()
                },
                "required": ["path"],
                "additionalProperties": false
            }
        },
        {
            "name": "add_image",
            "description": "Place a declared asset in the draft. The image fills its authored box.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "assetId": {"type": "string", "minLength": 1},
                    "box": box_schema,
                    "name": {"type": "string"},
                    "fit": {"enum": ["cover", "contain"]},
                    "themeId": theme_id()
                },
                "required": ["assetId", "box"],
                "additionalProperties": false
            }
        },
        {
            "name": "add_chart",
            "description": "Add a line, bar or gauge chart reading one or more bindings. The box is its top-left, like every other compose tool.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "family": {"enum": ["line", "bar", "gauge"]},
                    "box": box_schema,
                    "name": {"type": "string"},
                    "binding": binding,
                    "bindings": {"type": "array", "items": binding},
                    "settings": {},
                    "themeId": theme_id()
                },
                "required": ["family", "box"],
                "additionalProperties": false
            }
        },
        {
            "name": "add_binding",
            "description": "Replace one object's semantic bindings. Binding ids must be unique across the document.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "objectId": {"type": "string", "minLength": 1},
                    "bindings": {"type": "array", "items": binding},
                    "themeId": theme_id()
                },
                "required": ["objectId", "bindings"],
                "additionalProperties": false
            }
        },
        {
            "name": "update_object",
            "description": "Shallow-merge a validated JSON patch into one scene object.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "objectId": {"type": "string", "minLength": 1},
                    "patch": {"type": "object"},
                    "themeId": theme_id()
                },
                "required": ["objectId", "patch"],
                "additionalProperties": false
            }
        },
        {
            "name": "remove_object",
            "description": "Remove one scene object and every binding it carried.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "objectId": {"type": "string", "minLength": 1},
                    "themeId": theme_id()
                },
                "required": ["objectId"],
                "additionalProperties": false
            }
        },
        {
            "name": "reorder_object",
            "description": "Restack one scene object among its siblings: front paints last, back paints first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "objectId": {"type": "string", "minLength": 1},
                    "move": {"enum": ["front", "forward", "backward", "back"]},
                    "themeId": theme_id()
                },
                "required": ["objectId", "move"],
                "additionalProperties": false
            }
        },
        {
            "name": "group_objects",
            "description": "Wrap root scene objects in a group and name it for the component they form.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "objectIds": {"type": "array", "minItems": 1, "items": {"type": "string"}},
                    "name": {"type": "string"},
                    "themeId": theme_id()
                },
                "required": ["objectIds"],
                "additionalProperties": false
            }
        },
        {
            "name": "ungroup_objects",
            "description": "Lift a root group's children back into the scene and remove the group.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "groupId": {"type": "string", "minLength": 1},
                    "themeId": theme_id()
                },
                "required": ["groupId"],
                "additionalProperties": false
            }
        },
        {
            "name": "set_artboard",
            "description": "Set the draft's artboard size, shape and content fit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "width": {"type": "number", "exclusiveMinimum": 0},
                    "height": {"type": "number", "exclusiveMinimum": 0},
                    "shape": {"enum": ["rect", "circle"]},
                    "contentFit": {"enum": ["contain", "cover"]},
                    "themeId": theme_id()
                },
                "required": ["width", "height"],
                "additionalProperties": false
            }
        },
        {
            "name": "set_background",
            "description": "Set the draft's background colour token or literal.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "color": {"type": "string"},
                    "themeId": theme_id()
                },
                "required": ["color"],
                "additionalProperties": false
            }
        },
        {
            "name": "set_palette",
            "description": "Merge palette entries into the draft. A value is a CSS colour or a paint token.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string", "minLength": 1},
                                "name": {"type": "string", "minLength": 1},
                                "value": {}
                            },
                            "required": ["id", "name", "value"],
                            "additionalProperties": false
                        }
                    },
                    "themeId": theme_id()
                },
                "required": ["entries"],
                "additionalProperties": false
            }
        },
        {
            "name": "set_type_tokens",
            "description": "Set a role's type family, weight and packaged face.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "heading": {"type": "object"},
                    "body": {"type": "object"},
                    "mono": {"type": "object"},
                    "themeId": theme_id()
                },
                "additionalProperties": false
            }
        },
        {
            "name": "set_theme_metadata",
            "description": "Merge name, author, description, version or language into the draft's metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "author": {"type": "string"},
                    "description": {"type": "string"},
                    "version": {"type": "string"},
                    "language": {"type": "string"},
                    "themeId": theme_id()
                },
                "additionalProperties": false
            }
        }
    ])
}

/// Runs one composition tool, or `None` when the name is not one of ours.
pub fn call(app: &Shared, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let result = match name {
        "create_theme" => create_theme(app, args),
        "open_theme" => open_theme(app, args),
        "save_theme" => save_theme(app, args),
        "discard_draft" => discard_draft(app, args),
        "list_drafts" => list_drafts(app),
        "get_draft" => get_draft(app, args),
        "validate_theme" => validate_theme(app, args),
        "list_objects" => list_objects(app, args),
        "add_text" => add_text(app, args),
        "add_shape" => add_shape(app, args),
        "import_asset" => import_asset(app, args),
        "add_image" => add_image(app, args),
        "add_chart" => add_chart(app, args),
        "add_binding" => add_binding(app, args),
        "update_object" => update_object(app, args),
        "remove_object" => remove_object(app, args),
        "reorder_object" => reorder_object(app, args),
        "group_objects" => group_objects(app, args),
        "ungroup_objects" => ungroup_objects(app, args),
        "set_artboard" => set_artboard(app, args),
        "set_background" => set_background(app, args),
        "set_palette" => set_palette(app, args),
        "set_type_tokens" => set_type_tokens(app, args),
        "set_theme_metadata" => set_theme_metadata(app, args),
        _ => return None,
    };
    Some(result)
}

fn theme_id(args: &Value) -> Option<&str> {
    args.get("themeId").and_then(Value::as_str)
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn number(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(Value::as_f64)
}

fn truth(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn with_draft<T>(
    app: &Shared,
    args: &Value,
    edit: impl FnOnce(&mut Draft) -> Result<T, String>,
) -> Result<T, String> {
    let mut drafts = app.drafts.lock().map_err(|_| NO_DRAFT.to_owned())?;
    let draft = drafts.draft_mut(theme_id(args)).ok_or(NO_DRAFT)?;
    edit(draft)
}

fn with_read<T>(
    app: &Shared,
    args: &Value,
    read: impl FnOnce(&Draft) -> Result<T, String>,
) -> Result<T, String> {
    let drafts = app.drafts.lock().map_err(|_| NO_DRAFT.to_owned())?;
    let draft = drafts.draft(theme_id(args)).ok_or(NO_DRAFT)?;
    read(draft)
}

fn create_theme(app: &Shared, args: &Value) -> Result<Value, String> {
    let document = if truth(args, "starter") {
        yunari_model::presets::starter()
    } else {
        yunari_model::presets::blank()
    };
    let mut document = document;
    if let Some(name) = text(args, "name") {
        document.metadata.name = Some(name);
    }
    let theme_id = app
        .drafts
        .lock()
        .map_err(|_| NO_DRAFT.to_owned())?
        .create(document);
    Ok(json!({"themeId": theme_id}))
}

fn open_theme(app: &Shared, args: &Value) -> Result<Value, String> {
    let id = text(args, "id").ok_or("open_theme needs an id.")?;
    let stored = app.library.read(&id).map_err(|error| error.to_string())?;
    let assets = app.library.assets(&id, &stored.document);
    let draft = Draft {
        document: stored.document,
        assets,
        base: Some(stored.base),
    };
    let theme_id = app
        .drafts
        .lock()
        .map_err(|_| NO_DRAFT.to_owned())?
        .adopt(draft);
    Ok(json!({"themeId": theme_id, "opened": id}))
}

fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut dash = false;
    for chr in name.chars() {
        if chr.is_ascii_alphanumeric() {
            out.push(chr.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_owned();
    (!out.is_empty()).then(|| out.chars().take(64).collect())
}

fn save_theme(app: &Shared, args: &Value) -> Result<Value, String> {
    let overwrite = truth(args, "overwrite");
    let wanted = text(args, "id");
    let mut drafts = app.drafts.lock().map_err(|_| NO_DRAFT.to_owned())?;
    let draft = drafts.draft_mut(theme_id(args)).ok_or(NO_DRAFT)?;
    let id = wanted
        .or_else(|| draft.document.metadata.name.as_deref().and_then(slug))
        .or_else(|| slug(&draft.document.id))
        .ok_or("The draft has no name to save under.")?;
    if let Err(issues) = validate_document(&draft.document) {
        return Err(issues
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "));
    }
    draft.document.id = id.clone();
    let base = draft.base.clone();
    let saved = app
        .library
        .save(&yunari_library::SaveRequest {
            id: &id,
            document: &draft.document,
            assets: &draft.assets,
            base: base.as_deref(),
            overwrite,
        })
        .map_err(|error| error.to_string())?;
    draft.base = Some(saved.base.clone());
    Ok(json!({"id": saved.entry.id, "base": saved.base}))
}

fn discard_draft(app: &Shared, args: &Value) -> Result<Value, String> {
    let dropped = app
        .drafts
        .lock()
        .map_err(|_| NO_DRAFT.to_owned())?
        .remove(theme_id(args))
        .ok_or(NO_DRAFT)?;
    Ok(json!({"discarded": dropped.document.id}))
}

fn list_drafts(app: &Shared) -> Result<Value, String> {
    let drafts = app.drafts.lock().map_err(|_| NO_DRAFT.to_owned())?;
    Ok(json!({"drafts": drafts.ids()}))
}

fn get_draft(app: &Shared, args: &Value) -> Result<Value, String> {
    with_read(app, args, |draft| {
        Ok(json!({"themeId": draft.document.id, "document": draft.document}))
    })
}

fn validate_theme(app: &Shared, args: &Value) -> Result<Value, String> {
    with_read(app, args, |draft| {
        Ok(match validate_document(&draft.document) {
            Ok(()) => json!({"ok": true, "issues": []}),
            Err(issues) => json!({
                "ok": false,
                "issues": issues.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }),
        })
    })
}

fn list_objects(app: &Shared, args: &Value) -> Result<Value, String> {
    with_read(app, args, |draft| {
        Ok(json!({
            "themeId": draft.document.id,
            "objects": draft.document.nodes,
            "bindings": draft.document.bindings,
            "assets": draft.document.assets.iter().map(|asset| json!({
                "id": asset.id,
                "kind": asset.kind,
                "path": asset.path,
                "sha256": asset.sha256,
            })).collect::<Vec<_>>(),
        }))
    })
}

fn free_id(document: &ThemeDocument, prefix: &str) -> String {
    let taken = |id: &str| find(&document.nodes, id).is_some();
    let mut index = 1;
    loop {
        let id = format!("{prefix}-{index}");
        if !taken(&id) {
            return id;
        }
        index += 1;
    }
}

fn find<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find(&node.children, id) {
            return Some(found);
        }
    }
    None
}

fn find_mut<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_mut(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

fn contains(nodes: &[Node], id: &str) -> bool {
    find(nodes, id).is_some()
}

fn transform(args: &Value) -> Result<Transform, String> {
    let Some(box_) = args.get("box") else {
        return Err("This tool needs a box.".to_owned());
    };
    let field = |key: &str| {
        box_.get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("The box needs {key}."))
    };
    Ok(Transform {
        x: field("left")?,
        y: field("top")?,
        width: field("width")?,
        height: field("height")?,
        ..Transform::default()
    })
}

fn bindings_of(args: &Value) -> Result<Vec<Binding>, String> {
    let mut out = Vec::new();
    let list = args
        .get("bindings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let single = args.get("binding").cloned();
    for value in single.into_iter().chain(list) {
        out.push(binding_of(&value)?);
    }
    Ok(out)
}

fn binding_of(value: &Value) -> Result<Binding, String> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("A binding needs an id.")?;
    let semantic_key = value
        .get("semanticKey")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or("A binding needs a semanticKey.")?;
    Ok(Binding {
        id: id.to_owned(),
        semantic_key: semantic_key.to_owned(),
        precision: value
            .get("precision")
            .and_then(Value::as_u64)
            .map(|value| value as u8),
        unit_display: value.get("unitDisplay").and_then(unit_display),
        scale: value.get("scale").and_then(Value::as_f64),
        offset: value.get("offset").and_then(Value::as_f64),
        unit: text(value, "unit"),
        auto_scale: value.get("autoScale").and_then(Value::as_bool),
        format: text(value, "format"),
        time_zone: text(value, "timeZone"),
    })
}

fn unit_display(value: &Value) -> Option<UnitDisplay> {
    match value.as_str()? {
        "none" => Some(UnitDisplay::None),
        "short" => Some(UnitDisplay::Short),
        "long" => Some(UnitDisplay::Long),
        _ => None,
    }
}

/// Insert one object and the bindings that ride with it, refusing a binding id
/// another object already owns.
fn added(draft: &mut Draft, node: Node, bindings: Vec<Binding>) -> Result<Value, String> {
    if contains(&draft.document.nodes, &node.id) {
        return Err(format!("There is already an object \"{}\".", node.id));
    }
    let object_id = node.id.clone();
    for binding in &bindings {
        if let Some((owner, _)) = draft.document.bindings.iter().find(|(owner, list)| {
            owner.as_str() != object_id && list.iter().any(|b| b.id == binding.id)
        }) {
            return Err(format!(
                "Binding id \"{}\" already belongs to object \"{owner}\".",
                binding.id
            ));
        }
    }
    draft.document.nodes.push(node);
    if !bindings.is_empty() {
        draft.document.bindings.insert(object_id.clone(), bindings);
    }
    Ok(json!({"themeId": draft.document.id, "objectId": object_id}))
}

fn style_ref(prefix: &str, token: &str) -> yunari_model::document::StyleValue {
    use yunari_model::document::{StyleRef, StyleValue};
    StyleValue::Ref(StyleRef {
        r#ref: format!("{prefix}.{token}"),
    })
}

fn style_literal(value: Value) -> yunari_model::document::StyleValue {
    use yunari_model::document::{StyleLiteral, StyleValue};
    StyleValue::Literal(StyleLiteral { value })
}

fn add_text(app: &Shared, args: &Value) -> Result<Value, String> {
    let transform = transform(args)?;
    let runs = args
        .get("runs")
        .and_then(Value::as_array)
        .ok_or("add_text needs runs.")?;
    let bindings = bindings_of(args)?;
    let binding_ids: Vec<&str> = bindings.iter().map(|binding| binding.id.as_str()).collect();
    let mut built = Vec::new();
    for run in runs {
        let size = run
            .get("size")
            .and_then(Value::as_f64)
            .filter(|size| *size > 0.0)
            .ok_or("Every run needs a positive size.")?;
        let mut style = yunari_model::document::StyleMap::new();
        style.insert("fontSize".to_owned(), style_literal(Value::from(size)));
        if let Some(token) = text(run, "token") {
            style.insert("fill".to_owned(), style_ref("palette", &token));
        }
        let type_token = text(run, "typeToken")
            .map(|role| format!("typeTokens.{role}"))
            .unwrap_or_else(|| "typeTokens.body".to_owned());
        let kind = text(run, "kind").unwrap_or_else(|| {
            if run.get("bindingId").is_some() {
                "value".to_owned()
            } else {
                "literal".to_owned()
            }
        });
        if kind == "value" {
            let id = text(run, "bindingId").ok_or("A value run needs a bindingId.")?;
            if !binding_ids.contains(&id.as_str()) {
                return Err(format!(
                    "A value run reads binding \"{id}\", which is not in this tool's bindings."
                ));
            }
            built.push(TextRun::Value {
                binding_id: id,
                precision: run
                    .get("precision")
                    .and_then(Value::as_u64)
                    .map(|value| value as u8),
                unit_display: run.get("unitDisplay").and_then(unit_display),
                type_token,
                style,
            });
        } else {
            built.push(TextRun::Literal {
                text: text(run, "text").unwrap_or_default(),
                type_token,
                style,
            });
        }
    }
    with_draft(app, args, |draft| {
        let id = free_id(&draft.document, "text");
        let node = Node {
            id,
            name: text(args, "name"),
            kind: NodeKind::Text {
                content: TextContent {
                    runs: built,
                    box_: None,
                    overflow: match text(args, "overflow").as_deref() {
                        Some("clip") => TextOverflow::Clip,
                        Some("ellipsis") => TextOverflow::Ellipsis,
                        _ => TextOverflow::Visible,
                    },
                    align: match text(args, "align").as_deref() {
                        Some("center") => TextAlign::Center,
                        Some("right") => TextAlign::Right,
                        _ => TextAlign::Left,
                    },
                    vertical_align: match text(args, "verticalAlign").as_deref() {
                        Some("middle") => VerticalAlign::Middle,
                        Some("bottom") => VerticalAlign::Bottom,
                        _ => VerticalAlign::Top,
                    },
                    vertical: truth(args, "vertical"),
                },
            },
            transform,
            visible: true,
            locked: false,
            style: yunari_model::document::StyleMap::new(),
            glass: None,
            children: Vec::new(),
        };
        added(draft, node, bindings)
    })
}

fn add_shape(app: &Shared, args: &Value) -> Result<Value, String> {
    let kind = text(args, "kind").ok_or("add_shape needs a kind.")?;
    let transform = transform(args)?;
    let token = text(args, "token").unwrap_or_else(|| match kind.as_str() {
        "path" => "text".to_owned(),
        _ => "panel".to_owned(),
    });
    let radius = number(args, "radius");
    let points = args.get("points").and_then(Value::as_array);
    let stroke_width = number(args, "strokeWidth");
    if kind != "path" && (points.is_some() || stroke_width.is_some()) {
        return Err(format!(
            "\"points\" and \"strokeWidth\" are only for a path shape, not a \"{kind}\"."
        ));
    }
    if kind != "rect" && radius.is_some() {
        return Err(format!(
            "\"radius\" is only for a rect shape, not a \"{kind}\"."
        ));
    }
    let mut style = yunari_model::document::StyleMap::new();
    let node_kind = match kind.as_str() {
        "rect" => {
            style.insert("fill".to_owned(), style_ref("palette", &token));
            NodeKind::Rectangle {
                corner_radius: radius.unwrap_or(0.0),
            }
        }
        "ellipse" => {
            style.insert("fill".to_owned(), style_ref("palette", &token));
            NodeKind::Ellipse
        }
        "path" => {
            let points = points.ok_or("A path shape needs its own commands.")?;
            let mut data = String::new();
            for command in points {
                let Some(list) = command.as_array() else {
                    return Err("Every path command is a list.".to_owned());
                };
                let Some(letter) = list.first().and_then(Value::as_str) else {
                    return Err("Every path command starts with its letter.".to_owned());
                };
                if data.is_empty() {
                    data.push_str(letter);
                } else {
                    data.push(' ');
                    data.push_str(letter);
                }
                for value in &list[1..] {
                    let Some(value) = value.as_f64() else {
                        return Err("A path command's parameters are numbers.".to_owned());
                    };
                    data.push_str(&format!(" {value}"));
                }
            }
            let measured = crate::path_presets::measure(&data)
                .ok_or("That path data cannot be read, or draws nothing.")?;
            let (_, _, paint) = measured;
            let stroke = stroke_width.unwrap_or_else(|| (transform.width / 240.0).round().max(2.0));
            let closed = paint == crate::path_presets::Paint::Fill;
            if closed {
                style.insert("fill".to_owned(), style_ref("palette", &token));
            } else {
                style.insert("stroke".to_owned(), style_ref("palette", &token));
                style.insert("strokeWidth".to_owned(), style_literal(Value::from(stroke)));
            }
            NodeKind::Path {
                data,
                stroke_width: stroke,
                closed,
            }
        }
        other => return Err(format!("\"{other}\" is not a shape kind.")),
    };
    with_draft(app, args, |draft| {
        let node = Node {
            id: free_id(&draft.document, "shape"),
            name: text(args, "name"),
            kind: node_kind,
            transform,
            visible: true,
            locked: false,
            style,
            glass: None,
            children: Vec::new(),
        };
        added(draft, node, Vec::new())
    })
}

const NOT_A_PICTURE: &str = "That file is not a picture this host can show.";
const MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;

/// The kind a file's own bytes say it is. The name is only consulted for SVG,
/// which has no magic number of its own.
fn picture_kind(name: &str, bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some(("png", "image"));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some(("jpg", "image"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("gif", "gif"));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some(("webp", "image"));
    }
    if name.to_ascii_lowercase().ends_with(".svg")
        && let Ok(text) = std::str::from_utf8(bytes)
        && text.contains("<svg")
    {
        return Some(("svg", "svg"));
    }
    None
}

/// An SVG is text the raster backend parses, so a script or an outside
/// reference is refused before the bytes become an asset.
fn svg_is_safe(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const REFUSED: [&str; 10] = [
        "<script",
        "<foreignobject",
        "javascript:",
        "onload=",
        "onerror=",
        "onclick=",
        "onmouseover=",
        "onbegin=",
        "<!entity",
        "href=\"http",
    ];
    !REFUSED.iter().any(|needle| lower.contains(needle)) && !lower.contains("url(http")
}

fn asset_stem(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    let cleaned: String = stem
        .chars()
        .map(|chr| {
            if chr.is_ascii_alphanumeric() || chr == '-' || chr == '_' {
                chr
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_owned();
    if cleaned.is_empty() {
        "picture".to_owned()
    } else {
        cleaned
    }
}

fn import_asset(app: &Shared, args: &Value) -> Result<Value, String> {
    use yunari_model::document::{AssetKind, AssetReference};

    let path = text(args, "path").ok_or("import_asset needs a path.")?;
    let meta = std::fs::metadata(&path).map_err(|error| format!("{path}: {error}"))?;
    if meta.len() > MAX_ASSET_BYTES {
        return Err("That file is too large to import.".to_owned());
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{path}: {error}"))?;
    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&path);
    let Some((extension, kind)) = picture_kind(name, &bytes) else {
        return Err(NOT_A_PICTURE.to_owned());
    };
    if kind == "svg" {
        let text = std::str::from_utf8(&bytes).map_err(|_| NOT_A_PICTURE.to_owned())?;
        if !svg_is_safe(text) {
            return Err(
                "That SVG carries a script or an outside reference, so it was not imported."
                    .to_owned(),
            );
        }
    }
    let kind = match kind {
        "gif" => AssetKind::Gif,
        "svg" => AssetKind::Svg,
        _ => AssetKind::Image,
    };
    with_draft(app, args, |draft| {
        let stem = asset_stem(name);
        let mut id = stem.clone();
        let mut n = 2;
        while draft.document.assets.iter().any(|asset| asset.id == id) {
            id = format!("{stem}-{n}");
            n += 1;
        }
        let asset_path = format!("assets/{id}.{extension}");
        let reference = AssetReference {
            id: id.clone(),
            kind,
            path: asset_path.clone(),
            sha256: Some(yunari_library::sha256_hex(&bytes)),
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        };
        draft.document.assets.push(reference);
        draft.assets.insert(asset_path, bytes);
        Ok(json!({"themeId": draft.document.id, "assetId": id}))
    })
}

fn add_image(app: &Shared, args: &Value) -> Result<Value, String> {
    let asset_id = text(args, "assetId").ok_or("add_image needs an assetId.")?;
    let transform = transform(args)?;
    with_draft(app, args, |draft| {
        if !draft
            .document
            .assets
            .iter()
            .any(|asset| asset.id == asset_id)
        {
            return Err(format!("This theme declares no asset \"{asset_id}\"."));
        }
        let node = Node {
            id: free_id(&draft.document, "image"),
            name: text(args, "name"),
            kind: NodeKind::Image {
                content: ImageContent {
                    asset_id,
                    fit: match text(args, "fit").as_deref() {
                        Some("cover") => ImageFit::Cover,
                        _ => ImageFit::Contain,
                    },
                    monochrome: None,
                },
            },
            transform,
            visible: true,
            locked: false,
            style: yunari_model::document::StyleMap::new(),
            glass: None,
            children: Vec::new(),
        };
        added(draft, node, Vec::new())
    })
}

fn add_chart(app: &Shared, args: &Value) -> Result<Value, String> {
    let transform = transform(args)?;
    let bindings = bindings_of(args)?;
    if bindings.is_empty() {
        return Err("A chart needs at least one binding.".to_owned());
    }
    let family = match text(args, "family").as_deref() {
        Some("line") => ChartFamily::Line,
        Some("bar") => ChartFamily::Bar,
        Some("gauge") => ChartFamily::Gauge,
        _ => return Err("A chart family is line, bar or gauge.".to_owned()),
    };
    let settings = args
        .get("settings")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    with_draft(app, args, |draft| {
        let node = Node {
            id: free_id(&draft.document, "chart"),
            name: text(args, "name"),
            kind: NodeKind::Chart {
                content: ChartContent {
                    family,
                    settings: ChartSettings(settings),
                },
            },
            transform,
            visible: true,
            locked: false,
            style: yunari_model::document::StyleMap::new(),
            glass: None,
            children: Vec::new(),
        };
        added(draft, node, bindings)
    })
}

fn add_binding(app: &Shared, args: &Value) -> Result<Value, String> {
    let object_id = text(args, "objectId").ok_or("add_binding needs an objectId.")?;
    let bindings = bindings_of(args)?;
    with_draft(app, args, |draft| {
        if !contains(&draft.document.nodes, &object_id) {
            return Err(format!("There is no object \"{object_id}\" in this scene."));
        }
        let mut incoming = Vec::new();
        for binding in &bindings {
            if let Some((owner, _)) = draft.document.bindings.iter().find(|(owner, list)| {
                owner.as_str() != object_id && list.iter().any(|b| b.id == binding.id)
            }) {
                return Err(format!(
                    "Binding id \"{}\" already belongs to object \"{owner}\".",
                    binding.id
                ));
            }
            if incoming.contains(&binding.id) {
                return Err(format!("Binding id \"{}\" is repeated.", binding.id));
            }
            incoming.push(binding.id.clone());
        }
        draft
            .document
            .bindings
            .insert(object_id.clone(), bindings.clone());
        Ok(json!({
            "themeId": draft.document.id,
            "objectId": object_id,
            "bindings": bindings,
        }))
    })
}

fn update_object(app: &Shared, args: &Value) -> Result<Value, String> {
    let object_id = text(args, "objectId").ok_or("update_object needs an objectId.")?;
    let patch = args
        .get("patch")
        .and_then(Value::as_object)
        .ok_or("update_object needs a patch object.")?;
    with_draft(app, args, |draft| {
        let node = find(&draft.document.nodes, &object_id)
            .ok_or_else(|| format!("There is no object \"{object_id}\" in this scene."))?;
        let mut value = serde_json::to_value(node).map_err(|error| error.to_string())?;
        let Some(target) = value.as_object_mut() else {
            return Err("That object cannot be patched.".to_owned());
        };
        for (key, value) in patch {
            if matches!(key.as_str(), "__proto__" | "constructor" | "prototype") {
                return Err(format!("\"{key}\" is not a field."));
            }
            target.insert(key.clone(), value.clone());
        }
        let patched: Node = serde_json::from_value(value)
            .map_err(|error| format!("That patch does not make an object: {error}"))?;
        if patched.id != object_id {
            return Err("A patch may not change an object's id.".to_owned());
        }
        *find_mut(&mut draft.document.nodes, &object_id).expect("the object was found") = patched;
        Ok(json!({"themeId": draft.document.id, "objectId": object_id}))
    })
}

fn remove_object(app: &Shared, args: &Value) -> Result<Value, String> {
    let object_id = text(args, "objectId").ok_or("remove_object needs an objectId.")?;
    with_draft(app, args, |draft| {
        let removed = take(&mut draft.document.nodes, &object_id);
        if !removed {
            return Err(format!("There is no object \"{object_id}\" in this scene."));
        }
        draft.document.bindings.remove(&object_id);
        Ok(json!({"themeId": draft.document.id, "removed": object_id}))
    })
}

fn take(nodes: &mut Vec<Node>, id: &str) -> bool {
    if let Some(index) = nodes.iter().position(|node| node.id == id) {
        nodes.remove(index);
        return true;
    }
    nodes.iter_mut().any(|node| take(&mut node.children, id))
}

fn reorder_object(app: &Shared, args: &Value) -> Result<Value, String> {
    let object_id = text(args, "objectId").ok_or("reorder_object needs an objectId.")?;
    let movement = text(args, "move").ok_or("reorder_object needs a move.")?;
    with_draft(app, args, |draft| {
        let done = restack(&mut draft.document.nodes, &object_id, &movement);
        if !done {
            return Err(format!("There is no object \"{object_id}\" in this scene."));
        }
        Ok(json!({"themeId": draft.document.id, "objectId": object_id, "move": movement}))
    })
}

fn restack(nodes: &mut Vec<Node>, id: &str, movement: &str) -> bool {
    if let Some(index) = nodes.iter().position(|node| node.id == id) {
        let last = nodes.len() - 1;
        let target = match movement {
            "front" => last,
            "back" => 0,
            "forward" => (index + 1).min(last),
            "backward" => index.saturating_sub(1),
            _ => return false,
        };
        let node = nodes.remove(index);
        nodes.insert(target, node);
        return true;
    }
    nodes
        .iter_mut()
        .any(|node| restack(&mut node.children, id, movement))
}

fn group_objects(app: &Shared, args: &Value) -> Result<Value, String> {
    let ids: Vec<String> = args
        .get("objectIds")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err("group_objects needs at least one object.".to_owned());
    }
    with_draft(app, args, |draft| {
        for id in &ids {
            let node = draft
                .document
                .nodes
                .iter()
                .find(|node| &node.id == id)
                .ok_or_else(|| format!("\"{id}\" is not a root object."))?;
            if node.kind == NodeKind::Group {
                return Err(format!("\"{id}\" is already a group."));
            }
        }
        let mut children = Vec::new();
        let mut index = 0;
        while index < draft.document.nodes.len() {
            if ids.contains(&draft.document.nodes[index].id) {
                children.push(draft.document.nodes.remove(index));
            } else {
                index += 1;
            }
        }
        let bounds = bounds_of(&children);
        let node = Node {
            id: free_id(&draft.document, "group"),
            name: text(args, "name"),
            kind: NodeKind::Group,
            transform: bounds,
            visible: true,
            locked: false,
            style: yunari_model::document::StyleMap::new(),
            glass: None,
            children,
        };
        let group_id = node.id.clone();
        draft.document.nodes.push(node);
        Ok(json!({"themeId": draft.document.id, "groupId": group_id}))
    })
}

fn bounds_of(nodes: &[Node]) -> Transform {
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for node in nodes {
        let transform = &node.transform;
        left = left.min(transform.x);
        top = top.min(transform.y);
        right = right.max(transform.x + transform.width);
        bottom = bottom.max(transform.y + transform.height);
    }
    if !left.is_finite() {
        return Transform::default();
    }
    Transform {
        x: left,
        y: top,
        width: (right - left).max(0.0),
        height: (bottom - top).max(0.0),
        ..Transform::default()
    }
}

fn ungroup_objects(app: &Shared, args: &Value) -> Result<Value, String> {
    let group_id = text(args, "groupId").ok_or("ungroup_objects needs a groupId.")?;
    with_draft(app, args, |draft| {
        let Some(index) = draft
            .document
            .nodes
            .iter()
            .position(|node| node.id == group_id)
        else {
            return Err(format!("There is no root group \"{group_id}\"."));
        };
        if draft.document.nodes[index].kind != NodeKind::Group {
            return Err(format!("\"{group_id}\" is not a group."));
        }
        let group = draft.document.nodes.remove(index);
        let children = group.children.len();
        draft.document.nodes.extend(group.children);
        Ok(json!({"themeId": draft.document.id, "lifted": children}))
    })
}

fn set_artboard(app: &Shared, args: &Value) -> Result<Value, String> {
    use yunari_model::document::{ArtboardShape, ContentFit};
    let width = number(args, "width").ok_or("set_artboard needs a width.")?;
    let height = number(args, "height").ok_or("set_artboard needs a height.")?;
    with_draft(app, args, |draft| {
        draft.document.artboard.width = width;
        draft.document.artboard.height = height;
        if let Some(shape) = text(args, "shape") {
            draft.document.artboard.shape = match shape.as_str() {
                "circle" => ArtboardShape::Circle,
                "rect" => ArtboardShape::Rect,
                other => return Err(format!("\"{other}\" is not an artboard shape.")),
            };
        }
        if let Some(fit) = text(args, "contentFit") {
            draft.document.artboard.content_fit = match fit.as_str() {
                "cover" => ContentFit::Cover,
                "contain" => ContentFit::Contain,
                other => return Err(format!("\"{other}\" is not a content fit.")),
            };
        }
        Ok(json!({"themeId": draft.document.id, "artboard": draft.document.artboard}))
    })
}

fn set_background(app: &Shared, args: &Value) -> Result<Value, String> {
    let color = text(args, "color").ok_or("set_background needs a color.")?;
    with_draft(app, args, |draft| {
        draft.document.background.color =
            Some(if let Some(token) = color.strip_prefix("palette.") {
                style_ref("palette", token)
            } else {
                style_literal(Value::String(color.clone()))
            });
        Ok(json!({"themeId": draft.document.id}))
    })
}

fn set_palette(app: &Shared, args: &Value) -> Result<Value, String> {
    use yunari_model::document::{Paint as PaintKind, PaletteEntry};
    let entries = args
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("set_palette needs entries.")?;
    let mut parsed = Vec::new();
    for entry in entries {
        let id = text(entry, "id").ok_or("Every palette entry needs an id.")?;
        let name = text(entry, "name").ok_or("Every palette entry needs a name.")?;
        let value = entry
            .get("value")
            .ok_or("Every palette entry needs a value.")?;
        let paint = match value {
            Value::String(colour) => PaintKind::Solid {
                color: colour.clone(),
            },
            Value::Object(map) => serde_json::from_value::<PaintKind>(Value::Object(map.clone()))
                .map_err(|error| format!("\"{id}\" is not a paint: {error}"))?,
            _ => return Err(format!("\"{id}\" is not a paint.")),
        };
        parsed.push((id, PaletteEntry { name, value: paint }));
    }
    with_draft(app, args, |draft| {
        for (id, entry) in parsed {
            draft.document.palette.insert(id, entry);
        }
        Ok(json!({"themeId": draft.document.id, "palette": draft.document.palette}))
    })
}

fn set_type_tokens(app: &Shared, args: &Value) -> Result<Value, String> {
    use yunari_model::document::{TypeToken, TypeWeight};

    fn token(id: &str, value: &Value) -> Result<TypeToken, String> {
        let family = text(value, "family").ok_or_else(|| format!("\"{id}\" needs a family."))?;
        let weight = match value.get("weight") {
            Some(Value::Number(number)) => number.as_f64().map(TypeWeight::Number),
            Some(Value::String(text)) => Some(TypeWeight::Name(text.clone())),
            _ => None,
        };
        let face = match value.get("face") {
            Some(Value::String(asset_id)) if !asset_id.is_empty() => {
                Some(yunari_model::document::Face {
                    asset_id: asset_id.clone(),
                })
            }
            Some(Value::Object(map)) => Some(
                serde_json::from_value(map.clone().into())
                    .map_err(|error| format!("\"{id}\" has no usable face: {error}"))?,
            ),
            _ => None,
        };
        Ok(TypeToken {
            family,
            weight,
            face,
        })
    }

    let mut parsed = Vec::new();
    for role in ["heading", "body", "mono"] {
        if let Some(value) = args.get(role) {
            parsed.push((role.to_owned(), token(role, value)?));
        }
    }
    if parsed.is_empty() {
        return Err("Name heading, body or mono.".to_owned());
    }
    with_draft(app, args, |draft| {
        for (role, token) in parsed {
            draft.document.type_tokens.insert(role, token);
        }
        Ok(json!({"themeId": draft.document.id, "typeTokens": draft.document.type_tokens}))
    })
}

fn set_theme_metadata(app: &Shared, args: &Value) -> Result<Value, String> {
    with_draft(app, args, |draft| {
        for (key, field) in [
            ("name", &mut draft.document.metadata.name),
            ("author", &mut draft.document.metadata.author),
            ("description", &mut draft.document.metadata.description),
            ("version", &mut draft.document.metadata.version),
            ("language", &mut draft.document.metadata.theme_language),
        ] {
            if let Some(value) = text(args, key) {
                *field = Some(value);
            }
        }
        Ok(json!({"themeId": draft.document.id, "metadata": draft.document.metadata}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Shared {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let config = crate::config::HostConfig::resolve(
            Some(std::env::temp_dir().join(format!(
                "yunari-mcp-compose-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))),
            Some(0),
            false,
        );
        std::fs::create_dir_all(config.settings_dir()).ok();
        crate::server::app(config)
    }

    fn draft_id(app: &Shared) -> String {
        call(
            app,
            "create_theme",
            &json!({"name": "Boards", "starter": true}),
        )
        .expect("a tool")
        .expect("a draft")["themeId"]
            .as_str()
            .expect("an id")
            .to_owned()
    }

    #[test]
    fn a_draft_is_composed_and_saved_under_its_name() {
        let app = app();
        let theme_id = draft_id(&app);
        let added = call(
            &app,
            "add_text",
            &json!({
                "themeId": theme_id,
                "box": {"left": 20, "top": 30, "width": 400, "height": 60},
                "runs": [{"kind": "literal", "text": "Hello", "size": 40, "typeToken": "heading"}],
            }),
        )
        .expect("a tool")
        .expect("a text object");
        assert_eq!(added["objectId"], "text-1");
        let saved = call(&app, "save_theme", &json!({"themeId": theme_id}))
            .expect("a tool")
            .expect("a save");
        assert_eq!(saved["id"], "boards");
        assert!(app.library.read("boards").is_ok());
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn a_value_run_must_name_a_binding_the_call_supplies() {
        let app = app();
        let theme_id = draft_id(&app);
        let refused = call(
            &app,
            "add_text",
            &json!({
                "themeId": theme_id,
                "box": {"left": 0, "top": 0, "width": 10, "height": 10},
                "runs": [{"kind": "value", "bindingId": "absent", "size": 12}],
            }),
        )
        .expect("a tool");
        assert!(refused.is_err());
        // The same call with the binding attached is accepted.
        call(
            &app,
            "add_text",
            &json!({
                "themeId": theme_id,
                "box": {"left": 0, "top": 0, "width": 10, "height": 10},
                "runs": [{"kind": "value", "bindingId": "load", "size": 12}],
                "bindings": [{"id": "load", "semanticKey": "gpu.load"}],
            }),
        )
        .expect("a tool")
        .expect("a value object");
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn a_picture_is_sniffed_and_written_with_the_theme() {
        let app = app();
        let theme_id = draft_id(&app);
        let source = app.config.settings_dir().join("photo.png");
        // A PNG signature and one byte of body: the sniff is the boundary
        // here, not a decoder.
        std::fs::write(&source, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0]).unwrap();
        let imported = call(
            &app,
            "import_asset",
            &json!({"themeId": theme_id, "path": source}),
        )
        .expect("a tool")
        .expect("an asset");
        assert_eq!(imported["assetId"], "photo");
        call(
            &app,
            "add_image",
            &json!({
                "themeId": theme_id,
                "assetId": "photo",
                "box": {"left": 0, "top": 0, "width": 10, "height": 10},
            }),
        )
        .expect("a tool")
        .expect("an image");
        call(&app, "save_theme", &json!({"themeId": theme_id}))
            .expect("a tool")
            .expect("a save");
        let stored = app.library.read("boards").expect("the theme");
        assert!(
            stored
                .document
                .assets
                .iter()
                .any(|asset| asset.id == "photo")
        );
        let files = app.library.assets("boards", &stored.document);
        assert_eq!(files["assets/photo.png"].len(), 9);
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn a_path_is_painted_by_its_geometry() {
        let app = app();
        let theme_id = draft_id(&app);
        let open = call(
            &app,
            "add_shape",
            &json!({
                "themeId": theme_id,
                "kind": "path",
                "box": {"left": 0, "top": 0, "width": 200, "height": 200},
                "points": [["M", 0, 0], ["L", 200, 0]],
            }),
        )
        .expect("a tool")
        .expect("a line");
        let objects = call(&app, "list_objects", &json!({"themeId": theme_id}))
            .expect("a tool")
            .expect("a listing");
        let shape = objects["objects"]
            .as_array()
            .expect("objects")
            .iter()
            .find(|node| node["id"] == open["objectId"])
            .expect("the shape");
        assert_eq!(shape["kind"]["closed"], false);
        assert!(shape["style"]["stroke"].is_object());
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }
}
