//! Schema-3 Fabric scene objects → paint geometry.
//!
//! The scene is serialized Fabric state, not a second document format: each
//! object's visual fields become one display op, and a group contributes only
//! its position to the children it owns. Rotation, scale and the origin point
//! `left`/`top` pins compose into one [`Affine`] per object, in parent space.

use serde_json::Value;
use yunari_model::document::{
    FabricObject, StyleMap, TextAlign, TextContent, TextOverflow, ThemeDocument, VerticalAlign,
};
use yunari_providers::units::UnitPreferences;

use crate::affine::Affine;
use crate::display_list::{ChartFamily, DisplayList, Op, Paint, Shape, Stroke};
use crate::plan::{
    IssueCode, PaintSpec, PlanIssue, ResolvedStyle, page_surface, place_paint,
    resolve_object_paint, run_style_for,
};
use crate::reading::{self, Readings};
use crate::text::{
    Fonts, TextAlign as PlanTextAlign, TextOverflow as PlanTextOverflow, TextRequest, TextRunPlan,
    VerticalAlign as PlanVerticalAlign, layout,
};

/// Build the frame from a schema-3 document's Fabric scene.
///
/// Later conversions (text, paths, images, charts, glass) extend `object_ops`;
/// the walk, transform and error contract stay here.
pub fn build(
    document: &ThemeDocument,
    fonts: &mut Fonts,
    readings: &Readings,
    preferences: &UnitPreferences,
) -> (DisplayList, Vec<PlanIssue>) {
    let width = document.artboard.width as f32;
    let height = document.artboard.height as f32;
    let mut issues = Vec::new();
    let (background, backdrop) = page_surface(document, width, height, &mut issues);
    let mut list = DisplayList {
        width,
        height,
        background,
        backdrop,
        ops: Vec::new(),
    };

    for object in &document.scene.objects {
        emit(
            object,
            Affine::IDENTITY,
            1.0,
            document,
            fonts,
            readings,
            preferences,
            &mut list,
            &mut issues,
        );
    }
    if !document.scene.objects.is_empty() && list.ops.is_empty() {
        issues.push(PlanIssue {
            code: IssueCode::UnsupportedContent,
            node_id: None,
            detail: "The authored scene produced no drawable ops.".into(),
        });
    }
    (list, issues)
}

/// Every object, roots first, then each group's children in depth order.
pub fn walk(objects: &[FabricObject]) -> Vec<&FabricObject> {
    let mut out = Vec::new();
    for object in objects {
        collect(object, &mut out);
    }
    out
}

fn collect<'a>(object: &'a FabricObject, out: &mut Vec<&'a FabricObject>) {
    out.push(object);
    for child in &object.children {
        collect(child, out);
    }
}

/// Fabric fields live in the object's flattened `data` map: only identity and
/// nesting are typed, so a reader must refuse a wrong-shaped value rather than
/// coerce it to a number or a paint.
fn number(object: &FabricObject, key: &str) -> Option<f64> {
    object
        .data
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

fn text<'a>(object: &'a FabricObject, key: &str) -> Option<&'a str> {
    object.data.get(key).and_then(Value::as_str)
}

/// Fabric stores a Textbox's box in `yunariText.box` and omits the flattened
/// `height`. Only text geometry reads that authored box; shapes keep their own
/// Fabric width/height.
fn local_size(object: &FabricObject) -> (f32, f32) {
    let kind = object.object_type.as_deref().unwrap_or("");
    let authored = (matches!(kind, "Textbox" | "IText" | "Text")
        && object.data.get("height").is_none())
    .then(|| {
        object
            .data
            .get("yunariText")
            .and_then(Value::as_object)
            .and_then(|payload| payload.get("box"))
            .and_then(Value::as_object)
    })
    .flatten();
    let (width, height) = match authored {
        Some(box_) => (
            box_.get("width").and_then(Value::as_f64).unwrap_or(0.0),
            box_.get("height").and_then(Value::as_f64).unwrap_or(0.0),
        ),
        None => (
            number(object, "width").unwrap_or(0.0),
            number(object, "height").unwrap_or(0.0),
        ),
    };
    (width as f32, height as f32)
}

/// Fabric's `visible` is a boolean on the object; absent means shown. A hidden
/// object paints nothing, and nothing under it paints either.
fn visible(object: &FabricObject) -> bool {
    object
        .data
        .get("visible")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// How far the object's leading edge sits from its `left`/`top` anchor. Fabric
/// stores the anchor at the origin named by `originX`/`originY`, so a centre
/// origin puts the leading edge half a *scaled* size back.
fn origin_offset(origin: Option<&str>, scaled_size: f32) -> f32 {
    match origin.unwrap_or("center") {
        "left" | "top" => 0.0,
        "right" | "bottom" => scaled_size,
        // Fabric's centre is the default when no origin is authored.
        _ => scaled_size / 2.0,
    }
}

/// The centre the object's `left`/`top` anchors, in parent space.
fn centre(object: &FabricObject) -> (f32, f32) {
    // A text object's centre comes from its authored box, not the missing
    // flattened height; a group has no box of its own and is unchanged.
    let (local_width, local_height) = local_size(object);
    let width = local_width * number(object, "scaleX").unwrap_or(1.0) as f32;
    let height = local_height * number(object, "scaleY").unwrap_or(1.0) as f32;
    let x = number(object, "left").unwrap_or(0.0) as f32
        - origin_offset(text(object, "originX"), width)
        + width / 2.0;
    let y = number(object, "top").unwrap_or(0.0) as f32
        - origin_offset(text(object, "originY"), height)
        + height / 2.0;
    (x, y)
}

fn scale_affine(object: &FabricObject) -> Affine {
    Affine::scale(
        number(object, "scaleX").unwrap_or(1.0) as f32,
        number(object, "scaleY").unwrap_or(1.0) as f32,
    )
}

/// A shape's own matrix: rotate and scale about its centre, then place that
/// centre. The leading-edge correction makes `(0, 0)` the shape's top-left
/// corner, which is where the backend's rect/ellipse paths start.
fn shape_matrix(object: &FabricObject) -> Affine {
    let (width, height) = local_size(object);
    let (cx, cy) = centre(object);
    Affine::translate(-width / 2.0, -height / 2.0)
        .then(scale_affine(object))
        .then(Affine::rotate(number(object, "angle").unwrap_or(0.0) as f32))
        .then(Affine::translate(cx, cy))
}

/// A group's children are stored around its centre, which is the local origin
/// of a Fabric group: no leading-edge correction, because the group paints no
/// box of its own. Composing this once maps every child into parent space.
fn group_matrix(object: &FabricObject) -> Affine {
    let (cx, cy) = centre(object);
    scale_affine(object)
        .then(Affine::rotate(number(object, "angle").unwrap_or(0.0) as f32))
        .then(Affine::translate(cx, cy))
}

#[allow(clippy::too_many_arguments)]
fn emit(
    object: &FabricObject,
    parent: Affine,
    opacity: f32,
    document: &ThemeDocument,
    fonts: &mut Fonts,
    readings: &Readings,
    preferences: &UnitPreferences,
    list: &mut DisplayList,
    issues: &mut Vec<PlanIssue>,
) {
    if !visible(object) {
        return;
    }
    let kind = object.object_type.as_deref().unwrap_or("");
    let is_group = kind == "Group" || (!object.children.is_empty() && kind.is_empty());
    let object_opacity = opacity * object_opacity(object);

    if is_group {
        // Fabric groups paint no box of their own, so there is no interior to
        // sample through. A treatment is valid in the model but not drawable:
        // report it rather than silently dropping the authored frosting.
        if let Some(glass) = &object.yunari_glass
            && glass.blur_radius > 0.0
        {
            issues.push(PlanIssue {
                code: IssueCode::UnsupportedContent,
                node_id: Some(object.id.clone()),
                detail: "Glass needs a closed shape to sample through.".into(),
            });
        }
    } else {
        match object_ops(
            object,
            kind,
            document,
            object_opacity,
            fonts,
            readings,
            preferences,
            issues,
        ) {
            Some((shape, fill, stroke)) => {
                let transform = shape_matrix(object).then(parent);
                // Glass samples the frame under the base shape, so its blur op
                // goes immediately before the base. An unsupported blur is an
                // issue, never a blank surface: the base shape still paints.
                if let Some(glass) = &object.yunari_glass
                    && glass.blur_radius > 0.0
                {
                    if glass_supports_shape(&shape) {
                        list.ops.push(Op {
                            transform,
                            shape: Shape::Glass {
                                shape: Box::new(shape.clone()),
                                blur_radius: glass.blur_radius as f32,
                            },
                            fill: None,
                            stroke: None,
                        });
                    } else {
                        issues.push(PlanIssue {
                            code: IssueCode::UnsupportedContent,
                            node_id: Some(object.id.clone()),
                            detail: "Glass needs a closed shape to sample through.".into(),
                        });
                    }
                }
                list.ops.push(Op {
                    transform,
                    shape,
                    fill,
                    stroke,
                });
            }
            None => issues.push(PlanIssue {
                code: IssueCode::UnsupportedContent,
                node_id: Some(object.id.clone()),
                detail: format!(
                    "Fabric object type {:?} has no renderer.",
                    if kind.is_empty() { "<none>" } else { kind }
                ),
            }),
        }
    }

    let child_parent = if is_group {
        group_matrix(object).then(parent)
    } else {
        shape_matrix(object).then(parent)
    };
    for child in &object.children {
        emit(
            child,
            child_parent,
            object_opacity,
            document,
            fonts,
            readings,
            preferences,
            list,
            issues,
        );
    }
}

/// A Fabric text object's authored runs, face, size and layout → one laid-out
/// paragraph. `yunariText` is the same authored content the major-1 node owns;
/// it is deserialized into the model type so formatting has one owner. Run
/// styles win over the object's flattened Fabric fields, which is how the
/// editor resolves a token's face, size and tracking onto the object.
#[allow(clippy::too_many_arguments)]
fn text_shape(
    object: &FabricObject,
    document: &ThemeDocument,
    fonts: &mut Fonts,
    readings: &Readings,
    preferences: &UnitPreferences,
    opacity: f32,
    issues: &mut Vec<PlanIssue>,
) -> Option<crate::text::TextLayout> {
    let content: TextContent = match object.data.get("yunariText") {
        Some(authored) => serde_json::from_value(authored.clone())
            .map_err(|error| {
                issues.push(PlanIssue {
                    code: IssueCode::UnsupportedContent,
                    node_id: Some(object.id.clone()),
                    detail: format!("A yunariText payload is not readable: {error}"),
                });
            })
            .ok()?,
        // A text object with no authored payload still carries its rendered
        // string and face: one body-token literal run, as the source lifter does.
        None => flat_text_content(object)?,
    };

    // The object's fill is the fallback run colour: an authored ref in
    // `yunariPaint` wins over Fabric's flattened CSS snapshot, matching shapes.
    let id = if object.id.is_empty() {
        None
    } else {
        Some(object.id.as_str())
    };
    let fill = object
        .data
        .get("yunariPaint")
        .and_then(Value::as_object)
        .and_then(|map| map.get("fill"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| text(object, "fill").filter(|value| !value.is_empty()))
        .and_then(|value| {
            resolve_object_paint(
                document,
                value,
                id,
                &format!("scene/objects/{}/fill", object.id),
                issues,
            )
        });
    let node_style = ResolvedStyle {
        fill,
        ..ResolvedStyle::default()
    };

    let mut runs = Vec::new();
    for run in &content.runs {
        let (text, type_token, style) = match run {
            yunari_model::document::TextRun::Literal {
                text,
                type_token,
                style,
            } => (text.clone(), type_token.as_str(), style),
            yunari_model::document::TextRun::Value {
                type_token, style, ..
            } => (
                reading::value_text_for(document, &object.id, run, readings, preferences, issues),
                type_token.as_str(),
                style,
            ),
        };
        if text.is_empty() {
            continue;
        }
        let mut style = style.clone();
        // Fabric resolves the face, size, weight and tracking onto the object
        // from the first run's token. Only fields the run did not author take
        // the object's flattened value, so the run stays the rule owner.
        // `charSpacing` is derived from that first run only, so it is not
        // copied into later runs that authored no tracking of their own.
        fill_object_style(object, &mut style, runs.is_empty());
        let path = format!("scene/objects/{}/yunariText/runs", object.id);
        runs.push(TextRunPlan {
            text,
            style: run_style_for(
                document,
                id,
                &path,
                type_token,
                &style,
                &node_style,
                opacity,
                issues,
            ),
        });
    }
    if runs.is_empty() {
        return None;
    }
    // Line height is a whole-object Fabric property; tracking arrives per run
    // as Fabric's charSpacing (1/1000 em) and is converted in `fill_object_style`.
    let request = TextRequest {
        runs: &runs,
        box_: content
            .box_
            .map(|box_| (box_.width as f32, box_.height as f32)),
        align: match content.align {
            TextAlign::Left => PlanTextAlign::Left,
            TextAlign::Center => PlanTextAlign::Center,
            TextAlign::Right => PlanTextAlign::Right,
        },
        vertical_align: match content.vertical_align {
            VerticalAlign::Top => PlanVerticalAlign::Top,
            VerticalAlign::Middle => PlanVerticalAlign::Middle,
            VerticalAlign::Bottom => PlanVerticalAlign::Bottom,
        },
        overflow: match content.overflow {
            TextOverflow::Visible => PlanTextOverflow::Visible,
            TextOverflow::Clip => PlanTextOverflow::Clip,
            TextOverflow::Ellipsis => PlanTextOverflow::Ellipsis,
        },
        vertical: content.vertical,
        line_height: number(object, "lineHeight").unwrap_or(1.2) as f32,
    };
    Some(layout(fonts, &request))
}

/// The source lifter's fallback for a text object with no `yunariText`: one
/// literal run in the body token, with Fabric's flattened face values as the
/// run's own style. The same fields become token overrides where the object
/// carries them, so a theme with no globals still paints its authored text.
fn flat_text_content(object: &FabricObject) -> Option<TextContent> {
    use yunari_model::document::{StyleLiteral, StyleValue, TextRun};
    let body = text(object, "text")?;
    let mut style = StyleMap::new();
    let literal = |value: &Value| {
        StyleValue::Literal(StyleLiteral {
            value: value.clone(),
        })
    };
    // A string fill is a CSS colour; a Fabric gradient object carries stops.
    // Resolve its first stop through the one paint owner rather than writing
    // an unreadable object into the style, which would paint default white.
    match object.data.get("fill") {
        Some(value @ Value::String(_)) => {
            style.insert("color".to_owned(), literal(value));
        }
        Some(Value::Object(gradient)) => {
            if let Some(first) = gradient
                .get("colorStops")
                .and_then(Value::as_array)
                .and_then(|stops| stops.first())
                .and_then(|stop| stop.get("color"))
                .and_then(Value::as_str)
            {
                style.insert(
                    "color".to_owned(),
                    StyleValue::Literal(StyleLiteral {
                        value: Value::from(first),
                    }),
                );
            }
        }
        _ => {}
    }
    for key in ["fontSize", "lineHeight"] {
        if let Some(value) = object.data.get(key) {
            style.insert(key.to_owned(), literal(value));
        }
    }
    Some(TextContent {
        runs: vec![TextRun::Literal {
            text: body.to_owned(),
            type_token: "typeTokens.body".to_owned(),
            style,
        }],
        box_: None,
        overflow: TextOverflow::Visible,
        align: TextAlign::Left,
        vertical_align: VerticalAlign::Top,
        vertical: false,
    })
}

/// Fill a run's style from the object's flattened Fabric fields for properties
/// it did not author. `charSpacing` is Fabric's 1/1000 em, so a size is needed
/// to write the artboard-unit tracking the layout measures with.
fn fill_object_style(object: &FabricObject, style: &mut StyleMap, first_run: bool) {
    use yunari_model::document::{StyleLiteral, StyleValue};
    let literal = |key: &str, value: f64, style: &mut StyleMap| {
        style.entry(key.to_owned()).or_insert_with(|| {
            StyleValue::Literal(StyleLiteral {
                value: Value::from(value),
            })
        });
    };
    let string = |key: &str, value: &str, style: &mut StyleMap| {
        style.entry(key.to_owned()).or_insert_with(|| {
            StyleValue::Literal(StyleLiteral {
                value: Value::from(value),
            })
        });
    };
    if let Some(family) = text(object, "fontFamily") {
        // Pass the stack through; the shared style owner splits it once.
        string("fontFamily", family, style);
    }
    if let Some(weight) = text(object, "fontWeight") {
        string("fontWeight", weight, style);
    }
    if let Some(size) = number(object, "fontSize") {
        literal("fontSize", size, style);
    }
    let size = style
        .get("fontSize")
        .and_then(|value| match value {
            StyleValue::Literal(f) => f.value.as_f64(),
            StyleValue::Ref(_) => None,
        })
        .or_else(|| number(object, "fontSize"));
    if first_run
        && let (Some(char_spacing), Some(size)) = (number(object, "charSpacing"), size)
        && size > 0.0
    {
        // Fabric charSpacing is 1/1000 em; the layout owns pixels.
        literal("letterSpacing", char_spacing / 1000.0 * size, style);
    }
}

fn object_opacity(object: &FabricObject) -> f32 {
    number(object, "opacity")
        .map(|o| o.clamp(0.0, 1.0) as f32)
        .unwrap_or(1.0)
}

/// Glass samples the frame through a closed interior. A rect and an ellipse
/// always have one; a path has one only when its authored data ends in `Z`.
/// Text, images and charts have no outline to sample through, so they are
/// refused rather than blurred over their whole box.
fn glass_supports_shape(shape: &Shape) -> bool {
    match shape {
        Shape::Rect { .. } | Shape::Ellipse { .. } => true,
        Shape::Path { closed, .. } => *closed,
        _ => false,
    }
}

/// Fabric stores a path as `[["M", x, y], ["C", x1, y1, x2, y2, x, y], ...]`.
/// The major-1 source lifter writes the same commands into one SVG string,
/// which `crate::path::parse` reads into absolute commands. Return `None` for
/// a command array this renderer cannot read rather than drawing a wrong path.
fn path_data(object: &FabricObject) -> Option<String> {
    let commands = object.data.get("path")?.as_array()?;
    let mut data = String::new();
    for command in commands {
        let parts = command.as_array()?;
        let (head, arguments) = parts.split_first()?;
        if !data.is_empty() {
            data.push(' ');
        }
        data.push_str(head.as_str()?);
        for argument in arguments {
            let value = argument.as_f64()?;
            if !value.is_finite() {
                return None;
            }
            data.push(' ');
            data.push_str(&format_number(value));
        }
    }
    Some(data)
}

/// Whether every subpath of a Fabric path closes. The last command alone is
/// not enough: a theme can close early contours and end with an open one.
fn path_is_closed(object: &FabricObject) -> bool {
    let Some(commands) = object.data.get("path").and_then(Value::as_array) else {
        return false;
    };
    let (mut open, mut saw_subpath) = (false, false);
    for command in commands {
        let Some(head) = command.as_array().and_then(|parts| parts.first()).and_then(Value::as_str) else {
            return false;
        };
        match head.to_ascii_uppercase().as_str() {
            "M" => {
                if open {
                    return false;
                }
                open = true;
                saw_subpath = true;
            }
            "Z" => {
                if !open {
                    return false;
                }
                open = false;
            }
            _ => {}
        }
    }
    saw_subpath && !open
}

/// Mirror the source lifter's number text so the shared SVG reader sees the
/// same data whether it came from a Fabric array or a major-1 path node.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// The shape and paint of one non-group object. `None` means the type has no
/// renderer yet; the caller turns that into an issue.
#[allow(clippy::too_many_arguments)]
fn object_ops(
    object: &FabricObject,
    kind: &str,
    document: &ThemeDocument,
    opacity: f32,
    fonts: &mut Fonts,
    readings: &Readings,
    preferences: &UnitPreferences,
    issues: &mut Vec<PlanIssue>,
) -> Option<(Shape, Option<Paint>, Option<Stroke>)> {
    let width = number(object, "width").unwrap_or(0.0) as f32;
    let height = number(object, "height").unwrap_or(0.0) as f32;
    if matches!(kind, "Textbox" | "IText" | "Text") {
        let layout = text_shape(
            object,
            document,
            fonts,
            readings,
            preferences,
            opacity,
            issues,
        )?;
        // Text carries its own colour; there is no separate fill/stroke paint.
        return Some((Shape::Text(layout), None, None));
    }
    let shape = match kind {
        "Rect" => Shape::Rect {
            width,
            height,
            corner_radius: number(object, "rx")
                .or_else(|| number(object, "ry"))
                .unwrap_or(0.0) as f32,
        },
        "Ellipse" | "Circle" => Shape::Ellipse { width, height },
        "Path" => {
            let Some(data) = path_data(object) else {
                issues.push(PlanIssue {
                    code: IssueCode::UnsupportedContent,
                    node_id: Some(object.id.clone()),
                    detail: "Fabric path data is not readable.".into(),
                });
                return None;
            };
            // Port of scene-fabric's `hasClosedPath`: a path is closed only when
            // every subpath opened by M ends with Z. A final Z does not close
            // an earlier open contour, which real multi-subpath themes contain.
            let closed = path_is_closed(object);
            match crate::path::parse(&data) {
                Ok(commands) => Shape::Path { commands, closed },
                Err(error) => {
                    issues.push(PlanIssue {
                        code: IssueCode::UnsupportedContent,
                        node_id: Some(object.id.clone()),
                        detail: match error {
                            crate::path::PathError::Syntax(detail) => {
                                format!("Path data is not readable: {detail}")
                            }
                            crate::path::PathError::Unsupported(detail) => detail.into(),
                        },
                    });
                    return None;
                }
            }
        }
        "Image" => {
            let asset_id = object
                .data
                .get("yunariAsset")
                .and_then(Value::as_object)
                .and_then(|map| map.get("assetId"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty());
            let Some(asset_id) = asset_id else {
                issues.push(PlanIssue {
                    code: IssueCode::UnsupportedContent,
                    node_id: Some(object.id.clone()),
                    detail: "An image object has no yunariAsset reference.".into(),
                });
                return None;
            };
            let Some(asset) = document.assets.iter().find(|asset| asset.id == asset_id) else {
                issues.push(PlanIssue {
                    code: IssueCode::UnresolvedGlobal,
                    node_id: Some(object.id.clone()),
                    detail: format!("asset {asset_id} is not declared."),
                });
                return None;
            };
            let fit = match text(object, "yunariFit")
                .or_else(|| {
                    object
                        .data
                        .get("yunariImage")
                        .and_then(Value::as_object)
                        .and_then(|map| map.get("fit"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("contain")
            {
                "cover" => yunari_model::document::ImageFit::Cover,
                "stretch" => yunari_model::document::ImageFit::Stretch,
                _ => yunari_model::document::ImageFit::Contain,
            };
            let monochrome = object
                .data
                .get("yunariPaint")
                .and_then(Value::as_object)
                .and_then(|map| map.get("monochrome"))
                .and_then(Value::as_str)
                .and_then(|value| {
                    resolve_object_paint(
                        document,
                        value,
                        Some(&object.id),
                        &format!("scene/objects/{}/image/monochrome", object.id),
                        issues,
                    )
                })
                .and_then(|spec| match spec {
                    crate::plan::PaintSpec::Solid(color) => Some(color),
                    crate::plan::PaintSpec::Gradient { .. } => {
                        issues.push(PlanIssue {
                            code: IssueCode::UnreadablePaint,
                            node_id: Some(object.id.clone()),
                            detail: "A monochrome mask is a single colour.".into(),
                        });
                        None
                    }
                });
            // The colour's resolved alpha is authored truth; scale it, never
            // overwrite it with the object opacity.
            let monochrome = monochrome.map(|color| crate::plan::scale_alpha(color, opacity));
            return Some((
                Shape::Image {
                    asset_id: asset_id.to_owned(),
                    path: asset.path.clone(),
                    width,
                    height,
                    fit,
                    monochrome,
                    video: false,
                    video_loop: false,
                },
                None,
                None,
            ));
        }
        "YunariChart" => {
            // The family and raw settings bag are authored on the Fabric object;
            // the backend owns their meaning, as it does for major-1 charts.
            let family = match text(object, "family").and_then(|name| match name {
                "gauge" => Some(ChartFamily::Gauge),
                "line" => Some(ChartFamily::Line),
                "bar" => Some(ChartFamily::Bar),
                "pie" => Some(ChartFamily::Pie),
                "radar" => Some(ChartFamily::Radar),
                "polarBar" => Some(ChartFamily::PolarBar),
                _ => None,
            }) {
                Some(family) => family,
                None => {
                    issues.push(PlanIssue {
                        code: IssueCode::UnsupportedContent,
                        node_id: Some(object.id.clone()),
                        detail: "A chart object has no known family.".into(),
                    });
                    return None;
                }
            };
            let settings = object.data.get("settings").cloned().unwrap_or_default();
            let bindings = document
                .bindings
                .get(&object.id)
                .map(|bindings| {
                    bindings
                        .iter()
                        .map(|binding| binding.semantic_key.clone())
                        .collect()
                })
                .unwrap_or_default();
            return Some((
                Shape::Chart {
                    id: object.id.clone(),
                    family,
                    settings,
                    bindings,
                    width,
                    height,
                    opacity,
                },
                None,
                None,
            ));
        }
        // Any other unhandled type is reported by the caller.
        _ => return None,
    };

    let (fill, stroke) = paints(object, document, opacity, width, height, issues);
    Some((shape, fill, stroke))
}

fn paints(
    object: &FabricObject,
    document: &ThemeDocument,
    opacity: f32,
    width: f32,
    height: f32,
    issues: &mut Vec<PlanIssue>,
) -> (Option<Paint>, Option<Stroke>) {
    let refs = object.data.get("yunariPaint").and_then(Value::as_object);
    let mut paint = |property: &str, key: &str| -> Option<PaintSpec> {
        // Authored references own the paint; Fabric's flattened colour is the
        // resolved snapshot the editor wrote for a plain CSS fill.
        let authored = refs
            .and_then(|map| map.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                text(object, property)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })?;
        let id = if object.id.is_empty() {
            "?"
        } else {
            object.id.as_str()
        };
        let node_id = if object.id.is_empty() {
            None
        } else {
            Some(object.id.as_str())
        };
        resolve_object_paint(
            document,
            &authored,
            node_id,
            &format!("scene/objects/{id}/{property}"),
            issues,
        )
    };

    let fill = paint("fill", "fill").map(|spec| place_paint(spec, width, height, opacity));
    let stroke_width = number(object, "strokeWidth").unwrap_or(0.0) as f32;
    let stroke = paint("stroke", "stroke").map(|spec| Stroke {
        paint: place_paint(spec, width, height, opacity),
        width: stroke_width,
    });
    (fill, stroke)
}
