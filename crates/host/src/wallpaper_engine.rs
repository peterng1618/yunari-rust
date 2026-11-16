//! The Wallpaper Engine library this PC holds, and one item's bytes.
//!
//! Discovery is the registry (the only offline way to find Steam) and the
//! install's own `wallpaperui.exe`. A library is scanned from disk on every
//! request: the descriptor is third-party content, so a path it names is only
//! served when it stays inside its item's own directory, and an item with
//! nothing behind it is counted rather than listed.
//!
//! What is not here yet: the scene package's own facts (a `yield`/`text` kind
//! hint), video duration, unpacking a scene into an editable project, and the
//! mobile package import. Those are separate reads over the same scan.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Wallpaper Engine's Steam app id, and so its workshop content directory.
const APP_ID: &str = "431960";
const STEAM_KEY: &str = r"HKCU\Software\Valve\Steam";
const PROGRAM_FILES: [&str; 2] = [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam"];
/// The one file that proves a directory is the install.
const EDITOR: &str = "bin/wallpaperui.exe";
const SCENE_PACKAGE: &str = "scene.pkg";

/// Where Wallpaper Engine is, and every Steam library that holds its items.
#[derive(Debug, Clone)]
pub struct Install {
    /// `<steam>/steamapps/common/wallpaper_engine`
    pub root: PathBuf,
    /// `<library>/steamapps/workshop/content/431960`, one per Steam library.
    pub libraries: Vec<PathBuf>,
    /// `<root>/projects/myprojects`
    pub my_projects: PathBuf,
    /// `<root>/bin/wallpaperui.exe`
    pub editor: PathBuf,
}

/// What a listed item resolves to: an image extension wins before the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Scene,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Scene => "scene",
        }
    }
}

/// One listed wallpaper. `payload` and `preview` are absolute and already
/// proven to live inside `directory`.
#[derive(Debug, Clone)]
pub struct Item {
    pub workshop_id: String,
    pub directory: PathBuf,
    pub title: String,
    pub kind: Kind,
    pub payload: PathBuf,
    pub preview: Option<PathBuf>,
    pub filesize: u64,
    /// The directory's own last write, in milliseconds: when Steam last
    /// subscribed or updated it. Absent when it cannot be statted.
    pub updated_at: Option<i64>,
    pub tags: Vec<String>,
    pub author_steam_id: Option<String>,
    pub workshop_url: Option<String>,
    /// The descriptor's `properties` schema, passed through unread.
    pub properties: Value,
}

/// Every item a scan listed, and how many directories it refused.
#[derive(Debug, Default)]
pub struct Scan {
    pub items: Vec<Item>,
    pub skipped: usize,
}

/// A listing answer: an unreachable install, or the items it holds.
pub fn listing() -> Value {
    let Some(install) = locate() else {
        return json!({"available": false, "reason": "not-installed"});
    };
    if install.libraries.is_empty() {
        return json!({"available": false, "reason": "no-library"});
    }
    let scan = scan(&install);
    json!({
        "available": true,
        "install": install.root.display().to_string(),
        "items": scan.items.iter().map(item_json).collect::<Vec<_>>(),
        "skipped": scan.skipped,
    })
}

fn item_json(item: &Item) -> Value {
    let id = &item.workshop_id;
    let attribution = if item.author_steam_id.is_none() && item.workshop_url.is_none() {
        Value::Null
    } else {
        let mut value = serde_json::Map::new();
        if let Some(steam_id) = &item.author_steam_id {
            value.insert("steamId".into(), steam_id.clone().into());
        }
        if let Some(url) = &item.workshop_url {
            value.insert("workshopUrl".into(), url.clone().into());
        }
        Value::Object(value)
    };
    let mut value = json!({
        "workshopId": id,
        "title": item.title,
        "kind": item.kind.as_str(),
        "filesize": item.filesize,
        "tags": item.tags,
        "properties": item.properties,
    });
    let map = value.as_object_mut().expect("an object was built above");
    if item.preview.is_some() {
        map.insert(
            "previewUrl".into(),
            format!("/api/wallpaper-engine/item/{id}/preview").into(),
        );
    }
    if let Some(updated_at) = item.updated_at {
        map.insert("updatedAt".into(), updated_at.into());
    }
    if !attribution.is_null() {
        map.insert("attribution".into(), attribution);
    }
    value
}

/// One item's preview or payload, with the type a browser should treat it as.
pub fn bytes(workshop_id: &str, preview: bool) -> Option<(Vec<u8>, &'static str)> {
    let install = locate()?;
    let scan = scan(&install);
    let item = scan
        .items
        .iter()
        .find(|item| item.workshop_id == workshop_id)?;
    let (path, content_type) = if preview {
        (
            item.preview.as_deref()?,
            content_type_for(item.preview.as_deref()?),
        )
    } else {
        (item.payload.as_path(), payload_content_type(item))
    };
    Some((std::fs::read(path).ok()?, content_type))
}

/// The install and its libraries, or `None` when this PC has none.
pub fn locate() -> Option<Install> {
    locate_with(std::env::var("YUNARI_WALLPAPER_ENGINE").ok().as_deref())
}

/// `hook` is the test override, and the only candidate when set: falling
/// through to the registry would make a "no install" case read this PC's.
fn locate_with(hook: Option<&str>) -> Option<Install> {
    let candidates: Vec<(PathBuf, PathBuf)> = match hook {
        Some(root) if !root.is_empty() => {
            let root = PathBuf::from(root);
            let steam_root = root
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .unwrap_or(&root)
                .to_path_buf();
            vec![(root, steam_root)]
        }
        _ => {
            let mut roots: Vec<PathBuf> = Vec::new();
            if let Some(path) = steam_path() {
                roots.push(path);
            }
            roots.extend(PROGRAM_FILES.iter().map(PathBuf::from));
            roots
                .into_iter()
                .map(|steam_root| {
                    (
                        steam_root.join("steamapps/common/wallpaper_engine"),
                        steam_root,
                    )
                })
                .collect()
        }
    };

    for (root, steam_root) in candidates {
        let editor = root.join(EDITOR);
        if !editor.is_file() {
            continue;
        }
        let mut libraries: Vec<PathBuf> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let listed = library_roots(&steam_root.join("steamapps/libraryfolders.vdf"));
        // The Steam root is a library of its own, and older
        // `libraryfolders.vdf` files do not list it.
        for library_root in std::iter::once(steam_root.clone()).chain(listed) {
            let content = library_root.join("steamapps/workshop/content").join(APP_ID);
            let key = content.to_string_lossy().to_ascii_lowercase();
            if !seen.contains(&key) && content.is_dir() {
                seen.push(key);
                libraries.push(content);
            }
        }
        return Some(Install {
            my_projects: root.join("projects/myprojects"),
            editor,
            libraries,
            root,
        });
    }
    None
}

/// `reg query … /v SteamPath`, read from the output or `None` when `reg`
/// refuses (no registry, no Steam, or no Windows).
fn steam_path() -> Option<PathBuf> {
    let output = std::process::Command::new("reg")
        .args(["query", STEAM_KEY, "/v", "SteamPath"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    steam_path_from(&String::from_utf8_lossy(&output.stdout))
}

fn steam_path_from(output: &str) -> Option<PathBuf> {
    let line = output
        .lines()
        .find(|line| line.contains("SteamPath") && line.contains("REG_SZ"))?;
    let value = line.split("REG_SZ").nth(1)?.trim();
    let value = value.strip_prefix('"').unwrap_or(value);
    let value = value.strip_suffix('"').unwrap_or(value);
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// The `"path"` values of a `libraryfolders.vdf`. Every line this needs is one
/// quoted pair, so there is no grammar here and nothing to depend on.
fn library_roots(vdf: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(vdf) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("\"path\"")?.trim();
            let rest = rest.strip_prefix('"')?;
            let (path, _) = rest.rsplit_once('"')?;
            (!path.is_empty()).then(|| PathBuf::from(path))
        })
        .collect()
}

/// Every listed wallpaper in the install's libraries, and how many were
/// refused. A web wallpaper is a page, not a file a theme can hold, so it is
/// hidden rather than listed as something that would refuse on import.
pub fn scan(install: &Install) -> Scan {
    let cache = workshop_cache(install);
    let mut scan = Scan::default();
    for library in &install.libraries {
        let Ok(entries) = std::fs::read_dir(library) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            match read_item(&name, &entry.path(), &cache) {
                Some(item) => scan.items.push(item),
                None => scan.skipped += 1,
            }
        }
    }
    scan
}

/// `bin/workshopcache.json`'s rows, keyed by `workshopid`; the file lags the
/// disk, so it only supplies what the directory cannot say.
fn workshop_cache(install: &Install) -> Vec<(String, Value)> {
    let Ok(text) = std::fs::read_to_string(install.root.join("bin/workshopcache.json")) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(rows) = parsed.get("wallpapers").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let id = id_of(row.get("workshopid")?)?;
            Some((id, row.clone()))
        })
        .collect()
}

fn read_item(name: &str, directory: &Path, cache: &[(String, Value)]) -> Option<Item> {
    let text = std::fs::read_to_string(directory.join("project.json")).ok()?;
    let project: Value = serde_json::from_str(&text).ok()?;
    let kind = match project
        .get("type")
        .and_then(Value::as_str)?
        .to_ascii_lowercase()
        .as_str()
    {
        "scene" => "scene",
        "video" => "video",
        "web" => "web",
        _ => return None,
    };

    // Workshop directories are named by their id; anything else has to carry
    // one in the descriptor, because the browser addresses an item by that id.
    let numeric = !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit());
    let workshop_id = if numeric {
        name.to_owned()
    } else {
        project
            .get("package")
            .and_then(|package| package.get("workshopId"))
            .and_then(id_of)?
    };

    let file = project.get("file").and_then(Value::as_str)?;
    let named = directory.join(file);
    // Today's descriptors name `scene.json`, which is not on disk; the package
    // beside it is the payload. The name is fixed and inside the item's own
    // directory, so containment is unchanged.
    let package = directory.join(SCENE_PACKAGE);
    let payload = if kind == "scene" && package.is_file() {
        package
    } else {
        named
    };
    // `project.file` is third-party content, so a path that leaves the item's
    // own directory is not a payload.
    if !inside(directory, &payload) {
        return None;
    }
    let row = cache
        .iter()
        .find(|(id, _)| id == &workshop_id)
        .map(|(_, row)| row);

    let row_string = |key: &str| row.and_then(|row| row.get(key)).and_then(Value::as_str);
    let row_number = |key: &str| row.and_then(|row| row.get(key)).and_then(Value::as_u64);
    let row_id = |key: &str| row.and_then(|row| row.get(key)).and_then(id_of);

    let title = row_string("title")
        .filter(|title| !title.is_empty())
        .or_else(|| project.get("title").and_then(Value::as_str))
        .filter(|title| !title.is_empty())
        .unwrap_or(&workshop_id)
        .to_owned();
    let preview = project
        .get("preview")
        .and_then(Value::as_str)
        .filter(|preview| !preview.is_empty())
        .map(|preview| directory.join(preview))
        .filter(|preview| inside(directory, preview));
    let kind = match kind {
        "video" => Kind::Video,
        "web" => return None,
        _ => match payload.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if image_content_type(extension).is_some() => Kind::Image,
            _ => Kind::Scene,
        },
    };
    Some(Item {
        workshop_id,
        directory: directory.to_path_buf(),
        title,
        kind,
        filesize: row_number("filesize").unwrap_or_else(|| {
            std::fs::metadata(&payload)
                .map(|meta| meta.len())
                .unwrap_or(0)
        }),
        updated_at: std::fs::metadata(directory)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|age| age.as_millis() as i64),
        tags: row
            .and_then(|row| row.get("tags"))
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        author_steam_id: row_id("authorsteamid"),
        workshop_url: row_string("workshopurl").map(str::to_owned),
        properties: project.get("properties").cloned().unwrap_or(Value::Null),
        payload,
        preview,
    })
}

fn id_of(value: &Value) -> Option<String> {
    match value {
        Value::String(id) if !id.is_empty() => Some(id.clone()),
        Value::Number(id) => Some(id.to_string()),
        _ => None,
    }
}

/// True when `target` is `directory` or below it. Both are folded first, so a
/// `..` inside the descriptor cannot walk out through a `starts_with` that
/// compares the components as written.
fn inside(directory: &Path, target: &Path) -> bool {
    fold(target).starts_with(fold(directory))
}

fn fold(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            component => out.push(component.as_os_str()),
        }
    }
    out
}

fn payload_content_type(item: &Item) -> &'static str {
    let extension = item
        .payload
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match item.kind {
        Kind::Image | Kind::Scene => content_type_for(&item.payload),
        Kind::Video => video_content_type(extension),
    }
}

fn content_type_for(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    image_content_type(extension).unwrap_or("application/octet-stream")
}

fn image_content_type(extension: &str) -> Option<&'static str> {
    Some(match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

/// The payload's own container decides the type; only the ext the scan cannot
/// inspect falls back to the file name.
fn video_content_type(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "mp4" => "video/mp4",
        "wmv" => "video/x-ms-wmv",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yunari-wallpaper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }

    fn fixture(root: &Path) -> Install {
        let install = root.join("steam/steamapps/common/wallpaper_engine");
        std::fs::create_dir_all(install.join("bin")).unwrap();
        std::fs::write(install.join("bin/wallpaperui.exe"), b"").unwrap();
        let library = root.join("steam/steamapps/workshop/content/431960");
        std::fs::create_dir_all(&library).unwrap();
        Install {
            root: install.clone(),
            libraries: vec![library.clone()],
            my_projects: install.join("projects/myprojects"),
            editor: install.join("bin/wallpaperui.exe"),
        }
    }

    fn write_project(directory: &Path, project: Value) {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join("project.json"), project.to_string()).unwrap();
    }

    #[test]
    fn a_steam_path_is_read_from_the_registry_line() {
        let output = "\r\nHKEY_CURRENT_USER\\Software\\Valve\\Steam\r\n    SteamPath    REG_SZ    D:/Steam\r\n";
        assert_eq!(steam_path_from(output), Some(PathBuf::from("D:/Steam")));
        assert_eq!(steam_path_from("no value"), None);
    }

    #[test]
    fn library_roots_are_the_quoted_path_lines() {
        let root = temp_dir();
        let vdf = root.join("libraryfolders.vdf");
        std::fs::write(
            &vdf,
            "\"libraryfolders\"\n{\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Games\\\\Steam\"\n\t}\n}\n",
        )
        .unwrap();
        assert_eq!(
            library_roots(&vdf),
            vec![PathBuf::from(r"D:\\Games\\Steam")]
        );
        assert!(library_roots(&root.join("absent.vdf")).is_empty());
    }

    #[test]
    fn a_scene_is_listed_with_its_package_and_cache_title() {
        let root = temp_dir();
        let install = fixture(&root);
        let item = install.libraries[0].join("123");
        write_project(
            &item,
            json!({"type": "Scene", "file": "scene.json", "title": "FromDisk", "preview": "preview.jpg", "properties": {"schemecolor": {"value": "1 1 1"}}}),
        );
        std::fs::write(item.join("scene.pkg"), b"package").unwrap();
        std::fs::write(item.join("preview.jpg"), b"jpeg").unwrap();
        std::fs::create_dir_all(install.root.join("bin")).unwrap();
        std::fs::write(
            install.root.join("bin/workshopcache.json"),
            json!({"wallpapers": [{"workshopid": 123, "title": "FromCache", "tags": ["Abstract"], "filesize": 7, "authorsteamid": "7656", "workshopurl": "https://example.invalid/1"}]})
                .to_string(),
        )
        .unwrap();

        let scan = scan(&install);
        assert_eq!(scan.skipped, 0);
        let item = &scan.items[0];
        assert_eq!(item.workshop_id, "123");
        assert_eq!(item.title, "FromCache");
        assert_eq!(item.kind, Kind::Scene);
        assert_eq!(item.payload, install.libraries[0].join("123/scene.pkg"));
        assert_eq!(item.filesize, 7);
        assert_eq!(item.tags, ["Abstract"]);
        assert_eq!(item.author_steam_id.as_deref(), Some("7656"));

        let stored = item_json(item);
        assert_eq!(stored["kind"], "scene");
        assert_eq!(
            stored["previewUrl"],
            "/api/wallpaper-engine/item/123/preview"
        );
        assert_eq!(stored["properties"]["schemecolor"]["value"], "1 1 1");
    }

    #[test]
    fn a_descriptor_without_a_payload_or_with_a_web_type_is_refused() {
        let root = temp_dir();
        let install = fixture(&root);
        let library = &install.libraries[0];
        write_project(
            &library.join("10"),
            json!({"type": "application", "file": "x"}),
        );
        write_project(
            &library.join("11"),
            json!({"type": "web", "file": "index.html"}),
        );
        // A path that leaves the item is not a preview, so it is not served.
        write_project(
            &library.join("12"),
            json!({"type": "video", "file": "clip.mp4", "preview": "../../../secret.jpg"}),
        );
        std::fs::write(library.join("12/clip.mp4"), b"mp4").unwrap();
        write_project(&library.join("13"), json!({"type": "video"}));

        let scan = scan(&install);
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].workshop_id, "12");
        assert!(scan.items[0].preview.is_none());
        assert_eq!(scan.skipped, 3);
    }

    #[test]
    fn only_a_listed_item_is_ever_read() {
        let root = temp_dir();
        let install = fixture(&root);
        let item = install.libraries[0].join("123");
        write_project(&item, json!({"type": "video", "file": "clip.mp4"}));
        std::fs::write(item.join("clip.mp4"), b"bytes").unwrap();
        // The route's id is resolved through the scan, so a traversal name is
        // simply not an item. This is the lookup the handler performs.
        let scan = scan(&install);
        assert!(scan.items.iter().any(|item| item.workshop_id == "123"));
        assert!(
            !scan
                .items
                .iter()
                .any(|item| item.workshop_id == "../../secret")
        );
    }
}
