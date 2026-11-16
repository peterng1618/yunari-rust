//! The SignalRGB colour receiver's settings.
//!
//! The port is a fact about this machine, so it lives with the other host
//! settings rather than in a theme. Only the port is stored: every other value
//! on the seam is fixed by the contract.
//!
//! A port this host cannot bind is refused rather than clamped to a neighbour
//! the owner never named, so the file's own normaliser and the settings route
//! cannot disagree.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use yunari_panels::signalrgb::DEFAULT_SIGNALRGB_UDP_PORT;

pub const FILE: &str = "signalrgb-bridge.json";

#[derive(Debug, Clone)]
pub struct Settings {
    path: PathBuf,
    udp_port: u16,
}

impl Settings {
    pub fn load(directory: &Path) -> Self {
        let path = directory.join(FILE);
        let raw: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let udp_port = raw
            .get("udpPort")
            .and_then(port_of)
            .unwrap_or(DEFAULT_SIGNALRGB_UDP_PORT);
        Self { path, udp_port }
    }

    /// The port the receiver should bind.
    pub fn port(&self) -> u16 {
        self.udp_port
    }

    /// Stores a whole-number port from 1 to 65535 and answers what was stored.
    pub fn write_udp_port(&mut self, value: &Value) -> Result<u16, String> {
        let port = port_of(value).ok_or_else(|| {
            "The bridge's udpPort must be a whole number from 1 to 65535.".to_owned()
        })?;
        crate::config::write_json(&self.path, &json!({ "udpPort": port }))
            .map_err(|_| "The bridge port could not be saved.".to_owned())?;
        self.udp_port = port;
        Ok(port)
    }
}

/// A port a socket can bind: a whole number inside the 16-bit range.
fn port_of(value: &Value) -> Option<u16> {
    let number = value.as_f64()?;
    if number.fract() != 0.0 || !(1.0..=65_535.0).contains(&number) {
        return None;
    }
    Some(number as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "yunari-signalrgb-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|time| time.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).ok();
        root
    }

    #[test]
    fn the_contract_s_port_is_the_default_and_a_stored_one_wins() {
        let root = directory();
        let mut settings = Settings::load(&root);
        assert_eq!(settings.port(), DEFAULT_SIGNALRGB_UDP_PORT);
        assert_eq!(settings.write_udp_port(&json!(9000)).ok(), Some(9000));
        assert_eq!(Settings::load(&root).port(), 9000);
        // A value no socket can bind is refused, and the stored port stays.
        assert!(settings.write_udp_port(&json!(0)).is_err());
        assert!(settings.write_udp_port(&json!(65536)).is_err());
        assert!(settings.write_udp_port(&json!(1.5)).is_err());
        assert!(settings.write_udp_port(&json!("9000")).is_err());
        assert_eq!(Settings::load(&root).port(), 9000);
        let _ = std::fs::remove_dir_all(root);
    }
}
