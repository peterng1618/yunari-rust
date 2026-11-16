//! The portable package: one zip an author exports and someone else imports.
//!
//! ```text
//! manifest.json   {"format":"yunari-theme-package","kind":"theme","version":1,"theme":"theme.json"}
//! theme.json
//! thumbnail.png   optional, named by the manifest
//! credits.json    optional, named by the manifest
//! assets/<declared path>
//! ```
//!
//! Reading bounds the archive before inflating anything: entry count, entry
//! size, expanded size and the archive itself. An undeclared member is refused,
//! as is a declared asset with no file.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use serde::{Deserialize, Serialize};
use yunari_model::document::ThemeDocument;
use yunari_model::validate::validate_document;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::store::{Library, LibraryError, SaveRequest, Saved, sha256_hex};

/// One entry's cap, and the archive's own.
pub const MAX_ARCHIVE_BYTES: usize = 95 * 1024 * 1024;
pub const MAX_ENTRY_BYTES: usize = 80 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: usize = 180 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 128;
pub const MAX_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;

pub const MANIFEST: &str = "manifest.json";
pub const THEME: &str = "theme.json";
pub const THUMBNAIL: &str = "thumbnail.png";
pub const CREDITS: &str = "credits.json";

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    #[serde(default)]
    kind: Option<String>,
    version: u32,
    theme: String,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    credits: Option<String>,
}

/// A package as read.
#[derive(Debug, Clone)]
pub struct Package {
    pub document: ThemeDocument,
    pub assets: BTreeMap<String, Vec<u8>>,
    pub thumbnail: Option<Vec<u8>>,
}

/// Writes a theme and its assets as a package.
pub fn write_package(
    document: &ThemeDocument,
    assets: &BTreeMap<String, Vec<u8>>,
    thumbnail: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if let Err(issues) = validate_document(document) {
        return Err(issues.first().map_or_else(
            || "the theme is not valid".to_owned(),
            |issue| issue.to_string(),
        ));
    }
    let declared: BTreeSet<&str> = document
        .assets
        .iter()
        .map(|asset| asset.path.as_str())
        .collect();
    if assets.len() != declared.len() || assets.keys().any(|path| !declared.contains(path.as_str()))
    {
        return Err("the package's assets do not match the theme's declarations.".to_owned());
    }
    if thumbnail.is_some_and(|bytes| !is_png(bytes) || bytes.len() > MAX_THUMBNAIL_BYTES) {
        return Err("the thumbnail is not a PNG within the size cap.".to_owned());
    }
    for (path, bytes) in assets {
        if !allowed_asset_path(path) || bytes.len() > MAX_ENTRY_BYTES {
            return Err(format!("{path} cannot travel in a package."));
        }
    }
    let manifest = Manifest {
        format: "yunari-theme-package".to_owned(),
        kind: Some("theme".to_owned()),
        version: 1,
        theme: THEME.to_owned(),
        thumbnail: thumbnail.map(|_| THUMBNAIL.to_owned()),
        credits: None,
    };
    let manifest = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    let theme = serde_json::to_vec(document).map_err(|error| error.to_string())?;

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
        writer
            .start_file(name, options)
            .map_err(|error| error.to_string())?;
        writer.write_all(bytes).map_err(|error| error.to_string())
    };
    put(MANIFEST, &manifest)?;
    put(THEME, &theme)?;
    if let Some(thumbnail) = thumbnail {
        put(THUMBNAIL, thumbnail)?;
    }
    for (path, bytes) in assets {
        put(path, bytes)?;
    }
    let cursor = writer.finish().map_err(|error| error.to_string())?;
    let bytes = cursor.into_inner();
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("the package is larger than a theme package may be.".to_owned());
    }
    Ok(bytes)
}

/// Reads a package, bounding it before anything is inflated.
pub fn read_package(bytes: &[u8]) -> Result<Package, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err("the package is larger than a theme package may be.".to_owned());
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "the file is not a zip archive.".to_owned())?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!("a package carries at most {MAX_ENTRIES} files."));
    }
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut expanded = 0_usize;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| "a package member could not be read.".to_owned())?;
        let Some(name) = entry.enclosed_name() else {
            return Err("a package member names a path outside the package.".to_owned());
        };
        let name = name.to_string_lossy().replace('\\', "/");
        if !allowed_member(&name) {
            return Err(format!("{name} is not part of a theme package."));
        }
        if entry.size() as usize > MAX_ENTRY_BYTES {
            return Err(format!("{name} is larger than a package file may be."));
        }
        let mut member = Vec::with_capacity(entry.size() as usize);
        entry
            .by_ref()
            .take((MAX_ENTRY_BYTES + 1) as u64)
            .read_to_end(&mut member)
            .map_err(|_| format!("{name} could not be read."))?;
        if member.len() > MAX_ENTRY_BYTES {
            return Err(format!("{name} is larger than a package file may be."));
        }
        expanded += member.len();
        if expanded > MAX_EXPANDED_BYTES {
            return Err("the package expands beyond its cap.".to_owned());
        }
        files.insert(name, member);
    }

    let manifest_bytes = files
        .get(MANIFEST)
        .ok_or_else(|| "a package needs manifest.json and theme.json.".to_owned())?;
    let theme_bytes = files
        .get(THEME)
        .ok_or_else(|| "a package needs manifest.json and theme.json.".to_owned())?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|_| "Unsupported package manifest.".to_owned())?;
    if manifest.format != "yunari-theme-package"
        || manifest.version != 1
        || manifest.theme != THEME
        || manifest.kind.as_deref().is_some_and(|kind| kind != "theme")
        || manifest
            .thumbnail
            .as_deref()
            .is_some_and(|name| name != THUMBNAIL)
    {
        return Err("Unsupported package manifest.".to_owned());
    }
    if manifest
        .credits
        .as_deref()
        .is_some_and(|name| name != CREDITS)
    {
        return Err("Unsupported package manifest.".to_owned());
    }
    let document: ThemeDocument = serde_json::from_slice(theme_bytes)
        .map_err(|error| format!("theme.json is not a theme: {error}"))?;
    if let Err(issues) = validate_document(&document) {
        return Err(issues
            .first()
            .map_or_else(|| "Invalid theme.".to_owned(), |issue| issue.to_string()));
    }
    let declared: BTreeSet<&str> = document
        .assets
        .iter()
        .map(|asset| asset.path.as_str())
        .collect();
    if let Some(undeclared) = files.keys().find(|name| {
        !matches!(name.as_str(), MANIFEST | THEME | THUMBNAIL | CREDITS)
            && !declared.contains(name.as_str())
    }) {
        return Err(format!("{undeclared} is not declared by the theme."));
    }
    let mut assets = BTreeMap::new();
    for path in &declared {
        let bytes = files
            .get(*path)
            .ok_or_else(|| format!("the theme declares {path}, which the package lacks."))?;
        assets.insert((*path).to_owned(), bytes.clone());
    }
    let thumbnail = files.get(THUMBNAIL).cloned();
    if thumbnail.as_ref().is_some_and(|bytes| !is_png(bytes)) {
        return Err("the thumbnail is not a PNG.".to_owned());
    }
    Ok(Package {
        document,
        assets,
        thumbnail,
    })
}

/// Stores a package under the first free id, never overwriting a theme.
pub fn import_into(library: &Library, package: &Package) -> Result<Saved, LibraryError> {
    let wanted = &package.document.id;
    for candidate in id_candidates(wanted) {
        let mut document = package.document.clone();
        document.id.clone_from(&candidate);
        let saved = library.save(&SaveRequest {
            id: &candidate,
            document: &document,
            assets: &package.assets,
            base: None,
            overwrite: false,
        });
        match saved {
            Ok(saved) => return Ok(saved),
            Err(LibraryError::Conflict { .. }) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(LibraryError::Conflict { id: wanted.clone() })
}

/// `alpha`, `alpha-2` … `alpha-100`, then a hash suffix.
fn id_candidates(wanted: &str) -> Vec<String> {
    let mut candidates = vec![wanted.to_owned()];
    for suffix in 2..=100 {
        candidates.push(format!("{wanted}-{suffix}"));
    }
    let digest = sha256_hex(wanted.as_bytes());
    candidates.push(format!("{wanted}-{}", &digest[..6]));
    candidates
}

fn allowed_member(name: &str) -> bool {
    matches!(name, MANIFEST | THEME | THUMBNAIL | CREDITS) || allowed_asset_path(name)
}

fn allowed_asset_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("assets/") else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 200
        && !path.contains("..")
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use yunari_model::document::{
        Artboard, ArtboardShape, AssetKind, AssetReference, Background, ContentFit, Metadata,
    };

    use super::*;

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
            background: Background::default(),
            palette: BTreeMap::new(),
            type_tokens: BTreeMap::new(),
            assets: Vec::new(),
            nodes: Vec::new(),
            bindings: BTreeMap::new(),
            editor_metadata: BTreeMap::new(),
        }
    }

    fn with_asset(mut document: ThemeDocument) -> ThemeDocument {
        document.assets.push(AssetReference {
            id: "a".to_owned(),
            kind: AssetKind::Image,
            path: "assets/a.png".to_owned(),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        });
        document
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .expect("a zip entry");
            writer.write_all(bytes).expect("written bytes");
        }
        writer.finish().expect("a finished zip").into_inner()
    }

    fn manifest() -> Vec<u8> {
        serde_json::to_vec(&Manifest {
            format: "yunari-theme-package".to_owned(),
            kind: Some("theme".to_owned()),
            version: 1,
            theme: THEME.to_owned(),
            thumbnail: None,
            credits: None,
        })
        .expect("a manifest")
    }

    #[test]
    fn a_written_package_reads_back_with_its_assets() {
        let document = with_asset(document("alpha"));
        let assets = BTreeMap::from([("assets/a.png".to_owned(), vec![1, 2, 3])]);
        let bytes = write_package(&document, &assets, None).expect("a package");
        let package = read_package(&bytes).expect("a readable package");
        assert_eq!(package.document, document);
        assert_eq!(package.assets, assets);
    }

    #[test]
    fn a_package_without_a_manifest_is_refused() {
        let bytes = zip_with(&[(THEME, b"{}")]);
        assert!(read_package(&bytes).is_err());
    }

    #[test]
    fn a_package_naming_an_undeclared_asset_is_refused() {
        let theme = serde_json::to_vec(&document("alpha")).expect("a theme");
        let bytes = zip_with(&[
            (MANIFEST, &manifest()),
            (THEME, &theme),
            ("assets/b.png", &[1]),
        ]);
        assert!(read_package(&bytes).is_err());
    }

    #[test]
    fn a_package_over_the_entry_cap_is_refused() {
        let mut entries: Vec<(String, Vec<u8>)> = vec![
            (MANIFEST.to_owned(), manifest()),
            (
                THEME.to_owned(),
                serde_json::to_vec(&document("alpha")).expect("a theme"),
            ),
        ];
        for index in 0..MAX_ENTRIES {
            entries.push((format!("assets/{index}.png"), vec![0]));
        }
        let borrowed: Vec<(&str, &[u8])> = entries
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let bytes = zip_with(&borrowed);
        assert!(read_package(&bytes).is_err());
    }

    #[test]
    fn import_never_overwrites() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "yunari-package-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let library = Library::new(&root);
        let first = library.save(&SaveRequest {
            id: "alpha",
            document: &document("alpha"),
            assets: &BTreeMap::new(),
            base: None,
            overwrite: false,
        });
        assert!(first.is_ok());
        let package = Package {
            document: document("alpha"),
            assets: BTreeMap::new(),
            thumbnail: None,
        };
        let saved = import_into(&library, &package).expect("an imported copy");
        assert_eq!(saved.entry.id, "alpha-2");
        let _ = fs::remove_dir_all(&root);
    }
}
