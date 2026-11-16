//! Palette tokens: the named paints every style and chart refers to.
//!
//! A token is edited through the inspector, refused when the id is taken, and
//! kept while anything in the document still points at it — a save that names a
//! token which no longer exists renders a gap.

use serde_json::Value;
use yunari_model::document::{Paint, ThemeDocument};

use crate::document::Editor;

/// A token's paint as the inspector shows it. Both kinds are written as the
/// text a later edit reads back, so a token round-trips through one field.
pub fn paint_text(paint: &Paint) -> String {
    match paint {
        Paint::Solid { color } => color.clone(),
        Paint::Gradient { angle, stops } => {
            let stops: Vec<String> = stops
                .iter()
                .map(|stop| format!("{} {:.0}%", stop.color, stop.offset * 100.0))
                .collect();
            format!("gradient({angle:.0}deg, {})", stops.join(", "))
        }
    }
}

/// A typed paint: a hex colour, or `gradient(<angle>deg, <colour> <pct>%, …)`
/// with two to eight stops. Anything else is refused before it can reach the
/// document.
pub fn parse_paint(text: &str) -> Result<Paint, String> {
    let text = text.trim();
    let Some(rest) = text.strip_prefix("gradient(") else {
        return solid(text);
    };
    let Some(body) = rest.strip_suffix(')') else {
        return Err("A gradient is gradient(<angle>deg, <colour> <percent>%, …)".to_owned());
    };
    let mut parts = body.split(',');
    let angle = parts
        .next()
        .and_then(|part| part.trim().strip_suffix("deg"))
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|angle| angle.is_finite() && (0.0..=360.0).contains(angle))
        .ok_or_else(|| "A gradient angle is 0 to 360 degrees".to_owned())?;
    let mut stops = Vec::new();
    for part in parts {
        let mut words = part.split_whitespace();
        let color = words
            .next()
            .ok_or_else(|| "A gradient stop is a colour and a percent".to_owned())?;
        solid(color)?;
        let percent = words
            .next()
            .and_then(|word| word.strip_suffix('%'))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|percent| (0.0..=100.0).contains(percent))
            .ok_or_else(|| "A gradient stop offset is 0 to 100%".to_owned())?;
        if words.next().is_some() {
            return Err("A gradient stop is a colour and a percent".to_owned());
        }
        stops.push(yunari_model::document::GradientStop {
            offset: percent / 100.0,
            color: color.to_owned(),
        });
    }
    if !(2..=8).contains(&stops.len()) {
        return Err("A gradient has two to eight stops".to_owned());
    }
    Ok(Paint::Gradient { angle, stops })
}

/// A typed colour as a solid paint.
pub fn solid(text: &str) -> Result<Paint, String> {
    let text = text.trim();
    let hex = text.strip_prefix('#').unwrap_or("");
    if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Paint::Solid {
            color: text.to_owned(),
        });
    }
    Err("A palette colour is #rrggbb".to_owned())
}

/// A name as a token id: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut id = String::new();
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            id.push(character.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-').to_owned();
    if id.is_empty() {
        "token".to_owned()
    } else {
        id
    }
}

/// The first thing in the document that points at `palette.<id>`, if any.
pub fn referenced_by(document: &ThemeDocument, id: &str) -> Option<String> {
    let reference = format!("palette.{id}");
    let json = serde_json::to_value(document).ok()?;
    find_string(&json, &reference).map(|_| reference)
}

fn find_string(value: &Value, wanted: &str) -> Option<()> {
    match value {
        Value::String(text) if text == wanted => Some(()),
        Value::String(_) => None,
        Value::Array(items) => items.iter().find_map(|item| find_string(item, wanted)),
        Value::Object(map) => map.values().find_map(|item| find_string(item, wanted)),
        _ => None,
    }
}

impl Editor {
    /// Adds a solid token under an id derived from its name, or `None` when the
    /// id is already taken and the name yields nothing new.
    pub fn add_palette(&mut self, name: &str, colour: &str) -> Option<String> {
        let paint = solid(colour).ok()?;
        let base = slug(name);
        let mut id = base.clone();
        let mut number = 2;
        if base == "none" {
            return None;
        }
        while self.document().palette.contains_key(&id) {
            id = format!("{base}-{number}");
            number += 1;
        }
        let name = if name.trim().is_empty() {
            id.clone()
        } else {
            name.trim().to_owned()
        };
        let id_for_edit = id.clone();
        let changed = self.edit(|document| {
            document.palette.insert(
                id_for_edit.clone(),
                yunari_model::document::PaletteEntry { name, value: paint },
            );
        });
        changed.then_some(id)
    }

    pub fn rename_palette(&mut self, id: &str, name: &str) -> bool {
        if id == "none" {
            return false;
        }
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_control) {
            return false;
        }
        let (id, name) = (id.to_owned(), name.to_owned());
        self.edit(|document| {
            if let Some(entry) = document.palette.get_mut(&id) {
                entry.name = name;
            }
        })
    }

    pub fn set_palette_colour(&mut self, id: &str, colour: &str) -> Result<(), String> {
        if id == "none" {
            return Err("The none token is reserved".to_owned());
        }
        let paint = parse_paint(colour)?;
        let id = id.to_owned();
        let mut found = false;
        self.edit(|document| {
            if let Some(entry) = document.palette.get_mut(&id) {
                entry.value = paint;
                found = true;
            }
        });
        if found {
            Ok(())
        } else {
            Err("No palette token with that id".to_owned())
        }
    }

    /// Removes a token nothing points at. A reference keeps it: the reason names
    /// the token rather than a silent deletion that would blank a save.
    pub fn remove_palette(&mut self, id: &str) -> Result<(), String> {
        if id == "none" {
            return Err("The none token is reserved".to_owned());
        }
        if let Some(reference) = referenced_by(self.document(), id) {
            return Err(format!("{reference} is still in use"));
        }
        if !self.document().palette.contains_key(id) {
            return Err("No palette token with that id".to_owned());
        }
        let id = id.to_owned();
        self.edit(|document| {
            document.palette.remove(&id);
        });
        Ok(())
    }
}

/// Tests only: a document with one style that refers to `palette.<id>`.
#[cfg(test)]
mod tests {
    use yunari_model::document::{StyleLiteral, StyleRef, StyleValue};

    use crate::fixtures::{document, rect};

    use super::*;

    #[test]
    fn the_none_token_is_reserved() {
        let mut editor = Editor::new(document(vec![]));
        assert!(editor.add_palette("none", "#00000000").is_none());
        editor.add_palette("Panel", "#000000");
        assert!(editor.rename_palette("panel", "other"));
        assert!(!editor.rename_palette("none", "other"));
        assert!(editor.set_palette_colour("none", "#ffffff").is_err());
    }

    #[test]
    fn a_slug_is_lowercase_with_dashes() {
        assert_eq!(slug("Card Accent!"), "card-accent");
        assert_eq!(slug("   "), "token");
    }

    #[test]
    fn adding_a_token_mints_a_free_id() {
        let mut editor = Editor::new(document(vec![]));
        assert_eq!(
            editor.add_palette("Accent", "#ff0000").as_deref(),
            Some("accent")
        );
        assert_eq!(
            editor.add_palette("Accent", "#00ff00").as_deref(),
            Some("accent-2")
        );
        assert_eq!(editor.document().palette.len(), 2);
        assert!(editor.undo());
        assert_eq!(editor.document().palette.len(), 1);
    }

    #[test]
    fn a_referenced_token_is_not_removed() {
        let mut node = rect("a", 0.0, 0.0, 10.0, 10.0);
        node.style.insert(
            "fill".to_owned(),
            StyleValue::Ref(StyleRef {
                r#ref: "palette.accent".to_owned(),
            }),
        );
        let mut editor = Editor::new(document(vec![node]));
        editor.add_palette("accent", "#ff0000");
        assert!(editor.remove_palette("accent").is_err());
        editor.edit(|document| {
            document.nodes[0].style.insert(
                "fill".to_owned(),
                StyleValue::Literal(StyleLiteral {
                    value: serde_json::json!("#111111"),
                }),
            );
        });
        assert!(editor.remove_palette("accent").is_ok());
        assert!(editor.document().palette.is_empty());
    }

    #[test]
    fn a_gradient_round_trips_through_the_one_field() {
        let paint = Paint::Gradient {
            angle: 90.0,
            stops: vec![
                yunari_model::document::GradientStop {
                    offset: 0.0,
                    color: "#000000".to_owned(),
                },
                yunari_model::document::GradientStop {
                    offset: 1.0,
                    color: "#ffffff".to_owned(),
                },
            ],
        };
        assert_eq!(
            paint_text(&paint),
            "gradient(90deg, #000000 0%, #ffffff 100%)"
        );
        assert_eq!(parse_paint(&paint_text(&paint)), Ok(paint));
        assert!(parse_paint("gradient(90deg, #000000 0%)").is_err());
        assert!(parse_paint("gradient(90deg, #000000 0%, #fff 100%, #eee 50%)").is_ok());
        assert!(parse_paint("gradient(400deg, #000000 0%, #fff 100%)").is_err());
        assert!(parse_paint("gradient(90deg, nope 0%, #fff 100%)").is_err());
    }

    #[test]
    fn a_gradient_token_is_written_to_the_document() {
        let mut editor = Editor::new(document(vec![]));
        editor.add_palette("Sky", "#000000");
        editor
            .set_palette_colour("sky", "gradient(45deg, #000000 0%, #ffffff 100%)")
            .unwrap();
        match &editor.document().palette["sky"].value {
            Paint::Gradient { angle, stops } => {
                assert_eq!(*angle, 45.0);
                assert_eq!(stops.len(), 2);
            }
            other => panic!("expected a gradient, got {other:?}"),
        }
        assert!(editor.undo());
        assert!(matches!(
            editor.document().palette["sky"].value,
            Paint::Solid { .. }
        ));
    }

}
