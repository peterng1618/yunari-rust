//! The inspector: the selected node described as sections of editable fields.
//!
//! A field is one key, a label and the text of its current value. Writing goes
//! back through one function, [`apply`], so every row edits the document the
//! same way and a refused value changes nothing.

use yunari_model::document::{Node, ThemeDocument};

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
    vec![Section {
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
    }]
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
