//! Authored state → paint geometry.
//!
//! Every decision a backend must not make lives here: which paint a style
//! names, where a node's box sits once its parents have had their say, and why
//! a value could not be drawn. A node that cannot be painted becomes an issue
//! with a reason, never a guess.

use yunari_model::document::{Node, NodeKind, StyleMap, StyleValue, ThemeDocument};

use crate::affine::Affine;
use crate::color::{Color, parse_color};
use crate::display_list::{DisplayList, GradientStop, Op, Paint, Shape, Stroke};

/// Why a frame could not paint something, as a runtime diagnostic rather than
/// a document error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIssue {
    pub code: IssueCode,
    pub node_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCode {
    /// A token reference names something the document does not declare.
    UnresolvedGlobal,
    /// A paint is written in a form this renderer does not read.
    UnreadablePaint,
    /// The node's content is not drawable yet.
    UnsupportedContent,
}

#[derive(Debug, Clone, PartialEq)]
enum PaintSpec {
    Solid(Color),
    Gradient {
        angle: f32,
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone, Default)]
struct ResolvedStyle {
    fill: Option<PaintSpec>,
    stroke: Option<PaintSpec>,
    stroke_width: f32,
    opacity: f32,
}

/// Build the frame's geometry and the reasons anything in it is missing.
pub fn build(document: &ThemeDocument) -> (DisplayList, Vec<PlanIssue>) {
    let width = document.artboard.width as f32;
    let height = document.artboard.height as f32;
    let mut issues = Vec::new();
    let background = resolve_background(document, width, height, &mut issues);
    let mut list = DisplayList {
        width,
        height,
        background,
        ops: Vec::new(),
    };
    for node in &document.nodes {
        walk(document, node, Affine::IDENTITY, 1.0, &mut list, &mut issues);
    }
    (list, issues)
}

/// One owner for "where the page paint lives": the media's colour when a
/// picture is declared, the background's own otherwise.
fn resolve_background(
    document: &ThemeDocument,
    width: f32,
    height: f32,
    issues: &mut Vec<PlanIssue>,
) -> Option<Paint> {
    let background = &document.background;
    let value = match &background.media {
        Some(media) => media.color.as_ref(),
        None => background.color.as_ref(),
    }?;
    let spec = resolve_paint(document, value, None, "background/color", issues)?;
    Some(place_paint(spec, width, height, 1.0))
}

fn walk(
    document: &ThemeDocument,
    node: &Node,
    parent: Affine,
    opacity: f32,
    list: &mut DisplayList,
    issues: &mut Vec<PlanIssue>,
) {
    if !node.visible {
        return;
    }
    let style = resolve_style(document, &node.style, node, issues);
    let opacity = opacity * style.opacity;
    let transform = parent.then(Affine::from_node(&node.transform));
    let box_width = node.transform.width as f32;
    let box_height = node.transform.height as f32;

    let shape = match &node.kind {
        NodeKind::Group => None,
        NodeKind::Rectangle { corner_radius } => Some(Shape::Rect {
            width: box_width,
            height: box_height,
            corner_radius: *corner_radius as f32,
        }),
        NodeKind::Ellipse => Some(Shape::Ellipse {
            width: box_width,
            height: box_height,
        }),
        NodeKind::Line => Some(Shape::Line {
            length: box_width,
        }),
        NodeKind::Path { .. } => {
            issues.push(PlanIssue {
                code: IssueCode::UnsupportedContent,
                node_id: Some(node.id.clone()),
                detail: "A path is not drawn yet.".into(),
            });
            None
        }
        NodeKind::Text { .. } | NodeKind::Chart { .. } | NodeKind::Image { .. }
        | NodeKind::Video { .. } => None,
    };

    if let Some(shape) = shape {
        let fill = style
            .fill
            .map(|spec| place_paint(spec, box_width, box_height, opacity));
        let stroke = style.stroke.map(|spec| Stroke {
            paint: place_paint(spec, box_width, box_height, opacity),
            width: style.stroke_width,
        });
        if fill.is_some() || stroke.is_some() {
            list.ops.push(Op {
                transform,
                shape,
                fill,
                stroke,
            });
        }
    }

    for child in &node.children {
        walk(document, child, transform, opacity, list, issues);
    }
}

/// A gradient's axis across its own box, in the canvas's y-down space, where
/// the stored angle is `90 - cssDegrees`: 0 runs left to right and 90 runs
/// bottom to top.
fn place_paint(spec: PaintSpec, width: f32, height: f32, opacity: f32) -> Paint {
    match spec {
        PaintSpec::Solid(color) => Paint::Solid(scale_alpha(color, opacity)),
        PaintSpec::Gradient { angle, stops } => {
            let radians = angle.to_radians();
            let dx = radians.cos();
            let dy = -radians.sin();
            let reach = (width * dx).abs() / 2.0 + (height * dy).abs() / 2.0;
            let (centre_x, centre_y) = (width / 2.0, height / 2.0);
            Paint::Linear {
                from: (centre_x - dx * reach, centre_y - dy * reach),
                to: (centre_x + dx * reach, centre_y + dy * reach),
                stops: stops
                    .into_iter()
                    .map(|stop| GradientStop {
                        offset: stop.offset,
                        color: scale_alpha(stop.color, opacity),
                    })
                    .collect(),
            }
        }
    }
}

fn scale_alpha(color: Color, opacity: f32) -> Color {
    if opacity >= 1.0 {
        return color;
    }
    let alpha = (f32::from(color.a) * opacity.clamp(0.0, 1.0)).round();
    color.with_alpha(alpha as u8)
}

fn resolve_style(
    document: &ThemeDocument,
    style: &StyleMap,
    node: &Node,
    issues: &mut Vec<PlanIssue>,
) -> ResolvedStyle {
    let path = |key: &str| format!("nodes/{}/style/{key}", node.id);
    let fill = style
        .get("fill")
        .and_then(|value| resolve_paint(document, value, Some(&node.id), &path("fill"), issues));
    let stroke = style.get("stroke").and_then(|value| {
        resolve_paint(document, value, Some(&node.id), &path("stroke"), issues)
    });
    ResolvedStyle {
        fill,
        stroke,
        stroke_width: style
            .get("strokeWidth")
            .and_then(style_number)
            .unwrap_or(0.0) as f32,
        opacity: style
            .get("opacity")
            .and_then(style_number)
            .map_or(1.0, |opacity| opacity.clamp(0.0, 1.0) as f32),
    }
}

fn style_number(value: &StyleValue) -> Option<f64> {
    match value {
        StyleValue::Literal(literal) => literal.value.as_f64(),
        StyleValue::Ref(_) => None,
    }
}

fn resolve_paint(
    document: &ThemeDocument,
    value: &StyleValue,
    node_id: Option<&str>,
    path: &str,
    issues: &mut Vec<PlanIssue>,
) -> Option<PaintSpec> {
    match value {
        StyleValue::Literal(literal) => match literal.value.as_str() {
            Some(text) => parse_paint_string(text, node_id, path, issues),
            None => {
                unreadable(issues, node_id, path, "A literal paint is a colour string.");
                None
            }
        },
        StyleValue::Ref(reference) => {
            let Some(id) = reference.r#ref.strip_prefix("palette.") else {
                issues.push(PlanIssue {
                    code: IssueCode::UnresolvedGlobal,
                    node_id: node_id.map(ToOwned::to_owned),
                    detail: format!("{} is not a paint this frame can use.", reference.r#ref),
                });
                return None;
            };
            let Some(entry) = document.palette.get(id) else {
                issues.push(PlanIssue {
                    code: IssueCode::UnresolvedGlobal,
                    node_id: node_id.map(ToOwned::to_owned),
                    detail: format!("palette.{id} is not declared."),
                });
                return None;
            };
            match &entry.value {
                yunari_model::document::Paint::Solid { color } => {
                    parse_paint_string(color, node_id, path, issues)
                }
                yunari_model::document::Paint::Gradient { angle, stops } => {
                    let stops = stops
                        .iter()
                        .filter_map(|stop| {
                            let color = parse_paint_string(&stop.color, node_id, path, issues)?;
                            let PaintSpec::Solid(color) = color else {
                                return None;
                            };
                            Some(GradientStop {
                                offset: stop.offset as f32,
                                color,
                            })
                        })
                        .collect();
                    Some(PaintSpec::Gradient {
                        angle: *angle as f32,
                        stops,
                    })
                }
            }
        }
    }
}

fn parse_paint_string(
    text: &str,
    node_id: Option<&str>,
    path: &str,
    issues: &mut Vec<PlanIssue>,
) -> Option<PaintSpec> {
    match parse_color(text) {
        Some(color) => Some(PaintSpec::Solid(color)),
        None => {
            unreadable(
                issues,
                node_id,
                path,
                &format!("\"{text}\" is not a colour this frame reads."),
            );
            None
        }
    }
}

fn unreadable(issues: &mut Vec<PlanIssue>, node_id: Option<&str>, path: &str, detail: &str) {
    issues.push(PlanIssue {
        code: IssueCode::UnreadablePaint,
        node_id: node_id.map(ToOwned::to_owned),
        detail: format!("{path}: {detail}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_model::document::parse_document;

    fn document(json: &str) -> ThemeDocument {
        parse_document(json).expect("fixture document parses")
    }

    /// A plan is built from whatever the editor currently holds, which the
    /// document validator has not necessarily passed; that is the only way a
    /// reference can reach the renderer unresolved.
    fn unvalidated(json: &str) -> ThemeDocument {
        serde_json::from_str(json).expect("fixture document reads")
    }

    fn one_node(kind: &str, style: &str, transform: &str) -> ThemeDocument {
        document(&format!(
            r##"{{"schemaVersion":"1.0.0","id":"t","artboard":{{"width":100,"height":80}},
                "nodes":[{{"id":"n1","kind":{kind},"transform":{transform},"style":{style}}}]}}"##
        ))
    }

    #[test]
    fn an_empty_document_paints_nothing() {
        let (list, issues) = build(&document(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},"nodes":[]}"##,
        ));
        assert_eq!(list.width, 100.0);
        assert_eq!(list.height, 80.0);
        assert!(list.ops.is_empty());
        assert!(issues.is_empty());
    }

    #[test]
    fn a_filled_rectangle_keeps_its_box_and_place() {
        let (list, issues) = build(&one_node(
            r##"{"type":"rectangle","cornerRadius":4}"##,
            r##"{"fill":{"value":"#ff0000"}}"##,
            r##"{"x":10,"y":20,"width":30,"height":40}"##,
        ));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(list.ops.len(), 1);
        let op = &list.ops[0];
        assert_eq!(
            op.shape,
            Shape::Rect {
                width: 30.0,
                height: 40.0,
                corner_radius: 4.0
            }
        );
        assert_eq!(op.fill, Some(Paint::Solid(Color::rgb(255, 0, 0))));
        assert_eq!(op.transform.apply(0.0, 0.0), (10.0, 20.0));
    }

    #[test]
    fn a_group_offsets_its_children() {
        let (list, _) = build(&document(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"g","kind":{"type":"group"},
                "transform":{"x":10,"y":0,"width":10,"height":10},
                "children":[{"id":"c","kind":{"type":"ellipse"},
                "transform":{"x":5,"y":0,"width":2,"height":2},
                "style":{"fill":{"value":"#00ff00"}}}]}]}"##,
        ));
        assert_eq!(list.ops.len(), 1);
        assert_eq!(list.ops[0].transform.apply(0.0, 0.0), (15.0, 0.0));
    }

    #[test]
    fn a_hidden_node_paints_nothing_including_its_children() {
        let (list, _) = build(&document(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"g","kind":{"type":"group"},"visible":false,
                "children":[{"id":"c","kind":{"type":"rectangle"},
                "transform":{"width":2,"height":2},"style":{"fill":{"value":"#00ff00"}}}]}]}"##,
        ));
        assert!(list.ops.is_empty());
    }

    #[test]
    fn a_palette_reference_is_the_paint() {
        let (list, issues) = build(&document(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "palette":{"accent":{"name":"Accent","value":{"kind":"solid","color":"#0000ff"}}},
                "nodes":[{"id":"n1","kind":{"type":"rectangle"},
                "transform":{"width":10,"height":10},"style":{"fill":{"ref":"palette.accent"}}}]}"##,
        ));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(list.ops[0].fill, Some(Paint::Solid(Color::rgb(0, 0, 255))));
    }

    #[test]
    fn an_undeclared_token_is_a_reason_not_a_paint() {
        let (list, issues) = build(&unvalidated(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"n1","kind":{"type":"rectangle"},
                "transform":{"width":10,"height":10},
                "style":{"fill":{"ref":"palette.missing"}}}]}"##,
        ));
        assert!(list.ops.is_empty());
        assert_eq!(issues[0].code, IssueCode::UnresolvedGlobal);
        assert!(issues[0].detail.contains("palette.missing"));
    }

    #[test]
    fn a_colour_it_cannot_read_is_refused_with_its_text() {
        let (list, issues) = build(&one_node(
            r##"{"type":"rectangle"}"##,
            r##"{"fill":{"value":"rebeccapurple"}}"##,
            r##"{"width":10,"height":10}"##,
        ));
        assert!(list.ops.is_empty());
        assert_eq!(issues[0].code, IssueCode::UnreadablePaint);
        assert!(issues[0].detail.contains("rebeccapurple"));
    }

    #[test]
    fn a_stroke_without_a_width_still_paints_its_fill() {
        let (list, _) = build(&one_node(
            r##"{"type":"rectangle"}"##,
            r##"{"fill":{"value":"#ffffff"},"stroke":{"value":"#000000"}}"##,
            r##"{"width":10,"height":10}"##,
        ));
        assert_eq!(list.ops[0].stroke.as_ref().map(|stroke| stroke.width), Some(0.0));
        assert!(list.ops[0].fill.is_some());
    }

    #[test]
    fn opacity_multiplies_through_a_group() {
        let (list, _) = build(&document(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"g","kind":{"type":"group"},"style":{"opacity":{"value":0.5}},
                "children":[{"id":"c","kind":{"type":"rectangle"},
                "transform":{"width":2,"height":2},
                "style":{"fill":{"value":"#ffffff"},"opacity":{"value":0.5}}}]}]}"##,
        ));
        assert_eq!(
            list.ops[0].fill,
            Some(Paint::Solid(Color::rgb(255, 255, 255).with_alpha(64)))
        );
    }

    #[test]
    fn a_gradient_axis_spans_its_own_box() {
        let (list, issues) = build(&document(
            r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":200,"height":100},
                "palette":{"plate":{"name":"Plate","value":{"kind":"gradient","angle":90,
                "stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}}},
                "nodes":[{"id":"n1","kind":{"type":"rectangle"},
                "transform":{"width":100,"height":50},"style":{"fill":{"ref":"palette.plate"}}}]}"##,
        ));
        assert!(issues.is_empty(), "{issues:?}");
        let Some(Paint::Linear { from, to, stops }) = &list.ops[0].fill else {
            panic!("expected a gradient");
        };
        // 90 reads as bottom-to-top in the canvas's y-down space.
        assert!((from.0 - 50.0).abs() < 1e-3 && (from.1 - 50.0).abs() < 1e-3, "{from:?}");
        assert!((to.0 - 50.0).abs() < 1e-3 && to.1.abs() < 1e-3, "{to:?}");
        assert_eq!(stops.len(), 2);
    }
}
