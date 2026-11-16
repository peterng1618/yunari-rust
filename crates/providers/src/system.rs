//! Baseline readings: CPU, memory, disks and network, from this machine.
//!
//! Everything the operating system tells us goes through [`Snapshot`], so the
//! keys this provider offers can be tested without depending on the machine the
//! test runs on.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::provider::{Descriptor, Provider};
use crate::sample::Reading;
use crate::vocabulary::semantic_id;

const GIB: f64 = 1_073_741_824.0;

/// What one poll of the operating system saw.
pub trait Snapshot: Send + Sync {
    fn cpu_load(&self) -> Option<f64>;
    fn memory(&self) -> Option<Memory>;
    fn disks(&self) -> Vec<Disk>;
    fn networks(&self) -> Vec<Network>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Memory {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub name: String,
    /// Cumulative counters since boot; a rate is their difference over time.
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
}

/// The machine this app runs on.
pub struct SysinfoSnapshot {
    // `None` when the operating system refused the first query. That is a
    // machine with nothing to report, not a reason to stop the app.
    state: Mutex<Option<sysinfo::System>>,
    disks: Mutex<Option<sysinfo::Disks>>,
    networks: Mutex<Option<sysinfo::Networks>>,
}

impl SysinfoSnapshot {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(sysinfo::System::new().ok()),
            disks: Mutex::new(sysinfo::Disks::new_with_refreshed_list().ok()),
            networks: Mutex::new(sysinfo::Networks::new_with_refreshed_list().ok()),
        }
    }
}

impl Default for SysinfoSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot for SysinfoSnapshot {
    fn cpu_load(&self) -> Option<f64> {
        let mut guard = self.state.lock().ok()?;
        let system = guard.as_mut()?;
        // The second refresh is what gives sysinfo a window to measure across.
        system.refresh_cpu_usage();
        Some(f64::from(system.global_cpu_usage()))
    }

    fn memory(&self) -> Option<Memory> {
        let mut guard = self.state.lock().ok()?;
        let system = guard.as_mut()?;
        system.refresh_memory();
        let total_bytes = system.total_memory();
        (total_bytes > 0).then_some(Memory {
            used_bytes: system.used_memory(),
            total_bytes,
        })
    }

    fn disks(&self) -> Vec<Disk> {
        let Ok(mut guard) = self.disks.lock() else {
            return Vec::new();
        };
        let Some(disks) = guard.as_mut() else {
            return Vec::new();
        };
        disks.refresh(true);
        disks
            .iter()
            .map(|disk| {
                let total_bytes = disk.total_space();
                Disk {
                    name: disk.name().to_string_lossy().into_owned(),
                    used_bytes: total_bytes.saturating_sub(disk.available_space()),
                    total_bytes,
                }
            })
            .collect()
    }

    fn networks(&self) -> Vec<Network> {
        let Ok(mut guard) = self.networks.lock() else {
            return Vec::new();
        };
        let Some(networks) = guard.as_mut() else {
            return Vec::new();
        };
        networks.refresh(true);
        networks
            .iter()
            .map(|(name, data)| Network {
                name: name.to_string_lossy().into_owned(),
                received_bytes: data.total_received(),
                transmitted_bytes: data.total_transmitted(),
            })
            .collect()
    }
}

/// The previous cumulative counters, so a rate can be derived on the next poll.
#[derive(Debug, Clone, Copy)]
struct Counters {
    received_bytes: u64,
    transmitted_bytes: u64,
    at_ms: u64,
}

/// CPU, memory, disks and network, in the units the vocabulary declares.
pub struct SystemProvider {
    snapshot: Box<dyn Snapshot>,
    previous: Mutex<BTreeMap<String, Counters>>,
    /// Which drive answers the abstract `disk.*` slots. Empty until the host
    /// hands this provider the settings file.
    devices: crate::devices::Loader,
}

impl SystemProvider {
    pub fn new() -> Self {
        Self {
            snapshot: Box::new(SysinfoSnapshot::new()),
            previous: Mutex::new(BTreeMap::new()),
            devices: std::sync::Arc::new(crate::devices::Assignment::default),
        }
    }

    pub fn with_snapshot(snapshot: impl Snapshot + 'static) -> Self {
        Self {
            snapshot: Box::new(snapshot),
            previous: Mutex::new(BTreeMap::new()),
            devices: std::sync::Arc::new(crate::devices::Assignment::default),
        }
    }

    pub fn with_devices(mut self, devices: crate::devices::Loader) -> Self {
        self.devices = devices;
        self
    }
}

/// The keys the given devices offer.
fn offered_keys(disks: &[Disk], networks: &[Network]) -> Vec<(Descriptor, Key)> {
    {
        let mut keys = vec![
            (Descriptor::new("cpu.load", Some("%")), Key::CpuLoad),
            (Descriptor::new("ram.used", Some("GB")), Key::RamUsed),
            (Descriptor::new("ram.total", Some("GB")), Key::RamTotal),
            (
                Descriptor::new("ram.used.percent", Some("%")),
                Key::RamPercent,
            ),
        ];
        // The abstract disk slots: a theme asks `disk.used` and the host says
        // which drive that is. They exist whenever any drive does, so a theme
        // written on one PC reads on another.
        if !disks.is_empty() {
            keys.push((Descriptor::new("disk.name", None), Key::DiskName));
            for (slot, prefix) in [(Slot::System, "disk"), (Slot::Data, "disk.data")] {
                keys.push((
                    Descriptor::new(format!("{prefix}.used"), Some("GB")),
                    Key::DiskSlot {
                        slot,
                        quantity: "used".to_owned(),
                    },
                ));
                keys.push((
                    Descriptor::new(format!("{prefix}.total"), Some("GB")),
                    Key::DiskSlot {
                        slot,
                        quantity: "total".to_owned(),
                    },
                ));
                keys.push((
                    Descriptor::new(format!("{prefix}.used.percent"), Some("%")),
                    Key::DiskSlot {
                        slot,
                        quantity: "percent".to_owned(),
                    },
                ));
            }
        }
        for disk in disks {
            let slug = semantic_id(&disk.name);
            if slug.is_empty() {
                continue;
            }
            for quantity in ["used", "total", "free"] {
                keys.push((
                    Descriptor::new(format!("disk.{slug}.{quantity}"), Some("GB")),
                    Key::Disk {
                        name: disk.name.clone(),
                        quantity: quantity.to_owned(),
                    },
                ));
            }
        }
        for network in networks {
            let slug = semantic_id(&network.name);
            if slug.is_empty() {
                continue;
            }
            for direction in ["download", "upload"] {
                keys.push((
                    Descriptor::new(format!("network.{slug}.{direction}"), Some("Mb/s")),
                    Key::Network {
                        name: network.name.clone(),
                        download: direction == "download",
                    },
                ));
            }
        }
        keys
    }
}

impl SystemProvider {
    /// Every key this machine offers, from one snapshot.
    fn offered(&self) -> Vec<(Descriptor, Key)> {
        offered_keys(&self.snapshot.disks(), &self.snapshot.networks())
    }
}

impl Default for SystemProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// What a key reads, so one snapshot can answer every key it offered.
enum Key {
    CpuLoad,
    RamUsed,
    RamTotal,
    RamPercent,
    Disk {
        name: String,
        quantity: String,
    },
    /// One of the abstract slots a theme binds without knowing this machine's
    /// drives: the assigned drive, or the first one reported.
    DiskSlot {
        slot: Slot,
        quantity: String,
    },
    DiskName,
    Network {
        name: String,
        download: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    System,
    Data,
}

/// The drive an abstract slot answers with, and the id the settings name it by.
/// The system slot falls back to the first drive reported, the way a theme with
/// no choice shows the machine's main disk; the data slot answers only once a
/// drive is assigned to it.
fn slot_disk<'a>(
    disks: &'a [Disk],
    devices: &crate::devices::Assignment,
    slot: Slot,
) -> Option<(String, &'a Disk)> {
    let listed: Vec<(String, String)> =
        crate::devices::ids_for(disks.iter().map(|disk| disk.name.clone()));
    let ids: Vec<String> = listed.iter().map(|(_, id)| id.clone()).collect();
    let names: Vec<String> = listed.iter().map(|(name, _)| name.clone()).collect();
    let assigned = match slot {
        Slot::System => devices.system_disk.as_deref(),
        Slot::Data => Some(devices.data_disk.as_deref()?),
    };
    let index = match assigned {
        Some(assigned) => crate::devices::index_of(&ids, &names, devices, assigned)?,
        None => 0,
    };
    Some((ids.get(index)?.clone(), disks.get(index)?))
}

impl Provider for SystemProvider {
    fn id(&self) -> &str {
        "system"
    }

    fn label(&self) -> &str {
        "System (baseline)"
    }

    fn describe(&self) -> Vec<Descriptor> {
        self.offered()
            .into_iter()
            .map(|(descriptor, _)| descriptor)
            .collect()
    }

    fn devices(&self) -> Vec<crate::provider::Device> {
        crate::devices::ids_for(self.snapshot.disks().into_iter().map(|disk| disk.name))
            .into_iter()
            .flat_map(|(name, id)| {
                ["system-disk", "data-disk"].map(|group| crate::provider::Device {
                    group: group.to_owned(),
                    id: id.clone(),
                    name: name.clone(),
                })
            })
            .collect()
    }

    fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String> {
        // One snapshot answers every key: the devices this poll sees decide
        // which keys exist at all.
        let memory = self.snapshot.memory();
        let disks = self.snapshot.disks();
        let networks = self.snapshot.networks();
        let offered: BTreeMap<String, Key> = offered_keys(&disks, &networks)
            .into_iter()
            .map(|(descriptor, key)| (descriptor.semantic_key, key))
            .collect();
        let wanted: Vec<(&String, &Key)> = keys
            .iter()
            .filter_map(|key| offered.get_key_value(key))
            .collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let mut readings = Vec::with_capacity(wanted.len());
        let devices = (self.devices)();
        for (semantic_key, key) in wanted {
            let reading = match key {
                Key::DiskSlot { slot, quantity } => match slot_disk(&disks, &devices, *slot) {
                    Some((_, disk)) => {
                        let percent = |disk: &Disk| {
                            if disk.total_bytes == 0 {
                                0.0
                            } else {
                                disk.used_bytes as f64 / disk.total_bytes as f64 * 100.0
                            }
                        };
                        match quantity.as_str() {
                            "used" => Reading::number(
                                semantic_key.clone(),
                                disk.used_bytes as f64 / GIB,
                                "GB",
                            ),
                            "total" => Reading::number(
                                semantic_key.clone(),
                                disk.total_bytes as f64 / GIB,
                                "GB",
                            ),
                            _ => Reading::number(semantic_key.clone(), percent(disk), "%"),
                        }
                    }
                    None if *slot == Slot::Data => {
                        Reading::missing(semantic_key.clone(), "Assign a data disk in Settings.")
                    }
                    None => Reading::missing(semantic_key.clone(), "this machine reports no disk"),
                },
                Key::DiskName => match slot_disk(&disks, &devices, Slot::System) {
                    Some((id, disk)) => Reading::text(
                        semantic_key.clone(),
                        devices.name_of(&id, &disk.name).to_owned(),
                    ),
                    None => Reading::missing(semantic_key.clone(), "this machine reports no disk"),
                },
                Key::CpuLoad => match self.snapshot.cpu_load() {
                    Some(load) => Reading::number(semantic_key.clone(), load, "%"),
                    None => {
                        Reading::missing(semantic_key.clone(), "the CPU load could not be read")
                    }
                },
                Key::RamUsed | Key::RamTotal | Key::RamPercent => match memory {
                    Some(memory) => match key {
                        Key::RamTotal => Reading::number(
                            semantic_key.clone(),
                            memory.total_bytes as f64 / GIB,
                            "GB",
                        ),
                        Key::RamUsed => Reading::number(
                            semantic_key.clone(),
                            memory.used_bytes as f64 / GIB,
                            "GB",
                        ),
                        _ => Reading::number(
                            semantic_key.clone(),
                            memory.used_bytes as f64 / memory.total_bytes as f64 * 100.0,
                            "%",
                        ),
                    },
                    None => Reading::missing(semantic_key.clone(), "memory could not be read"),
                },
                Key::Disk { name, quantity } => {
                    match disks.iter().find(|disk| &disk.name == name) {
                        Some(disk) => {
                            let gib = match quantity.as_str() {
                                "used" => disk.used_bytes as f64 / GIB,
                                "total" => disk.total_bytes as f64 / GIB,
                                _ => disk.total_bytes.saturating_sub(disk.used_bytes) as f64 / GIB,
                            };
                            Reading::number(semantic_key.clone(), gib, "GB")
                        }
                        None => Reading::missing(
                            semantic_key.clone(),
                            format!("disk {name} is no longer present"),
                        ),
                    }
                }
                Key::Network { name, download } => {
                    match self.rate(name, *download, &networks, now_ms) {
                        Some(mbps) => Reading::number(semantic_key.clone(), mbps, "Mb/s"),
                        None => {
                            Reading::missing(semantic_key.clone(), "a rate needs a second sample")
                        }
                    }
                }
            };
            readings.push(reading);
        }
        Ok(readings)
    }
}

impl SystemProvider {
    /// Megabits per second since the last poll, or `None` while there is no
    /// previous counter to compare against.
    fn rate(&self, name: &str, download: bool, networks: &[Network], now_ms: u64) -> Option<f64> {
        let network = networks.iter().find(|network| network.name == name)?;
        let mut previous = self.previous.lock().ok()?;
        let current = Counters {
            received_bytes: network.received_bytes,
            transmitted_bytes: network.transmitted_bytes,
            at_ms: now_ms,
        };
        let last = previous.insert(name.to_owned(), current)?;
        let elapsed_ms = now_ms.checked_sub(last.at_ms)?;
        if elapsed_ms == 0 {
            return None;
        }
        // A counter that went backwards means the interface restarted; that
        // tick has no rate rather than a fabricated one.
        let (now, before) = if download {
            (current.received_bytes, last.received_bytes)
        } else {
            (current.transmitted_bytes, last.transmitted_bytes)
        };
        let delta = now.checked_sub(before)?;
        Some(delta as f64 * 8.0 / 1_000_000.0 / (elapsed_ms as f64 / 1000.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct Fixed {
        cpu: Option<f64>,
        memory: Option<Memory>,
        disks: Vec<Disk>,
        networks: Vec<Network>,
    }

    impl Snapshot for Fixed {
        fn cpu_load(&self) -> Option<f64> {
            self.cpu
        }

        fn memory(&self) -> Option<Memory> {
            self.memory
        }

        fn disks(&self) -> Vec<Disk> {
            self.disks.clone()
        }

        fn networks(&self) -> Vec<Network> {
            self.networks.clone()
        }
    }

    fn two_disks() -> Fixed {
        Fixed {
            cpu: Some(12.5),
            memory: Some(Memory {
                used_bytes: 8 * 1_073_741_824,
                total_bytes: 16 * 1_073_741_824,
            }),
            disks: vec![
                Disk {
                    name: "Samsung SSD 990 PRO".to_owned(),
                    used_bytes: 100 * 1_073_741_824,
                    total_bytes: 500 * 1_073_741_824,
                },
                Disk {
                    name: "WD Blue".to_owned(),
                    used_bytes: 1_073_741_824,
                    total_bytes: 2 * 1_073_741_824,
                },
            ],
            networks: Vec::new(),
        }
    }

    #[test]
    fn two_disks_offer_two_key_sets() {
        let provider = SystemProvider::with_snapshot(two_disks());
        let described = provider.describe();
        let keys: Vec<&str> = described.iter().map(|d| d.semantic_key.as_str()).collect();
        assert!(keys.contains(&"disk.samsung-ssd-990-pro.used"));
        assert!(keys.contains(&"disk.wd-blue.free"));
        // Six per-device keys plus the abstract slots a theme may bind instead.
        assert!(keys.contains(&"disk.used"));
        assert!(keys.contains(&"disk.used.percent"));
        assert!(keys.contains(&"disk.total"));
        assert!(keys.contains(&"disk.name"));
        assert!(keys.contains(&"disk.data.used"));
        assert_eq!(
            keys.iter().filter(|key| key.starts_with("disk.")).count(),
            13
        );
    }

    #[test]
    fn the_devices_a_person_can_assign_are_the_drives_reported() {
        let provider = SystemProvider::with_snapshot(two_disks());
        let devices = provider.devices();
        assert!(
            devices.iter().any(|device| {
                device.group == "system-disk" && device.id == "samsung-ssd-990-pro"
            })
        );
        assert!(
            devices
                .iter()
                .any(|device| device.group == "data-disk" && device.name == "WD Blue")
        );
    }

    #[test]
    fn the_abstract_slots_answer_the_assigned_drive() {
        let assignment = crate::devices::Assignment {
            system_disk: Some("wd-blue".to_owned()),
            data_disk: Some("samsung-ssd-990-pro".to_owned()),
            names: [("wd-blue".to_owned(), "Data drive".to_owned())]
                .into_iter()
                .collect(),
            ..crate::devices::Assignment::default()
        };
        let provider = SystemProvider::with_snapshot(two_disks())
            .with_devices(Arc::new(move || assignment.clone()));
        let readings = provider
            .sample(
                &[
                    "disk.used".to_owned(),
                    "disk.total".to_owned(),
                    "disk.name".to_owned(),
                    "disk.data.total".to_owned(),
                    "disk.data.used.percent".to_owned(),
                ],
                1_000,
            )
            .expect("a poll");
        // The assigned system drive, by its saved name.
        assert_eq!(readings[0].value, Some(1.0));
        assert_eq!(readings[1].value, Some(2.0));
        assert_eq!(readings[2].text.as_deref(), Some("Data drive"));
        // The data slot is the other drive.
        assert_eq!(readings[3].value, Some(500.0));
        assert!((readings[4].value.unwrap() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn an_unassigned_data_slot_is_a_reason_not_the_wrong_drive() {
        let provider = SystemProvider::with_snapshot(two_disks());
        let readings = provider
            .sample(
                &["disk.data.used".to_owned(), "disk.used".to_owned()],
                1_000,
            )
            .expect("a poll");
        assert_eq!(readings[0].status, crate::sample::Status::Missing);
        assert_eq!(
            readings[0].message.as_deref(),
            Some("Assign a data disk in Settings.")
        );
        // Unassigned, the system slot is the first drive the machine reports.
        assert_eq!(readings[1].value, Some(100.0));
    }

    #[test]
    fn a_reading_this_machine_cannot_give_is_missing_not_zero() {
        let provider = SystemProvider::with_snapshot(Fixed::default());
        let readings = provider
            .sample(&["cpu.load".to_owned(), "ram.used".to_owned()], 1_000)
            .expect("a poll with nothing to fail");
        assert_eq!(readings.len(), 2);
        for reading in &readings {
            assert_eq!(reading.status, crate::sample::Status::Missing);
            assert!(reading.value.is_none());
            assert!(reading.message.is_some());
        }
    }

    #[test]
    fn a_percentage_is_the_share_of_total() {
        let provider = SystemProvider::with_snapshot(two_disks());
        let readings = provider
            .sample(&["ram.used.percent".to_owned()], 1_000)
            .expect("a poll");
        assert_eq!(readings[0].value, Some(50.0));
        assert_eq!(readings[0].unit.as_deref(), Some("%"));
    }

    #[test]
    fn a_network_rate_needs_two_samples_and_starts_after_the_first() {
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Wiring(Arc<AtomicU64>);

        impl Snapshot for Wiring {
            fn cpu_load(&self) -> Option<f64> {
                None
            }

            fn memory(&self) -> Option<Memory> {
                None
            }

            fn disks(&self) -> Vec<Disk> {
                Vec::new()
            }

            fn networks(&self) -> Vec<Network> {
                vec![Network {
                    name: "ethernet".to_owned(),
                    received_bytes: self.0.load(Ordering::SeqCst),
                    transmitted_bytes: 0,
                }]
            }
        }

        let counters = Arc::new(AtomicU64::new(1_000_000));
        let provider = SystemProvider::with_snapshot(Wiring(counters.clone()));
        let key = ["network.ethernet.download".to_owned()];
        let first = provider.sample(&key, 1_000).expect("a poll");
        assert_eq!(first[0].status, crate::sample::Status::Missing);

        // 1 MB received in one second is 8 Mb/s.
        counters.store(2_000_000, Ordering::SeqCst);
        let second = provider.sample(&key, 2_000).expect("a poll");
        assert_eq!(second[0].value, Some(8.0));
        assert_eq!(second[0].unit.as_deref(), Some("Mb/s"));
    }

    #[test]
    fn a_key_nothing_offers_is_not_answered() {
        let provider = SystemProvider::with_snapshot(two_disks());
        let readings = provider
            .sample(&["gpu.temp".to_owned()], 1_000)
            .expect("a poll with nothing to fail");
        assert!(readings.is_empty());
    }
}
