//! The pieces every weather backend shares: one bounded request, and the
//! reading of fields out of its answer.
//!
//! A reason names the service, never the URL or the key in it: the reason
//! reaches every display on the network.

use serde_json::Value;

use super::open_meteo::Fetch;

/// The timeout a weather request is held to.
pub const TIMEOUT_MS: u64 = 10_000;

/// One bounded GET, parsed as JSON, or the service's reason.
pub fn json(fetch: &Fetch, url: &str, timeout_ms: u64, label: &str) -> Result<Value, String> {
    let body =
        fetch(url, timeout_ms).map_err(|error| format!("{label} could not be reached: {error}"))?;
    serde_json::from_str(&body).map_err(|_| format!("{label} answered something unexpected."))
}

pub fn object<'a>(record: &'a Value, field: &str, label: &str) -> Result<&'a Value, String> {
    record
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| format!("{label}'s answer is missing {field}."))
}

pub fn number(record: &Value, field: &str, label: &str) -> Result<f64, String> {
    record
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{label}'s answer is missing {field}."))
}

pub fn text<'a>(record: &'a Value, field: &str, label: &str) -> Result<&'a str, String> {
    record
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{label}'s answer is missing {field}."))
}

pub fn list<'a>(record: &'a Value, field: &str, label: &str) -> Result<&'a Vec<Value>, String> {
    record
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label}'s answer is missing {field}."))
}

/// One list entry that has to be an object, or the field's refusal.
pub fn record<'a>(entry: Option<&'a Value>, field: &str, label: &str) -> Result<&'a Value, String> {
    entry
        .filter(|value| value.is_object())
        .ok_or_else(|| format!("{label}'s answer is missing {field}."))
}

/// A value a set of positions may hold: `800` is `800.0` in an answer read as
/// a number, and a whole number as text is a code, not a reading.
pub fn first<'a>(record: &'a Value, field: &str, label: &str) -> Result<&'a Value, String> {
    list(record, field, label)?
        .first()
        .ok_or_else(|| format!("{label}'s answer is missing {field}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_field_that_is_blank_or_a_number_is_missing() {
        let record = serde_json::json!({"a": "", "b": 4});
        assert!(text(&record, "a", "Svc").is_err());
        assert!(text(&record, "b", "Svc").is_err());
        assert_eq!(
            text(&serde_json::json!({"a": " x "}), "a", "Svc"),
            Ok(" x ")
        );
    }

    #[test]
    fn a_number_that_is_a_string_is_missing_and_a_list_must_be_a_list() {
        assert!(number(&serde_json::json!({"a": "4"}), "a", "Svc").is_err());
        assert!(list(&serde_json::json!({"a": {}}), "a", "Svc").is_err());
        assert!(record(Some(&serde_json::json!("x")), "list", "Svc").is_err());
    }
}
