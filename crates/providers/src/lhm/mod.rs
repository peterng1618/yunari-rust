//! LibreHardwareMonitor as an optional external program: this reads the JSON
//! its web server publishes and never links or compiles its .NET library.
//!
//! When LHM is absent or its server is off, every key it owns is `missing` with
//! a reason, so the registry can fall back and a display can say why the gap is
//! there.

pub mod bindings;
pub mod tree;

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;
use crate::vocabulary;
use bindings::{Group, group_of};
use tree::LhmSensor;

/// LHM's web server default; `--lhm-url` overrides it.
pub const DEFAULT_URL: &str = "http://127.0.0.1:8085";
/// Bounded so a hung LHM cannot stall the host's poll cycle.
const TIMEOUT_MS: u64 = 2_000;

/// Every key this provider can answer, whether or not a machine has the sensor.
pub const KEYS: &[&str] = &[
    "disk.data.used",
    "disk.data.used.percent",
    "disk.data.total",
    "cpu.temp",
    "cpu.temp.package",
    "cpu.temp.core.max",
    "cpu.temp.core.avg",
    "cpu.power",
    "cpu.voltage",
    "cpu.clock",
    "cpu.fan",
    "motherboard.temp",
    "cooling.temp",
    "psu.power",
    "psu.temp",
    "psu.fan",
    "battery.level",
    "battery.voltage",
    "battery.rate",
    "battery.timeRemaining",
    "gpu.load",
    "gpu.temp",
    "gpu.temp.core",
    "gpu.temp.hotspot",
    "gpu.power",
    "gpu.voltage",
    "gpu.clock",
    "gpu.clock.memory",
    "gpu.fan",
    "gpu.name",
    "vram.used",
    "vram.used.percent",
    "vram.total",
    "vram.temp",
    "disk.used",
    "disk.used.percent",
    "disk.total",
    "disk.name",
    "network.download",
    "network.upload",
];

/// How one GET is performed. A seam so the tests need no server.
pub type Fetch = Box<dyn Fn(&str, u64) -> Result<String, String> + Send + Sync>;

pub struct LhmProvider {
    base_url: String,
    fetch: Fetch,
    failure: std::sync::Mutex<Option<String>>,
}

impl LhmProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_fetch(base_url, Box::new(crate::http::get))
    }

    pub fn with_fetch(base_url: impl Into<String>, fetch: Fetch) -> Self {
        Self {
            base_url: base_url.into(),
            fetch,
            failure: std::sync::Mutex::new(None),
        }
    }

    fn sensors(&self) -> Result<Vec<LhmSensor>, String> {
        let url = format!("{}/data.json", self.base_url.trim_end_matches('/'));
        let body = (self.fetch)(&url, TIMEOUT_MS)?;
        let payload: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| format!("its answer is not JSON: {error}"))?;
        Ok(tree::flatten(&payload))
    }
}

impl Provider for LhmProvider {
    fn id(&self) -> &str {
        "lhm"
    }

    fn label(&self) -> &str {
        "LibreHardwareMonitor"
    }

    fn describe(&self) -> Vec<Descriptor> {
        KEYS.iter()
            .map(|key| {
                Descriptor::new(*key, vocabulary::describe(key).and_then(|entry| entry.unit))
            })
            .collect()
    }

    fn sample(&self, keys: &[String], _now_ms: u64) -> Result<Vec<Reading>, String> {
        let owned: Vec<&String> = keys
            .iter()
            .filter(|key| KEYS.contains(&key.as_str()))
            .collect();
        let sensors = match self.sensors() {
            Ok(sensors) => {
                *self.failure.lock().expect("the LHM failure") = None;
                sensors
            }
            Err(error) => {
                let reason = format!(
                    "LibreHardwareMonitor is not reachable at its configured address: {error}"
                );
                *self.failure.lock().expect("the LHM failure") = Some(reason.clone());
                return Ok(owned
                    .into_iter()
                    .map(|key| Reading::missing(key.clone(), reason.clone()))
                    .collect());
            }
        };

        Ok(owned
            .into_iter()
            .map(|key| reading_of(&sensors, key))
            .collect())
    }

    fn health(&self) -> Health {
        match self.failure.lock().expect("the LHM failure").clone() {
            Some(reason) => Health::Unavailable(reason),
            None => Health::Ok,
        }
    }
}

/// One key's reading from an already-flattened tree.
fn reading_of(sensors: &[LhmSensor], key: &str) -> Reading {
    if key == "gpu.name" {
        return gpu_name(sensors).map_or_else(
            || Reading::missing(key, "LibreHardwareMonitor reports no GPU."),
            |name| Reading::text(key, name),
        );
    }
    if key == "disk.name" {
        return disk_name(sensors).map_or_else(
            || Reading::missing(key, "LibreHardwareMonitor reports no drive."),
            |name| Reading::text(key, name),
        );
    }

    let Some(value) = bindings::read(sensors, key) else {
        return Reading::missing(
            key,
            format!("LibreHardwareMonitor has no sensor for {key} on this machine."),
        );
    };
    let unit = vocabulary::describe(key)
        .and_then(|entry| entry.unit)
        .unwrap_or_default();
    Reading::number(key, round(value), unit)
}

/// Two decimals is what a display shows; the extra digits are noise from a
/// sensor that reports tenths.
fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn gpu_name(sensors: &[LhmSensor]) -> Option<String> {
    sensors
        .iter()
        .find(|sensor| group_of(&sensor.hardware_name, &sensor.hardware_id) == Some(Group::Gpu))
        .map(|sensor| sensor.hardware_name.clone())
}

fn disk_name(sensors: &[LhmSensor]) -> Option<String> {
    sensors
        .iter()
        .find(|sensor| group_of(&sensor.hardware_name, &sensor.hardware_id) == Some(Group::Storage))
        .map(|sensor| sensor.hardware_name.clone())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;
    use crate::sample::Status;

    fn payload() -> Result<String, String> {
        Ok(serde_json::json!({
            "Children": [{
                "Text": "AMD Ryzen 9 5950X",
                "HardwareId": "/amdcpu/0",
                "Children": [{
                    "Text": "CPU Package",
                    "SensorId": "/amdcpu/0/temperature/0",
                    "Type": "Temperature",
                    "RawValue": "64.0 °C"
                }, {
                    "Text": "CPU Total",
                    "SensorId": "/amdcpu/0/load/0",
                    "Type": "Load",
                    "RawValue": "12.5 %"
                }]
            }, {
                "Text": "NVIDIA GeForce RTX 4090",
                "HardwareId": "/gpu-nvidia/0",
                "Children": [{
                    "Text": "GPU Core",
                    "SensorId": "/gpu-nvidia/0/load/0",
                    "Type": "Load",
                    "RawValue": "45.0 %"
                }]
            }, {
                "Text": "Corsair HX1000i",
                "HardwareId": "/psu/0",
                "Children": [{
                    "Text": "Total watts",
                    "SensorId": "/psu/0/power/0",
                    "Type": "Power",
                    "RawValue": "212.0 W"
                }]
            }]
        })
        .to_string())
    }

    fn provider(body: impl Fn() -> Result<String, String> + Send + Sync + 'static) -> LhmProvider {
        let body = Arc::new(body);
        LhmProvider::with_fetch(DEFAULT_URL, Box::new(move |_url, _timeout| body()))
    }

    #[test]
    fn a_running_server_answers_its_keys() {
        let provider = provider(payload);
        let keys = vec![
            "cpu.temp".to_owned(),
            "gpu.load".to_owned(),
            "psu.power".to_owned(),
            "cpu.fan".to_owned(),
        ];
        let readings = provider.sample(&keys, 0).expect("a cycle");
        assert_eq!(readings.len(), 4);
        assert_eq!(readings[0].value, Some(64.0));
        assert_eq!(readings[0].unit.as_deref(), Some("°C"));
        assert_eq!(readings[1].value, Some(45.0));
        assert_eq!(readings[2].value, Some(212.0));
        // No fan on this machine is a gap, and the gap says why.
        assert_eq!(readings[3].status, Status::Missing);
        assert!(
            readings[3]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("no sensor"))
        );
    }

    #[test]
    fn an_unreachable_server_is_ordinary_and_says_what_to_do() {
        let provider = provider(|| Err("connection refused".to_owned()));
        let keys = vec!["cpu.temp".to_owned()];
        let readings = provider.sample(&keys, 0).expect("a cycle, not an error");
        assert_eq!(readings[0].status, Status::Missing);
        assert!(
            readings[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("not reachable"))
        );
        assert!(matches!(provider.health(), Health::Unavailable(_)));
    }

    #[test]
    fn only_the_keys_asked_for_are_read() {
        let provider = provider(payload);
        let readings = provider
            .sample(&["cpu.temp".to_owned(), "weather.temp".to_owned()], 0)
            .expect("a cycle");
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].semantic_key, "cpu.temp");
    }

    #[test]
    fn a_name_key_reads_the_devices_own_name() {
        let provider = provider(payload);
        let readings = provider
            .sample(&["gpu.name".to_owned(), "disk.name".to_owned()], 0)
            .expect("a cycle");
        assert_eq!(readings[0].text.as_deref(), Some("NVIDIA GeForce RTX 4090"));
        assert_eq!(readings[1].status, Status::Missing);
        let _ = Mutex::new(());
    }
}
