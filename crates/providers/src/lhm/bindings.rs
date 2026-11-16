//! Which LHM sensor answers which semantic key.
//!
//! A table, not a branch per caption: LHM names sensors by hardware family and
//! every binding says which group, which sensor kind and which captions it
//! accepts, in priority order.

use super::tree::LhmSensor;

/// Which hardware node a binding applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Cpu,
    Cooling,
    Gpu,
    Storage,
    Network,
    Motherboard,
    Psu,
    Battery,
}

/// How repeated hardware instances combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combine {
    Highest,
    Sum,
}

pub struct Binding {
    pub key: &'static str,
    pub group: Group,
    /// LHM's sensor kind, matched exactly.
    pub kind: &'static str,
    /// Accepted captions, in priority order; first match wins.
    pub any_text: &'static [&'static str],
    pub combine: Combine,
}

/// LHM reports `Data` in GB and `SmallData` in MB.
const MIB_PER_GB: f64 = 1024.0;
/// `Throughput` is bytes per second; the vocabulary is Mb/s.
const MEGABIT_PER_BYTE_PER_SECOND: f64 = 8.0 / 1_000_000.0;

/// Which group a hardware node belongs to.
///
/// Order matters. PSU and battery come first because their node text is a
/// product name; then network (a "Bluetooth Network Connection" is a link), then
/// the specific device kinds before the board: a board-vendor GPU or SSD must
/// keep its own group, and a GPU named "Intel Arc" must not be read as a CPU.
pub fn group_of(hardware_name: &str, hardware_id: &str) -> Option<Group> {
    let name = hardware_name.to_ascii_lowercase();
    let id = hardware_id.to_ascii_lowercase();
    if id == "/psu" || id.starts_with("/psu/") {
        return Some(Group::Psu);
    }
    if id == "/battery" || id.starts_with("/battery/") {
        return Some(Group::Battery);
    }
    if contains_any(
        &name,
        &["ethernet", "wifi", "wi-fi", "wireless", "network", "802.11"],
    ) {
        return Some(Group::Network);
    }
    if contains_any(
        &name,
        &[
            "ssd", "hdd", "nvme", "disk", "wdc", "wd0", "wd1", "wd2", "wd3", "wd4", "wd5", "wd6",
            "wd8", "st1", "st2", "st3", "st4", "ct1", "micron", "samsung", "lexar", "seagate",
            "kingston", "toshiba", "crucial", "sandisk",
        ],
    ) {
        return Some(Group::Storage);
    }
    if contains_any(&name, &["nvidia", "geforce", "radeon", "gpu", "arc a"]) {
        return Some(Group::Gpu);
    }
    if contains_any(&name, &["intel", "amd", "ryzen", "threadripper"]) {
        return Some(Group::Cpu);
    }
    if contains_any(
        &name,
        &[
            "motherboard",
            "gigabyte",
            "ite it",
            "nuvoton",
            "fintek",
            "prime",
            "aorus",
            "tomahawk",
        ],
    ) {
        return Some(Group::Motherboard);
    }
    if contains_any(
        &name,
        &[
            "kraken", "corsair", "nzxt", "coolit", "aio", "liquid", "cooler",
        ],
    ) {
        return Some(Group::Cooling);
    }
    None
}

fn contains_any(name: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| name.contains(needle))
}

pub static BINDINGS: &[Binding] = &[
    // CPU. `Core Average` is the package temperature LHM reports when a
    // dedicated package sensor is absent; `Core Max` is the hotter reading.
    Binding {
        key: "cpu.temp",
        group: Group::Cpu,
        kind: "Temperature",
        any_text: &["CPU Package", "Core Average", "Core Max"],
        combine: Combine::Highest,
    },
    // A liquid cooler's own sensor answers the CPU heat too.
    Binding {
        key: "cpu.temp",
        group: Group::Cooling,
        kind: "Temperature",
        any_text: &["Liquid"],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.temp.package",
        group: Group::Cpu,
        kind: "Temperature",
        any_text: &["CPU Package"],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.temp.core.max",
        group: Group::Cpu,
        kind: "Temperature",
        any_text: &["Core Max"],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.temp.core.avg",
        group: Group::Cpu,
        kind: "Temperature",
        any_text: &["Core Average"],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.power",
        group: Group::Cpu,
        kind: "Power",
        any_text: &["CPU Package", "Package", "CPU Cores"],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.clock",
        group: Group::Cpu,
        kind: "Clock",
        any_text: &[
            "Cores (Average)",
            "Core Average",
            "CPU Core #1",
            "Bus Speed",
        ],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.voltage",
        group: Group::Cpu,
        kind: "Voltage",
        any_text: &["CPU Core"],
        combine: Combine::Highest,
    },
    // Only an explicit CPU fan name answers this; a case or AIO fan does not.
    Binding {
        key: "cpu.fan",
        group: Group::Cpu,
        kind: "Fan",
        any_text: &["CPU Fan", "CPU"],
        combine: Combine::Highest,
    },
    Binding {
        key: "cpu.fan",
        group: Group::Cooling,
        kind: "Fan",
        any_text: &[
            "Pump", "Fan #1", "Fan #2", "Fan #3", "Fan #4", "Fan #5", "Fan #6",
        ],
        combine: Combine::Highest,
    },
    // The super-I/O headers answered `cpu.fan` before the board got its own
    // group, so they still do: an aggregate key keeps its precedence.
    Binding {
        key: "cpu.fan",
        group: Group::Motherboard,
        kind: "Fan",
        any_text: &["Fan #1", "Fan #2", "Fan #3", "Fan #4", "Fan #5", "Fan #6"],
        combine: Combine::Highest,
    },
    // Motherboard and cooler. A board LHM cannot label reports its super-I/O
    // temperature sources as `Temperature #N`.
    Binding {
        key: "motherboard.temp",
        group: Group::Motherboard,
        kind: "Temperature",
        any_text: &[
            "Temperature #1",
            "Temperature #2",
            "Temperature #3",
            "Temperature #4",
            "Temperature #5",
            "Temperature #6",
        ],
        combine: Combine::Highest,
    },
    Binding {
        key: "cooling.temp",
        group: Group::Cooling,
        kind: "Temperature",
        any_text: &["Liquid"],
        combine: Combine::Highest,
    },
    // PSU and battery: the hardware type comes from the path root; the sensor
    // names are the ones LHM's own PSU and battery classes create.
    Binding {
        key: "psu.power",
        group: Group::Psu,
        kind: "Power",
        any_text: &["Total watts", "Total Output"],
        combine: Combine::Highest,
    },
    Binding {
        key: "psu.temp",
        group: Group::Psu,
        kind: "Temperature",
        any_text: &["VRM", "Case"],
        combine: Combine::Highest,
    },
    Binding {
        key: "psu.fan",
        group: Group::Psu,
        kind: "Fan",
        any_text: &["Case"],
        combine: Combine::Highest,
    },
    Binding {
        key: "battery.level",
        group: Group::Battery,
        kind: "Level",
        any_text: &["Charge Level"],
        combine: Combine::Highest,
    },
    Binding {
        key: "battery.voltage",
        group: Group::Battery,
        kind: "Voltage",
        any_text: &["Voltage"],
        combine: Combine::Highest,
    },
    Binding {
        key: "battery.rate",
        group: Group::Battery,
        kind: "Power",
        any_text: &["Charge/Discharge Rate"],
        combine: Combine::Highest,
    },
    Binding {
        key: "battery.timeRemaining",
        group: Group::Battery,
        kind: "TimeSpan",
        any_text: &["Remaining Time (Estimated)"],
        combine: Combine::Highest,
    },
    // GPU
    Binding {
        key: "gpu.load",
        group: Group::Gpu,
        kind: "Load",
        any_text: &["GPU Core", "D3D 3D"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.temp",
        group: Group::Gpu,
        kind: "Temperature",
        any_text: &["GPU Core"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.temp.core",
        group: Group::Gpu,
        kind: "Temperature",
        any_text: &["GPU Core"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.temp.hotspot",
        group: Group::Gpu,
        kind: "Temperature",
        any_text: &["GPU Hot Spot"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.power",
        group: Group::Gpu,
        kind: "Power",
        any_text: &["GPU Package", "GPU Power"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.voltage",
        group: Group::Gpu,
        kind: "Voltage",
        any_text: &["GPU Core Voltage"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.clock",
        group: Group::Gpu,
        kind: "Clock",
        any_text: &["GPU Core"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.clock.memory",
        group: Group::Gpu,
        kind: "Clock",
        any_text: &["GPU Memory"],
        combine: Combine::Highest,
    },
    Binding {
        key: "gpu.fan",
        group: Group::Gpu,
        kind: "Fan",
        any_text: &["GPU Fan 1", "GPU Fan"],
        combine: Combine::Highest,
    },
    // VRAM. LHM reports GPU memory as `SmallData` in MB, not `Data` in GB.
    Binding {
        key: "vram.used",
        group: Group::Gpu,
        kind: "SmallData",
        any_text: &["D3D Dedicated Memory Used", "GPU Memory Used"],
        combine: Combine::Highest,
    },
    Binding {
        key: "vram.total",
        group: Group::Gpu,
        kind: "SmallData",
        any_text: &["GPU Memory Total"],
        combine: Combine::Highest,
    },
    Binding {
        key: "vram.used.percent",
        group: Group::Gpu,
        kind: "Load",
        any_text: &["GPU Memory"],
        combine: Combine::Highest,
    },
    Binding {
        key: "vram.temp",
        group: Group::Gpu,
        kind: "Temperature",
        any_text: &["GPU Memory Junction"],
        combine: Combine::Highest,
    },
    // Disk capacity. `Used Space` is typed Load but carries a percentage, so it
    // answers only the percent key; `disk.used` is derived from Total minus
    // Free.
    Binding {
        key: "disk.used.percent",
        group: Group::Storage,
        kind: "Load",
        any_text: &["Used Space"],
        combine: Combine::Highest,
    },
    Binding {
        key: "disk.total",
        group: Group::Storage,
        kind: "Data",
        any_text: &["Total Space"],
        combine: Combine::Sum,
    },
    Binding {
        key: "disk.free",
        group: Group::Storage,
        kind: "Data",
        any_text: &["Free Space"],
        combine: Combine::Sum,
    },
    // Network. LHM's `Throughput` is bytes per second; the vocabulary is Mb/s.
    Binding {
        key: "network.download",
        group: Group::Network,
        kind: "Throughput",
        any_text: &["Download Speed"],
        combine: Combine::Highest,
    },
    Binding {
        key: "network.upload",
        group: Group::Network,
        kind: "Throughput",
        any_text: &["Upload Speed"],
        combine: Combine::Highest,
    },
];

/// The value `key` would read from these sensors, in the vocabulary's unit.
///
/// A key with no matching sensor is `None`: LHM simply does not have it on this
/// machine, which is a gap rather than a zero.
pub fn read(sensors: &[LhmSensor], key: &str) -> Option<f64> {
    // `disk.used` has no direct LHM sensor: LHM's `Used Space` is a percentage.
    if key == "disk.used" {
        let total = read(sensors, "disk.total")?;
        let free = read(sensors, "disk.free")?;
        return Some(total - free);
    }

    for binding in BINDINGS.iter().filter(|binding| binding.key == key) {
        // Caption order is priority order: the first caption this machine
        // reports answers, and a lower-priority caption never overrides it.
        for caption in binding.any_text {
            let mut highest: Option<f64> = None;
            let mut sum = 0.0;
            let mut found = false;
            for sensor in sensors {
                let Some(value) = sensor.value else {
                    continue;
                };
                if sensor.kind != binding.kind
                    || group_of(&sensor.hardware_name, &sensor.hardware_id) != Some(binding.group)
                    || sensor.text != *caption
                {
                    continue;
                }
                let value = scale(binding.kind, value);
                found = true;
                sum += value;
                highest = Some(highest.map_or(value, |best: f64| best.max(value)));
            }
            if found {
                return Some(match binding.combine {
                    Combine::Highest => highest.unwrap_or_default(),
                    Combine::Sum => sum,
                });
            }
        }
    }
    None
}

/// A sensor's value in the vocabulary's unit for its kind.
fn scale(kind: &str, value: f64) -> f64 {
    match kind {
        // MB to GB, the unit the vocabulary declares for VRAM.
        "SmallData" => value / MIB_PER_GB,
        "Throughput" => value * MEGABIT_PER_BYTE_PER_SECOND,
        // `Data` is already GB, and the rest carry their own unit.
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensor(
        hardware_name: &str,
        hardware_id: &str,
        text: &str,
        kind: &str,
        value: f64,
    ) -> LhmSensor {
        LhmSensor {
            sensor_id: format!("{hardware_id}/{kind}/0"),
            hardware_id: hardware_id.to_owned(),
            hardware_name: hardware_name.to_owned(),
            text: text.to_owned(),
            kind: kind.to_owned(),
            value: Some(value),
        }
    }

    #[test]
    fn a_group_reads_the_device_kind_before_the_board() {
        assert_eq!(
            group_of("Gigabyte GeForce RTX 4090", "/gpu-nvidia/0"),
            Some(Group::Gpu)
        );
        assert_eq!(group_of("Intel Arc A770", "/gpu-intel/0"), Some(Group::Gpu));
        assert_eq!(
            group_of("Bluetooth Network Connection", "/network/0"),
            Some(Group::Network)
        );
        assert_eq!(
            group_of("AORUS NVMe Gen4 SSD", "/nvme/0"),
            Some(Group::Storage)
        );
        // The path root names a PSU whatever the node text says.
        assert_eq!(group_of("Corsair HX1000i", "/psu/0"), Some(Group::Psu));
        assert_eq!(group_of("DELL 97WH", "/battery/0"), Some(Group::Battery));
        assert_eq!(group_of("ITE IT8689E", "/lpc/0"), Some(Group::Motherboard));
        assert_eq!(group_of("NZXT Kraken X63", "/lpc/1"), Some(Group::Cooling));
        assert_eq!(group_of("Generic Device", "/unknown/0"), None);
    }

    #[test]
    fn the_package_temperature_wins_over_a_core_extreme() {
        let sensors = vec![
            sensor(
                "AMD Ryzen 9 5950X",
                "/amdcpu/0",
                "Core Max",
                "Temperature",
                71.0,
            ),
            sensor(
                "AMD Ryzen 9 5950X",
                "/amdcpu/0",
                "CPU Package",
                "Temperature",
                64.0,
            ),
        ];
        assert_eq!(read(&sensors, "cpu.temp"), Some(64.0));
        assert_eq!(read(&sensors, "cpu.temp.core.max"), Some(71.0));
        // A key LHM has no sensor for is a gap.
        assert_eq!(read(&sensors, "cpu.power"), None);
    }

    #[test]
    fn a_coolers_liquid_temperature_also_answers_the_cpu() {
        let sensors = vec![sensor(
            "NZXT Kraken X63",
            "/lpc/1",
            "Liquid",
            "Temperature",
            31.5,
        )];
        assert_eq!(read(&sensors, "cpu.temp"), Some(31.5));
        assert_eq!(read(&sensors, "cooling.temp"), Some(31.5));
    }

    #[test]
    fn vram_megabytes_become_gigabytes_and_throughput_becomes_megabits() {
        let sensors = vec![
            sensor(
                "NVIDIA GeForce RTX 4090",
                "/gpu-nvidia/0",
                "GPU Memory Total",
                "SmallData",
                24576.0,
            ),
            sensor(
                "Ethernet",
                "/network/0",
                "Download Speed",
                "Throughput",
                125_000_000.0,
            ),
        ];
        assert_eq!(read(&sensors, "vram.total"), Some(24.0));
        assert_eq!(read(&sensors, "network.download"), Some(1_000.0));
    }

    #[test]
    fn disk_capacity_sums_every_drive_and_used_is_derived() {
        let sensors = vec![
            sensor("Samsung SSD 970", "/nvme/0", "Total Space", "Data", 500.0),
            sensor("Samsung SSD 970", "/nvme/0", "Free Space", "Data", 300.0),
            sensor("WDC WD40", "/hdd/0", "Total Space", "Data", 4000.0),
            sensor("WDC WD40", "/hdd/0", "Free Space", "Data", 1000.0),
        ];
        assert_eq!(read(&sensors, "disk.total"), Some(4500.0));
        assert_eq!(read(&sensors, "disk.used"), Some(3200.0));
    }

    #[test]
    fn a_sensor_that_could_not_be_read_never_answers() {
        let mut sensor = sensor(
            "AMD Ryzen 9 5950X",
            "/amdcpu/0",
            "CPU Package",
            "Temperature",
            0.0,
        );
        sensor.value = None;
        assert_eq!(read(&[sensor], "cpu.temp"), None);
    }
}
