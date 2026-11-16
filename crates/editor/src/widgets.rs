//! Inserting a widget unit into the open theme.
//!
//! A unit arrives as its own scene, so insertion is three merges: the tokens
//! its styles and text runs name, the assets its nodes reference (with their
//! bytes), and the nodes themselves with fresh ids. Nothing mutates the pack.

use std::collections::BTreeMap;

use yunari_model::document::{Node, NodeKind};
use yunari_packs::Unit;

use crate::clipboard::{self, Clipboard};
use crate::document::Editor;

pub const EMPTY_UNIT_MESSAGE: &str = "That unit holds nothing this editor can place.";

/// Inserts one unit's scene, offset from where the package put it. Returns the
/// id of its top-level node.
pub fn insert(editor: &mut Editor, unit: &Unit, offset: f64) -> Result<String, String> {
    if unit.document.nodes.is_empty() {
        return Err(EMPTY_UNIT_MESSAGE.to_owned());
    }
    let asset_map = asset_ids(editor, unit);
    let mut clipboard = Clipboard {
        nodes: unit.document.nodes.clone(),
        bindings: unit.document.bindings.clone(),
        ..Clipboard::default()
    };
    for node in &mut clipboard.nodes {
        remap_assets(node, &asset_map);
    }

    let mut inserted = None;
    editor.edit(|document| {
        for (id, value) in &unit.document.palette {
            document
                .palette
                .entry(id.clone())
                .or_insert_with(|| value.clone());
        }
        for (id, token) in &unit.document.type_tokens {
            document
                .type_tokens
                .entry(id.clone())
                .or_insert_with(|| token.clone());
        }
        for asset in &unit.document.assets {
            let Some(fresh) = asset_map.get(&asset.id) else {
                continue;
            };
            if document.assets.iter().any(|held| held.id == *fresh) {
                continue;
            }
            let mut declaration = asset.clone();
            declaration.id = fresh.clone();
            document.assets.push(declaration);
        }
        let pasted = clipboard::insert(document, &clipboard, offset);
        inserted = pasted.ids.first().cloned();
    });
    for declaration in &unit.document.assets {
        if let Some(bytes) = unit.assets.get(&declaration.path) {
            editor.put_asset(declaration.path.clone(), bytes.clone());
        }
    }
    inserted.ok_or_else(|| EMPTY_UNIT_MESSAGE.to_owned())
}

/// The id each package asset takes in this document: the one already holding
/// the same path, or a fresh id when the document has never seen it.
fn asset_ids(editor: &Editor, unit: &Unit) -> BTreeMap<String, String> {
    let document = editor.document();
    let mut mapping = BTreeMap::new();
    let mut taken: Vec<String> = document.assets.iter().map(|a| a.id.clone()).collect();
    for asset in &unit.document.assets {
        if let Some(held) = document.assets.iter().find(|held| held.path == asset.path) {
            mapping.insert(asset.id.clone(), held.id.clone());
            continue;
        }
        let mut id = asset.id.clone();
        let mut counter = 2;
        while taken.contains(&id) {
            id = format!("{}-{counter}", asset.id);
            counter += 1;
        }
        taken.push(id.clone());
        mapping.insert(asset.id.clone(), id);
    }
    mapping
}

fn remap_assets(node: &mut Node, mapping: &BTreeMap<String, String>) {
    match &mut node.kind {
        NodeKind::Image { content } => {
            if let Some(fresh) = mapping.get(&content.asset_id) {
                content.asset_id = fresh.clone();
            }
        }
        NodeKind::Video { content } => {
            if let Some(fresh) = mapping.get(&content.asset_id) {
                content.asset_id = fresh.clone();
            }
        }
        _ => {}
    }
    for child in &mut node.children {
        remap_assets(child, mapping);
    }
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
        AssetKind, AssetReference, Binding, ImageContent, ImageFit, NodeKind, Transform,
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
