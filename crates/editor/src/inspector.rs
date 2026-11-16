//! The inspector: the selected node described as sections of editable fields.
//!
//! A field is one key, a label and the text of its current value. Writing goes
//! back through one function, [`apply`], so every row edits the document the
//! same way and a refused value changes nothing.

use yunari_model::document::{
    ArtboardShape, AssetKind, BackgroundAnchor, BackgroundBounds, BackgroundComposition,
    BackgroundMedia, BackgroundMode, ChartContent, ChartFamily, ContentFit, ImageFit, Node,
    NodeKind, StyleLiteral, StyleMap, StyleRef, StyleValue, TextAlign, TextContent, TextOverflow,
    TextRun, ThemeDocument, UnitDisplay, VerticalAlign,
};

use crate::selection::find_mut;

/// The smallest a side may be; the map and every drag share it.
pub const MIN_SIZE: f64 = 1.0;
/// The largest artboard edge the format accepts.
pub const MAX_ARTBOARD: f64 = 16_384.0;
/// Background media ranges, copied from the model's validator.
pub const MIN_BACKGROUND_SCALE: f64 = 1.0;
pub const MAX_BACKGROUND_SCALE: f64 = 5.0;
pub const MIN_BACKGROUND_OFFSET: f64 = -5.0;
pub const MAX_BACKGROUND_OFFSET: f64 = 5.0;
pub const MIN_BACKGROUND_EXTENT: f64 = 0.1;
pub const MAX_BACKGROUND_EXTENT: f64 = 1.0;
pub const MIN_BACKGROUND_FADE: f64 = 0.01;
pub const MAX_BACKGROUND_FADE: f64 = 1.0;
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
            number("transform.width", "Width", transform.width, Some(MIN_SIZE)),
            number(
                "transform.height",
                "Height",
                transform.height,
                Some(MIN_SIZE),
            ),
            number(
                "transform.rotation",
                "Rotation",
                transform.rotation,
                Some(0.0),
            ),
            number(
                "transform.scaleX",
                "Scale X",
                transform.scale_x,
                Some(0.000_001),
            ),
            number(
                "transform.scaleY",
                "Scale Y",
                transform.scale_y,
                Some(0.000_001),
            ),
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
                text_field(
                    "kind.content.assetId",
                    "Asset",
                    Some(content.asset_id.as_str()),
                ),
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
                text_field(
                    "kind.content.assetId",
                    "Asset",
                    Some(content.asset_id.as_str()),
                ),
                toggle("kind.content.loop", "Loop", content.loop_),
                toggle("kind.content.muted", "Muted", content.muted),
            ],
        }),
        NodeKind::Text { content } => {
            let mut rows = text_rows(content);
            rows.push(Field {
                key: "kind.content.lineHeight".to_owned(),
                label: "Line height",
                kind: FieldKind::Number {
                    min: 0.01,
                    max: 10.0,
                    step: 0.1,
                },
                value: style_number(node, "lineHeight").map_or_else(String::new, format_number),
            });
            sections.push(Section {
                title: "Text",
                rows,
            });
        }
        NodeKind::Chart { content } => sections.push(Section {
            title: "Chart",
            rows: chart_rows(content),
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
                value: binding
                    .precision
                    .map_or_else(String::new, |p| p.to_string()),
            });
            rows.push(Field {
                key: format!("bindings.{}.unitDisplay", binding.id),
                label: "Unit display",
                kind: FieldKind::Choice(Vec::new()),
                value: match binding.unit_display {
                    None => "default".to_owned(),
                    Some(UnitDisplay::None) => "none".to_owned(),
                    Some(UnitDisplay::Short) => "short".to_owned(),
                    Some(UnitDisplay::Long) => "long".to_owned(),
                },
            });
            rows.push(Field {
                key: format!("bindings.{}.format", binding.id),
                label: "Format",
                kind: FieldKind::Text,
                value: binding.format.clone().unwrap_or_default(),
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
                text_field(
                    "theme.description",
                    "Description",
                    metadata.description.as_deref(),
                ),
                text_field("theme.version", "Version", metadata.version.as_deref()),
                text_field(
                    "theme.language",
                    "Language",
                    metadata.theme_language.as_deref(),
                ),
                text_field(
                    "theme.tags",
                    "Tags",
                    Some(metadata.tags.join(", ").as_str()),
                ),
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
                colour(
                    "background.color",
                    "Colour",
                    background
                        .media
                        .as_ref()
                        .and_then(|media| media.color.as_ref())
                        .or(background.color.as_ref()),
                ),
                text_field(
                    "background.media",
                    "Media",
                    background
                        .media
                        .as_ref()
                        .map(|media| media.asset_id.as_str()),
                ),
            ],
        },
    ];
    if let Some(media) = &background.media {
        let mut rows = vec![
            choice(
                "background.media.bounds",
                "Bounds",
                match media.bounds {
                    BackgroundBounds::Theme => "theme",
                    BackgroundBounds::Display => "display",
                },
            ),
            number(
                "background.media.scale",
                "Scale",
                media.scale,
                Some(MIN_BACKGROUND_SCALE),
            ),
            number(
                "background.media.offsetX",
                "Offset X",
                media.offset_x,
                Some(MIN_BACKGROUND_OFFSET),
            ),
            number(
                "background.media.offsetY",
                "Offset Y",
                media.offset_y,
                Some(MIN_BACKGROUND_OFFSET),
            ),
            toggle("background.media.flipX", "Flip horizontal", media.flip_x),
            toggle("background.media.flipY", "Flip vertical", media.flip_y),
        ];
        let video = media_kind(document, &media.asset_id) == Some(AssetKind::Video);
        if video {
            rows.push(Field {
                key: "background.media.fallbackFrame".to_owned(),
                label: "Fallback frame",
                kind: FieldKind::Number {
                    min: 0.0,
                    max: f64::MAX,
                    step: 1.0,
                },
                value: media
                    .fallback_frame
                    .map_or_else(String::new, |frame| frame.to_string()),
            });
        }
        if background.mode == BackgroundMode::Advanced {
            rows.push(choice(
                "background.media.anchor",
                "Anchor",
                match media
                    .composition
                    .as_ref()
                    .map(|composition| composition.anchor)
                {
                    None => "full",
                    Some(BackgroundAnchor::Top) => "top",
                    Some(BackgroundAnchor::Right) => "right",
                    Some(BackgroundAnchor::Bottom) => "bottom",
                    Some(BackgroundAnchor::Left) => "left",
                    Some(BackgroundAnchor::Center) => "center",
                },
            ));
            if let Some(composition) = &media.composition
                && composition.anchor != BackgroundAnchor::Center
            {
                rows.push(Field {
                    key: "background.media.extent".to_owned(),
                    label: "Size",
                    kind: FieldKind::Number {
                        min: MIN_BACKGROUND_EXTENT,
                        max: MAX_BACKGROUND_EXTENT,
                        step: 0.1,
                    },
                    value: composition.extent.map_or_else(String::new, format_number),
                });
            }
            if media.composition.is_some() {
                rows.push(Field {
                    key: "background.media.fade".to_owned(),
                    label: "Fade",
                    kind: FieldKind::Number {
                        min: 0.0,
                        max: MAX_BACKGROUND_FADE,
                        step: 0.1,
                    },
                    value: media
                        .composition
                        .as_ref()
                        .and_then(|composition| composition.fade)
                        .map_or_else(String::new, format_number),
                });
            }
        }
        sections.push(Section {
            title: "Background media",
            rows,
        });
    }
    sections
}

/// The kind of a declared asset, so a field only appears where it applies.
fn media_kind(document: &ThemeDocument, asset_id: &str) -> Option<AssetKind> {
    document
        .assets
        .iter()
        .find(|asset| asset.id == asset_id)
        .map(|asset| asset.kind)
}

fn is_media_kind(kind: AssetKind) -> bool {
    matches!(kind, AssetKind::Image | AssetKind::Svg | AssetKind::Video)
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
pub fn apply_document(document: &mut ThemeDocument, key: &str, value: &str) -> Result<(), String> {
    match key {
        "artboard.width" | "artboard.height" => {
            let parsed = value
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("{} needs a number", friendly(key)))?;
            if !parsed.is_finite() || parsed < MIN_SIZE || parsed > MAX_ARTBOARD {
                return Err(format!(
                    "{} must be a whole number from 1 to {MAX_ARTBOARD:.0}",
                    friendly(key)
                ));
            }
            if parsed.fract() != 0.0 {
                return Err(format!("{} is a whole number", friendly(key)));
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
            // The source editor paints the background from palette tokens only,
            // and the colour belongs to the media once media is present.
            let text = value.trim();
            let slot = if text.is_empty() {
                None
            } else if text.starts_with("palette.") {
                Some(paint(text)?)
            } else {
                return Err("A background colour is a palette.<id> reference".to_owned());
            };
            match &mut document.background.media {
                Some(media) => media.color = slot,
                None => document.background.color = slot,
            }
        }
        "background.media" => {
            let asset_id = value.trim();
            if asset_id.is_empty() {
                document.background.media = None;
                document.background.color = None;
                return Ok(());
            }
            let kind = media_kind(document, asset_id)
                .ok_or_else(|| format!("No asset named {asset_id}"))?;
            if !is_media_kind(kind) {
                return Err(format!("{asset_id} is not a picture or video"));
            }
            match &mut document.background.media {
                Some(media) => media.asset_id = asset_id.to_owned(),
                None => {
                    document.background.media = Some(BackgroundMedia {
                        asset_id: asset_id.to_owned(),
                        color: None,
                        bounds: BackgroundBounds::Display,
                        offset_x: 0.0,
                        offset_y: 0.0,
                        scale: 1.0,
                        flip_x: false,
                        flip_y: false,
                        fallback_frame: None,
                        composition: None,
                    });
                }
            }
        }
        key if key.starts_with("background.media.") => {
            let field = key.trim_start_matches("background.media.");
            let media = document
                .background
                .media
                .as_mut()
                .ok_or_else(|| "The background has no media".to_owned())?;
            let number = |min: f64, max: f64| -> Result<f64, String> {
                let parsed = value
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("{field} needs a number"))?;
                if !parsed.is_finite() || parsed < min || parsed > max {
                    return Err(format!("{field} runs from {min} to {max}"));
                }
                Ok(parsed)
            };
            match field {
                "bounds" => {
                    media.bounds = match value.trim() {
                        "theme" => BackgroundBounds::Theme,
                        "display" => BackgroundBounds::Display,
                        other => return Err(format!("{other} is not a background bound")),
                    }
                }
                "scale" => media.scale = number(MIN_BACKGROUND_SCALE, MAX_BACKGROUND_SCALE)?,
                "offsetX" => media.offset_x = number(MIN_BACKGROUND_OFFSET, MAX_BACKGROUND_OFFSET)?,
                "offsetY" => media.offset_y = number(MIN_BACKGROUND_OFFSET, MAX_BACKGROUND_OFFSET)?,
                "flipX" | "flipY" => {
                    let flag = match value.trim() {
                        "true" => true,
                        "false" => false,
                        other => return Err(format!("{other} is not true or false")),
                    };
                    if field == "flipX" {
                        media.flip_x = flag;
                    } else {
                        media.flip_y = flag;
                    }
                }
                "fallbackFrame" => {
                    let text = value.trim();
                    if text.is_empty() {
                        media.fallback_frame = None;
                    } else {
                        let frame: u32 = text
                            .parse()
                            .map_err(|_| "A fallback frame is a whole number".to_owned())?;
                        media.fallback_frame = Some(frame);
                    }
                }
                "anchor" | "extent" | "fade" => {
                    if document.background.mode != BackgroundMode::Advanced {
                        return Err("Composition needs advanced mode".to_owned());
                    }
                    match field {
                        "anchor" => {
                            let anchor = match value.trim() {
                                "full" => None,
                                "top" => Some(BackgroundAnchor::Top),
                                "right" => Some(BackgroundAnchor::Right),
                                "bottom" => Some(BackgroundAnchor::Bottom),
                                "left" => Some(BackgroundAnchor::Left),
                                "center" => Some(BackgroundAnchor::Center),
                                other => {
                                    return Err(format!("{other} is not a background anchor"));
                                }
                            };
                            media.composition = anchor.map(|anchor| BackgroundComposition {
                                anchor,
                                extent: None,
                                fade: None,
                            });
                        }
                        "extent" => {
                            let composition = media
                                .composition
                                .as_mut()
                                .ok_or_else(|| "Choose an edge anchor before a size".to_owned())?;
                            if composition.anchor == BackgroundAnchor::Center {
                                return Err("A centred composition has no size".to_owned());
                            }
                            composition.extent =
                                Some(number(MIN_BACKGROUND_EXTENT, MAX_BACKGROUND_EXTENT)?);
                        }
                        "fade" => {
                            let composition = media
                                .composition
                                .as_mut()
                                .ok_or_else(|| "Choose an anchor before a fade".to_owned())?;
                            let text = value.trim();
                            if text.is_empty() || text == "0" {
                                composition.fade = None;
                            } else {
                                composition.fade =
                                    Some(number(MIN_BACKGROUND_FADE, MAX_BACKGROUND_FADE)?);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                _ => return Err(format!("{field} is not a background media field")),
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
        let (token, style) = match run {
            TextRun::Literal {
                text,
                type_token,
                style,
            } => {
                rows.push(Field {
                    key: format!("kind.content.runs.{index}.text"),
                    label: "Text",
                    kind: FieldKind::Text,
                    value: text.clone(),
                });
                (type_token, style)
            }
            TextRun::Value {
                type_token, style, ..
            } => (type_token, style),
        };
        rows.push(Field {
            key: format!("kind.content.runs.{index}.typeToken"),
            label: "Token",
            kind: FieldKind::Text,
            value: token.clone(),
        });
        rows.push(Field {
            key: format!("kind.content.runs.{index}.fontSize"),
            label: "Size",
            kind: FieldKind::Number {
                min: 1.0,
                max: MAX_SCALE * 100.0,
                step: 1.0,
            },
            value: style
                .get("fontSize")
                .and_then(|value| match value {
                    StyleValue::Literal(literal) => literal.value.as_f64(),
                    StyleValue::Ref(_) => None,
                })
                .map_or_else(String::new, format_number),
        });
        rows.push(Field {
            key: format!("kind.content.runs.{index}.fill"),
            label: "Colour",
            kind: FieldKind::Colour,
            value: paint_text(style.get("fill")),
        });
    }
    rows.extend([
        choice("kind.content.align", "Align", align_name(content.align)),
        choice(
            "kind.content.overflow",
            "Overflow",
            overflow_name(content.overflow),
        ),
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

/// One chart setting the panel edits: a number, a flag or a paint.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Setting {
    Number(&'static str, &'static str),
    Flag(&'static str, &'static str),
    Paint(&'static str, &'static str),
}

const GAUGE: &[Setting] = &[
    Setting::Number("min", "Min"),
    Setting::Number("max", "Max"),
    Setting::Number("startAngle", "Start angle"),
    Setting::Number("endAngle", "End angle"),
    Setting::Number("thickness", "Thickness"),
    Setting::Flag("roundCap", "Round cap"),
    Setting::Number("gradientSegments", "Gradient segments"),
    Setting::Number("segmentDivisor", "Segment divisor"),
    Setting::Paint("track", "Track"),
    Setting::Paint("progress", "Progress"),
];
const LINE: &[Setting] = &[
    Setting::Number("min", "Min"),
    Setting::Number("max", "Max"),
    Setting::Flag("showAxes", "Show axes"),
    Setting::Number("windowSeconds", "Window (s)"),
    Setting::Number("lineWidth", "Line width"),
    Setting::Number("markerSize", "Marker size"),
    Setting::Flag("showMarkers", "Show markers"),
    Setting::Paint("stroke", "Stroke"),
];
const BAR: &[Setting] = &[
    Setting::Number("min", "Min"),
    Setting::Number("max", "Max"),
    Setting::Number("barWidth", "Bar width"),
    Setting::Number("cornerRadius", "Corner radius"),
    Setting::Number("trackCornerRadius", "Track corner radius"),
    Setting::Number("categoryGapPercent", "Category gap %"),
    Setting::Flag("vertical", "Vertical"),
    Setting::Flag("showAxes", "Show axes"),
    Setting::Paint("fill", "Fill"),
    Setting::Paint("stroke", "Stroke"),
];
const PIE: &[Setting] = &[
    Setting::Number("startAngle", "Start angle"),
    Setting::Number("padAngle", "Pad angle"),
    Setting::Number("innerRadiusPercent", "Inner radius %"),
    Setting::Number("outerRadiusPercent", "Outer radius %"),
    Setting::Paint("fill", "Fill"),
];
const RADAR: &[Setting] = &[
    Setting::Number("min", "Min"),
    Setting::Number("max", "Max"),
    Setting::Number("splitNumber", "Split number"),
    Setting::Number("lineWidth", "Line width"),
    Setting::Number("markerSize", "Marker size"),
    Setting::Flag("showAxes", "Show axes"),
    Setting::Flag("showMarkers", "Show markers"),
    Setting::Paint("stroke", "Stroke"),
];
const POLAR_BAR: &[Setting] = &[
    Setting::Number("min", "Min"),
    Setting::Number("max", "Max"),
    Setting::Number("startAngle", "Start angle"),
    Setting::Number("barWidth", "Bar width"),
    Setting::Number("cornerRadius", "Corner radius"),
    Setting::Number("trackCornerRadius", "Track corner radius"),
    Setting::Number("categoryGapPercent", "Category gap %"),
    Setting::Number("innerRadiusPercent", "Inner radius %"),
    Setting::Number("outerRadiusPercent", "Outer radius %"),
    Setting::Flag("showAxes", "Show axes"),
    Setting::Paint("fill", "Fill"),
    Setting::Paint("track", "Track"),
];

fn chart_settings(family: ChartFamily) -> &'static [Setting] {
    match family {
        ChartFamily::Gauge => GAUGE,
        ChartFamily::Line => LINE,
        ChartFamily::Bar => BAR,
        ChartFamily::Pie => PIE,
        ChartFamily::Radar => RADAR,
        ChartFamily::PolarBar => POLAR_BAR,
    }
}

fn chart_family_name(family: ChartFamily) -> &'static str {
    match family {
        ChartFamily::Gauge => "gauge",
        ChartFamily::Line => "line",
        ChartFamily::Bar => "bar",
        ChartFamily::Pie => "pie",
        ChartFamily::Radar => "radar",
        ChartFamily::PolarBar => "polarBar",
    }
}

fn chart_family(name: &str) -> Option<ChartFamily> {
    match name {
        "gauge" => Some(ChartFamily::Gauge),
        "line" => Some(ChartFamily::Line),
        "bar" => Some(ChartFamily::Bar),
        "pie" => Some(ChartFamily::Pie),
        "radar" => Some(ChartFamily::Radar),
        "polarBar" => Some(ChartFamily::PolarBar),
        _ => None,
    }
}

fn chart_rows(content: &ChartContent) -> Vec<Field> {
    let mut rows = vec![choice(
        "kind.content.family",
        "Family",
        chart_family_name(content.family),
    )];
    for setting in chart_settings(content.family) {
        let key = match setting {
            Setting::Number(key, _) | Setting::Flag(key, _) | Setting::Paint(key, _) => *key,
        };
        let label = match setting {
            Setting::Number(_, label) | Setting::Flag(_, label) | Setting::Paint(_, label) => label,
        };
        let value = content.settings.0.get(key);
        let (kind, text) = match setting {
            Setting::Number(..) => (
                FieldKind::Number {
                    min: f64::MIN,
                    max: f64::MAX,
                    step: 1.0,
                },
                value
                    .and_then(serde_json::Value::as_f64)
                    .map_or_else(String::new, format_number),
            ),
            Setting::Flag(..) => (
                FieldKind::Toggle,
                value
                    .and_then(serde_json::Value::as_bool)
                    .map_or_else(String::new, |flag| flag.to_string()),
            ),
            Setting::Paint(..) => (FieldKind::Colour, chart_paint_text(value)),
        };
        rows.push(Field {
            key: format!("kind.chart.{key}"),
            label,
            kind,
            value: text,
        });
    }
    rows
}

/// A chart paint reads as a hex colour, its palette reference, or the shape it
/// has when a field cannot edit it.
fn chart_paint_text(value: Option<&serde_json::Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    match value.get("kind").and_then(serde_json::Value::as_str) {
        Some("solid") => value
            .get("color")
            .and_then(serde_json::Value::as_str)
            .map_or_else(String::new, ToOwned::to_owned),
        Some(other) => other.to_owned(),
        None => value
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .map_or_else(String::new, ToOwned::to_owned),
    }
}

/// A setting field's own JSON shape: a number, a flag or a chart fill.
fn chart_setting(key: &str, value: &str) -> Result<Option<serde_json::Value>, String> {
    let found = [GAUGE, LINE, BAR, PIE, RADAR, POLAR_BAR]
        .iter()
        .flat_map(|table| table.iter())
        .find(|setting| match setting {
            Setting::Number(held, _) | Setting::Flag(held, _) | Setting::Paint(held, _) => {
                *held == key
            }
        })
        .ok_or_else(|| format!("{key} is not a chart setting"))?;
    let text = value.trim();
    match found {
        Setting::Number(..) => {
            if text.is_empty() {
                return Ok(None);
            }
            let number = text
                .parse::<f64>()
                .map_err(|_| format!("{key} needs a number"))?;
            if !number.is_finite() {
                return Err(format!("{key} needs a finite number"));
            }
            Ok(Some(serde_json::json!(number)))
        }
        Setting::Flag(..) => match text {
            "true" => Ok(Some(serde_json::json!(true))),
            "false" => Ok(Some(serde_json::json!(false))),
            other => Err(format!("{other} is not true or false")),
        },
        Setting::Paint(..) => {
            if text.is_empty() {
                return Ok(None);
            }
            Ok(Some(match paint(text)? {
                StyleValue::Ref(reference) => serde_json::json!({ "ref": reference.r#ref }),
                StyleValue::Literal(literal) => {
                    serde_json::json!({ "kind": "solid", "color": literal.value })
                }
            }))
        }
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
    let node =
        find_mut(&mut document.nodes, id).ok_or_else(|| "No node with that id".to_owned())?;
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
        return Err(format!(
            "{asset_id} is not the kind of asset this node draws"
        ));
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
        "unitDisplay" => {
            binding.unit_display = match value.trim() {
                "" | "default" => None,
                "none" => Some(UnitDisplay::None),
                "short" => Some(UnitDisplay::Short),
                "long" => Some(UnitDisplay::Long),
                other => return Err(format!("{other} is not a unit display")),
            };
        }
        "format" => {
            let text = value.trim();
            if text.len() > 64 {
                return Err("A format is at most 64 characters".to_owned());
            }
            if text.chars().any(char::is_control) {
                return Err("A format is one line".to_owned());
            }
            binding.format = (!text.is_empty()).then(|| text.to_owned());
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
            ("fontSize", TextRun::Literal { style, .. } | TextRun::Value { style, .. }) => {
                let text = value.trim();
                if text.is_empty() {
                    style.remove("fontSize");
                } else {
                    let size: f64 = text.parse().map_err(|_| "Size needs a number".to_owned())?;
                    if !size.is_finite() || size < 1.0 {
                        return Err("Size must be at least 1".to_owned());
                    }
                    style.insert(
                        "fontSize".to_owned(),
                        StyleValue::Literal(StyleLiteral {
                            value: serde_json::json!(size),
                        }),
                    );
                }
                Ok(())
            }
            ("fill", TextRun::Literal { style, .. } | TextRun::Value { style, .. }) => {
                if value.trim().is_empty() {
                    style.remove("fill");
                } else {
                    style.insert("fill".to_owned(), paint(value)?);
                }
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
        "kind.content.lineHeight" => {
            let style: &mut StyleMap = &mut node.style;
            let text = value.trim();
            if text.is_empty() {
                style.remove("lineHeight");
            } else {
                let height: f64 = text
                    .parse()
                    .map_err(|_| "Line height needs a number".to_owned())?;
                if !height.is_finite() || height < 0.01 {
                    return Err("Line height must be at least 0.01".to_owned());
                }
                style.insert(
                    "lineHeight".to_owned(),
                    StyleValue::Literal(StyleLiteral {
                        value: serde_json::json!(height),
                    }),
                );
            }
        }
        "kind.content.family" => match &mut node.kind {
            NodeKind::Chart { content } => {
                let name = value.trim();
                content.family =
                    chart_family(name).ok_or_else(|| format!("{name} is not a chart family"))?;
            }
            _ => return Err("Only a chart has a family".to_owned()),
        },
        key if key.starts_with("kind.chart.") => {
            let setting = key.trim_start_matches("kind.chart.");
            let NodeKind::Chart { content } = &mut node.kind else {
                return Err("Only a chart has chart settings".to_owned());
            };
            match chart_setting(setting, value)? {
                Some(json) => {
                    content.settings.0.insert(setting.to_owned(), json);
                }
                None => {
                    content.settings.0.remove(setting);
                }
            }
        }
        "kind.content.align"
        | "kind.content.overflow"
        | "kind.content.verticalAlign"
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
        document.type_tokens.insert(
            "body".to_owned(),
            TypeToken {
                family: "Inter".to_owned(),
                weight: None,
                face: None,
            },
        );
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
                "kind.content.runs.0.fontSize",
                "kind.content.runs.0.fill",
                "kind.content.align",
                "kind.content.overflow",
                "kind.content.verticalAlign",
                "kind.content.vertical",
                "kind.content.lineHeight",
            ]
        );
        assert_eq!(section.rows[0].value, "Hello");
        assert_eq!(section.rows[4].value, "left");
    }

    #[test]
    fn run_text_and_a_known_token_are_written() {
        let mut document = with_token(document(vec![text_node()]));
        assert!(
            apply(
                &mut document,
                "label",
                "kind.content.runs.0.text",
                "Hi there"
            )
            .is_ok()
        );
        assert!(
            apply(
                &mut document,
                "label",
                "kind.content.runs.0.typeToken",
                "typeTokens.body"
            )
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
        assert!(
            apply(
                &mut document,
                "label",
                "kind.content.runs.0.typeToken",
                "title"
            )
            .is_err()
        );
        assert!(apply(&mut document, "label", "kind.content.runs.7.text", "x").is_err());
        assert!(apply(&mut document, "label", "kind.content.runs.zero.text", "x").is_err());
    }

    #[test]
    fn alignment_and_overflow_take_their_named_values() {
        let mut document = with_token(document(vec![text_node()]));
        assert!(apply(&mut document, "label", "kind.content.align", "center").is_ok());
        assert!(apply(&mut document, "label", "kind.content.overflow", "ellipsis").is_ok());
        assert!(
            apply(
                &mut document,
                "label",
                "kind.content.verticalAlign",
                "middle"
            )
            .is_ok()
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
        assert!(apply_document(&mut document, "artboard.width", "12.5").is_err());
        assert!(apply_document(&mut document, "artboard.width", "16385").is_err());
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
    fn the_background_colour_is_a_palette_reference() {
        let mut document = document(vec![]);
        assert!(apply_document(&mut document, "background.color", "#101014").is_err());
        assert!(apply_document(&mut document, "background.color", "palette.desk").is_ok());
        assert!(matches!(
            document.background.color,
            Some(StyleValue::Ref(_))
        ));
        assert!(apply_document(&mut document, "background.color", "").is_ok());
        assert_eq!(document.background.color, None);
        assert!(apply_document(&mut document, "background.color", "navy").is_err());
        assert!(apply_document(&mut document, "background.mode", "advanced").is_ok());
        assert_eq!(document.background.mode, BackgroundMode::Advanced);
    }

    #[test]
    fn background_media_is_chosen_from_the_declared_assets() {
        use yunari_model::document::AssetKind;
        let mut document = document(vec![]);
        document.assets.push(asset("pic", AssetKind::Image));
        document.assets.push(asset_mp4("clip"));
        assert!(apply_document(&mut document, "background.media", "missing").is_err());
        assert!(apply_document(&mut document, "background.media", "pic").is_ok());
        let media = document.background.media.as_ref().expect("media");
        assert_eq!(media.asset_id, "pic");
        assert_eq!(media.bounds, BackgroundBounds::Display);
        assert!(apply_document(&mut document, "background.media.bounds", "theme").is_ok());
        assert!(apply_document(&mut document, "background.media.scale", "2").is_ok());
        assert!(apply_document(&mut document, "background.media.scale", "9").is_err());
        assert!(apply_document(&mut document, "background.media.offsetX", "-3").is_ok());
        assert!(apply_document(&mut document, "background.media.flipX", "true").is_ok());
        // Composition is refused until the mode is advanced.
        assert!(apply_document(&mut document, "background.media.anchor", "top").is_err());
        apply_document(&mut document, "background.mode", "advanced").expect("advanced");
        assert!(apply_document(&mut document, "background.media.anchor", "top").is_ok());
        assert!(apply_document(&mut document, "background.media.extent", "0.5").is_ok());
        assert!(apply_document(&mut document, "background.media.fade", "0.3").is_ok());
        let media = document.background.media.as_ref().expect("media");
        let composition = media.composition.as_ref().expect("composition");
        assert_eq!(composition.anchor, BackgroundAnchor::Top);
        assert_eq!(composition.extent, Some(0.5));
        assert_eq!(composition.fade, Some(0.3));
        // With media present, the colour belongs to the media slot.
        assert!(apply_document(&mut document, "background.color", "palette.desk").is_ok());
        let media = document.background.media.as_ref().expect("media");
        assert!(media.color.is_some());
        assert_eq!(document.background.color, None);
        // A video asset shows a fallback frame field; clearing media clears it.
        assert!(apply_document(&mut document, "background.media", "clip").is_ok());
        assert!(apply_document(&mut document, "background.media.fallbackFrame", "3").is_ok());
        assert!(apply_document(&mut document, "background.media", "").is_ok());
        assert!(document.background.media.is_none());
    }

    fn asset_mp4(id: &str) -> yunari_model::document::AssetReference {
        let mut held = asset(id, yunari_model::document::AssetKind::Video);
        held.path = format!("assets/{id}.mp4");
        held
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

    fn asset(
        id: &str,
        kind: yunari_model::document::AssetKind,
    ) -> yunari_model::document::AssetReference {
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

    fn chart_node(family: ChartFamily) -> Node {
        let mut node = rect("chart", 0.0, 0.0, 120.0, 120.0);
        node.kind = NodeKind::Chart {
            content: ChartContent {
                family,
                settings: yunari_model::document::ChartSettings(serde_json::Map::new()),
            },
        };
        node
    }

    #[test]
    fn a_chart_shows_its_family_and_settings() {
        let document = document(vec![chart_node(ChartFamily::Gauge)]);
        let sections = sections(&document, "chart");
        let chart = sections
            .iter()
            .find(|section| section.title == "Chart")
            .expect("a chart section");
        assert_eq!(chart.rows[0].key, "kind.content.family");
        assert_eq!(chart.rows[0].value, "gauge");
        assert_eq!(chart.rows[1].key, "kind.chart.min");
        assert_eq!(chart.rows[1].value, "");
    }

    #[test]
    fn chart_settings_are_written_in_their_own_shapes() {
        let mut document = document(vec![chart_node(ChartFamily::Bar)]);
        assert!(apply(&mut document, "chart", "kind.chart.min", "10").is_ok());
        assert!(apply(&mut document, "chart", "kind.chart.vertical", "true").is_ok());
        assert!(apply(&mut document, "chart", "kind.chart.fill", "#ff0000").is_ok());
        assert!(
            apply(
                &mut document,
                "chart",
                "kind.chart.stroke",
                "palette.accent"
            )
            .is_ok()
        );
        assert!(apply(&mut document, "chart", "kind.content.family", "pie").is_ok());
        let NodeKind::Chart { content } = &document.nodes[0].kind else {
            panic!("still a chart");
        };
        assert_eq!(content.family, ChartFamily::Pie);
        assert_eq!(content.settings.0["min"], serde_json::json!(10.0));
        assert_eq!(content.settings.0["vertical"], serde_json::json!(true));
        assert_eq!(
            content.settings.0["fill"],
            serde_json::json!({ "kind": "solid", "color": "#ff0000" })
        );
        assert_eq!(
            content.settings.0["stroke"],
            serde_json::json!({ "ref": "palette.accent" })
        );
        assert!(apply(&mut document, "chart", "kind.chart.min", "").is_ok());
        let NodeKind::Chart { content } = &document.nodes[0].kind else {
            panic!("still a chart");
        };
        assert!(!content.settings.0.contains_key("min"));
        assert!(apply(&mut document, "chart", "kind.content.family", "bubble").is_err());
        assert!(apply(&mut document, "chart", "kind.chart.nonsense", "1").is_err());
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
