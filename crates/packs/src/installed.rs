//! The packs installed on this PC, kept under the host's settings folder.
//!
//! A pack is a file, so what is installed is what the folder holds: one
//! directory scan per read, and a file the reader refuses is skipped rather
//! than failing the whole list. Installation validates first and writes a
//! free name, so a bad package never lands and a reinstall never overwrites.

use std::path::{Path, PathBuf};

use crate::package::{self, Pack, Unit};

pub const DIRECTORY: &str = "widget-packs";
pub const SUFFIX: &str = ".yunari-widget-package";

#[derive(Debug, Clone)]
pub struct InstalledPack {
    pub id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub units: Vec<Unit>,
}

/// Every package the folder holds that this build can read.
pub fn installed(directory: &Path) -> Vec<InstalledPack> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut packs: Vec<InstalledPack> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(SUFFIX)?;
            let bytes = std::fs::read(entry.path()).ok()?;
            let pack = package::read(&bytes).ok()?;
            Some(InstalledPack {
                id: id.to_owned(),
                name: pack.name,
                version: pack.version,
                units: pack.units,
            })
        })
        .collect();
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    packs
}

/// Installs one package's bytes, returning what it now offers.
pub fn install(directory: &Path, bytes: &[u8]) -> Result<InstalledPack, String> {
    let Pack {
        name,
        version,
        units,
    } = package::read(bytes)?;
    let id = free_id(directory, &slug(name.as_deref().unwrap_or("pack")));
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let target = directory.join(format!("{id}{SUFFIX}"));
    let scratch = directory.join(format!(".{id}.tmp"));
    std::fs::write(&scratch, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&scratch, &target).map_err(|error| error.to_string())?;
    Ok(InstalledPack {
        id,
        name,
        version,
        units,
    })
}

/// A free id: the name as a path-safe slug, with a suffix when one is taken.
fn free_id(directory: &Path, stem: &str) -> String {
    let mut id = stem.to_owned();
    let mut counter = 2;
    while directory.join(format!("{id}{SUFFIX}")).exists() {
        id = format!("{stem}-{counter}");
        counter += 1;
    }
    id
}

fn slug(name: &str) -> String {
    let mut id: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() {
                char
            } else {
                '-'
            }
        })
        .collect();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_owned();
    if id.is_empty() { "pack".to_owned() } else { id }
}

/// The folder's path under a settings directory.
pub fn directory(settings_dir: &Path) -> PathBuf {
    settings_dir.join(DIRECTORY)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn package_bytes() -> Vec<u8> {
        let widget = serde_json::json!({
            "schemaVersion": "3.0.0",
            "id": "card",
            "metadata": {"name": "Starter cards"},
            "scene": {"objects": [{"type": "Rect", "id": "card"}]},
        });
        let manifest = serde_json::json!({
            "format": package::FORMAT,
            "version": 1,
            "widget": package::WIDGET,
            "units": [{"id": "card", "label": "Card", "objectId": "card"}],
        });
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        writer.start_file("widget.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&widget).unwrap())
            .unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn directory(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "yunari-packs-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn installing_names_the_pack_and_listing_reads_it_back() {
        let dir = directory("install");
        let installed = install(&dir, &package_bytes()).expect("an install");
        assert_eq!(installed.id, "starter-cards");
        assert_eq!(installed.units.len(), 1);
        let listed = self::installed(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "starter-cards");
        assert_eq!(listed[0].units[0].id, "card");
        // A second install lands beside the first rather than over it.
        let again = install(&dir, &package_bytes()).expect("another install");
        assert_eq!(again.id, "starter-cards-2");
        assert_eq!(self::installed(&dir).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_the_reader_refuses_is_not_installed_and_does_not_break_listing() {
        let dir = directory("refuse");
        assert!(install(&dir, b"not a package").is_err());
        assert!(!dir.exists() || self::installed(&dir).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.yunari-widget-package"), b"nope").unwrap();
        assert!(self::installed(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
