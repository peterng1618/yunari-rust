//! Turning many displays' asks into one poll.

use std::collections::{BTreeMap, BTreeSet};

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;

/// One display's request for the keys its bindings name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ask {
    pub owner: String,
    pub keys: Vec<String>,
}

impl Ask {
    pub fn new(owner: impl Into<String>, keys: impl IntoIterator<Item = String>) -> Self {
        Self {
            owner: owner.into(),
            keys: keys.into_iter().collect(),
        }
    }
}

/// A provider that could not complete its part of a poll. The message is
/// redacted: it may reach a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub provider: String,
    pub message: String,
}

/// Everything one poll produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cycle {
    pub readings: BTreeMap<String, Reading>,
    /// Every key some display asked for and no provider answered.
    pub unmapped: Vec<String>,
    pub failures: Vec<Failure>,
}

impl Cycle {
    pub fn reading(&self, key: &str) -> Option<&Reading> {
        self.readings.get(key)
    }
}

/// The providers this app has, and the polls it runs over them.
#[derive(Default)]
pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, provider: impl Provider + 'static) {
        self.providers.push(Box::new(provider));
    }

    pub fn providers(&self) -> impl Iterator<Item = &dyn Provider> {
        self.providers.iter().map(|provider| provider.as_ref())
    }

    /// Every key the registry can read, sorted and without duplicates.
    pub fn describe(&self) -> Vec<Descriptor> {
        let mut keys: Vec<Descriptor> = self
            .providers
            .iter()
            .flat_map(|provider| provider.describe())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn health(&self) -> Vec<(&str, Health)> {
        self.providers
            .iter()
            .map(|provider| (provider.id(), provider.health()))
            .collect()
    }

    /// Each provider's name and whether it can read right now. A settings
    /// surface shows the reason a provider is unavailable without knowing why.
    pub fn status(&self) -> Vec<(&str, &str, Health)> {
        self.providers
            .iter()
            .map(|provider| (provider.id(), provider.label(), provider.health()))
            .collect()
    }

    /// Every physical device any provider can name, deduplicated.
    pub fn devices(&self) -> Vec<crate::provider::Device> {
        let mut devices: Vec<crate::provider::Device> = self
            .providers
            .iter()
            .flat_map(|provider| provider.devices())
            .collect();
        devices.sort_by(|left, right| (&left.group, &left.id).cmp(&(&right.group, &right.id)));
        devices.dedup();
        devices
    }

    /// Polls every provider once with the union of the asks' keys.
    ///
    /// A provider that fails contributes a failure and nothing else. A reading
    /// for a key nobody asked for is dropped, and a key no provider answered is
    /// unmapped rather than zero.
    pub fn sample(&self, asks: &[Ask], now_ms: u64) -> Cycle {
        let wanted: BTreeSet<String> = asks
            .iter()
            .flat_map(|ask| ask.keys.iter().cloned())
            .collect();
        let union: Vec<String> = wanted.iter().cloned().collect();
        let mut cycle = Cycle::default();
        for provider in &self.providers {
            match provider.sample(&union, now_ms) {
                Ok(readings) => {
                    for reading in readings {
                        if wanted.contains(&reading.semantic_key) {
                            cycle
                                .readings
                                .entry(reading.semantic_key.clone())
                                .or_insert(reading);
                        }
                    }
                }
                Err(message) => cycle.failures.push(Failure {
                    provider: provider.id().to_owned(),
                    message: redact(&message),
                }),
            }
        }
        cycle.unmapped = wanted
            .into_iter()
            .filter(|key| !cycle.readings.contains_key(key))
            .collect();
        cycle
    }
}

/// A provider's message as it may leave the machine: one line, bounded length,
/// no control characters.
fn redact(message: &str) -> String {
    let flat: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut bounded: String = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if bounded.chars().count() > 200 {
        bounded = bounded.chars().take(199).collect::<String>() + "…";
    }
    bounded
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    struct Counting {
        calls: Arc<AtomicUsize>,
        asked: Arc<Mutex<Vec<String>>>,
        keys: Vec<&'static str>,
        fail: Option<&'static str>,
    }

    impl Counting {
        fn new(keys: &[&'static str]) -> (Self, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let asked = Arc::new(Mutex::new(Vec::new()));
            let provider = Self {
                calls: calls.clone(),
                asked: asked.clone(),
                keys: keys.to_vec(),
                fail: None,
            };
            (provider, calls, asked)
        }
    }

    impl Provider for Counting {
        fn id(&self) -> &str {
            "counting"
        }

        fn label(&self) -> &str {
            "Counting"
        }

        fn describe(&self) -> Vec<Descriptor> {
            self.keys
                .iter()
                .map(|key| Descriptor::new(*key, Some("%")))
                .collect()
        }

        fn sample(&self, keys: &[String], _now_ms: u64) -> Result<Vec<Reading>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.asked.lock().unwrap() = keys.to_vec();
            if let Some(message) = self.fail {
                return Err(message.to_owned());
            }
            Ok(keys
                .iter()
                .filter(|key| self.keys.contains(&key.as_str()))
                .map(|key| Reading::number(key.clone(), 42.0, "%"))
                .collect())
        }
    }

    fn ask(owner: &str, keys: &[&str]) -> Ask {
        Ask::new(owner, keys.iter().map(|key| (*key).to_owned()))
    }

    #[test]
    fn two_asks_cost_one_poll_of_the_union() {
        let (provider, calls, asked) = Counting::new(&["cpu.load", "ram.used"]);
        let mut registry = Registry::new();
        registry.add(provider);
        let cycle = registry.sample(
            &[
                ask("panel-a", &["cpu.load"]),
                ask("panel-b", &["ram.used", "cpu.load"]),
            ],
            1_000,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*asked.lock().unwrap(), vec!["cpu.load", "ram.used"]);
        assert_eq!(cycle.readings.len(), 2);
        assert!(cycle.unmapped.is_empty());
    }

    #[test]
    fn a_failing_provider_leaves_the_others_readings_alone() {
        let (mut broken, ..) = Counting::new(&["gpu.temp"]);
        broken.fail = Some("device\u{0007} exploded\nagain");
        let (working, ..) = Counting::new(&["cpu.load"]);
        let mut registry = Registry::new();
        registry.add(broken);
        registry.add(working);
        let cycle = registry.sample(&[ask("panel", &["gpu.temp", "cpu.load"])], 1_000);
        assert_eq!(cycle.readings.len(), 1);
        assert!(cycle.reading("cpu.load").is_some());
        assert_eq!(cycle.failures.len(), 1);
        assert_eq!(cycle.failures[0].message, "device exploded again");
        assert_eq!(cycle.unmapped, vec!["gpu.temp"]);
    }

    #[test]
    fn a_key_nothing_reads_is_unmapped_once_however_many_displays_ask() {
        let (provider, ..) = Counting::new(&["cpu.load"]);
        let mut registry = Registry::new();
        registry.add(provider);
        let cycle = registry.sample(
            &[
                ask("panel-a", &["custom.thing.one"]),
                ask("panel-b", &["custom.thing.one"]),
            ],
            1_000,
        );
        assert_eq!(cycle.unmapped, vec!["custom.thing.one"]);
    }

    #[test]
    fn a_reading_nobody_asked_for_is_dropped() {
        let (provider, ..) = Counting::new(&["cpu.load", "ram.used"]);
        let mut registry = Registry::new();
        registry.add(provider);
        let cycle = registry.sample(&[ask("panel", &["cpu.load"])], 1_000);
        assert_eq!(cycle.readings.len(), 1);
        assert!(cycle.reading("ram.used").is_none());
        assert!(cycle.unmapped.is_empty());
    }
}
