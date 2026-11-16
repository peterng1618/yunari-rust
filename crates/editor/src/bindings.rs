//! Bindings: what a value run reads, and the key that names the reading.
//!
//! A binding lives under the node whose subtree names it. Its semantic key is
//! the document format's (`family.quantity[.qualifier]`), so an edit is refused
//! here rather than saved as an invalid theme.

use yunari_model::document::{Binding, Node, NodeKind, TextRun, ThemeDocument};
use yunari_model::validate::semantic_key_refusal;

use crate::document::Editor;

/// The binding that a value run in this node's subtree names, if any.
pub fn referenced_by(document: &ThemeDocument, binding_id: &str) -> Option<String> {
    document
        .nodes
        .iter()
        .find_map(|node| find_holder(node, binding_id))
        .map(|node| format!("A run in {} reads {binding_id}", node.id))
}

fn find_holder<'a>(node: &'a Node, binding_id: &str) -> Option<&'a Node> {
    if let NodeKind::Text { content } = &node.kind {
        for run in &content.runs {
            if let TextRun::Value {
                binding_id: held, ..
            } = run
                && held == binding_id
            {
                return Some(node);
            }
        }
    }
    node.children
        .iter()
        .find_map(|child| find_holder(child, binding_id))
}

fn empty_binding(id: String, semantic_key: String) -> Binding {
    Binding {
        id,
        semantic_key,
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

impl Editor {
    /// Adds a binding to a node's map under a free `binding-<n>` id.
    pub fn add_binding(&mut self, node_id: &str, key: &str) -> Result<String, String> {
        if crate::selection::find(self.document(), node_id).is_none() {
            return Err("No node with that id".to_owned());
        }
        let key = key.trim();
        if let Some(refusal) = semantic_key_refusal(key) {
            return Err(refusal.to_owned());
        }
        let taken: Vec<String> = self
            .document()
            .bindings
            .values()
            .flatten()
            .map(|binding| binding.id.clone())
            .collect();
        let mut number = 1;
        let id = loop {
            let candidate = format!("binding-{number}");
            number += 1;
            if !taken.contains(&candidate) {
                break candidate;
            }
        };
        let id_for_edit = id.clone();
        let (node_id, key) = (node_id.to_owned(), key.to_owned());
        self.edit(|document| {
            document
                .bindings
                .entry(node_id)
                .or_default()
                .push(empty_binding(id_for_edit, key));
        });
        Ok(id)
    }

    /// Writes one binding field on a node. A refused value changes nothing.
    pub fn set_binding(
        &mut self,
        node_id: &str,
        binding_id: &str,
        field: &str,
        value: &str,
    ) -> Result<(), String> {
        let (field, value) = (field.to_owned(), value.to_owned());
        let mut outcome = Ok(());
        self.edit(|document| {
            outcome =
                crate::inspector::apply_binding(document, node_id, binding_id, &field, &value);
        });
        outcome
    }

    /// Removes a binding no run reads, so a save never names a vanished reading.
    pub fn remove_binding(&mut self, node_id: &str, binding_id: &str) -> Result<(), String> {
        if let Some(holder) = referenced_by(self.document(), binding_id) {
            return Err(format!("{holder} and keeps it"));
        }
        let (node_id, binding_id) = (node_id.to_owned(), binding_id.to_owned());
        let mut found = false;
        self.edit(|document| {
            if let Some(held) = document.bindings.get_mut(&node_id) {
                let before = held.len();
                held.retain(|binding| binding.id != binding_id);
                found = held.len() != before;
                if held.is_empty() {
                    document.bindings.remove(&node_id);
                }
            }
        });
        if found {
            Ok(())
        } else {
            Err("No binding with that id on this node".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::{TextContent, VerticalAlign};

    use crate::fixtures::{document, rect};

    use super::*;

    fn value_run_node(id: &str, binding: &str) -> Node {
        let mut node = rect(id, 0.0, 0.0, 100.0, 30.0);
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

    #[test]
    fn a_key_is_checked_against_the_format() {
        let mut editor = Editor::new(document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]));
        assert!(editor.add_binding("a", "CPU Usage").is_err());
        assert!(editor.add_binding("missing", "cpu.load").is_err());
        let id = editor.add_binding("a", "cpu.load").expect("a binding");
        assert_eq!(id, "binding-1");
        assert_eq!(editor.document().bindings["a"][0].semantic_key, "cpu.load");
        assert_eq!(
            editor.add_binding("a", "gpu.temp").as_deref(),
            Ok("binding-2")
        );
    }

    #[test]
    fn a_referenced_binding_is_not_removed() {
        let mut editor = Editor::new(document(vec![value_run_node("label", "binding-1")]));
        let id = editor.add_binding("label", "cpu.load").expect("a binding");
        assert!(editor.remove_binding("label", &id).is_err());
        editor.edit(|document| {
            // Point the run at nothing that exists: the reference is gone.
            if let NodeKind::Text { content } = &mut document.nodes[0].kind
                && let TextRun::Value { binding_id, .. } = &mut content.runs[0]
            {
                *binding_id = "elsewhere".to_owned();
            }
        });
        assert!(editor.remove_binding("label", &id).is_ok());
        assert!(editor.document().bindings.is_empty());
    }

    #[test]
    fn binding_fields_validate_precision_and_unit() {
        let mut editor = Editor::new(document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]));
        let id = editor.add_binding("a", "cpu.load").expect("a binding");
        assert!(editor.set_binding("a", &id, "unit", "%").is_ok());
        assert!(editor.set_binding("a", &id, "precision", "2").is_ok());
        let binding = &editor.document().bindings["a"][0];
        assert_eq!(binding.unit.as_deref(), Some("%"));
        assert_eq!(binding.precision, Some(2));
        assert!(editor.set_binding("a", &id, "precision", "7").is_err());
        assert!(editor.set_binding("a", &id, "precision", "one").is_err());
        assert!(editor.set_binding("a", &id, "key", "nonsense").is_err());
        assert!(editor.set_binding("a", &id, "key", "gpu.load").is_ok());
    }
}
