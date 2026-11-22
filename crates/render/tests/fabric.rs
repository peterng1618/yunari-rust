//! Schema-3 Fabric scene objects walk into display ops in authored order.
//!
//! Group children live in their parent's centre-origin space, origins name
//! which point `left`/`top` pins, and transforms compose once into parent space.

use std::collections::HashMap;

use yunari_model::document::{ThemeDocument, parse_document};
use yunari_providers::sample::Reading;
use yunari_render::plan::MISSING_VALUE_TEXT;
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

/// The `Clock` group from the active Kraken theme, verbatim. The panel adds a
/// 90° device turn, but the group's own angle is 0; the transform still has to
/// match Fabric's `translate(centre) · rotate · scale` or the clock lands in
/// the wrong half of the frame before the panel turns it.
const KRAKEN_CLOCK_GROUP: &str = r##"{"type":"Group","id":"clock","name":"Clock",
    "left":160,"top":243,"width":354.3895,"height":128,
    "scaleX":0.7399,"scaleY":0.7399,
    "objects":[
      {"type":"Textbox","id":"time","left":-177.1948,"top":-64,
       "width":353.3895,"originX":"left","originY":"top","fontSize":84,
       "fill":"#F3EEE5",
       "yunariText":{"box":{"width":353.3895,"height":100},"align":"center",
         "verticalAlign":"middle","overflow":"ellipsis",
         "runs":[{"kind":"literal","text":"16:10:00","typeToken":"typeTokens.heading",
           "style":{"fontSize":{"value":84}}}]}},
      {"type":"Rect","id":"rule","left":-24.5,"top":27.6009,
       "width":48,"height":1,"originX":"left","originY":"top","fill":"#C9A227"}
    ]}"##;

#[test]
fn the_kraken_clock_group_matches_fabric_absolute_placement() {
    let doc = document(
        KRAKEN_CLOCK_GROUP,
        r##","globals":{
            "palette":{
                "none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
                "text":{"name":"Text","value":{"kind":"solid","color":"#f3eee5"}},
                "brass":{"name":"Brass","value":{"kind":"solid","color":"#c9a227"}}
            },
            "typeTokens":{
                "heading":{"family":"serif"},"body":{"family":"serif"},
                "mono":{"family":"monospace"}
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
    assert_eq!(list.ops.len(), 2);

    // Fabric 7.4's `Group.calcOwnMatrix()` for these fields is
    // [0.7399, 0, 0, 0.7399, 160, 243]. The textbox's own left/top origin is
    // group-local: its top-left maps to (28.89, 195.65), and the authored
    // 353.3895×100 box keeps its scaled size. The clock's ink lands in the
    // panel's middle band; after the device's 90° turn Chromium shows the same
    // band at x≈66..106, y≈60..261.
    let transform = list.ops[0].transform;
    close(transform.apply(0.0, 0.0), (28.89357, 195.6464));
    close(transform.apply(353.3895, 100.0), (290.36646, 269.63638));

    // The brass rule's own left/top are group-local; Fabric's child matrix
    // maps its leading edge to (141.87, 263.42).
    let rule = list.ops[1].transform;
    close(rule.apply(0.0, 0.0), (141.87245, 263.4219));
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

#[test]
fn a_hidden_group_and_its_children_paint_nothing() {
    let doc = document(
        r##"
        {"type":"Rect","id":"shown","left":0,"top":0,"width":10,"height":10,
         "originX":"left","originY":"top","fill":"#ff0000"},
        {"type":"Group","id":"hidden-group","visible":false,"left":0,"top":0,
         "width":20,"height":20,"originX":"left","originY":"top","objects":[
            {"type":"Rect","id":"hidden-child","left":5,"top":5,"width":8,"height":6,
             "originX":"left","originY":"top","fill":"#00ff00"},
            {"type":"Group","id":"hidden-grandchild","visible":true,"objects":[
                {"type":"Ellipse","id":"also-hidden","left":0,"top":0,"width":2,"height":2,
                 "originX":"left","originY":"top","fill":"#0000ff"}]}]}
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
    assert_eq!(list.ops.len(), 1, "only the visible root rect paints");
    assert_eq!(
        list.ops[0].shape,
        Shape::Rect {
            width: 10.0,
            height: 10.0,
            corner_radius: 0.0
        }
    );
}

#[test]
fn a_hidden_leaf_paints_nothing() {
    let doc = document(
        r##"{"type":"Rect","id":"hidden","visible":false,"left":0,"top":0,
             "width":10,"height":10,"originX":"left","originY":"top","fill":"#ff0000"}"##,
        "",
    );
    let (list, _) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(list.ops.is_empty());
}

#[test]
fn non_uniform_scale_then_angle_matches_fabric_order() {
    // Fabric composes scale before rotation: a point sees scale, then rotation,
    // then the centre translation. For a 40x20 box at centre (100,50) with
    // scaleX 2 and angle 90, corner (0,0) lands at (110,10).
    let doc = document(
        r##"{"type":"Rect","id":"r","left":100,"top":50,"width":40,"height":20,
            "originX":"center","originY":"center","scaleX":2,"scaleY":1,"angle":90,
            "fill":"#00ff00"}"##,
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
    close(list.ops[0].transform.apply(0.0, 0.0), (110.0, 10.0));
}

#[test]
fn a_zero_scale_flattens_instead_of_being_rewritten_to_one() {
    let doc = document(
        r##"{"type":"Rect","id":"flat","left":100,"top":50,"width":40,"height":20,
            "originX":"left","originY":"top","scaleX":0,"fill":"#00ff00"}"##,
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
    // scaleX 0 collapses the width axis: every x maps to the centre's x=100,
    // while the y axis is untouched. The op is flat, not silently unscaled.
    close(list.ops[0].transform.apply(0.0, 0.0), (100.0, 50.0));
    close(list.ops[0].transform.apply(40.0, 20.0), (100.0, 70.0));
}

/// A text object with globals and an optional binding owner. The extras carry
/// the palette/typeTokens and the `bindings` map a value run needs.
fn text_document(object: &str, extra: &str) -> ThemeDocument {
    document(object, extra)
}

const GLOBALS: &str = r##","globals":{
        "palette":{
            "none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
            "text":{"name":"Text","value":{"kind":"solid","color":"#ff8800"}}
        },
        "typeTokens":{
            "heading":{"family":"Inter","weight":700},
            "body":{"family":"Inter","weight":400},
            "mono":{"family":"Consolas","weight":400}
        }
    }"##;

fn text_op(doc: &ThemeDocument) -> yunari_render::text::TextLayout {
    let (list, issues) = fabric::build(
        doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op, got {:?}", list.ops);
    };
    layout.clone()
}

#[test]
fn a_textbox_with_a_literal_run_yields_one_text_op_with_authored_style() {
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":10,"top":20,"width":200,"height":40,
            "originX":"left","originY":"top","fontSize":28,"fontFamily":"Inter",
            "yunariPaint":{"fill":"palette.text"},
            "yunariText":{"box":{"width":200,"height":40},"align":"left",
                "runs":[{"kind":"literal","text":"Hello","typeToken":"typeTokens.heading",
                    "style":{"fontSize":{"value":28}}}]}}"##,
        GLOBALS,
    );
    let layout = text_op(&doc);
    assert_eq!(layout.segments.len(), 1, "{:?}", layout.segments);
    let segment = &layout.segments[0];
    assert_eq!(segment.text, "Hello");
    assert_eq!(
        segment.style.family.as_ref().and_then(|f| f.name()),
        Some("Inter")
    );
    assert_eq!(segment.style.weight, 700);
    assert_eq!(segment.style.size, 28.0);
    assert_eq!(
        segment.style.color,
        Color::rgb(255, 136, 0),
        "the palette.text ref resolves through globals"
    );
}

#[test]
fn a_run_colour_ref_resolves_through_globals_palette() {
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":100,"height":30,
            "originX":"left","originY":"top","fontSize":20,"fill":"#000000",
            "yunariText":{"runs":[{"kind":"literal","text":"Hi",
                "typeToken":"typeTokens.body","style":{"fontSize":{"value":20},
                    "color":{"ref":"palette.text"}}}]}}"##,
        GLOBALS,
    );
    let layout = text_op(&doc);
    assert_eq!(
        layout.segments[0].style.color,
        Color::rgb(255, 136, 0),
        "the run's palette ref wins over the object's flattened fill"
    );
}

#[test]
fn a_textbox_fill_from_the_flat_fabric_colour_is_used_when_no_ref_is_authored() {
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":100,"height":30,
            "originX":"left","originY":"top","fontSize":20,"fill":"#123456",
            "yunariText":{"runs":[{"kind":"literal","text":"Hi",
                "typeToken":"typeTokens.body","style":{"fontSize":{"value":20}}}]}}"##,
        GLOBALS,
    );
    let layout = text_op(&doc);
    assert_eq!(layout.segments[0].style.color, Color::rgb(18, 52, 86));
}

#[test]
fn a_flat_face_is_the_fallback_when_the_theme_declares_no_type_tokens() {
    // A legacy theme with no globals still paints its authored face: the
    // object's flattened Fabric fields are the fallback, not a second owner.
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":100,"height":30,
            "originX":"left","originY":"top","text":"42","fontFamily":"Segoe UI, sans-serif",
            "fontWeight":"700","fontSize":24,"fill":"#ffffff"}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op");
    };
    assert_eq!(
        layout.segments[0]
            .style
            .family
            .as_ref()
            .and_then(|f| f.name()),
        Some("Segoe UI")
    );
    assert_eq!(layout.segments[0].style.weight, 700);
}

#[test]
fn a_textbox_resolves_family_and_weight_through_globals_type_tokens() {
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":200,"height":40,
            "originX":"left","originY":"top","fontFamily":"Consolas","fontWeight":"600",
            "fontSize":20,"fill":"#ffffff",
            "yunariText":{"runs":[{"kind":"literal","text":"AB",
                "typeToken":"typeTokens.mono","style":{"fontSize":{"value":20}}}]}}"##,
        GLOBALS,
    );
    let layout = text_op(&doc);
    // mono's token owns the face and weight; the document's globals win over
    // the object's flattened Fabric snapshot of the same fields.
    assert_eq!(
        layout.segments[0]
            .style
            .family
            .as_ref()
            .and_then(|f| f.name()),
        Some("Consolas")
    );
    assert_eq!(layout.segments[0].style.weight, 400);
}

#[test]
fn a_flat_text_with_a_fabric_gradient_fill_paints_its_first_stop() {
    // Fabric stores a gradient `fill` as an object, not a CSS string. The flat
    // fallback must resolve it to a colour rather than silently painting white.
    let doc = text_document(
        r##"{"type":"IText","id":"label","left":0,"top":0,"width":100,"height":30,
            "originX":"left","originY":"top","text":"42","fontSize":20,
            "fill":{"type":"linear","gradientUnits":"pixels","coords":{"x1":0,"y1":0,"x2":100,"y2":0},
                "colorStops":[{"offset":0,"color":"#ff0000"},{"offset":1,"color":"#0000ff"}]}}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op");
    };
    assert_eq!(layout.segments[0].style.color, Color::rgb(255, 0, 0));
}

#[test]
fn a_css_stack_in_a_type_token_selects_its_first_face() {
    // A real theme declares `body.family = "Segoe UI, sans-serif"`. fontdb
    // exact-matches a whole string, so passing it verbatim falls back to the
    // shaper's default; the shared owner takes the first usable face.
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":100,"height":30,
            "originX":"left","originY":"top","fontSize":20,"fill":"#ffffff",
            "yunariText":{"runs":[{"kind":"literal","text":"Hi",
                "typeToken":"typeTokens.body","style":{"fontSize":{"value":20}}}]}}"##,
        r##","globals":{"palette":{"none":{"name":"None","value":{"kind":"solid","color":"transparent"}}},
            "typeTokens":{"heading":{"family":"serif"},"body":{"family":"Segoe UI, sans-serif"},
                "mono":{"family":"monospace"}}}"##,
    );
    let layout = text_op(&doc);
    assert_eq!(
        layout.segments[0]
            .style
            .family
            .as_ref()
            .and_then(|f| f.name()),
        Some("Segoe UI")
    );
}

#[test]
fn a_generic_only_token_family_maps_to_a_cosmic_generic() {
    use yunari_render::text::FontFamily;
    assert_eq!(FontFamily::parse("sans-serif"), Some(FontFamily::SansSerif));
    assert_eq!(FontFamily::parse("monospace"), Some(FontFamily::Monospace));
    assert_eq!(
        FontFamily::parse("'Segoe UI', sans-serif"),
        Some(FontFamily::Named("Segoe UI".into()))
    );
}

#[test]
fn a_textbox_line_height_and_char_spacing_come_from_the_object() {
    // Layout consumes both fields through the one owner: tracking is visible
    // on the style, line height in the gap between wrapped line origins.
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":40,"height":100,
            "originX":"left","originY":"top","fontSize":20,"lineHeight":1.5,
            "charSpacing":500,"fill":"#ffffff",
            "yunariText":{"box":{"width":40,"height":100},
                "runs":[{"kind":"literal","text":"AA BB","typeToken":"typeTokens.body",
                    "style":{"fontSize":{"value":20}}}]}}"##,
        GLOBALS,
    );
    let layout = text_op(&doc);
    assert_eq!(
        layout.segments[0].style.letter_spacing, 10.0,
        "charSpacing 500 == 0.5em == 10px at 20px"
    );
    let tops: std::collections::BTreeSet<i32> = layout
        .segments
        .iter()
        .map(|segment| (segment.origin.1).round() as i32)
        .collect();
    assert!(tops.len() > 1, "tracking should wrap the text: {tops:?}");
    let gap = layout.segments[1].origin.1 - layout.segments[0].origin.1;
    assert!(
        (gap - 30.0).abs() < 1.0,
        "lineHeight 1.5 at 20px means 30px between lines, got {gap}"
    );
}

#[test]
fn object_char_spacing_applies_only_to_the_first_run() {
    // Fabric derives `charSpacing` from the first run's tracking, so it is
    // authored once on the object. Each run resolves its own tracking: a run
    // that authored none must not inherit the first one's spacing.
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":200,"height":40,
            "originX":"left","originY":"top","fontSize":20,"charSpacing":1000,
            "fill":"#ffffff",
            "yunariText":{"runs":[
                {"kind":"literal","text":"A","typeToken":"typeTokens.body",
                 "style":{"fontSize":{"value":20}}},
                {"kind":"literal","text":"B","typeToken":"typeTokens.body",
                 "style":{"fontSize":{"value":20}}}]}}"##,
        GLOBALS,
    );
    let layout = text_op(&doc);
    assert_eq!(layout.segments.len(), 2, "{:?}", layout.segments);
    assert_eq!(
        layout.segments[0].style.letter_spacing, 20.0,
        "charSpacing 1000 == 1em == 20px at 20px"
    );
    assert_eq!(
        layout.segments[1].style.letter_spacing, 0.0,
        "a later run that authored no tracking keeps none"
    );
}

#[test]
fn a_textbox_with_a_missing_reading_shows_the_em_dash_and_an_issue() {
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":200,"height":40,
            "originX":"left","originY":"top","fontSize":20,
            "yunariText":{"runs":[{"kind":"value","bindingId":"b",
                "typeToken":"typeTokens.body","style":{"fontSize":{"value":20}}}]}}"##,
        &format!(
            "{}{}",
            GLOBALS, r#","bindings":{"label":[{"id":"b","semanticKey":"cpu.load"}]}"#
        ),
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op");
    };
    assert_eq!(layout.segments[0].text, MISSING_VALUE_TEXT, "never a zero");
    assert!(
        issues.iter().any(|issue| {
            issue.code == IssueCode::MissingReading && issue.detail.contains("cpu.load")
        }),
        "{issues:?}"
    );
}

#[test]
fn a_text_object_with_no_authored_payload_uses_its_flat_text() {
    // Older themes carry only Fabric's flattened `text` and face; like the
    // source lifter, the plan turns that into one body-token literal run.
    let doc = text_document(
        r##"{"type":"IText","id":"label","left":0,"top":0,"width":100,"height":30,
            "originX":"left","originY":"top","text":"123","fontFamily":"Inter",
            "fontSize":22,"fill":"#ffffff"}"##,
        GLOBALS,
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op");
    };
    assert_eq!(layout.segments[0].text, "123");
    assert_eq!(
        layout.segments[0]
            .style
            .family
            .as_ref()
            .and_then(|f| f.name()),
        Some("Inter")
    );
    assert_eq!(layout.segments[0].style.size, 22.0);
}

#[test]
fn an_itext_non_ok_reading_shows_the_em_dash_with_its_reason() {
    let doc = text_document(
        r##"{"type":"IText","id":"label","left":0,"top":0,"width":200,"height":40,
            "originX":"left","originY":"top","fontSize":20,
            "yunariText":{"runs":[{"kind":"value","bindingId":"b",
                "typeToken":"typeTokens.body","style":{"fontSize":{"value":20}}}]}}"##,
        &format!(
            "{}{}",
            GLOBALS, r#","bindings":{"label":[{"id":"b","semanticKey":"cpu.load"}]}"#
        ),
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::new([Reading::missing("cpu.load", "the sensor is asleep")]),
        &yunari_providers::units::UnitPreferences::default(),
    );
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op");
    };
    assert_eq!(layout.segments[0].text, MISSING_VALUE_TEXT);
    assert!(
        issues.iter().any(|issue| {
            issue.code == IssueCode::MissingReading && issue.detail.contains("the sensor is asleep")
        }),
        "{issues:?}"
    );
}

#[test]
fn a_textbox_value_run_formats_through_the_binding_rules() {
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":0,"top":0,"width":200,"height":40,
            "originX":"left","originY":"top","fontSize":20,
            "yunariText":{"runs":[{"kind":"value","bindingId":"b",
                "typeToken":"typeTokens.body","style":{"fontSize":{"value":20}}}]}}"##,
        &format!(
            "{}{}",
            GLOBALS,
            r#","bindings":{"label":[{"id":"b","semanticKey":"custom.probe.value","scale":2,"precision":2}]}"#
        ),
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::new([Reading::number("custom.probe.value", 1.5, "MB")]),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op");
    };
    assert_eq!(layout.segments[0].text, "3.00 MB");
}

#[test]
fn a_textbox_without_fabric_height_uses_its_authored_box_for_geometry() {
    // Fabric's Textbox omits `height`: its box lives in `yunariText.box`.
    // A group-local clock uses negative left/top with left/top origins; sizing
    // from the missing height would put its centre above the box and paint
    // nowhere near the authored coordinates.
    let doc = text_document(
        r##"{"type":"Group","id":"clock","left":160,"top":243,"width":354.3895,"height":128,
            "originX":"center","originY":"center","scaleX":0.7399,"scaleY":0.7399,
            "objects":[{"type":"Textbox","id":"time","left":-177.1948,"top":-64,
                "width":353.3895,"originX":"left","originY":"top","fontSize":84,
                "yunariText":{"box":{"width":353.3895,"height":100},"align":"center",
                    "verticalAlign":"middle","overflow":"ellipsis",
                    "runs":[{"kind":"literal","text":"12:34","typeToken":"typeTokens.heading",
                        "style":{"fontSize":{"value":84}}}]}}]}"##,
        GLOBALS,
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Text(layout)) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Text op, got {:?}", list.ops);
    };
    assert!(!layout.segments.is_empty(), "text must have segments");
    for segment in &layout.segments {
        assert!(
            segment.origin.0 < 353.3895 && segment.origin.1 < 100.0,
            "segment origin {:?} must lie inside the authored box",
            segment.origin
        );
    }
    // The authored 353.3895x100 box must map as a box, not collapse to a
    // zero-height line: its lower-right corner stays separated from its origin.
    let top_left = list.ops[0].transform.apply(0.0, 0.0);
    let bottom_right = list.ops[0].transform.apply(353.3895, 100.0);
    assert!(
        (bottom_right.0 - top_left.0 - 353.3895 * 0.7399).abs() < 1e-3
            && (bottom_right.1 - top_left.1 - 100.0 * 0.7399).abs() < 1e-3,
        "the authored box must survive the group transform: {top_left:?} -> {bottom_right:?}"
    );
}

#[test]
fn a_centre_origin_textbox_without_height_centres_on_its_authored_box() {
    // Fabric's default origin is the box centre. With no flattened height the
    // old geometry put the centre at `top`, shifting the whole paragraph up by
    // half the authored box height.
    let doc = text_document(
        r##"{"type":"Textbox","id":"label","left":100,"top":50,"width":200,
            "fontSize":20,"fill":"#ffffff",
            "yunariText":{"box":{"width":200,"height":100},"align":"center",
                "runs":[{"kind":"literal","text":"Hi","typeToken":"typeTokens.body",
                    "style":{"fontSize":{"value":20}}}]}}"##,
        GLOBALS,
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 1);
    // A centre origin places the box's top-left at left-width/2, top-height/2.
    close(list.ops[0].transform.apply(0.0, 0.0), (0.0, 0.0));
    close(list.ops[0].transform.apply(200.0, 100.0), (200.0, 100.0));
}

/// A schema-3 document with assets, used by the image/path/glass tests below.
fn document_with_assets(objects: &str, extra: &str) -> ThemeDocument {
    document(
        objects,
        &format!(
            r##", "assets":[{{"id":"logo","kind":"image","path":"assets/logo.png"}}]{extra}"##
        ),
    )
}

#[test]
fn a_path_object_yields_parsed_commands_and_keeps_its_paint() {
    // Real themes store a Fabric Path as [["M",x,y],["L",x,y],...]; the same
    // path reader the major-1 planner uses turns that into drawable commands.
    let doc = document_with_assets(
        r##"{"type":"Path","id":"rule","left":10,"top":20,"width":100,"height":10,
            "originX":"left","originY":"top","strokeWidth":2,
            "path":[["M",0,0],["L",100,0],["L",100,10],["Z"]],
            "fill":"#ff0000","stroke":"#0000ff",
            "yunariPaint":{"fill":"palette.accent"}}"##,
        r##","globals":{
            "palette":{
                "none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
                "accent":{"name":"Accent","value":{"kind":"solid","color":"#00ff00"}}
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
    let Some(Shape::Path { commands, closed }) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Path op, got {:?}", list.ops);
    };
    assert_eq!(
        commands,
        &vec![
            yunari_render::PathCommand::MoveTo(0.0, 0.0),
            yunari_render::PathCommand::LineTo(100.0, 0.0),
            yunari_render::PathCommand::LineTo(100.0, 10.0),
            yunari_render::PathCommand::Close,
        ]
    );
    assert!(*closed, "a trailing Z makes the path closed");
    assert_eq!(
        list.ops[0].fill,
        Some(Paint::Solid(Color::rgb(0, 255, 0))),
        "the authored fill ref wins through the globals owner"
    );
    assert_eq!(list.ops[0].stroke.as_ref().map(|stroke| stroke.width), Some(2.0));
}

#[test]
fn unreadable_path_data_reports_an_issue_and_paints_nothing() {
    let doc = document_with_assets(
        r##"{"type":"Path","id":"broken","left":0,"top":0,"width":10,"height":10,
            "originX":"left","originY":"top","path":[["M",0,0],["L"]],"stroke":"#ff0000"}"##,
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
        issues.iter().any(|issue| {
            issue.code == IssueCode::UnsupportedContent
                && issue.node_id.as_deref() == Some("broken")
        }),
        "{issues:?}"
    );
}

#[test]
fn an_image_object_carries_its_asset_path_and_box() {
    let doc = document_with_assets(
        r##"{"type":"Image","id":"pic","left":10,"top":20,"width":120,"height":80,
            "originX":"left","originY":"top",
            "yunariAsset":{"assetId":"logo","kind":"image"},
            "yunariImage":{"fit":"cover"}}"##,
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
    let Some(Shape::Image {
        asset_id,
        path,
        width,
        height,
        fit,
        ..
    }) = list.ops.first().map(|op| &op.shape)
    else {
        panic!("expected a Shape::Image op, got {:?}", list.ops);
    };
    assert_eq!(asset_id, "logo");
    assert_eq!(path, "assets/logo.png");
    assert_eq!((*width, *height), (120.0, 80.0));
    assert_eq!(*fit, yunari_model::document::ImageFit::Cover);
    close(list.ops[0].transform.apply(0.0, 0.0), (10.0, 20.0));
    assert!(!list.has_video());
}

#[test]
fn an_image_with_an_undeclared_asset_reports_an_issue() {
    let doc = document_with_assets(
        r##"{"type":"Image","id":"pic","left":0,"top":0,"width":10,"height":10,
            "originX":"left","originY":"top",
            "yunariAsset":{"assetId":"missing","kind":"image"}}"##,
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
        issues.iter().any(|issue| {
            issue.code == IssueCode::UnresolvedGlobal
                && issue.node_id.as_deref() == Some("pic")
        }),
        "{issues:?}"
    );
}

#[test]
fn glass_wraps_the_base_shape_and_leaves_it_visible() {
    let doc = document(
        r##"{"type":"Rect","id":"panel","left":10,"top":20,"width":40,"height":30,
            "originX":"left","originY":"top","fill":"#ffffff",
            "yunariGlass":{"blurRadius":12}}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 2, "a blur op before the wrapped base shape");
    let Some(Shape::Glass { shape, blur_radius }) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected a Shape::Glass op, got {:?}", list.ops);
    };
    assert_eq!(*blur_radius, 12.0);
    assert_eq!(
        **shape,
        Shape::Rect {
            width: 40.0,
            height: 30.0,
            corner_radius: 0.0
        },
        "glass wraps, never replaces, the authored shape"
    );
    assert_eq!(
        list.ops[1].shape,
        Shape::Rect {
            width: 40.0,
            height: 30.0,
            corner_radius: 0.0
        },
        "the base shape still paints"
    );
    assert!(list.ops[1].fill.is_some());
    assert_eq!(list.ops[0].transform, list.ops[1].transform);
    close(list.ops[0].transform.apply(0.0, 0.0), (10.0, 20.0));
}

#[test]
fn zero_glass_blur_does_not_add_a_blur_op() {
    let doc = document(
        r##"{"type":"Rect","id":"panel","left":0,"top":0,"width":10,"height":10,
            "originX":"left","originY":"top","fill":"#ffffff",
            "yunariGlass":{"blurRadius":0}}"##,
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
    assert!(!matches!(list.ops[0].shape, Shape::Glass { .. }));
}

#[test]
fn unsupported_glass_reports_an_issue_and_leaves_the_base_visible() {
    // An open path has no interior to sample through; the source renderer
    // refuses glass on it. Refuse the blur loudly but still paint the base path.
    let doc = document(
        r##"{"type":"Path","id":"sketch","left":0,"top":0,"width":100,"height":10,
            "originX":"left","originY":"top","stroke":"#ff0000","strokeWidth":2,
            "path":[["M",0,0],["L",100,0]],
            "yunariGlass":{"blurRadius":12}}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert_eq!(list.ops.len(), 1, "the base path still paints");
    assert!(matches!(list.ops[0].shape, Shape::Path { .. }));
    assert!(
        issues.iter().any(|issue| {
            issue.code == IssueCode::UnsupportedContent
                && issue.node_id.as_deref() == Some("sketch")
                && issue.detail.to_lowercase().contains("glass")
        }),
        "{issues:?}"
    );
}

#[test]
fn glass_on_a_rect_whose_paint_cannot_be_resolved_still_blurs_the_base() {
    // An unresolved paint is its own issue; it must not silently drop the glass
    // op. The base shape and its blur both survive.
    let doc = document(
        r##"{"type":"Rect","id":"panel","left":0,"top":0,"width":10,"height":10,
            "originX":"left","originY":"top","fill":"#ffffff",
            "yunariPaint":{"fill":"palette.nope"},
            "yunariGlass":{"blurRadius":6}}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert_eq!(list.ops.len(), 2, "blur and base shape both present");
    assert!(matches!(list.ops[0].shape, Shape::Glass { .. }));
    assert!(
        issues.iter().any(|issue| issue.code == IssueCode::UnresolvedGlobal),
        "{issues:?}"
    );
}

#[test]
fn glass_on_a_group_reports_an_issue_and_children_still_paint() {
    // Model validation admits glass on a Group, but Fabric gives a group no
    // render boundary to sample through. Refuse the blur loudly; children paint.
    let doc = document(
        r##"{"type":"Group","id":"frame","left":0,"top":0,"width":50,"height":50,
            "originX":"left","originY":"top","yunariGlass":{"blurRadius":8},
            "objects":[
                {"type":"Rect","id":"child","left":5,"top":5,"width":10,"height":10,
                 "originX":"left","originY":"top","fill":"#ff0000"}]}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert_eq!(list.ops.len(), 1, "the group's child still paints");
    assert!(matches!(list.ops[0].shape, Shape::Rect { .. }));
    assert!(
        issues.iter().any(|issue| {
            issue.code == IssueCode::UnsupportedContent
                && issue.node_id.as_deref() == Some("frame")
                && issue.detail.to_lowercase().contains("glass")
        }),
        "{issues:?}"
    );
}

#[test]
fn an_image_monochrome_mask_keeps_its_resolved_alpha() {
    // A palette colour of rgba(255,0,0,0.5) at object opacity 1 must stay at
    // alpha 128. Overwriting with opacity's byte loses the authored alpha.
    let doc = document_with_assets(
        r##"{"type":"Image","id":"icon","left":0,"top":0,"width":10,"height":10,
            "originX":"left","originY":"top","opacity":1,
            "yunariAsset":{"assetId":"logo","kind":"image"},
            "yunariPaint":{"monochrome":"palette.ink"}}"##,
        r##","globals":{"palette":{
            "none":{"name":"None","value":{"kind":"solid","color":"transparent"}},
            "ink":{"name":"Ink","value":{"kind":"solid","color":"rgba(255,0,0,0.5)"}}
        },"typeTokens":{
            "heading":{"family":"Inter"},"body":{"family":"Inter"},"mono":{"family":"Inter"}
        }}"##,
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Some(Shape::Image { monochrome, .. }) = list.ops.first().map(|op| &op.shape) else {
        panic!("expected an image op");
    };
    assert_eq!(*monochrome, Some(Color::rgb(255, 0, 0).with_alpha(128)));
}

#[test]
fn a_path_closed_before_the_last_subpath_still_supports_glass() {
    // Real themes end with a final open contour after earlier closed ones;
    // "closed" means every subpath closes, not just the final command.
    let doc = document(
        r##"{"type":"Path","id":"rings","left":0,"top":0,"width":20,"height":20,
            "originX":"left","originY":"top","stroke":"#ff0000",
            "path":[["M",0,0],["L",10,0],["Z"],["M",5,5],["L",15,5],["Z"]],
            "yunariGlass":{"blurRadius":8}}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(list.ops.len(), 2, "glass wraps the closed multi-subpath");
    assert!(matches!(list.ops[0].shape, Shape::Glass { .. }));
    assert!(matches!(list.ops[1].shape, Shape::Path { .. }));
}

#[test]
fn a_path_with_one_open_subpath_refuses_glass_even_when_it_ends_with_z() {
    let doc = document(
        r##"{"type":"Path","id":"mixed","left":0,"top":0,"width":20,"height":20,
            "originX":"left","originY":"top","stroke":"#ff0000",
            "path":[["M",0,0],["L",10,0],["M",5,5],["L",15,5],["Z"]],
            "yunariGlass":{"blurRadius":8}}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert_eq!(list.ops.len(), 1, "the base path still paints");
    assert!(matches!(list.ops[0].shape, Shape::Path { .. }));
    assert!(
        issues.iter().any(|issue| {
            issue.code == IssueCode::UnsupportedContent
                && issue.node_id.as_deref() == Some("mixed")
                && issue.detail.to_lowercase().contains("glass")
        }),
        "{issues:?}"
    );
}

#[test]
fn a_fabric_chart_carries_family_id_box_bindings_and_opacity() {
    let doc = document(
        r##"{"type":"YunariChart","id":"chart-7","family":"polarBar",
            "left":40,"top":60,"width":120,"height":80,
            "originX":"left","originY":"top","opacity":0.5,
            "settings":{"startAngle":90}}"##,
        r##","bindings":{"chart-7":[{"id":"a","semanticKey":"cpu.load"}]}"##,
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let Shape::Chart {
        id,
        family,
        settings,
        bindings,
        width,
        height,
        opacity,
    } = &list.ops[0].shape else {
        panic!("expected a chart op");
    };
    assert_eq!(id, "chart-7");
    assert_eq!(*family, yunari_render::ChartFamily::PolarBar);
    assert_eq!(settings["startAngle"], 90);
    assert_eq!(bindings, &["cpu.load".to_owned()]);
    assert_eq!((*width, *height), (120.0, 80.0));
    assert_eq!(*opacity, 0.5);
    close(list.ops[0].transform.apply(0.0, 0.0), (40.0, 60.0));
}

#[test]
fn an_unknown_fabric_chart_family_reports_an_issue() {
    let doc = document(
        r##"{"type":"YunariChart","id":"chart-x","family":"bubble",
            "left":0,"top":0,"width":20,"height":20}"##,
        "",
    );
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(list.ops.is_empty(), "no guessed chart is drawn");
    assert!(issues.iter().any(|issue| {
        issue.code == IssueCode::UnsupportedContent
            && issue.node_id.as_deref() == Some("chart-x")
    }), "{issues:?}");
}

/// An advanced media declares where the picture goes, but its crop waits for
/// the bytes. A caller that has measured them supplies the source size here.
#[test]
fn an_advanced_backdrop_resolves_media_from_its_source_size() {
    let doc = parse_document(
        r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":40,"height":20},
            "background":{"mode":"advanced","media":{"assetId":"pic","bounds":"display",
              "color":{"value":"#ff0000"},
              "composition":{"anchor":"top","extent":0.5,"fade":0.1}}},
            "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
            "scene":{"version":"7.4.0","objects":[]}}"##,
    )
    .expect("fixture document parses");
    let sizes = HashMap::from([("assets/pic.png".to_owned(), (16.0, 9.0))]);
    let (list, issues) = fabric::build_with_sources(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
        &sizes,
    );
    assert!(issues.is_empty(), "{issues:?}");
    let backdrop = list.backdrop.expect("a declared background has a picture");
    let resolved = &backdrop.resolved;
    let media = resolved.media.expect("the media is placed");
    assert!(media.cover.width > 0.0 && media.cover.height > 0.0);
    assert!(!resolved.masks.is_empty(), "a fade has a ramp");
}

#[test]
fn a_backdrop_without_a_measured_source_has_no_media() {
    let doc = parse_document(
        r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":40,"height":20},
            "background":{"mode":"advanced","media":{"assetId":"pic","bounds":"display",
              "color":{"value":"#ff0000"},
              "composition":{"anchor":"top","extent":0.5,"fade":0.1}}},
            "assets":[{"id":"pic","kind":"image","path":"assets/pic.png"}],
            "scene":{"version":"7.4.0","objects":[]}}"##,
    )
    .expect("fixture document parses");
    let (list, issues) = fabric::build(
        &doc,
        &mut yunari_render::Fonts::new(),
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    );
    assert!(issues.is_empty(), "{issues:?}");
    let backdrop = list.backdrop.expect("a declared background has a picture");
    let resolved = &backdrop.resolved;
    assert!(resolved.media.is_none(), "no size means no placed picture");
    assert!(
        !resolved.masks.is_empty(),
        "the area and its ramps are still known"
    );
}
