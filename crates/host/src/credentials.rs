//! The credentials this host keeps for its providers, under the OS protector.
//!
//! One file holds `{"<id>": "<secret>"}` sealed whole with DPAPI, so a copy of
//! it taken to another Windows account or machine yields nothing. The ids are
//! the ones a settings file names (`weatherapi`, `openweathermap`, a custom
//! source's own id); the secrets are never logged, never returned by a route
//! and never written in plain text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const FILE: &str = "credentials.bin";
/// A credential id is a stable id: lowercase letters, digits, `_` and `-`.
pub const MAX_ID_LEN: usize = 64;

#[derive(Debug, Clone)]
pub struct Credentials {
    directory: PathBuf,
}

impl Credentials {
    pub fn at(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    fn file(&self) -> PathBuf {
        self.directory.join(FILE)
    }

    /// One secret, when it is stored and readable on this account.
    pub fn get(&self, id: &str) -> Option<String> {
        self.read().remove(id)
    }

    /// Ids that have a secret, sorted. The secrets themselves are not part of
    /// this answer, so it cannot leak one by accident.
    pub fn ids(&self) -> Vec<String> {
        self.read().into_keys().collect()
    }

    /// Stores one secret, replacing a previous one. An empty secret removes the
    /// entry: a settings form that was cleared means "forget this key".
    pub fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        if !is_credential_id(id) {
            return Err("A credential needs a stable id.".to_owned());
        }
        let mut entries = self.read();
        let secret = secret.trim();
        if secret.is_empty() {
            entries.remove(id);
        } else {
            entries.insert(id.to_owned(), secret.to_owned());
        }
        self.save(&entries)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut entries = self.read();
        entries.remove(id);
        self.save(&entries)
    }

    fn read(&self) -> BTreeMap<String, String> {
        let Ok(sealed) = std::fs::read(self.file()) else {
            return BTreeMap::new();
        };
        let Ok(plain) = yunari_store::secret::unprotect(&sealed) else {
            return BTreeMap::new();
        };
        let Ok(Value::Object(entries)) = serde_json::from_slice::<Value>(&plain) else {
            return BTreeMap::new();
        };
        entries
            .into_iter()
            .filter(|(id, _)| is_credential_id(id))
            .filter_map(|(id, value)| Some((id, value.as_str()?.to_owned())))
            .filter(|(_, secret)| !secret.is_empty())
            .collect()
    }

    fn save(&self, entries: &BTreeMap<String, String>) -> Result<(), String> {
        if entries.is_empty() {
            match std::fs::remove_file(self.file()) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
        }
        let mut object = Map::new();
        for (id, secret) in entries {
            object.insert(id.clone(), Value::String(secret.clone()));
        }
        let text = serde_json::to_vec(&Value::Object(object)).map_err(|error| error.to_string())?;
        let sealed = yunari_store::secret::protect(&text)?;
        std::fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
        let temp = self.directory.join(format!("{FILE}.new"));
        std::fs::write(&temp, &sealed).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, self.file()).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })
    }
}

/// Lowercase letters, digits, `_` and `-`, at most [`MAX_ID_LEN`].
pub fn is_credential_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunari-credentials-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_secret_survives_a_write_and_read_without_being_plain_text() {
        let dir = directory("round-trip");
        let credentials = Credentials::at(&dir);
        assert!(credentials.ids().is_empty());
        credentials
            .set("weatherapi", "auth-key-123")
            .expect("a stored secret");
        assert_eq!(
            credentials.get("weatherapi").as_deref(),
            Some("auth-key-123")
        );
        assert_eq!(credentials.ids(), vec!["weatherapi".to_owned()]);
        let stored = std::fs::read(credentials.file()).unwrap();
        assert!(
            !stored
                .windows(b"auth-key-123".len())
                .any(|window| window == b"auth-key-123"),
            "the secret reached the file in plain text"
        );
        // On a build without the OS protector the bytes are kept as they are;
        // the round trip still holds, the sealing claim does not.
        #[cfg(not(windows))]
        assert!(stored.windows(12).any(|window| window == b"auth-key-123"));
        credentials.remove("weatherapi").expect("a removed secret");
        assert!(credentials.get("weatherapi").is_none());
        assert!(!credentials.file().exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_secret_forgets_the_entry_and_a_bad_id_is_refused() {
        let dir = directory("empty");
        let credentials = Credentials::at(&dir);
        credentials
            .set("openweathermap", "k")
            .expect("a stored secret");
        credentials
            .set("openweathermap", "  ")
            .expect("a cleared secret");
        assert!(credentials.get("openweathermap").is_none());
        assert!(credentials.set("Not An Id", "k").is_err());
        assert!(credentials.set("openweathermap:", "k").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_damaged_file_reads_as_no_credentials() {
        let dir = directory("damaged");
        std::fs::write(dir.join(FILE), b"not a sealed document").unwrap();
        let credentials = Credentials::at(&dir);
        assert!(credentials.get("weatherapi").is_none());
        assert!(credentials.ids().is_empty());
        // A write after the damage starts a fresh file rather than failing.
        credentials.set("weatherapi", "k").expect("a stored secret");
        assert_eq!(credentials.get("weatherapi").as_deref(), Some("k"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
