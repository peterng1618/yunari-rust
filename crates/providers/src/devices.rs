//! `devices.json`: which physical device answers a theme's abstract slot, and
//! what a person calls each device on screen.
//!
//! A theme is authored against `gpu.load`, but a machine may have two cards and
//! several drives. The answer lives here, per host, so one theme works
//! everywhere. The file is hand-editable and read defensively: an entry this
//! build cannot use is dropped rather than stopping the host.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

pub const FILE: &str = "devices.json";
/// A display name is shown, not used as a key, so it is only length-bounded.
pub const MAX_NAME_CHARS: usize = 48;
const MAX_ID_CHARS: usize = 64;

/// The settled choice, with the names that ride beside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assignment {
    /// The device id whose readings answer the unsuffixed `gpu.*` keys.
    pub gpu: Option<String>,
    /// The device id answering `disk.used`, `disk.total`, `disk.name`.
    pub system_disk: Option<String>,
    /// The device id answering the `disk.data.*` slot.
    pub data_disk: Option<String>,
    /// What a person calls each device, keyed by device id.
    pub names: BTreeMap<String, String>,
}

impl Assignment {
    pub fn load(directory: &Path) -> Self {
        let stored: Value = std::fs::read_to_string(directory.join(FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        Self::from_json(&stored)
    }

    pub fn from_json(value: &Value) -> Self {
        let mut assignment = Self::default();
        let Some(record) = value.as_object() else {
            return assignment;
        };
        for (key, slot) in [
            ("gpu", &mut assignment.gpu),
            ("system-disk", &mut assignment.system_disk),
            ("data-disk", &mut assignment.data_disk),
        ] {
            *slot = record
                .get("assigned")
                .and_then(Value::as_object)
                .and_then(|assigned| assigned.get(key))
                .and_then(Value::as_str)
                .filter(|id| valid_id(id))
                .map(str::to_owned);
        }
        if let Some(names) = record.get("names").and_then(Value::as_object) {
            for (id, name) in names {
                let Some(name) = name.as_str() else {
                    continue;
                };
                let name = name.trim();
                if valid_id(id) && !name.is_empty() {
                    assignment.names.insert(
                        id.clone(),
                        name.chars().take(MAX_NAME_CHARS).collect::<String>(),
                    );
                }
            }
        }
        assignment
    }

    pub fn to_json(&self) -> Value {
        let mut assigned = Map::new();
        for (key, slot) in [
            ("gpu", &self.gpu),
            ("system-disk", &self.system_disk),
            ("data-disk", &self.data_disk),
        ] {
            if let Some(id) = slot {
                assigned.insert(key.to_owned(), Value::String(id.clone()));
            }
        }
        let names: Map<String, Value> = self
            .names
            .iter()
            .map(|(id, name)| (id.clone(), Value::String(name.clone())))
            .collect();
        serde_json::json!({"assigned": assigned, "names": names})
    }

    /// Writes the choice where the providers read it on their next poll.
    pub fn save(&self, directory: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let temp = directory.join("devices.json.new");
        std::fs::write(&temp, format!("{text}\n")).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, directory.join(FILE)).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })
    }

    /// The words a display shows for a device: the person's own name, else the
    /// model the machine reports.
    pub fn name_of<'a>(&'a self, device_id: &str, detected: &'a str) -> &'a str {
        self.names.get(device_id).map_or(detected, String::as_str)
    }
}

/// The host's one id shape: lowercase, digits and `-`, 1–64 characters,
/// starting with a letter or digit.
pub fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    bytes.len() <= MAX_ID_CHARS
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// How a provider is asked for the current choice, so a settings write is seen
/// on the next poll without a restart.
pub type Loader = std::sync::Arc<dyn Fn() -> Assignment + Send + Sync>;

/// Mint an id per device name, in the order the machine reports them. Two
/// devices with one name get `-2`, `-3`, … so an id always names one device.
pub fn ids_for(names: impl IntoIterator<Item = String>) -> Vec<(String, String)> {
    let mut ids = Vec::new();
    let mut taken = std::collections::BTreeSet::new();
    for name in names {
        let slug = crate::vocabulary::semantic_id(&name);
        let base = if slug.is_empty() {
            "device".to_owned()
        } else {
            slug.chars().take(MAX_ID_CHARS).collect::<String>()
        };
        let mut id = base.clone();
        let mut n = 2;
        while taken.contains(&id) {
            let suffix = format!("-{n}");
            let keep = MAX_ID_CHARS.saturating_sub(suffix.len());
            id = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
            n += 1;
        }
        taken.insert(id.clone());
        ids.push((name, id));
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_an_unusable_entry_keeps_what_it_can() {
        let assignment = Assignment::from_json(&serde_json::json!({
            "assigned": {"gpu": "nvidia-geforce-rtx-4090", "system-disk": "Bad Id"},
            "names": {"nvidia-geforce-rtx-4090": "  Main card  ", "x": 7}
        }));
        assert_eq!(assignment.gpu.as_deref(), Some("nvidia-geforce-rtx-4090"));
        assert_eq!(assignment.system_disk, None);
        assert_eq!(
            assignment.name_of("nvidia-geforce-rtx-4090", "NVIDIA GeForce RTX 4090"),
            "Main card"
        );
        // A name the file does not carry is the detected one.
        assert_eq!(assignment.name_of("other", "Detected"), "Detected");
        assert_eq!(
            Assignment::from_json(&serde_json::json!("not an object")),
            Assignment::default()
        );
    }

    #[test]
    fn a_choice_survives_a_save_and_a_reload() {
        let dir = std::env::temp_dir().join(format!(
            "yunari-devices-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        let mut assignment = Assignment {
            gpu: Some("amd-radeon-rx-6800".to_owned()),
            data_disk: Some("wdc-wd30nmvw".to_owned()),
            ..Assignment::default()
        };
        assignment
            .names
            .insert("amd-radeon-rx-6800".to_owned(), "Gaming".to_owned());
        assignment.save(&dir).expect("a written choice");
        assert_eq!(Assignment::load(&dir), assignment);
        // A name longer than a strip can show is shortened, not refused.
        let long = "x".repeat(MAX_NAME_CHARS + 20);
        let stored = Assignment::from_json(&serde_json::json!({"names": {"a": long}}));
        assert_eq!(
            stored.names["a"].chars().count(),
            MAX_NAME_CHARS,
            "a shortened name"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_devices_with_one_name_get_distinct_ids() {
        let ids = ids_for([
            "Samsung SSD 990 PRO 2TB".to_owned(),
            "Samsung SSD 990 PRO 2TB".to_owned(),
        ]);
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0].1, ids[1].1);
        assert!(ids.iter().all(|(_, id)| valid_id(id)));
        assert!(valid_id("nvidia-geforce-rtx-4090"));
        assert!(!valid_id("Bad Id"));
        assert!(!valid_id(""));
        assert!(!valid_id("-leading-dash"));
    }
}
