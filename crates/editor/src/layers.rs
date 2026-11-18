//! The layer list: what the author sees in the right-hand panel, and the edits
//! the panel can ask for.
//!
//! The list is topmost-first, which is the reverse of paint order, so the node
//! covering the others reads first. A row's hidden or locked state includes
//! everything above it: a child of a hidden group is hidden.

use std::collections::BTreeSet;

use yunari_model::document::{Node, NodeKind, ThemeDocument};

use crate::document::Editor;
use crate::selection::Selection;

/// Names are the author's; this keeps one from swallowing the panel or a save.
pub const MAX_NAME: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Front,
    Forward,
    Backward,
    Back,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub depth: usize,
    pub kind: &'static str,
    /// The row's role in the source's words: every primitive is a `Shape`, a
    /// chart names itself as a chart whatever its family.
    pub role: &'static str,
    pub has_children: bool,
    pub visible: bool,
    pub locked: bool,
    pub selected: bool,
    pub bound: String,
}

pub fn rows(
    document: &ThemeDocument,
    selection: &Selection,
    expanded: &BTreeSet<String>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    collect(
        &document.nodes,
        0,
        true,
        true,
        selection,
        expanded,
        &mut rows,
    );
    for row in &mut rows {
        row.bound = document
            .bindings
            .get(&row.id)
            .and_then(|bindings| bindings.first())
            .map(|binding| binding.semantic_key.clone())
            .unwrap_or_default();
    }
    rows.push(Row {
        id: "$background".to_owned(),
        name: "Background".to_owned(),
        depth: 0,
        kind: "Background",
        role: "Background",
        has_children: false,
        visible: true,
        locked: false,
        selected: selection.contains("$background"),
        bound: String::new(),
    });
    rows
}

fn collect(
    nodes: &[Node],
    depth: usize,
    visible: bool,
    locked: bool,
    selection: &Selection,
    expanded: &BTreeSet<String>,
    rows: &mut Vec<Row>,
) {
    for node in nodes.iter().rev() {
        let visible = visible && node.visible;
        let locked = locked || node.locked;
        rows.push(Row {
            id: node.id.clone(),
            name: node
                .name
                .clone()
                .unwrap_or_else(|| kind_name(node).to_owned()),
            depth,
            kind: kind_name(node),
            role: role_name(node),
            has_children: !node.children.is_empty(),
            visible,
            locked,
            selected: selection.contains(&node.id),
            bound: String::new(),
        });
        if !node.children.is_empty() && expanded.contains(&node.id) {
            collect(
                &node.children,
                depth + 1,
                visible,
                locked,
                selection,
                expanded,
                rows,
            );
        }
    }
}

pub fn kind_name(node: &Node) -> &'static str {
    match node.kind {
        NodeKind::Group => "Group",
        NodeKind::Rectangle { .. } => "Rectangle",
        NodeKind::Ellipse => "Ellipse",
        NodeKind::Line => "Line",
        NodeKind::Path { .. } => "Path",
        NodeKind::Text { .. } => "Text",
        NodeKind::Chart { .. } => "Chart",
        NodeKind::Image { .. } => "Image",
        NodeKind::Video { .. } => "Video",
    }
}

/// Every node in the tree, collapsed ones included. The pane header's count.
pub fn count(document: &ThemeDocument) -> usize {
    fn walk(nodes: &[Node]) -> usize {
        nodes.iter().map(|node| 1 + walk(&node.children)).sum()
    }
    walk(&document.nodes)
}

/// The row's role in the source's vocabulary (`ui-copy.ts` `layerKindLabels`).
pub fn role_name(node: &Node) -> &'static str {
    match node.kind {
        NodeKind::Group => "Group",
        NodeKind::Rectangle { .. }
        | NodeKind::Ellipse
        | NodeKind::Line
        | NodeKind::Path { .. } => "Shape",
        NodeKind::Text { .. } => "Text",
        NodeKind::Chart { .. } => "Chart",
        NodeKind::Image { .. } | NodeKind::Video { .. } => "Image",
    }
}

impl Editor {
    /// Moves a node among its siblings. The move is one history entry.
    /// Moves a layer to another row's sibling slot, one history entry. A drop
    /// onto a group's row inserts into that group. A dropped node never becomes
    /// its own child.
    pub fn move_node_to(&mut self, source: &str, target: &str) -> bool {
        if source == target {
            return false;
        }
        self.edit(|document| {
            let Some(node) = take(&mut document.nodes, source) else {
                return;
            };
            if !insert_before(&mut document.nodes, target, node.clone()) {
                // The target vanished with the source's subtree; put the node
                // back rather than dropping it from the document.
                document.nodes.push(node);
            }
        })
    }

    pub fn raise(&mut self, id: &str, step: Move) -> bool {
        self.edit(|document| {
            reorder(&mut document.nodes, id, step);
        })
    }

    /// Hides a node, or shows it with every group above it: showing a child of
    /// a hidden group must actually show it.
    pub fn set_visible(&mut self, id: &str, visible: bool) -> bool {
        self.edit(|document| {
            if visible {
                reveal(&mut document.nodes, id);
            }
            set_visible_in(&mut document.nodes, id, visible);
        })
    }

    /// Locking applies to the node's root, so the whole painted group refuses
    /// to move.
    pub fn set_locked(&mut self, id: &str, locked: bool) -> bool {
        self.edit(|document| {
            for node in &mut document.nodes {
                if holds(node, id) {
                    node.locked = locked;
                    return;
                }
            }
        })
    }

    /// A trimmed name; whitespace alone clears it. Anything with control
    /// characters or over [`MAX_NAME`] is refused.
    pub fn rename_node(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        if name.len() > MAX_NAME || name.chars().any(char::is_control) {
            return false;
        }
        let name = (!name.is_empty()).then(|| name.to_owned());
        self.edit(|document| {
            set_name(&mut document.nodes, id, name.clone());
        })
    }
}

fn reorder(nodes: &mut Vec<Node>, id: &str, step: Move) -> bool {
    if let Some(index) = nodes.iter().position(|node| node.id == id) {
        let last = nodes.len().saturating_sub(1);
        let target = match step {
            Move::Front => last,
            Move::Forward => (index + 1).min(last),
            Move::Backward => index.saturating_sub(1),
            Move::Back => 0,
        };
        if target != index {
            let node = nodes.remove(index);
            nodes.insert(target, node);
        }
        return true;
    }
    for node in nodes {
        if reorder(&mut node.children, id, step) {
            return true;
        }
    }
    false
}

/// Removes a node from whichever sibling list holds it.
fn take(nodes: &mut Vec<Node>, id: &str) -> Option<Node> {
    if let Some(index) = nodes.iter().position(|node| node.id == id) {
        return Some(nodes.remove(index));
    }
    for node in nodes {
        if let Some(found) = take(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

/// Inserts a node immediately before `target` among its siblings. A group row
/// takes the drop into the group when the target names the group itself only if
/// it is expanded; the caller resolves that before calling.
fn insert_before(nodes: &mut Vec<Node>, target: &str, node: Node) -> bool {
    if let Some(index) = nodes.iter().position(|candidate| candidate.id == target) {
        nodes.insert(index, node);
        return true;
    }
    for candidate in nodes {
        if insert_before(&mut candidate.children, target, node.clone()) {
            return true;
        }
    }
    false
}

fn reveal(nodes: &mut [Node], id: &str) -> bool {
    for node in nodes {
        if node.id == id {
            return true;
        }
        if reveal(&mut node.children, id) {
            node.visible = true;
            return true;
        }
    }
    false
}

fn set_visible_in(nodes: &mut [Node], id: &str, visible: bool) -> bool {
    for node in nodes {
        if node.id == id {
            node.visible = visible;
            return true;
        }
        if set_visible_in(&mut node.children, id, visible) {
            return true;
        }
    }
    false
}

fn set_name(nodes: &mut [Node], id: &str, name: Option<String>) -> bool {
    for node in nodes {
        if node.id == id {
            node.name = name;
            return true;
        }
        if set_name(&mut node.children, id, name.clone()) {
            return true;
        }
    }
    false
}

/// Whether `node`'s subtree carries `wanted` (including the node itself).
fn holds(node: &Node, wanted: &str) -> bool {
    node.id == wanted || node.children.iter().any(|child| holds(child, wanted))
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{document, group, rect};

    use super::*;

    #[test]
    fn rows_are_topmost_first_with_depth() {
        let document = document(vec![
            rect("under", 0.0, 0.0, 10.0, 10.0),
            group("holder", vec![rect("child", 0.0, 0.0, 5.0, 5.0)]),
        ]);
        let selection = Selection::new();
        let closed = rows(&document, &selection, &BTreeSet::new());
        assert_eq!(
            closed.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["holder", "under", "$background"]
        );
        let mut expanded = BTreeSet::new();
        expanded.insert("holder".to_owned());
        let open = rows(&document, &selection, &expanded);
        assert_eq!(
            open.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["holder", "child", "under", "$background"]
        );
        assert_eq!(open[1].depth, 1);
    }

    #[test]
    fn a_layer_can_move_into_another_groups_slot() {
        let document = document(vec![
            rect("under", 0.0, 0.0, 10.0, 10.0),
            group("holder", vec![rect("child", 0.0, 0.0, 5.0, 5.0)]),
        ]);
        let mut editor = crate::document::Editor::new(document);
        assert!(editor.move_node_to("under", "child"));
        let rows = rows(
            editor.document(),
            &Selection::new(),
            &BTreeSet::from(["holder".to_owned()]),
        );
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["holder", "child", "under", "$background"]
        );
        // A node cannot become its own descendant.
        assert!(!editor.move_node_to("holder", "child"));
    }

    #[test]
    fn rows_carry_the_source_role_words() {
        let document = document(vec![group(
            "holder",
            vec![rect("child", 0.0, 0.0, 5.0, 5.0)],
        )]);
        let mut expanded = BTreeSet::new();
        expanded.insert("holder".to_owned());
        let rows = rows(&document, &Selection::new(), &expanded);
        assert_eq!(rows[0].role, "Group");
        assert_eq!(rows[1].role, "Shape");
        assert!(rows[0].has_children);
        assert!(!rows[1].has_children);
    }

    #[test]
    fn count_includes_collapsed_children() {
        let document = document(vec![
            rect("under", 0.0, 0.0, 10.0, 10.0),
            group("holder", vec![rect("child", 0.0, 0.0, 5.0, 5.0)]),
        ]);
        assert_eq!(count(&document), 3);
    }

    #[test]
    fn a_child_of_a_hidden_group_reads_as_hidden() {
        let mut holder = group("holder", vec![rect("child", 0.0, 0.0, 5.0, 5.0)]);
        holder.visible = false;
        let document = document(vec![holder]);
        let mut expanded = BTreeSet::new();
        expanded.insert("holder".to_owned());
        let rows = rows(&document, &Selection::new(), &expanded);
        assert!(!rows[1].visible);
    }

    #[test]
    fn raising_a_node_moves_it_one_place_and_is_one_undo() {
        let mut editor = Editor::new(document(vec![
            rect("a", 0.0, 0.0, 10.0, 10.0),
            rect("b", 0.0, 0.0, 10.0, 10.0),
        ]));
        assert!(editor.raise("a", Move::Forward));
        assert_eq!(
            editor
                .document()
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(editor.undo());
        assert_eq!(
            editor
                .document()
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(!editor.can_undo());
    }

    #[test]
    fn showing_a_child_reveals_its_ancestors() {
        let mut holder = group("holder", vec![rect("child", 0.0, 0.0, 5.0, 5.0)]);
        holder.visible = false;
        let mut editor = Editor::new(document(vec![holder]));
        assert!(editor.set_visible("child", true));
        assert!(editor.document().nodes[0].visible);
    }

    #[test]
    fn renaming_to_whitespace_clears_the_name() {
        let mut named = rect("a", 0.0, 0.0, 10.0, 10.0);
        named.name = Some("Label".to_owned());
        let mut editor = Editor::new(document(vec![named]));
        assert!(editor.rename_node("a", "  "));
        assert_eq!(editor.document().nodes[0].name, None);
        assert!(
            !editor.rename_node("a", "bad\nname"),
            "control characters are refused"
        );
    }
}
