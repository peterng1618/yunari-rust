//! User-authored HTTP JSON sources as readings.
//!
//! `sample()` is a cache read: a display asking for one of a source's keys
//! records demand, and one refresher fetches each demanded source at that
//! source's own interval. A source is never fetched while nothing asks for it,
//! and a failing source costs only its own keys.

pub mod pointer;
pub mod sources;

use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;
use sources::{CustomField, CustomSource, Kind};

pub const WAITING_MESSAGE: &str = "Waiting for the first reading.";
pub const STALE_MESSAGE: &str = "The last reading is out of date.";
pub const NO_SOURCES_MESSAGE: &str = "No custom sources are configured.";
pub const NOT_JSON_MESSAGE: &str = "The source answered something that is not JSON.";

/// Missed intervals after which a last good value is stale rather than fresh.
const STALE_INTERVALS: u64 = 3;
/// The same grace bounds how long a source keeps polling with no demand.
const DEMAND_GRACE_INTERVALS: u64 = 3;
/// How often the refresher checks which source is due.
const WORKER_TICK: Duration = Duration::from_millis(500);
/// The longest pointer a reason quotes.
const MAX_POINTER_CHARS: usize = 64;
/// A reason is one line on a fixed strip.
const MESSAGE_MAX_CHARS: usize = 120;

/// One GET performed for a source. A seam so a test never reaches the network.
pub type Fetch = Arc<dyn Fn(&crate::http::Get) -> Result<String, String> + Send + Sync>;

/// What `sources.json` said: the sources that can be read, and one line per
/// entry that was skipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded {
    pub sources: Vec<CustomSource>,
    pub issues: Vec<String>,
}

/// How the file is read. A seam so a test holds it still, and so a file edited
/// mid-session is picked up on the next poll.
pub type SettingsLoader = Arc<dyn Fn() -> Loaded + Send + Sync>;

struct Runtime {
    source: CustomSource,
    document: Option<Value>,
    fetched_at_ms: Option<u64>,
    /// Redacted; may reach a browser through `health()` and a reading.
    failure: Option<String>,
    next_due_ms: u64,
    last_demand_ms: Option<u64>,
}

impl Runtime {
    fn new(source: CustomSource) -> Self {
        Self {
            source,
            document: None,
            fetched_at_ms: None,
            failure: None,
            next_due_ms: 0,
            last_demand_ms: None,
        }
    }

    fn demanded(&self, now_ms: u64) -> bool {
        self.last_demand_ms.is_some_and(|last| {
            now_ms.saturating_sub(last) <= DEMAND_GRACE_INTERVALS * self.source.interval_ms
        })
    }
}

#[derive(Default)]
struct State {
    runtimes: Vec<Runtime>,
    issues: Vec<String>,
}

pub struct CustomProvider {
    settings: SettingsLoader,
    fetch: Fetch,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    state: Arc<Mutex<State>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// False in tests: fetching is driven by [`CustomProvider::refresh`].
    auto: bool,
}

impl CustomProvider {
    pub fn new(settings: SettingsLoader) -> Self {
        Self::with_fetch(
            settings,
            Arc::new(crate::http::get_with),
            Arc::new(now_ms),
            true,
        )
    }

    pub fn with_fetch(
        settings: SettingsLoader,
        fetch: Fetch,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        auto: bool,
    ) -> Self {
        Self {
            settings,
            fetch,
            clock,
            state: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            auto,
        }
    }

    /// Fetches every source that is due, and reports how many were fetched.
    pub fn refresh(&self, now_ms: u64) -> usize {
        let loaded = (self.settings)();
        {
            let mut state = self.state.lock().expect("the custom state");
            sync(&mut state, loaded);
        }
        refresh(&self.state, &self.fetch, now_ms)
    }

    /// Starts the refresher unless one is already running. It ends by itself
    /// once no display has asked for a while, and the next ask starts another.
    fn ensure_worker(&self) {
        let mut worker = self.worker.lock().expect("the custom worker");
        if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        let settings = Arc::clone(&self.settings);
        let state = Arc::clone(&self.state);
        let clock = Arc::clone(&self.clock);
        let fetch = Arc::clone(&self.fetch);
        *worker = Some(thread::spawn(move || {
            loop {
                let now = clock();
                let loaded = settings();
                let demanded = {
                    let mut state = state.lock().expect("the custom state");
                    sync(&mut state, loaded);
                    state.runtimes.iter().any(|runtime| runtime.demanded(now))
                };
                if !demanded {
                    return;
                }
                refresh(&state, &fetch, now);
                thread::sleep(WORKER_TICK);
            }
        }));
    }
}

impl Provider for CustomProvider {
    fn id(&self) -> &str {
        "custom"
    }

    fn label(&self) -> &str {
        "Custom sources"
    }

    fn describe(&self) -> Vec<Descriptor> {
        (self.settings)()
            .sources
            .into_iter()
            .flat_map(|source| {
                let keys: Vec<Descriptor> = source
                    .fields
                    .iter()
                    .map(|field| Descriptor::new(source.key(field), field.unit.as_deref()))
                    .collect();
                keys
            })
            .collect()
    }

    fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String> {
        let loaded = (self.settings)();
        let mut state = self.state.lock().expect("the custom state");
        sync(&mut state, loaded);

        // A sample is the demand signal: only a key a source declares starts
        // that source's clock.
        let mut asked: Vec<(usize, usize)> = Vec::new();
        for key in keys {
            let Some((source_id, field_id)) = split_key(key) else {
                continue;
            };
            let Some(source_index) = state
                .runtimes
                .iter()
                .position(|runtime| runtime.source.id == source_id)
            else {
                continue;
            };
            let Some(field_index) = state.runtimes[source_index]
                .source
                .fields
                .iter()
                .position(|field| field.id == field_id)
            else {
                continue;
            };
            state.runtimes[source_index].last_demand_ms = Some(now_ms);
            asked.push((source_index, field_index));
        }
        if asked.is_empty() {
            return Ok(Vec::new());
        }

        if self.auto {
            drop(state);
            self.ensure_worker();
            state = self.state.lock().expect("the custom state");
        }

        Ok(asked
            .into_iter()
            .map(|(source_index, field_index)| {
                let runtime = &state.runtimes[source_index];
                reading_of(
                    &runtime.source,
                    &runtime.source.fields[field_index],
                    runtime,
                    now_ms,
                )
            })
            .collect())
    }

    fn health(&self) -> Health {
        let state = self.state.lock().expect("the custom state");
        if state.runtimes.is_empty() {
            let issue = state
                .issues
                .first()
                .cloned()
                .unwrap_or_else(|| NO_SOURCES_MESSAGE.to_owned());
            return Health::Unavailable(issue);
        }
        if !state.issues.is_empty() {
            let count = state.issues.len();
            return Health::Degraded(format!(
                "{count} issue{} in sources.json; see that file in the settings folder.",
                if count == 1 { "" } else { "s" }
            ));
        }
        match state
            .runtimes
            .iter()
            .find_map(|runtime| runtime.failure.clone())
        {
            Some(reason) => Health::Degraded(reason),
            None => Health::Ok,
        }
    }
}

/// Keeps one runtime per configured source, in the file's order. A changed
/// source starts from nothing: its old document was fetched from another place.
fn sync(state: &mut State, loaded: Loaded) {
    state.issues = loaded.issues;
    let mut existing = std::mem::take(&mut state.runtimes);
    let mut next = Vec::with_capacity(loaded.sources.len());
    for source in loaded.sources {
        let found = existing
            .iter()
            .position(|runtime| runtime.source.id == source.id)
            .map(|index| existing.remove(index));
        match found {
            Some(mut runtime) => {
                if runtime.source != source {
                    runtime = Runtime::new(source);
                }
                next.push(runtime);
            }
            None => next.push(Runtime::new(source)),
        }
    }
    state.runtimes = next;
}

/// Fetches every source that is due. One source's failure is its own keys'.
fn refresh(state: &Mutex<State>, fetch: &Fetch, now_ms: u64) -> usize {
    let due: Vec<(usize, crate::http::Get, u64)> = {
        let state = state.lock().expect("the custom state");
        state
            .runtimes
            .iter()
            .enumerate()
            .filter(|(_, runtime)| runtime.demanded(now_ms) && now_ms >= runtime.next_due_ms)
            .map(|(index, runtime)| {
                (
                    index,
                    crate::http::Get {
                        url: runtime.source.request_url(),
                        headers: runtime.source.headers.clone(),
                        timeout_ms: runtime.source.timeout_ms,
                    },
                    runtime.source.interval_ms,
                )
            })
            .collect()
    };

    for (index, request, interval_ms) in &due {
        let outcome = match crate::http::refusal(&request.url) {
            Some(refusal) => Err(refusal),
            None => fetch(request).and_then(|body| {
                serde_json::from_str::<Value>(&body).map_err(|_| NOT_JSON_MESSAGE.to_owned())
            }),
        };
        let mut state = state.lock().expect("the custom state");
        let Some(runtime) = state.runtimes.get_mut(*index) else {
            continue;
        };
        match outcome {
            Ok(document) => {
                runtime.document = Some(document);
                runtime.fetched_at_ms = Some(now_ms);
                runtime.failure = None;
            }
            Err(reason) => runtime.failure = Some(reason),
        }
        runtime.next_due_ms = now_ms.saturating_add(*interval_ms);
    }
    due.len()
}

/// `custom.<source>.<field>`; neither id may contain `.`, so the split is whole.
fn split_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("custom.")?;
    let (source, field) = rest.split_once('.')?;
    (!source.is_empty() && !field.is_empty() && !field.contains('.')).then_some((source, field))
}

/// One field's reading from the document the refresher stored.
fn reading_of(
    source: &CustomSource,
    field: &CustomField,
    runtime: &Runtime,
    now_ms: u64,
) -> Reading {
    let key = source.key(field);
    let Some(document) = &runtime.document else {
        return match &runtime.failure {
            Some(reason) => Reading::failed(key, reason.clone()),
            None => Reading::missing(key, WAITING_MESSAGE),
        };
    };

    let Some(value) = pointer::resolve(document, &field.pointer) else {
        return Reading::missing(
            key,
            message_at(field, |at| {
                format!("The value at {at} is missing from the source's response.")
            }),
        );
    };

    let mismatched = |expected: &str| {
        Reading::failed(
            key.clone(),
            message_at(field, |at| {
                format!("The value at {at} did not match; expected {expected}.")
            }),
        )
    };

    let mut number = None;
    let mut text = None;
    match field.kind {
        Kind::Number => match value.as_f64().filter(|number| number.is_finite()) {
            Some(value) => {
                let scaled = value * field.scale + field.offset;
                if !scaled.is_finite() {
                    return mismatched("a finite number");
                }
                number = Some(scaled);
            }
            None => return mismatched("a number"),
        },
        Kind::Text => match value.as_str() {
            Some(value) => text = Some(value.to_owned()),
            None => return mismatched("text"),
        },
        // The source's renderer shows a boolean as these two words.
        Kind::Boolean => match value.as_bool() {
            Some(value) => {
                text = Some(if value { "on" } else { "off" }.to_owned());
            }
            None => return mismatched("a boolean"),
        },
    }

    if let Some(fetched_at_ms) = runtime.fetched_at_ms
        && now_ms.saturating_sub(fetched_at_ms) >= STALE_INTERVALS * source.interval_ms
    {
        return Reading::missing(key, STALE_MESSAGE);
    }

    match field.kind {
        Kind::Number => Reading::number(
            key,
            number.unwrap_or_default(),
            field.unit.clone().unwrap_or_default(),
        ),
        _ => Reading::text(key, text.unwrap_or_default()),
    }
}

/// A reason that names where the value should have been. The pointer is the
/// person's own text, so it is quoted verbatim — bounded to one line — while
/// the sentence around it stays fixed.
fn message_at(field: &CustomField, sentence: impl Fn(&str) -> String) -> String {
    let subject = if !pointer::valid(&field.pointer) {
        format!("field \"{}\"", field.id)
    } else if field.pointer.is_empty() {
        "the whole document".to_owned()
    } else {
        let bounded: String = if field.pointer.chars().count() > MAX_POINTER_CHARS {
            field
                .pointer
                .chars()
                .take(MAX_POINTER_CHARS - 1)
                .chain(std::iter::once('…'))
                .collect()
        } else {
            field.pointer.clone()
        };
        format!("the pointer {bounded}")
    };
    let one_line: String = subject
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                ' '
            } else if character.is_control() {
                '\u{0}'
            } else {
                character
            }
        })
        .filter(|character| *character != '\u{0}')
        .collect();
    sentence(&one_line)
        .chars()
        .take(MESSAGE_MAX_CHARS)
        .collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::sample::Status;

    fn loaded() -> Loaded {
        let (sources, issues) = sources::parse(&serde_json::json!({
            "sources": [{
                "id": "lab",
                "name": "Lab",
                "url": "http://127.0.0.1:9000/status",
                "intervalMs": 60000,
                "fields": [
                    {"id": "temp", "label": "Temperature", "pointer": "/a/b/0", "type": "number", "unit": "°C", "scale": 2, "offset": 1},
                    {"id": "name", "pointer": "/a/name", "type": "text"},
                    {"id": "up", "pointer": "/up", "type": "boolean"},
                    {"id": "gone", "pointer": "/a/missing", "type": "number"}
                ]
            }]
        }));
        Loaded { sources, issues }
    }

    fn body() -> String {
        serde_json::json!({"a": {"b": [20.5], "name": "Lab"}, "up": true}).to_string()
    }

    fn provider(
        loaded: Loaded,
        fetch: impl Fn(&crate::http::Get) -> Result<String, String> + Send + Sync + 'static,
    ) -> (CustomProvider, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let urls = Arc::new(Mutex::new(Vec::new()));
        let counted = Arc::clone(&calls);
        let recorded = Arc::clone(&urls);
        let fetch: Fetch = Arc::new(move |request| {
            counted.fetch_add(1, Ordering::SeqCst);
            recorded.lock().unwrap().push(request.url.clone());
            fetch(request)
        });
        let source = loaded.clone();
        let provider = CustomProvider::with_fetch(
            Arc::new(move || source.clone()),
            fetch,
            Arc::new(|| 0),
            false,
        );
        (provider, calls, urls)
    }

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn reading<'a>(readings: &'a [Reading], key: &str) -> &'a Reading {
        readings
            .iter()
            .find(|reading| reading.semantic_key == key)
            .expect("a reading for the key")
    }

    #[test]
    fn a_pointer_reads_the_first_array_element_and_the_field_types_map() {
        let (provider, calls, urls) = provider(loaded(), |_request| Ok(body()));
        let asked = keys(&[
            "custom.lab.temp",
            "custom.lab.name",
            "custom.lab.up",
            "custom.lab.gone",
        ]);
        // A sample is the demand signal; the fetch follows it.
        provider.sample(&asked, 1_000).unwrap();
        assert_eq!(provider.refresh(1_000), 1);
        let readings = provider.sample(&asked, 1_100).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*urls.lock().unwrap(), vec!["http://127.0.0.1:9000/status"]);
        // 20.5 scaled by 2 and offset by 1, in the field's own unit.
        assert_eq!(reading(&readings, "custom.lab.temp").value, Some(42.0));
        assert_eq!(
            reading(&readings, "custom.lab.temp").unit.as_deref(),
            Some("°C")
        );
        assert_eq!(
            reading(&readings, "custom.lab.name").text.as_deref(),
            Some("Lab")
        );
        assert_eq!(
            reading(&readings, "custom.lab.up").text.as_deref(),
            Some("on")
        );
        let gone = reading(&readings, "custom.lab.gone");
        assert_eq!(gone.status, Status::Missing);
        assert_eq!(
            gone.message.as_deref(),
            Some("The value at the pointer /a/missing is missing from the source's response.")
        );
    }

    #[test]
    fn a_value_of_the_wrong_type_is_a_reason_not_a_zero() {
        let (provider, ..) = provider(loaded(), |_request| {
            Ok(serde_json::json!({"a": {"b": ["hot"]}, "up": "yes"}).to_string())
        });
        let asked = keys(&["custom.lab.temp", "custom.lab.up"]);
        provider.sample(&asked, 1_000).unwrap();
        provider.refresh(1_000);
        let readings = provider.sample(&asked, 1_100).unwrap();
        assert_eq!(readings[0].status, Status::Error);
        assert_eq!(
            readings[0].message.as_deref(),
            Some("The value at the pointer /a/b/0 did not match; expected a number.")
        );
        assert_eq!(readings[1].status, Status::Error);
    }

    #[test]
    fn a_file_url_is_refused_before_any_request() {
        let url = "file:///etc/passwd";
        let (sources, _) = sources::parse(&serde_json::json!({
            "sources": [{"id": "files", "url": url, "fields": [
                {"id": "x", "pointer": "", "type": "number"}
            ]}]
        }));
        // The file itself is refused, so the source never reaches a runtime.
        assert!(sources.is_empty());

        // And a source that somehow carried one is refused before the fetch.
        let source = CustomSource {
            id: "files".to_owned(),
            name: "Files".to_owned(),
            url: url.to_owned(),
            interval_ms: 60_000,
            timeout_ms: sources::TIMEOUT_DEFAULT_MS,
            headers: Vec::new(),
            query: Vec::new(),
            fields: vec![CustomField {
                id: "x".to_owned(),
                label: "X".to_owned(),
                pointer: String::new(),
                kind: Kind::Number,
                unit: None,
                scale: 1.0,
                offset: 0.0,
            }],
        };
        let (provider, calls, _) = provider(
            Loaded {
                sources: vec![source],
                issues: Vec::new(),
            },
            |_request| Ok("{}".to_owned()),
        );
        provider.sample(&keys(&["custom.files.x"]), 1_000).unwrap();
        provider.refresh(1_000);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let readings = provider.sample(&keys(&["custom.files.x"]), 1_100).unwrap();
        assert_eq!(readings[0].status, Status::Error);
        assert!(
            readings[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("only http and https")),
            "{:?}",
            readings[0].message
        );
    }

    #[test]
    fn a_last_good_value_goes_stale_after_three_intervals() {
        let (provider, calls, _) = provider(loaded(), |_request| Ok(body()));
        provider.sample(&keys(&["custom.lab.temp"]), 1_000).unwrap();
        provider.refresh(1_000);
        let fresh = provider.sample(&keys(&["custom.lab.temp"]), 2_000).unwrap();
        assert_eq!(fresh[0].status, Status::Ok);
        let stale = provider
            .sample(&keys(&["custom.lab.temp"]), 181_000)
            .unwrap();
        assert_eq!(stale[0].status, Status::Missing);
        assert_eq!(stale[0].message.as_deref(), Some(STALE_MESSAGE));
        // A stale sample does not fetch; only a due one does.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failed_fetch_is_its_own_keys_reason_and_the_next_one_waits_for_the_interval() {
        let (provider, calls, _) =
            provider(loaded(), |_request| Err("connection refused".to_owned()));
        provider.sample(&keys(&["custom.lab.temp"]), 1_000).unwrap();
        assert_eq!(provider.refresh(1_000), 1);
        let readings = provider.sample(&keys(&["custom.lab.temp"]), 1_100).unwrap();
        assert_eq!(readings[0].status, Status::Error);
        assert_eq!(readings[0].message.as_deref(), Some("connection refused"));
        assert!(matches!(provider.health(), Health::Degraded(_)));
        // Not due until the interval has passed.
        assert_eq!(provider.refresh(2_000), 0);
        assert_eq!(provider.refresh(61_000), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_sources_headers_query_and_deadline_reach_the_request() {
        let loaded = Loaded {
            sources: sources::parse(&serde_json::json!({
                "sources": [{
                    "id": "lab",
                    "url": "http://127.0.0.1:9000/status",
                    "timeoutMs": 5000,
                    "headers": {"accept": "application/json"},
                    "query": {"place": "a b&c=d"},
                    "fields": [{"id": "x", "pointer": "", "type": "number"}]
                }]
            }))
            .0,
            issues: Vec::new(),
        };
        let (provider, _, urls) = provider(loaded, |request| {
            assert_eq!(request.timeout_ms, 5_000);
            assert_eq!(
                request.headers,
                vec![("accept".to_owned(), "application/json".to_owned())]
            );
            Ok(serde_json::json!({"x": 1}).to_string())
        });
        provider.sample(&keys(&["custom.lab.x"]), 1_000).unwrap();
        provider.refresh(1_000);
        assert_eq!(
            urls.lock().unwrap().as_slice(),
            ["http://127.0.0.1:9000/status?place=a%20b%26c%3Dd"]
        );
    }

    #[test]
    fn a_header_that_could_split_the_request_is_refused_at_parse() {
        let (sources, issues) = sources::parse(&serde_json::json!({
            "sources": [{
                "id": "lab",
                "url": "http://127.0.0.1:9000/status",
                "headers": {"x-bad": "a b"},
                "fields": [{"id": "x", "pointer": "", "type": "number"}]
            }]
        }));
        // A space is fine in a value; a control character is not.
        assert_eq!(sources.len(), 1, "{issues:?}");
        let (sources, issues) = sources::parse(&serde_json::json!({
            "sources": [{
                "id": "lab",
                "url": "http://127.0.0.1:9000/status",
                "headers": {"bad name": "value"},
                "fields": [{"id": "x", "pointer": "", "type": "number"}]
            }]
        }));
        assert!(sources.is_empty());
        assert!(
            issues.iter().any(|issue| issue.contains("header name")),
            "{issues:?}"
        );
    }

    #[test]
    fn an_unconfigured_key_starts_no_fetch() {
        let (provider, calls, _) = provider(loaded(), |_request| Ok(body()));
        let readings = provider
            .sample(&keys(&["custom.other.temp", "cpu.load"]), 1_000)
            .unwrap();
        assert!(readings.is_empty());
        assert_eq!(provider.refresh(1_000), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_file_that_is_not_http_is_not_a_source() {
        let loaded = Loaded {
            sources: Vec::new(),
            issues: vec!["The source x: file is not an http address.".to_owned()],
        };
        let (provider, ..) = provider(loaded, |_request| Ok(body()));
        assert!(matches!(provider.health(), Health::Unavailable(_)));
        assert!(provider.describe().is_empty());
    }
}
