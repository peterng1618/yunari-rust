//! Schema-3 migration: version gate, top-level fields, and typed nested
//! `globals`/`scene` shapes. These tests pin that real installed themes parse
//! with their palette, type tokens and Fabric scene retained as typed data.

use yunari_model::document::{
    DocumentError, SCHEMA_VERSION, SUPPORTED_SCHEMA_MAJOR, TypeWeight, parse_document,
    write_document,
};

/// A minimal document carrying every field the schema marks as required.
/// `globals` and `scene` are minimal but real so the placeholders cannot pass
/// by dropping them.
const MINIMAL_SCHEMA3: &str = r#"{
    "schemaVersion": "3.0.0",
    "fabricVersion": "7.4.0",
    "id": "theme_1",
    "artboard": { "width": 800, "height": 480 },
    "globals": { "palette": {}, "typeTokens": {} },
    "scene": { "version": "7.4.0", "objects": [] }
}"#;

#[test]
fn parses_a_minimal_schema3_document() {
    let doc = parse_document(MINIMAL_SCHEMA3).expect("minimal schema-3 document parses");
    assert_eq!(SCHEMA_VERSION, "3.0.0");
    assert_eq!(SUPPORTED_SCHEMA_MAJOR, 3);
    assert_eq!(doc.schema_version, "3.0.0");
    assert_eq!(doc.fabric_version, "7.4.0");
}

#[test]
fn round_trips_required_top_level_fields() {
    let doc = parse_document(MINIMAL_SCHEMA3).expect("parses");
    let json = write_document(&doc).expect("writes");
    let reparsed = parse_document(&json).expect("reparses");
    assert_eq!(reparsed, doc);
    // The nested bags survive a write as data, even before Task 2 types them.
    let raw: serde_json::Value = serde_json::from_str(&json).expect("written JSON");
    assert_eq!(raw["fabricVersion"], "7.4.0");
    assert_eq!(
        raw["globals"],
        serde_json::json!({"palette": {}, "typeTokens": {}})
    );
    assert_eq!(
        raw["scene"],
        serde_json::json!({"version": "7.4.0", "objects": []})
    );
}

#[test]
fn refuses_schema_major_one_without_migrating() {
    let json = MINIMAL_SCHEMA3.replace("3.0.0", "1.0.0");
    let error = parse_document(&json).expect_err("major 1 is refused");
    let DocumentError::Invalid(issues) = error else {
        panic!("expected a version validation error, got {error:?}");
    };
    assert!(issues.iter().any(|issue| issue.path == "schemaVersion"));
}

/// A reduced copy of the legacy global bag carried by two installed 2.0.0
/// themes: a `name` and a `value` with face, size, weight, leading, optional
/// tracking and a trio role. It is read as data, never converted to a
/// schema-3 `typeTokens` token.
const LEGACY_V2_PRESETS: &str = r##"{
    "schemaVersion": "2.0.0",
    "fabricVersion": "7.4.0",
    "id": "legacy_v2",
    "metadata": { "name": "Legacy v2", "themeLanguage": "en" },
    "artboard": { "width": 1920, "height": 480 },
    "background": { "color": { "ref": "palette.none" } },
    "globals": {
        "palette": {
            "none": { "name": "None", "value": { "kind": "solid", "color": "transparent" } }
        },
        "typePresets": {
            "20-400": { "name": "Caption", "value": {
                "family": "Segoe UI, sans-serif", "size": 20, "weight": "400",
                "lineHeight": 1.18, "trioRole": "body" } },
            "17-400": { "name": "Overline", "value": {
                "family": "Segoe UI, sans-serif", "size": 17, "weight": "400",
                "letterSpacing": 6, "lineHeight": 1.18, "trioRole": "body" } }
        }
    },
    "scene": { "version": "7.4.0", "objects": [] }
}"##;

#[test]
fn reads_installed_legacy_type_presets_as_typed_data() {
    let doc = parse_document(LEGACY_V2_PRESETS).expect("the installed legacy bag parses");
    assert_eq!(doc.schema_version, "2.0.0");
    assert_eq!(doc.globals.type_presets.len(), 2);
    let caption = &doc.globals.type_presets["20-400"];
    assert_eq!(caption.name, "Caption");
    assert_eq!(caption.value.family, "Segoe UI, sans-serif");
    assert_eq!(caption.value.size, 20.0);
    assert_eq!(caption.value.weight, Some(TypeWeight::Name("400".to_owned())));
    assert_eq!(caption.value.line_height, 1.18);
    assert_eq!(caption.value.letter_spacing, None);
    assert_eq!(caption.value.trio_role, "body");
    assert_eq!(doc.globals.type_presets["17-400"].value.letter_spacing, Some(6.0));
}

#[test]
fn reads_a_legacy_text_run_preset_reference() {
    let run: serde_json::Value = serde_json::from_str(
        r#"{"kind":"literal","text":"YUNARI","typePreset":"typePresets.36-500","style":{}}"#,
    )
    .expect("legacy run JSON");
    let content: yunari_model::document::TextContent = serde_json::from_value(serde_json::json!({
        "box": { "width": 460, "height": 40.68 },
        "runs": [run]
    }))
    .expect("the installed legacy run shape deserializes");
    let yunari_model::document::TextRun::Literal { type_token, .. } = &content.runs[0] else {
        panic!("expected a literal run");
    };
    assert_eq!(type_token, "typePresets.36-500");
}

#[test]
fn refuses_a_malformed_legacy_type_preset() {
    let json = LEGACY_V2_PRESETS.replace(
        "\"family\": \"Segoe UI, sans-serif\", \"size\": 20",
        "\"family\": 20, \"size\": 20",
    );
    let error = parse_document(&json).expect_err("a numeric family is malformed");
    assert!(
        error.to_string().contains("expected a string"),
        "the typed bag refused the wrong family shape: {error}"
    );
}

#[test]
fn refuses_legacy_type_presets_in_schema3() {
    let json = LEGACY_V2_PRESETS.replace("\"2.0.0\"", "\"3.0.0\"");
    let error = parse_document(&json).expect_err("schema 3 does not declare typePresets");
    assert!(error.to_string().contains("typePresets"), "{error}");
}

#[test]
fn refuses_schema_major_four_without_coercion() {
    let json = MINIMAL_SCHEMA3.replace("3.0.0", "4.0.0");
    let error = parse_document(&json).expect_err("major 4 is refused");
    let DocumentError::Invalid(issues) = error else {
        panic!("expected a version validation error, got {error:?}");
    };
    assert!(issues.iter().any(|issue| issue.path == "schemaVersion"));
}

/// A real 3.0.0 document with populated globals and a populated Fabric scene.
/// Kept inline so the typed-shape assertions run offline; the installed-theme
/// test below runs only when `YUNARI_TEST_THEME` points at a real file.
const POPULATED_SCHEMA3: &str = r##"{
    "schemaVersion": "3.0.0",
    "fabricVersion": "7.4.0",
    "id": "populated",
    "artboard": { "width": 800, "height": 480 },
    "globals": {
        "palette": {
            "none": { "name": "None", "value": { "kind": "solid", "color": "transparent" } },
            "text": { "name": "Text", "value": { "kind": "solid", "color": "#ecf5ff" } }
        },
        "typeTokens": {
            "heading": { "family": "Segoe UI", "weight": 600 },
            "body": { "family": "Segoe UI", "weight": 400 },
            "mono": { "family": "Consolas", "weight": 400 }
        }
    },
    "scene": {
        "version": "7.4.0",
        "objects": [
            { "type": "Rect", "id": "rule", "left": 0, "top": 0, "width": 10, "height": 10 },
            { "type": "Group", "id": "group", "objects": [
                { "type": "Circle", "id": "dot", "left": 2, "top": 2 }
            ] }
        ]
    }
}"##;

#[test]
fn parses_typed_globals_and_scene() {
    let doc = parse_document(POPULATED_SCHEMA3).expect("populated schema-3 document parses");
    // Typed access must reach into globals, not a raw serde_json::Value bag.
    assert_eq!(doc.globals.palette["none"].name, "None");
    assert_eq!(doc.globals.type_tokens["body"].family, "Segoe UI");
    assert_eq!(doc.scene.version.as_deref(), Some("7.4.0"));
    assert_eq!(doc.scene.objects.len(), 2);
}

#[test]
fn fabric_objects_stay_scene_data_not_nodes() {
    let doc = parse_document(POPULATED_SCHEMA3).expect("parses");
    assert!(
        doc.nodes.is_empty(),
        "Fabric objects stay in the scene, never folded into major-1 nodes"
    );
    let rule = &doc.scene.objects[0];
    assert_eq!(rule.id, "rule");
    assert_eq!(rule.object_type.as_deref(), Some("Rect"));
    assert_eq!(rule.data["width"], 10);
    let group = &doc.scene.objects[1];
    assert_eq!(group.children.len(), 1, "nesting is retained as scene data");
    assert_eq!(group.children[0].id, "dot");
}

#[test]
fn refused_nested_paint_and_malformed_scene_fields() {
    let bad_paint = POPULATED_SCHEMA3.replace("\"color\": \"#ecf5ff\"", "\"colour\": \"#ecf5ff\"");
    parse_document(&bad_paint).expect_err("a paint with no colour field is refused");

    let bad_version = POPULATED_SCHEMA3.replace("\"version\": \"7.4.0\",", "\"version\": \"7.4\",");
    let DocumentError::Invalid(issues) =
        parse_document(&bad_version).expect_err("scene.version must be major.minor.patch")
    else {
        panic!("expected validation issues");
    };
    assert!(issues.iter().any(|issue| issue.path == "scene/version"));
}

#[test]
fn empty_defaults_serialize_as_objects_not_null() {
    let doc = parse_document(MINIMAL_SCHEMA3).expect("parses");
    let json = write_document(&doc).expect("writes");
    let raw: serde_json::Value = serde_json::from_str(&json).expect("written JSON");
    assert!(raw["globals"].is_object(), "globals is never null");
    assert!(raw["scene"].is_object(), "scene is never null");
    assert!(raw["scene"]["objects"].is_array(), "objects is never null");
}

#[test]
fn written_schema3_document_has_no_legacy_top_level_fields() {
    let doc = parse_document(POPULATED_SCHEMA3).expect("parses");
    let json = write_document(&doc).expect("writes");
    let raw: serde_json::Value = serde_json::from_str(&json).expect("written JSON");
    for legacy in ["palette", "typeTokens", "nodes"] {
        assert!(
            raw.get(legacy).is_none(),
            "schema-3 output must not contain legacy top-level field {legacy}"
        );
    }
    // The typed nested bags survive the write.
    assert_eq!(raw["globals"]["palette"]["none"]["name"], "None");
    assert_eq!(raw["scene"]["objects"][0]["id"], "rule");
}

/// Reads a real installed theme from the environment. Absent means offline:
/// the test skips with a clear message, so the suite stays self-contained.
#[test]
fn parses_a_real_installed_theme() {
    let Ok(path) = std::env::var("YUNARI_TEST_THEME") else {
        eprintln!("SKIP: YUNARI_TEST_THEME is not set; no real theme to parse");
        return;
    };
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {path}: {error}"));
    let doc = parse_document(&json)
        .unwrap_or_else(|error| panic!("parse_document rejected {path}: {error}"));
    assert_eq!(doc.schema_version, "3.0.0");
    assert!(!doc.fabric_version.is_empty(), "fabricVersion is preserved");
    assert!(
        !doc.globals.palette.is_empty(),
        "globals.palette is retained as typed data"
    );
    assert!(
        !doc.globals.type_tokens.is_empty(),
        "globals.typeTokens is retained as typed data"
    );
    assert!(
        !doc.scene.objects.is_empty(),
        "scene.objects is retained as typed scene data"
    );
    assert!(
        doc.nodes.is_empty(),
        "real Fabric objects are not folded into major-1 nodes"
    );
}

/// Validates written output against the authoritative schema. Both paths come
/// from the environment so the suite stays offline and this crate never keeps
/// a second copy of the schema.
#[test]
fn written_real_theme_satisfies_the_authoritative_schema() {
    let (Ok(theme_path), Ok(schema_path)) = (
        std::env::var("YUNARI_TEST_THEME"),
        std::env::var("YUNARI_TEST_SCHEMA"),
    ) else {
        eprintln!(
            "SKIP: YUNARI_TEST_THEME and YUNARI_TEST_SCHEMA are both required for schema validation"
        );
        return;
    };
    let json = std::fs::read_to_string(&theme_path)
        .unwrap_or_else(|error| panic!("could not read {theme_path}: {error}"));
    let schema = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|error| panic!("could not read {schema_path}: {error}"));
    let document = parse_document(&json).expect("real theme parses");
    let written = write_document(&document).expect("real theme writes");
    let instance: serde_json::Value = serde_json::from_str(&written).expect("written JSON");
    let schema: serde_json::Value = serde_json::from_str(&schema).expect("schema JSON");
    let validator = jsonschema::validator_for(&schema).expect("valid schema");
    let errors: Vec<String> = validator
        .iter_errors(&instance)
        .map(|error| format!("{}: {}", error.instance_path, error))
        .collect();
    assert!(
        errors.is_empty(),
        "written theme violates the authoritative schema ({} errors):\n{}",
        errors.len(),
        errors.join("\n")
    );
}

#[test]
fn refuses_a_theme_missing_the_transparent_token_or_a_role_token() {
    let no_none = POPULATED_SCHEMA3.replace(
        "\"none\": { \"name\": \"None\", \"value\": { \"kind\": \"solid\", \"color\": \"transparent\" } },",
        "",
    );
    let DocumentError::Invalid(issues) =
        parse_document(&no_none).expect_err("the none token is required")
    else {
        panic!("expected validation issues");
    };
    assert!(issues.iter().any(|i| i.path == "globals/palette/none"));

    let no_mono = POPULATED_SCHEMA3.replace(
        ",\n            \"mono\": { \"family\": \"Consolas\", \"weight\": 400 }",
        "",
    );
    let error = parse_document(&no_mono).expect_err("all three role tokens are required");
    assert!(error.to_string().contains("globals/typeTokens/mono"));
}

#[test]
fn refuses_a_scene_without_an_objects_list() {
    let no_objects = POPULATED_SCHEMA3.replace(
        "\"objects\": [\n            { \"type\": \"Rect\", \"id\": \"rule\", \"left\": 0, \"top\": 0, \"width\": 10, \"height\": 10 },\n            { \"type\": \"Group\", \"id\": \"group\", \"objects\": [\n                { \"type\": \"Circle\", \"id\": \"dot\", \"left\": 2, \"top\": 2 }\n            ] }\n        ]",
        "\"objects\": []",
    );
    // An empty objects list is valid; a missing key is not.
    parse_document(&no_objects).expect("empty objects list parses");
    let missing = POPULATED_SCHEMA3.replace("\"objects\": [", "\"unused\": [");
    let DocumentError::Invalid(issues) =
        parse_document(&missing).expect_err("a scene must declare objects")
    else {
        panic!("expected validation issues");
    };
    assert!(issues.iter().any(|i| i.path == "scene/objects"));
}

#[test]
fn refuses_a_fabric_object_name_over_120_characters() {
    let long_name = "x".repeat(121);
    let json = POPULATED_SCHEMA3.replace(
        "\"type\": \"Rect\", \"id\": \"rule\"",
        &format!("\"type\": \"Rect\", \"id\": \"rule\", \"name\": \"{long_name}\""),
    );
    parse_document(&json).expect_err("a Fabric object name is at most 120 characters");
}
