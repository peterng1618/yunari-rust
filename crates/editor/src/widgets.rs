//! Inserting a widget unit into the open theme.
//!
//! A unit arrives as its own scene, so insertion is three merges: the tokens
//! its styles and text runs name, the assets its nodes reference (with their
//! bytes), and the nodes themselves with fresh ids. Nothing mutates the pack.

use std::collections::BTreeMap;

use yunari_packs::Unit;

use crate::clipboard;
use crate::document::Editor;

pub const EMPTY_UNIT_MESSAGE: &str = "That unit holds nothing this editor can place.";

/// Inserts one unit's scene, offset from where the package put it. Returns the
/// id of its top-level node.
pub fn insert(editor: &mut Editor, unit: &Unit, offset: f64) -> Result<String, String> {
    let root = unit
        .document
        .nodes
        .first()
        .ok_or_else(|| EMPTY_UNIT_MESSAGE.to_owned())?;
    let left = root.transform.x + offset;
    let top = root.transform.y + offset;
    let mut inserted = None;
    editor.edit(|document| inserted = Some(yunari_packs::scene::place(document, unit, left, top)));
    let inserted = inserted.ok_or_else(|| EMPTY_UNIT_MESSAGE.to_owned())?;
    let id = inserted
        .ids
        .first()
        .cloned()
        .ok_or_else(|| EMPTY_UNIT_MESSAGE.to_owned())?;
    for declaration in &unit.document.assets {
        if let Some(bytes) = unit.assets.get(&declaration.path) {
            editor.put_asset(declaration.path.clone(), bytes.clone());
        }
    }
    Ok(id)
}

/// A package holding the selected top-level nodes and what travels with them.
/// The tokens come from the whole theme, because a unit that names one must
/// carry it.
pub fn selection_package(editor: &Editor, ids: &[String]) -> Result<Vec<u8>, String> {
    let document = editor.document();
    let clipboard = clipboard::pick(document, ids).ok_or_else(|| EMPTY_UNIT_MESSAGE.to_owned())?;
    let mut assets = BTreeMap::new();
    let mut missing = Vec::new();
    for declaration in &document.assets {
        if !clipboard.assets.contains(&declaration.id) {
            continue;
        }
        match editor.assets().get(&declaration.path) {
            Some(bytes) => {
                assets.insert(declaration.path.clone(), bytes.clone());
            }
            None => missing.push(declaration.path.clone()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "A declared picture has no bytes to export: {}",
            missing.join(", ")
        ));
    }
    let mut unit = document.clone();
    unit.nodes = clipboard.nodes;
    unit.bindings = clipboard.bindings;
    unit.assets
        .retain(|declaration| clipboard.assets.contains(&declaration.id));
    yunari_packs::write(&unit, &assets)
}

/// The declarations a unit's insertion could not carry, for the status line.
pub fn missing_assets(editor: &Editor, unit: &Unit) -> Vec<String> {
    let document = editor.document();
    let mut missing = Vec::new();
    for asset in &unit.document.assets {
        if !unit.assets.contains_key(&asset.path)
            && !document.assets.iter().any(|held| held.path == asset.path)
        {
            missing.push(asset.path.clone());
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use yunari_model::document::{
        AssetKind, AssetReference, Binding, ImageContent, ImageFit, Node, NodeKind, Transform,
    };

    use crate::fixtures::{document, rect};

    use super::*;

    fn picture_node(id: &str, asset_id: &str) -> Node {
        Node {
            id: id.to_owned(),
            name: None,
            kind: NodeKind::Image {
                content: ImageContent {
                    asset_id: asset_id.to_owned(),
                    fit: ImageFit::Contain,
                    monochrome: None,
                },
            },
            transform: Transform {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 80.0,
                ..Transform::default()
            },
            visible: true,
            locked: false,
            style: Default::default(),
            glass: None,
            children: vec![rect(&format!("{id}-value"), 0.0, 0.0, 10.0, 10.0)],
        }
    }

    fn unit() -> Unit {
        let mut scene = document(Vec::new());
        scene.nodes = vec![picture_node("card", "photo")];
        scene.assets = vec![AssetReference {
            id: "photo".to_owned(),
            kind: AssetKind::Image,
            path: "assets/photo.png".to_owned(),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        }];
        scene.bindings.insert(
            "card-value".to_owned(),
            vec![Binding {
                id: "reading".to_owned(),
                semantic_key: "cpu.load".to_owned(),
                precision: None,
                unit_display: None,
                scale: None,
                offset: None,
                unit: None,
                auto_scale: None,
                format: None,
                time_zone: None,
            }],
        );
        Unit {
            id: "card".to_owned(),
            label: "Card".to_owned(),
            document: scene,
            assets: BTreeMap::from([("assets/photo.png".to_owned(), vec![1, 2, 3])]),
        }
    }

    #[test]
    fn inserting_twice_keeps_ids_apart_and_lands_at_the_offset() {
        let mut editor = Editor::new(document(Vec::new()));
        let unit = unit();
        let first = insert(&mut editor, &unit, 10.0).expect("an insert");
        let second = insert(&mut editor, &unit, 10.0).expect("a second insert");
        assert_ne!(first, second);
        let roots: Vec<&Node> = editor.document().nodes.iter().collect();
        assert_eq!(roots.len(), 2);
        assert_eq!((roots[0].transform.x, roots[0].transform.y), (20.0, 30.0));
        assert_eq!((roots[1].transform.x, roots[1].transform.y), (20.0, 30.0));
        // Each copy's binding is keyed by its own fresh node id.
        assert_eq!(editor.document().bindings.len(), 2);
        // One undo takes the second insertion away again.
        editor.undo();
        assert_eq!(editor.document().nodes.len(), 1);
    }

    #[test]
    fn a_selection_exports_as_a_package_its_own_reader_accepts() {
        let mut scene = document(Vec::new());
        scene.id = "lab".to_owned();
        scene.metadata.name = Some("Lab".to_owned());
        scene.nodes = vec![
            picture_node("card", "photo"),
            rect("other", 0.0, 0.0, 4.0, 4.0),
        ];
        scene.assets = vec![AssetReference {
            id: "photo".to_owned(),
            kind: AssetKind::Image,
            path: "assets/photo.png".to_owned(),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        }];
        let mut editor = Editor::new(scene);
        editor.put_asset("assets/photo.png".to_owned(), vec![9, 8, 7]);
        let bytes = selection_package(&editor, &["card".to_owned()]).expect("an export");
        let pack = yunari_packs::read(&bytes).expect("its own reader");
        assert_eq!(pack.name.as_deref(), Some("Lab"));
        assert_eq!(pack.units.len(), 1);
        let unit = &pack.units[0];
        assert_eq!(unit.document.nodes.len(), 1);
        assert_eq!(unit.document.nodes[0].id, "card");
        assert_eq!(unit.assets.get("assets/photo.png"), Some(&vec![9, 8, 7]));
        // A declared picture with no bytes is refused rather than shipped bare.
        let mut editor = Editor::new(document(Vec::new()));
        editor.edit(|document| {
            document.assets = vec![AssetReference {
                id: "photo".to_owned(),
                kind: AssetKind::Image,
                path: "assets/photo.png".to_owned(),
                sha256: None,
                source_url: None,
                license: None,
                family: None,
                weight: None,
                style: None,
                format: None,
                subset: None,
            }];
            document.nodes = vec![picture_node("card", "photo")];
        });
        let error = selection_package(&editor, &["card".to_owned()]).unwrap_err();
        assert!(error.contains("assets/photo.png"), "{error}");
    }

    #[test]
    fn an_asset_lands_once_with_its_bytes() {
        let mut editor = Editor::new(document(Vec::new()));
        let unit = unit();
        insert(&mut editor, &unit, 10.0).expect("an insert");
        insert(&mut editor, &unit, 10.0).expect("a second insert");
        assert_eq!(editor.document().assets.len(), 1);
        assert_eq!(
            editor.assets().get("assets/photo.png"),
            Some(&vec![1, 2, 3])
        );
        // Both nodes name the single declaration.
        let mut named = Vec::new();
        for node in &editor.document().nodes {
            let NodeKind::Image { content } = &node.kind else {
                continue;
            };
            named.push(content.asset_id.clone());
        }
        assert_eq!(named, vec!["photo".to_owned(), "photo".to_owned()]);
    }
}
