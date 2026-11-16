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
    /// Which card and drive answer the unsuffixed keys. Empty until the host
    /// hands this provider the settings file.
    devices: crate::devices::Loader,
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
            devices: std::sync::Arc::new(crate::devices::Assignment::default),
            failure: std::sync::Mutex::new(None),
        }
    }

    pub fn with_devices(mut self, devices: crate::devices::Loader) -> Self {
        self.devices = devices;
        self
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

        let assignment = (self.devices)();
        Ok(owned
            .into_iter()
            .map(|key| reading_of(&sensors, key, &assignment))
            .collect())
    }

    fn devices(&self) -> Vec<crate::provider::Device> {
        let Ok(sensors) = self.sensors() else {
            return Vec::new();
        };
        let assignment = (self.devices)();
        let mut devices = Vec::new();
        for (group, key) in [
            (Group::Gpu, "gpu"),
            (Group::Storage, "system-disk"),
            (Group::Storage, "data-disk"),
        ] {
            devices.extend(
                group_devices(&sensors, group, &assignment)
                    .into_iter()
                    .map(|node| crate::provider::Device {
                        group: key.to_owned(),
                        id: node.id,
                        name: node.name,
                    }),
            );
        }
        devices
    }

    fn health(&self) -> Health {
        match self.failure.lock().expect("the LHM failure").clone() {
            Some(reason) => Health::Unavailable(reason),
            None => Health::Ok,
        }
    }
}

/// One key's reading from an already-flattened tree.
fn reading_of(sensors: &[LhmSensor], key: &str, devices: &crate::devices::Assignment) -> Reading {
    let group = if key.starts_with("gpu.") || key.starts_with("vram.") {
        Group::Gpu
    } else if key.starts_with("disk.") {
        Group::Storage
    } else {
        return numeric(sensors, key, None);
    };
    let assigned = match (group, key.starts_with("disk.data.")) {
        (Group::Gpu, _) => devices.gpu.as_deref(),
        // The second slot answers only once a drive is assigned to it.
        (_, true) => devices.data_disk.as_deref(),
        (_, false) => devices.system_disk.as_deref(),
    };
    let lookup = device_of(sensors, group, assigned, devices);
    if key.starts_with("disk.data.") && assigned.is_none() {
        return Reading::missing(key, "Assign a data disk in Settings.");
    }
    if key == "gpu.name" || key == "disk.name" {
        return match lookup {
            Lookup::Found(device) => Reading::text(key, device.name),
            Lookup::Gone => Reading::missing(key, assigned_gone(group)),
            Lookup::NoneDetected => Reading::missing(
                key,
                match group {
                    Group::Storage if assigned.is_none() => {
                        "Assign a system disk in Settings so a drive can be named."
                    }
                    Group::Storage => "LibreHardwareMonitor reports no drive.",
                    _ => "LibreHardwareMonitor reports no GPU.",
                },
            ),
        };
    }
    let node = match lookup {
        Lookup::Found(device) => Some(device.hardware_id),
        Lookup::Gone => return Reading::missing(key, assigned_gone(group)),
        // An unassigned storage group is the machine's total across drives — a
        // legitimate figure no single drive's name describes. An unassigned GPU
        // is still one card, and nothing has to be scoped for that.
        Lookup::NoneDetected => None,
    };
    numeric(sensors, key, node.as_deref())
}

fn assigned_gone(group: Group) -> String {
    match group {
        Group::Storage => "The assigned drive is not present on this machine.".to_owned(),
        _ => "The assigned GPU is not present on this machine.".to_owned(),
    }
}

/// One key's number, from the scoped sensors.
fn numeric(sensors: &[LhmSensor], key: &str, node: Option<&str>) -> Reading {
    let Some(value) = bindings::read_in(sensors, key, node) else {
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

/// A hardware node the machine reports.
struct Found {
    hardware_id: String,
    name: String,
}

enum Lookup {
    Found(Found),
    /// This machine reports no device of that group at all.
    NoneDetected,
    /// An assignment names a device the machine no longer reports: a gap, never
    /// another device's figures.
    Gone,
}

/// The hardware node a group's unsuffixed keys describe: the assigned one when
/// the settings name it, else the first the machine reports — never "the
/// busiest", because a caption that changes text every sample cannot be read.
fn device_of(
    sensors: &[LhmSensor],
    group: Group,
    assigned: Option<&str>,
    devices: &crate::devices::Assignment,
) -> Lookup {
    let nodes = group_devices(sensors, group, devices);
    if nodes.is_empty() {
        return Lookup::NoneDetected;
    }
    let chosen = match assigned {
        Some(assigned) => match nodes.iter().position(|node| node.id == assigned) {
            Some(index) => index,
            None => return Lookup::Gone,
        },
        None => 0,
    };
    let node = &nodes[chosen];
    Lookup::Found(Found {
        hardware_id: node.hardware_id.clone(),
        name: node.name.clone(),
    })
}

/// The hardware nodes of one group, in the order the machine reports them.
fn group_devices(
    sensors: &[LhmSensor],
    group: Group,
    devices: &crate::devices::Assignment,
) -> Vec<NodeInfo> {
    let mut names: Vec<(String, String)> = Vec::new();
    for sensor in sensors {
        if group_of(&sensor.hardware_name, &sensor.hardware_id) != Some(group) {
            continue;
        }
        if !names
            .iter()
            .any(|(hardware_id, _)| hardware_id == &sensor.hardware_id)
        {
            names.push((sensor.hardware_id.clone(), sensor.hardware_name.clone()));
        }
    }
    crate::devices::ids_for(names.iter().map(|(_, name)| name.clone()))
        .into_iter()
        .zip(names)
        .map(|((_, id), (hardware_id, detected))| NodeInfo {
            hardware_id,
            name: devices.name_of(&id, &detected).to_owned(),
            id,
        })
        .collect()
}

struct NodeInfo {
    hardware_id: String,
    id: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
    fn the_devices_a_person_can_assign_are_named_by_their_id() {
        let running = provider(payload);
        let devices = running.devices();
        assert_eq!(
            devices
                .iter()
                .find(|device| device.group == "gpu")
                .map(|device| device.id.as_str()),
            Some("nvidia-geforce-rtx-4090")
        );
        // An unreachable server offers no device rather than a guess.
        assert!(provider(|| Err("refused".to_owned())).devices().is_empty());
    }

    #[test]
    fn a_name_key_reads_the_devices_own_name() {
        let provider = provider(payload);
        let readings = provider
            .sample(&["gpu.name".to_owned(), "disk.name".to_owned()], 0)
            .expect("a cycle");
        assert_eq!(readings[0].text.as_deref(), Some("NVIDIA GeForce RTX 4090"));
        // A drive's name follows its assignment; an unassigned storage group is
        // a total across drives, which no drive's name describes.
        assert_eq!(readings[1].status, Status::Missing);
    }

    #[test]
    fn an_assigned_card_and_drive_answer_the_unsuffixed_keys() {
        let body = || {
            Ok(serde_json::json!({
                "Children": [{
                    "Text": "NVIDIA GeForce RTX 4090",
                    "HardwareId": "/gpu-nvidia/0",
                    "Children": [{
                        "Text": "GPU Core", "SensorId": "/gpu-nvidia/0/load/0",
                        "Type": "Load", "RawValue": "45.0 %"
                    }]
                }, {
                    "Text": "AMD Radeon RX 6800",
                    "HardwareId": "/gpu-amd/0",
                    "Children": [{
                        "Text": "GPU Core", "SensorId": "/gpu-amd/0/load/0",
                        "Type": "Load", "RawValue": "10.0 %"
                    }]
                }, {
                    "Text": "Samsung SSD 990 PRO",
                    "HardwareId": "/hdd/0",
                    "Children": [{
                        "Text": "Total Space", "SensorId": "/hdd/0/data/0",
                        "Type": "Data", "RawValue": "1000.0 GB"
                    }, {
                        "Text": "Free Space", "SensorId": "/hdd/0/data/1",
                        "Type": "Data", "RawValue": "400.0 GB"
                    }]
                }]
            })
            .to_string())
        };
        let assignment = crate::devices::Assignment {
            gpu: Some("amd-radeon-rx-6800".to_owned()),
            system_disk: Some("samsung-ssd-990-pro".to_owned()),
            names: [("amd-radeon-rx-6800".to_owned(), "Gaming".to_owned())]
                .into_iter()
                .collect(),
            ..crate::devices::Assignment::default()
        };
        let provider = provider(body).with_devices(Arc::new(move || assignment.clone()));
        let readings = provider
            .sample(
                &[
                    "gpu.load".to_owned(),
                    "gpu.name".to_owned(),
                    "disk.used".to_owned(),
                    "disk.name".to_owned(),
                ],
                0,
            )
            .expect("a cycle");
        assert_eq!(readings[0].value, Some(10.0));
        assert_eq!(readings[1].text.as_deref(), Some("Gaming"));
        assert_eq!(readings[2].value, Some(600.0));
        assert_eq!(readings[3].text.as_deref(), Some("Samsung SSD 990 PRO"));
    }

    #[test]
    fn an_assigned_device_the_machine_lost_is_a_gap_not_another_device() {
        let assignment = crate::devices::Assignment {
            gpu: Some("a-card-this-pc-does-not-have".to_owned()),
            ..crate::devices::Assignment::default()
        };
        let provider = provider(payload).with_devices(Arc::new(move || assignment.clone()));
        let readings = provider
            .sample(&["gpu.load".to_owned(), "gpu.name".to_owned()], 0)
            .expect("a cycle");
        assert_eq!(readings[0].status, Status::Missing);
        assert_eq!(readings[1].status, Status::Missing);
    }
}
