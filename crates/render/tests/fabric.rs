//! Schema-3 Fabric scene objects walk into display ops in authored order.
//!
//! Group children live in their parent's centre-origin space, origins name
//! which point `left`/`top` pins, and transforms compose once into parent space.

use yunari_model::document::{ThemeDocument, parse_document};
use yunari_render::{Color, IssueCode, Paint, Shape, fabric};

fn document(objects: &str, extra: &str) -> ThemeDocument {
    parse_document(&format!(
        r##"{{"schemaVersion":"3.0.0","id":"t","artboard":{{"width":400,"height":300}},
            "scene":{{"version":"7.4.0","objects":[{objects}]}}{extra}}}"##
    ))
    .expect("fixture document parses")
}

fn close(actual: (f32, f32), expected: (f32, f32)) {
    assert!(
        (actual.0 - expected.0).abs() < 1e-4 && (actual.1 - expected.1).abs() < 1e-4,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn a_root_rect_and_a_grouped_ellipse_produce_two_ops() {
    let doc = document(
        r##"
        {"type":"Rect","id":"r","left":10,"top":20,"width":30,"height":40,
         "originX":"left","originY":"top","fill":"#ff0000"},
        {"type":"Group","id":"g","left":100,"top":50,"width":40,"height":20,
         "originX":"left","originY":"top","objects":[
            {"type":"Ellipse","id":"e","left":5,"top":5,"width":8,"height":6,
             "originX":"left","originY":"top","fill":"#00ff00"}]}
        "##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 2, "a group itself is not an op");
    assert_eq!(
        list.ops[0].shape,
        Shape::Rect {
            width: 30.0,
            height: 40.0,
            corner_radius: 0.0
        }
    );
    close(list.ops[0].transform.apply(0.0, 0.0), (10.0, 20.0));
    assert_eq!(
        list.ops[1].shape,
        Shape::Ellipse {
            width: 8.0,
            height: 6.0
        }
    );
    // The group's left/top put its centre at (120, 60); the child's own
    // left/top are relative to that centre.
    close(list.ops[1].transform.apply(0.0, 0.0), (125.0, 65.0));
}

#[test]
fn an_unscaled_child_lands_at_the_group_centre_plus_its_own_left_and_top() {
    let doc = document(
        r##"{"type":"Group","id":"g","left":100,"top":50,"width":40,"height":20,
            "originX":"left","originY":"top","objects":[
            {"type":"Rect","id":"c","left":5,"top":5,"width":8,"height":6,
             "originX":"left","originY":"top","fill":"#00ff00"}]}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 1);
    close(list.ops[0].transform.apply(0.0, 0.0), (125.0, 65.0));
}

#[test]
fn a_center_origin_shifts_the_box_by_half_its_scaled_size() {
    let doc = document(
        r##"{"type":"Rect","id":"c","left":100,"top":50,"width":40,"height":20,
            "originX":"center","originY":"center","scaleX":2,"fill":"#00ff00"}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 1);
    // Scaled width is 80; a centre origin pins the box centre at left=100, so
    // its leading edge is at 60. scaleY stays 1, so top shifts by 10.
    close(list.ops[0].transform.apply(0.0, 0.0), (60.0, 40.0));
    close(list.ops[0].transform.apply(40.0, 20.0), (140.0, 60.0));
}

#[test]
fn a_right_and_bottom_origin_shifts_by_the_full_scaled_size() {
    let doc = document(
        r##"{"type":"Rect","id":"c","left":100,"top":50,"width":40,"height":20,
            "originX":"right","originY":"bottom","fill":"#00ff00"}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    close(list.ops[0].transform.apply(0.0, 0.0), (60.0, 30.0));
}

#[test]
fn angle_turns_about_the_box_centre() {
    let doc = document(
        r##"{"type":"Rect","id":"c","left":100,"top":50,"width":40,"height":20,
            "originX":"center","originY":"center","angle":90,"fill":"#00ff00"}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    // The box centre stays at (100, 50); the corner swings to (110, 30).
    close(list.ops[0].transform.apply(0.0, 0.0), (110.0, 30.0));
    close(list.ops[0].transform.apply(20.0, 10.0), (100.0, 50.0));
}

#[test]
fn a_group_scale_and_rotation_reach_its_children() {
    let doc = document(
        r##"{"type":"Group","id":"g","left":100,"top":50,"width":40,"height":20,
            "originX":"center","originY":"center","scaleX":2,"objects":[
            {"type":"Rect","id":"c","left":10,"top":0,"width":4,"height":4,
             "originX":"left","originY":"top","fill":"#00ff00"}]}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 1);
    // The group centre is (100, 50); the child sits 10 units right of it and
    // the group's scaleX doubles that to 20.
    close(list.ops[0].transform.apply(0.0, 0.0), (120.0, 50.0));
}

#[test]
fn fill_and_stroke_resolve_through_globals_and_keep_opacity() {
    let doc = document(
        r##"{"type":"Rect","id":"r","left":0,"top":0,"width":10,"height":10,
            "originX":"left","originY":"top","opacity":0.5,
            "fill":"#123456","stroke":"#abcdef","strokeWidth":2,
            "yunariPaint":{"fill":"palette.accent","stroke":"palette.edge"}}"##,
        r##","globals":{
            "palette":{
                "none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
                "accent":{"name":"Accent","value":{"kind":"solid","color":"#ff0000"}},
                "edge":{"name":"Edge","value":{"kind":"solid","color":"#0000ff"}}
            },
            "typeTokens":{
                "heading":{"family":"Inter"},"body":{"family":"Inter"},"mono":{"family":"Inter"}
            }
        }"##,
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 1);
    assert_eq!(
        list.ops[0].fill,
        Some(Paint::Solid(Color::rgb(255, 0, 0).with_alpha(128)))
    );
    let stroke = list.ops[0].stroke.as_ref().expect("a stroke");
    assert_eq!(stroke.width, 2.0);
    assert_eq!(
        stroke.paint,
        Paint::Solid(Color::rgb(0, 0, 255).with_alpha(128))
    );
}

#[test]
fn an_unknown_object_type_is_reported_with_its_id() {
    let doc = document(r##"{"type":"Widget","id":"mystery","left":0,"top":0}"##, "");
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(list.ops.is_empty());
    assert!(
        issues.iter().any(|issue| {
            issue.code == IssueCode::UnsupportedContent
                && issue.node_id.as_deref() == Some("mystery")
                && issue.detail.contains("Widget")
        }),
        "{issues:?}"
    );
}

#[test]
fn a_nonempty_scene_with_no_drawable_objects_is_an_error() {
    let doc = document(
        r##"{"type":"Group","id":"empty","left":0,"top":0,"width":10,"height":10}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(list.ops.is_empty());
    assert!(
        issues
            .iter()
            .any(|issue| issue.code == IssueCode::UnsupportedContent),
        "a non-empty scene must not silently build nothing: {issues:?}"
    );
}

#[test]
fn walk_visits_roots_first_then_groups_recursively() {
    let doc = document(
        r##"
        {"type":"Rect","id":"r","left":0,"top":0},
        {"type":"Group","id":"g","objects":[
            {"type":"Ellipse","id":"e","left":0,"top":0},
            {"type":"Group","id":"inner","objects":[
                {"type":"Path","id":"p","left":0,"top":0}]}]}
        "##,
        "",
    );
    let ids: Vec<_> = fabric::walk(&doc.scene.objects)
        .iter()
        .map(|object| object.id.as_str())
        .collect();
    assert_eq!(ids, ["r", "g", "e", "inner", "p"]);
}
