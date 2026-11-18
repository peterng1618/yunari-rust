//! Whether phones on this network may reach the host.
//!
//! The setting is read when the host starts; changing it takes effect the next
//! time the host binds.

use std::net::{IpAddr, UdpSocket};
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

/// The address a phone on this network reaches. Connecting a UDP socket picks
/// the outbound interface without sending a packet; the host answers this
/// itself, because a locally composed `127.0.0.1` would look right and fail on
/// every phone.
pub fn lan_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("198.51.100.1:443").ok()?;
    socket.local_addr().ok().map(|address| address.ip())
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
