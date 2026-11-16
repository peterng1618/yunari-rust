//! The inspector: the selected node described as sections of editable fields.
//!
//! A field is one key, a label and the text of its current value. Writing goes
//! back through one function, [`apply`], so every row edits the document the
//! same way and a refused value changes nothing.

use yunari_model::document::{
    ImageFit, Node, NodeKind, StyleLiteral, StyleMap, StyleRef, StyleValue, TextAlign, TextContent,
    TextOverflow, TextRun, ThemeDocument, VerticalAlign,
};

use crate::selection::find_mut;

/// The smallest a side may be; the map and every drag share it.
pub const MIN_SIZE: f64 = 1.0;
/// How far a scale may grow, matching the camera's zoom ceiling.
pub const MAX_SCALE: f64 = 64.0;

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Number { min: f64, max: f64, step: f64 },
    Text,
    Colour,
    Choice(Vec<&'static str>),
    Toggle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: String,
    pub label: &'static str,
    pub kind: FieldKind,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Field>,
}

/// The selected node's fields, in the order the panel shows them.
pub fn sections(document: &ThemeDocument, id: &str) -> Vec<Section> {
    let Some(node) = crate::selection::find(document, id) else {
        return Vec::new();
    };
    let transform = node.transform;
    let mut sections = vec![Section {
        title: "Transform",
        rows: vec![
            number("transform.x", "X", transform.x, None),
            number("transform.y", "Y", transform.y, None),
            number(
                "transform.width",
                "Width",
                transform.width,
                Some(MIN_SIZE),
            ),
            number(
                "transform.height",
                "Height",
                transform.height,
                Some(MIN_SIZE),
            ),
            number("transform.rotation", "Rotation", transform.rotation, Some(0.0)),
            number("transform.scaleX", "Scale X", transform.scale_x, Some(0.000_001)),
            number("transform.scaleY", "Scale Y", transform.scale_y, Some(0.000_001)),
        ],
    }];
    sections.push(Section {
        title: "Appearance",
        rows: vec![
            colour("style.fill", "Fill", node.style.get("fill")),
            colour("style.stroke", "Stroke", node.style.get("stroke")),
            Field {
                key: "style.strokeWidth".to_owned(),
                label: "Stroke width",
                kind: FieldKind::Number {
                    min: 0.0,
                    max: 999.0,
                    step: 1.0,
                },
                value: format_number(style_number(node, "strokeWidth").unwrap_or(0.0)),
            },
            Field {
                key: "style.opacity".to_owned(),
                label: "Opacity",
                kind: FieldKind::Number {
                    min: 0.0,
                    max: 1.0,
                    step: 0.1,
                },
                value: format_number(style_number(node, "opacity").unwrap_or(1.0)),
            },
        ],
    });
    match &node.kind {
        NodeKind::Rectangle { corner_radius } => sections.push(Section {
            title: "Rectangle",
            rows: vec![Field {
                key: "kind.cornerRadius".to_owned(),
                label: "Corner radius",
                kind: FieldKind::Number {
                    min: 0.0,
                    max: 999.0,
                    step: 1.0,
                },
                value: format_number(*corner_radius),
            }],
        }),
        NodeKind::Image { content } => sections.push(Section {
            title: "Image",
            rows: vec![Field {
                key: "kind.content.fit".to_owned(),
                label: "Fit",
                kind: FieldKind::Choice(vec!["cover", "contain", "stretch"]),
                value: match content.fit {
                    ImageFit::Cover => "cover".to_owned(),
                    ImageFit::Contain => "contain".to_owned(),
                    ImageFit::Stretch => "stretch".to_owned(),
                },
            }],
        }),
        NodeKind::Video { content } => sections.push(Section {
            title: "Video",
            rows: vec![
                toggle("kind.content.loop", "Loop", content.loop_),
                toggle("kind.content.muted", "Muted", content.muted),
            ],
        }),
        NodeKind::Text { content } => sections.push(Section {
            title: "Text",
            rows: text_rows(content),
        }),
        _ => {}
    }
    sections
}

fn text_rows(content: &TextContent) -> Vec<Field> {
    let mut rows = Vec::new();
    for (index, run) in content.runs.iter().enumerate() {
        let token = match run {
            TextRun::Literal {
                text, type_token, ..
            } => {
                rows.push(Field {
                    key: format!("kind.content.runs.{index}.text"),
                    label: "Text",
                    kind: FieldKind::Text,
                    value: text.clone(),
                });
                type_token
            }
            TextRun::Value { type_token, .. } => type_token,
        };
        rows.push(Field {
            key: format!("kind.content.runs.{index}.typeToken"),
            label: "Token",
            kind: FieldKind::Text,
            value: token.clone(),
        });
    }
    rows.extend([
        choice("kind.content.align", "Align", align_name(content.align)),
        choice("kind.content.overflow", "Overflow", overflow_name(content.overflow)),
        choice(
            "kind.content.verticalAlign",
            "Vertical align",
            vertical_name(content.vertical_align),
        ),
        toggle("kind.content.vertical", "Vertical", content.vertical),
    ]);
    rows
}

/// A choice row: the value list is read from the enum at render time, so the
/// row carries the current option's name.
fn choice(key: &'static str, label: &'static str, value: &str) -> Field {
    Field {
        key: key.to_owned(),
        label,
        kind: FieldKind::Choice(Vec::new()),
        value: value.to_owned(),
    }
}

fn align_name(value: TextAlign) -> &'static str {
    match value {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    }
}

fn overflow_name(value: TextOverflow) -> &'static str {
    match value {
        TextOverflow::Visible => "visible",
        TextOverflow::Clip => "clip",
        TextOverflow::Ellipsis => "ellipsis",
    }
}

fn vertical_name(value: VerticalAlign) -> &'static str {
    match value {
        VerticalAlign::Top => "top",
        VerticalAlign::Middle => "middle",
        VerticalAlign::Bottom => "bottom",
    }
}

fn colour(key: &'static str, label: &'static str, value: Option<&StyleValue>) -> Field {
    Field {
        key: key.to_owned(),
        label,
        kind: FieldKind::Colour,
        value: paint_text(value),
    }
}

/// How a paint reads in a field: its reference, or its literal text.
fn paint_text(value: Option<&StyleValue>) -> String {
    match value {
        Some(StyleValue::Ref(reference)) => reference.r#ref.clone(),
        Some(StyleValue::Literal(literal)) => literal
            .value
            .as_str()
            .map_or_else(String::new, ToOwned::to_owned),
        None => String::new(),
    }
}

fn style_number(node: &Node, key: &str) -> Option<f64> {
    match node.style.get(key) {
        Some(StyleValue::Literal(literal)) => literal.value.as_f64(),
        _ => None,
    }
}

fn toggle(key: &'static str, label: &'static str, value: bool) -> Field {
    Field {
        key: key.to_owned(),
        label,
        kind: FieldKind::Toggle,
        value: if value {
            "true".to_owned()
        } else {
            "false".to_owned()
        },
    }
}

fn number(key: &'static str, label: &'static str, value: f64, min: Option<f64>) -> Field {
    Field {
        key: key.to_owned(),
        label,
        kind: FieldKind::Number {
            min: min.unwrap_or(f64::MIN),
            max: f64::MAX,
            step: 1.0,
        },
        value: format_number(value),
    }
}

/// Whole numbers without a trailing `.0`, everything else as `f64` prints it.
pub fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// Writes one field. A refused value leaves the document untouched.
pub fn apply(document: &mut ThemeDocument, id: &str, key: &str, value: &str) -> Result<(), String> {
    if key.ends_with(".typeToken") {
        let token = token_id(value);
        if !document.type_tokens.contains_key(token) {
            return Err(format!("No type token named {token}"));
        }
    }
    let node = find_mut(&mut document.nodes, id).ok_or_else(|| "No node with that id".to_owned())?;
    apply_to_node(node, key, value)
}

/// The id a run's token field writes: `body` and `typeTokens.body` are the same
/// token, and the document stores the bare id.
fn token_id(value: &str) -> &str {
    let value = value.trim();
    value.strip_prefix("typeTokens.").unwrap_or(value)
}

fn apply_to_node(node: &mut Node, key: &str, value: &str) -> Result<(), String> {
    if key.starts_with("style.") {
        return apply_style(node, key, value);
    }
    if key.starts_with("kind.") {
        return apply_kind(node, key, value);
    }
    let number = || {
        let parsed = value
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("{} needs a number", friendly(key)))?;
        if !parsed.is_finite() {
            return Err(format!("{} needs a finite number", friendly(key)));
        }
        Ok(parsed)
    };
    let transform = &mut node.transform;
    match key {
        "transform.x" => transform.x = number()?,
        "transform.y" => transform.y = number()?,
        "transform.width" => transform.width = at_least(number()?, MIN_SIZE, "Width")?,
        "transform.height" => transform.height = at_least(number()?, MIN_SIZE, "Height")?,
        "transform.rotation" => transform.rotation = number()?.rem_euclid(360.0),
        "transform.scaleX" => transform.scale_x = scale(number()?)?,
        "transform.scaleY" => transform.scale_y = scale(number()?)?,
        _ => return Err(format!("{} is not editable yet", friendly(key))),
    }
    Ok(())
}

/// A paint is `<hex>` or `palette.<id>`; anything else is refused before it can
/// reach a save.
fn paint(value: &str) -> Result<StyleValue, String> {
    let value = value.trim();
    if let Some(id) = value.strip_prefix("palette.") {
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err("A palette reference needs a token id".to_owned());
        }
        return Ok(StyleValue::Ref(StyleRef {
            r#ref: value.to_owned(),
        }));
    }
    let hex = value.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(StyleValue::Literal(StyleLiteral {
            value: serde_json::Value::String(value.to_owned()),
        }));
    }
    Err("A colour is #rrggbb or palette.<id>".to_owned())
}

fn apply_style(node: &mut Node, key: &str, value: &str) -> Result<(), String> {
    let style: &mut StyleMap = &mut node.style;
    match key {
        "style.fill" | "style.stroke" => {
            let name = key.trim_start_matches("style.");
            if value.trim().is_empty() {
                style.remove(name);
                return Ok(());
            }
            let paint = paint(value)?;
            style.insert(name.to_owned(), paint);
        }
        "style.strokeWidth" | "style.opacity" => {
            let parsed = value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("{} needs a number", friendly(key)))?;
            if !parsed.is_finite() || parsed < 0.0 {
                return Err(format!("{} cannot be negative", friendly(key)));
            }
            if key == "style.opacity" && parsed > 1.0 {
                return Err("Opacity runs from 0 to 1".to_owned());
            }
            style.insert(
                key.trim_start_matches("style.").to_owned(),
                StyleValue::Literal(StyleLiteral {
                    value: serde_json::json!(parsed),
                }),
            );
        }
        _ => return Err(format!("{} is not editable yet", friendly(key))),
    }
    Ok(())
}

fn apply_kind(node: &mut Node, key: &str, value: &str) -> Result<(), String> {
    let number = || {
        let parsed = value
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("{} needs a number", friendly(key)))?;
        if !parsed.is_finite() || parsed < 0.0 {
            return Err(format!("{} cannot be negative", friendly(key)));
        }
        Ok(parsed)
    };
    if let Some(rest) = key.strip_prefix("kind.content.runs.") {
        let Some((index, field)) = rest.split_once('.') else {
            return Err("A run field is kind.content.runs.<i>.<field>".to_owned());
        };
        let index: usize = index
            .parse()
            .map_err(|_| "A run index is a whole number".to_owned())?;
        let NodeKind::Text { content } = &mut node.kind else {
            return Err("Only text has runs".to_owned());
        };
        let run = content
            .runs
            .get_mut(index)
            .ok_or_else(|| format!("This node has no run {index}"))?;
        return match (field, run) {
            ("text", TextRun::Literal { text, .. }) => {
                *text = value.to_owned();
                Ok(())
            }
            ("text", TextRun::Value { .. }) => {
                Err("A bound run takes its text from its binding".to_owned())
            }
            ("typeToken", TextRun::Literal { type_token, .. })
            | ("typeToken", TextRun::Value { type_token, .. }) => {
                *type_token = token_id(value).to_owned();
                Ok(())
            }
            _ => Err(format!("{} is not editable yet", friendly(key))),
        };
    }
    match key {
        "kind.cornerRadius" => match &mut node.kind {
            NodeKind::Rectangle { corner_radius } => *corner_radius = number()?,
            _ => return Err("Only a rectangle has a corner radius".to_owned()),
        },
        "kind.content.fit" => match &mut node.kind {
            NodeKind::Image { content } => {
                content.fit = match value.trim() {
                    "cover" => ImageFit::Cover,
                    "contain" => ImageFit::Contain,
                    "stretch" => ImageFit::Stretch,
                    other => return Err(format!("{other} is not a fit")),
                }
            }
            _ => return Err("Only an image has a fit".to_owned()),
        },
        "kind.content.loop" | "kind.content.muted" => match &mut node.kind {
            NodeKind::Video { content } => {
                let flag = match value.trim() {
                    "true" => true,
                    "false" => false,
                    other => return Err(format!("{other} is not true or false")),
                };
                if key == "kind.content.loop" {
                    content.loop_ = flag;
                } else {
                    content.muted = flag;
                }
            }
            _ => return Err("Only a video has playback flags".to_owned()),
        },
        "kind.content.align" | "kind.content.overflow" | "kind.content.verticalAlign"
        | "kind.content.vertical" => match &mut node.kind {
            NodeKind::Text { content } => match (key, value.trim()) {
                ("kind.content.align", "left") => content.align = TextAlign::Left,
                ("kind.content.align", "center") => content.align = TextAlign::Center,
                ("kind.content.align", "right") => content.align = TextAlign::Right,
                ("kind.content.overflow", "visible") => content.overflow = TextOverflow::Visible,
                ("kind.content.overflow", "clip") => content.overflow = TextOverflow::Clip,
                ("kind.content.overflow", "ellipsis") => content.overflow = TextOverflow::Ellipsis,
                ("kind.content.verticalAlign", "top") => {
                    content.vertical_align = VerticalAlign::Top;
                }
                ("kind.content.verticalAlign", "middle") => {
                    content.vertical_align = VerticalAlign::Middle;
                }
                ("kind.content.verticalAlign", "bottom") => {
                    content.vertical_align = VerticalAlign::Bottom;
                }
                ("kind.content.vertical", flag) => {
                    content.vertical = flag
                        .parse::<bool>()
                        .map_err(|_| "Vertical is true or false".to_owned())?;
                }
                (_, other) => return Err(format!("{other} is not a {} value", friendly(key))),
            },
            _ => return Err("Only text has layout fields".to_owned()),
        },
        _ => return Err(format!("{} is not editable yet", friendly(key))),
    }
    Ok(())
}

fn at_least(value: f64, minimum: f64, what: &str) -> Result<f64, String> {
    if value < minimum {
        return Err(format!("{what} must be at least {minimum}"));
    }
    Ok(value)
}

fn scale(value: f64) -> Result<f64, String> {
    if value <= 0.0 {
        return Err("Scale must be greater than zero".to_owned());
    }
    if value > MAX_SCALE {
        return Err(format!("Scale must be at most {MAX_SCALE}"));
    }
    Ok(value)
}

/// A key as a person reads it: the last segment, spaced.
fn friendly(key: &str) -> String {
    key.rsplit('.').next().unwrap_or(key).to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::TypeToken;

    use crate::fixtures::{document, rect};

    use super::*;

    fn text_node() -> Node {
        let mut node = rect("label", 0.0, 0.0, 200.0, 40.0);
        node.kind = NodeKind::Text {
            content: TextContent {
                runs: vec![TextRun::Literal {
                    text: "Hello".to_owned(),
                    type_token: "body".to_owned(),
                    style: BTreeMap::new(),
                }],
                box_: None,
                overflow: TextOverflow::Visible,
                align: TextAlign::Left,
                vertical_align: VerticalAlign::Top,
                vertical: false,
            },
        };
        node
    }

    fn with_token(mut document: ThemeDocument) -> ThemeDocument {
        document.type_tokens.insert("body".to_owned(), TypeToken {
            family: "Inter".to_owned(),
            weight: None,
            face: None,
        });
        document
    }

    #[test]
    fn a_text_run_shows_its_text_token_and_layout() {
        let document = document(vec![text_node()]);
        let section = sections(&document, "label")
            .into_iter()
            .find(|section| section.title == "Text")
            .expect("a text section");
        let keys: Vec<&str> = section.rows.iter().map(|row| row.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "kind.content.runs.0.text",
                "kind.content.runs.0.typeToken",
                "kind.content.align",
                "kind.content.overflow",
                "kind.content.verticalAlign",
                "kind.content.vertical",
            ]
        );
        assert_eq!(section.rows[0].value, "Hello");
        assert_eq!(section.rows[2].value, "left");
    }

    #[test]
    fn run_text_and_a_known_token_are_written() {
        let mut document = with_token(document(vec![text_node()]));
        assert!(apply(&mut document, "label", "kind.content.runs.0.text", "Hi there").is_ok());
        assert!(
            apply(&mut document, "label", "kind.content.runs.0.typeToken", "typeTokens.body")
                .is_ok()
        );
        let NodeKind::Text { content } = &document.nodes[0].kind else {
            panic!("still text");
        };
        let TextRun::Literal {
            text, type_token, ..
        } = &content.runs[0]
        else {
            panic!("still literal");
        };
        assert_eq!(text, "Hi there");
        assert_eq!(type_token, "body");
    }

    #[test]
    fn an_unknown_token_or_run_is_refused() {
        let mut document = with_token(document(vec![text_node()]));
        assert!(apply(&mut document, "label", "kind.content.runs.0.typeToken", "title").is_err());
        assert!(apply(&mut document, "label", "kind.content.runs.7.text", "x").is_err());
        assert!(apply(&mut document, "label", "kind.content.runs.zero.text", "x").is_err());
    }

    #[test]
    fn alignment_and_overflow_take_their_named_values() {
        let mut document = with_token(document(vec![text_node()]));
        assert!(apply(&mut document, "label", "kind.content.align", "center").is_ok());
        assert!(apply(&mut document, "label", "kind.content.overflow", "ellipsis").is_ok());
        assert!(
            apply(&mut document, "label", "kind.content.verticalAlign", "middle").is_ok()
        );
        let NodeKind::Text { content } = &document.nodes[0].kind else {
            panic!("still text");
        };
        assert_eq!(content.align, TextAlign::Center);
        assert_eq!(content.overflow, TextOverflow::Ellipsis);
        assert_eq!(content.vertical_align, VerticalAlign::Middle);
        assert!(apply(&mut document, "label", "kind.content.align", "justify").is_err());
        assert!(apply(&mut document, "label", "kind.content.vertical", "maybe").is_err());
    }

    #[test]
    fn a_bound_run_refuses_typed_text() {
        let mut node = text_node();
        node.kind = NodeKind::Text {
            content: TextContent {
                runs: vec![TextRun::Value {
                    binding_id: "b".to_owned(),
                    precision: None,
                    unit_display: None,
                    type_token: "body".to_owned(),
                    style: BTreeMap::new(),
                }],
                box_: None,
                overflow: TextOverflow::Visible,
                align: TextAlign::Left,
                vertical_align: VerticalAlign::Top,
                vertical: false,
            },
        };
        let mut document = with_token(document(vec![node]));
        assert!(apply(&mut document, "label", "kind.content.runs.0.text", "x").is_err());
    }

    #[test]
    fn the_transform_section_leads_for_a_selected_node() {
        let document = document(vec![rect("a", 10.0, 20.0, 30.0, 40.0)]);
        let sections = sections(&document, "a");
        assert_eq!(sections[0].title, "Transform");
        assert_eq!(sections[0].rows[0].key, "transform.x");
        assert_eq!(sections[0].rows[0].value, "10");
        assert_eq!(sections[0].rows[2].value, "30");
    }

    #[test]
    fn a_fill_takes_a_hex_or_a_palette_reference() {
        let mut document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]);
        assert!(apply(&mut document, "a", "style.fill", "#ff0000").is_ok());
        let fill = document.nodes[0].style.get("fill").cloned();
        assert_eq!(
            fill,
            Some(StyleValue::Literal(StyleLiteral {
                value: serde_json::json!("#ff0000")
            }))
        );
        assert!(apply(&mut document, "a", "style.fill", "palette.accent").is_ok());
        assert_eq!(
            document.nodes[0].style.get("fill").cloned(),
            Some(StyleValue::Ref(StyleRef {
                r#ref: "palette.accent".to_owned()
            }))
        );
        assert!(apply(&mut document, "a", "style.fill", "red").is_err());
    }

    #[test]
    fn opacity_stays_between_zero_and_one() {
        let mut document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]);
        assert!(apply(&mut document, "a", "style.opacity", "1.5").is_err());
        assert!(apply(&mut document, "a", "style.opacity", "0.5").is_ok());
    }

    #[test]
    fn a_rectangle_has_a_radius_and_an_ellipse_does_not() {
        let mut document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]);
        assert!(apply(&mut document, "a", "kind.cornerRadius", "12").is_ok());
        let NodeKind::Rectangle { corner_radius } = document.nodes[0].kind else {
            panic!("still a rectangle");
        };
        assert_eq!(corner_radius, 12.0);
        document.nodes[0].kind = NodeKind::Ellipse;
        assert!(apply(&mut document, "a", "kind.cornerRadius", "4").is_err());
    }

    #[test]
    fn a_typed_number_becomes_the_nodes_transform() {
        let mut document = document(vec![rect("a", 10.0, 20.0, 30.0, 40.0)]);
        assert!(apply(&mut document, "a", "transform.x", "42").is_ok());
        assert_eq!(document.nodes[0].transform.x, 42.0);
        assert!(apply(&mut document, "a", "transform.width", "12.5").is_ok());
        assert_eq!(document.nodes[0].transform.width, 12.5);
    }

    #[test]
    fn a_non_number_is_refused_and_changes_nothing() {
        let mut document = document(vec![rect("a", 10.0, 20.0, 30.0, 40.0)]);
        assert!(apply(&mut document, "a", "transform.x", "wide").is_err());
        assert_eq!(document.nodes[0].transform.x, 10.0);
        assert!(apply(&mut document, "missing", "transform.x", "1").is_err());
    }

    #[test]
    fn geometry_never_goes_below_one_unit() {
        let mut document = document(vec![rect("a", 10.0, 20.0, 30.0, 40.0)]);
        assert!(apply(&mut document, "a", "transform.width", "0").is_err());
        assert_eq!(document.nodes[0].transform.width, 30.0);
        assert!(apply(&mut document, "a", "transform.scaleX", "0").is_err());
    }

    #[test]
    fn the_rotation_wraps_into_zero_to_three_sixty() {
        let mut document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]);
        assert!(apply(&mut document, "a", "transform.rotation", "400").is_ok());
        assert_eq!(document.nodes[0].transform.rotation, 40.0);
    }
}
