//! What the author has selected, and what the canvas under a pointer means.
//!
//! Selection is a list of node ids, in the order the author picked them, which
//! is the order arrange operations anchor on. The list is never trusted: every
//! state change that can remove nodes filters it against the document.

use yunari_model::document::{Node, ThemeDocument};
use yunari_render::Affine;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<String>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn first(&self) -> Option<&str> {
        self.ids.first().map(String::as_str)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.iter().any(|held| held == id)
    }

    /// A plain click: this node and nothing else.
    pub fn replace(&mut self, id: impl Into<String>) {
        self.ids = vec![id.into()];
    }

    /// A shift-click: the node joins, or leaves if it was already in.
    pub fn toggle(&mut self, id: &str) {
        match self.ids.iter().position(|held| held == id) {
            Some(at) => {
                self.ids.remove(at);
            }
            None => self.ids.push(id.to_owned()),
        }
    }

    pub fn set(&mut self, ids: Vec<String>) {
        self.ids = ids;
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Drops ids the document no longer has, after a delete, undo or ungroup.
    pub fn retain_existing(&mut self, document: &ThemeDocument) {
        self.ids.retain(|id| find(document, id).is_some());
    }
}

/// The node with this id, anywhere in the tree.
pub fn find<'a>(document: &'a ThemeDocument, id: &str) -> Option<&'a Node> {
    find_in(&document.nodes, id)
}

/// The node with this id, mutable, anywhere in the tree.
pub fn find_mut<'a>(nodes: &'a mut [Node], id: &str) -> Option<&'a mut Node> {
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

pub fn find_in<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_in(&node.children, id) {
            return Some(found);
        }
    }
    None
}

/// The topmost visible node whose drawn box holds the point. Rotation and scale
/// are part of what is drawn, so the test inverts them rather than guessing.
pub fn hit(document: &ThemeDocument, x: f64, y: f64) -> Option<String> {
    document
        .nodes
        .iter()
        .rev()
        .find(|node| node.visible && holds(node, Affine::IDENTITY, x, y))
        .map(|node| node.id.clone())
}

/// Every top-level node whose box lies wholly inside the rectangle and is drawn.
/// Locked nodes are left out: a marquee is a transform gesture's first step.
pub fn marquee(document: &ThemeDocument, rect: (f64, f64, f64, f64)) -> Vec<String> {
    let (left, top) = (rect.0, rect.1);
    let right = rect.0 + rect.2;
    let bottom = rect.1 + rect.3;
    document
        .nodes
        .iter()
        .filter(|node| {
            if !node.visible || node.locked {
                return false;
            }
            let affine = Affine::from_node(&node.transform);
            let width = node.transform.width as f32;
            let height = node.transform.height as f32;
            [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
            .iter()
            .all(|(x, y)| {
                let (x, y) = affine.apply(*x, *y);
                let (x, y) = (f64::from(x), f64::from(y));
                x >= left && x <= right && y >= top && y <= bottom
            })
        })
        .map(|node| node.id.clone())
        .collect()
}

/// A point in a node's own space, given the transform of everything above it.
fn holds(node: &Node, parent: Affine, x: f64, y: f64) -> bool {
    let affine = parent.then(Affine::from_node(&node.transform));
    let Some(inverse) = affine.inverse() else {
        return false;
    };
    let (x, y) = inverse.apply(x as f32, y as f32);
    x >= 0.0 && x <= node.transform.width as f32 && y >= 0.0 && y <= node.transform.height as f32
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::{
        Artboard, ArtboardShape, Background, ContentFit, Metadata, NodeKind, StyleMap, Transform,
    };

    use super::*;

    fn document(nodes: Vec<Node>) -> ThemeDocument {
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

    fn rect(id: &str, x: f64, y: f64, width: f64, height: f64) -> Node {
        Node {
            id: id.to_owned(),
            name: None,
            kind: NodeKind::Rectangle {
                corner_radius: 0.0,
            },
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

    #[test]
    fn shift_click_toggles_and_a_plain_click_replaces() {
        let mut selection = Selection::new();
        selection.replace("a");
        assert_eq!(selection.ids(), ["a"]);
        selection.toggle("b");
        assert_eq!(selection.ids(), ["a", "b"]);
        selection.toggle("a");
        assert_eq!(selection.ids(), ["b"]);
        selection.replace("c");
        assert_eq!(selection.ids(), ["c"]);
    }

    #[test]
    fn a_marquee_takes_only_fully_contained_visible_nodes() {
        let hidden = {
            let mut group = rect("group", 0.0, 0.0, 50.0, 50.0);
            let mut child = rect("child", 10.0, 10.0, 10.0, 10.0);
            child.visible = true;
            group.visible = false;
            group.children.push(child);
            group
        };
        let document = document(vec![
            rect("inside", 10.0, 10.0, 20.0, 20.0),
            rect("partly-out", 90.0, 90.0, 30.0, 30.0),
            hidden,
        ]);
        assert_eq!(marquee(&document, (0.0, 0.0, 100.0, 100.0)), ["inside"]);
    }

    #[test]
    fn a_rotated_node_is_hit_where_it_is_drawn() {
        let mut turned = rect("turned", 0.0, 0.0, 100.0, 100.0);
        turned.transform.rotation = 45.0;
        let document = document(vec![turned]);
        // The centre is inside; the original top-left corner is not, because the
        // square has turned into a diamond that no longer covers it.
        assert_eq!(hit(&document, 50.0, 50.0).as_deref(), Some("turned"));
        assert_eq!(hit(&document, 50.0, 20.0).as_deref(), Some("turned"));
        assert_eq!(hit(&document, 10.0, 10.0), None);
    }

    #[test]
    fn a_click_takes_the_topmost_node_under_it() {
        let document = document(vec![
            rect("under", 0.0, 0.0, 100.0, 100.0),
            rect("over", 50.0, 50.0, 100.0, 100.0),
        ]);
        assert_eq!(hit(&document, 60.0, 60.0).as_deref(), Some("over"));
        assert_eq!(hit(&document, 10.0, 10.0).as_deref(), Some("under"));
        assert_eq!(hit(&document, 500.0, 500.0), None);
    }

    #[test]
    fn a_locked_node_is_still_selectable() {
        let mut locked = rect("locked", 0.0, 0.0, 100.0, 100.0);
        locked.locked = true;
        let document = document(vec![locked]);
        assert_eq!(hit(&document, 10.0, 10.0).as_deref(), Some("locked"));
    }

    #[test]
    fn a_hidden_node_is_not_hit() {
        let mut hidden = rect("hidden", 0.0, 0.0, 100.0, 100.0);
        hidden.visible = false;
        let document = document(vec![hidden]);
        assert_eq!(hit(&document, 50.0, 50.0), None);
    }

    #[test]
    fn selection_after_undo_drops_ids_the_document_no_longer_has() {
        let document = document(vec![rect("kept", 0.0, 0.0, 10.0, 10.0)]);
        let mut selection = Selection::new();
        selection.set(vec!["kept".to_owned(), "gone".to_owned()]);
        selection.retain_existing(&document);
        assert_eq!(selection.ids(), ["kept"]);
    }

    #[test]
    fn a_nested_node_is_found_by_id_but_not_hit_from_the_canvas() {
        let mut group = rect("group", 0.0, 0.0, 50.0, 50.0);
        group.children.push(rect("child", 0.0, 0.0, 10.0, 10.0));
        let document = document(vec![group]);
        assert!(find(&document, "child").is_some());
        // A click lands on the group, which is the object the author sees.
        assert_eq!(hit(&document, 5.0, 5.0).as_deref(), Some("group"));
    }
}
