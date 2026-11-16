//! Grouping and ungrouping: the selection becomes one node at its union box,
//! and splitting puts the children back where they were drawn.
//!
//! Children are stored relative to the group, so the group can be moved as one
//! thing; ungrouping adds the group's offset back and takes the group away.

use yunari_model::document::{Node, NodeKind, StyleMap, ThemeDocument, Transform};

use crate::document::Editor;

/// Groups the named top-level nodes. Returns the new group's id when at least
/// two named nodes were there and none is locked.
pub fn group(document: &mut ThemeDocument, ids: &[String]) -> Option<String> {
    let members: Vec<usize> = ids
        .iter()
        .filter_map(|id| document.nodes.iter().position(|node| node.id == *id))
        .collect();
    if members.len() < 2 {
        return None;
    }
    if members.iter().any(|index| document.nodes[*index].locked) {
        return None;
    }
    let (left, top, right, bottom) = members.iter().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(left, top, right, bottom), &index| {
            let transform = document.nodes[index].transform;
            (
                left.min(transform.x),
                top.min(transform.y),
                right.max(transform.x + transform.width * transform.scale_x),
                bottom.max(transform.y + transform.height * transform.scale_y),
            )
        },
    );
    let group_id = free_group_id(document);
    let mut children = Vec::new();
    let insert_at = *members.iter().max().expect("two members");
    for &index in &members {
        let mut node = document.nodes[index].clone();
        node.transform.x -= left;
        node.transform.y -= top;
        children.push(node);
    }
    // Remove from the back so the earlier indices stay valid.
    for &index in members.iter().rev() {
        document.nodes.remove(index);
    }
    let position = insert_at + 1 - members.len();
    document.nodes.insert(
        position.min(document.nodes.len()),
        Node {
            id: group_id.clone(),
            name: None,
            kind: NodeKind::Group,
            transform: Transform {
                x: left,
                y: top,
                width: (right - left).max(1.0),
                height: (bottom - top).max(1.0),
                ..Transform::default()
            },
            visible: true,
            locked: false,
            style: StyleMap::new(),
            glass: None,
            children,
        },
    );
    Some(group_id)
}

/// Puts a group's children back in the root list at the group's place, at the
/// world positions they were drawn at. Returns their ids.
pub fn ungroup(document: &mut ThemeDocument, group_id: &str) -> Vec<String> {
    let Some(index) = document.nodes.iter().position(|node| node.id == group_id) else {
        return Vec::new();
    };
    if document.nodes[index].kind != NodeKind::Group {
        return Vec::new();
    }
    let group = document.nodes.remove(index);
    let transform = group.transform;
    let mut ids = Vec::new();
    let children: Vec<Node> = group
        .children
        .into_iter()
        .map(|mut child| {
            // A turned or scaled group cannot be flattened without rewriting
            // the child's rotation, so only the offset is baked back.
            child.transform.x += transform.x;
            child.transform.y += transform.y;
            ids.push(child.id.clone());
            child
        })
        .collect();
    let at = index.min(document.nodes.len());
    for (offset, child) in children.into_iter().enumerate() {
        document.nodes.insert(at + offset, child);
    }
    ids
}

fn free_group_id(document: &ThemeDocument) -> String {
    let mut number = 1;
    loop {
        let id = format!("group-{number}");
        if !document.nodes.iter().any(|node| node.id == id) {
            return id;
        }
        number += 1;
    }
}

impl Editor {
    /// One history entry, one group.
    pub fn group(&mut self, ids: &[String]) -> Option<String> {
        let mut created = None;
        self.edit(|document| created = group(document, ids));
        created
    }

    pub fn ungroup(&mut self, group_id: &str) -> Vec<String> {
        let mut freed = Vec::new();
        self.edit(|document| freed = ungroup(document, group_id));
        freed
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{document, group as group_node, rect};
    use crate::selection::find;

    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn grouping_two_nodes_keeps_them_where_they_were_drawn() {
        let mut document = document(vec![
            rect("a", 10.0, 10.0, 50.0, 50.0),
            rect("b", 60.0, 30.0, 20.0, 20.0),
        ]);
        let id = group(&mut document, &ids(&["a", "b"])).expect("a group");
        let holder = find(&document, &id).expect("the group exists");
        assert_eq!(holder.transform.x, 10.0);
        assert_eq!(holder.transform.y, 10.0);
        let first = find(&document, "a").expect("a is inside");
        assert_eq!((first.transform.x, first.transform.y), (0.0, 0.0));
        let second = find(&document, "b").expect("b is inside");
        assert_eq!((second.transform.x, second.transform.y), (50.0, 20.0));
    }

    #[test]
    fn a_group_needs_two_members_and_no_locks() {
        let mut locked = rect("b", 0.0, 0.0, 10.0, 10.0);
        locked.locked = true;
        let mut document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0), locked]);
        assert!(group(&mut document, &ids(&["a"])).is_none());
        assert!(group(&mut document, &ids(&["a", "b"])).is_none());
    }

    #[test]
    fn ungroup_restores_world_positions_and_one_history_step() {
        let mut editor = Editor::new(document(vec![
            rect("a", 10.0, 10.0, 50.0, 50.0),
            rect("b", 60.0, 30.0, 20.0, 20.0),
        ]));
        let id = editor.group(&ids(&["a", "b"])).expect("a group");
        assert_eq!(editor.document().nodes.len(), 1);
        let freed = editor.ungroup(&id);
        assert_eq!(freed, ["a", "b"]);
        assert_eq!(editor.document().nodes.len(), 2);
        assert_eq!(editor.document().nodes[0].transform.x, 10.0);
        assert_eq!(editor.document().nodes[1].transform.x, 60.0);
        assert!(editor.undo());
        assert_eq!(editor.document().nodes.len(), 1, "undo restores the group");
        assert!(editor.undo());
        assert_eq!(editor.document().nodes.len(), 2, "and the pair before it");
    }

    #[test]
    fn ungrouping_something_that_is_not_a_group_does_nothing() {
        let mut document = document(vec![
            group_node("old", vec![rect("child", 5.0, 5.0, 10.0, 10.0)]),
            rect("plain", 0.0, 0.0, 10.0, 10.0),
        ]);
        assert!(ungroup(&mut document, "plain").is_empty());
        assert_eq!(document.nodes.len(), 2);
    }
}
