//! Palette tokens: the named paints every style and chart refers to.
//!
//! A token is edited through the inspector, refused when the id is taken, and
//! kept while anything in the document still points at it — a save that names a
//! token which no longer exists renders a gap.

use serde_json::Value;
use yunari_model::document::{Paint, StyleValue, ThemeDocument};

use crate::document::Editor;

/// A token's paint as the inspector shows it: a solid is its colour, a gradient
/// says how many stops it has because a field cannot edit a stop list.
pub fn paint_text(paint: &Paint) -> String {
    match paint {
        Paint::Solid { color } => color.clone(),
        Paint::Gradient { stops, .. } => format!("gradient ({} stops)", stops.len()),
    }
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
    if id.is_empty() { "token".to_owned() } else { id }
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
            document
                .palette
                .insert(id_for_edit.clone(), yunari_model::document::PaletteEntry {
                    name,
                    value: paint,
                });
        });
        changed.then_some(id)
    }

    pub fn rename_palette(&mut self, id: &str, name: &str) -> bool {
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
        let paint = solid(colour)?;
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
    use yunari_model::document::{StyleLiteral, StyleRef};

    use crate::fixtures::{document, rect};

    use super::*;

    #[test]
    fn a_slug_is_lowercase_with_dashes() {
        assert_eq!(slug("Card Accent!"), "card-accent");
        assert_eq!(slug("   "), "token");
    }

    #[test]
    fn adding_a_token_mints_a_free_id() {
        let mut editor = Editor::new(document(vec![]));
        assert_eq!(editor.add_palette("Accent", "#ff0000").as_deref(), Some("accent"));
        assert_eq!(editor.add_palette("Accent", "#00ff00").as_deref(), Some("accent-2"));
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
    fn a_gradient_token_reads_its_stop_count_and_refuses_a_solid_edit() {
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
        assert_eq!(paint_text(&paint), "gradient (2 stops)");
        assert!(solid("gradient (2 stops)").is_err());
        assert!(solid("#fff").is_ok());
    }
}
