//! RFC 6901 JSON Pointer, limited to what a reading needs: resolve a pointer
//! against a parsed document, and report a miss rather than panic. A pointer
//! comes from `sources.json`, so an unusable one costs a sample, not the fetch
//! that produced the document.

use serde_json::Value;

/// An array index: `0`, or a decimal without a leading zero. `-` is not one.
fn array_index(token: &str) -> Option<usize> {
    if token.is_empty() || (token.len() > 1 && token.starts_with('0')) {
        return None;
    }
    token
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| token.parse().ok())?
}

/// The value a pointer names, or nothing when the document does not have it.
pub fn resolve<'a>(document: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return Some(document);
    }
    let mut current = document;
    for raw in pointer.strip_prefix('/')?.split('/') {
        // `~1` first, then `~0`: `~01` is `~0` followed by a literal `1`.
        if raw.contains('~') && invalid_escape(raw) {
            return None;
        }
        let token = raw.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Array(items) => items.get(array_index(&token)?)?,
            Value::Object(record) => record.get(&token)?,
            _ => return None,
        };
    }
    Some(current)
}

fn invalid_escape(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'~' && !matches!(bytes.get(index + 1), Some(b'0' | b'1')) {
            return true;
        }
        index += 1;
    }
    false
}

/// Whether a pointer is a location at all, which decides how a reason names it.
pub fn valid(pointer: &str) -> bool {
    pointer.is_empty()
        || (pointer.starts_with('/')
            && pointer
                .split('/')
                .skip(1)
                .all(|token| !invalid_escape(token)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Value {
        serde_json::json!({
            "a": {"b": [10, "two", {"c": true}]},
            "~1": "escaped slash name",
            "": 7
        })
    }

    #[test]
    fn a_pointer_reads_the_first_array_element_and_nested_objects() {
        let document = document();
        assert_eq!(resolve(&document, ""), Some(&document));
        assert_eq!(resolve(&document, "/a/b/0"), Some(&Value::from(10)));
        assert_eq!(resolve(&document, "/a/b/1"), Some(&Value::from("two")));
        assert_eq!(resolve(&document, "/a/b/2/c"), Some(&Value::Bool(true)));
        assert_eq!(
            resolve(&document, "/"),
            Some(&Value::from(7)),
            "the empty token names the empty key"
        );
        assert_eq!(
            resolve(&document, "/~01"),
            Some(&Value::from("escaped slash name"))
        );
    }

    #[test]
    fn a_missing_or_malformed_pointer_is_nothing_not_the_whole_document() {
        let document = document();
        assert_eq!(resolve(&document, "/a/b/3"), None);
        assert_eq!(resolve(&document, "/a/b/01"), None);
        assert_eq!(resolve(&document, "/a/b/-"), None);
        assert_eq!(resolve(&document, "/a/missing"), None);
        assert_eq!(resolve(&document, "/a/b/0/odd"), None);
        assert_eq!(resolve(&document, "a/b"), None);
        // `~2` is not an escape this pointer language has.
        assert_eq!(resolve(&document, "/a~2b"), None);
    }

    #[test]
    fn a_pointer_is_or_is_not_a_location() {
        assert!(valid(""));
        assert!(valid("/a/b/0"));
        assert!(valid("/~0"));
        assert!(!valid("a/b"));
        assert!(!valid("/~2"));
    }
}
