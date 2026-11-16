//! Saving, opening, exporting and importing.
//!
//! A save names the base it was written against; a save that names none onto an
//! id that exists is a conflict rather than a silent overwrite. Save As adopts
//! the new identity only after the bytes are stored, so a failed write cannot
//! leave the document believing it is clean.

use std::collections::BTreeMap;

use yunari_library::package::{self, Package};
use yunari_library::{Library, LibraryError, SaveRequest, Saved};

use crate::document::{Editor, Source};

impl Editor {
    /// Writes the open document to its library theme.
    pub fn save(&mut self, library: &Library, overwrite: bool) -> Result<Saved, LibraryError> {
        let Source::Library { id, base } = self.source().clone() else {
            return Err(LibraryError::Conflict {
                id: self.document().id.clone(),
            });
        };
        let saved = library.save(&SaveRequest {
            id: &id,
            document: self.document(),
            assets: self.assets(),
            base: base.as_deref(),
            overwrite,
        })?;
        self.mark_saved(saved.entry.id.clone(), saved.base.clone());
        Ok(saved)
    }

    /// Writes the document under a new id and adopts it.
    pub fn save_as(&mut self, library: &Library, id: &str) -> Result<Saved, LibraryError> {
        self.save_as_with(library, id, &BTreeMap::new())
    }

    /// Save As with assets the caller holds outside the editor (a picture the
    /// author just inserted, before the asset store exists).
    pub fn save_as_with(
        &mut self,
        library: &Library,
        id: &str,
        extra_assets: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Saved, LibraryError> {
        let mut document = self.document().clone();
        document.id = id.to_owned();
        let mut assets: BTreeMap<String, Vec<u8>> = self
            .assets()
            .iter()
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect();
        for (path, bytes) in extra_assets {
            assets.insert(path.clone(), bytes.clone());
        }
        let saved = library.save(&SaveRequest {
            id,
            document: &document,
            assets: &assets,
            base: None,
            overwrite: false,
        })?;
        self.replace_assets(assets);
        // The document's id is part of it, but identity is not an edit: the id
        // is adopted across history, and the baseline is taken after, so the
        // freshly stored document is clean and undo never walks back to the old
        // id.
        self.adopt_id(id);
        self.mark_saved(saved.entry.id.clone(), saved.base.clone());
        Ok(saved)
    }

    /// The document as a portable package.
    pub fn export(&self) -> Result<Vec<u8>, String> {
        package::write_package(self.document(), self.assets(), None)
    }

    /// Opens a stored theme together with the asset bytes it declares. A save
    /// writes a fresh directory, so opening without them would drop the files.
    pub fn open_stored(&mut self, library: &Library, id: &str) -> Result<(), LibraryError> {
        let stored = library.read(id)?;
        let assets = library.assets(id, &stored.document);
        self.open(stored);
        self.replace_assets(assets);
        Ok(())
    }
}

/// Reads a package, stores it under the first free id, and hands back that id
/// so a caller can open it.
pub fn import_package(library: &Library, bytes: &[u8]) -> Result<String, String> {
    let package: Package = package::read_package(bytes)?;
    let saved = package::import_into(library, &package).map_err(|error| error.to_string())?;
    Ok(saved.entry.id)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use yunari_model::document::{
        Artboard, ArtboardShape, AssetKind, AssetReference, Background, ContentFit, Metadata, Node,
        NodeKind, ThemeDocument, Transform,
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

    fn library() -> (Library, std::path::PathBuf) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "yunari-editor-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        (Library::new(&root), root)
    }

    fn rectangle() -> Node {
        Node {
            id: "rect-1".to_owned(),
            name: None,
            kind: NodeKind::Rectangle { corner_radius: 0.0 },
            transform: Transform {
                x: 10.0,
                y: 10.0,
                width: 100.0,
                height: 50.0,
                ..Transform::default()
            },
            visible: true,
            locked: false,
            style: BTreeMap::new(),
            glass: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn a_first_save_as_adopts_the_identity_and_marks_clean() {
        let (library, root) = library();
        let mut editor = Editor::new(document("draft"));
        assert!(editor.add_node(rectangle()));
        let saved = editor.save_as(&library, "first").expect("a first save");
        assert_eq!(saved.entry.id, "first");
        assert!(!editor.is_dirty());
        assert_eq!(editor.document().id, "first");
        let reopened = library.read("first").expect("a stored theme");
        assert_eq!(reopened.document.nodes.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_save_after_a_conflict_reports_it_and_stays_dirty() {
        let (library, root) = library();
        let mut editor = Editor::new(document("draft"));
        editor.save_as(&library, "first").expect("a first save");
        editor.rename_theme("Edited");
        // Someone else wrote the file under the same id meanwhile.
        fs::write(library.root().join("first/theme.json"), b"{}").expect("a foreign write");
        assert!(editor.save(&library, false).is_err());
        assert!(editor.is_dirty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_export_reads_back_as_the_same_document() {
        let mut editor = Editor::new(document("draft"));
        editor.add_node(rectangle());
        let bytes = editor.export().expect("a package");
        let package = package::read_package(&bytes).expect("a readable package");
        assert_eq!(package.document, *editor.document());
    }

    #[test]
    fn an_imported_theme_gets_a_free_id() {
        let (library, root) = library();
        let mut editor = Editor::new(document("alpha"));
        editor.save_as(&library, "alpha").expect("a first save");
        let bytes = editor.export().expect("a package");
        let id = import_package(&library, &bytes).expect("an imported copy");
        assert_eq!(id, "alpha-2");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reopening_a_theme_keeps_the_assets_a_save_never_read() {
        let (library, root) = library();
        let mut editor = Editor::new(document("alpha"));
        editor.edit(|document| {
            document.assets.push(AssetReference {
                id: "pic".to_owned(),
                kind: AssetKind::Image,
                path: "assets/pic.png".to_owned(),
                sha256: None,
                source_url: None,
                license: None,
                family: None,
                weight: None,
                style: None,
                format: None,
                subset: None,
            });
        });
        editor.put_asset("assets/pic.png", b"not really a png".to_vec());
        editor.save_as(&library, "alpha").expect("a first save");

        let mut reopened = Editor::new(document("draft"));
        reopened
            .open_stored(&library, "alpha")
            .expect("a stored theme");
        assert_eq!(
            reopened.assets().get("assets/pic.png").map(Vec::as_slice),
            Some(&b"not really a png"[..])
        );
        reopened.rename_theme("Edited");
        reopened.save(&library, false).expect("a save after opening");
        // The file is still on disk, because the save handed it back.
        let stored = fs::read(library.root().join("alpha/assets/pic.png"));
        assert_eq!(stored.expect("a kept file"), b"not really a png");
        let _ = fs::remove_dir_all(root);
    }
}
