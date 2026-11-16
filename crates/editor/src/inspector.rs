//! The inspector: the selected node described as sections of editable fields.
//!
//! A field is one key, a label and the text of its current value. Writing goes
//! back through one function, [`apply`], so every row edits the document the
//! same way and a refused value changes nothing.

use yunari_model::document::{
    ArtboardShape, BackgroundMode, ContentFit, ImageFit, Node, NodeKind, StyleLiteral, StyleMap,
    StyleRef, StyleValue, TextAlign, TextContent, TextOverflow, TextRun, ThemeDocument,
    VerticalAlign,
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
            rows: vec![
                text_field("kind.content.assetId", "Asset", Some(content.asset_id.as_str())),
                Field {
                    key: "kind.content.fit".to_owned(),
                    label: "Fit",
                    kind: FieldKind::Choice(vec!["cover", "contain", "stretch"]),
                    value: match content.fit {
                        ImageFit::Cover => "cover".to_owned(),
                        ImageFit::Contain => "contain".to_owned(),
                        ImageFit::Stretch => "stretch".to_owned(),
                    },
                },
            ],
        }),
        NodeKind::Video { content } => sections.push(Section {
            title: "Video",
            rows: vec![
                text_field("kind.content.assetId", "Asset", Some(content.asset_id.as_str())),
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
    if let Some(held) = document.bindings.get(id) {
        let mut rows = Vec::new();
        for binding in held {
            rows.push(Field {
                key: format!("bindings.{}.key", binding.id),
                label: "Key",
                kind: FieldKind::Text,
                value: binding.semantic_key.clone(),
            });
            rows.push(Field {
                key: format!("bindings.{}.unit", binding.id),
                label: "Unit",
                kind: FieldKind::Text,
                value: binding.unit.clone().unwrap_or_default(),
            });
            rows.push(Field {
                key: format!("bindings.{}.precision", binding.id),
                label: "Precision",
                kind: FieldKind::Number {
                    min: 0.0,
                    max: 6.0,
                    step: 1.0,
                },
                value: binding.precision.map_or_else(String::new, |p| p.to_string()),
            });
        }
        sections.push(Section {
            title: "Bindings",
            rows,
        });
    }
    sections
}

/// The theme's own fields, shown when no node is selected: what the theme is
/// called, how big the artboard is and what sits behind every node.
pub fn document_sections(document: &ThemeDocument) -> Vec<Section> {
    let metadata = &document.metadata;
    let artboard = &document.artboard;
    let background = &document.background;
    let mut sections = vec![
        Section {
            title: "Theme",
            rows: vec![
                text_field("theme.name", "Name", metadata.name.as_deref()),
                text_field("theme.author", "Author", metadata.author.as_deref()),
                text_field("theme.description", "Description", metadata.description.as_deref()),
                text_field("theme.version", "Version", metadata.version.as_deref()),
                text_field("theme.language", "Language", metadata.theme_language.as_deref()),
                text_field("theme.tags", "Tags", Some(metadata.tags.join(", ").as_str())),
            ],
        },
        Section {
            title: "Artboard",
            rows: vec![
                number("artboard.width", "Width", artboard.width, Some(MIN_SIZE)),
                number("artboard.height", "Height", artboard.height, Some(MIN_SIZE)),
                choice(
                    "artboard.contentFit",
                    "Content fit",
                    match artboard.content_fit {
                        ContentFit::Contain => "contain",
                        ContentFit::Cover => "cover",
                    },
                ),
                choice(
                    "artboard.shape",
                    "Shape",
                    match artboard.shape {
                        ArtboardShape::Rect => "rect",
                        ArtboardShape::Circle => "circle",
                    },
                ),
            ],
        },
        Section {
            title: "Background",
            rows: vec![
                choice(
                    "background.mode",
                    "Mode",
                    match background.mode {
                        BackgroundMode::Simple => "simple",
                        BackgroundMode::Advanced => "advanced",
                    },
                ),
                colour("background.color", "Colour", background.color.as_ref()),
            ],
        },
    ];
    if let Some(media) = &background.media {
        sections.push(Section {
            title: "Background media",
            rows: vec![text_field("background.media", "Asset", Some(media.asset_id.as_str()))],
        });
    }
    sections
}

fn text_field(key: &'static str, label: &'static str, value: Option<&str>) -> Field {
    Field {
        key: key.to_owned(),
        label,
        kind: FieldKind::Text,
        value: value.unwrap_or_default().to_owned(),
    }
}

/// Writes one document-level field: the theme's metadata, the artboard or the
/// background. The same refusal rule applies — a rejected value changes nothing.
pub fn apply_document(
    document: &mut ThemeDocument,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match key {
        "artboard.width" | "artboard.height" => {
            let parsed = value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("{} needs a number", friendly(key)))?;
            if !parsed.is_finite() || parsed < MIN_SIZE {
                return Err(format!("{} must be at least {MIN_SIZE}", friendly(key)));
            }
            if key == "artboard.width" {
                document.artboard.width = parsed;
            } else {
                document.artboard.height = parsed;
            }
        }
        "artboard.contentFit" => {
            document.artboard.content_fit = match value.trim() {
                "contain" => ContentFit::Contain,
                "cover" => ContentFit::Cover,
                other => return Err(format!("{other} is not a content fit")),
            };
        }
        "artboard.shape" => {
            document.artboard.shape = match value.trim() {
                "rect" => ArtboardShape::Rect,
                "circle" => ArtboardShape::Circle,
                other => return Err(format!("{other} is not an artboard shape")),
            };
        }
        "background.mode" => {
            document.background.mode = match value.trim() {
                "simple" => BackgroundMode::Simple,
                "advanced" => BackgroundMode::Advanced,
                other => return Err(format!("{other} is not a background mode")),
            };
        }
        "background.color" => {
            if value.trim().is_empty() {
                document.background.color = None;
            } else {
                document.background.color = Some(paint(value)?);
            }
        }
        "theme.tags" => {
            document.metadata.tags = value
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned)
                .collect();
        }
        _ => {
            let Some(field) = key.strip_prefix("theme.") else {
                return Err(format!("{} is not editable yet", friendly(key)));
            };
            let trimmed = value.trim();
            if trimmed.chars().any(char::is_control) {
                return Err(format!("{} is one line", friendly(key)));
            }
            let slot = match field {
                "name" => &mut document.metadata.name,
                "author" => &mut document.metadata.author,
                "description" => &mut document.metadata.description,
                "version" => &mut document.metadata.version,
                "language" => &mut document.metadata.theme_language,
                _ => return Err(format!("{} is not a theme field", friendly(key))),
            };
            *slot = (!trimmed.is_empty()).then(|| trimmed.to_owned());
        }
    }
    Ok(())
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
    if let Some(rest) = key.strip_prefix("bindings.") {
        let Some((binding_id, field)) = rest.rsplit_once('.') else {
            return Err("A binding field is bindings.<id>.<field>".to_owned());
        };
        return apply_binding(document, id, binding_id, field, value);
    }
    if key.ends_with(".typeToken") {
        let token = token_id(value);
        if !document.type_tokens.contains_key(token) {
            return Err(format!("No type token named {token}"));
        }
    }
    if key.ends_with(".assetId") {
        check_asset(document, id, value)?;
    }
    let node = find_mut(&mut document.nodes, id).ok_or_else(|| "No node with that id".to_owned())?;
    apply_to_node(node, key, value)
}

/// A node may only name a declared asset of the kind it can draw: a picture
/// node takes an image, SVG or GIF, and a video node a video.
fn check_asset(document: &ThemeDocument, node_id: &str, value: &str) -> Result<(), String> {
    let asset_id = value.trim();
    let asset = document
        .assets
        .iter()
        .find(|asset| asset.id == asset_id)
        .ok_or_else(|| format!("No asset named {asset_id}"))?;
    let node = crate::selection::find(document, node_id)
        .ok_or_else(|| "No node with that id".to_owned())?;
    let fits = match &node.kind {
        NodeKind::Image { .. } => matches!(
            asset.kind,
            yunari_model::document::AssetKind::Image
                | yunari_model::document::AssetKind::Svg
                | yunari_model::document::AssetKind::Gif
        ),
        NodeKind::Video { .. } => asset.kind == yunari_model::document::AssetKind::Video,
        _ => false,
    };
    if !fits {
        return Err(format!("{asset_id} is not the kind of asset this node draws"));
    }
    Ok(())
}

/// Writes one binding field on a node. A refused value changes nothing.
pub fn apply_binding(
    document: &mut ThemeDocument,
    node_id: &str,
    binding_id: &str,
    field: &str,
    value: &str,
) -> Result<(), String> {
    let held = document
        .bindings
        .get_mut(node_id)
        .ok_or_else(|| "This node has no bindings".to_owned())?;
    let binding = held
        .iter_mut()
        .find(|binding| binding.id == binding_id)
        .ok_or_else(|| "No binding with that id".to_owned())?;
    match field {
        "key" => {
            let key = value.trim();
            if let Some(refusal) = yunari_model::validate::semantic_key_refusal(key) {
                return Err(refusal.to_owned());
            }
            binding.semantic_key = key.to_owned();
        }
        "unit" => {
            let unit = value.trim();
            if unit.chars().any(char::is_control) {
                return Err("A unit is one line".to_owned());
            }
            binding.unit = (!unit.is_empty()).then(|| unit.to_owned());
        }
        "precision" => {
            let text = value.trim();
            if text.is_empty() {
                binding.precision = None;
            } else {
                let precision: u8 = text
                    .parse()
                    .map_err(|_| "Precision is a whole number".to_owned())?;
                if precision > 6 {
                    return Err("Precision runs from 0 to 6".to_owned());
                }
                binding.precision = Some(precision);
            }
        }
        _ => return Err(format!("{} is not a binding field", friendly(field))),
    }
    Ok(())
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
        "kind.content.assetId" => match &mut node.kind {
            NodeKind::Image { content } => content.asset_id = value.trim().to_owned(),
            NodeKind::Video { content } => content.asset_id = value.trim().to_owned(),
            _ => return Err("Only an image or video names an asset".to_owned()),
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
    fn the_document_shows_theme_artboard_and_background() {
        let document = document(vec![]);
        let sections = document_sections(&document);
        let titles: Vec<&str> = sections.iter().map(|section| section.title).collect();
        assert_eq!(titles, ["Theme", "Artboard", "Background"]);
        let artboard = &sections[1].rows;
        assert_eq!(artboard[0].value, "450");
        assert_eq!(artboard[2].value, "contain");
        assert_eq!(artboard[3].value, "rect");
    }

    #[test]
    fn artboard_edits_are_written_and_refused_values_change_nothing() {
        let mut document = document(vec![]);
        assert!(apply_document(&mut document, "artboard.width", "1920").is_ok());
        assert_eq!(document.artboard.width, 1920.0);
        assert!(apply_document(&mut document, "artboard.width", "0").is_err());
        assert_eq!(document.artboard.width, 1920.0);
        assert!(apply_document(&mut document, "artboard.shape", "circle").is_ok());
        assert_eq!(document.artboard.shape, ArtboardShape::Circle);
        assert!(apply_document(&mut document, "artboard.shape", "hexagon").is_err());
        assert!(apply_document(&mut document, "artboard.height", "wide").is_err());
    }

    #[test]
    fn metadata_fields_take_text_and_clear_when_empty() {
        let mut document = document(vec![]);
        assert!(apply_document(&mut document, "theme.name", " Desk ").is_ok());
        assert_eq!(document.metadata.name.as_deref(), Some("Desk"));
        assert!(apply_document(&mut document, "theme.name", "  ").is_ok());
        assert_eq!(document.metadata.name, None);
        assert!(apply_document(&mut document, "theme.tags", "dark, minimal").is_ok());
        assert_eq!(document.metadata.tags, ["dark", "minimal"]);
        assert!(apply_document(&mut document, "theme.version", "1\n2").is_err());
        assert!(apply_document(&mut document, "theme.unknown", "x").is_err());
    }

    #[test]
    fn the_background_takes_a_colour_or_a_palette_reference() {
        let mut document = document(vec![]);
        assert!(apply_document(&mut document, "background.color", "#101014").is_ok());
        assert!(matches!(
            document.background.color,
            Some(StyleValue::Literal(_))
        ));
        assert!(apply_document(&mut document, "background.color", "palette.desk").is_ok());
        assert!(matches!(document.background.color, Some(StyleValue::Ref(_))));
        assert!(apply_document(&mut document, "background.color", "").is_ok());
        assert_eq!(document.background.color, None);
        assert!(apply_document(&mut document, "background.color", "navy").is_err());
        assert!(apply_document(&mut document, "background.mode", "advanced").is_ok());
        assert_eq!(document.background.mode, BackgroundMode::Advanced);
    }

    fn image_node(id: &str, asset: &str) -> Node {
        let mut node = rect(id, 0.0, 0.0, 100.0, 100.0);
        node.kind = NodeKind::Image {
            content: yunari_model::document::ImageContent {
                asset_id: asset.to_owned(),
                fit: ImageFit::Cover,
                monochrome: None,
            },
        };
        node
    }

    fn asset(id: &str, kind: yunari_model::document::AssetKind) -> yunari_model::document::AssetReference {
        yunari_model::document::AssetReference {
            id: id.to_owned(),
            kind,
            path: format!("assets/{id}.png"),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        }
    }

    #[test]
    fn an_image_names_a_declared_asset_of_its_kind() {
        use yunari_model::document::AssetKind;
        let mut document = document(vec![image_node("photo", "pic")]);
        document.assets.push(asset("pic", AssetKind::Image));
        document.assets.push(asset("clip", AssetKind::Video));
        let sections = sections(&document, "photo");
        let image = sections
            .iter()
            .find(|section| section.title == "Image")
            .expect("an image section");
        assert_eq!(image.rows[0].key, "kind.content.assetId");
        assert_eq!(image.rows[0].value, "pic");
        assert!(apply(&mut document, "photo", "kind.content.assetId", "missing").is_err());
        assert!(apply(&mut document, "photo", "kind.content.assetId", "clip").is_err());
        assert!(apply(&mut document, "photo", "kind.content.assetId", "pic").is_ok());
    }

    #[test]
    fn bindings_show_and_edit_through_the_node_section() {
        let mut document = document(vec![rect("a", 0.0, 0.0, 10.0, 10.0)]);
        document.bindings.insert(
            "a".to_owned(),
            vec![yunari_model::document::Binding {
                id: "binding-1".to_owned(),
                semantic_key: "cpu.load".to_owned(),
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
        let sections = sections(&document, "a");
        let bindings = sections
            .iter()
            .find(|section| section.title == "Bindings")
            .expect("a bindings section");
        assert_eq!(bindings.rows[0].key, "bindings.binding-1.key");
        assert_eq!(bindings.rows[0].value, "cpu.load");
        assert!(apply(&mut document, "a", "bindings.binding-1.key", "gpu.load").is_ok());
        assert!(apply(&mut document, "a", "bindings.binding-1.unit", "%").is_ok());
        assert!(apply(&mut document, "a", "bindings.binding-1.precision", "2").is_ok());
        let held = &document.bindings["a"][0];
        assert_eq!(held.semantic_key, "gpu.load");
        assert_eq!(held.unit.as_deref(), Some("%"));
        assert_eq!(held.precision, Some(2));
        assert!(apply(&mut document, "a", "bindings.nope.key", "cpu.load").is_err());
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
