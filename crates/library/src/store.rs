//! The theme directory: read, write, list, remove.
//!
//! ```text
//! <library>/<theme-id>/theme.json
//! <library>/<theme-id>/assets/<declared path>
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use yunari_model::document::ThemeDocument;
use yunari_model::validate::{ValidationIssue, is_stable_id, validate_document};

/// One theme file, in bytes.
pub const MAX_THEME_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// One asset, in bytes.
pub const MAX_ASSET_BYTES: u64 = 80 * 1024 * 1024;
pub const MAX_ASSET_COUNT: usize = 128;
/// A theme's whole directory, assets included.
pub const MAX_THEME_BYTES: u64 = 90 * 1024 * 1024;

/// What a save was written against, and why it was refused.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    /// The theme exists and the save named no base, or the base is stale.
    #[error("theme {id} already exists; save from the stored copy or overwrite it")]
    Conflict { id: String },
    #[error("the document is not valid")]
    Invalid(Vec<ValidationIssue>),
    #[error("{0}")]
    TooLarge(String),
    #[error("{0}")]
    Unreadable(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// One theme as the library lists it, read from its `theme.json` alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub updated_at: SystemTime,
    pub node_count: usize,
}

/// A theme as it is stored.
#[derive(Debug, Clone)]
pub struct StoredTheme {
    pub entry: ThemeEntry,
    pub document: ThemeDocument,
    pub base: String,
}

/// What a caller asks to store.
pub struct SaveRequest<'a> {
    pub id: &'a str,
    pub document: &'a ThemeDocument,
    pub assets: &'a BTreeMap<String, Vec<u8>>,
    /// The base of the copy this save is written against. `None` means the
    /// caller believes no theme exists under this id.
    pub base: Option<&'a str>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub entry: ThemeEntry,
    /// The identity of the bytes just written; a later save sends it back.
    pub base: String,
}

/// The themes on this machine.
#[derive(Debug, Clone)]
pub struct Library {
    root: PathBuf,
}

/// `%USERPROFILE%\.yunari\themes`, or `$HOME`'s equivalent.
pub fn default_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".yunari").join("themes")
}

impl Library {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every stored theme, by name. Unreadable and invalid themes are skipped:
    /// one broken directory must not empty the library.
    pub fn list(&self) -> Vec<ThemeEntry> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        self.repair();
        let mut themes: Vec<ThemeEntry> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| {
                let id = entry.file_name().to_str()?.to_owned();
                is_stable_id(&id).then_some(id)
            })
            .filter_map(|id| self.read(&id).ok().map(|stored| stored.entry))
            .collect();
        themes.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        themes
    }

    pub fn read(&self, id: &str) -> Result<StoredTheme, LibraryError> {
        if !is_stable_id(id) {
            return Err(LibraryError::Unreadable(format!("{id} is not a theme id")));
        }
        let directory = self.root.join(id);
        let bytes = fs::read(directory.join("theme.json"))
            .map_err(|error| LibraryError::Unreadable(format!("{id}/theme.json: {error}")))?;
        if bytes.len() as u64 > MAX_THEME_FILE_BYTES {
            return Err(LibraryError::TooLarge(format!(
                "{id}/theme.json is over {MAX_THEME_FILE_BYTES} bytes"
            )));
        }
        let base = sha256_hex(&bytes);
        let document: ThemeDocument = read_document(&bytes)
            .map_err(|error| LibraryError::Unreadable(format!("{id}/theme.json: {error}")))?;
        let entry = entry_of(id, &document, file_time(&directory.join("theme.json")));
        Ok(StoredTheme {
            entry,
            document,
            base,
        })
    }

    /// The bytes of every asset the stored document declares, by path.
    ///
    /// A save writes a fresh directory, so an editor that opened a theme must
    /// hand these back with the document or the save drops files it never read.
    /// A declared file that is missing is left out rather than invented.
    pub fn assets(&self, id: &str, document: &ThemeDocument) -> BTreeMap<String, Vec<u8>> {
        let mut assets = BTreeMap::new();
        if !is_stable_id(id) {
            return assets;
        }
        for reference in &document.assets {
            if !asset_path_is_safe(&reference.path) {
                continue;
            }
            let Ok(bytes) = fs::read(self.root.join(id).join(&reference.path)) else {
                continue;
            };
            if bytes.len() as u64 <= MAX_ASSET_BYTES {
                assets.insert(reference.path.clone(), bytes);
            }
        }
        assets
    }

    /// Writes a theme under its id, swapping it into place.
    ///
    /// A save naming no base onto an id that exists is a conflict, and so is one
    /// naming a base the stored file no longer has: the caller is working from a
    /// copy that is no longer current.
    pub fn save(&self, request: &SaveRequest<'_>) -> Result<Saved, LibraryError> {
        if !is_stable_id(request.id) {
            return Err(LibraryError::Unreadable(format!(
                "{} is not a theme id",
                request.id
            )));
        }
        if request.document.id != request.id {
            return Err(LibraryError::Unreadable(format!(
                "the document says its id is {}, but it is being saved as {}",
                request.document.id, request.id
            )));
        }
        if let Err(issues) = validate_document(request.document) {
            return Err(LibraryError::Invalid(issues));
        }
        let bytes = serde_json::to_string_pretty(request.document)
            .map_err(|error| LibraryError::Unreadable(error.to_string()))?
            .into_bytes();
        if bytes.len() as u64 > MAX_THEME_FILE_BYTES {
            return Err(LibraryError::TooLarge(format!(
                "theme.json is over {MAX_THEME_FILE_BYTES} bytes"
            )));
        }
        if request.assets.len() > MAX_ASSET_COUNT {
            return Err(LibraryError::TooLarge(format!(
                "a theme carries at most {MAX_ASSET_COUNT} assets"
            )));
        }
        let total: u64 = bytes.len() as u64
            + request
                .assets
                .values()
                .map(|asset| asset.len() as u64)
                .sum::<u64>();
        if total > MAX_THEME_BYTES {
            return Err(LibraryError::TooLarge(format!(
                "a theme carries at most {MAX_THEME_BYTES} bytes"
            )));
        }
        for (path, asset) in request.assets {
            if !asset_path_is_safe(path) {
                return Err(LibraryError::Unreadable(format!(
                    "{path} is not an asset path"
                )));
            }
            if asset.len() as u64 > MAX_ASSET_BYTES {
                return Err(LibraryError::TooLarge(format!(
                    "{path} is over {MAX_ASSET_BYTES} bytes"
                )));
            }
        }
        let target = self.root.join(request.id);
        let existing = fs::read(target.join("theme.json")).ok();
        if !request.overwrite {
            match (existing.as_ref(), request.base) {
                (Some(stored), Some(base)) => {
                    if sha256_hex(stored) != base {
                        return Err(LibraryError::Conflict {
                            id: request.id.to_owned(),
                        });
                    }
                }
                (Some(_), None) => {
                    return Err(LibraryError::Conflict {
                        id: request.id.to_owned(),
                    });
                }
                (None, _) => {}
            }
        }

        fs::create_dir_all(&self.root)?;
        let nonce = nonce();
        let staging = self.root.join(format!(".staging-{nonce}.{}", request.id));
        write_theme(&staging, &bytes, request.assets)?;
        let retired = self.root.join(format!(".retired-{nonce}.{}", request.id));
        if target.exists() {
            rename(&target, &retired)?;
        }
        if let Err(error) = rename(&staging, &target) {
            // Put the old copy back rather than leaving a theme missing.
            if retired.exists() {
                let _ = rename(&retired, &target);
            }
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        let _ = fs::remove_dir_all(&retired);
        let base = sha256_hex(&bytes);
        Ok(Saved {
            entry: entry_of(
                request.id,
                request.document,
                file_time(&target.join("theme.json")),
            ),
            base,
        })
    }

    /// Moves a theme out of the library into `.trash`, so a mistaken delete is
    /// recoverable by hand.
    pub fn remove(&self, id: &str) -> Result<(), LibraryError> {
        if !is_stable_id(id) {
            return Err(LibraryError::Unreadable(format!("{id} is not a theme id")));
        }
        let target = self.root.join(id);
        if !target.exists() {
            return Err(LibraryError::Unreadable(format!("no theme {id}")));
        }
        let trash = self.root.join(".trash");
        fs::create_dir_all(&trash)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();
        rename(&target, &trash.join(format!("{id}-{stamp}")))?;
        Ok(())
    }

    /// Clears scratch directories a crashed save left behind, and restores a
    /// retired copy whose target went missing.
    fn repair(&self) {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with(".staging-") {
                // `.staging-<nonce>.<id>` was never swapped in.
                let _ = fs::remove_dir_all(entry.path());
            } else if let Some(rest) = name.strip_prefix(".retired-") {
                let Some((_, id)) = rest.split_once('.') else {
                    continue;
                };
                let target = self.root.join(id);
                if !target.exists() {
                    let _ = rename(&entry.path(), &target);
                } else {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }
}

fn write_theme(
    directory: &Path,
    document: &[u8],
    assets: &BTreeMap<String, Vec<u8>>,
) -> io::Result<()> {
    fs::create_dir_all(directory.join("assets"))?;
    fs::write(directory.join("theme.json"), document)?;
    for (path, bytes) in assets {
        let file = directory.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, bytes)?;
    }
    Ok(())
}

/// A rename that tolerates a scanner or indexer holding the directory open.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut delay = 10;
    for attempt in 0..6 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 5 && is_busy(&error) => {
                sleep(Duration::from_millis(delay));
                delay *= 2;
            }
            Err(error) => return Err(error),
        }
    }
    fs::rename(from, to)
}

fn is_busy(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::WouldBlock
    )
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default()
}

fn file_time(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(UNIX_EPOCH)
}

fn entry_of(id: &str, document: &ThemeDocument, updated_at: SystemTime) -> ThemeEntry {
    ThemeEntry {
        id: id.to_owned(),
        name: document
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| id.to_owned()),
        author: document.metadata.author.clone(),
        version: document.metadata.version.clone(),
        updated_at,
        node_count: document.nodes.len(),
    }
}

/// An asset path as the document declares it: under `assets/`, no traversal.
fn asset_path_is_safe(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("assets/") else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 200
        && !path.contains("..")
        && !path.contains('\\')
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

/// A stored theme, lifted from the source app's envelope when it is one. A
/// theme written by the TypeScript editor carries the same palette, tokens,
/// assets and bindings but a different envelope and flat scene objects, so it
/// is read through the model's source reader instead of being refused.
fn read_document(bytes: &[u8]) -> Result<ThemeDocument, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if yunari_model::source::looks_like_source(&value) {
        return yunari_model::source::from_source_value(value);
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use yunari_model::document::{Artboard, ArtboardShape, ContentFit, Metadata};

    struct TempLibrary {
        library: Library,
    }

    impl TempLibrary {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "yunari-library-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            );
            let root = std::env::temp_dir().join(unique);
            Self {
                library: Library::new(&root),
            }
        }
    }

    impl Drop for TempLibrary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.library.root());
        }
    }

    fn document(id: &str) -> ThemeDocument {
        ThemeDocument {
            schema_version: "1.0.0".to_owned(),
            id: id.to_owned(),
            metadata: Metadata {
                name: Some(format!("Theme {id}")),
                ..Metadata::default()
            },
            artboard: Artboard {
                width: 450.0,
                height: 675.0,
                content_fit: ContentFit::Contain,
                shape: ArtboardShape::Rect,
            },
            background: yunari_model::document::Background::default(),
            palette: BTreeMap::new(),
            type_tokens: BTreeMap::new(),
            assets: Vec::new(),
            nodes: Vec::new(),
            bindings: BTreeMap::new(),
            editor_metadata: BTreeMap::new(),
        }
    }

    fn save(
        library: &Library,
        document: &ThemeDocument,
        base: Option<&str>,
        overwrite: bool,
    ) -> Result<Saved, LibraryError> {
        let assets = BTreeMap::new();
        library.save(&SaveRequest {
            id: &document.id,
            document,
            assets: &assets,
            base,
            overwrite,
        })
    }

    #[test]
    fn a_saved_theme_lists_and_reads_back() {
        let temp = TempLibrary::new();
        let saved = save(&temp.library, &document("alpha"), None, false).expect("a first save");
        let listed = temp.library.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Theme alpha");
        let stored = temp.library.read("alpha").expect("a readable theme");
        assert_eq!(stored.document, document("alpha"));
        assert_eq!(stored.base, saved.base);
    }

    #[test]
    fn a_theme_stored_by_the_source_editor_reads() {
        let temp = TempLibrary::new();
        let directory = temp.library.root().join("stored");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("theme.json"),
            r##"{"schemaVersion":"3.0.0","fabricVersion":"7.4.0",
                "id":"stored","metadata":{"name":"Stored"},
                "artboard":{"width":480,"height":480},
                "background":{"color":{"ref":"palette.bg"}},
                "globals":{"palette":{"bg":{"name":"Backdrop",
                    "value":{"kind":"solid","color":"#0c0e13"}}}},
                "assets":[],"bindings":{},
                "scene":{"version":"7.4.0","objects":[
                    {"type":"Rect","id":"plate","left":8,"top":8,"width":464,"height":464,
                     "rx":12,"yunariPaint":{"fill":"palette.bg"}}]}}"##,
        )
        .unwrap();
        let listed = temp.library.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Stored");
        let stored = temp.library.read("stored").expect("a readable theme");
        assert_eq!(stored.document.nodes.len(), 1);
        assert_eq!(stored.document.palette["bg"].name, "Backdrop");
        assert_eq!(validate_document(&stored.document), Ok(()));
    }

    #[test]
    fn a_second_first_save_is_a_conflict_until_overwrite() {
        let temp = TempLibrary::new();
        save(&temp.library, &document("alpha"), None, false).expect("a first save");
        assert!(matches!(
            save(&temp.library, &document("alpha"), None, false),
            Err(LibraryError::Conflict { .. })
        ));
        save(&temp.library, &document("alpha"), None, true).expect("an overwrite");
    }

    #[test]
    fn a_stale_base_is_a_conflict() {
        let temp = TempLibrary::new();
        save(&temp.library, &document("alpha"), None, false).expect("a first save");
        assert!(matches!(
            save(
                &temp.library,
                &document("alpha"),
                Some("not the base"),
                false
            ),
            Err(LibraryError::Conflict { .. })
        ));
    }

    #[test]
    fn a_document_validation_refuses_is_never_written() {
        let temp = TempLibrary::new();
        let mut broken = document("alpha");
        broken.artboard.width = 20_000.0;
        assert!(matches!(
            save(&temp.library, &broken, None, false),
            Err(LibraryError::Invalid(_))
        ));
        assert!(temp.library.list().is_empty());
    }

    #[test]
    fn removing_a_theme_takes_it_out_of_the_list_and_keeps_a_recoverable_copy() {
        let temp = TempLibrary::new();
        save(&temp.library, &document("alpha"), None, false).expect("a first save");
        temp.library.remove("alpha").expect("a removal");
        assert!(temp.library.list().is_empty());
        let trash = temp.library.root().join(".trash");
        let entries: Vec<_> = fs::read_dir(trash)
            .expect("a trash directory")
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn an_asset_path_outside_assets_is_refused() {
        let temp = TempLibrary::new();
        let document = document("alpha");
        let mut assets = BTreeMap::new();
        assets.insert("../escape.png".to_owned(), vec![1]);
        let result = temp.library.save(&SaveRequest {
            id: "alpha",
            document: &document,
            assets: &assets,
            base: None,
            overwrite: false,
        });
        assert!(matches!(result, Err(LibraryError::Unreadable(_))));
    }
}
