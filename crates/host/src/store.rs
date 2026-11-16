//! The theme store, proxied so a browser never talks to a third origin.
//!
//! The catalogue lives at one configured origin (`store.json`); this host
//! reads it, keeps the answer for a few minutes and hands it on unchanged, so
//! a slow or absent store is one sentence rather than a page that hangs.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Where the catalogue is, when the user pointed this host at one.
pub const FILE: &str = "store.json";
pub const NO_STORE_MESSAGE: &str = "No theme store is configured.";
pub const NEVER_REACHED_MESSAGE: &str = "The theme store could not be reached: ";
pub const UNEXPECTED_MESSAGE: &str = "The theme store answered something unexpected.";
/// One store read a crowd of displays can share.
pub const CACHE_MS: i64 = 300_000;
pub const TIMEOUT_MS: u64 = 5_000;

/// One GET. A seam so a test counts network reads.
pub type Fetch = Arc<dyn Fn(&str, u64) -> Result<String, String> + Send + Sync>;

struct Cached {
    body: Value,
    fetched_at_ms: i64,
}

pub struct Store {
    origin: Option<String>,
    fetch: Fetch,
    cached: Mutex<Option<Cached>>,
}

impl Store {
    pub fn load(settings_dir: &Path) -> Self {
        Self::with_fetch(
            read_origin(settings_dir),
            Arc::new(yunari_providers::http::get),
        )
    }

    pub fn with_fetch(origin: Option<String>, fetch: Fetch) -> Self {
        Self {
            origin: origin.filter(|origin| yunari_providers::http::refusal(origin).is_none()),
            fetch,
            cached: Mutex::new(None),
        }
    }

    /// The Worker's origin, when the user pointed this host at one.
    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    /// The catalogue, from the cache while it is fresh.
    pub fn themes(&self, now_ms: i64) -> Result<Value, String> {
        if let Some(cached) = self.cached.lock().expect("the store cache").as_ref()
            && now_ms.saturating_sub(cached.fetched_at_ms) < CACHE_MS
        {
            return Ok(cached.body.clone());
        }
        let Some(origin) = &self.origin else {
            return Err(NO_STORE_MESSAGE.to_owned());
        };
        let url = format!("{}/themes", origin.trim_end_matches('/'));
        let body = (self.fetch)(&url, TIMEOUT_MS)
            .map_err(|error| format!("{NEVER_REACHED_MESSAGE}{error}"))?;
        // The store's answer is data, never a route or a file: it is parsed
        // here and served as JSON, so a malformed answer is one sentence.
        let body: Value = serde_json::from_str(&body).map_err(|_| UNEXPECTED_MESSAGE.to_owned())?;
        *self.cached.lock().expect("the store cache") = Some(Cached {
            body: body.clone(),
            fetched_at_ms: now_ms,
        });
        Ok(body)
    }
}

fn read_origin(settings_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(settings_dir.join(FILE)).ok()?;
    let document: Value = serde_json::from_str(&text).ok()?;
    let origin = document.get("origin")?.as_str()?.trim();
    (!origin.is_empty()).then(|| origin.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn store(fetched: Arc<AtomicUsize>) -> Store {
        Store::with_fetch(
            Some("https://store.example/".to_owned()),
            Arc::new(move |url, _| {
                fetched.fetch_add(1, Ordering::SeqCst);
                assert_eq!(url, "https://store.example/themes");
                Ok(r#"{"themes":[{"id":"aurora"}]}"#.to_owned())
            }),
        )
    }

    #[test]
    fn a_repeated_request_is_answered_from_the_cache() {
        let fetched = Arc::new(AtomicUsize::new(0));
        let store = store(Arc::clone(&fetched));
        let first = store.themes(1_000).expect("a catalogue");
        assert_eq!(first["themes"][0]["id"], "aurora");
        store.themes(1_000 + CACHE_MS - 1).expect("a catalogue");
        assert_eq!(fetched.load(Ordering::SeqCst), 1);
        // Past the window it is read again.
        store.themes(1_000 + CACHE_MS).expect("a catalogue");
        assert_eq!(fetched.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn no_store_and_an_unreadable_answer_are_both_one_sentence() {
        let store = Store::with_fetch(None, Arc::new(|_, _| Err("never called".to_owned())));
        assert_eq!(store.themes(1_000), Err(NO_STORE_MESSAGE.to_owned()));

        let store = Store::with_fetch(
            Some("https://store.example".to_owned()),
            Arc::new(|_, _| Ok("not json".to_owned())),
        );
        assert_eq!(store.themes(1_000), Err(UNEXPECTED_MESSAGE.to_owned()));

        // A file address is refused before any request.
        let store = Store::with_fetch(
            Some("file:///C:/secrets".to_owned()),
            Arc::new(|_, _| Err("never called".to_owned())),
        );
        assert_eq!(store.themes(1_000), Err(NO_STORE_MESSAGE.to_owned()));
    }
}
