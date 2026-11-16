//! `sources.json`, read into the shape the provider fetches.
//!
//! A file a person edits by hand: an entry that cannot be used is skipped with
//! a one-line issue rather than taking the file or the host down. Only the
//! fields this build reads are taken; a key it does not know is ignored, so a
//! file written for the desktop app still loads.

use serde_json::Value;

use crate::http::encode;

pub const FILE: &str = "sources.json";
pub const MAX_SOURCES: usize = 32;
pub const MAX_FIELDS: usize = 64;

pub const INTERVAL_DEFAULT_MS: u64 = 60_000;
pub const TIMEOUT_DEFAULT_MS: u64 = 2_000;
const TIMEOUT_MIN_MS: u64 = 500;
const TIMEOUT_MAX_MS: u64 = 30_000;
pub const MAX_HEADERS: usize = 16;
const MAX_HEADER_NAME: usize = 64;
const MAX_HEADER_VALUE: usize = 1024;
const INTERVAL_MIN_MS: u64 = 10_000;
const INTERVAL_MAX_MS: u64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Text,
    Boolean,
}

impl Kind {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "number" => Some(Self::Number),
            "text" => Some(Self::Text),
            "boolean" => Some(Self::Boolean),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomField {
    pub id: String,
    pub label: String,
    pub pointer: String,
    pub kind: Kind,
    pub unit: Option<String>,
    pub scale: f64,
    pub offset: f64,
}

pub const MAX_PAGES: u32 = 100;

/// Pages fetched in order and merged only when every one succeeded: a partial
/// body is not a complete reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paging {
    /// The query parameter the page number goes in.
    pub page_param: String,
    pub first_page: u32,
    pub max_pages: u32,
    /// A pointer to the array each page carries, when the page is an envelope.
    pub items_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub interval_ms: u64,
    /// The source's own request deadline.
    pub timeout_ms: u64,
    /// Headers sent with the request, as the file names them.
    pub headers: Vec<(String, String)>,
    /// Query parameters added to the address, already encoded.
    pub query: Vec<(String, String)>,
    /// How several pages become one document, when the source is paged.
    pub paging: Option<Paging>,
    pub fields: Vec<CustomField>,
}

impl CustomSource {
    /// `custom.<source>.<field>`, the key a field is read by.
    pub fn key(&self, field: &CustomField) -> String {
        format!("custom.{}.{}", self.id, field.id)
    }

    /// The address the request goes to, with the source's query parameters and
    /// the page number when one is named.
    pub fn request_url(&self, page: Option<u32>) -> String {
        let mut pairs: Vec<String> = self
            .query
            .iter()
            .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
            .collect();
        if let (Some(paging), Some(page)) = (&self.paging, page) {
            pairs.push(format!("{}={page}", encode(&paging.page_param)));
        }
        if pairs.is_empty() {
            return self.url.clone();
        }
        let separator = if self.url.contains('?') { '&' } else { '?' };
        format!("{}{separator}{}", self.url, pairs.join("&"))
    }
}

/// How a paged source becomes one document.
fn parse_paging(id: &str, value: Option<&Value>) -> Result<Option<Paging>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(record) = value.as_object() else {
        return Err(format!(
            "The source {id} has a paging block that is not an object."
        ));
    };
    let page_param = record
        .get("pageParam")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if page_param.is_empty()
        || page_param.len() > MAX_HEADER_NAME
        || !page_param
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "The source {id} pages on a parameter name that is not usable."
        ));
    }
    let whole = |field: &str, fallback: u32, min: u32, max: u32| {
        record
            .get(field)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(min as f64, max as f64) as u32)
            .unwrap_or(fallback)
    };
    let first_page = whole("firstPage", 1, 0, MAX_PAGES - 1);
    let max_pages = whole("maxPages", 1, 1, MAX_PAGES);
    let items_path = record
        .get("itemsPath")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(path) = &items_path
        && !crate::custom::pointer::valid(path)
    {
        return Err(format!(
            "The source {id} pages on an items path that is not a JSON pointer."
        ));
    }
    Ok(Some(Paging {
        page_param: page_param.to_owned(),
        first_page,
        max_pages,
        items_path,
    }))
}

/// The headers a source sends. A name is a header token and a value is one
/// line: a pair that could split the request is refused rather than sent.
fn parse_headers(id: &str, value: Option<&Value>) -> Result<Vec<(String, String)>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(record) = value.as_object() else {
        return Err(format!(
            "The source {id} has a headers list that is not an object."
        ));
    };
    if record.len() > MAX_HEADERS {
        return Err(format!(
            "The source {id} names more than {MAX_HEADERS} headers."
        ));
    }
    let mut headers = Vec::with_capacity(record.len());
    for (name, value) in record {
        let Some(value) = value.as_str() else {
            return Err(format!("The source {id} has a header that is not text."));
        };
        let usable = !name.is_empty()
            && name.len() <= MAX_HEADER_NAME
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if !usable {
            return Err(format!(
                "The source {id} has a header name that is not usable."
            ));
        }
        if value.len() > MAX_HEADER_VALUE || value.chars().any(char::is_control) {
            return Err(format!(
                "The source {id} has a value for {name} that cannot be sent."
            ));
        }
        headers.push((name.clone(), value.to_owned()));
    }
    Ok(headers)
}

/// The query parameters a source adds to its own address.
fn parse_query(id: &str, value: Option<&Value>) -> Result<Vec<(String, String)>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(record) = value.as_object() else {
        return Err(format!(
            "The source {id} has a query that is not an object."
        ));
    };
    let mut query = Vec::with_capacity(record.len());
    for (name, value) in record {
        let Some(value) = value.as_str() else {
            return Err(format!(
                "The source {id} has a query value that is not text."
            ));
        };
        if name.is_empty() || name.len() > MAX_HEADER_NAME || value.len() > MAX_HEADER_VALUE {
            return Err(format!(
                "The source {id} has a query pair that is not usable."
            ));
        }
        query.push((name.clone(), value.to_owned()));
    }
    Ok(query)
}

/// The sources this build can read, and one line per entry it skipped.
pub fn parse(value: &Value) -> (Vec<CustomSource>, Vec<String>) {
    let mut sources = Vec::new();
    let mut issues = Vec::new();
    let Some(entries) = value.get("sources").and_then(Value::as_array) else {
        return (sources, issues);
    };
    for entry in entries.iter().take(MAX_SOURCES) {
        let Some(record) = entry.as_object() else {
            issues.push("A source in sources.json is not an object.".to_owned());
            continue;
        };
        let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
        if !valid_source_id(id) {
            issues.push(format!("A source id {id:?} is not usable."));
            continue;
        }
        let Some(url) = record.get("url").and_then(Value::as_str) else {
            issues.push(format!("The source {id} has no address."));
            continue;
        };
        if let Some(refusal) = crate::http::refusal(url) {
            issues.push(format!("The source {id}: {refusal}"));
            continue;
        }
        let headers = match parse_headers(id, record.get("headers")) {
            Ok(headers) => headers,
            Err(issue) => {
                issues.push(issue);
                continue;
            }
        };
        let query = match parse_query(id, record.get("query")) {
            Ok(query) => query,
            Err(issue) => {
                issues.push(issue);
                continue;
            }
        };
        let paging = match parse_paging(id, record.get("paging")) {
            Ok(paging) => paging,
            Err(issue) => {
                issues.push(issue);
                continue;
            }
        };
        let timeout_ms = record
            .get("timeoutMs")
            .and_then(Value::as_f64)
            .filter(|value| {
                value.is_finite()
                    && *value >= TIMEOUT_MIN_MS as f64
                    && *value <= TIMEOUT_MAX_MS as f64
            })
            .map_or(TIMEOUT_DEFAULT_MS, |value| value as u64);
        let interval_ms = record
            .get("intervalMs")
            .and_then(Value::as_f64)
            .filter(|value| {
                value.is_finite()
                    && *value >= INTERVAL_MIN_MS as f64
                    && *value <= INTERVAL_MAX_MS as f64
            })
            .map_or(INTERVAL_DEFAULT_MS, |value| value as u64);
        let fields: Vec<CustomField> = record
            .get("fields")
            .and_then(Value::as_array)
            .map(|fields| {
                fields
                    .iter()
                    .take(MAX_FIELDS)
                    .filter_map(|field| parse_field(id, field, &mut issues))
                    .collect()
            })
            .unwrap_or_default();
        if fields.is_empty() {
            issues.push(format!("The source {id} has no field a display can read."));
            continue;
        }
        sources.push(CustomSource {
            id: id.to_owned(),
            name: record
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_owned(),
            url: url.to_owned(),
            interval_ms,
            timeout_ms,
            headers,
            query,
            paging,
            fields,
        });
    }
    (sources, issues)
}

fn parse_field(source_id: &str, value: &Value, issues: &mut Vec<String>) -> Option<CustomField> {
    let record = value.as_object()?;
    let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
    if !valid_field_id(id) {
        issues.push(format!(
            "The source {source_id} has a field id {id:?} that is not usable."
        ));
        return None;
    }
    let Some(pointer) = record.get("pointer").and_then(Value::as_str) else {
        issues.push(format!("The field {source_id}.{id} has no pointer."));
        return None;
    };
    let Some(kind) = record
        .get("type")
        .and_then(Value::as_str)
        .and_then(Kind::parse)
    else {
        issues.push(format!(
            "The field {source_id}.{id} is not a number, text or boolean."
        ));
        return None;
    };
    let number = |field: &str, fallback: f64| {
        record
            .get(field)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(fallback)
    };
    Some(CustomField {
        id: id.to_owned(),
        label: record
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_owned(),
        pointer: pointer.to_owned(),
        kind,
        unit: record
            .get("unit")
            .and_then(Value::as_str)
            .filter(|unit| !unit.is_empty())
            .map(str::to_owned),
        scale: number("scale", 1.0),
        offset: number("offset", 0.0),
    })
}

/// A source id may carry `_` and `-`, and may start with one.
fn valid_source_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn valid_field_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> Value {
        serde_json::json!({
            "sources": [
                {
                    "id": "my_source",
                    "name": "Lab",
                    "url": "http://127.0.0.1:9000/status",
                    "intervalMs": 5000,
                    "unknown": "ignored",
                    "fields": [
                        {"id": "field_1", "label": "Temperature", "pointer": "/a/b/0", "type": "number", "unit": "°C", "scale": 2},
                        {"id": "Name", "pointer": "", "type": "text"},
                        {"id": "up", "pointer": "/up", "type": "boolean", "offset": 1}
                    ]
                },
                {"id": "Bad Id", "url": "http://127.0.0.1:1/x", "fields": []},
                {"id": "files", "url": "file:///etc/passwd", "fields": []}
            ]
        })
    }

    #[test]
    fn a_file_is_read_into_the_sources_its_fields_name() {
        let (sources, issues) = parse(&file());
        assert_eq!(sources.len(), 1, "{issues:?}");
        let source = &sources[0];
        assert_eq!(source.id, "my_source");
        assert_eq!(source.name, "Lab");
        // Below the floor is the default cadence, not a five-second poll.
        assert_eq!(source.interval_ms, INTERVAL_DEFAULT_MS);
        assert_eq!(source.fields.len(), 3);
        assert_eq!(source.key(&source.fields[0]), "custom.my_source.field_1");
        assert_eq!(source.fields[0].kind, Kind::Number);
        assert_eq!(source.fields[0].unit.as_deref(), Some("°C"));
        assert_eq!(source.fields[0].scale, 2.0);
        assert_eq!(source.fields[1].label, "Name");
        assert_eq!(source.fields[2].offset, 1.0);
        // Two entries were skipped, each with its own line.
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues.iter().any(|issue| issue.contains("Bad Id")));
        assert!(issues.iter().any(|issue| issue.contains("only http")));
    }

    #[test]
    fn a_file_that_is_not_an_object_yields_no_sources_rather_than_a_panic() {
        assert_eq!(parse(&serde_json::json!(null)).0, Vec::new());
        assert_eq!(parse(&serde_json::json!({"sources": 7})).0, Vec::new());
    }
}
