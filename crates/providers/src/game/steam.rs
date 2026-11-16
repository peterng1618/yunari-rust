//! Steam's local library, read from the files Steam itself writes.
//!
//! The install root comes from `HKCU\Software\Valve\Steam\SteamPath`, with the
//! two Program Files locations as a fallback. `libraryfolders.vdf` lists the
//! other drives, and each `appmanifest_*.acf` names one installed app. Box art
//! is a local `librarycache` file: nothing here reaches the network.

use std::path::{Path, PathBuf};

use super::vdf::{self, Value};

#[derive(Clone)]
pub struct SteamApp {
    pub app_id: String,
    pub name: String,
    /// Absolute `<library>\steamapps\common\<installdir>`.
    pub install_dir: PathBuf,
}

/// Where Steam is installed, when this machine has it.
pub fn root() -> Option<PathBuf> {
    registry_root().or_else(|| {
        ["ProgramFiles(x86)", "ProgramFiles"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .map(|program_files| Path::new(&program_files).join("Steam"))
            .find(|candidate| candidate.join("steamapps").is_dir())
    })
}

#[cfg(windows)]
fn registry_root() -> Option<PathBuf> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    let path = PathBuf::from(path);
    path.join("steamapps").is_dir().then_some(path)
}

#[cfg(not(windows))]
fn registry_root() -> Option<PathBuf> {
    None
}

/// The install root plus every folder `libraryfolders.vdf` lists.
pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![root.to_path_buf()];
    if let Some(text) = read(&root.join("steamapps").join("libraryfolders.vdf"))
        && let Some(block) = vdf::parse(&text)
        && let Some(folders) = block.get("libraryfolders").and_then(Value::block)
    {
        for value in folders.values() {
            // Current Steam nests `path`; older versions put the path directly.
            let path = match value {
                Value::Object(folder) => folder.get("path").and_then(Value::text),
                Value::String(path) => Some(path.as_str()),
            };
            let Some(path) = path.map(Path::new) else {
                continue;
            };
            let library = path.to_path_buf();
            if !libraries.contains(&library) && library.join("steamapps").is_dir() {
                libraries.push(library);
            }
        }
    }
    libraries
}

/// Every installed app across every library.
pub fn apps(root: &Path) -> Vec<SteamApp> {
    libraries(root)
        .iter()
        .flat_map(|library| apps_in(library))
        .collect()
}

fn apps_in(library: &Path) -> Vec<SteamApp> {
    let steamapps = library.join("steamapps");
    let Ok(entries) = std::fs::read_dir(&steamapps) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                return None;
            }
            let text = read(&entry.path())?;
            manifest(&text).map(|(app_id, name, install)| SteamApp {
                app_id,
                name,
                install_dir: steamapps.join("common").join(install),
            })
        })
        .collect()
}

/// One manifest's app id, name and install folder.
pub fn manifest(text: &str) -> Option<(String, String, String)> {
    let block = vdf::parse(text)?;
    let state = block.get("AppState").and_then(Value::block)?;
    let app_id = state.get("appid").and_then(Value::text)?;
    let name = state.get("name").and_then(Value::text)?;
    let install = state.get("installdir").and_then(Value::text)?;
    valid_app_id(app_id).then(|| (app_id.to_owned(), name.to_owned(), install.to_owned()))
}

/// The app whose install folder holds `executable`.
pub fn app_for<'a>(apps: &'a [SteamApp], executable: &Path) -> Option<&'a SteamApp> {
    let executable = comparable(executable);
    apps.iter().find(|app| {
        if app.install_dir.as_os_str().is_empty() {
            return false;
        }
        let install = comparable(&app.install_dir);
        executable.starts_with(&install)
    })
}

/// Lower-case, `/`-separated, with a trailing `/` so a sibling folder whose
/// name merely starts the same is not a match.
fn comparable(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if !text.ends_with('/') {
        text.push('/');
    }
    text
}

/// A local cover picture Steam already cached, if it has one.
pub fn box_art(root: &Path, app_id: &str) -> Option<PathBuf> {
    if !valid_app_id(app_id) {
        return None;
    }
    let cache = root.join("appcache").join("librarycache");
    [
        format!("{app_id}_header.jpg"),
        format!("{app_id}_library_600x900.jpg"),
        format!("{app_id}_library_hero.jpg"),
        format!("{app_id}.jpg"),
        format!("{app_id}.png"),
    ]
    .iter()
    .map(|name| cache.join(name))
    .find(|candidate| candidate.is_file())
}

/// A Steam app id is decimal digits, at most 20 of them.
pub fn valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty() && app_id.len() <= 20 && app_id.bytes().all(|byte| byte.is_ascii_digit())
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#""AppState"
{
	"appid"		"730"
	"name"		"Counter-Strike 2"
	"installdir"		"Counter-Strike Global Offensive"
	"StateFlags"		"4"
}
"#;

    #[test]
    fn a_manifest_finds_the_app_id_name_and_install_folder() {
        let (app_id, name, install) = manifest(MANIFEST).expect("a manifest");
        assert_eq!(app_id, "730");
        assert_eq!(name, "Counter-Strike 2");
        assert_eq!(install, "Counter-Strike Global Offensive");
        // An id that is not digits is not an app.
        assert!(manifest(&MANIFEST.replace("\"730\"", "\"../../x\"")).is_none());
        assert!(manifest("not keyvalues").is_none());
    }

    #[test]
    fn a_library_file_names_the_other_drives() {
        let root = std::env::temp_dir().join(format!(
            "yunari-steam-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::write(
            root.join("steamapps").join("libraryfolders.vdf"),
            r#""libraryfolders" { "0" { "path" "C:\\Steam" } }"#,
        )
        .unwrap();
        // A folder the file names but that does not exist is not a library.
        let libraries = libraries(&root);
        assert_eq!(libraries, vec![root.clone()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_executable_is_matched_to_the_library_it_sits_in() {
        let apps = vec![
            SteamApp {
                app_id: "730".to_owned(),
                name: "Counter-Strike 2".to_owned(),
                install_dir: PathBuf::from(
                    r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive",
                ),
            },
            SteamApp {
                app_id: "570".to_owned(),
                name: "Dota 2".to_owned(),
                install_dir: PathBuf::from(r"D:\SteamLibrary\steamapps\common\dota 2 beta"),
            },
        ];
        let executable = Path::new(
            r"d:\steamlibrary\STEAMAPPS\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe",
        );
        assert_eq!(
            app_for(&apps, executable).map(|app| app.app_id.as_str()),
            Some("730")
        );
        // A sibling with a shared prefix is not a match.
        let sibling = Path::new(r"D:\SteamLibrary\steamapps\common\dota 2 betaother\x.exe");
        assert!(app_for(&apps, sibling).is_none());
        assert!(app_for(&apps, Path::new(r"C:\Windows\notepad.exe")).is_none());
    }

    #[test]
    fn box_art_is_only_a_local_cached_file() {
        let root = std::env::temp_dir().join(format!(
            "yunari-steam-art-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        std::fs::create_dir_all(root.join("appcache").join("librarycache")).unwrap();
        std::fs::write(root.join("appcache/librarycache/730_header.jpg"), b"jpg").unwrap();
        assert!(box_art(&root, "730").is_some());
        assert!(box_art(&root, "107410").is_none());
        assert!(box_art(&root, "../730").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
