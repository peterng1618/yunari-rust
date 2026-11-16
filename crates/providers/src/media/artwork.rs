//! Media artwork as files on disk, content-addressed by the SHA-256 of the
//! bytes, so a changed track can never be served under the previous picture.
//!
//! The name is the whole allow-list: 64 lowercase hex digits and an image
//! extension. A name that does not hash to the bytes it names, or whose
//! extension does not describe them, is an absence rather than a wrong picture.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use sha2::{Digest, Sha256};

/// Files the cache keeps before it evicts the oldest.
pub const CAP: usize = 64;
/// An interrupted write is only stale once nothing can still be renaming it.
const TEMP_STALE_MS: u128 = 60_000;

/// A name the route will serve and the cache will write.
pub fn valid_name(name: &str) -> bool {
    let Some((sha, ext)) = name.split_once('.') else {
        return false;
    };
    sha.len() == 64
        && sha
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && matches!(ext, "png" | "jpg" | "jpeg" | "webp")
}

/// The format the bytes actually are; the caller's extension is never trusted.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
        return Some("png");
    }
    if bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("jpg");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    None
}

/// MIME for a name this cache wrote; a name outside the set never gets here.
pub fn content_type(name: &str) -> &'static str {
    if name.ends_with(".png") {
        "image/png"
    } else if name.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub struct Artwork {
    directory: PathBuf,
}

impl Artwork {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    /// Stores the bytes under their digest and returns the URL a reading names.
    pub fn put(&self, bytes: &[u8], claimed_ext: &str) -> Result<String, String> {
        let Some(ext) = sniff(bytes).filter(|_| !bytes.is_empty()) else {
            return Err(format!(
                "The artwork bytes are not a supported image (claimed {claimed_ext})."
            ));
        };
        let name = format!("{}.{ext}", sha256_hex(bytes));
        let target = self.directory.join(&name);
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("The artwork folder could not be created: {error}"))?;
        // Content-addressed: an existing file is the same picture already.
        if !target.exists() {
            let scratch = self.directory.join(format!(
                ".{name}.{}.{}.tmp",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|age| age.as_millis())
                    .unwrap_or_default()
            ));
            fs::write(&scratch, bytes)
                .map_err(|error| format!("The artwork could not be written: {error}"))?;
            // Written beside the target and renamed, so a reader never sees
            // half an image under a name that promises the whole digest.
            if let Err(error) = fs::rename(&scratch, &target) {
                let _ = fs::remove_file(&scratch);
                if !target.exists() {
                    return Err(format!("The artwork could not be stored: {error}"));
                }
            }
        }
        prune(&self.directory, CAP);
        Ok(format!("/api/media/artwork/{name}"))
    }

    /// The bytes for a name, or nothing when it is not this entry.
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        if !valid_name(name) {
            return None;
        }
        let bytes = fs::read(self.directory.join(name)).ok()?;
        if !name.starts_with(&sha256_hex(&bytes)) {
            return None;
        }
        let extension = name.rsplit_once('.')?.1;
        let sniffed = sniff(&bytes)?;
        (sniffed == extension || (sniffed == "jpg" && extension == "jpeg")).then_some(bytes)
    }
}

/// Keeps the newest `cap` files and sweeps scratch files left by an interrupted
/// write.
fn prune(directory: &Path, cap: usize) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut kept: Vec<(String, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if name.starts_with('.') && name.ends_with(".tmp") {
            let stale = now
                .duration_since(modified)
                .is_ok_and(|age| age.as_millis() >= TEMP_STALE_MS);
            if stale {
                let _ = fs::remove_file(entry.path());
            }
            continue;
        }
        if valid_name(&name) {
            kept.push((name, modified));
        }
    }
    if kept.len() <= cap {
        return;
    }
    // Ties on mtime fall back to the name, so eviction is deterministic.
    kept.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    for (name, _) in &kept[cap..] {
        let _ = fs::remove_file(directory.join(name));
    }
}

/// The bytes a `artwork` line carries, decoded.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

    fn cache(name: &str) -> (Artwork, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "yunari-artwork-{name}-{}-{}",
            std::process::id(),
            sha256_hex(PNG)
        ));
        let _ = fs::remove_dir_all(&directory);
        (Artwork::new(&directory), directory)
    }

    #[test]
    fn a_stored_picture_is_read_back_under_its_digest() {
        let (cache, directory) = cache("stored");
        let url = cache.put(PNG, "jpg").expect("a picture");
        let name = url.rsplit('/').next().expect("a name");
        assert_eq!(name, format!("{}.png", sha256_hex(PNG)));
        assert_eq!(content_type(name), "image/png");
        assert_eq!(cache.read(name).as_deref(), Some(PNG));
        assert!(!directory.join(".tmp").exists());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn bytes_that_are_not_an_image_are_refused() {
        let (cache, directory) = cache("refused");
        assert_eq!(
            cache.put(b"not a picture", "png").unwrap_err(),
            "The artwork bytes are not a supported image (claimed png)."
        );
        assert!(cache.put(b"", "png").is_err());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_name_that_does_not_hash_or_describe_its_bytes_is_not_served() {
        let (cache, directory) = cache("mismatch");
        let url = cache.put(PNG, "png").expect("a picture");
        let name = url.rsplit('/').next().expect("a name").to_owned();
        // The right bytes under someone else's name are not this entry.
        fs::write(directory.join(format!("{}.png", sha256_hex(b"other"))), PNG).unwrap();
        assert!(
            cache
                .read(&format!("{}.png", sha256_hex(b"other")))
                .is_none()
        );
        // The right bytes with an extension they are not are not either.
        let jpeg = format!("{}.jpg", sha256_hex(PNG));
        fs::write(directory.join(&jpeg), PNG).unwrap();
        assert!(cache.read(&jpeg).is_none());
        assert!(cache.read(&name).is_some());
        for evil in [
            "../secrets.png",
            "short.png",
            &name.replace(".png", ".gif"),
            &name.to_uppercase(),
        ] {
            assert!(!valid_name(evil), "{evil}");
            assert!(cache.read(evil).is_none(), "{evil}");
        }
        let _ = fs::remove_dir_all(&directory);
    }
}
