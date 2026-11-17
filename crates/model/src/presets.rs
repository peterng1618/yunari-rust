//! The documents a new theme starts from: a blank board and the starter with
//! a title and a live clock. Kept in the model so every surface that mints a
//! theme — the editor and the agent surface — gives the same one.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::document::{
    Artboard, ArtboardShape, Background, Binding, ContentFit, Metadata, Node, NodeKind,
    Paint as PaintKind, PaletteEntry, StyleLiteral, StyleMap, StyleValue, TextAlign, TextContent,
    TextOverflow, TextRun, ThemeDocument, Transform, TypeToken, TypeWeight, VerticalAlign,
};

/// The tokens and artboard a new document shares: the three role tokens and
/// the eight paint tokens every card expects, on a 1920 x 1080 artboard.
pub fn blank() -> ThemeDocument {
    let mut type_tokens = BTreeMap::new();
    for (id, family, weight) in [
        ("heading", "Segoe UI, sans-serif", 600.0),
        ("body", "Segoe UI, sans-serif", 400.0),
        ("mono", "Consolas, monospace", 400.0),
    ] {
        type_tokens.insert(
            id.to_owned(),
            TypeToken {
                family: family.to_owned(),
                weight: Some(TypeWeight::Number(weight)),
                face: None,
            },
        );
    }
    let mut palette = BTreeMap::new();
    for (id, name, colour) in [
        ("none", "None", "#00000000"),
        ("text", "Text", "#ecf5ff"),
        ("dim", "Muted text", "#a8bed0"),
        ("panel", "Panel", "#081523d9"),
        ("frost", "Frosted panel", "#0815234d"),
        ("panelStroke", "Panel outline", "#9fc7e52b"),
        ("rule", "Rule", "#7dbde0"),
        ("chartTrack", "Chart track", "#223047"),
    ] {
        palette.insert(
            id.to_owned(),
            PaletteEntry {
                name: name.to_owned(),
                value: PaintKind::Solid {
                    color: colour.to_owned(),
                },
            },
        );
    }
    ThemeDocument {
        schema_version: "1.0.0".to_owned(),
        id: "untitled".to_owned(),
        metadata: Metadata {
            name: Some("Untitled".to_owned()),
            theme_language: Some("en".to_owned()),
            ..Metadata::default()
        },
        artboard: Artboard {
            width: 1920.0,
            height: 1080.0,
            content_fit: ContentFit::Contain,
            shape: ArtboardShape::Rect,
        },
        background: Background {
            mode: Default::default(),
            color: Some(StyleValue::Ref(crate::document::StyleRef {
                r#ref: "palette.chartTrack".to_owned(),
            })),
            media: None,
        },
        palette,
        type_tokens,
        assets: Vec::new(),
        nodes: Vec::new(),
        bindings: BTreeMap::new(),
        editor_metadata: BTreeMap::new(),
    }
}

/// The theme a new document starts from: a title and a clock reading live, so
/// the first frame shows a working value rather than an empty board.
pub fn starter() -> ThemeDocument {
    let mut document = blank();
    let mut title = text_node("title-1");
    title.transform = Transform {
        x: 96.0,
        y: 88.0,
        width: 720.0,
        height: 80.0,
        ..Transform::default()
    };
    if let NodeKind::Text { content } = &mut title.kind {
        content.runs = vec![TextRun::Literal {
            text: "Untitled".to_owned(),
            type_token: "typeTokens.heading".to_owned(),
            style: fill("#e8e8ea"),
        }];
    }
    document.nodes.push(title);

    let mut clock = text_node("clock-1");
    clock.transform = Transform {
        x: 96.0,
        y: 176.0,
        width: 360.0,
        height: 96.0,
        ..Transform::default()
    };
    if let NodeKind::Text { content } = &mut clock.kind {
        content.runs = vec![TextRun::Value {
            binding_id: "clock-value".to_owned(),
            precision: None,
            unit_display: None,
            type_token: "typeTokens.heading".to_owned(),
            style: fill("#ecf5ff"),
        }];
    }
    document.nodes.push(clock);
    document.bindings.insert(
        "clock-1".to_owned(),
        vec![Binding {
            id: "clock-value".to_owned(),
            semantic_key: "time.now".to_owned(),
            precision: None,
            unit_display: None,
            scale: None,
            offset: None,
            unit: None,
            auto_scale: None,
            format: None,
            time_zone: None,
        }],
    );
    document
}

fn text_node(id: &str) -> Node {
    let mut style = fill("#e8e8ea");
    style.insert(
        "fontSize".to_owned(),
        StyleValue::Literal(StyleLiteral {
            value: Value::from(32),
        }),
    );
    Node {
        id: id.to_owned(),
        name: None,
        kind: NodeKind::Text {
            content: TextContent {
                runs: vec![TextRun::Literal {
                    text: "New text".to_owned(),
                    type_token: "typeTokens.body".to_owned(),
                    style,
                }],
                box_: None,
                overflow: TextOverflow::Visible,
                align: TextAlign::Left,
                vertical_align: VerticalAlign::Top,
                vertical: false,
            },
        },
        transform: Transform::default(),
        visible: true,
        locked: false,
        style: StyleMap::new(),
        glass: None,
        children: Vec::new(),
    }
}

fn fill(color: &str) -> StyleMap {
    let mut style = StyleMap::new();
    style.insert(
        "fill".to_owned(),
        StyleValue::Literal(StyleLiteral {
            value: Value::String(color.to_owned()),
        }),
    );
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::validate_document;

    #[test]
    fn the_blank_theme_is_valid_and_named() {
        let document = blank();
        assert!(validate_document(&document).is_ok());
        assert_eq!(document.artboard.width, 1920.0);
        assert_eq!(document.palette.len(), 8);
    }

    #[test]
    fn the_starter_reads_time_without_asking_the_author() {
        let document = starter();
        assert!(validate_document(&document).is_ok());
        let clock = document
            .nodes
            .iter()
            .find(|node| node.id == "clock-1")
            .expect("a clock");
        assert!(matches!(clock.kind, NodeKind::Text { .. }));
        assert_eq!(document.bindings["clock-1"][0].semantic_key, "time.now");
    }
}
