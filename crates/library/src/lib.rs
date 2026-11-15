#![forbid(unsafe_code)]
//! Where themes live.
//!
//! A theme is a directory read and written in place — the archive format is
//! only what an author exports, so nothing on the working path compresses and
//! re-expands. A save is bounded, validated and swapped into place, and the
//! base it returns is the identity a later save is written against.

pub mod store;

pub use store::{
    Library, LibraryError, SaveRequest, Saved, StoredTheme, ThemeEntry, default_root,
    sha256_hex,
};
