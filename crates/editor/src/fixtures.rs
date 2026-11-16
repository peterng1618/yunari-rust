//! Fixtures shared by the editor's tests.

use std::collections::BTreeMap;

use yunari_model::document::{
    Artboard, ArtboardShape, Background, ContentFit, Metadata, Node, NodeKind, StyleMap,
    ThemeDocument, Transform,
};

pub fn document(nodes: Vec<Node>) -> ThemeDocument {
    ThemeDocument {
        schema_version: "1.0.0".to_owned(),
        id: "test".to_owned(),
        metadata: Metadata::default(),
        artboard: Artboard {
            width: 450.0,
            height: 675.0,
            content_fit: ContentFit::Contain,
            shape: ArtboardShape::Rect,
        },
        background: Background::default(),
        palette: BTreeMap::new(),
        type_tokens: BTreeMap::new(),
        assets: Vec::new(),
        nodes,
        bindings: BTreeMap::new(),
        editor_metadata: BTreeMap::new(),
    }
}

pub fn rect(id: &str, x: f64, y: f64, width: f64, height: f64) -> Node {
    Node {
        id: id.to_owned(),
        name: None,
        kind: NodeKind::Rectangle { corner_radius: 0.0 },
        transform: Transform {
            x,
            y,
            width,
            height,
            ..Transform::default()
        },
        visible: true,
        locked: false,
        style: StyleMap::new(),
        glass: None,
        children: Vec::new(),
    }
}

pub fn group(id: &str, children: Vec<Node>) -> Node {
    let mut group = rect(id, 0.0, 0.0, 100.0, 100.0);
    group.kind = NodeKind::Group;
    group.children = children;
    group
}
