//! Task 1 of the schema-3 migration: the version gate and the top-level fields
//! the schema marks as required. Nested `globals`/`scene` shapes belong to Task
//! 2; these tests only pin that Task 1 does not drop their data.

use yunari_model::document::{
    DocumentError, SCHEMA_VERSION, SUPPORTED_SCHEMA_MAJOR, parse_document, write_document,
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

#[test]
fn refuses_schema_major_four_without_coercion() {
    let json = MINIMAL_SCHEMA3.replace("3.0.0", "4.0.0");
    let error = parse_document(&json).expect_err("major 4 is refused");
    let DocumentError::Invalid(issues) = error else {
        panic!("expected a version validation error, got {error:?}");
    };
    assert!(issues.iter().any(|issue| issue.path == "schemaVersion"));
}
