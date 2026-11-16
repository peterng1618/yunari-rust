//! This host's name and the id a phone stores to recognise it again.

use std::path::Path;

use rand::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::write_json;

pub const MAX_HOST_NAME_LENGTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostIdentity {
    pub id: String,
    pub name: String,
}

impl HostIdentity {
    /// The file `host-identity.json`, minting one on first use.
    pub fn load_or_mint(settings_dir: &Path) -> Self {
        let path = settings_dir.join("host-identity.json");
        let stored: Option<HostIdentity> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        if let Some(stored) = stored
            && is_stable_identity(&stored)
        {
            return stored;
        }
        let minted = Self {
            id: mint_id(),
            name: machine_name(),
        };
        // A settings directory that cannot be written still gets a working
        // host for this run; the next run mints again.
        write_json(&path, &minted).ok();
        minted
    }
}

fn is_stable_identity(identity: &HostIdentity) -> bool {
    identity.id.len() == 16
        && identity.id.chars().all(|c| c.is_ascii_hexdigit())
        && !identity.name.trim().is_empty()
        && identity.name.chars().count() <= MAX_HOST_NAME_LENGTH
}

fn mint_id() -> String {
    let mut bytes = [0u8; 8];
    rand::rng().fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn machine_name() -> String {
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".to_owned());
    let name: String = name.chars().take(MAX_HOST_NAME_LENGTH).collect();
    if name.trim().is_empty() {
        "This PC".to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yunari-identity-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn an_identity_is_minted_once() {
        let dir = dir("once");
        let first = HostIdentity::load_or_mint(&dir);
        assert_eq!(first.id.len(), 16);
        assert!(first.id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(first.name.chars().count() <= MAX_HOST_NAME_LENGTH);
        let second = HostIdentity::load_or_mint(&dir);
        assert_eq!(first, second);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_broken_file_is_replaced() {
        let dir = dir("broken");
        std::fs::write(dir.join("host-identity.json"), r#"{"id":"nope"}"#).unwrap();
        let identity = HostIdentity::load_or_mint(&dir);
        assert_eq!(identity.id.len(), 16);
        std::fs::remove_dir_all(&dir).ok();
    }
}
