//! Snapping: the lines a drag may land on, and the correction that lands it
//! there.
//!
//! The lines are the artboard's edges and centre and the edges and centres of
//! every node that is not being dragged. A line within five screen pixels
//! captures the edge nearest it; holding Ctrl turns the whole thing off.

use std::collections::BTreeSet;

use yunari_model::document::{Node, ThemeDocument};

/// Five screen pixels, in artboard units at the current zoom.
pub const SNAP_THRESHOLD: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// A line the drag was corrected to, as the source draws it: across the whole
/// artboard, through the point it captured.
#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    pub axis: Axis,
    pub at: f64,
    pub from: f64,
    pub to: f64,
    /// Whether this line came from equal spacing rather than an edge or centre.
    pub spacing: bool,
    /// The interval a spacing guide marks, printed beside it in the source.
    pub label: Option<String>,
}

/// Which kind of line won, so a boundary beats an edge and an edge a centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Boundary,
    Edge,
    Centre,
}

#[derive(Debug, Clone, Copy)]
struct Line {
    at: f64,
    rank: Rank,
}

/// Where a translated selection lands: the corrected origin of its first node,
/// and the lines it was pulled to.
pub fn snap_move(
    document: &ThemeDocument,
    moving: &[String],
    proposed: (f64, f64),
    zoom: f32,
    enabled: bool,
) -> (f64, f64, Vec<Guide>) {
    let Some(anchor) = moving
        .first()
        .and_then(|id| crate::selection::find(document, id))
    else {
        return (proposed.0, proposed.1, Vec::new());
    };
    if !enabled {
        return (proposed.0, proposed.1, Vec::new());
    }
    let Some(bounds) = union(document, moving) else {
        return (proposed.0, proposed.1, Vec::new());
    };
    let dx = proposed.0 - anchor.transform.x;
    let dy = proposed.1 - anchor.transform.y;
    let (x_lines, y_lines) = candidate_lines(document, moving);
    let threshold = SNAP_THRESHOLD / f64::from(zoom.max(f32::MIN_POSITIVE));
    let width = document.artboard.width;
    let height = document.artboard.height;

    let mut guides = Vec::new();
    let mut x = proposed.0;
    let edges_x = [
        bounds.0 + dx,
        (bounds.0 + bounds.2) / 2.0 + dx,
        bounds.2 + dx,
    ];
    if let Some((line, edge)) = nearest_edge(&edges_x, &x_lines, threshold) {
        x = proposed.0 + line.at - edge;
        guides.push(Guide {
            axis: Axis::X,
            at: line.at,
            from: 0.0,
            to: height,
            spacing: false,
            label: None,
        });
    }
    let mut y = proposed.1;
    let edges_y = [
        bounds.1 + dy,
        (bounds.1 + bounds.3) / 2.0 + dy,
        bounds.3 + dy,
    ];
    if let Some((line, edge)) = nearest_edge(&edges_y, &y_lines, threshold) {
        y = proposed.1 + line.at - edge;
        guides.push(Guide {
            axis: Axis::Y,
            at: line.at,
            from: 0.0,
            to: width,
            spacing: false,
            label: None,
        });
    }
    if let Some((shift, spacing_guides)) =
        equal_spacing(document, moving, bounds, dx + (x - proposed.0), Axis::X, threshold)
    {
        x += shift;
        guides.extend(spacing_guides);
    }
    if let Some((shift, spacing_guides)) =
        equal_spacing(document, moving, bounds, dy + (y - proposed.1), Axis::Y, threshold)
    {
        y += shift;
        guides.extend(spacing_guides);
    }
    (x, y, guides)
}

/// Equal-spacing correction for a moved selection, treated as one interval,
/// between its two nearest
/// neighbours on an axis. The source offers the same interval as a snap when
/// the two gaps are within its hold margin; the guide marks both intervals.
fn equal_spacing(
    document: &ThemeDocument,
    moving: &[String],
    bounds: (f64, f64, f64, f64),
    delta: f64,
    axis: Axis,
    threshold: f64,
) -> Option<(f64, Vec<Guide>)> {
    let mut neighbors: Vec<(f64, f64)> = Vec::new();
    collect_intervals(&document.nodes, moving, axis, &mut neighbors);
    neighbors.sort_by(|left, right| left.0.total_cmp(&right.0));
    let (active_start, active_end) = match axis {
        Axis::X => (bounds.0 + delta, bounds.2 + delta),
        Axis::Y => (bounds.1 + delta, bounds.3 + delta),
    };
    let before = neighbors.iter().rev().find(|candidate| candidate.1 <= active_start);
    let after = neighbors.iter().find(|candidate| candidate.0 >= active_end);
    let (before_end, after_start) = match (before, after) {
        (Some(left), Some(right)) => (left.1, right.0),
        _ => return None,
    };
    let gap_before = active_start - before_end;
    let gap_after = after_start - active_end;
    if gap_before <= 0.0 || gap_after <= 0.0 || (gap_before - gap_after).abs() > threshold {
        return None;
    }
    let shift = (gap_after - gap_before) / 2.0;
    let (from, to, midpoint_before, midpoint_after) = match axis {
        Axis::X => (
            0.0,
            document.artboard.height,
            before_end + gap_before / 2.0,
            active_end + gap_after / 2.0 + shift,
        ),
        Axis::Y => (
            0.0,
            document.artboard.width,
            before_end + gap_before / 2.0,
            active_end + gap_after / 2.0 + shift,
        ),
    };
    Some((
        shift,
        vec![
            Guide { axis, at: midpoint_before, from, to, spacing: true, label: Some(format!("{:.0}", gap_before.round())) },
            Guide { axis, at: midpoint_after, from, to, spacing: true, label: Some(format!("{:.0}", gap_after.round())) },
        ],
    ))
}

fn collect_intervals(nodes: &[Node], moving: &[String], axis: Axis, out: &mut Vec<(f64, f64)>) {
    for node in nodes {
        if !moving.contains(&node.id) && node.visible {
            let (x, y, width, height) = box_of(node);
            out.push(match axis {
                Axis::X => (x, x + width),
                Axis::Y => (y, y + height),
            });
        }
        collect_intervals(&node.children, moving, axis, out);
    }
}

/// The edge nearest a line, and which edge it was, when it is within reach.
fn nearest_edge(edges: &[f64], lines: &[Line], threshold: f64) -> Option<(Line, f64)> {
    let mut best: Option<(f64, Line, f64)> = None;
    for &edge in edges {
        let Some(line) = nearest(lines, edge) else {
            continue;
        };
        let distance = (line.at - edge).abs();
        if distance >= threshold {
            continue;
        }
        best = match best {
            None => Some((distance, line, edge)),
            Some((had, _, _)) if distance < had - 1e-9 => Some((distance, line, edge)),
            other => other,
        };
    }
    best.map(|(_, line, edge)| (line, edge))
}

/// The closest line, with ties going to the more important kind.
fn nearest(lines: &[Line], at: f64) -> Option<Line> {
    let mut best: Option<(f64, Line)> = None;
    for &line in lines {
        let distance = (line.at - at).abs();
        best = match best {
            None => Some((distance, line)),
            Some((had, held)) => {
                let closer = distance < had - 1e-9;
                let tied = (distance - had).abs() <= 1e-9 && line.rank < held.rank;
                if closer || tied {
                    Some((distance, line))
                } else {
                    Some((had, held))
                }
            }
        };
    }
    best.map(|(_, line)| line)
}

/// The bounding box of the moving nodes, in artboard units.
fn union(document: &ThemeDocument, moving: &[String]) -> Option<(f64, f64, f64, f64)> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for id in moving {
        let Some(node) = crate::selection::find(document, id) else {
            continue;
        };
        let (x, y, width, height) = box_of(node);
        bounds = Some(match bounds {
            None => (x, y, x + width, y + height),
            Some((left, top, right, bottom)) => (
                left.min(x),
                top.min(y),
                right.max(x + width),
                bottom.max(y + height),
            ),
        });
    }
    bounds
}

fn box_of(node: &Node) -> (f64, f64, f64, f64) {
    let transform = &node.transform;
    (
        transform.x,
        transform.y,
        transform.width * transform.scale_x,
        transform.height * transform.scale_y,
    )
}

/// Every line the drag may land on, in a stable order: the artboard first.
fn candidate_lines(document: &ThemeDocument, moving: &[String]) -> (Vec<Line>, Vec<Line>) {
    let mut x_lines = artboard_lines(document.artboard.width);
    let mut y_lines = artboard_lines(document.artboard.height);
    let excluded = excluded_ids(document, moving);
    for node in &document.nodes {
        collect(node, &excluded, &mut x_lines, &mut y_lines);
    }
    (x_lines, y_lines)
}

fn artboard_lines(extent: f64) -> Vec<Line> {
    [0.0, extent / 2.0, extent]
        .into_iter()
        .map(|at| Line {
            at,
            rank: Rank::Boundary,
        })
        .collect()
}

fn collect(node: &Node, excluded: &BTreeSet<String>, x: &mut Vec<Line>, y: &mut Vec<Line>) {
    if !excluded.contains(&node.id) && node.visible {
        let (x0, y0, width, height) = box_of(node);
        x.push(Line {
            at: x0,
            rank: Rank::Edge,
        });
        x.push(Line {
            at: x0 + width / 2.0,
            rank: Rank::Centre,
        });
        x.push(Line {
            at: x0 + width,
            rank: Rank::Edge,
        });
        y.push(Line {
            at: y0,
            rank: Rank::Edge,
        });
        y.push(Line {
            at: y0 + height / 2.0,
            rank: Rank::Centre,
        });
        y.push(Line {
            at: y0 + height,
            rank: Rank::Edge,
        });
    }
    for child in &node.children {
        collect(child, excluded, x, y);
    }
}

/// The moving nodes, everything under them, and everything above them: a drag
/// is not pulled towards its own geometry or the box that contains it.
fn excluded_ids(document: &ThemeDocument, moving: &[String]) -> BTreeSet<String> {
    let mut excluded = BTreeSet::new();
    let wanted: BTreeSet<&str> = moving.iter().map(String::as_str).collect();
    walk(&document.nodes, &wanted, &mut Vec::new(), &mut excluded);
    excluded
}

fn walk(
    nodes: &[Node],
    moving: &BTreeSet<&str>,
    parents: &mut Vec<String>,
    excluded: &mut BTreeSet<String>,
) {
    for node in nodes {
        let inside = moving.contains(node.id.as_str());
        let under_moving = parents.iter().any(|id| moving.contains(id.as_str()));
        if inside {
            exclude_subtree(node, excluded);
            for parent in parents.iter() {
                excluded.insert(parent.clone());
            }
        } else if under_moving {
            exclude_subtree(node, excluded);
        }
        parents.push(node.id.clone());
        walk(&node.children, moving, parents, excluded);
        parents.pop();
    }
}

fn exclude_subtree(node: &Node, excluded: &mut BTreeSet<String>) {
    excluded.insert(node.id.clone());
    for child in &node.children {
        exclude_subtree(child, excluded);
    }
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
            schema_version: "3.0.0".to_owned(),
            fabric_version: "7.4.0".to_owned(),
            id: "test".to_owned(),
            metadata: Metadata::default(),
            artboard: Artboard {
                width: 450.0,
                height: 675.0,
                content_fit: ContentFit::Contain,
                shape: ArtboardShape::Rect,
            },
            background: Background::default(),
            globals: Default::default(),
            scene: Default::default(),
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

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_node_near_the_artboard_centre_snaps_to_it() {
        let document = document(vec![rect("a", 10.0, 100.0, 20.0, 20.0)]);
        let (x, _, guides) = snap_move(&document, &ids(&["a"]), (224.6, 100.0), 1.0, true);
        assert!((x - 225.0).abs() < 1e-9, "{x}");
        assert_eq!(guides.len(), 1);
        assert_eq!(guides[0].axis, Axis::X);
        assert!((guides[0].at - 225.0).abs() < 1e-9);
    }

    #[test]
    fn beyond_the_threshold_nothing_snaps() {
        let document = document(vec![rect("a", 10.0, 100.0, 20.0, 20.0)]);
        let (x, y, guides) = snap_move(&document, &ids(&["a"]), (220.0, 100.0), 1.0, true);
        assert_eq!((x, y), (220.0, 100.0));
        assert!(guides.is_empty());
    }

    #[test]
    fn a_disabled_snap_leaves_the_position_alone() {
        let document = document(vec![rect("a", 10.0, 100.0, 20.0, 20.0)]);
        let (x, _, guides) = snap_move(&document, &ids(&["a"]), (224.6, 100.0), 1.0, false);
        assert_eq!(x, 224.6);
        assert!(guides.is_empty());
    }

    #[test]
    fn the_dragged_nodes_own_subtree_offers_no_lines() {
        let mut parent = rect("parent", 0.0, 0.0, 10.0, 10.0);
        parent.children.push(rect("child", 20.0, 0.0, 10.0, 10.0));
        let document = document(vec![parent]);
        // The child's left edge (30 at this offset) is the only nearby line, and
        // it would drag the parent onto it. A sibling would still capture.
        let (x, _, guides) = snap_move(&document, &ids(&["parent"]), (9.6, 300.0), 1.0, true);
        assert_eq!(x, 9.6);
        assert!(guides.is_empty());
    }

    #[test]
    fn a_sibling_offers_the_line_the_node_snaps_to() {
        let document = document(vec![
            rect("a", 0.0, 0.0, 10.0, 10.0),
            rect("b", 100.0, 100.0, 10.0, 10.0),
        ]);
        let (x, _, guides) = snap_move(&document, &ids(&["a"]), (91.0, 300.0), 1.0, true);
        assert!(
            (x - 90.0).abs() < 1e-9,
            "right edge 101 lands on b's left 100"
        );
        assert_eq!(guides.len(), 1);
    }

    #[test]
    fn hidden_nodes_offer_no_lines() {
        let mut hidden = rect("hidden", 100.0, 0.0, 10.0, 10.0);
        hidden.visible = false;
        let document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0), hidden]);
        let (x, _, guides) = snap_move(&document, &ids(&["a"]), (89.5, 300.0), 1.0, true);
        assert_eq!(x, 89.5);
        assert!(guides.is_empty());
    }

    #[test]
    fn a_move_between_two_neighbours_can_land_on_an_equal_gap() {
        let document = document(vec![
            rect("left", 0.0, 0.0, 10.0, 10.0),
            rect("moving", 20.0, 0.0, 10.0, 10.0),
            rect("right", 45.0, 0.0, 10.0, 10.0),
        ]);
        // The gaps are 10 and 12.5. Moving 1.25 right makes both 11.25.
        let (x, _, guides) = snap_move(&document, &ids(&["moving"]), (21.25, 0.0), 1.0, true);
        assert_eq!(x, 22.5);
        assert!(guides.len() >= 2);
        assert!(guides.iter().any(|guide| guide.axis == Axis::X));
    }

    #[test]
    fn a_two_object_selection_can_snap_its_outer_gaps() {
        let document = document(vec![
            rect("left", 0.0, 0.0, 10.0, 10.0),
            rect("first", 20.0, 0.0, 10.0, 10.0),
            rect("second", 30.0, 0.0, 10.0, 10.0),
            rect("right", 57.0, 0.0, 10.0, 10.0),
        ]);
        let (x, _, guides) = snap_move(
            &document,
            &ids(&["first", "second"]),
            (21.25, 0.0),
            1.0,
            true,
        );
        assert!(x > 21.25);
        assert!(guides.len() >= 2);
    }

    #[test]
    fn a_boundary_wins_a_tie_over_an_edge_and_an_edge_over_a_centre() {
        let boundary = Line {
            at: 100.0,
            rank: Rank::Boundary,
        };
        let edge = Line {
            at: 100.0,
            rank: Rank::Edge,
        };
        let centre = Line {
            at: 100.0,
            rank: Rank::Centre,
        };
        assert_eq!(
            nearest(&[edge, boundary], 100.0).unwrap().rank,
            Rank::Boundary
        );
        assert_eq!(nearest(&[centre, edge], 100.0).unwrap().rank, Rank::Edge);
        assert_eq!(nearest(&[centre], 100.0).unwrap().rank, Rank::Centre);
    }
}
