//! Whether phones on this network may reach the host.
//!
//! The setting is read when the host starts; changing it takes effect the next
//! time the host binds.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{read_json, write_json};

pub const FILE: &str = "hosting.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hosting {
    #[serde(default)]
    pub lan: bool,
}

impl Hosting {
    pub fn load(settings_dir: &Path) -> Self {
        read_json(&settings_dir.join(FILE))
    }

    pub fn save(&self, settings_dir: &Path) {
        write_json(&settings_dir.join(FILE), self).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_round_trips_and_starts_local() {
        let dir = std::env::temp_dir().join(format!("yunari-hosting-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        assert!(!Hosting::load(&dir).lan);
        Hosting { lan: true }.save(&dir);
        assert!(Hosting::load(&dir).lan);
        std::fs::remove_dir_all(&dir).ok();
    }
}
