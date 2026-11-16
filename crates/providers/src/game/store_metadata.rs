//! Steam store metadata for a game with no local manifest.
//!
//! Two keyless requests resolve a local name to an app id, and the answer —
//! including "no match" and a ten-minute backoff after a failure — is cached in
//! the settings folder. Cover bytes are cached only when their magic number
//! says JPEG or PNG; the route that serves them is host-relative, so no display
//! ever talks to Valve's CDN itself.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::name_match;

pub const STORE_ORIGIN: &str = "https://store.steampowered.com";
pub const PORTRAIT_ORIGIN: &str = "https://cdn.akamai.steamstatic.com";
pub const CACHE_FILE: &str = "game-metadata.json";
pub const BACKOFF_MS: u64 = 10 * 60_000;
pub const MAX_CACHE_ENTRIES: usize = 500;
pub const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;
/// A stalled store request must not hold a poll.
pub const TIMEOUT_MS: u64 = 3_000;

/// One GET. A seam so a test performs no request.
pub type Fetch = std::sync::Arc<dyn Fn(&str, u64) -> Result<String, String> + Send + Sync>;
/// One cover download, returning its bytes.
pub type Download = std::sync::Arc<dyn Fn(&str, u64) -> Result<Vec<u8>, String> + Send + Sync>;

/// The store's answer, with "no match" kept distinct from "no answer".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Match { id: u64, name: String },
    None,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// `None` is the store's own "no match" answer.
    id: Option<u64>,
    name: String,
    /// The spelling actually searched; another spelling may match anew.
    term: String,
    at_ms: u64,
}

/// The cache, read once and written only when a lookup answered.
pub struct Store {
    directory: PathBuf,
    entries: std::collections::BTreeMap<String, Entry>,
    /// A failed lookup is not retried before this.
    retry_at_ms: u64,
    fetch: Fetch,
    download: Download,
}

impl Store {
    pub fn new(directory: PathBuf, fetch: Fetch, download: Download) -> Self {
        Self {
            entries: read_cache(&directory),
            directory,
            retry_at_ms: 0,
            fetch,
            download,
        }
    }

    /// A local name resolved to a store app, cached across relaunches.
    pub fn lookup(&mut self, name: &str, now_ms: u64) -> Lookup {
        let key = name_match::normalize(name);
        if key.is_empty() {
            return Lookup::None;
        }
        if let Some(entry) = self.entries.get(&key)
            && entry.term == name
        {
            return match entry.id {
                Some(id) => Lookup::Match {
                    id,
                    name: entry.name.clone(),
                },
                None => Lookup::None,
            };
        }
        if now_ms < self.retry_at_ms {
            return Lookup::Unavailable;
        }
        let terms = [name.to_owned()];
        let mut candidates = Vec::new();
        let mut answered = false;
        for term in &terms {
            match (self.fetch)(&search_url(term), TIMEOUT_MS) {
                Ok(body) => {
                    answered = true;
                    candidates.extend(candidates_of(&body));
                }
                Err(_) => break,
            }
        }
        if !answered {
            self.retry_at_ms = now_ms.saturating_add(BACKOFF_MS);
            return Lookup::Unavailable;
        }
        let matched = candidates
            .into_iter()
            .find(|candidate| name_match::same_game(name, &candidate.1));
        let Some((id, candidate_name)) = matched else {
            self.remember(&key, None, name, name, now_ms);
            return Lookup::None;
        };
        // The store's own spelling is what a display shows.
        let name_on_store = (self.fetch)(&details_url(id), TIMEOUT_MS)
            .ok()
            .and_then(|body| details_name(&body, id))
            .unwrap_or(candidate_name);
        self.remember(&key, Some(id), &name_on_store, name, now_ms);
        Lookup::Match {
            id,
            name: name_on_store,
        }
    }

    fn remember(&mut self, key: &str, id: Option<u64>, name: &str, term: &str, now_ms: u64) {
        if self.entries.len() >= MAX_CACHE_ENTRIES
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.at_ms)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            key.to_owned(),
            Entry {
                id,
                name: name.to_owned(),
                term: term.to_owned(),
                at_ms: now_ms,
            },
        );
        self.write();
    }

    /// The cached cover for a matched app, downloaded once when it is missing.
    /// Only JPEG and PNG bytes are kept.
    pub fn cover(&self, id: u64) -> Option<String> {
        let existing = cover_path(&self.directory, id);
        if existing.is_file() {
            return existing
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| format!("/api/game/artwork/{name}"));
        }
        let bytes = (self.download)(&portrait_url(id), TIMEOUT_MS).ok()?;
        let extension = image_extension(&bytes)?;
        let path = self.directory.join(format!("store-{id}.{extension}"));
        std::fs::create_dir_all(&self.directory).ok()?;
        std::fs::write(&path, bytes).ok()?;
        Some(format!("/api/game/artwork/{}", path.file_name()?.to_str()?))
    }

    fn write(&self) {
        let entries: Map<String, Value> = self
            .entries
            .iter()
            .map(|(key, entry)| {
                let mut record = Map::new();
                match entry.id {
                    Some(id) => {
                        record.insert("id".to_owned(), Value::from(id));
                        record.insert("name".to_owned(), Value::String(entry.name.clone()));
                    }
                    None => {
                        record.insert("none".to_owned(), Value::Bool(true));
                    }
                };
                record.insert("term".to_owned(), Value::String(entry.term.clone()));
                record.insert("at".to_owned(), Value::from(entry.at_ms));
                (key.clone(), Value::Object(record))
            })
            .collect();
        let Ok(text) = serde_json::to_string_pretty(&Value::Object(entries)) else {
            return;
        };
        let _ = std::fs::create_dir_all(&self.directory);
        let temp = self.directory.join("game-metadata.json.new");
        if std::fs::write(&temp, format!("{text}\n")).is_ok() {
            let _ = std::fs::rename(&temp, self.directory.join(CACHE_FILE));
        }
    }
}

fn read_cache(directory: &Path) -> std::collections::BTreeMap<String, Entry> {
    let Ok(text) = std::fs::read_to_string(directory.join(CACHE_FILE)) else {
        return Default::default();
    };
    let Ok(Value::Object(records)) = serde_json::from_str::<Value>(&text) else {
        return Default::default();
    };
    records
        .into_iter()
        .filter_map(|(key, value)| {
            let record = value.as_object()?;
            let id = record.get("id").and_then(Value::as_u64);
            let none = record.get("none").and_then(Value::as_bool) == Some(true);
            if id.is_none() && !none {
                return None;
            }
            Some((
                key,
                Entry {
                    id,
                    name: record
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    term: record
                        .get("term")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    at_ms: record.get("at").and_then(Value::as_u64).unwrap_or_default(),
                },
            ))
        })
        .collect()
}

/// `items` of a storesearch answer, ignoring anything not shaped like an app: a
/// bundle's id would become a game's appid and route a wrong cover.
fn candidates_of(body: &str) -> Vec<(u64, String)> {
    let Ok(Value::Object(record)) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    let Some(Value::Array(items)) = record.get("items") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let item = item.as_object()?;
            if item.get("type").and_then(Value::as_str) != Some("app") {
                return None;
            }
            let id = item.get("id").and_then(Value::as_u64)?;
            let name = item.get("name").and_then(Value::as_str)?.trim();
            (!name.is_empty()).then(|| (id, name.to_owned()))
        })
        .collect()
}

/// `{appid: {"success": true, "data": {"name": …}}}`.
fn details_name(body: &str, id: u64) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let entry = value.get(id.to_string())?;
    if entry.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let name = entry.get("data")?.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn search_url(name: &str) -> String {
    format!(
        "{STORE_ORIGIN}/api/storesearch/?term={}&cc=us&l=en",
        crate::http::encode(name)
    )
}

fn details_url(id: u64) -> String {
    format!("{STORE_ORIGIN}/api/appdetails?appids={id}&filters=basic")
}

fn portrait_url(id: u64) -> String {
    format!("{PORTRAIT_ORIGIN}/steam/apps/{id}/library_600x900.jpg")
}

fn cover_path(directory: &Path, id: u64) -> PathBuf {
    for extension in ["jpg", "png"] {
        let path = directory.join(format!("store-{id}.{extension}"));
        if path.is_file() {
            return path;
        }
    }
    directory.join(format!("store-{id}.jpg"))
}

/// The extension the bytes' own magic number names, or nothing: only a real
/// JPEG or PNG is cached, never whatever a CDN error page returned.
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("jpg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn only_an_app_candidate_is_matched_and_an_absent_app_id_is_ignored() {
        let body = serde_json::json!({
            "items": [
                {"type": "bundle", "id": 1, "name": "Half-Life"},
                {"type": "app", "id": 70, "name": "Half-Life"},
                {"type": "app", "id": "x", "name": "Half-Life"}
            ]
        });
        assert_eq!(
            candidates_of(&body.to_string()),
            vec![(70, "Half-Life".to_owned())]
        );
        assert!(candidates_of("not json").is_empty());
    }

    #[test]
    fn a_lookup_answers_from_the_cache_and_a_failure_backs_off() {
        let dir = std::env::temp_dir().join(format!(
            "yunari-store-metadata-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let fetch: Fetch = Arc::new(move |url, _timeout| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if url.contains("storesearch") {
                Ok(serde_json::json!({
                    "items": [{"type": "app", "id": 70, "name": "Half-Life"}]
                })
                .to_string())
            } else if url.contains("appdetails") {
                Ok(serde_json::json!({
                    "70": {"success": true, "data": {"name": "Half-Life"}}
                })
                .to_string())
            } else {
                Err("refused".to_owned())
            }
        });
        let download: Download = Arc::new(|_url, _timeout| Ok(vec![0xff, 0xd8, 0xff, 1]));
        let mut store = Store::new(dir.clone(), fetch, download);
        assert_eq!(
            store.lookup("Half-Life", 1_000),
            Lookup::Match {
                id: 70,
                name: "Half-Life".to_owned()
            }
        );
        let after_match = calls.load(std::sync::atomic::Ordering::SeqCst);
        // The second ask is the cache, not a request.
        assert_eq!(store.lookup("Half-Life", 2_000).kind_id(), Some(70));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), after_match);

        // A cover is downloaded once and served by its file name.
        assert_eq!(
            store.cover(70).as_deref(),
            Some("/api/game/artwork/store-70.jpg")
        );
        assert!(dir.join("store-70.jpg").is_file());

        // A store that cannot answer is not "no such game", and is not asked
        // again inside the backoff.
        let mut store = Store::new(
            dir.clone(),
            Arc::new(|_url, _timeout| Err("refused".to_owned())),
            Arc::new(|_url, _timeout| Err("refused".to_owned())),
        );
        assert_eq!(store.lookup("Some Game", 1_000), Lookup::Unavailable);
        assert_eq!(store.lookup("Some Game", 2_000), Lookup::Unavailable);
        assert_eq!(image_extension(b"<html>"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    impl Lookup {
        fn kind_id(&self) -> Option<u64> {
            match self {
                Self::Match { id, .. } => Some(*id),
                _ => None,
            }
        }
    }
}
