//! The inspector: the selected node described as sections of editable fields.
//!
//! A field is one key, a label and the text of its current value. Writing goes
//! back through one function, [`apply`], so every row edits the document the
//! same way and a refused value changes nothing.

use yunari_model::document::{
    ImageFit, Node, NodeKind, StyleLiteral, StyleMap, StyleRef, StyleValue, ThemeDocument,
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
        _ => {}
    }
    sections
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
    let node = find_mut(&mut document.nodes, id).ok_or_else(|| "No node with that id".to_owned())?;
    apply_to_node(node, key, value)
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
    use crate::fixtures::{document, rect};

    use super::*;

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
