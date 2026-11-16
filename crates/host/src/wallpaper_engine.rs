//! The Wallpaper Engine library this PC holds, and one item's bytes.
//!
//! Discovery is the registry (the only offline way to find Steam) and the
//! install's own `wallpaperui.exe`. A library is scanned from disk on every
//! request: the descriptor is third-party content, so a path it names is only
//! served when it stays inside its item's own directory, and an item with
//! nothing behind it is counted rather than listed.
//!
//! A scene package is opened far enough to say whether it holds still and
//! whether it draws text: keyframes anywhere in the tree, particles, particles
//! layers or a model member with keyframes all mean it moves. A video's bytes
//! decide its container and its own box headers give its length, so a row never
//! states a guess the file could contradict.
//!
//! What is not here yet: unpacking a scene into an editable project and the
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
    /// A scene's own facts. Absent when the package cannot be read: never
    /// assumed still.
    pub at_rest: Option<bool>,
    pub has_text: Option<bool>,
    /// A video's sniffed container and its own reported length.
    pub container: Option<&'static str>,
    pub duration_ms: Option<u64>,
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
    listing_of(&install)
}

/// The same answer, kept in the settings folder until anything the scan read
/// changes. A scan opens every descriptor and every scene package; the
/// fingerprint only stats them, so a listing costs one pass over the disk.
pub fn listing_cached(settings: &Path) -> Value {
    let Some(install) = locate() else {
        return json!({"available": false, "reason": "not-installed"});
    };
    if install.libraries.is_empty() {
        return json!({"available": false, "reason": "no-library"});
    }
    cached_listing(&install, settings)
}

fn cached_listing(install: &Install, settings: &Path) -> Value {
    let path = settings.join(CACHE_FILE);
    let fingerprint = fingerprint(install);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(cached) = serde_json::from_str::<Value>(&text)
        && cached["version"] == CACHE_VERSION
        && cached["fingerprint"] == fingerprint
        && let Some(listing) = cached.get("listing")
    {
        return listing.clone();
    }
    let listing = listing_of(install);
    if let Ok(text) = serde_json::to_string(&json!({
        "version": CACHE_VERSION,
        "fingerprint": fingerprint,
        "listing": listing,
    })) {
        // A torn read is worse than a miss, so the cache swaps in whole.
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, text).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
    }
    listing
}

/// Bumped whenever the entry's shape or the scan's meaning changes.
const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = "wallpaper-listing.json";

fn listing_of(install: &Install) -> Value {
    let scan = scan(install);
    json!({
        "available": true,
        "install": install.root.display().to_string(),
        "items": scan.items.iter().map(item_json).collect::<Vec<_>>(),
        "skipped": scan.skipped,
    })
}

/// What the scan read, as a string that changes when anything it depends on
/// changes: the install, its workshop cache, every library, and each item's
/// descriptor and scene package. Sorted so two passes over one library agree.
fn fingerprint(install: &Install) -> String {
    let mut rows = vec![install.root.display().to_string().to_ascii_lowercase()];
    rows.push(format!(
        "cache:{}",
        stamp(&install.root.join("bin/workshopcache.json"))
    ));
    let mut libraries = install.libraries.clone();
    libraries.sort_by_key(|library| library.display().to_string().to_ascii_lowercase());
    for library in &libraries {
        rows.push(format!(
            "library:{}:{}",
            library.display().to_string().to_ascii_lowercase(),
            stamp(library)
        ));
        let mut directories: Vec<PathBuf> = std::fs::read_dir(library)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        directories.sort_by_key(|directory| directory.display().to_string().to_ascii_lowercase());
        for directory in directories {
            rows.push(format!(
                "item:{}:{}:{}",
                directory.display().to_string().to_ascii_lowercase(),
                stamp(&directory.join("project.json")),
                stamp(&directory.join(SCENE_PACKAGE))
            ));
        }
    }
    rows.join("\n")
}

fn stamp(path: &Path) -> String {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| {
            let modified = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?;
            Some(format!("{}:{}", modified.as_millis(), meta.len()))
        })
        .unwrap_or_else(|| "-".to_owned())
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
    if let Some(at_rest) = item.at_rest {
        map.insert("stillAtRest".into(), at_rest.into());
    }
    if let Some(has_text) = item.has_text {
        map.insert("hasText".into(), has_text.into());
    }
    if let Some(container) = item.container {
        map.insert("container".into(), container.into());
    }
    if let Some(duration_ms) = item.duration_ms {
        map.insert("durationMs".into(), duration_ms.into());
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
    let facts = match kind {
        Kind::Scene => std::fs::read(&payload)
            .ok()
            .and_then(|bytes| scene_facts(&bytes)),
        _ => None,
    };
    let video = match kind {
        Kind::Video => Some((video_container(&payload), video_duration_ms(&payload))),
        _ => None,
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
        at_rest: facts.as_ref().map(|facts| facts.at_rest),
        has_text: facts.as_ref().map(|facts| facts.has_text),
        container: video.as_ref().map(|video| video.0),
        duration_ms: video.as_ref().and_then(|video| video.1),
        payload,
        preview,
    })
}

/// A scene package's own two facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneFacts {
    /// Nothing moves without input: no keyframes, particles or animation
    /// layers anywhere in the scene or its models.
    pub at_rest: bool,
    pub has_text: bool,
}

/// One package table: `PKGV`/`PKGM` magic, then entries whose offsets are
/// relative to the table's end.
struct Package<'a> {
    bytes: &'a [u8],
    data_start: usize,
    members: Vec<(String, usize, usize)>,
}

impl<'a> Package<'a> {
    fn member(&self, name: &str) -> Option<&'a [u8]> {
        let (_, offset, length) = self.members.iter().find(|(member, _, _)| member == name)?;
        let start = self.data_start.checked_add(*offset)?;
        self.bytes.get(start..start + length)
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

fn read_package(bytes: &[u8]) -> Option<Package<'_>> {
    let magic_length = read_u32(bytes, 0)? as usize;
    if magic_length == 0 {
        return None;
    }
    let magic = bytes.get(4..4 + magic_length)?;
    if magic.len() != 8
        || !magic.starts_with(b"PKG")
        || !matches!(magic[3], b'V' | b'M')
        || !magic[4..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let count = read_u32(bytes, 4 + magic_length)? as usize;
    let mut cursor = 8 + magic_length;
    let mut members = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let name_length = read_u32(bytes, cursor)? as usize;
        if name_length == 0 {
            return None;
        }
        cursor += 4;
        let name = std::str::from_utf8(bytes.get(cursor..cursor + name_length)?).ok()?;
        cursor += name_length;
        let offset = read_u32(bytes, cursor)? as usize;
        let length = read_u32(bytes, cursor + 4)? as usize;
        cursor += 8;
        if members.iter().any(|(held, _, _)| held == name) {
            return None;
        }
        members.push((name.to_owned(), offset, length));
    }
    let data_start = cursor;
    for (_, offset, length) in &members {
        if data_start + offset + length > bytes.len() {
            return None;
        }
    }
    Some(Package {
        bytes,
        data_start,
        members,
    })
}

/// A mobile export's own two members: the video, and whatever credit its
/// descriptor offers. A still-at-rest scene exports as a video too, so this is
/// the only payload the import reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobilePackage {
    pub video: Vec<u8>,
    pub title: Option<String>,
    pub workshop_id: Option<String>,
}

/// The video and credit an `.mpkg` carries, or `None` when the container is
/// malformed or holds no video. A descriptor this code cannot parse costs the
/// metadata, never the video.
pub fn mobile_package(bytes: &[u8]) -> Option<MobilePackage> {
    let package = read_package(bytes)?;
    let video = package.member("wallpaper.mp4")?;
    if video.is_empty() {
        return None;
    }
    let (title, workshop_id) = package
        .member("project.json")
        .and_then(project_metadata)
        .unwrap_or((None, None));
    Some(MobilePackage {
        video: video.to_vec(),
        title,
        workshop_id,
    })
}

/// The two credit fields a mobile export's descriptor may carry. The descriptor
/// is third-party content: a number that is not a whole id becomes no id.
fn project_metadata(text: &[u8]) -> Option<(Option<String>, Option<String>)> {
    let parsed: Value = serde_json::from_slice(text).ok()?;
    let project = parsed.as_object()?;
    let title = project
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let workshop_id = project
        .get("package")
        .and_then(|package| package.get("workshopId"))
        .and_then(|id| match id {
            Value::String(id) if !id.is_empty() => Some(id.clone()),
            Value::Number(id) => id.as_u64().map(|n| n.to_string()),
            _ => None,
        });
    Some((title, workshop_id))
}

/// True when `key` appears at any depth in the parsed tree.
fn has_key_anywhere(value: &Value, key: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| has_key_anywhere(item, key)),
        Value::Object(map) => map
            .iter()
            .any(|(name, child)| name == key || has_key_anywhere(child, key)),
        _ => false,
    }
}

/// Reads a scene package's facts. A package that cannot be read, a scene with
/// no object list, or a model member that cannot be parsed all mean the scene
/// is unclassified rather than assumed still.
pub fn scene_facts(bytes: &[u8]) -> Option<SceneFacts> {
    let package = read_package(bytes)?;
    let scene: Value = serde_json::from_slice(package.member("scene.json")?).ok()?;
    if !scene.is_object() {
        return None;
    }
    let objects = scene.get("objects")?.as_array()?;
    let mut moves = has_key_anywhere(&scene, "animation");
    let mut has_text = false;
    for object in objects {
        let Some(object) = object.as_object() else {
            continue;
        };
        if object.contains_key("particle") || object.contains_key("animationlayers") {
            moves = true;
        }
        if object.contains_key("text") {
            has_text = true;
        }
        let Some(model) = object.get("model").and_then(Value::as_str) else {
            continue;
        };
        let parsed: Value = serde_json::from_slice(package.member(model)?).ok()?;
        if has_key_anywhere(&parsed, "animation") {
            moves = true;
        }
    }
    Some(SceneFacts {
        at_rest: !moves,
        has_text,
    })
}

/// What container a video's own header proves.
pub fn video_container(path: &Path) -> &'static str {
    let Some(header) = read_head(path, 1024) else {
        return "unknown";
    };
    if header.len() >= 8 && &header[4..8] == b"ftyp" {
        return "mp4";
    }
    if !header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return "unknown";
    }
    let text = String::from_utf8_lossy(&header).to_ascii_lowercase();
    if text.contains("webm") {
        "webm"
    } else {
        "matroska"
    }
}

fn read_head(path: &Path, length: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = vec![0; length];
    let read = file.read(&mut buffer).ok()?;
    buffer.truncate(read);
    Some(buffer)
}

/// A clip's length from its `moov` box, without reading the clip. A file
/// without one reports nothing rather than a guessed second.
pub fn video_duration_ms(path: &Path) -> Option<u64> {
    use std::io::{Read, Seek};
    const MAX_MOOV: u64 = 64 * 1024 * 1024;
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut offset = 0u64;
    while offset + 8 <= size {
        file.seek(std::io::SeekFrom::Start(offset)).ok()?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header[..8]).ok()?;
        let declared = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let kind: [u8; 4] = header[4..8].try_into().ok()?;
        let (length, payload) = match declared {
            1 => {
                file.read_exact(&mut header[8..]).ok()?;
                (
                    u64::from_be_bytes(header[8..].try_into().ok()?),
                    offset + 16,
                )
            }
            0 => (size - offset, offset + 8),
            length => (length, offset + 8),
        };
        if length < 8 || offset + length > size {
            return None;
        }
        if &kind == b"moov" {
            if length > MAX_MOOV {
                return None;
            }
            file.seek(std::io::SeekFrom::Start(payload)).ok()?;
            let mut box_bytes = vec![0u8; (length - (payload - offset)) as usize];
            file.read_exact(&mut box_bytes).ok()?;
            return mvhd_duration(&box_bytes);
        }
        offset += length;
    }
    None
}

/// The `mvhd` header inside a `moov` payload, its timescale and duration.
fn mvhd_duration(moov: &[u8]) -> Option<u64> {
    let mut cursor = 0usize;
    while cursor + 8 <= moov.len() {
        let declared = u32::from_be_bytes(moov.get(cursor..cursor + 4)?.try_into().ok()?) as u64;
        let kind = moov.get(cursor + 4..cursor + 8)?;
        let length = if declared == 1 {
            16 + u64::from_be_bytes(moov.get(cursor + 8..cursor + 16)?.try_into().ok()?)
        } else if declared == 0 {
            moov.len() as u64 - cursor as u64
        } else {
            declared
        };
        if kind == b"mvhd" {
            let body = moov.get(cursor + 8..cursor + length as usize)?;
            let (timescale, duration) = match body.first()? {
                0 => {
                    let timescale = u32::from_be_bytes(body.get(12..16)?.try_into().ok()?);
                    let duration = u32::from_be_bytes(body.get(16..20)?.try_into().ok()?);
                    (u64::from(timescale), u64::from(duration))
                }
                1 => {
                    let timescale = u32::from_be_bytes(body.get(20..24)?.try_into().ok()?);
                    let duration = u64::from_be_bytes(body.get(24..32)?.try_into().ok()?);
                    (u64::from(timescale), duration)
                }
                _ => return None,
            };
            if timescale == 0 {
                return None;
            }
            return Some(duration.saturating_mul(1000) / timescale);
        }
        cursor += length as usize;
    }
    None
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
        Kind::Video => match item.container {
            Some("mp4") => "video/mp4",
            Some("webm") => "video/webm",
            Some("matroska") => "video/x-matroska",
            _ => video_content_type(extension),
        },
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

/// Marks a project as this host's own conversion, so the engine's list tells
/// it apart from the item it came from.
const TITLE_PREFIX: &str = "[yunari] ";

/// Writes the item `workshop_id` names as an editable project under
/// `projects/myprojects/<id>`, and nothing else: the selection is the whole
/// request.
pub fn project(workshop_id: &str) -> Value {
    let Some(install) = locate() else {
        return json!({"ok": false, "reason": "no-install"});
    };
    unpack(&install, workshop_id)
}

/// The unpack itself, against an install. Never removes a directory this run
/// did not create: an existing project is reused, and a failed write removes
/// only what it made.
pub fn unpack(install: &Install, workshop_id: &str) -> Value {
    // A workshop id names one directory and nothing else.
    if workshop_id.is_empty() || !workshop_id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return json!({"ok": false, "reason": "refused"});
    }
    let Some(item) = item_of(install, workshop_id) else {
        return json!({"ok": false, "reason": "unreadable"});
    };
    let directory = install.my_projects.join(workshop_id);
    match std::fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // A project unpacked before the preview was copied still shows no
            // thumbnail; filling that in is the one write reuse performs.
            let _ = fill_preview(&item, &directory);
            return json!({"ok": true, "name": workshop_id, "reused": true});
        }
        Err(_) => return json!({"ok": false, "reason": "refused"}),
    }
    match write_project(&item, &directory) {
        Ok(()) => json!({"ok": true, "name": workshop_id, "reused": false}),
        Err(_) => {
            let _ = std::fs::remove_dir_all(&directory);
            json!({"ok": false, "reason": "unreadable"})
        }
    }
}

/// The listed item a workshop id names, read through the same descriptor rules
/// the library listing uses.
fn item_of(install: &Install, workshop_id: &str) -> Option<Item> {
    let cache = workshop_cache(install);
    install.libraries.iter().find_map(|library| {
        let directory = library.join(workshop_id);
        directory
            .is_dir()
            .then(|| read_item(workshop_id, &directory, &cache))
            .flatten()
    })
}

/// Where a member may be written. A member name is third-party data: real
/// packages nest under `/` and never use `\`, so nesting is allowed and
/// anything that could leave the project — an absolute path, a drive letter, a
/// backslash, an empty, `.` or `..` segment — is refused rather than cleaned up.
fn member_path(directory: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(char::from(92)) {
        return None;
    }
    if name
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    let target = name
        .split('/')
        .fold(directory.to_path_buf(), |path, segment| path.join(segment));
    inside(directory, &target).then_some(target)
}

/// Every member, then the rewritten descriptor and the preview.
fn write_project(item: &Item, directory: &Path) -> Result<(), ()> {
    let bytes = std::fs::read(&item.payload).map_err(|_| ())?;
    let package = read_package(&bytes).ok_or(())?;
    if package.member("scene.json").is_none() {
        return Err(());
    }
    let text = std::fs::read_to_string(item.directory.join("project.json")).map_err(|_| ())?;
    let mut descriptor: Value = serde_json::from_str(&text).map_err(|_| ())?;
    if !descriptor.is_object() {
        return Err(());
    }
    for (name, _, _) in &package.members {
        let target = member_path(directory, name).ok_or(())?;
        let member = package.member(name).ok_or(())?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|_| ())?;
        }
        std::fs::write(&target, member).map_err(|_| ())?;
    }
    // The one packed-only field: a subscribed project says `scene.pkg`, while a
    // source project — the shape the editor opens — says `scene.json`. A title
    // already carrying the prefix is left alone, so a re-unpack cannot double it.
    if let Some(object) = descriptor.as_object_mut() {
        let title = object
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .map(|title| {
                if title.starts_with(TITLE_PREFIX) {
                    title.to_owned()
                } else {
                    format!("{TITLE_PREFIX}{title}")
                }
            });
        if let Some(title) = title {
            object.insert("title".to_owned(), Value::String(title));
        }
        object.insert("file".to_owned(), Value::String("scene.json".to_owned()));
    }
    std::fs::write(
        directory.join("project.json"),
        serde_json::to_vec_pretty(&descriptor).map_err(|_| ())?,
    )
    .map_err(|_| ())?;
    fill_preview(item, directory)
}

/// The descriptor's own preview, copied beside it: the thumbnail is the
/// workshop's file, not a package member, so without this copy a local project
/// shows none in the engine's own list.
fn fill_preview(item: &Item, directory: &Path) -> Result<(), ()> {
    let text = std::fs::read_to_string(directory.join("project.json")).map_err(|_| ())?;
    let descriptor: Value = serde_json::from_str(&text).map_err(|_| ())?;
    let preview = descriptor
        .get("preview")
        .and_then(Value::as_str)
        .ok_or(())?;
    let source = item.preview.as_ref().ok_or(())?;
    if !inside(&item.directory, source) {
        return Err(());
    }
    let target = member_path(directory, preview).ok_or(())?;
    let bytes = std::fs::read(source).map_err(|_| ())?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|_| ())?;
    }
    std::fs::write(target, bytes).map_err(|_| ())
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

    fn package(members: &[(&str, &[u8])]) -> Vec<u8> {
        let magic = b"PKGV0024";
        let mut table = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        for (name, bytes) in members {
            table.extend((name.len() as u32).to_le_bytes());
            table.extend(name.as_bytes());
            table.extend((data.len() as u32).to_le_bytes());
            table.extend((bytes.len() as u32).to_le_bytes());
            data.extend(*bytes);
        }
        let mut out = (magic.len() as u32).to_le_bytes().to_vec();
        out.extend(magic);
        out.extend((members.len() as u32).to_le_bytes());
        out.extend(table);
        out.extend(data);
        out
    }

    #[test]
    fn a_scene_package_says_whether_it_moves_and_draws_text() {
        let still = package(&[("scene.json", br#"{"objects":[{"text":"hello"}]}"#)]);
        let facts = scene_facts(&still).expect("facts");
        assert!(facts.at_rest);
        assert!(facts.has_text);

        let moving = package(&[(
            "scene.json",
            br#"{"objects":[{"animation":{"options":{"fps":30}}}]}"#,
        )]);
        let facts = scene_facts(&moving).expect("facts");
        assert!(!facts.at_rest);
        assert!(!facts.has_text);

        // A model the scene names but the package does not hold is not a still
        // scene: the facts are absent rather than guessed.
        let missing_model = package(&[(
            "scene.json",
            br#"{"objects":[{"model":"models/thing.json"}]}"#,
        )]);
        assert!(scene_facts(&missing_model).is_none());
        assert!(scene_facts(b"not a package").is_none());
    }

    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(payload);
        out
    }

    #[test]
    fn a_videos_own_bytes_choose_the_container_and_the_length() {
        let directory = temp_dir();
        let mut mvhd = vec![0u8; 4];
        mvhd.extend(0u32.to_be_bytes());
        mvhd.extend(0u32.to_be_bytes());
        mvhd.extend(1000u32.to_be_bytes());
        mvhd.extend(5000u32.to_be_bytes());
        mvhd.extend([0u8; 80]);
        let mut mp4 = boxed(b"ftyp", b"isom\0\0\0\0isom");
        mp4.extend(boxed(b"moov", &boxed(b"mvhd", &mvhd)));
        let clip = directory.join("clip.mp4");
        std::fs::write(&clip, &mp4).unwrap();
        assert_eq!(video_container(&clip), "mp4");
        assert_eq!(video_duration_ms(&clip), Some(5000));

        // A descriptor's `.mp4` name over Matroska bytes is still Matroska.
        let mut matroska = vec![0x1a, 0x45, 0xdf, 0xa3];
        matroska.extend(b"matroska\0webm");
        let other = directory.join("clip2.mp4");
        std::fs::write(&other, &matroska).unwrap();
        assert_eq!(video_container(&other), "webm");
        assert!(video_duration_ms(&other).is_none());
        std::fs::remove_dir_all(&directory).ok();
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
    fn a_scene_package_unpacks_into_an_editable_project() {
        let root = temp_dir();
        let install = fixture(&root);
        std::fs::create_dir_all(&install.my_projects).unwrap();
        let item = install.libraries[0].join("123");
        write_project(
            &item,
            json!({"type": "Scene", "file": "scene.pkg", "title": "City", "preview": "preview.jpg"}),
        );
        let bytes = package(&[
            ("scene.json", br#"{"objects":[]}"#),
            ("materials/sky.tex", b"tex"),
        ]);
        std::fs::write(item.join("scene.pkg"), &bytes).unwrap();
        std::fs::write(item.join("preview.jpg"), b"jpeg").unwrap();

        let answer = unpack(&install, "123");
        assert_eq!(answer["ok"], true);
        let project = install.my_projects.join("123");
        assert_eq!(
            std::fs::read(project.join("scene.json")).unwrap(),
            br#"{"objects":[]}"#
        );
        assert_eq!(
            std::fs::read(project.join("materials/sky.tex")).unwrap(),
            b"tex"
        );
        assert_eq!(std::fs::read(project.join("preview.jpg")).unwrap(), b"jpeg");
        let descriptor: Value =
            serde_json::from_slice(&std::fs::read(project.join("project.json")).unwrap()).unwrap();
        assert_eq!(descriptor["file"], "scene.json");
        assert_eq!(descriptor["title"], "[yunari] City");

        // A second request reuses the project and cannot double the prefix.
        let again = unpack(&install, "123");
        assert_eq!(again["reused"], true);
        let descriptor: Value =
            serde_json::from_slice(&std::fs::read(project.join("project.json")).unwrap()).unwrap();
        assert_eq!(descriptor["title"], "[yunari] City");
    }

    #[test]
    fn a_listing_is_cached_until_something_it_read_changes() {
        let root = temp_dir();
        let install = fixture(&root);
        let settings = root.join("settings");
        std::fs::create_dir_all(&settings).unwrap();
        let item = install.libraries[0].join("200");
        write_project(
            &item,
            json!({"type": "Scene", "file": "scene.pkg", "title": "Cached"}),
        );
        std::fs::write(item.join("scene.pkg"), package(&[("scene.json", b"{}")])).unwrap();

        let first = cached_listing(&install, &settings);
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        // The file it left holds exactly the answer it returned.
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(settings.join(CACHE_FILE)).unwrap())
                .unwrap();
        assert_eq!(written["listing"], first);
        // A second pass answers the same and rewrites the same file.
        let again = cached_listing(&install, &settings);
        assert_eq!(again, first);

        // Replacing an item's package changes the fingerprint, so the next
        // pass reads the disk again.
        let before = fingerprint(&install);
        std::fs::write(
            item.join("scene.pkg"),
            package(&[("scene.json", br#"{"objects":[]}"#)]),
        )
        .unwrap();
        assert_ne!(fingerprint(&install), before);
        let third = cached_listing(&install, &settings);
        assert_eq!(third["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_member_that_could_leave_the_project_refuses_the_whole_item() {
        let root = temp_dir();
        let install = fixture(&root);
        std::fs::create_dir_all(&install.my_projects).unwrap();
        let item = install.libraries[0].join("9");
        write_project(
            &item,
            json!({"type": "Scene", "file": "scene.pkg", "title": "Bad"}),
        );
        let bytes = package(&[("scene.json", b"{}"), ("../escape.tex", b"x")]);
        std::fs::write(item.join("scene.pkg"), &bytes).unwrap();

        let answer = unpack(&install, "9");
        assert_eq!(answer["ok"], false);
        assert_eq!(answer["reason"], "unreadable");
        assert!(!install.my_projects.join("9").exists());
        assert!(!root.join("escape.tex").exists());

        // An id that is not one directory name is refused before a path exists.
        assert_eq!(unpack(&install, "..")["reason"], "refused");
        assert_eq!(unpack(&install, "a/b")["reason"], "refused");

        // A package with no scene member is not a project.
        let item = install.libraries[0].join("10");
        write_project(
            &item,
            json!({"type": "Scene", "file": "scene.pkg", "title": "NoScene"}),
        );
        std::fs::write(item.join("scene.pkg"), package(&[("other.tex", b"x")])).unwrap();
        assert_eq!(unpack(&install, "10")["reason"], "unreadable");
        assert!(!install.my_projects.join("10").exists());
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
