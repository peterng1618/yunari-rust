//! Align and distribute: the ops that move a selection onto the first member's
//! edges, or into equal gaps.
//!
//! Every op works on top-level nodes only, refuses the whole selection when any
//! member is locked, and reports whether it moved anything.

use yunari_model::document::ThemeDocument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    AlignLeft,
    AlignCenterX,
    AlignRight,
    AlignTop,
    AlignCenterY,
    AlignBottom,
    DistributeX,
    DistributeY,
}

impl Action {
    /// How many members the op needs to mean anything.
    pub fn needed(self) -> usize {
        match self {
            Self::DistributeX | Self::DistributeY => 3,
            _ => 2,
        }
    }
}

/// Applies the op in place. Returns whether anything moved.
pub fn apply(document: &mut ThemeDocument, ids: &[String], action: Action) -> bool {
    let indices: Vec<usize> = ids
        .iter()
        .filter_map(|id| document.nodes.iter().position(|node| node.id == *id))
        .collect();
    if indices.len() < action.needed() {
        return false;
    }
    if indices.iter().any(|index| document.nodes[*index].locked) {
        return false;
    }
    match action {
        Action::DistributeX | Action::DistributeY => {
            distribute(&mut document.nodes, &indices, action)
        }
        _ => align(&mut document.nodes, &indices, action),
    }
}

fn align(nodes: &mut [yunari_model::document::Node], indices: &[usize], action: Action) -> bool {
    let anchor = nodes[indices[0]].transform;
    let mut moved = false;
    for &index in &indices[1..] {
        let transform = &mut nodes[index].transform;
        let (x, y) = match action {
            Action::AlignLeft => (anchor.x, transform.y),
            Action::AlignCenterX => (
                anchor.x
                    + (anchor.width * anchor.scale_x - transform.width * transform.scale_x) / 2.0,
                transform.y,
            ),
            Action::AlignRight => (
                anchor.x + anchor.width * anchor.scale_x - transform.width * transform.scale_x,
                transform.y,
            ),
            Action::AlignTop => (transform.x, anchor.y),
            Action::AlignCenterY => (
                transform.x,
                anchor.y
                    + (anchor.height * anchor.scale_y - transform.height * transform.scale_y) / 2.0,
            ),
            Action::AlignBottom => (
                transform.x,
                anchor.y + anchor.height * anchor.scale_y - transform.height * transform.scale_y,
            ),
            Action::DistributeX | Action::DistributeY => unreachable!("handled above"),
        };
        if transform.x != x || transform.y != y {
            transform.x = x;
            transform.y = y;
            moved = true;
        }
    }
    moved
}

/// Equal gaps between consecutive boxes, with the outer two left where they are.
fn distribute(
    nodes: &mut [yunari_model::document::Node],
    indices: &[usize],
    action: Action,
) -> bool {
    let horizontal = action == Action::DistributeX;
    let mut order = indices.to_vec();
    order.sort_by(|a, b| {
        let key = |index: usize| {
            let transform = nodes[index].transform;
            if horizontal { transform.x } else { transform.y }
        };
        key(*a).total_cmp(&key(*b))
    });
    let extent = |index: usize| {
        let transform = nodes[index].transform;
        if horizontal {
            transform.width * transform.scale_x
        } else {
            transform.height * transform.scale_y
        }
    };
    let start = |index: usize| {
        let transform = nodes[index].transform;
        if horizontal { transform.x } else { transform.y }
    };
    let first = order[0];
    let last = *order.last().expect("three members or more");
    let span = start(last) + extent(last) - start(first);
    let sizes: Vec<f64> = order.iter().map(|index| extent(*index)).collect();
    let gap = ((span - sizes.iter().sum::<f64>()) / (order.len() as f64 - 1.0)).max(0.0);
    let mut cursor = start(first);
    let mut moved = false;
    for (position, &index) in order.iter().enumerate() {
        let width = sizes[position];
        let transform = &mut nodes[index].transform;
        if horizontal {
            if transform.x != cursor {
                transform.x = cursor;
                moved = true;
            }
        } else if transform.y != cursor {
            transform.y = cursor;
            moved = true;
        }
        cursor += width + gap;
    }
    moved
}

#[cfg(test)]
mod tests {
    use crate::fixtures::{document, rect};

    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    fn x_of(document: &ThemeDocument, id: &str) -> f64 {
        document
            .nodes
            .iter()
            .find(|node| node.id == id)
            .expect("node")
            .transform
            .x
    }

    #[test]
    fn aligning_left_uses_the_first_selected_node() {
        let mut document = document(vec![
            rect("a", 10.0, 0.0, 50.0, 10.0),
            rect("b", 40.0, 90.0, 20.0, 10.0),
        ]);
        assert!(apply(&mut document, &ids(&["a", "b"]), Action::AlignLeft));
        assert_eq!(x_of(&document, "b"), 10.0);
        assert_eq!(x_of(&document, "a"), 10.0, "the anchor stays put");
    }

    #[test]
    fn aligning_centre_uses_the_anchor_centre() {
        let mut document = document(vec![
            rect("a", 0.0, 0.0, 100.0, 10.0),
            rect("b", 0.0, 50.0, 20.0, 10.0),
        ]);
        assert!(apply(
            &mut document,
            &ids(&["a", "b"]),
            Action::AlignCenterX
        ));
        assert_eq!(x_of(&document, "b"), 40.0);
    }

    #[test]
    fn distributing_needs_three_members_and_equalises_gaps() {
        let mut document = document(vec![
            rect("a", 0.0, 0.0, 10.0, 10.0),
            rect("b", 50.0, 0.0, 10.0, 10.0),
            rect("c", 200.0, 0.0, 10.0, 10.0),
        ]);
        assert!(!apply(
            &mut document,
            &ids(&["a", "b"]),
            Action::DistributeX
        ));
        assert!(apply(
            &mut document,
            &ids(&["a", "b", "c"]),
            Action::DistributeX
        ));
        assert_eq!(x_of(&document, "a"), 0.0);
        assert_eq!(x_of(&document, "b"), 100.0);
        assert_eq!(x_of(&document, "c"), 200.0);
    }

    #[test]
    fn a_locked_member_refuses_the_whole_align() {
        let mut locked = rect("b", 40.0, 0.0, 20.0, 10.0);
        locked.locked = true;
        let mut document = document(vec![rect("a", 10.0, 0.0, 50.0, 10.0), locked]);
        assert!(!apply(&mut document, &ids(&["a", "b"]), Action::AlignLeft));
        assert_eq!(x_of(&document, "b"), 40.0);
    }
}
