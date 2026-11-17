//! The readings each key measured over the last minutes.
//!
//! A line chart draws a window, not one instant, so a frame cannot answer it
//! alone. A surface records every poll here and hands the history to the plan;
//! a key is kept only as long as a chart could ask for it, and a key nothing
//! recorded has no series at all rather than a flat one.

use std::collections::{HashMap, VecDeque};

use yunari_providers::sample::Status;

use crate::reading::Readings;

/// How many samples one key keeps. At a 250 ms tick this is a little over four
/// minutes, longer than any window a chart may ask for (1 s–10 min).
pub const CAPACITY: usize = 1024;

/// One key's measured figures, oldest first, with the moment each was read.
#[derive(Debug, Clone, Default)]
pub struct History {
    by_key: HashMap<String, VecDeque<(u64, f64)>>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep every finite reading of this poll. A missing or failed reading is
    /// an absence: nothing is recorded for it, so the series has a gap rather
    /// than a zero.
    pub fn record(&mut self, at_ms: u64, readings: &Readings) {
        for (key, value) in readings.finite_values() {
            let samples = self.by_key.entry(key.to_owned()).or_default();
            samples.push_back((at_ms, value));
            while samples.len() > CAPACITY {
                samples.pop_front();
            }
        }
    }

    /// One key's samples inside `window_seconds` of `now_ms`, oldest first,
    /// each with the moment it was read. Samples older than the window are not
    /// the chart's to draw.
    pub fn series(&self, key: &str, window_seconds: f64, now_ms: u64) -> Vec<(u64, f64)> {
        let Some(samples) = self.by_key.get(key) else {
            return Vec::new();
        };
        let window_ms = (window_seconds.max(0.0) * 1000.0).round() as u64;
        let from = now_ms.saturating_sub(window_ms);
        samples
            .iter()
            .filter(|(at, _)| *at >= from && *at <= now_ms)
            .copied()
            .collect()
    }
}

impl Readings {
    /// The readings that measured an ok, finite figure, in no particular order.
    pub(crate) fn finite_values(&self) -> impl Iterator<Item = (&str, f64)> {
        self.iter().filter_map(|(key, reading)| {
            (reading.status == Status::Ok)
                .then_some(reading.value)
                .flatten()
                .filter(|value| value.is_finite())
                .map(|value| (key, value))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_providers::sample::Reading;

    fn readings(values: &[(&str, Option<f64>)]) -> Readings {
        Readings::new(values.iter().map(|(key, value)| Reading {
            semantic_key: (*key).to_owned(),
            status: if value.is_some() {
                Status::Ok
            } else {
                Status::Missing
            },
            value: *value,
            ..Reading::missing(*key, "test")
        }))
    }

    #[test]
    fn a_window_keeps_only_its_own_samples_oldest_first() {
        let mut history = History::new();
        for (at, value) in [(0, 1.0), (1_000, 2.0), (2_000, 3.0)] {
            history.record(at, &readings(&[("cpu.load", Some(value))]));
        }
        assert_eq!(
            history.series("cpu.load", 1.5, 2_000),
            vec![(1_000, 2.0), (2_000, 3.0)]
        );
        assert_eq!(
            history.series("cpu.load", 10.0, 2_000),
            vec![(0, 1.0), (1_000, 2.0), (2_000, 3.0)]
        );
    }

    #[test]
    fn a_missing_reading_is_a_gap_not_a_zero() {
        let mut history = History::new();
        history.record(0, &readings(&[("cpu.load", Some(4.0))]));
        history.record(1_000, &readings(&[("cpu.load", None)]));
        assert_eq!(history.series("cpu.load", 60.0, 1_000), vec![(0, 4.0)]);
        assert!(history.series("gpu.load", 60.0, 1_000).is_empty());
    }

    #[test]
    fn a_key_keeps_a_bounded_number_of_samples() {
        let mut history = History::new();
        for at in 0..(CAPACITY as u64 + 10) {
            history.record(at, &readings(&[("cpu.load", Some(at as f64))]));
        }
        let series = history.series("cpu.load", 1_000_000.0, CAPACITY as u64 + 9);
        assert_eq!(series.len(), CAPACITY);
        assert_eq!(series[0], (10, 10.0));
    }
}
