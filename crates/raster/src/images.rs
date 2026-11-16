//! Decoded pictures, loaded once and kept for the frames that follow.
//!
//! A theme names an asset by its path inside the theme; decoding is the
//! backend's job because pixels are. The cache keys on that path, so a picture
//! used by several nodes is decoded once, and a path that fails to decode is
//! remembered as a failure rather than retried every frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A decoded picture as straight RGBA8, the same layout a [`crate::Raster`]
/// holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pictures a render can reach, relative to the theme's own directory.
#[derive(Debug, Default)]
pub struct Images {
    base: Option<PathBuf>,
    cache: HashMap<String, Option<Decoded>>,
}

impl Images {
    /// Pictures are looked up beside the theme file.
    pub fn at(base: impl AsRef<Path>) -> Self {
        Self {
            base: Some(base.as_ref().to_path_buf()),
            cache: HashMap::new(),
        }
    }

    /// No pictures: every image node draws nothing. A frame rendered without an
    /// asset source says so rather than inventing a picture.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A picture the caller holds in memory, such as one just imported and not
    /// yet saved. It takes the same path the document will store it under.
    pub fn put(&mut self, path: &str, bytes: &[u8]) {
        let decoded = decode_bytes(bytes);
        self.cache.insert(path.to_owned(), decoded);
    }

    /// The picture at an asset path, decoded on first use. `None` means the
    /// bytes are missing or are not an image this build reads, and nothing is
    /// substituted for them.
    pub fn load(&mut self, path: &str) -> Option<&Decoded> {
        if !self.cache.contains_key(path) {
            let decoded = self.base.as_ref().and_then(|base| decode(&base.join(path)));
            self.cache.insert(path.to_owned(), decoded);
        }
        self.cache.get(path).and_then(Option::as_ref)
    }
}

fn decode(path: &Path) -> Option<Decoded> {
    let bytes = std::fs::read(path).ok()?;
    decode_bytes(&bytes)
}

fn decode_bytes(bytes: &[u8]) -> Option<Decoded> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(Decoded {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_path_is_absent_and_stays_absent() {
        let mut images = Images::at(std::env::temp_dir());
        assert!(images.load("no-such-picture.png").is_none());
        assert!(images.load("no-such-picture.png").is_none());
    }

    #[test]
    fn an_empty_store_has_no_pictures() {
        let mut images = Images::empty();
        assert!(images.load("assets/a.png").is_none());
    }
}
