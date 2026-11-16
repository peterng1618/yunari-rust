//! Typography presets: three families and weights that give one theme a voice.
//!
//! A preset names families this PC can already draw, so applying one needs no
//! downloaded face; the source's packaged trios are a catalogue of the same
//! three roles, and a native theme reads the families Windows ships.

use yunari_model::document::{TypeToken, TypeWeight};

use crate::document::Editor;

/// One role's family and weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Role {
    pub family: &'static str,
    pub weight: f64,
}

/// A pairing of the three type roles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trio {
    pub id: &'static str,
    pub name: &'static str,
    pub heading: Role,
    pub body: Role,
    pub mono: Role,
}

const fn role(family: &'static str, weight: f64) -> Role {
    Role { family, weight }
}

/// The presets: Windows families a display can render without an install.
pub const TRIOS: [Trio; 8] = [
    Trio {
        id: "segoe-workbench",
        name: "Workbench  ·  Segoe UI and Consolas",
        heading: role("Segoe UI, sans-serif", 600.0),
        body: role("Segoe UI, sans-serif", 400.0),
        mono: role("Consolas, monospace", 400.0),
    },
    Trio {
        id: "editorial",
        name: "Editorial  ·  Georgia and Cambria",
        heading: role("Georgia, serif", 700.0),
        body: role("Georgia, serif", 400.0),
        mono: role("Consolas, monospace", 400.0),
    },
    Trio {
        id: "cambria-report",
        name: "Report  ·  Cambria and Calibri",
        heading: role("Cambria, serif", 700.0),
        body: role("Calibri, sans-serif", 400.0),
        mono: role("Consolas, monospace", 400.0),
    },
    Trio {
        id: "terminal",
        name: "Terminal  ·  Cascadia and Consolas",
        heading: role("Cascadia Code, monospace", 600.0),
        body: role("Consolas, monospace", 400.0),
        mono: role("Cascadia Mono, monospace", 400.0),
    },
    Trio {
        id: "classic",
        name: "Classic  ·  Times and Arial",
        heading: role("Times New Roman, serif", 700.0),
        body: role("Arial, sans-serif", 400.0),
        mono: role("Courier New, monospace", 400.0),
    },
    Trio {
        id: "humanist",
        name: "Humanist  ·  Trebuchet and Georgia",
        heading: role("Trebuchet MS, sans-serif", 700.0),
        body: role("Trebuchet MS, sans-serif", 400.0),
        mono: role("Consolas, monospace", 400.0),
    },
    Trio {
        id: "quiet",
        name: "Quiet  ·  Segoe UI Light and Calibri",
        heading: role("Segoe UI Light, sans-serif", 300.0),
        body: role("Calibri, sans-serif", 400.0),
        mono: role("Consolas, monospace", 400.0),
    },
    Trio {
        id: "display",
        name: "Display  ·  Segoe UI Semibold and Georgia",
        heading: role("Segoe UI Semibold, sans-serif", 600.0),
        body: role("Georgia, serif", 400.0),
        mono: role("Consolas, monospace", 400.0),
    },
];

impl Editor {
    /// Applies one preset to the three role tokens, creating a missing role so
    /// a theme imported from elsewhere still takes the pairing. One edit, so
    /// one undo puts the old typography back.
    pub fn apply_typography(&mut self, trio: &Trio) -> bool {
        self.edit(|document| {
            for (id, role) in [
                ("heading", trio.heading),
                ("body", trio.body),
                ("mono", trio.mono),
            ] {
                let token = document
                    .type_tokens
                    .entry(id.to_owned())
                    .or_insert_with(|| TypeToken {
                        family: role.family.to_owned(),
                        weight: Some(TypeWeight::Number(role.weight)),
                        face: None,
                    });
                token.family = role.family.to_owned();
                token.weight = Some(TypeWeight::Number(role.weight));
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::document;

    use super::*;

    #[test]
    fn applying_a_preset_sets_all_three_roles_and_undo_puts_them_back() {
        let mut theme = document(Vec::new());
        theme.type_tokens.insert(
            "heading".to_owned(),
            TypeToken {
                family: "Segoe UI, sans-serif".to_owned(),
                weight: Some(TypeWeight::Number(600.0)),
                face: None,
            },
        );
        let mut editor = Editor::new(theme);
        assert!(editor.apply_typography(&TRIOS[1]));
        let tokens = editor.document().type_tokens.clone();
        assert_eq!(tokens["heading"].family, "Georgia, serif");
        assert_eq!(tokens["body"].family, "Georgia, serif");
        assert_eq!(tokens["mono"].family, "Consolas, monospace");
        assert_eq!(tokens["heading"].weight, Some(TypeWeight::Number(700.0)));
        // The roles a theme needs are always present after applying one.
        let ids: Vec<&str> = tokens.keys().map(String::as_str).collect();
        for role in ["heading", "body", "mono"] {
            assert!(ids.contains(&role), "{role}");
        }
        editor.undo();
        assert_eq!(
            editor.document().type_tokens["heading"].family,
            "Segoe UI, sans-serif"
        );
        assert!(!editor.document().type_tokens.contains_key("body"));
    }

    #[test]
    fn every_preset_names_a_family_and_a_weight_its_role_can_use() {
        for trio in &TRIOS {
            for role in [trio.heading, trio.body, trio.mono] {
                assert!(!role.family.trim().is_empty(), "{}", trio.id);
                assert!((1.0..=1000.0).contains(&role.weight), "{}", trio.id);
            }
        }
        let ids: std::collections::BTreeSet<&str> = TRIOS.iter().map(|trio| trio.id).collect();
        assert_eq!(ids.len(), TRIOS.len(), "every preset id is its own");
    }
}
