//! Where each library theme came from, kept in the host's settings directory.
//!
//! "Installed" is a fact about this machine's copy, not about the catalogue
//! item: the item is the same for everyone, while this record exists only where
//! someone installed it. The file is
//! `{"<theme id>": {"catalogueId": …, "version": …, "installedAt": …}}`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const FILE: &str = "theme-origins.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub catalogue_id: String,
    pub version: String,
    pub installed_at: String,
}

pub struct Origins {
    directory: PathBuf,
}

impl Origins {
    pub fn at(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    fn file(&self) -> PathBuf {
        self.directory.join(FILE)
    }

    /// Every entry this build can use. A hand-edited file must not blank the
    /// whole map, so a bad entry is dropped rather than failing the read.
    pub fn read(&self) -> BTreeMap<String, Origin> {
        let Some(text) = std::fs::read_to_string(self.file()).ok() else {
            return BTreeMap::new();
        };
        let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(&text) else {
            return BTreeMap::new();
        };
        entries
            .into_iter()
            .filter(|(id, _)| yunari_model::validate::is_stable_id(id))
            .filter_map(|(id, value)| Some((id, origin_of(&value)?)))
            .collect()
    }

    pub fn write(&self, theme_id: &str, origin: &Origin) -> Result<(), String> {
        let mut entries = self.read();
        entries.insert(theme_id.to_owned(), origin.clone());
        self.save(&entries)
    }

    /// One theme's entry, for a theme that has been deleted.
    pub fn remove(&self, theme_id: &str) -> Result<(), String> {
        let mut entries = self.read();
        entries.remove(theme_id);
        self.save(&entries)
    }

    /// The library id a catalogue item landed under, when this machine has it.
    pub fn theme_of(&self, catalogue_id: &str) -> Option<String> {
        self.read()
            .into_iter()
            .find(|(_, origin)| origin.catalogue_id == catalogue_id)
            .map(|(id, _)| id)
    }

    fn save(&self, entries: &BTreeMap<String, Origin>) -> Result<(), String> {
        let mut object = Map::new();
        for (id, origin) in entries {
            object.insert(
                id.clone(),
                serde_json::json!({
                    "catalogueId": origin.catalogue_id,
                    "version": origin.version,
                    "installedAt": origin.installed_at,
                }),
            );
        }
        let text = serde_json::to_string_pretty(&Value::Object(object))
            .map_err(|error| error.to_string())?;
        std::fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
        let temp = self.directory.join(format!("{FILE}.new"));
        std::fs::write(&temp, format!("{text}\n")).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, self.file()).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })
    }
}

/// Three usable strings, or nothing.
fn origin_of(value: &Value) -> Option<Origin> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Some(Origin {
        catalogue_id: text("catalogueId")?,
        version: text("version")?,
        installed_at: text("installedAt")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunari-origins-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn origin(catalogue_id: &str, version: &str) -> Origin {
        Origin {
            catalogue_id: catalogue_id.to_owned(),
            version: version.to_owned(),
            installed_at: "2026-11-16T10:00:00Z".to_owned(),
        }
    }

    #[test]
    fn an_origin_survives_a_write_and_finds_its_theme() {
        let dir = directory("round-trip");
        let origins = Origins::at(&dir);
        assert!(origins.read().is_empty());
        origins
            .write(
                "aurora-copy",
                &origin("0123abcd-4567-89ab-cdef-0123456789ab", "1.0.0"),
            )
            .expect("a written origin");
        assert_eq!(
            origins.read().get("aurora-copy"),
            Some(&origin("0123abcd-4567-89ab-cdef-0123456789ab", "1.0.0"))
        );
        assert_eq!(
            origins
                .theme_of("0123abcd-4567-89ab-cdef-0123456789ab")
                .as_deref(),
            Some("aurora-copy")
        );
        origins.remove("aurora-copy").expect("a removed origin");
        assert!(origins.read().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bad_entry_is_dropped_without_blanking_the_file() {
        let dir = directory("bad-entry");
        let origins = Origins::at(&dir);
        std::fs::write(
            dir.join(FILE),
            r#"{
                "good": {"catalogueId":"a","version":"1.0.0","installedAt":"now"},
                "bad": {"catalogueId":"a"},
                "not an id!": {"catalogueId":"b","version":"1.0.0","installedAt":"now"}
            }"#,
        )
        .unwrap();
        let entries = origins.read();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("good"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
