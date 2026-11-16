//! The editor clipboard: copied subtrees carry their bindings with them and
//! paste as fresh nodes, with fresh ids and re-pointed runs.
//!
//! Asset references travel as references, not bytes: a copy pasted into a
//! document that does not declare the asset still pastes and reports the miss.

use std::collections::{BTreeMap, BTreeSet};

use yunari_model::document::{Node, NodeKind, ThemeDocument};

use crate::document::Editor;

/// How far a paste lands from what was copied, in artboard units.
pub const PASTE_OFFSET: f64 = 10.0;

#[derive(Debug, Clone, Default)]
pub struct Clipboard {
    pub nodes: Vec<Node>,
    pub bindings: BTreeMap<String, Vec<yunari_model::document::Binding>>,
    pub assets: BTreeSet<String>,
}

/// What a paste added, and which assets the document cannot show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pasted {
    pub ids: Vec<String>,
    pub missing_assets: Vec<String>,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

impl Editor {
    /// Copies the named top-level nodes and everything that travels with them.
    pub fn copy(&mut self, ids: &[String]) -> bool {
        let Some(clipboard) = pick(self.document(), ids) else {
            return false;
        };
        self.clipboard = Some(clipboard);
        true
    }

    pub fn cut(&mut self, ids: &[String]) -> bool {
        if !self.copy(ids) {
            return false;
        }
        self.remove_nodes(ids)
    }

    pub fn paste(&mut self) -> Option<Pasted> {
        let clipboard = self.clipboard.clone()?;
        if clipboard.is_empty() {
            return None;
        }
        let mut pasted = None;
        self.edit(|document| pasted = Some(insert(document, &clipboard, PASTE_OFFSET)));
        pasted
    }

    /// A copy that is pasted at once and leaves the clipboard alone.
    pub fn duplicate(&mut self, ids: &[String]) -> Option<Pasted> {
        let clipboard = pick(self.document(), ids)?;
        let mut pasted = None;
        self.edit(|document| pasted = Some(insert(document, &clipboard, PASTE_OFFSET)));
        pasted
    }
}

/// The clipboard a set of ids would copy to.
pub(crate) fn pick(document: &ThemeDocument, ids: &[String]) -> Option<Clipboard> {
    let nodes: Vec<Node> = document
        .nodes
        .iter()
        .filter(|node| ids.contains(&node.id))
        .cloned()
        .collect();
    if nodes.is_empty() {
        return None;
    }
    let mut clipboard = Clipboard {
        nodes,
        ..Clipboard::default()
    };
    for node in clipboard.nodes.clone() {
        collect(
            node,
            document,
            &mut clipboard.bindings,
            &mut clipboard.assets,
        );
    }
    Some(clipboard)
}

fn collect(
    node: Node,
    document: &ThemeDocument,
    bindings: &mut BTreeMap<String, Vec<yunari_model::document::Binding>>,
    assets: &mut BTreeSet<String>,
) {
    if let Some(held) = document.bindings.get(&node.id) {
        bindings.insert(node.id.clone(), held.clone());
    }
    if let Some(asset) = asset_of(&node) {
        assets.insert(asset);
    }
    for child in node.children {
        collect(child, document, bindings, assets);
    }
}

fn asset_of(node: &Node) -> Option<String> {
    match &node.kind {
        NodeKind::Image { content } => Some(content.asset_id.clone()),
        NodeKind::Video { content } => Some(content.asset_id.clone()),
        _ => None,
    }
}

/// Puts a copy of the clipboard into the document, offset and with fresh ids.
/// The scene merge lives in `yunari_packs`, so the agent surface's card
/// insertion and an editor paste are the same operation.
pub(crate) fn insert(document: &mut ThemeDocument, clipboard: &Clipboard, offset: f64) -> Pasted {
    let inserted = yunari_packs::insert(
        document,
        &clipboard.nodes,
        &clipboard.bindings,
        offset,
        offset,
    );
    Pasted {
        ids: inserted.ids,
        missing_assets: inserted.missing_assets,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::{
        AssetKind, AssetReference, Binding, ImageContent, ImageFit, TextContent, TextRun,
        VerticalAlign,
    };

    use crate::fixtures::{document, rect};

    use super::*;

    fn text_node(id: &str, binding: &str) -> Node {
        let mut node = rect(id, 20.0, 20.0, 100.0, 30.0);
        node.kind = NodeKind::Text {
            content: TextContent {
                runs: vec![TextRun::Value {
                    binding_id: binding.to_owned(),
                    precision: None,
                    unit_display: None,
                    type_token: "body".to_owned(),
                    style: BTreeMap::new(),
                }],
                box_: None,
                overflow: Default::default(),
                align: Default::default(),
                vertical_align: VerticalAlign::Top,
                vertical: false,
            },
        };
        node
    }

    fn binding(id: &str) -> Binding {
        Binding {
            id: id.to_owned(),
            semantic_key: "clock.time".to_owned(),
            precision: None,
            unit_display: None,
            scale: None,
            offset: None,
            unit: None,
            auto_scale: None,
            format: None,
            time_zone: None,
        }
    }

    #[test]
    fn paste_offsets_by_ten_and_mints_fresh_ids_and_bindings() {
        let mut document = document(vec![text_node("label", "clock")]);
        document
            .bindings
            .insert("label".to_owned(), vec![binding("clock")]);
        let mut editor = Editor::new(document);
        assert!(editor.copy(&["label".to_owned()]));
        let pasted = editor.paste().expect("a paste");
        assert_eq!(pasted.ids.len(), 1);
        assert_ne!(pasted.ids[0], "label");
        let copy = editor
            .document()
            .nodes
            .iter()
            .find(|node| node.id == pasted.ids[0])
            .expect("the copy exists");
        assert_eq!((copy.transform.x, copy.transform.y), (30.0, 30.0));
        let held = editor
            .document()
            .bindings
            .get(&pasted.ids[0])
            .expect("the binding follows the copy");
        assert_eq!(held.len(), 1);
        assert_ne!(held[0].id, "clock");
        let NodeKind::Text { content } = &copy.kind else {
            panic!("still text");
        };
        let TextRun::Value { binding_id, .. } = &content.runs[0] else {
            panic!("still a value run");
        };
        assert_eq!(*binding_id, held[0].id, "the run points at the new binding");
    }

    #[test]
    fn pasting_a_node_whose_asset_is_missing_still_pastes_and_reports_it() {
        let mut image = rect("photo", 0.0, 0.0, 100.0, 100.0);
        image.kind = NodeKind::Image {
            content: ImageContent {
                asset_id: "pic".to_owned(),
                fit: ImageFit::Cover,
                monochrome: None,
            },
        };
        let mut document = document(vec![image]);
        document.assets.push(AssetReference {
            id: "pic".to_owned(),
            kind: AssetKind::Image,
            path: "pic.png".to_owned(),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        });
        let mut editor = Editor::new(document);
        // The copy is pasted after the asset is gone, which is what opening a
        // theme without its pictures looks like.
        assert!(editor.copy(&["photo".to_owned()]));
        editor.edit(|document| document.assets.clear());
        let pasted = editor.paste().expect("a paste");
        assert_eq!(pasted.ids.len(), 1);
        assert_eq!(pasted.missing_assets, ["pic"]);
        assert_eq!(
            editor.document().nodes.len(),
            2,
            "the node is pasted even without its picture"
        );
    }

    #[test]
    fn duplicate_leaves_the_clipboard_alone() {
        let mut editor = Editor::new(document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]));
        assert!(editor.copy(&["a".to_owned()]));
        editor.copy(&["a".to_owned()]);
        let pasted = editor.duplicate(&["a".to_owned()]).expect("a copy");
        assert_eq!(pasted.ids.len(), 1);
        assert!(editor.paste().is_some(), "the clipboard still held a copy");
        assert_eq!(editor.document().nodes.len(), 3);
    }
}
