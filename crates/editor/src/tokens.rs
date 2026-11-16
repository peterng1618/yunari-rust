//! Type tokens: the family and weight a text run names.
//!
//! A run stores the token id, so a token is kept while any run names it and a
//! typed weight is stored in the shape it was typed: a number or a name.

use yunari_model::document::{Node, NodeKind, TextRun, ThemeDocument, TypeToken, TypeWeight};

use crate::document::Editor;

/// A weight as a field shows it.
pub fn weight_text(weight: &TypeWeight) -> String {
    match weight {
        TypeWeight::Name(name) => name.clone(),
        TypeWeight::Number(number) => crate::inspector::format_number(*number),
    }
}

/// `600` is a number, `semibold` a name, and an empty field clears the weight.
pub fn parse_weight(text: &str) -> Result<Option<TypeWeight>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Ok(number) = text.parse::<f64>() {
        if !number.is_finite() || !(1.0..=1000.0).contains(&number) {
            return Err("A weight runs from 1 to 1000".to_owned());
        }
        return Ok(Some(TypeWeight::Number(number)));
    }
    if text.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return Ok(Some(TypeWeight::Name(text.to_owned())));
    }
    Err("A weight is a number or a name like semibold".to_owned())
}

/// The first text run that names this token, if any.
pub fn referenced_by(document: &ThemeDocument, id: &str) -> Option<String> {
    let wanted = format!("typeTokens.{id}");
    for node in &document.nodes {
        if let Some(found) = find_in(node, id, &wanted) {
            return Some(found);
        }
    }
    None
}

fn find_in(node: &Node, id: &str, wanted: &str) -> Option<String> {
    if let NodeKind::Text { content } = &node.kind {
        for run in &content.runs {
            let token = match run {
                TextRun::Literal { type_token, .. } | TextRun::Value { type_token, .. } => {
                    type_token
                }
            };
            if token == id || token == wanted {
                return Some(format!("{} names {id}", node.id));
            }
        }
    }
    node.children
        .iter()
        .find_map(|child| find_in(child, id, wanted))
}

impl Editor {
    /// Adds a token under the id of its name, with the body family until one is
    /// typed. Returns the id taken.
    pub fn add_token(&mut self, name: &str) -> Option<String> {
        let base = crate::palette::slug(name);
        let mut id = base.clone();
        let mut number = 2;
        while self.document().type_tokens.contains_key(&id) {
            id = format!("{base}-{number}");
            number += 1;
        }
        let id_for_edit = id.clone();
        self.edit(|document| {
            document.type_tokens.insert(
                id_for_edit.clone(),
                TypeToken {
                    family: "Segoe UI".to_owned(),
                    weight: None,
                    face: None,
                },
            );
        });
        Some(id)
    }

    pub fn set_token_family(&mut self, id: &str, family: &str) -> Result<(), String> {
        let family = family.trim();
        if family.is_empty() {
            return Err("A token needs a family".to_owned());
        }
        if family.chars().any(char::is_control) {
            return Err("A family name is one line".to_owned());
        }
        let (id, family) = (id.to_owned(), family.to_owned());
        let mut found = false;
        self.edit(|document| {
            if let Some(token) = document.type_tokens.get_mut(&id) {
                token.family = family;
                found = true;
            }
        });
        if found {
            Ok(())
        } else {
            Err("No type token with that id".to_owned())
        }
    }

    pub fn set_token_weight(&mut self, id: &str, weight: &str) -> Result<(), String> {
        let parsed = parse_weight(weight)?;
        let (id, parsed) = (id.to_owned(), parsed);
        let mut found = false;
        self.edit(|document| {
            if let Some(token) = document.type_tokens.get_mut(&id) {
                token.weight = parsed;
                found = true;
            }
        });
        if found {
            Ok(())
        } else {
            Err("No type token with that id".to_owned())
        }
    }

    /// Removes a token no run names, so a save never points at a vanished face.
    pub fn remove_token(&mut self, id: &str) -> Result<(), String> {
        if let Some(holder) = referenced_by(self.document(), id) {
            return Err(format!("{holder} and keeps it"));
        }
        if !self.document().type_tokens.contains_key(id) {
            return Err("No type token with that id".to_owned());
        }
        let id = id.to_owned();
        self.edit(|document| {
            document.type_tokens.remove(&id);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::{TextContent, TextRun, VerticalAlign};

    use crate::fixtures::{document, rect};

    use super::*;

    fn text(id: &str, token: &str) -> Node {
        let mut node = rect(id, 0.0, 0.0, 100.0, 30.0);
        node.kind = NodeKind::Text {
            content: TextContent {
                runs: vec![TextRun::Literal {
                    text: "Hi".to_owned(),
                    type_token: token.to_owned(),
                    style: BTreeMap::new(),
                }],
                box_: None,
                overflow: Default::default(),
                align: Default::default(),
                vertical_align: VerticalAlign::Top,
                vertical: false,
            },
        };
        node
    }

    #[test]
    fn a_weight_reads_and_writes_both_shapes() {
        assert_eq!(
            parse_weight("600"),
            Ok(Some(TypeWeight::Number(600.0)))
        );
        assert_eq!(
            parse_weight("semibold"),
            Ok(Some(TypeWeight::Name("semibold".to_owned())))
        );
        assert_eq!(parse_weight("  "), Ok(None));
        assert!(parse_weight("0").is_err());
        assert!(parse_weight("600 kg").is_err());
        assert_eq!(weight_text(&TypeWeight::Number(300.0)), "300");
        assert_eq!(
            weight_text(&TypeWeight::Name("bold".to_owned())),
            "bold"
        );
    }

    #[test]
    fn a_token_in_use_is_not_removed() {
        let mut document = document(vec![text("label", "heading")]);
        document
            .type_tokens
            .insert("heading".to_owned(), TypeToken {
                family: "Segoe UI".to_owned(),
                weight: None,
                face: None,
            });
        let mut editor = Editor::new(document);
        assert!(editor.remove_token("heading").is_err());
        editor.edit(|document| {
            if let NodeKind::Text { content } = &mut document.nodes[0].kind
                && let TextRun::Literal { type_token, .. } = &mut content.runs[0]
            {
                *type_token = "body".to_owned();
            }
        });
        assert!(editor.remove_token("heading").is_ok());
    }

    #[test]
    fn family_and_weight_edits_are_one_entry_each() {
        let mut document = document(vec![]);
        document.type_tokens.insert("body".to_owned(), TypeToken {
            family: "Segoe UI".to_owned(),
            weight: None,
            face: None,
        });
        let mut editor = Editor::new(document);
        assert!(editor.set_token_family("body", "Inter").is_ok());
        assert!(editor.set_token_weight("body", "600").is_ok());
        let token = editor.document().type_tokens.get("body").expect("token");
        assert_eq!(token.family, "Inter");
        assert_eq!(token.weight, Some(TypeWeight::Number(600.0)));
        assert!(editor.set_token_family("body", " ").is_err());
    }

    #[test]
    fn adding_a_token_mints_a_free_id() {
        let mut editor = Editor::new(document(vec![]));
        assert_eq!(editor.add_token("Heading").as_deref(), Some("heading"));
        assert_eq!(editor.add_token("Heading").as_deref(), Some("heading-2"));
    }
}
