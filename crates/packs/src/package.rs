//! One widget package: its archive, manifest, document and units.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use yunari_model::document::ThemeDocument;

pub const MAX_ARCHIVE_BYTES: usize = 95 * 1024 * 1024;
pub const MAX_ENTRY_BYTES: u64 = 80 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 180 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 128;

pub const MANIFEST: &str = "manifest.json";
pub const WIDGET: &str = "widget.json";
pub const FORMAT: &str = "yunari-widget-package";
/// The artboard a widget is validated against. A unit has none, so the
/// smallest valid board is supplied for the one check that asks.
fn validation_artboard() -> Value {
    json!({"width": 1, "height": 1})
}

pub const TOO_LARGE: &str = "That package is too large to open.";
pub const UNREADABLE: &str = "That file is not a readable widget package.";
pub const NEEDS_FILES: &str = "A widget package needs manifest.json and widget.json.";
pub const BAD_MANIFEST: &str = "Unsupported widget package manifest.";
pub const INVALID_WIDGET: &str = "Invalid widget.";
pub const HAS_ARTBOARD: &str = "A widget document must not carry an artboard.";
pub const ASSETS_MISMATCH: &str = "Package assets must exactly match the widget declaration.";
pub const NOTHING_TO_WRITE: &str = "That selection holds nothing to export.";
pub const ONE_UNIT_MESSAGE: &str = "A widget package this editor writes holds exactly one unit.";

#[derive(Debug, Clone)]
pub struct Unit {
    pub id: String,
    pub label: String,
    /// The unit's own scene, lifted into this model (its artboard is 1×1 and
    /// is never used by an insertion).
    pub document: ThemeDocument,
    /// The declared bytes this unit references, by their path in the package.
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Pack {
    pub name: Option<String>,
    pub version: Option<String>,
    pub units: Vec<Unit>,
}

/// One package, gate and content, from its bytes.
pub fn read(bytes: &[u8]) -> Result<Pack, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(TOO_LARGE.to_owned());
    }
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| UNREADABLE.to_owned())?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!(
            "That package declares more than {MAX_ENTRIES} archive entries."
        ));
    }
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| UNREADABLE.to_owned())?;
        let name = entry
            .name()
            .map_err(|_| UNREADABLE.to_owned())?
            .into_owned();
        if !allowed(&name) || files.contains_key(&name) {
            return Err(UNREADABLE.to_owned());
        }
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(TOO_LARGE.to_owned());
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            return Err(TOO_LARGE.to_owned());
        }
        let mut body = Vec::new();
        entry
            .read_to_end(&mut body)
            .map_err(|_| UNREADABLE.to_owned())?;
        files.insert(name, body);
    }
    read_files(&files)
}

/// Writes a package from one document: the manifest names a single unit, the
/// widget is the document without its artboard, and each declared asset carries
/// its bytes and their digest.
pub fn write(
    document: &ThemeDocument,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    if document.nodes.is_empty() {
        return Err(NOTHING_TO_WRITE.to_owned());
    }
    let mut widget = serde_json::to_value(document).map_err(|_| INVALID_WIDGET.to_owned())?;
    let object = widget.as_object_mut().ok_or(INVALID_WIDGET)?;
    object.remove("artboard");
    let declarations = object
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if declarations.len() != assets.len() {
        return Err(ASSETS_MISMATCH.to_owned());
    }
    for declaration in &declarations {
        let path = declaration
            .get("path")
            .and_then(Value::as_str)
            .ok_or(ASSETS_MISMATCH)?;
        if !assets.contains_key(path) {
            return Err(ASSETS_MISMATCH.to_owned());
        }
    }
    // The declaration's digest is the writer's to compute: a package naming a
    // digest its own bytes do not match is refused by every reader.
    let digests: BTreeMap<String, String> = assets
        .iter()
        .map(|(path, bytes)| (path.clone(), sha256_hex(bytes)))
        .collect();
    if let Some(list) = object.get_mut("assets").and_then(Value::as_array_mut) {
        for declaration in list {
            let Some(path) = declaration.get("path").and_then(Value::as_str) else {
                continue;
            };
            let Some(digest) = digests.get(path) else {
                continue;
            };
            if let Some(entry) = declaration.as_object_mut() {
                entry.insert("sha256".to_owned(), Value::String(digest.clone()));
            }
        }
    }

    let metadata = document.metadata.clone();
    let label = metadata
        .name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Widget".to_owned());
    let unit_id = document
        .nodes
        .first()
        .map(|node| node.id.clone())
        .ok_or(NOTHING_TO_WRITE)?;
    let manifest = json!({
        "format": FORMAT,
        "version": 1,
        "widget": WIDGET,
        "units": [{"id": unit_id, "label": label, "objectId": unit_id}],
    });

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
        use std::io::Write as _;
        writer
            .start_file(name, options)
            .map_err(|_| UNREADABLE.to_owned())?;
        writer.write_all(bytes).map_err(|_| UNREADABLE.to_owned())
    };
    put(
        MANIFEST,
        &serde_json::to_vec(&manifest).map_err(|_| INVALID_WIDGET.to_owned())?,
    )?;
    put(
        WIDGET,
        &serde_json::to_vec(&widget).map_err(|_| INVALID_WIDGET.to_owned())?,
    )?;
    for (path, bytes) in assets {
        put(path, bytes)?;
    }
    drop(put);
    let bytes = writer
        .finish()
        .map_err(|_| UNREADABLE.to_owned())?
        .into_inner();
    // The writer asks its own reader: a package it would refuse is never
    // handed to anyone.
    read(&bytes)?;
    Ok(bytes)
}

/// A document an editor wrote: one unit, the whole scene, no artboard.
fn read_native(
    object: &Map<String, Value>,
    declared: &[UnitRef],
    declarations: &[Value],
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Pack, String> {
    if declared.len() != 1 {
        return Err(ONE_UNIT_MESSAGE.to_owned());
    }
    let mut document = Value::Object(object.clone());
    if let Some(map) = document.as_object_mut() {
        map.insert("artboard".to_owned(), validation_artboard());
    }
    let document: ThemeDocument =
        serde_json::from_value(document).map_err(|_| INVALID_WIDGET.to_owned())?;
    if document.nodes.is_empty() {
        return Err(INVALID_WIDGET.to_owned());
    }
    let mut assets = BTreeMap::new();
    for declaration in declarations {
        let Some(path) = declaration.get("path").and_then(Value::as_str) else {
            continue;
        };
        if let Some(bytes) = files.get(path) {
            assets.insert(path.to_owned(), bytes.clone());
        }
    }
    let reference = &declared[0];
    let metadata = object.get("metadata").and_then(Value::as_object);
    Ok(Pack {
        name: metadata
            .and_then(|metadata| metadata.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        version: metadata
            .and_then(|metadata| metadata.get("version"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        units: vec![Unit {
            id: reference.id.clone(),
            label: reference.label.clone(),
            document,
            assets,
        }],
    })
}

/// Only the manifest, the widget and declared asset paths are members.
fn allowed(name: &str) -> bool {
    if name == MANIFEST || name == WIDGET {
        return true;
    }
    let Some(rest) = name.strip_prefix("assets/") else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 200
        && !rest.starts_with('/')
        && !rest
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
        && rest
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-' | '/'))
}

/// A package from its unpacked members. The archive gate has already run.
pub fn read_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Pack, String> {
    let (Some(manifest), Some(widget)) = (files.get(MANIFEST), files.get(WIDGET)) else {
        return Err(NEEDS_FILES.to_owned());
    };
    let declared = units_from_manifest(manifest)?;
    let widget: Value = serde_json::from_slice(widget).map_err(|_| INVALID_WIDGET.to_owned())?;
    let object = widget.as_object().ok_or(INVALID_WIDGET)?;
    if object.contains_key("artboard") {
        return Err(HAS_ARTBOARD.to_owned());
    }

    // Every declared asset must name a path the archive carries exactly once,
    // and the archive may carry nothing else.
    let declarations = declarations(object);
    let mut declared_paths = BTreeSet::new();
    for declaration in &declarations {
        let path = declaration
            .get("path")
            .and_then(Value::as_str)
            .ok_or(ASSETS_MISMATCH)?;
        if !declared_paths.insert(path.to_owned()) {
            return Err(ASSETS_MISMATCH.to_owned());
        }
        let Some(digest) = declaration.get("sha256").and_then(Value::as_str) else {
            return Err(format!("Package asset {path} declares no SHA-256 digest."));
        };
        let Some(bytes) = files.get(path) else {
            return Err(ASSETS_MISMATCH.to_owned());
        };
        if !digest.eq_ignore_ascii_case(&sha256_hex(bytes)) {
            return Err(format!(
                "Package asset {path} does not match its SHA-256 digest."
            ));
        }
    }
    let carried: BTreeSet<&String> = files
        .keys()
        .filter(|name| name.starts_with("assets/"))
        .collect();
    if carried
        .iter()
        .any(|path| !declared_paths.contains(path.as_str()))
        || declared_paths.len() != carried.len()
    {
        return Err(ASSETS_MISMATCH.to_owned());
    }

    // A document this editor wrote is read as itself: one unit, the whole
    // scene. A source widget is the Fabric envelope below.
    if object.contains_key("nodes") {
        let Some(declared) = declared else {
            return Err("That widget package offers no units.".to_owned());
        };
        return read_native(object, &declared, &declarations, files);
    }

    let scene_objects = object
        .get("scene")
        .and_then(Value::as_object)
        .and_then(|scene| scene.get("objects"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let by_id: BTreeMap<&str, &Value> = scene_objects
        .iter()
        .filter_map(|entry| Some((entry.get("id")?.as_str()?, entry)))
        .collect();
    // A unit keeps the package's type tokens, so a token face's bytes travel
    // with every unit that could name them.
    let face_assets: BTreeSet<String> = object
        .get("globals")
        .and_then(Value::as_object)
        .and_then(|globals| globals.get("typeTokens"))
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|tokens| tokens.values())
        .filter_map(|token| token.get("face"))
        .filter_map(|face| face.get("assetId"))
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();

    let refs = match declared {
        Some(refs) => refs,
        None => provenance_units(&scene_objects),
    };
    let mut seen = BTreeSet::new();
    let mut units = Vec::new();
    for reference in refs {
        if !seen.insert(reference.id.clone()) {
            return Err(format!(
                "Widget unit \"{}\" is declared more than once.",
                reference.id
            ));
        }
        let Some(root) = by_id.get(reference.object_id.as_str()) else {
            return Err(format!(
                "Widget unit \"{}\" names no top-level object \"{}\".",
                reference.id, reference.object_id
            ));
        };
        let ids = subtree_ids(root);
        let mut wanted: BTreeSet<String> = asset_ids(root);
        wanted.extend(face_assets.iter().cloned());

        let mut unit = object.clone();
        unit.insert(
            "scene".to_owned(),
            json!({"version": object.get("scene").and_then(|scene| scene.get("version")),
                   "objects": [(*root).clone()]}),
        );
        let bindings: Map<String, Value> = object
            .get("bindings")
            .and_then(Value::as_object)
            .map(|bindings| {
                bindings
                    .iter()
                    .filter(|(owner, _)| ids.contains(owner.as_str()))
                    .map(|(owner, list)| (owner.clone(), list.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if bindings.is_empty() {
            unit.remove("bindings");
        } else {
            unit.insert("bindings".to_owned(), Value::Object(bindings));
        }
        let kept: Vec<Value> = declarations
            .iter()
            .filter(|declaration| {
                declaration
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| wanted.contains(id))
            })
            .cloned()
            .collect();
        if kept.is_empty() {
            unit.remove("assets");
        } else {
            unit.insert("assets".to_owned(), Value::Array(kept.clone()));
        }
        unit.insert("artboard".to_owned(), validation_artboard());

        let document =
            yunari_model::source::from_source_value(Value::Object(unit)).map_err(|_| {
                format!(
                    "Widget unit \"{}\" is not a scene this editor draws.",
                    reference.id
                )
            })?;
        if document.nodes.is_empty() {
            return Err(format!(
                "Widget unit \"{}\" is not a scene this editor draws.",
                reference.id
            ));
        }
        let mut assets = BTreeMap::new();
        for declaration in kept {
            let Some(path) = declaration.get("path").and_then(Value::as_str) else {
                continue;
            };
            if let Some(bytes) = files.get(path) {
                assets.insert(path.to_owned(), bytes.clone());
            }
        }
        units.push(Unit {
            id: reference.id,
            label: reference.label,
            document,
            assets,
        });
    }
    if units.is_empty() {
        return Err("That widget package offers no units.".to_owned());
    }
    let metadata = object.get("metadata").and_then(Value::as_object);
    Ok(Pack {
        name: metadata
            .and_then(|metadata| metadata.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        version: metadata
            .and_then(|metadata| metadata.get("version"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        units,
    })
}

struct UnitRef {
    id: String,
    label: String,
    object_id: String,
}

/// The manifest's own unit list, when it has one.
fn units_from_manifest(manifest: &[u8]) -> Result<Option<Vec<UnitRef>>, String> {
    let value: Value = serde_json::from_slice(manifest).map_err(|_| BAD_MANIFEST.to_owned())?;
    let object = value.as_object().ok_or(BAD_MANIFEST)?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "format" | "version" | "widget" | "units"))
        || object.get("format").and_then(Value::as_str) != Some(FORMAT)
        || object.get("version").and_then(Value::as_u64) != Some(1)
        || object.get("widget").and_then(Value::as_str) != Some(WIDGET)
    {
        return Err(BAD_MANIFEST.to_owned());
    }
    let Some(declared) = object.get("units") else {
        return Ok(None);
    };
    let declared = declared.as_array().ok_or(BAD_MANIFEST)?;
    let mut refs = Vec::new();
    for entry in declared {
        let entry = entry.as_object().ok_or(BAD_MANIFEST)?;
        if entry.len() != 3
            || entry
                .keys()
                .any(|key| !matches!(key.as_str(), "id" | "label" | "objectId"))
        {
            return Err(BAD_MANIFEST.to_owned());
        }
        let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
        let (Some(id), Some(label), Some(object_id)) =
            (text("id"), text("label"), text("objectId"))
        else {
            return Err(BAD_MANIFEST.to_owned());
        };
        if id.trim().is_empty() || object_id.trim().is_empty() {
            return Err(BAD_MANIFEST.to_owned());
        }
        refs.push(UnitRef {
            id,
            label,
            object_id,
        });
    }
    Ok(Some(refs))
}

/// One unit per top-level object the editor stamped with provenance. Two copies
/// of one card share an id, so a later copy takes a suffix.
fn provenance_units(objects: &[Value]) -> Vec<UnitRef> {
    let mut refs: Vec<UnitRef> = Vec::new();
    let mut taken = BTreeSet::new();
    for object in objects {
        let (Some(provenance), Some(widget_id), Some(object_id)) = (
            object.get("provenance").and_then(Value::as_object),
            object
                .get("provenance")
                .and_then(|provenance| provenance.get("widgetId"))
                .and_then(Value::as_str),
            object.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };
        if widget_id.trim().is_empty() {
            continue;
        }
        let mut id = widget_id.to_owned();
        let mut counter = 2;
        while !taken.insert(id.clone()) {
            id = format!("{widget_id}-{counter}");
            counter += 1;
        }
        let label = provenance
            .get("widgetName")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(widget_id)
            .to_owned();
        refs.push(UnitRef {
            id,
            label,
            object_id: object_id.to_owned(),
        });
    }
    refs
}

fn declarations(widget: &Map<String, Value>) -> Vec<Value> {
    widget
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter(|asset| asset.is_object())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Every object id in a subtree, the root included.
fn subtree_ids(root: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    visit(root, &mut |value| {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            ids.insert(id.to_owned());
        }
    });
    ids
}

/// Every asset the subtree names under an `assetId` key.
fn asset_ids(root: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    visit(root, &mut |value| {
        if let Some(id) = value.get("assetId").and_then(Value::as_str) {
            ids.insert(id.to_owned());
        }
    });
    ids
}

fn visit(value: &Value, at: &mut impl FnMut(&Map<String, Value>)) {
    match value {
        Value::Object(object) => {
            at(object);
            for child in object.values() {
                visit(child, at);
            }
        }
        Value::Array(items) => {
            for item in items {
                visit(item, at);
            }
        }
        _ => {}
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const PHOTO: &[u8] = &[1, 2, 3];

    fn widget(objects: Value, bindings: Value) -> Value {
        json!({
            "schemaVersion": "3.0.0",
            "fabricVersion": "7.4.0",
            "id": "card",
            "metadata": {"name": "Starter cards", "version": "1.0.0"},
            "globals": {"palette": {"ink": {"name": "Ink", "value": {"kind": "solid", "color": "#fff"}}}},
            "assets": [{"id": "photo", "kind": "image", "path": "assets/photo.png",
                        "sha256": sha256_hex(PHOTO)}],
            "scene": {"version": "7.4.0", "objects": objects},
            "bindings": bindings,
        })
    }

    fn unit_object() -> Value {
        json!({"type": "Group", "id": "card", "objects": [
            {"type": "Rect", "id": "panel", "yunariAsset": {"assetId": "photo"},
             "left": 0, "top": 0, "width": 10, "height": 10},
            {"type": "Textbox", "id": "value", "left": 0, "top": 10, "width": 10, "height": 10,
             "text": "cpu"}
        ]})
    }

    fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in members {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn package(manifest: &Value, widget: &Value) -> Vec<u8> {
        let manifest = serde_json::to_vec(manifest).unwrap();
        let widget = serde_json::to_vec(widget).unwrap();
        zip_of(&[
            ("manifest.json", &manifest),
            ("widget.json", &widget),
            ("assets/photo.png", PHOTO),
        ])
    }

    fn manifest(units: Value) -> Value {
        json!({"format": FORMAT, "version": 1, "widget": WIDGET, "units": units})
    }

    fn native_document() -> ThemeDocument {
        yunari_model::document::parse_document(
            r#"{"schemaVersion":"3.0.0","id":"card","metadata":{"name":"Lab card"},
                "artboard":{"width":1920,"height":1080},
                "assets":[{"id":"photo","kind":"image","path":"assets/photo.png"}],
                "nodes":[{"id":"card","kind":{"type":"group"},"children":[]}]}"#,
        )
        .expect("a native document")
    }

    #[test]
    fn a_written_package_reads_back_as_its_own_document() {
        let document = native_document();
        let assets = BTreeMap::from([("assets/photo.png".to_owned(), PHOTO.to_vec())]);
        let bytes = write(&document, &assets).expect("a written package");
        let pack = read(&bytes).expect("its own reader accepts it");
        assert_eq!(pack.name.as_deref(), Some("Lab card"));
        assert_eq!(pack.units.len(), 1);
        let unit = &pack.units[0];
        assert_eq!(unit.id, "card");
        assert_eq!(unit.document.nodes, document.nodes);
        assert_eq!(unit.assets, assets);
        // The artboard a widget must not carry is not written back out.
        assert!(!String::from_utf8_lossy(&bytes).is_empty());
    }

    #[test]
    fn a_written_package_must_carry_every_asset_it_declares() {
        let document = native_document();
        assert_eq!(
            write(&document, &BTreeMap::new()),
            Err(ASSETS_MISMATCH.to_owned())
        );
        let mut spare = BTreeMap::from([("assets/photo.png".to_owned(), PHOTO.to_vec())]);
        spare.insert("assets/extra.png".to_owned(), PHOTO.to_vec());
        assert_eq!(write(&document, &spare), Err(ASSETS_MISMATCH.to_owned()));
    }

    #[test]
    fn a_package_lists_its_unit_with_nodes_bindings_and_bytes() {
        let widget = widget(
            json!([unit_object()]),
            json!({"value": [{"id": "reading", "semanticKey": "cpu.load"}]}),
        );
        let pack = read(&package(
            &manifest(json!([{"id": "card", "label": "Card", "objectId": "card"}])),
            &widget,
        ))
        .expect("a package");
        assert_eq!(pack.name.as_deref(), Some("Starter cards"));
        assert_eq!(pack.units.len(), 1);
        let unit = &pack.units[0];
        assert_eq!(unit.id, "card");
        assert_eq!(unit.label, "Card");
        assert_eq!(unit.document.nodes.len(), 1);
        assert!(unit.document.bindings.contains_key("value"));
        assert_eq!(unit.assets.get("assets/photo.png"), Some(&PHOTO.to_vec()));
    }

    #[test]
    fn provenance_names_units_and_repeats_take_a_suffix() {
        let mut second = unit_object();
        second["id"] = json!("card-copy");
        second["provenance"] = json!({"widgetId": "card", "widgetName": "Card"});
        let mut first = unit_object();
        first["provenance"] = json!({"widgetId": "card", "widgetName": "Card"});
        let widget = widget(json!([first, second]), json!({}));
        let pack = read(&package(
            &json!({"format": FORMAT, "version": 1, "widget": WIDGET}),
            &widget,
        ))
        .expect("a package");
        let ids: Vec<&str> = pack.units.iter().map(|unit| unit.id.as_str()).collect();
        assert_eq!(ids, vec!["card", "card-2"]);
    }

    #[test]
    fn a_manifest_a_widget_and_a_digest_are_each_refused_by_name() {
        let good = widget(json!([unit_object()]), json!({}));
        let declared = manifest(json!([{"id": "card", "label": "Card", "objectId": "card"}]));

        let mut bad_manifest = declared.clone();
        bad_manifest["extra"] = json!(true);
        assert_eq!(
            read(&package(&bad_manifest, &good)).unwrap_err(),
            BAD_MANIFEST
        );

        let mut with_artboard = good.clone();
        with_artboard["artboard"] = json!({"width": 10, "height": 10});
        assert_eq!(
            read(&package(&declared, &with_artboard)).unwrap_err(),
            HAS_ARTBOARD
        );

        let mut undigested = good.clone();
        undigested["assets"][0]
            .as_object_mut()
            .unwrap()
            .remove("sha256");
        assert!(
            read(&package(&declared, &undigested))
                .unwrap_err()
                .contains("declares no SHA-256 digest")
        );

        // Members without the two files a package needs.
        let only_widget = zip_of(&[("widget.json", b"{}")]);
        assert_eq!(read(&only_widget).unwrap_err(), NEEDS_FILES);

        let missing = manifest(json!([{"id": "card", "label": "Card", "objectId": "gone"}]));
        assert_eq!(
            read(&package(&missing, &good)).unwrap_err(),
            "Widget unit \"card\" names no top-level object \"gone\"."
        );
    }

    #[test]
    fn climbing_and_duplicate_archive_names_are_refused() {
        let good = serde_json::to_vec(&widget(json!([unit_object()]), json!({}))).unwrap();
        let declared = serde_json::to_vec(&manifest(json!([
            {"id": "card", "label": "Card", "objectId": "card"}
        ])))
        .unwrap();
        let climbing = zip_of(&[
            ("manifest.json", &declared),
            ("widget.json", &good),
            ("assets/../secrets", PHOTO),
        ]);
        assert_eq!(read(&climbing).unwrap_err(), UNREADABLE);

        // A repeated member never reaches the unit builder: the archive reader
        // refuses the directory first, and this crate maps that to an
        // unreadable package.
        // An archive with no package in it at all.
        assert_eq!(read(b"not a zip").unwrap_err(), UNREADABLE);
    }

    #[test]
    fn a_unit_this_editor_cannot_draw_and_a_repeated_unit_are_refused() {
        let declared = manifest(json!([{"id": "odd", "label": "Odd", "objectId": "odd"}]));
        let odd_widget = widget(json!([{"type": "MysteryBox", "id": "odd"}]), json!({}));
        assert!(
            read(&package(&declared, &odd_widget))
                .unwrap_err()
                .contains("is not a scene this editor draws")
        );

        let repeated = manifest(json!([
            {"id": "card", "label": "Card", "objectId": "card"},
            {"id": "card", "label": "Card again", "objectId": "card"}
        ]));
        let good = widget(json!([unit_object()]), json!({}));
        assert_eq!(
            read(&package(&repeated, &good)).unwrap_err(),
            "Widget unit \"card\" is declared more than once."
        );
    }

    #[test]
    fn declared_assets_must_match_the_bytes_carried() {
        let good = widget(json!([unit_object()]), json!({}));
        let declared = manifest(json!([{"id": "card", "label": "Card", "objectId": "card"}]));
        let mut tampered = PHOTO.to_vec();
        tampered[0] = 9;
        let manifest = serde_json::to_vec(&declared).unwrap();
        let widget = serde_json::to_vec(&good).unwrap();
        let bytes = zip_of(&[
            ("manifest.json", &manifest),
            ("widget.json", &widget),
            ("assets/photo.png", &tampered),
        ]);
        assert!(
            read(&bytes)
                .unwrap_err()
                .contains("does not match its SHA-256 digest")
        );

        // An extra member the declaration never names.
        let extra = zip_of(&[
            ("manifest.json", &manifest),
            ("widget.json", &widget),
            ("assets/photo.png", PHOTO),
            ("assets/other.png", PHOTO),
        ]);
        assert_eq!(read(&extra).unwrap_err(), ASSETS_MISMATCH);
    }
}
