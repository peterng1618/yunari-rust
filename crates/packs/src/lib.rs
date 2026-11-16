//! Installed `yunari-widget-package` archives, read as card units.
//!
//! A package is untrusted: the archive gate caps it, the manifest is strict,
//! every declared asset must match its digest, and a unit's scene is lifted by
//! the one Fabric reader in `yunari_model::source`. What comes out is nodes,
//! bindings and bytes the editor can insert — never a second scene format.

pub mod installed;
pub mod package;
pub mod scene;

/// The card pack a fresh install starts with. Committed, never generated.
pub const STARTER: &[u8] = include_bytes!("../assets/starter-card.yunari-widget-package");

pub use installed::{InstalledPack, directory, install, installed};
pub use package::{
    MAX_ARCHIVE_BYTES, MAX_ENTRIES, MAX_ENTRY_BYTES, MAX_EXPANDED_BYTES, Pack, Unit, read, write,
};
pub use scene::{Inserted, insert, place};

#[cfg(test)]
mod tests {
    #[test]
    fn the_committed_starter_pack_reads_as_one_card() {
        let pack = super::read(super::STARTER).expect("the starter pack");
        assert_eq!(pack.name.as_deref(), Some("Starter cards"));
        assert_eq!(pack.units.len(), 1);
        let unit = &pack.units[0];
        assert_eq!(unit.id, "card");
        assert_eq!(unit.label, "CPU card");
        assert_eq!(unit.document.nodes.len(), 1);
        assert_eq!(unit.document.nodes[0].children.len(), 4);
        // The value row reads a key through the binding it declares.
        let value = &unit.document.nodes[0].children[3];
        let yunari_model::document::NodeKind::Text { content } = &value.kind else {
            panic!("a value row");
        };
        assert!(matches!(
            &content.runs[0],
            yunari_model::document::TextRun::Value { binding_id, .. } if binding_id == "reading"
        ));
    }
}
