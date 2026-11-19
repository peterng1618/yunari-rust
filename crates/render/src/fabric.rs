//! Schema-3 Fabric scene objects → paint geometry.
//!
//! The scene is serialized Fabric state, not a second document format: each
//! object's visual fields become one display op, and a group contributes only
//! its position to the children it owns. Rotation, scale and the origin point
//! `left`/`top` pins compose into one [`Affine`] per object, in parent space.

use serde_json::Value;
use yunari_model::document::{FabricObject, ThemeDocument};
use yunari_providers::units::UnitPreferences;

use crate::affine::Affine;
use crate::display_list::{DisplayList, Op, Paint, Shape, Stroke};
use crate::plan::{
    IssueCode, PaintSpec, PlanIssue, page_surface, place_paint, resolve_object_paint,
};
use crate::reading::Readings;
use crate::text::Fonts;

/// Build the frame from a schema-3 document's Fabric scene.
///
/// Later conversions (text, paths, images, charts, glass) extend `object_ops`;
/// the walk, transform and error contract stay here.
pub fn build(
    document: &ThemeDocument,
    _fonts: &mut Fonts,
    _readings: &Readings,
    _preferences: &UnitPreferences,
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

fn scaled_size(object: &FabricObject, dimension: &str, scale_key: &str) -> f32 {
    let size = number(object, dimension).unwrap_or(0.0);
    let scale = number(object, scale_key).unwrap_or(1.0);
    (size * scale) as f32
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
    let width = scaled_size(object, "width", "scaleX");
    let height = scaled_size(object, "height", "scaleY");
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
    let width = number(object, "width").unwrap_or(0.0) as f32;
    let height = number(object, "height").unwrap_or(0.0) as f32;
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

fn emit(
    object: &FabricObject,
    parent: Affine,
    opacity: f32,
    document: &ThemeDocument,
    list: &mut DisplayList,
    issues: &mut Vec<PlanIssue>,
) {
    if !visible(object) {
        return;
    }
    let kind = object.object_type.as_deref().unwrap_or("");
    let is_group = kind == "Group" || (!object.children.is_empty() && kind.is_empty());
    let object_opacity = opacity * object_opacity(object);

    if !is_group {
        match object_ops(object, kind, document, object_opacity, issues) {
            Some((shape, fill, stroke)) => list.ops.push(Op {
                transform: shape_matrix(object).then(parent),
                shape,
                fill,
                stroke,
            }),
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
        emit(child, child_parent, object_opacity, document, list, issues);
    }
}

fn object_opacity(object: &FabricObject) -> f32 {
    number(object, "opacity")
        .map(|o| o.clamp(0.0, 1.0) as f32)
        .unwrap_or(1.0)
}

/// The shape and paint of one non-group object. `None` means the type has no
/// renderer yet; the caller turns that into an issue.
fn object_ops(
    object: &FabricObject,
    kind: &str,
    document: &ThemeDocument,
    opacity: f32,
    issues: &mut Vec<PlanIssue>,
) -> Option<(Shape, Option<Paint>, Option<Stroke>)> {
    let width = number(object, "width").unwrap_or(0.0) as f32;
    let height = number(object, "height").unwrap_or(0.0) as f32;
    let shape = match kind {
        "Rect" => Shape::Rect {
            width,
            height,
            corner_radius: number(object, "rx")
                .or_else(|| number(object, "ry"))
                .unwrap_or(0.0) as f32,
        },
        "Ellipse" | "Circle" => Shape::Ellipse { width, height },
        // Text, paths, images and charts are added by later tasks; an object
        // whose type is known but not yet converted is reported, not dropped.
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
