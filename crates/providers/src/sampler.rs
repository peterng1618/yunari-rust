//! When a poll happens: on a cadence, for the union of what displays asked.

use std::collections::{BTreeMap, BTreeSet};

use crate::registry::{Ask, Cycle, Registry};

/// The fastest a display can ask to see new readings, unless a key changes.
pub const DEFAULT_CADENCE_MS: u64 = 1_000;

/// Paces polls over a registry.
///
/// Sampling is synchronous, so a tick cannot re-enter a provider that is still
/// running: the borrow of `&mut self` ends only when the poll does. A caller
/// that wants polls off its own thread drives one sampler from one thread.
pub struct Sampler {
    registry: Registry,
    cadence_ms: u64,
    requests: BTreeMap<String, BTreeSet<String>>,
    last_poll_ms: Option<u64>,
    /// A request that arrived after the last poll is worth one early poll.
    pending: bool,
}

impl Sampler {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            cadence_ms: DEFAULT_CADENCE_MS,
            requests: BTreeMap::new(),
            last_poll_ms: None,
            pending: false,
        }
    }

    pub fn set_cadence(&mut self, cadence_ms: u64) {
        self.cadence_ms = cadence_ms.max(1);
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Replaces one display's keys. A display that shows nothing requests an
    /// empty set, which drops its keys from the union.
    pub fn request(&mut self, owner: impl Into<String>, keys: impl IntoIterator<Item = String>) {
        let keys: BTreeSet<String> = keys.into_iter().collect();
        let owner = owner.into();
        if keys.is_empty() {
            self.requests.remove(&owner);
        } else {
            self.requests.insert(owner, keys);
        }
        self.pending = true;
    }

    /// The union of every display's keys.
    pub fn wanted(&self) -> BTreeSet<String> {
        self.requests
            .values()
            .flat_map(|keys| keys.iter().cloned())
            .collect()
    }

    /// Polls when the cadence has elapsed or a request is waiting, and returns
    /// nothing otherwise.
    pub fn tick(&mut self, now_ms: u64) -> Option<Cycle> {
        let wanted = self.wanted();
        if wanted.is_empty() {
            return None;
        }
        let due = match self.last_poll_ms {
            None => true,
            Some(last) => self.pending || now_ms.saturating_sub(last) >= self.cadence_ms,
        };
        if !due {
            return None;
        }
        let asks: Vec<Ask> = self
            .requests
            .iter()
            .map(|(owner, keys)| Ask::new(owner.clone(), keys.iter().cloned()))
            .collect();
        let cycle = self.registry.sample(&asks, now_ms);
        self.last_poll_ms = Some(now_ms);
        self.pending = false;
        Some(cycle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::provider::{Descriptor, Provider};
    use crate::sample::Reading;

    struct Counter {
        polls: Arc<AtomicUsize>,
    }

    impl Provider for Counter {
        fn id(&self) -> &str {
            "counter"
        }

        fn label(&self) -> &str {
            "Counter"
        }

        fn describe(&self) -> Vec<Descriptor> {
            vec![Descriptor::new("cpu.load", Some("%"))]
        }

        fn sample(&self, keys: &[String], _now_ms: u64) -> Result<Vec<Reading>, String> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Ok(keys
                .iter()
                .map(|key| Reading::number(key.clone(), 1.0, "%"))
                .collect())
        }
    }

    fn sampler() -> (Sampler, Arc<AtomicUsize>) {
        let polls = Arc::new(AtomicUsize::new(0));
        let mut registry = Registry::new();
        registry.add(Counter {
            polls: polls.clone(),
        });
        (Sampler::new(registry), polls)
    }

    #[test]
    fn two_ticks_inside_one_cadence_poll_once() {
        let (mut sampler, polls) = sampler();
        sampler.request("panel", ["cpu.load".to_owned()]);
        assert!(sampler.tick(1_000).is_some());
        assert!(sampler.tick(1_500).is_none());
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert!(sampler.tick(2_000).is_some());
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_new_request_makes_the_next_tick_due() {
        let (mut sampler, polls) = sampler();
        sampler.request("panel", ["cpu.load".to_owned()]);
        sampler.tick(1_000);
        sampler.request("other", ["ram.used".to_owned()]);
        assert!(sampler.tick(1_100).is_some());
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_display_showing_nothing_is_not_polled_for() {
        let (mut sampler, polls) = sampler();
        sampler.request("panel", ["cpu.load".to_owned()]);
        sampler.tick(1_000);
        sampler.request("panel", Vec::new());
        assert!(sampler.tick(2_000).is_none(), "nothing is wanted");
        assert!(sampler.wanted().is_empty());
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_union_covers_every_display() {
        let (mut sampler, _) = sampler();
        sampler.request("a", ["cpu.load".to_owned()]);
        sampler.request("b", ["cpu.load".to_owned(), "ram.used".to_owned()]);
        let wanted: Vec<String> = sampler.wanted().into_iter().collect();
        assert_eq!(wanted, vec!["cpu.load", "ram.used"]);
    }
}
