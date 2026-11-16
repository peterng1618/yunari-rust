//! The theme-composition tools of the agent surface: a draft in memory, the
//! objects a brief becomes, and one save that puts it in the library.
//!
//! Every argument is validated here, not by the tool's JSON schema alone: a
//! schema a client ignores is still a boundary. A tool that cannot apply its
//! call says why in a sentence and leaves the draft as it was.

use serde_json::{Value, json};
use yunari_model::credits::{CreditsAuthor, CreditsEntry, CreditsSource, ThemeCredits};
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
            "name": "add_card",
            "description": "Insert a card-library unit into the draft at artboard coordinates. Returns the created ids and any kit issues.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "unit": {"type": "string", "minLength": 1},
                    "left": {"type": "number"},
                    "top": {"type": "number"},
                    "themeId": theme_id()
                },
                "required": ["unit", "left", "top"],
                "additionalProperties": false
            }
        },
        {
            "name": "add_icon",
            "description": "Place a glyph from the shared icon set.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "icon": {"type": "string", "minLength": 1},
                    "box": box_schema,
                    "name": {"type": "string", "minLength": 1},
                    "token": {"type": "string", "minLength": 1},
                    "themeId": theme_id()
                },
                "required": ["icon", "box"],
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
            "description": "Declare an asset in the open draft from a finder result or a local file the user named. A local file needs the licence the user stated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "assetRef": {"type": "string", "minLength": 1},
                    "path": {"type": "string", "minLength": 1},
                    "license": {"type": "string", "minLength": 1},
                    "author": {"type": "string"},
                    "themeId": theme_id()
                },
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
            "name": "assign_theme",
            "description": "Make a stored theme the one a display shows. The default display is used when no displayId is named.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "minLength": 1},
                    "displayId": {"type": "string", "minLength": 1}
                },
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "yunari_status",
            "description": "Report this host's version, bound port, library size and open drafts.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "preview_theme",
            "description": "Saves the draft to the host's theme library, then previews it. A browser display answers the player URL and an artboard-aspect viewport; a panel display answers a base64 PNG at its geometry. A refused save returns its reason and no preview.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "themeId": theme_id(),
                    "displayId": {"type": "string", "minLength": 1}
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
        },
        {
            "name": "list_asset_sources",
            "description": "List the asset sources this host can search, with each source's label, category and credential requirement.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "search_assets",
            "description": "Search the host's asset finder. Each row carries an opaque assetRef for import_asset.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "minLength": 1},
                    "sourceIds": {"type": "array", "items": {"type": "string", "minLength": 1}},
                    "page": {"type": "number", "minimum": 1}
                },
                "required": ["query"],
                "additionalProperties": false
            }
        },
        {
            "name": "preview_asset",
            "description": "Fetch one finder result through the host and return it as image content.",
            "inputSchema": {
                "type": "object",
                "properties": {"assetRef": {"type": "string", "minLength": 1}},
                "required": ["assetRef"],
                "additionalProperties": false
            }
        },
        {
            "name": "remove_asset",
            "description": "Remove a declared asset the draft no longer uses, with its bytes. Refuses an asset an object, the background or a font face still references.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "assetId": {"type": "string", "minLength": 1},
                    "themeId": theme_id()
                },
                "required": ["assetId"],
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
        "add_card" => add_card(app, args),
        "add_text" => add_text(app, args),
        "add_shape" => add_shape(app, args),
        "add_icon" => add_icon(app, args),
        "import_asset" => import_asset(app, args),
        "list_asset_sources" => list_asset_sources(),
        "search_assets" => search_assets(app, args),
        "preview_asset" => preview_asset(app, args),
        "remove_asset" => remove_asset(app, args),
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
        "preview_theme" => preview_theme(app, args),
        "set_theme_metadata" => set_theme_metadata(app, args),
        "assign_theme" => assign_theme(app, args),
        "yunari_status" => yunari_status(app),
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
        credits: stored.credits,
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
            credits: draft.credits.as_ref(),
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

fn add_card(app: &Shared, args: &Value) -> Result<Value, String> {
    let unit_id = text(args, "unit").ok_or("add_card needs a unit.")?;
    let left = number(args, "left").ok_or("add_card needs a left.")?;
    let top = number(args, "top").ok_or("add_card needs a top.")?;
    let directory = crate::packs_routes::directory(app);
    let unit = yunari_packs::installed(&directory)
        .into_iter()
        .flat_map(|pack| pack.units)
        .find(|unit| unit.id == unit_id)
        .ok_or_else(|| format!("No installed card is named \"{unit_id}\"."))?;
    with_draft(app, args, |draft| {
        if unit.document.nodes.is_empty() {
            return Err(format!("The \"{unit_id}\" card holds nothing to place."));
        }
        let placed = yunari_packs::place(&mut draft.document, &unit, left, top);
        for declaration in &unit.document.assets {
            if let Some(bytes) = unit.assets.get(&declaration.path) {
                draft.assets.insert(declaration.path.clone(), bytes.clone());
            }
        }
        Ok(json!({
            "themeId": draft.document.id,
            "objectIds": placed.ids,
            "missingAssets": placed.missing_assets,
        }))
    })
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

fn add_icon(app: &Shared, args: &Value) -> Result<Value, String> {
    let name = text(args, "icon").ok_or("add_icon needs an icon.")?;
    let transform = transform(args)?;
    let size = transform.width.min(transform.height);
    let glyph = yunari_render::icons::glyph(&name, size).ok_or_else(|| {
        format!(
            "There is no icon \"{name}\" in the shared icon set ({}).",
            yunari_render::icons::names().join(", ")
        )
    })?;
    let token = text(args, "token").unwrap_or_else(|| "text".to_owned());
    let mut style = yunari_model::document::StyleMap::new();
    style.insert("stroke".to_owned(), style_ref("palette", &token));
    style.insert(
        "strokeWidth".to_owned(),
        style_literal(Value::from(glyph.stroke_width)),
    );
    with_draft(app, args, |draft| {
        let node = Node {
            id: free_id(&draft.document, "icon"),
            name: text(args, "name"),
            kind: NodeKind::Path {
                data: glyph.data.clone(),
                stroke_width: glyph.stroke_width,
                closed: false,
            },
            transform: Transform {
                width: size,
                height: size,
                ..transform
            },
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

/// The sources this host can search: the two local libraries and every stock
/// source it serves in-process.
fn list_asset_sources() -> Result<Value, String> {
    let mut sources = vec![
        json!({
            "id": "pictures",
            "label": "This PC's Pictures",
            "category": "image",
            "body": "grid",
            "requires": null,
            "configurable": false
        }),
        json!({
            "id": "wallpaper-engine",
            "label": "Wallpaper Engine",
            "category": "image",
            "body": "wallpaper-engine",
            "requires": null,
            "configurable": false
        }),
    ];
    for stock in crate::assets::STOCKS {
        sources.push(json!({
            "id": stock.id,
            "label": stock.label,
            "category": stock.category,
            "body": stock.body,
            "requires": stock.requires,
            "presetKind": stock.preset,
            "configurable": false,
        }));
    }
    Ok(json!({"sources": sources}))
}

/// An opaque, hand-carryable reference to one finder row: base64 JSON, as the
/// source's own pointer, never a file path a caller can forge past the source
/// it names.
fn encode_ref(reference: &Value) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(reference.to_string())
}

fn asset_reference(source: &str, id: &str, name: &str, kind: &str, licence: &str) -> String {
    encode_ref(&json!({
        "source": source,
        "id": id,
        "name": name,
        "kind": kind,
        "licence": licence,
    }))
}

fn decode_asset_ref(value: &str) -> Result<Value, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| "That assetRef is not base64 JSON.".to_owned())?;
    let parsed: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "That assetRef is not an asset reference.".to_owned())?;
    let field = |key: &str| parsed.get(key).and_then(Value::as_str);
    if field("source").is_none() || field("id").is_none() {
        return Err("That assetRef has no source or id.".to_owned());
    }
    Ok(parsed)
}

/// The Pictures folder, folded, so a ref can only name a file below it.
fn pictures_root() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    std::fs::canonicalize(std::path::PathBuf::from(home).join("Pictures")).ok()
}

fn pictures() -> Vec<(String, std::path::PathBuf)> {
    let Some(root) = pictures_root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut rows: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg"
                        )
                    })
        })
        .take(200)
        .map(|path| {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name, path)
        })
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

/// One row's bytes, the name it declares under, and the media type the host
/// already knows it by.
struct Fetched {
    name: String,
    bytes: Vec<u8>,
    mime: Option<&'static str>,
}

/// The secret a keyed source needs, from this host's own store. The agent
/// never holds it.
fn reference_credential(app: &Shared, source: &str) -> Option<String> {
    crate::assets::stock(source)
        .and_then(|stock| stock.requires)
        .and_then(|id| app.credentials.get(id))
}

/// The bytes a finder reference stands for. A forged or moved reference is a
/// sentence, never a read.
fn reference_bytes(app: &Shared, reference: &Value) -> Result<Fetched, String> {
    let source = reference
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = reference
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if crate::assets::stock(source).is_some() {
        let credential = reference_credential(app, source);
        let (bytes, mime, name) = crate::assets::bytes_with(source, id, credential.as_deref())
            .ok_or_else(|| {
            "That row is a preset, not a file this host can import.".to_owned()
        })??;
        return Ok(Fetched {
            name,
            bytes,
            mime: Some(mime),
        });
    }
    match source {
        "pictures" => {
            let root = pictures_root().ok_or("This PC has no Pictures folder.")?;
            let path = std::fs::canonicalize(id).map_err(|_| NOT_A_PICTURE.to_owned())?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err("That file is not in this PC's Pictures folder.".to_owned());
            }
            let meta = std::fs::metadata(&path).map_err(|_| NOT_A_PICTURE.to_owned())?;
            if meta.len() > MAX_ASSET_BYTES {
                return Err("That file is too large to import.".to_owned());
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "picture".to_owned());
            let bytes = std::fs::read(&path).map_err(|_| NOT_A_PICTURE.to_owned())?;
            Ok(Fetched {
                name,
                bytes,
                mime: None,
            })
        }
        "wallpaper-engine" => {
            let (bytes, _) = crate::wallpaper_engine::bytes(id, true)
                .ok_or("That wallpaper has no preview to import.")?;
            Ok(Fetched {
                name: format!("wallpaper-{id}.jpg"),
                bytes,
                mime: Some("image/jpeg"),
            })
        }
        _ => Err("That assetRef names a source this host does not serve.".to_owned()),
    }
}

/// A stock row as the agent carries it: everything a later import needs to
/// declare the asset travels in the opaque ref.
fn stock_row(source: &str, result: &Value) -> Value {
    let field = |key: &str| result.get(key).and_then(Value::as_str).unwrap_or_default();
    let id = field("id");
    let name = field("name");
    let kind = field("kind");
    let licence = field("licence");
    let source_url = field("sourceUrl");
    let cut = result
        .get("font")
        .and_then(|font| font.get("cuts"))
        .and_then(Value::as_array)
        .and_then(|cuts| cuts.first());
    let preview = match field("preview") {
        "" => cut
            .and_then(|cut| cut.get("previewUrl"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        own => own,
    };
    let font = cut.map(|cut| {
        let mut font = json!({
            "family": cut.get("family").cloned().unwrap_or(Value::Null),
            "weight": cut.get("weight").cloned().unwrap_or(Value::Null),
            "style": cut.get("style").cloned().unwrap_or(Value::Null),
            "format": cut.get("format").cloned().unwrap_or(Value::Null),
        });
        if let Some(subset) = cut.get("subset") {
            font["subset"] = subset.clone();
        }
        font
    });
    let mut reference = json!({
        "source": source,
        "id": id,
        "name": name,
        "kind": kind,
        "preview": preview,
        "licence": licence,
        "sourceUrl": source_url,
    });
    if let Some(subtitle) = result.get("subtitle") {
        reference["subtitle"] = subtitle.clone();
    }
    if let Some(font) = &font {
        reference["font"] = font.clone();
    }
    let mut row = json!({
        "assetRef": encode_ref(&reference),
        "id": format!("{source}:{id}"),
        "name": name,
        "kind": kind,
        "preview": preview,
        "licence": licence,
        "sourceUrl": source_url,
    });
    for key in ["subtitle", "font"] {
        if let Some(value) = reference.get(key) {
            row[key] = value.clone();
        }
    }
    row
}

fn search_assets(app: &Shared, args: &Value) -> Result<Value, String> {
    let query = text(args, "query").ok_or_else(|| "search_assets needs a query.".to_owned())?;
    let folded = query.to_ascii_lowercase();
    let page = args.get("page").and_then(Value::as_u64).unwrap_or(1).max(1) as u32;
    let enabled: Vec<String> = args
        .get("sourceIds")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| {
            let mut ids = vec!["pictures".to_owned(), "wallpaper-engine".to_owned()];
            ids.extend(
                crate::assets::STOCKS
                    .iter()
                    .map(|stock| stock.id.to_owned()),
            );
            ids
        });
    let matches =
        |name: &str| folded.is_empty() || name.to_ascii_lowercase().contains(folded.as_str());

    let mut pages = Vec::new();
    if enabled.iter().any(|id| id == "pictures") {
        let results: Vec<Value> = pictures()
            .into_iter()
            .filter(|(name, _)| matches(name))
            .map(|(name, path)| {
                let id = path.display().to_string();
                json!({
                    "id": format!("pictures:{id}"),
                    "source": "pictures",
                    "name": name,
                    "kind": "image",
                    "subtitle": "Pictures folder",
                    "assetRef": asset_reference("pictures", &id, &name, "image", "Own work"),
                })
            })
            .collect();
        pages.push(json!({"source": "pictures", "results": results, "totalPages": 1}));
    }
    if enabled.iter().any(|id| id == "wallpaper-engine") {
        let mut results = Vec::new();
        if let Some(install) = crate::wallpaper_engine::locate() {
            for item in crate::wallpaper_engine::scan(&install).items {
                if !matches(&item.title) {
                    continue;
                }
                results.push(json!({
                    "id": format!("wallpaper-engine:{}", item.workshop_id),
                    "source": "wallpaper-engine",
                    "name": item.title,
                    "kind": item.kind.as_str(),
                    "subtitle": "Steam Workshop",
                    "previewUrl": format!(
                        "/api/wallpaper-engine/item/{}/preview",
                        item.workshop_id
                    ),
                    "assetRef": asset_reference(
                        "wallpaper-engine",
                        &item.workshop_id,
                        &item.title,
                        item.kind.as_str(),
                        "Steam Workshop",
                    ),
                }));
            }
        }
        pages.push(json!({
            "source": "wallpaper-engine",
            "results": results,
            "totalPages": 1,
        }));
    }
    for id in &enabled {
        let Some(stock) = crate::assets::stock(id) else {
            continue;
        };
        // A listing-only body has no search of its own.
        if stock.body == "wallpaper-engine" {
            continue;
        }
        let credential = reference_credential(app, id);
        match crate::assets::search_with(id, &query, page, credential.as_deref()) {
            Ok(answer) => pages.push(json!({
                "source": stock.id,
                "results": answer
                    .results
                    .iter()
                    .map(|result| stock_row(stock.id, result))
                    .collect::<Vec<_>>(),
                "totalPages": answer.total_pages,
            })),
            // One source that cannot answer is one reported page, not a
            // failed search: the others this call asked for still arrive.
            Err(message) => pages.push(json!({
                "source": stock.id,
                "results": [],
                "totalPages": 1,
                "failure": {
                    "reason": "failed",
                    "message": message,
                    "credential": stock.requires,
                },
            })),
        }
    }
    Ok(json!({"pages": pages}))
}

/// One finder row's picture as MCP image content. A format no image content
/// type carries is a sentence rather than a wrong label.
fn preview_asset(app: &Shared, args: &Value) -> Result<Value, String> {
    use base64::Engine;
    let encoded =
        text(args, "assetRef").ok_or_else(|| "preview_asset needs an assetRef.".to_owned())?;
    let reference = decode_asset_ref(&encoded)?;
    let kind = reference
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = reference
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    // A font or a video is not a picture, so no fetch is spent finding that
    // out.
    if !matches!(kind, "image" | "svg" | "gif") {
        return Err(format!(
            "\"{name}\" is a {kind}, not an image this tool can preview."
        ));
    }
    let fetched = reference_bytes(app, &reference)?;
    let mime = match fetched.mime {
        Some(mime) if mime.starts_with("image/") => mime,
        _ => {
            let (extension, _) =
                picture_kind(&fetched.name, &fetched.bytes).ok_or(NOT_A_PICTURE)?;
            match extension {
                "png" => "image/png",
                "jpg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "svg" => "image/svg+xml",
                _ => return Err("That picture has no preview this surface can carry.".to_owned()),
            }
        }
    };
    Ok(json!({
        "content": [{
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(&fetched.bytes),
            "mimeType": mime,
        }],
        "isError": false,
    }))
}

/// The object that still names an asset, or nothing.
fn asset_owner(document: &ThemeDocument, asset_id: &str) -> Option<String> {
    if document
        .background
        .media
        .as_ref()
        .is_some_and(|media| media.asset_id == asset_id)
    {
        return Some("the background".to_owned());
    }
    if let Some((name, _)) = document.type_tokens.iter().find(|(_, token)| {
        token
            .face
            .as_ref()
            .is_some_and(|face| face.asset_id == asset_id)
    }) {
        return Some(format!("type token {name}"));
    }
    fn walk(nodes: &[Node], asset_id: &str) -> Option<String> {
        for node in nodes {
            match &node.kind {
                NodeKind::Image { content } if content.asset_id == asset_id => {
                    return Some(format!("object {}", node.id));
                }
                NodeKind::Video { content } if content.asset_id == asset_id => {
                    return Some(format!("object {}", node.id));
                }
                _ => {}
            }
            if let Some(owner) = walk(&node.children, asset_id) {
                return Some(owner);
            }
        }
        None
    }
    walk(&document.nodes, asset_id)
}

fn remove_asset(app: &Shared, args: &Value) -> Result<Value, String> {
    let asset_id = text(args, "assetId").ok_or("remove_asset needs an assetId.")?;
    with_draft(app, args, |draft| {
        let Some(asset) = draft
            .document
            .assets
            .iter()
            .find(|asset| asset.id == asset_id)
            .cloned()
        else {
            return Err(format!("There is no asset {asset_id} in this theme."));
        };
        if let Some(owner) = asset_owner(&draft.document, &asset_id) {
            return Err(format!(
                "{asset_id} is still used by {owner}; remove that reference first."
            ));
        }
        draft.document.assets.retain(|held| held.id != asset_id);
        draft.assets.remove(&asset.path);
        draft.credits = draft
            .credits
            .as_ref()
            .map(|credits| credits.without(&asset_id));
        Ok(json!({
            "themeId": draft.document.id,
            "removed": asset_id,
            "path": asset.path,
        }))
    })
}

/// The attribution one import writes.
struct CreditFacts {
    kind: String,
    title: String,
    licence: String,
    source_url: Option<String>,
    author: Option<String>,
    workshop_id: Option<String>,
}

impl CreditFacts {
    fn entry(&self, asset_id: &str) -> CreditsEntry {
        CreditsEntry {
            kind: self.kind.clone(),
            title: self.title.clone(),
            author: self.author.clone().map(|name| CreditsAuthor {
                name: Some(name),
                steam_id: None,
            }),
            source: (self.source_url.is_some() || self.workshop_id.is_some()).then(|| {
                CreditsSource {
                    url: self.source_url.clone(),
                    workshop_id: self.workshop_id.clone(),
                }
            }),
            license: self.licence.clone(),
            changes: "Imported unchanged".to_owned(),
            asset_id: Some(asset_id.to_owned()),
        }
    }
}

/// The facts a finder ref carries, already checked by the source it came from.
fn credit_of_ref(reference: &Value, fetched: &Fetched) -> CreditFacts {
    let field = |key: &str| {
        reference
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let kind = field("kind").unwrap_or_else(|| "image".to_owned());
    CreditFacts {
        // An image row's subtitle is its photographer; an icon row's is a
        // style, not a person.
        author: (kind == "image").then(|| field("subtitle")).flatten(),
        workshop_id: (field("source").as_deref() == Some("wallpaper-engine"))
            .then(|| field("id"))
            .flatten(),
        title: field("name").unwrap_or_else(|| fetched.name.clone()),
        licence: field("licence").unwrap_or_else(|| "Unknown".to_owned()),
        source_url: field("sourceUrl"),
        kind,
    }
}

/// Adds one credit to a draft, replacing an earlier one for the same asset.
fn credit_draft(draft: &mut Draft, entry: CreditsEntry) -> Result<(), String> {
    let credits = match draft.credits.take() {
        Some(held) => held.merged(entry)?,
        None => ThemeCredits::new(Vec::new()).merged(entry)?,
    };
    draft.credits = Some(credits);
    Ok(())
}

/// A font row's face, as the declaration needs it. A row that cannot name its
/// face is refused rather than declared half a face.
fn import_font(
    app: &Shared,
    args: &Value,
    fetched: Fetched,
    reference: &Value,
) -> Result<Value, String> {
    use yunari_model::document::{
        AssetKind, AssetLicense, AssetReference, FontFormat, FontStyle, TypeWeight,
    };

    let font = reference
        .get("font")
        .ok_or("That row declares no font face.")?;
    let family = font
        .get("family")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .ok_or("That font row has no family.")?;
    let weight = match font.get("weight") {
        Some(Value::Number(weight)) => weight.as_f64(),
        Some(Value::String(weight)) => weight.parse::<f64>().ok(),
        _ => None,
    }
    .ok_or("That font row has no usable weight.")?;
    let style = match font.get("style").and_then(Value::as_str) {
        Some("normal") => FontStyle::Normal,
        Some("italic") => FontStyle::Italic,
        _ => return Err("That font row has no usable style.".to_owned()),
    };
    let format = match font.get("format").and_then(Value::as_str) {
        Some("woff2") => FontFormat::Woff2,
        Some("woff") => FontFormat::Woff,
        Some("ttf") => FontFormat::Ttf,
        Some("otf") => FontFormat::Otf,
        _ => return Err("That font row has no usable format.".to_owned()),
    };
    let field = |key: &str| {
        reference
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let licence = field("licence");
    let source_url = field("sourceUrl");
    let facts = CreditFacts {
        kind: "font".to_owned(),
        title: family.to_owned(),
        licence: licence.clone().unwrap_or_else(|| "Unknown".to_owned()),
        source_url: source_url.clone(),
        author: None,
        workshop_id: None,
    };
    let subset = font
        .get("subset")
        .and_then(Value::as_str)
        .map(str::to_owned);
    with_draft(app, args, |draft| {
        let stem = asset_stem(family);
        let mut id = stem.clone();
        let mut n = 2;
        while draft.document.assets.iter().any(|asset| asset.id == id) {
            id = format!("{stem}-{n}");
            n += 1;
        }
        let extension = format.extension();
        let asset_path = format!("assets/{id}.{extension}");
        let reference = AssetReference {
            id: id.clone(),
            kind: AssetKind::Font,
            path: asset_path.clone(),
            sha256: Some(yunari_library::sha256_hex(&fetched.bytes)),
            source_url: source_url.clone(),
            license: licence.map(|name| AssetLicense {
                name: Some(name),
                url: source_url,
                attribution: None,
            }),
            family: Some(family.to_owned()),
            weight: Some(TypeWeight::Number(weight)),
            style: Some(style),
            format: Some(format),
            subset,
        };
        draft.document.assets.push(reference);
        draft.assets.insert(asset_path, fetched.bytes);
        credit_draft(draft, facts.entry(&id))?;
        Ok(json!({
            "themeId": draft.document.id,
            "assetId": id,
            "kind": "font",
            "family": family,
            "format": extension,
        }))
    })
}

fn import_asset(app: &Shared, args: &Value) -> Result<Value, String> {
    use yunari_model::document::{AssetKind, AssetReference};

    let (fetched, reference) = match (text(args, "assetRef"), text(args, "path")) {
        (Some(encoded), _) => {
            let decoded = decode_asset_ref(&encoded)?;
            (reference_bytes(app, &decoded)?, Some(decoded))
        }
        (None, Some(path)) => {
            if text(args, "license").is_none() {
                return Err(format!(
                    "A local import needs the licence the user stated for \"{path}\"."
                ));
            }
            let meta = std::fs::metadata(&path).map_err(|error| format!("{path}: {error}"))?;
            if meta.len() > MAX_ASSET_BYTES {
                return Err("That file is too large to import.".to_owned());
            }
            let bytes = std::fs::read(&path).map_err(|error| format!("{path}: {error}"))?;
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_owned());
            (
                Fetched {
                    name,
                    bytes,
                    mime: None,
                },
                None,
            )
        }
        (None, None) => return Err("import_asset needs an assetRef or a path.".to_owned()),
    };
    if fetched.bytes.len() as u64 > MAX_ASSET_BYTES {
        return Err("That file is too large to import.".to_owned());
    }
    if fetched.bytes.is_empty() {
        return Err("That file is empty.".to_owned());
    }
    if let Some(reference) = reference
        .as_ref()
        .filter(|reference| reference.get("font").is_some())
    {
        return import_font(app, args, fetched, reference);
    }
    let Some((extension, kind)) = picture_kind(&fetched.name, &fetched.bytes) else {
        return Err(NOT_A_PICTURE.to_owned());
    };
    if kind == "svg" {
        let text = std::str::from_utf8(&fetched.bytes).map_err(|_| NOT_A_PICTURE.to_owned())?;
        if !svg_is_safe(text) {
            return Err(
                "That SVG carries a script or an outside reference, so it was not imported."
                    .to_owned(),
            );
        }
    }
    let kind_name = kind;
    let kind = match kind {
        "gif" => AssetKind::Gif,
        "svg" => AssetKind::Svg,
        _ => AssetKind::Image,
    };
    let facts = match &reference {
        Some(reference) => credit_of_ref(reference, &fetched),
        None => CreditFacts {
            kind: kind_name.to_owned(),
            title: fetched.name.clone(),
            licence: text(args, "license").unwrap_or_else(|| "Unknown".to_owned()),
            source_url: None,
            author: text(args, "author"),
            workshop_id: None,
        },
    };
    with_draft(app, args, |draft| {
        let stem = asset_stem(&fetched.name);
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
            sha256: Some(yunari_library::sha256_hex(&fetched.bytes)),
            source_url: facts.source_url.clone(),
            license: Some(yunari_model::document::AssetLicense {
                name: Some(facts.licence.clone()),
                url: facts.source_url.clone(),
                attribution: None,
            }),
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        };
        draft.document.assets.push(reference);
        draft.assets.insert(asset_path, fetched.bytes);
        credit_draft(draft, facts.entry(&id))?;
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

fn assign_theme(app: &Shared, args: &Value) -> Result<Value, String> {
    let id = text(args, "id").ok_or("assign_theme needs a theme id.")?;
    if app.library.read(&id).is_err() {
        return Err(format!("There is no stored theme \"{id}\"."));
    }
    let display =
        text(args, "displayId").unwrap_or_else(|| crate::displays::DEFAULT_DISPLAY.to_owned());
    let mut assignment = app
        .assignment
        .lock()
        .map_err(|_| "The host is not ready.".to_owned())?;
    assignment.set(&display, Some(&id));
    Ok(json!({"displayId": display, "theme": id}))
}

fn preview_theme(app: &Shared, args: &Value) -> Result<Value, String> {
    // A player reads a stored theme by id, so a preview saves first.
    let saved = save_theme(app, args)?;
    let id = saved["id"]
        .as_str()
        .ok_or("The save gave no id.")?
        .to_owned();
    let (artboard, bound) = with_read(app, args, |draft| {
        let bound: Vec<String> = draft
            .document
            .bindings
            .values()
            .flatten()
            .map(|binding| binding.semantic_key.clone())
            .collect();
        Ok((draft.document.artboard.clone(), bound))
    })?;
    let answering: Vec<String> = app
        .registry
        .describe()
        .into_iter()
        .map(|descriptor| descriptor.semantic_key)
        .collect();
    let missing_sensors: Vec<String> = bound
        .iter()
        .filter(|key| !answering.iter().any(|available| available == *key))
        .cloned()
        .collect();
    let display = {
        let store = app.displays.lock().map_err(|_| NO_DRAFT.to_owned())?;
        match text(args, "displayId") {
            Some(wanted) => store
                .get(&wanted)
                .cloned()
                .ok_or_else(|| format!("There is no display \"{wanted}\" on this host."))?,
            None => store
                .list()
                .first()
                .cloned()
                .ok_or("This host has no display to preview on.")?,
        }
    };
    if display.kind != crate::displays::DisplayKind::Panel {
        let port = app.bound_port.load(std::sync::atomic::Ordering::Relaxed);
        let mut url = format!("http://127.0.0.1:{port}/?theme={id}");
        if text(args, "displayId").is_some() {
            url.push_str(&format!("&display={}", display.id));
        }
        return Ok(json!({
            "mode": "browser",
            "themeId": id,
            "displayId": display.id,
            "url": url,
            "viewport": viewport(&artboard),
            "shape": artboard.shape,
            "boundSensors": bound,
            "missingSensors": missing_sensors,
        }));
    }
    let family = display
        .device
        .as_ref()
        .and_then(|device| yunari_panels::for_device(device.vendor_id, device.product_id));
    let Some(family) = family else {
        return Err("No driver on this host claims that panel.".to_owned());
    };
    let record = crate::panels::manager_record(&display);
    let png = app
        .panel_frames
        .preview_of(&id, &record)
        .map_err(|error| error.to_string())?;
    use base64::Engine;
    Ok(json!({
        "mode": "panel",
        "themeId": id,
        "display": {"id": display.id, "label": display.label},
        "geometry": {"width": family.geometry.width, "height": family.geometry.height},
        "shape": match family.shape {
            yunari_panels::PanelShape::Rect => "rect",
            yunari_panels::PanelShape::Circle => "circle",
        },
        "mimeType": "image/png",
        "png": base64::engine::general_purpose::STANDARD.encode(&png),
        "missingSensors": missing_sensors,
    }))
}

/// A browser preview's viewport: the artboard's aspect, its long side capped
/// at 1600, and a square for a round board.
fn viewport(artboard: &yunari_model::document::Artboard) -> Value {
    let long = artboard.width.max(artboard.height);
    let capped = long.min(1600.0);
    if artboard.shape == yunari_model::document::ArtboardShape::Circle {
        let side = capped.round().max(1.0);
        return json!({"width": side, "height": side});
    }
    let scale = if long > 0.0 { capped / long } else { 1.0 };
    json!({
        "width": (artboard.width * scale).round().max(1.0),
        "height": (artboard.height * scale).round().max(1.0),
    })
}

fn yunari_status(app: &Shared) -> Result<Value, String> {
    let drafts = app.drafts.lock().map_err(|_| NO_DRAFT.to_owned())?;
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "port": app.bound_port.load(std::sync::atomic::Ordering::Relaxed),
        "themes": app.library.list().len(),
        "displays": app.displays.lock().map(|store| store.list().len()).unwrap_or_default(),
        "sensors": app.registry.describe().len(),
        "assetSources": ["pictures", "wallpaper-engine"],
        "drafts": drafts.ids(),
        "providers": app.registry.status().into_iter().map(|(id, label, health)| {
            json!({"id": id, "label": label, "reason": health.reason()})
        }).collect::<Vec<_>>(),
    }))
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
    fn an_installed_card_lands_at_the_corner_the_call_names() {
        let app = app();
        yunari_packs::install(&crate::packs_routes::directory(&app), yunari_packs::STARTER)
            .expect("the starter pack");
        let theme_id = draft_id(&app);
        let placed = call(
            &app,
            "add_card",
            &json!({"themeId": theme_id, "unit": "card", "left": 240, "top": 90}),
        )
        .expect("a tool")
        .expect("a placed card");
        let object_ids = placed["objectIds"].as_array().expect("ids");
        assert_eq!(object_ids.len(), 1);
        let object_id = object_ids[0].as_str().expect("an id").to_owned();
        let drafts = app.drafts.lock().expect("drafts");
        let draft = drafts.draft(Some(&theme_id)).expect("the draft");
        let root = draft
            .document
            .nodes
            .iter()
            .find(|node| node.id == object_id)
            .expect("the placed card");
        assert_eq!(
            (root.transform.x, root.transform.y),
            (240.0, 90.0),
            "the card's corner is where the call put it"
        );
        assert!(!root.children.is_empty(), "the card keeps its rows");
        let carried: Vec<&str> = draft
            .document
            .bindings
            .values()
            .flatten()
            .map(|binding| binding.semantic_key.as_str())
            .collect();
        assert!(
            carried.contains(&"cpu.load"),
            "the card's readings travel with it: {carried:?}"
        );
        drop(drafts);
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn an_icon_lands_as_path_data_and_an_unknown_name_is_refused() {
        let app = app();
        let theme_id = draft_id(&app);
        let placed = call(
            &app,
            "add_icon",
            &json!({
                "themeId": theme_id,
                "icon": "cpu",
                "box": {"left": 40, "top": 50, "width": 48, "height": 48}
            }),
        )
        .expect("a tool")
        .expect("a glyph");
        let object_id = placed["objectId"].as_str().expect("an id").to_owned();
        let drafts = app.drafts.lock().expect("drafts");
        let draft = drafts.draft(Some(&theme_id)).expect("the draft");
        let node = draft
            .document
            .nodes
            .iter()
            .find(|node| node.id == object_id)
            .expect("the glyph");
        let NodeKind::Path {
            data,
            stroke_width,
            closed,
        } = &node.kind
        else {
            panic!("an icon is a path");
        };
        assert!(data.starts_with('M'), "{data}");
        assert!(!closed);
        assert!((*stroke_width - 4.0).abs() < 1e-9, "{stroke_width}");
        assert_eq!((node.transform.width, node.transform.height), (48.0, 48.0));
        drop(drafts);
        let refused = call(
            &app,
            "add_icon",
            &json!({
                "themeId": theme_id,
                "icon": "no-such-icon",
                "box": {"left": 0, "top": 0, "width": 24, "height": 24}
            }),
        )
        .expect("a tool")
        .expect_err("an unknown glyph");
        assert!(refused.contains("shared icon set"), "{refused}");
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn a_preview_saves_first_and_answers_a_player_url() {
        let app = app();
        let display = app
            .displays
            .lock()
            .expect("displays")
            .adopt_local("Desk", None, None, None)
            .expect("a display");
        let theme_id = draft_id(&app);
        let preview = call(
            &app,
            "preview_theme",
            &json!({"themeId": theme_id, "displayId": display.id}),
        )
        .expect("a tool")
        .expect("a preview");
        assert_eq!(preview["mode"], "browser");
        assert!(app.library.read("boards").is_ok(), "the draft was saved");
        let url = preview["url"].as_str().expect("a url");
        assert!(url.contains("?theme=boards"), "{url}");
        assert!(url.contains(&format!("&display={}", display.id)), "{url}");
        assert_eq!(preview["viewport"]["width"], 1600.0);
        assert_eq!(preview["viewport"]["height"], 900.0);
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
            &json!({"themeId": theme_id, "path": source, "license": "Own work"}),
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

    #[test]
    fn an_asset_reference_is_opaque_and_cannot_name_a_file_outside_its_source() {
        let app = app();
        let reference = asset_reference(
            "pictures",
            "C:/Windows/win.ini",
            "win.ini",
            "image",
            "Own work",
        );
        let decoded = decode_asset_ref(&reference).expect("a reference");
        assert_eq!(decoded["source"], "pictures");
        assert!(reference_bytes(&app, &decoded).is_err());
        assert!(decode_asset_ref("not base64").is_err());
        assert!(decode_asset_ref("aGVsbG8=").is_err());
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn an_asset_in_use_is_kept_and_a_free_one_is_removed() {
        let app = app();
        let theme_id = draft_id(&app);
        let source = app.config.settings_dir().join("dot.png");
        std::fs::write(&source, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0]).unwrap();
        let imported = call(
            &app,
            "import_asset",
            &json!({"themeId": theme_id, "path": source.display().to_string(), "license": "Own work"}),
        )
        .expect("a tool")
        .expect("an asset");
        let asset_id = imported["assetId"].as_str().expect("an id").to_owned();
        let image = call(
            &app,
            "add_image",
            &json!({
                "themeId": theme_id,
                "assetId": asset_id,
                "box": {"left": 0, "top": 0, "width": 10, "height": 10},
            }),
        )
        .expect("a tool")
        .expect("an image");
        let refused = call(
            &app,
            "remove_asset",
            &json!({"themeId": theme_id, "assetId": asset_id}),
        )
        .expect("a tool")
        .unwrap_err();
        assert!(refused.contains("still used"), "{refused}");
        call(
            &app,
            "remove_object",
            &json!({"themeId": theme_id, "objectId": image["objectId"]}),
        )
        .expect("a tool")
        .expect("a removal");
        let removed = call(
            &app,
            "remove_asset",
            &json!({"themeId": theme_id, "assetId": asset_id}),
        )
        .expect("a tool")
        .expect("a freed asset");
        assert_eq!(removed["removed"], asset_id);
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn the_source_list_names_every_stock_this_host_serves() {
        let sources = call(&app(), "list_asset_sources", &json!({}))
            .expect("a tool")
            .expect("a list")["sources"]
            .as_array()
            .expect("sources")
            .clone();
        let names: Vec<&str> = sources
            .iter()
            .map(|source| source["id"].as_str().unwrap_or_default())
            .collect();
        assert!(names.contains(&"pictures"));
        assert!(names.contains(&"curated-fonts"));
        assert!(names.contains(&"google-fonts"));
        let unsplash = sources
            .iter()
            .find(|source| source["id"] == "unsplash")
            .expect("Unsplash");
        assert_eq!(unsplash["requires"], "unsplash");
        assert_eq!(unsplash["category"], "image");
        let curated = sources
            .iter()
            .find(|source| source["id"] == "curated-fonts")
            .expect("the curated catalogue");
        assert_eq!(curated["body"], "font-picker");
    }

    #[test]
    fn a_stock_search_carries_a_ref_a_font_import_can_declare() {
        let app = app();
        let answer = call(
            &app,
            "search_assets",
            &json!({"query": "fira code", "sourceIds": ["curated-fonts"]}),
        )
        .expect("a tool")
        .expect("a page");
        let pages = answer["pages"].as_array().expect("pages");
        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        assert_eq!(page["source"], "curated-fonts");
        let rows = page["results"].as_array().expect("rows");
        let row = rows
            .iter()
            .find(|row| row["name"] == "Fira Code")
            .expect("the Fira Code family");
        assert!(
            row["id"]
                .as_str()
                .unwrap()
                .starts_with("curated-fonts:cut:")
        );
        assert_eq!(row["kind"], "font");
        let decoded = decode_asset_ref(row["assetRef"].as_str().expect("a ref")).expect("a ref");
        assert_eq!(decoded["source"], "curated-fonts");
        assert_eq!(decoded["font"]["family"], "Fira Code");
        assert_eq!(decoded["font"]["format"], "woff2");
        assert!(decoded["sourceUrl"].as_str().unwrap().ends_with(".woff2"));

        // The declaration itself needs no network: the bytes are supplied.
        let theme_id = draft_id(&app);
        let imported = import_font(
            &app,
            &json!({"themeId": theme_id}),
            Fetched {
                name: "fira-code.woff2".to_owned(),
                bytes: vec![1, 2, 3],
                mime: Some("font/woff2"),
            },
            &decoded,
        )
        .expect("a font declaration");
        assert_eq!(imported["kind"], "font");
        assert_eq!(imported["format"], "woff2");
        let draft = app.drafts.lock().unwrap();
        let asset = draft
            .draft(Some(&theme_id))
            .expect("the draft")
            .document
            .assets
            .iter()
            .find(|asset| asset.id == imported["assetId"].as_str().unwrap())
            .expect("the asset");
        assert_eq!(asset.kind, yunari_model::document::AssetKind::Font);
        assert_eq!(asset.family.as_deref(), Some("Fira Code"));
        assert_eq!(asset.path, format!("assets/{}.woff2", asset.id));
        drop(draft);
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn a_keyed_source_without_its_key_is_a_failure_page_not_a_failed_search() {
        let app = app();
        let answer = call(
            &app,
            "search_assets",
            &json!({"query": "mountain", "sourceIds": ["unsplash"]}),
        )
        .expect("a tool")
        .expect("a page");
        let page = &answer["pages"][0];
        assert_eq!(page["source"], "unsplash");
        assert!(page["results"].as_array().unwrap().is_empty());
        assert_eq!(page["failure"]["credential"], "unsplash");
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }

    #[test]
    fn a_font_row_has_no_picture_to_preview() {
        let app = app();
        let result = crate::assets::search("curated-fonts", "", 1)
            .expect("the built-in catalogue")
            .results
            .into_iter()
            .next()
            .expect("a row");
        let row = stock_row("curated-fonts", &result);
        let refused = call(&app, "preview_asset", &json!({"assetRef": row["assetRef"]}))
            .expect("a tool")
            .unwrap_err();
        assert!(refused.contains("not an image"), "{refused}");
        std::fs::remove_dir_all(app.config.settings_dir()).ok();
    }
}
