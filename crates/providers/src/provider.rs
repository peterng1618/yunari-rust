//! Where a reading comes from.

use crate::sample::Reading;

/// A key a provider can read, with the unit it reports in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Descriptor {
    pub semantic_key: String,
    pub unit: Option<String>,
}

impl Descriptor {
    pub fn new(semantic_key: impl Into<String>, unit: Option<&str>) -> Self {
        Self {
            semantic_key: semantic_key.into(),
            unit: unit.map(str::to_owned),
        }
    }
}

/// Whether a provider can still do its job, and why not when it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Ok,
    Degraded(String),
    Unavailable(String),
}

impl Health {
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ok => None,
            Self::Degraded(reason) | Self::Unavailable(reason) => Some(reason),
        }
    }
}

/// One physical device a person may assign to a theme's abstract slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// `gpu`, `system-disk` or `data-disk`.
    pub group: String,
    /// The id `devices.json` names the device by.
    pub id: String,
    /// The model the machine reports.
    pub name: String,
}

/// One source of readings.
///
/// `sample` takes the union of every display's asks, so many displays cost one
/// poll. An error ends that provider's part of the cycle and nothing else: the
/// registry keeps the other providers' readings.
pub trait Provider: Send + Sync {
    /// Stable identifier, unique within a registry.
    fn id(&self) -> &str;

    /// The name a consumer sees.
    fn label(&self) -> &str;

    /// Every key this provider can currently read. A device-scoped provider
    /// lists what this machine has.
    fn describe(&self) -> Vec<Descriptor>;

    fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String>;

    /// The physical devices this provider can name, so a settings surface can
    /// offer a choice without knowing how any provider enumerates hardware.
    fn devices(&self) -> Vec<Device> {
        Vec::new()
    }

    fn health(&self) -> Health {
        Health::Ok
    }
}
