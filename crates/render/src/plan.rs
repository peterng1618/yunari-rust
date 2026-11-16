//! Authored state → paint geometry.
//!
//! Every decision a backend must not make lives here: which paint a style
//! names, where a node's box sits once its parents have had their say, and why
//! a value could not be drawn. A node that cannot be painted becomes an issue
//! with a reason, never a guess.

use yunari_model::document::{
    ChartFamily, Node, NodeKind, StyleMap, StyleValue, TextAlign, TextContent, TextOverflow,
    TextRun, ThemeDocument, TypeWeight, VerticalAlign,
};

use crate::affine::Affine;
use crate::color::{Color, parse_color};
use crate::display_list::{DisplayList, GradientStop, Op, Paint, Shape, Stroke};
use yunari_providers::units::UnitPreferences;

use crate::reading::{self, Readings};
use crate::text::{
    Fonts, TextAlign as PlanTextAlign, TextOverflow as PlanTextOverflow, TextRequest, TextRunPlan,
    TextStyle, VerticalAlign as PlanVerticalAlign,
};

/// What a run reads when its value has no sample. The em dash is the product's
/// own placeholder, not an empty string that would look like a real reading.
pub const MISSING_VALUE_TEXT: &str = "\u{2014}";

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
    /// A value run names a binding or key nothing can answer.
    UnmappedKey,
    /// A key is bound, but this frame has no reading for it.
    MissingReading,
    /// A binding asked for a unit the key cannot be shown in.
    UnsupportedUnit,
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

/// Build the frame's geometry and the reasons anything in it is missing, with
/// no readings: every value run draws the placeholder.
pub fn build(document: &ThemeDocument, fonts: &mut Fonts) -> (DisplayList, Vec<PlanIssue>) {
    build_with(
        document,
        fonts,
        &Readings::default(),
        &UnitPreferences::default(),
    )
}

/// Build the frame's geometry with the readings a poll returned and the
/// consumer's unit choices applied at presentation.
pub fn build_with(
    document: &ThemeDocument,
    fonts: &mut Fonts,
    readings: &Readings,
    preferences: &UnitPreferences,
) -> (DisplayList, Vec<PlanIssue>) {
    let width = document.artboard.width as f32;
    let height = document.artboard.height as f32;
    let mut issues = Vec::new();
    let background = resolve_background(document, width, height, &mut issues);
    let backdrop = resolve_backdrop(document, &mut issues);
    let mut list = DisplayList {
        width,
        height,
        background,
        backdrop,
        ops: Vec::new(),
    };
    for node in &document.nodes {
        walk(
            document,
            fonts,
            node,
            Affine::IDENTITY,
            1.0,
            readings,
            preferences,
            &mut list,
            &mut issues,
        );
    }
    (list, issues)
}

/// The page's picture, when the theme declares one. A simple background keeps
/// no composition: it covers the box.
fn resolve_backdrop(
    document: &ThemeDocument,
    issues: &mut Vec<PlanIssue>,
) -> Option<crate::Backdrop> {
    let media = document.background.media.as_ref()?;
    let Some(asset) = document
        .assets
        .iter()
        .find(|asset| asset.id == media.asset_id)
    else {
        issues.push(PlanIssue {
            code: IssueCode::UnresolvedGlobal,
            node_id: None,
            detail: format!("background media asset {} is not declared.", media.asset_id),
        });
        return None;
    };
    Some(crate::Backdrop {
        asset_id: media.asset_id.clone(),
        path: asset.path.clone(),
        composition: crate::background::active(&document.background).cloned(),
        geometry: crate::background::Geometry::of(Some(media)),
        flip_x: media.flip_x,
        flip_y: media.flip_y,
    })
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

#[allow(clippy::too_many_arguments)]
fn walk(
    document: &ThemeDocument,
    fonts: &mut Fonts,
    node: &Node,
    parent: Affine,
    opacity: f32,
    readings: &Readings,
    preferences: &UnitPreferences,
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

    let mut carries_own_paint = false;
    let mut authored_stroke_width = 0.0_f32;
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
        NodeKind::Line => Some(Shape::Line { length: box_width }),
        NodeKind::Path {
            data,
            stroke_width,
            closed,
        } => match crate::path::parse(data) {
            Ok(commands) => {
                authored_stroke_width = *stroke_width as f32;
                Some(Shape::Path {
                    commands,
                    closed: *closed,
                })
            }
            Err(error) => {
                issues.push(PlanIssue {
                    code: IssueCode::UnsupportedContent,
                    node_id: Some(node.id.clone()),
                    detail: match error {
                        crate::path::PathError::Syntax(detail) => {
                            format!("Path data is not readable: {detail}")
                        }
                        crate::path::PathError::Unsupported(detail) => detail.into(),
                    },
                });
                None
            }
        },
        NodeKind::Text { content } => {
            carries_own_paint = true;
            text_shape(
                document,
                fonts,
                node,
                content,
                &style,
                opacity,
                readings,
                preferences,
                issues,
            )
        }
        NodeKind::Chart { content } => {
            let request = crate::chart::ChartRequest {
                document,
                node_id: &node.id,
                width: box_width,
                height: box_height,
                reading: None,
                opacity,
                settings: &content.settings.0,
            };
            let drawn = match content.family {
                ChartFamily::Gauge => crate::chart::gauge::build(&request, fonts, issues),
                ChartFamily::Line => crate::chart::line::build(&request, fonts, issues),
                ChartFamily::Bar => crate::chart::bar::build(&request, fonts, issues),
                ChartFamily::Pie => crate::chart::pie::build(&request, fonts, issues),
                ChartFamily::Radar => crate::chart::radar::build(&request, fonts, issues),
                ChartFamily::PolarBar => crate::chart::polar_bar::build(&request, fonts, issues),
            };
            list.ops.extend(drawn.into_iter().map(|mut op| {
                op.transform = transform;
                op
            }));
            None
        }
        NodeKind::Image { content } => {
            carries_own_paint = true;
            let monochrome = content.monochrome.as_ref().and_then(|value| {
                match resolve_paint(
                    document,
                    value,
                    Some(&node.id),
                    &format!("nodes/{}/image/monochrome", node.id),
                    issues,
                )? {
                    PaintSpec::Solid(color) => Some(scale_alpha(color, opacity)),
                    // A mask is one colour; a ramp would have to choose a
                    // pixel's colour from its position, which is not a mask.
                    PaintSpec::Gradient { .. } => {
                        issues.push(PlanIssue {
                            code: IssueCode::UnreadablePaint,
                            node_id: Some(node.id.clone()),
                            detail: "A monochrome mask is a single colour.".into(),
                        });
                        None
                    }
                }
            });
            match document
                .assets
                .iter()
                .find(|asset| asset.id == content.asset_id)
            {
                Some(asset) => Some(Shape::Image {
                    asset_id: content.asset_id.clone(),
                    path: asset.path.clone(),
                    width: box_width,
                    height: box_height,
                    fit: content.fit,
                    monochrome,
                }),
                None => {
                    issues.push(PlanIssue {
                        code: IssueCode::UnresolvedGlobal,
                        node_id: Some(node.id.clone()),
                        detail: format!("asset {} is not declared.", content.asset_id),
                    });
                    None
                }
            }
        }
        NodeKind::Video { .. } => {
            issues.push(PlanIssue {
                code: IssueCode::UnsupportedContent,
                node_id: Some(node.id.clone()),
                detail: "Video needs a media player, which this frame does not open.".into(),
            });
            None
        }
    };

    if let Some(shape) = shape {
        let (fill, stroke) = if carries_own_paint {
            (None, None)
        } else {
            (
                style
                    .fill
                    .map(|spec| place_paint(spec, box_width, box_height, opacity)),
                style.stroke.map(|spec| Stroke {
                    paint: place_paint(spec, box_width, box_height, opacity),
                    // A path's own width is its authoring default; a style
                    // stroke width overrides it.
                    width: if style.stroke_width > 0.0 {
                        style.stroke_width
                    } else {
                        authored_stroke_width
                    },
                }),
            )
        };
        // A frosted panel samples what is painted beneath it, so its blur is
        // its own op, placed before the panel's fill.
        if let Some(glass) = &node.glass
            && glass.blur_radius > 0.0
        {
            list.ops.push(Op {
                transform,
                shape: Shape::Glass {
                    shape: Box::new(shape.clone()),
                    blur_radius: glass.blur_radius as f32,
                },
                fill: None,
                stroke: None,
            });
        }
        if fill.is_some() || stroke.is_some() || carries_own_paint {
            list.ops.push(Op {
                transform,
                shape,
                fill,
                stroke,
            });
        }
    }

    for child in &node.children {
        walk(
            document,
            fonts,
            child,
            transform,
            opacity,
            readings,
            preferences,
            list,
            issues,
        );
    }
}

/// A text node's paragraph, arranged for the box it declares.
///
/// A value run reads the product's placeholder until a sample source exists:
/// the plan has no clock and no providers yet, and a made-up number would be
/// worse than a dash.
#[allow(clippy::too_many_arguments)]
fn text_shape(
    document: &ThemeDocument,
    fonts: &mut Fonts,
    node: &Node,
    content: &TextContent,
    node_style: &ResolvedStyle,
    opacity: f32,
    readings: &Readings,
    preferences: &UnitPreferences,
    issues: &mut Vec<PlanIssue>,
) -> Option<Shape> {
    let mut runs = Vec::new();
    for run in &content.runs {
        let (text, type_token, style) = match run {
            TextRun::Literal {
                text,
                type_token,
                style,
            } => (text.clone(), type_token, style),
            TextRun::Value {
                type_token, style, ..
            } => (
                reading::value_text(document, node, run, readings, preferences, issues),
                type_token,
                style,
            ),
        };
        if text.is_empty() {
            continue;
        }
        runs.push(TextRunPlan {
            text,
            style: run_style(
                document, node, type_token, style, node_style, opacity, issues,
            ),
        });
    }
    if runs.is_empty() {
        return None;
    }

    let request = TextRequest {
        runs: &runs,
        box_: content
            .box_
            .map(|box_| (box_.width as f32, box_.height as f32)),
        align: match content.align {
            TextAlign::Left => PlanTextAlign::Left,
            TextAlign::Center => PlanTextAlign::Center,
            TextAlign::Right => PlanTextAlign::Right,
        },
        vertical_align: match content.vertical_align {
            VerticalAlign::Top => PlanVerticalAlign::Top,
            VerticalAlign::Middle => PlanVerticalAlign::Middle,
            VerticalAlign::Bottom => PlanVerticalAlign::Bottom,
        },
        overflow: match content.overflow {
            TextOverflow::Visible => PlanTextOverflow::Visible,
            TextOverflow::Clip => PlanTextOverflow::Clip,
            TextOverflow::Ellipsis => PlanTextOverflow::Ellipsis,
        },
        vertical: content.vertical,
        line_height: document_line_height(node),
    };
    Some(Shape::Text(crate::text::layout(fonts, &request)))
}

fn document_line_height(node: &Node) -> f32 {
    node.style
        .get("lineHeight")
        .and_then(style_number)
        .map_or(1.2, |line_height| line_height as f32)
}

/// The face, size and colour a run paints with: its type token's family and
/// weight, the size it authors or the role default, and the first colour that
/// applies to it.
fn run_style(
    document: &ThemeDocument,
    node: &Node,
    type_token: &str,
    style: &StyleMap,
    node_style: &ResolvedStyle,
    opacity: f32,
    issues: &mut Vec<PlanIssue>,
) -> TextStyle {
    let path = |key: &str| format!("nodes/{}/kind/content/runs/{key}", node.id);
    let token = document
        .type_tokens
        .get(type_token.strip_prefix("typeTokens.").unwrap_or(type_token));
    let size = style.get("fontSize").and_then(style_number).map_or_else(
        || default_type_size(type_token, document.artboard.height),
        |size| size as f32,
    );
    let colour = style
        .get("fill")
        .and_then(|value| resolve_paint(document, value, Some(&node.id), &path("fill"), issues))
        .or(node_style.fill.clone())
        .and_then(|spec| match spec {
            PaintSpec::Solid(color) => Some(scale_alpha(color, opacity)),
            PaintSpec::Gradient { stops, .. } => {
                issues.push(PlanIssue {
                    code: IssueCode::UnsupportedContent,
                    node_id: Some(node.id.clone()),
                    detail: "A gradient text run paints its first stop.".into(),
                });
                stops.first().map(|stop| scale_alpha(stop.color, opacity))
            }
        })
        .unwrap_or(Color::rgb(255, 255, 255));
    TextStyle {
        family: token.map(|token| token.family.clone()),
        weight: weight_of(token.and_then(|token| token.weight.as_ref())),
        italic: false,
        size,
        color: colour,
    }
}

/// A run that authors no size takes the role's share of the artboard height:
/// heading 1/15, body 1/67.5, mono 1/77 — 72/16/14 at 1080. The role comes
/// from the token's id, and an id that names no role takes the body ratio.
fn default_type_size(type_token: &str, artboard_height: f64) -> f32 {
    if !artboard_height.is_finite() {
        return 1.0;
    }
    let id = type_token.strip_prefix("typeTokens.").unwrap_or(type_token);
    let ratio = match id {
        "heading" => 1.0 / 15.0,
        "mono" => 1.0 / 77.0,
        _ => 1.0 / 67.5,
    };
    (artboard_height * ratio).round().max(1.0) as f32
}

/// A named weight as the CSS number it stands for. An unknown name is regular
/// rather than an error: the document's own validator owns the vocabulary.
fn weight_of(weight: Option<&TypeWeight>) -> u16 {
    match weight {
        Some(TypeWeight::Number(weight)) => weight.round().clamp(1.0, 1000.0) as u16,
        Some(TypeWeight::Name(name)) => match name.to_ascii_lowercase().as_str() {
            "thin" => 100,
            "extralight" | "extra-light" | "ultralight" => 200,
            "light" => 300,
            "medium" => 500,
            "semibold" | "semi-bold" | "demibold" => 600,
            "bold" => 700,
            "extrabold" | "extra-bold" | "ultrabold" => 800,
            "black" | "heavy" => 900,
            _ => 400,
        },
        None => 400,
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
    let stroke = style
        .get("stroke")
        .and_then(|value| resolve_paint(document, value, Some(&node.id), &path("stroke"), issues));
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
    use yunari_providers::sample::Reading;

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
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},"nodes":[]}"##,
            ),
            &mut Fonts::new(),
        );
        assert_eq!(list.width, 100.0);
        assert_eq!(list.height, 80.0);
        assert!(list.ops.is_empty());
        assert!(issues.is_empty());
    }

    #[test]
    fn a_filled_rectangle_keeps_its_box_and_place() {
        let (list, issues) = build(
            &one_node(
                r##"{"type":"rectangle","cornerRadius":4}"##,
                r##"{"fill":{"value":"#ff0000"}}"##,
                r##"{"x":10,"y":20,"width":30,"height":40}"##,
            ),
            &mut Fonts::new(),
        );
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
        let (list, _) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"g","kind":{"type":"group"},
                "transform":{"x":10,"y":0,"width":10,"height":10},
                "children":[{"id":"c","kind":{"type":"ellipse"},
                "transform":{"x":5,"y":0,"width":2,"height":2},
                "style":{"fill":{"value":"#00ff00"}}}]}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert_eq!(list.ops.len(), 1);
        assert_eq!(list.ops[0].transform.apply(0.0, 0.0), (15.0, 0.0));
    }

    #[test]
    fn a_hidden_node_paints_nothing_including_its_children() {
        let (list, _) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"g","kind":{"type":"group"},"visible":false,
                "children":[{"id":"c","kind":{"type":"rectangle"},
                "transform":{"width":2,"height":2},"style":{"fill":{"value":"#00ff00"}}}]}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(list.ops.is_empty());
    }

    #[test]
    fn a_palette_reference_is_the_paint() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "palette":{"accent":{"name":"Accent","value":{"kind":"solid","color":"#0000ff"}}},
                "nodes":[{"id":"n1","kind":{"type":"rectangle"},
                "transform":{"width":10,"height":10},"style":{"fill":{"ref":"palette.accent"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(list.ops[0].fill, Some(Paint::Solid(Color::rgb(0, 0, 255))));
    }

    #[test]
    fn an_undeclared_token_is_a_reason_not_a_paint() {
        let (list, issues) = build(
            &unvalidated(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"n1","kind":{"type":"rectangle"},
                "transform":{"width":10,"height":10},
                "style":{"fill":{"ref":"palette.missing"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(list.ops.is_empty());
        assert_eq!(issues[0].code, IssueCode::UnresolvedGlobal);
        assert!(issues[0].detail.contains("palette.missing"));
    }

    #[test]
    fn a_colour_it_cannot_read_is_refused_with_its_text() {
        let (list, issues) = build(
            &one_node(
                r##"{"type":"rectangle"}"##,
                r##"{"fill":{"value":"rebeccapurple"}}"##,
                r##"{"width":10,"height":10}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(list.ops.is_empty());
        assert_eq!(issues[0].code, IssueCode::UnreadablePaint);
        assert!(issues[0].detail.contains("rebeccapurple"));
    }

    #[test]
    fn a_stroke_without_a_width_still_paints_its_fill() {
        let (list, _) = build(
            &one_node(
                r##"{"type":"rectangle"}"##,
                r##"{"fill":{"value":"#ffffff"},"stroke":{"value":"#000000"}}"##,
                r##"{"width":10,"height":10}"##,
            ),
            &mut Fonts::new(),
        );
        assert_eq!(
            list.ops[0].stroke.as_ref().map(|stroke| stroke.width),
            Some(0.0)
        );
        assert!(list.ops[0].fill.is_some());
    }

    #[test]
    fn opacity_multiplies_through_a_group() {
        let (list, _) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                "nodes":[{"id":"g","kind":{"type":"group"},"style":{"opacity":{"value":0.5}},
                "children":[{"id":"c","kind":{"type":"rectangle"},
                "transform":{"width":2,"height":2},
                "style":{"fill":{"value":"#ffffff"},"opacity":{"value":0.5}}}]}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert_eq!(
            list.ops[0].fill,
            Some(Paint::Solid(Color::rgb(255, 255, 255).with_alpha(64)))
        );
    }

    #[test]
    fn a_gradient_axis_spans_its_own_box() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":200,"height":100},
                "palette":{"plate":{"name":"Plate","value":{"kind":"gradient","angle":90,
                "stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}}},
                "nodes":[{"id":"n1","kind":{"type":"rectangle"},
                "transform":{"width":100,"height":50},"style":{"fill":{"ref":"palette.plate"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let Some(Paint::Linear { from, to, stops }) = &list.ops[0].fill else {
            panic!("expected a gradient");
        };
        // 90 reads as bottom-to-top in the canvas's y-down space.
        assert!(
            (from.0 - 50.0).abs() < 1e-3 && (from.1 - 50.0).abs() < 1e-3,
            "{from:?}"
        );
        assert!((to.0 - 50.0).abs() < 1e-3 && to.1.abs() < 1e-3, "{to:?}");
        assert_eq!(stops.len(), 2);
    }

    #[test]
    fn a_path_node_paints_its_authored_commands() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                    "nodes":[{"id":"n1","transform":{"width":20,"height":20},
                    "kind":{"type":"path","data":"M0 0 L10 0 L10 10 Z","strokeWidth":2},
                    "style":{"stroke":{"value":"#00ff00"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let Some(Shape::Path { commands, closed }) = list.ops.first().map(|op| &op.shape) else {
            panic!("expected a path");
        };
        assert_eq!(commands.len(), 4);
        assert!(!closed);
        assert_eq!(
            list.ops[0].stroke.as_ref().map(|stroke| stroke.width),
            Some(2.0)
        );
    }

    #[test]
    fn unreadable_path_data_is_a_reason_not_a_shape() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":80},
                    "nodes":[{"id":"n1","transform":{"width":20,"height":20},
                    "kind":{"type":"path","data":"M0 0 A 5 5 0 0 1 10 0"},
                    "style":{"stroke":{"value":"#00ff00"},"strokeWidth":{"value":1}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(list.ops.is_empty());
        assert_eq!(issues[0].code, IssueCode::UnsupportedContent);
        assert!(issues[0].detail.contains("arc"));
    }

    #[test]
    fn a_gauge_node_paints_its_ring_under_the_node_transform() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":400,"height":400},
                    "nodes":[{"id":"g","transform":{"x":50,"y":60,"width":200,"height":200},
                    "kind":{"type":"chart","content":{"family":"gauge",
                    "settings":{"thickness":16}}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(list.ops.len(), 1);
        assert_eq!(list.ops[0].transform.apply(0.0, 0.0), (50.0, 60.0));
    }

    #[test]
    fn a_video_node_says_it_needs_a_player() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":400,"height":400},
                    "assets":[{"id":"clip","kind":"video","path":"assets/clip.mp4"}],
                    "nodes":[{"id":"v","transform":{"width":200,"height":200},
                    "kind":{"type":"video","content":{"assetId":"clip"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(list.ops.is_empty());
        assert_eq!(issues[0].code, IssueCode::UnsupportedContent);
    }

    #[test]
    fn a_glass_panel_blurs_before_it_fills() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":100,"height":100},
                    "nodes":[
                    {"id":"behind","kind":{"type":"rectangle"},
                    "transform":{"width":100,"height":100},
                    "style":{"fill":{"value":"#ff0000"}}},
                    {"id":"panel","kind":{"type":"rectangle"},
                    "transform":{"x":10,"y":10,"width":40,"height":40},
                    "glass":{"blurRadius":12},
                    "style":{"fill":{"value":"rgba(255, 255, 255, 0.2)"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(list.ops.len(), 3);
        assert!(
            matches!(list.ops[1].shape, Shape::Glass { blur_radius, .. } if blur_radius == 12.0)
        );
        assert_eq!(list.ops[1].transform.apply(0.0, 0.0), (10.0, 10.0));
        assert!(list.ops[2].fill.is_some());
    }

    #[test]
    fn a_zero_glass_blur_draws_nothing() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":10,"height":10},
                    "nodes":[{"id":"panel","kind":{"type":"rectangle"},
                    "transform":{"width":10,"height":10},
                    "glass":{"blurRadius":0},
                    "style":{"fill":{"value":"#ff0000"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(list.ops.len(), 1);
        assert!(!matches!(list.ops[0].shape, Shape::Glass { .. }));
    }

    #[test]
    fn an_image_node_carries_its_asset_fit_and_box() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":400,"height":400},
                    "assets":[{"id":"logo","kind":"image","path":"assets/logo.png"}],
                    "nodes":[{"id":"i","transform":{"x":10,"y":20,"width":120,"height":80},
                    "kind":{"type":"image","content":{"assetId":"logo","fit":"cover"}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let Some(Shape::Image {
            asset_id,
            path,
            width,
            height,
            fit,
            monochrome,
        }) = list.ops.first().map(|op| &op.shape)
        else {
            panic!("expected an image op");
        };
        assert_eq!(asset_id, "logo");
        assert_eq!(path, "assets/logo.png");
        assert_eq!((*width, *height), (120.0, 80.0));
        assert_eq!(*fit, yunari_model::document::ImageFit::Cover);
        assert_eq!(*monochrome, None);
        assert_eq!(list.ops[0].transform.apply(0.0, 0.0), (10.0, 20.0));
    }

    #[test]
    fn an_image_mask_resolves_its_palette_colour_and_node_opacity() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":400,"height":400},
                    "palette":{"ink":{"name":"Ink","value":{"kind":"solid","color":"#ff0000"}}},
                    "assets":[{"id":"icon","kind":"image","path":"assets/icon.png"}],
                    "nodes":[{"id":"i","transform":{"width":40,"height":40},
                    "style":{"opacity":{"value":0.5}},
                    "kind":{"type":"image","content":{"assetId":"icon",
                    "monochrome":{"ref":"palette.ink"}}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let Some(Shape::Image { monochrome, .. }) = list.ops.first().map(|op| &op.shape) else {
            panic!("expected an image op");
        };
        assert_eq!(*monochrome, Some(Color::rgb(255, 0, 0).with_alpha(128)));
    }

    #[test]
    fn a_literal_run_becomes_placed_text() {
        let (list, issues) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":450,"height":675},
                    "typeTokens":{"body":{"family":"Inter"}},
                    "nodes":[{"id":"n1","transform":{"x":10,"y":20,"width":200,"height":40},
                    "kind":{"type":"text","content":{"runs":[
                        {"kind":"literal","text":"Hello","typeToken":"typeTokens.body"}]}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        assert!(issues.is_empty(), "{issues:?}");
        let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
            panic!("expected a text op");
        };
        assert_eq!(layout.segments.len(), 1);
        assert_eq!(layout.segments[0].text, "Hello");
        assert_eq!(layout.segments[0].style.family.as_deref(), Some("Inter"));
        // 675 / 67.5 is the body role's share of the artboard height.
        assert_eq!(layout.segments[0].style.size, 10.0);
    }

    #[test]
    fn a_run_authored_size_wins_over_the_role_default() {
        let (list, _) = build(
            &document(
                r##"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":450,"height":675},
                    "typeTokens":{"body":{"family":"Inter"}},
                    "nodes":[{"id":"n1","transform":{"width":200,"height":40},
                    "kind":{"type":"text","content":{"runs":[
                        {"kind":"literal","text":"Hi","typeToken":"typeTokens.body","style":{"fontSize":{"value":42}}}]}}}]}"##,
            ),
            &mut Fonts::new(),
        );
        let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
            panic!("expected a text op");
        };
        assert_eq!(layout.segments[0].style.size, 42.0);
    }

    fn value_run_document(binding: &str) -> ThemeDocument {
        document(&format!(
            r##"{{"schemaVersion":"1.0.0","id":"t","artboard":{{"width":450,"height":675}},
                "typeTokens":{{"body":{{"family":"Inter"}}}},
                "nodes":[{{"id":"n1","transform":{{"width":200,"height":40}},
                "kind":{{"type":"text","content":{{"runs":[
                    {{"kind":"value","bindingId":"b","typeToken":"typeTokens.body"}}]}}}}}}],
                "bindings":{{"n1":[{binding}]}}}}"##
        ))
    }

    fn value_run_text(document: &ThemeDocument, readings: &Readings) -> (String, Vec<PlanIssue>) {
        let (list, issues) = build_with(
            document,
            &mut Fonts::new(),
            readings,
            &UnitPreferences::default(),
        );
        let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
            panic!("expected a text op");
        };
        (layout.segments[0].text.clone(), issues)
    }

    fn reading(semantic_key: &str, value: f64, unit: &str) -> Reading {
        Reading::number(semantic_key, value, unit)
    }

    #[test]
    fn a_value_run_with_no_reading_reads_the_placeholder_and_says_which_key() {
        let document = value_run_document(r#"{"id":"b","semanticKey":"cpu.load"}"#);
        let (text, issues) = value_run_text(&document, &Readings::default());
        assert_eq!(text, MISSING_VALUE_TEXT);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, IssueCode::MissingReading);
        assert!(issues[0].detail.contains("cpu.load"));
    }

    #[test]
    fn a_reading_is_written_with_its_unit() {
        let document = value_run_document(r#"{"id":"b","semanticKey":"cpu.load"}"#);
        let readings = Readings::new([reading("cpu.load", 42.0, "%")]);
        let (text, issues) = value_run_text(&document, &readings);
        assert_eq!(text, "42%");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn a_binding_scale_acts_once_before_the_author_s_precision() {
        let document = value_run_document(
            r#"{"id":"b","semanticKey":"custom.probe.value","scale":2,"precision":2}"#,
        );
        let readings = Readings::new([reading("custom.probe.value", 1.5, "MB")]);
        let (text, issues) = value_run_text(&document, &readings);
        assert_eq!(text, "3.00 MB");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn an_unsupported_unit_keeps_the_measured_value_and_names_the_unit() {
        let document = value_run_document(r#"{"id":"b","semanticKey":"cpu.load","unit":"GiB"}"#);
        let readings = Readings::new([reading("cpu.load", 42.0, "%")]);
        let (text, issues) = value_run_text(&document, &readings);
        assert_eq!(text, "42%");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, IssueCode::UnsupportedUnit);
        assert!(issues[0].detail.contains("GiB"));
    }

    #[test]
    fn an_instant_reads_with_the_author_s_format() {
        let document =
            value_run_document(r#"{"id":"b","semanticKey":"time.now","format":"HH:mm"}"#);
        let readings = Readings::new([Reading::instant("time.now", "2026-09-24T23:07:09+09:00")]);
        let (text, issues) = value_run_text(&document, &readings);
        assert_eq!(text, "23:07");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn a_missing_reading_reports_why_it_is_missing() {
        let document = value_run_document(r#"{"id":"b","semanticKey":"cpu.load"}"#);
        let readings = Readings::new([Reading::missing("cpu.load", "the sensor is asleep")]);
        let (text, issues) = value_run_text(&document, &readings);
        assert_eq!(text, MISSING_VALUE_TEXT);
        assert_eq!(issues[0].code, IssueCode::MissingReading);
        assert!(issues[0].detail.contains("the sensor is asleep"));
    }
}
