//! Differential checks against fixtures generated from the source app.
//!
//! `scripts/parity-fixtures.mjs` runs the source's own formatting functions and
//! writes their answers here. These tests replay the same inputs through the
//! port. A failure means the port disagrees with the source, not with an
//! agent's reading of it.
//!
//! Regenerate after a source change:
//!   cd D:/git-repos/yunari/src/web
//!   npx tsx D:/git-repos/yunari-rust/scripts/parity-fixtures.mjs \
//!     D:/git-repos/yunari-rust/crates/render/tests/fixtures

use std::collections::BTreeMap;

use serde_json::Value;
use yunari_model::document::UnitDisplay;
use yunari_render::instant::format_instant;
use yunari_render::{format_number, format_unit};

fn fixtures() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/source-format.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("fixture file"))
        .expect("fixture JSON")
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

#[test]
fn numbers_match_the_source_in_every_language() {
    let document = fixtures();
    let mut checked = 0;
    let mut unexpected = Vec::new();
    for case in document["numbers"].as_array().expect("numbers") {
        let value: f64 = str_at(case, "value")
            .parse()
            .unwrap_or_else(|_| panic!("value {case}"));
        let precision = case["precision"]
            .as_u64()
            .map(|digits| u8::try_from(digits).expect("precision fits"));
        let separator: char = str_at(case, "separator").chars().next().expect("separator");
        let expected = str_at(case, "expected");
        let actual = format_number(value, precision, separator);
        checked += 1;
        if actual != expected {
            // The source prints `-0` for a negative reading that rounds to zero
            // at a pinned precision; its own comment calls that a rounding
            // artefact. The port never prints negative zero, so this one case
            // is a recorded divergence rather than a port bug.
            if str_at(case, "value") == "-0.05" && precision == Some(0) {
                assert_eq!(actual, "0", "port keeps its no-negative-zero rule");
                continue;
            }
            unexpected.push(format!(
                "value {} precision {:?} language {}: port {actual:?} source {expected:?}",
                str_at(case, "value"),
                precision,
                str_at(case, "language")
            ));
        }
    }
    assert_eq!(checked, 315, "fixture set changed: {checked}");
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

#[test]
fn units_match_the_source_spacing() {
    let document = fixtures();
    for case in document["units"].as_array().expect("units") {
        let unit = str_at(case, "unit");
        let display = match str_at(case, "display") {
            "none" => UnitDisplay::None,
            "short" => UnitDisplay::Short,
            "long" => UnitDisplay::Long,
            other => panic!("unknown display {other}"),
        };
        let long: Option<BTreeMap<String, String>> = case.get("long").and_then(|value| {
            value.as_object().map(|map| {
                map.iter()
                    .map(|(key, value)| {
                        (key.clone(), value.as_str().unwrap_or_default().to_owned())
                    })
                    .collect()
            })
        });
        assert_eq!(
            format_unit(Some(unit), display, long.as_ref()),
            str_at(case, "expected"),
            "unit {unit} display {}",
            str_at(case, "display")
        );
    }
}

#[test]
fn instants_match_the_source_tokens_and_words() {
    let document = fixtures();
    let mut checked = 0;
    for case in document["instants"].as_array().expect("instants") {
        let zone = case["zone"].as_str();
        let language = str_at(case, "language");
        let expected = case["expected"].as_str().map(str::to_owned);
        let actual = format_instant(
            str_at(case, "value"),
            str_at(case, "format"),
            zone,
            Some(language),
        );
        assert_eq!(
            actual,
            expected,
            "value {} format {} zone {:?} language {language}",
            str_at(case, "value"),
            str_at(case, "format"),
            zone
        );
        checked += 1;
    }
    assert!(checked >= 180, "fixture set shrank: {checked}");
}
