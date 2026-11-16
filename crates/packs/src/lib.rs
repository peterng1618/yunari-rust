//! Installed `yunari-widget-package` archives, read as card units.
//!
//! A package is untrusted: the archive gate caps it, the manifest is strict,
//! every declared asset must match its digest, and a unit's scene is lifted by
//! the one Fabric reader in `yunari_model::source`. What comes out is nodes,
//! bindings and bytes the editor can insert — never a second scene format.

pub mod installed;
pub mod package;

pub use installed::{InstalledPack, directory, install, installed};
pub use package::{
    MAX_ARCHIVE_BYTES, MAX_ENTRIES, MAX_ENTRY_BYTES, MAX_EXPANDED_BYTES, Pack, Unit, read,
};
