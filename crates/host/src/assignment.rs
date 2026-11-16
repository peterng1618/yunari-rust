//! Which theme each display shows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::write_json;
use crate::displays::{DEFAULT_DISPLAY, RETIRED_MACHINE_DISPLAY_ID};
use yunari_library::Library;

pub const FILE: &str = "active-theme.json";

#[derive(Debug, Clone)]
pub struct Assignment {
    path: PathBuf,
    map: BTreeMap<String, String>,
}

impl Assignment {
    pub fn load(settings_dir: &Path) -> Self {
        let path = settings_dir.join(FILE);
        let raw: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let mut map = BTreeMap::new();
        if let Value::Object(object) = raw {
            // The first file held the theme under `id`; that machine row is
            // the default display now.
            if let Some(Value::String(id)) = object.get("id") {
                map.insert(DEFAULT_DISPLAY.to_owned(), id.clone());
            }
            for (display, value) in object {
                let Value::String(theme) = value else {
                    continue;
                };
                let display = if display == RETIRED_MACHINE_DISPLAY_ID {
                    DEFAULT_DISPLAY.to_owned()
                } else {
                    display
                };
                map.insert(display, theme);
            }
        }
        Self { path, map }
    }

    pub fn get(&self, display: &str) -> Option<&str> {
        self.map.get(display).map(String::as_str)
    }

    pub fn map(&self) -> &BTreeMap<String, String> {
        &self.map
    }

    pub fn set(&mut self, display: &str, theme: Option<&str>) {
        match theme {
            Some(theme) => {
                self.map.insert(display.to_owned(), theme.to_owned());
            }
            None => {
                self.map.remove(display);
            }
        }
        // A settings directory that cannot be written still serves this run.
        write_json(&self.path, &self.map).ok();
    }

    /// The theme a display should paint: an explicit request that exists, then
    /// its assignment, then the default's, then the only theme in the library.
    pub fn resolve(
        &self,
        library: &Library,
        display: &str,
        requested: Option<&str>,
    ) -> Option<String> {
        if let Some(id) = requested
            && library.read(id).is_ok()
        {
            return Some(id.to_owned());
        }
        if let Some(id) = self.get(display)
            && library.read(id).is_ok()
        {
            return Some(id.to_owned());
        }
        if display != DEFAULT_DISPLAY
            && let Some(id) = self.get(DEFAULT_DISPLAY)
            && library.read(id).is_ok()
        {
            return Some(id.to_owned());
        }
        let themes = library.list();
        (themes.len() == 1).then(|| themes[0].id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunari-assign-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn an_assignment_round_trips() {
        let dir = dir("round");
        let mut assignment = Assignment::load(&dir);
        assignment.set("phone", Some("night"));
        let reloaded = Assignment::load(&dir);
        assert_eq!(reloaded.get("phone"), Some("night"));
        assignment.set("phone", None);
        assert_eq!(Assignment::load(&dir).get("phone"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_old_machine_row_moves_to_the_default() {
        let dir = dir("legacy");
        std::fs::write(dir.join(FILE), r#"{"id":"retro","this-pc":"retro"}"#).unwrap();
        let assignment = Assignment::load(&dir);
        assert_eq!(assignment.get(DEFAULT_DISPLAY), Some("retro"));
        assert_eq!(assignment.get(RETIRED_MACHINE_DISPLAY_ID), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
