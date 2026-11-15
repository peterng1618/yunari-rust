//! The semantic-key vocabulary: what each reading means, not whether this PC
//! can produce it.
//!
//! Generated once from the source vocabulary (`renderer-core`'s
//! `data/semantic-keys.ts`) and kept honest by its tests rather than by a
//! parser at build time.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Picker grouping order. A provider may contribute families of its own.
pub static FAMILIES: &[&str] = &["cpu", "ram", "gpu", "vram", "disk", "network", "time", "date", "cooling", "motherboard", "battery", "psu", "system", "audio", "wifi", "weather", "media", "frame", "game", "custom"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Every build can read this on the platform it runs on.
    Baseline,
    /// Needs a sensor source beyond the platform's own counters.
    Extended,
}

/// The conversion contract. A unit without one is shown as measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Percent,
    Temperature,
    Clock,
    Data,
    DataRate,
    Power,
    Voltage,
    Current,
    Rotation,
    Duration,
    Speed,
    Pressure,
    Count,
    Text,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Text,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// The binding renders an image URL rather than a value.
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Converts {
    /// The display's measurement preference converts this key.
    Temperature,
}

/// A time or date reading: the provider sends an ISO 8601 instant, so the
/// binding's format and zone decide how it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instant {
    pub default_format: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticKeyDescriptor {
    pub key: &'static str,
    pub family: &'static str,
    pub label: &'static str,
    pub unit: Option<&'static str>,
    pub unit_kind: Option<UnitKind>,
    /// Units this key may be displayed in, besides `unit`.
    pub display_units: &'static [&'static str],
    pub value_kind: Option<ValueKind>,
    pub media_kind: Option<MediaKind>,
    pub expected_tier: Tier,
    pub converts: Option<Converts>,
    pub instant: Option<Instant>,
}

/// The built-in keys. Device-scoped keys are described by shape, below.
pub static SEMANTIC_KEYS: &[SemanticKeyDescriptor] = &[
    SemanticKeyDescriptor {
        key: "cpu.load",
        family: "cpu",
        label: "CPU load",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.temp",
        family: "cpu",
        label: "CPU temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.temp.package",
        family: "cpu",
        label: "CPU package temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.temp.core.max",
        family: "cpu",
        label: "CPU hottest core",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.temp.core.avg",
        family: "cpu",
        label: "CPU average core temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.power",
        family: "cpu",
        label: "CPU power",
        unit: Some("W"),
        unit_kind: Some(UnitKind::Power),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.voltage",
        family: "cpu",
        label: "CPU voltage",
        unit: Some("V"),
        unit_kind: Some(UnitKind::Voltage),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.clock",
        family: "cpu",
        label: "CPU clock",
        unit: Some("MHz"),
        unit_kind: Some(UnitKind::Clock),
        display_units: &["MHz", "GHz"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.cores.physical",
        family: "cpu",
        label: "CPU physical cores",
        unit: None,
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: Some(ValueKind::Number),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.cores.logical",
        family: "cpu",
        label: "CPU logical cores",
        unit: None,
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: Some(ValueKind::Number),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.fan",
        family: "cpu",
        label: "CPU fan",
        unit: Some("RPM"),
        unit_kind: Some(UnitKind::Rotation),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.manufacturer",
        family: "cpu",
        label: "CPU manufacturer",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.brand",
        family: "cpu",
        label: "CPU brand",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cpu.model",
        family: "cpu",
        label: "CPU model",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "ram.used",
        family: "ram",
        label: "RAM used",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "ram.used.percent",
        family: "ram",
        label: "RAM used (share of total)",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "ram.total",
        family: "ram",
        label: "RAM total",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.load",
        family: "gpu",
        label: "GPU load",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.temp",
        family: "gpu",
        label: "GPU temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.temp.core",
        family: "gpu",
        label: "GPU core temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.temp.hotspot",
        family: "gpu",
        label: "GPU hotspot temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.power",
        family: "gpu",
        label: "GPU power",
        unit: Some("W"),
        unit_kind: Some(UnitKind::Power),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.voltage",
        family: "gpu",
        label: "GPU voltage",
        unit: Some("V"),
        unit_kind: Some(UnitKind::Voltage),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.clock",
        family: "gpu",
        label: "GPU clock",
        unit: Some("MHz"),
        unit_kind: Some(UnitKind::Clock),
        display_units: &["MHz", "GHz"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.clock.memory",
        family: "gpu",
        label: "GPU memory clock",
        unit: Some("MHz"),
        unit_kind: Some(UnitKind::Clock),
        display_units: &["MHz", "GHz"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.fan",
        family: "gpu",
        label: "GPU fan",
        unit: Some("RPM"),
        unit_kind: Some(UnitKind::Rotation),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "gpu.name",
        family: "gpu",
        label: "GPU model",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "vram.used",
        family: "vram",
        label: "VRAM used",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "vram.used.percent",
        family: "vram",
        label: "VRAM used (share of total)",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "vram.total",
        family: "vram",
        label: "VRAM total",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "vram.temp",
        family: "vram",
        label: "VRAM temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "time.now",
        family: "time",
        label: "Time",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: Some(Instant { default_format: "HH:mm" }),
    },
    SemanticKeyDescriptor {
        key: "date.today",
        family: "date",
        label: "Date",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: Some(Instant { default_format: "DD MMM YYYY" }),
    },
    SemanticKeyDescriptor {
        key: "disk.used",
        family: "disk",
        label: "Disk used",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "disk.data.used",
        family: "disk",
        label: "Data disk used",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "disk.data.used.percent",
        family: "disk",
        label: "Data disk used (share of total)",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "disk.data.total",
        family: "disk",
        label: "Data disk total",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "disk.used.percent",
        family: "disk",
        label: "Disk used (share of total)",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "disk.total",
        family: "disk",
        label: "Disk total",
        unit: Some("GB"),
        unit_kind: Some(UnitKind::Data),
        display_units: &["MB", "GB", "TB", "MiB", "GiB", "TiB"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "disk.name",
        family: "disk",
        label: "Volume name",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "network.download",
        family: "network",
        label: "Network download",
        unit: Some("Mb/s"),
        unit_kind: Some(UnitKind::DataRate),
        display_units: &["kb/s", "Mb/s", "Gb/s", "kB/s", "MB/s", "GB/s"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "network.upload",
        family: "network",
        label: "Network upload",
        unit: Some("Mb/s"),
        unit_kind: Some(UnitKind::DataRate),
        display_units: &["kb/s", "Mb/s", "Gb/s", "kB/s", "MB/s", "GB/s"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "cooling.temp",
        family: "cooling",
        label: "Coolant temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "motherboard.temp",
        family: "motherboard",
        label: "Motherboard temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "chipset.temp",
        family: "motherboard",
        label: "Chipset temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "vrm.temp",
        family: "motherboard",
        label: "VRM temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.level",
        family: "battery",
        label: "Battery level",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.state",
        family: "battery",
        label: "Battery state",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.charging",
        family: "battery",
        label: "Battery charging",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Boolean),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.timeRemaining",
        family: "battery",
        label: "Battery time remaining",
        unit: Some("s"),
        unit_kind: Some(UnitKind::Duration),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.rate",
        family: "battery",
        label: "Battery rate",
        unit: Some("W"),
        unit_kind: Some(UnitKind::Power),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.voltage",
        family: "battery",
        label: "Battery voltage",
        unit: Some("V"),
        unit_kind: Some(UnitKind::Voltage),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.health",
        family: "battery",
        label: "Battery health",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "battery.ac",
        family: "battery",
        label: "Battery on AC",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Boolean),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "psu.power",
        family: "psu",
        label: "PSU power",
        unit: Some("W"),
        unit_kind: Some(UnitKind::Power),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "psu.temp",
        family: "psu",
        label: "PSU temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "psu.fan",
        family: "psu",
        label: "PSU fan",
        unit: Some("RPM"),
        unit_kind: Some(UnitKind::Rotation),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "system.uptime",
        family: "system",
        label: "Uptime",
        unit: Some("s"),
        unit_kind: Some(UnitKind::Duration),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "system.processes",
        family: "system",
        label: "Process count",
        unit: None,
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: Some(ValueKind::Number),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "system.hostname",
        family: "system",
        label: "Host name",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "system.os",
        family: "system",
        label: "Operating system",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "system.osBuild",
        family: "system",
        label: "OS build",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "audio.volume",
        family: "audio",
        label: "Volume",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "audio.muted",
        family: "audio",
        label: "Muted",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Boolean),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "wifi.ssid",
        family: "wifi",
        label: "Wi-Fi network",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "wifi.signal",
        family: "wifi",
        label: "Wi-Fi signal",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "wifi.linkSpeed",
        family: "wifi",
        label: "Wi-Fi link speed",
        unit: Some("Mb/s"),
        unit_kind: Some(UnitKind::DataRate),
        display_units: &["kb/s", "Mb/s", "Gb/s", "kB/s", "MB/s", "GB/s"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.location",
        family: "weather",
        label: "Weather location",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.temp",
        family: "weather",
        label: "Temperature",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.feelsLike",
        family: "weather",
        label: "Feels like",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.high",
        family: "weather",
        label: "Daily high",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.low",
        family: "weather",
        label: "Daily low",
        unit: Some("°C"),
        unit_kind: Some(UnitKind::Temperature),
        display_units: &["°C", "°F"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: Some(Converts::Temperature),
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.conditionCode",
        family: "weather",
        label: "Condition code",
        unit: None,
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: Some(ValueKind::Number),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.condition",
        family: "weather",
        label: "Condition",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.icon",
        family: "weather",
        label: "Condition icon",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: Some(MediaKind::Image),
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.humidity",
        family: "weather",
        label: "Humidity",
        unit: Some("%"),
        unit_kind: Some(UnitKind::Percent),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.windSpeed",
        family: "weather",
        label: "Wind speed",
        unit: Some("km/h"),
        unit_kind: Some(UnitKind::Speed),
        display_units: &["km/h", "mph", "m/s"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.windDirection",
        family: "weather",
        label: "Wind direction",
        unit: Some("°"),
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.windDirection.compass",
        family: "weather",
        label: "Wind direction (compass)",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.pressure",
        family: "weather",
        label: "Pressure",
        unit: Some("hPa"),
        unit_kind: Some(UnitKind::Pressure),
        display_units: &["hPa", "inHg"],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.precipitation",
        family: "weather",
        label: "Precipitation",
        unit: Some("mm"),
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.isDay",
        family: "weather",
        label: "Daylight",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Boolean),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "weather.sunrise",
        family: "weather",
        label: "Sunrise",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: Some(Instant { default_format: "HH:mm" }),
    },
    SemanticKeyDescriptor {
        key: "weather.sunset",
        family: "weather",
        label: "Sunset",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: Some(Instant { default_format: "HH:mm" }),
    },
    SemanticKeyDescriptor {
        key: "weather.attribution",
        family: "weather",
        label: "Weather attribution",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.title",
        family: "media",
        label: "Title",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.artist",
        family: "media",
        label: "Artist",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.album",
        family: "media",
        label: "Album",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.artwork",
        family: "media",
        label: "Artwork",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: Some(MediaKind::Image),
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.state",
        family: "media",
        label: "Playback state",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.position",
        family: "media",
        label: "Playback position",
        unit: Some("s"),
        unit_kind: Some(UnitKind::Duration),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.duration",
        family: "media",
        label: "Track duration",
        unit: Some("s"),
        unit_kind: Some(UnitKind::Duration),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "media.app",
        family: "media",
        label: "Media app",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "fps.current",
        family: "frame",
        label: "FPS",
        unit: Some("fps"),
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "frametime.current",
        family: "frame",
        label: "Frame time",
        unit: Some("ms"),
        unit_kind: Some(UnitKind::Duration),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "fps.low1",
        family: "frame",
        label: "1% low FPS",
        unit: Some("fps"),
        unit_kind: Some(UnitKind::Count),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "frame.app",
        family: "frame",
        label: "Game process",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "game.name",
        family: "game",
        label: "Game name",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "game.artwork",
        family: "game",
        label: "Game artwork",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: Some(MediaKind::Image),
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "game.running",
        family: "game",
        label: "Game running",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Boolean),
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
    SemanticKeyDescriptor {
        key: "game.app",
        family: "game",
        label: "Game process",
        unit: None,
        unit_kind: None,
        display_units: &[],
        value_kind: Some(ValueKind::Text),
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    },
];

static BY_KEY: LazyLock<HashMap<&'static str, &'static SemanticKeyDescriptor>> =
    LazyLock::new(|| SEMANTIC_KEYS.iter().map(|key| (key.key, key)).collect());

/// A key as this build describes it. Built-in keys own nothing beyond the
/// strings the table already holds; a device-scoped key carries the label its
/// device and quantity spell.
#[derive(Debug, Clone, PartialEq)]
pub struct Described {
    pub key: String,
    pub family: &'static str,
    pub label: String,
    pub unit: Option<&'static str>,
    pub unit_kind: Option<UnitKind>,
    pub display_units: &'static [&'static str],
    pub value_kind: Option<ValueKind>,
    pub media_kind: Option<MediaKind>,
    pub expected_tier: Tier,
    pub converts: Option<Converts>,
    pub instant: Option<Instant>,
}

impl Described {
    fn of(key: &SemanticKeyDescriptor) -> Self {
        Self {
            key: key.key.to_owned(),
            family: key.family,
            label: key.label.to_owned(),
            unit: key.unit,
            unit_kind: key.unit_kind,
            display_units: key.display_units,
            value_kind: key.value_kind,
            media_kind: key.media_kind,
            expected_tier: key.expected_tier,
            converts: key.converts,
            instant: key.instant,
        }
    }
}

/// The descriptor for a key, built-in or discovered at runtime. `None` means
/// the vocabulary does not know the key; a caller shows the raw key rather than
/// refusing it, because a newer provider may still read it.
pub fn describe(key: &str) -> Option<Described> {
    if let Some(built_in) = BY_KEY.get(key) {
        return Some(Described::of(built_in));
    }
    dynamic(key)
}

pub fn is_known(key: &str) -> bool {
    describe(key).is_some()
}

/// A key's label, or the key itself when the vocabulary does not know it.
pub fn label_for(key: &str) -> &str {
    BY_KEY.get(key).map_or(key, |entry| entry.label)
}

/// The display units a data, rate or temperature quantity may be pinned to.
const TEMPERATURE_UNITS: &[&str] = &["°C", "°F"];
const CLOCK_UNITS: &[&str] = &["MHz", "GHz"];
const DATA_UNITS: &[&str] = &["MB", "GB", "TB", "MiB", "GiB", "TiB"];
const RATE_UNITS: &[&str] = &["kb/s", "Mb/s", "Gb/s", "kB/s", "MB/s", "GB/s"];

/// Device-scoped keys whose identity only a provider knows at runtime.
fn dynamic(key: &str) -> Option<Described> {
    let parts: Vec<&str> = key.split('.').collect();
    let found = match parts.as_slice() {
        ["cpu", "core", core, quantity] => core_key(core.parse().ok()?, quantity)?,
        ["cooling", "fan", device, kind] => fan_key("cooling", device, kind)?,
        ["cooling", "pump", device, "rpm"] if is_slug(device) => Described {
            key: key.to_owned(),
            label: format!("Cooling pump {device} speed"),
            ..rotation("cooling")
        },
        ["motherboard", "fan", device, kind] => fan_key("motherboard", device, kind)?,
        ["motherboard", "voltage", device] if is_slug(device) => Described {
            key: key.to_owned(),
            label: format!("Motherboard voltage {device}"),
            ..voltage("motherboard")
        },
        ["gpu", "fan", fan] if is_digits(fan) => Described {
            key: key.to_owned(),
            label: format!("GPU fan {fan}"),
            ..rotation("gpu")
        },
        ["disk", device, quantity] => disk_key(key, device, quantity)?,
        ["network", device, direction @ ("download" | "upload")] if is_slug(device) => Described {
            key: key.to_owned(),
            family: "network",
            label: format!("Network {device} {direction}"),
            unit: Some("Mb/s"),
            unit_kind: Some(UnitKind::DataRate),
            display_units: RATE_UNITS,
            value_kind: None,
            media_kind: None,
            expected_tier: Tier::Baseline,
            converts: None,
            instant: None,
        },
        ["psu", "voltage", device] if is_slug(device) => Described {
            key: key.to_owned(),
            label: format!("PSU voltage {device}"),
            ..voltage("psu")
        },
        ["custom", source, field] if is_custom(source) && is_custom_field(field) => Described {
            key: key.to_owned(),
            family: "custom",
            label: format!("{source} {field}"),
            unit: None,
            unit_kind: None,
            display_units: &[],
            value_kind: Some(ValueKind::Number),
            media_kind: None,
            expected_tier: Tier::Extended,
            converts: None,
            instant: None,
        },
        _ => return None,
    };
    Some(found)
}

fn core_key(core: u32, quantity: &str) -> Option<Described> {
    if core == 0 {
        return None;
    }
    let (text, unit, unit_kind, units) = match quantity {
        "load" => ("load", "%", UnitKind::Percent, &[][..]),
        "temp" => (
            "temperature",
            "°C",
            UnitKind::Temperature,
            TEMPERATURE_UNITS,
        ),
        "clock" => ("clock", "MHz", UnitKind::Clock, CLOCK_UNITS),
        _ => return None,
    };
    Some(Described {
        key: format!("cpu.core.{core}.{quantity}"),
        family: "cpu",
        label: format!("CPU core {core} {text}"),
        unit: Some(unit),
        unit_kind: Some(unit_kind),
        display_units: units,
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: (unit_kind == UnitKind::Temperature).then_some(Converts::Temperature),
        instant: None,
    })
}

fn fan_key(family: &'static str, device: &str, kind: &str) -> Option<Described> {
    if !is_slug(device) {
        return None;
    }
    let name = if family == "cooling" {
        "Cooling fan"
    } else {
        "Motherboard fan"
    };
    Some(match kind {
        "rpm" => Described {
            key: format!("{family}.fan.{device}.rpm"),
            label: format!("{name} {device} speed"),
            ..rotation(family)
        },
        "percent" => Described {
            key: format!("{family}.fan.{device}.percent"),
            family,
            label: format!("{name} {device} control"),
            unit: Some("%"),
            unit_kind: Some(UnitKind::Percent),
            display_units: &[],
            value_kind: None,
            media_kind: None,
            expected_tier: Tier::Extended,
            converts: None,
            instant: None,
        },
        _ => return None,
    })
}

/// The quantities a per-disk key may name, with the unit each is read in.
const DISK_QUANTITIES: &[(&str, &str, &str, UnitKind)] = &[
    ("used", "used", "GB", UnitKind::Data),
    ("used.percent", "used (share)", "%", UnitKind::Percent),
    ("total", "total", "GB", UnitKind::Data),
    ("free", "free", "GB", UnitKind::Data),
    ("temp", "temperature", "°C", UnitKind::Temperature),
    ("health", "health", "%", UnitKind::Percent),
    ("read", "read rate", "MB/s", UnitKind::DataRate),
    ("write", "write rate", "MB/s", UnitKind::DataRate),
    ("activity", "activity", "%", UnitKind::Percent),
];

fn disk_key(key: &str, device: &str, quantity: &str) -> Option<Described> {
    if !is_slug(device) {
        return None;
    }
    let (_, text, unit, unit_kind) = DISK_QUANTITIES
        .iter()
        .find(|(name, ..)| *name == quantity)?;
    let display_units = match *unit_kind {
        UnitKind::Data => DATA_UNITS,
        UnitKind::DataRate => RATE_UNITS,
        UnitKind::Temperature => TEMPERATURE_UNITS,
        _ => &[],
    };
    Some(Described {
        key: key.to_owned(),
        family: "disk",
        label: format!("{device} {text}"),
        unit: Some(unit),
        unit_kind: Some(*unit_kind),
        display_units,
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Baseline,
        converts: (*unit_kind == UnitKind::Temperature).then_some(Converts::Temperature),
        instant: None,
    })
}

fn rotation(family: &'static str) -> Described {
    Described {
        key: String::new(),
        family,
        label: String::new(),
        unit: Some("RPM"),
        unit_kind: Some(UnitKind::Rotation),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    }
}

fn voltage(family: &'static str) -> Described {
    Described {
        key: String::new(),
        family,
        label: String::new(),
        unit: Some("V"),
        unit_kind: Some(UnitKind::Voltage),
        display_units: &[],
        value_kind: None,
        media_kind: None,
        expected_tier: Tier::Extended,
        converts: None,
        instant: None,
    }
}

/// `<slug>`: lowercase, hyphen-separated, at most 64 characters.
fn is_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.len() <= 3 && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// A custom source id may carry `_` and may start with one.
fn is_custom(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-')
}

fn is_custom_field(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

/// Slug for a device's model or name so it can appear in a semantic key.
pub fn semantic_id(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_dash = true;
    for character in name.to_lowercase().chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            slug.push(character);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_key_names_a_built_in_family() {
        for key in SEMANTIC_KEYS {
            assert!(
                FAMILIES.contains(&key.family),
                "{} has family {}",
                key.key,
                key.family
            );
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for key in SEMANTIC_KEYS {
            assert!(seen.insert(key.key), "{} is declared twice", key.key);
        }
    }

    #[test]
    fn a_percentage_reading_carries_its_unit_and_kind() {
        let key = describe("cpu.load").expect("cpu.load is a built-in key");
        assert_eq!(key.unit, Some("%"));
        assert_eq!(key.unit_kind, Some(UnitKind::Percent));
        assert_eq!(key.expected_tier, Tier::Baseline);
    }

    #[test]
    fn a_temperature_reading_declares_the_measurement_conversion() {
        let key = describe("cpu.temp").expect("cpu.temp is a built-in key");
        assert_eq!(key.converts, Some(Converts::Temperature));
        assert_eq!(key.display_units, &["°C", "°F"]);
    }

    #[test]
    fn a_clock_reading_is_an_instant_with_a_default_format() {
        let key = describe("time.now").expect("time.now is a built-in key");
        let instant = key.instant.expect("a time reading is an instant");
        assert!(!instant.default_format.is_empty());
    }

    #[test]
    fn an_unknown_key_is_absent_and_shows_its_raw_spelling() {
        assert!(describe("nonsense.key").is_none());
        assert!(!is_known("nonsense.key"));
        assert_eq!(label_for("nonsense.key"), "nonsense.key");
    }

    #[test]
    fn a_core_key_names_the_core_and_its_quantity() {
        let key = describe("cpu.core.3.load").expect("a core key");
        assert_eq!(key.label, "CPU core 3 load");
        assert_eq!(key.unit, Some("%"));
        assert!(describe("cpu.core.0.load").is_none(), "cores count from one");
        assert!(describe("cpu.core.x.load").is_none());
    }

    #[test]
    fn a_fan_key_reads_its_device_and_control() {
        let speed = describe("cooling.fan.radiator.rpm").expect("a fan key");
        assert_eq!(speed.unit, Some("RPM"));
        assert_eq!(speed.unit_kind, Some(UnitKind::Rotation));
        let control = describe("motherboard.fan.cpu.percent").expect("a fan key");
        assert_eq!(control.unit, Some("%"));
        assert_eq!(control.label, "Motherboard fan cpu control");
    }

    #[test]
    fn a_disk_key_reads_its_device_and_quantity() {
        let used = describe("disk.nvme0n1.used").expect("a disk key");
        assert_eq!(used.label, "nvme0n1 used");
        assert_eq!(used.unit, Some("GB"));
        assert_eq!(used.display_units, DATA_UNITS);
        assert_eq!(used.expected_tier, Tier::Baseline);
        let temp = describe("disk.nvme0n1.temp").expect("a disk temperature");
        assert_eq!(temp.converts, Some(Converts::Temperature));
        assert!(describe("disk.nvme0n1.nonsense").is_none());
    }

    #[test]
    fn a_network_key_names_its_interface_and_direction() {
        let key = describe("network.ethernet.download").expect("a network key");
        assert_eq!(key.unit_kind, Some(UnitKind::DataRate));
        assert_eq!(key.display_units, RATE_UNITS);
        assert!(describe("network.ethernet.sideways").is_none());
    }

    #[test]
    fn a_custom_key_keeps_the_alphabet_its_ids_may_carry() {
        let key = describe("custom.my_source.field_1").expect("a custom key");
        assert_eq!(key.family, "custom");
        assert_eq!(key.value_kind, Some(ValueKind::Number));
        assert!(
            describe("custom.MySource.field").is_none(),
            "identifiers are lower case"
        );
    }

    #[test]
    fn a_slug_is_lower_case_hyphenated_and_bounded() {
        assert_eq!(semantic_id("Samsung SSD 990 PRO"), "samsung-ssd-990-pro");
        assert_eq!(semantic_id("  --weird--  "), "weird");
        assert!(semantic_id("").is_empty());
        assert!(semantic_id(&"a".repeat(200)).len() <= 64);
    }
}
