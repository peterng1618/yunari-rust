//! Time and date as readings.
//!
//! What this sends is an instant, not a finished string: how a clock reads is
//! design, so the binding's format decides it. Deriving the time in a display
//! would let two panels disagree, and a phone with a wrong clock would show the
//! wrong time, so the reading comes from this PC.

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::provider::{Descriptor, Provider};
use crate::sample::Reading;

const CLOCK_KEYS: [&str; 2] = ["time.now", "date.today"];

/// The clock and the date, read in one zone.
#[derive(Debug, Default)]
pub struct ClockProvider {
    zone: Option<String>,
}

impl ClockProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read instants in `zone` (an IANA name) rather than this machine's zone.
    pub fn with_zone(zone: impl Into<String>) -> Self {
        Self {
            zone: Some(zone.into()),
        }
    }

    pub fn set_zone(&mut self, zone: Option<impl Into<String>>) {
        self.zone = zone.map(Into::into);
    }

    /// The zone this PC's time is read in. An unknown name falls back to the
    /// machine's zone rather than failing the poll.
    fn zone(&self) -> TimeZone {
        self.zone
            .as_deref()
            .and_then(|name| TimeZone::get(name).ok())
            .unwrap_or_else(TimeZone::system)
    }
}

impl Provider for ClockProvider {
    fn id(&self) -> &str {
        "clock"
    }

    fn label(&self) -> &str {
        "Clock (baseline)"
    }

    fn describe(&self) -> Vec<Descriptor> {
        CLOCK_KEYS
            .iter()
            .map(|key| Descriptor::new(*key, None))
            .collect()
    }

    fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String> {
        let wanted: Vec<&String> = keys
            .iter()
            .filter(|key| CLOCK_KEYS.contains(&key.as_str()))
            .collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let now_ms = i64::try_from(now_ms).map_err(|_| "instant is out of range".to_owned())?;
        let instant = Timestamp::from_millisecond(now_ms)
            .map_err(|error| format!("instant is out of range: {error}"))?;
        // The offset travels with the value, so a display needs no zone
        // knowledge and never re-converts.
        let iso = instant
            .to_zoned(self.zone())
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        // Both keys carry the same instant; the binding's format and zone
        // decide what each shows.
        Ok(wanted
            .into_iter()
            .map(|key| Reading::instant(key.clone(), iso.clone()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Health;

    /// 2026-09-24 14:07:09 UTC, a Thursday.
    const INSTANT_MS: u64 = 1_790_258_829_000;

    #[test]
    fn the_descriptors_are_the_vocabularys_time_keys() {
        let described = ClockProvider::new().describe();
        let keys: Vec<&str> = described.iter().map(|d| d.semantic_key.as_str()).collect();
        assert_eq!(keys, CLOCK_KEYS);
        assert!(described.iter().all(|d| d.unit.is_none()));
    }

    #[test]
    fn an_instant_carries_the_zone_it_was_read_in() {
        let provider = ClockProvider::with_zone("Asia/Tokyo");
        let readings = provider
            .sample(&["time.now".to_owned()], INSTANT_MS)
            .expect("a readable instant");
        assert_eq!(readings.len(), 1);
        assert_eq!(
            readings[0].instant.as_deref(),
            Some("2026-09-24T23:07:09+09:00")
        );
        assert_eq!(readings[0].status, crate::sample::Status::Ok);
    }

    #[test]
    fn the_date_and_the_clock_are_one_instant() {
        let readings = ClockProvider::with_zone("UTC")
            .sample(&["time.now".to_owned(), "date.today".to_owned()], INSTANT_MS)
            .expect("readable instants");
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].instant, readings[1].instant);
    }

    #[test]
    fn an_unknown_zone_falls_back_to_this_machine_rather_than_failing() {
        let readings = ClockProvider::with_zone("Mars/Olympus_Mons")
            .sample(&["time.now".to_owned()], INSTANT_MS)
            .expect("the local zone still answers");
        assert_eq!(readings.len(), 1);
        assert!(readings[0].instant.is_some());
    }

    #[test]
    fn only_the_requested_keys_are_answered() {
        let readings = ClockProvider::new()
            .sample(&["cpu.load".to_owned()], INSTANT_MS)
            .expect("nothing to fail");
        assert!(readings.is_empty());
    }

    #[test]
    fn health_is_always_ok() {
        assert_eq!(ClockProvider::new().health(), Health::Ok);
    }
}
