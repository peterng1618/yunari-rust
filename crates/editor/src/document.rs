//! The document an author is working on.
//!
//! Every mutation goes through [`Editor::edit`], so undo, redo and the dirty
//! flag cannot disagree about what changed. Dirty is derived by comparing the
//! document to the bytes last saved, never from an edit counter.

use std::collections::BTreeMap;

use yunari_library::{Saved, StoredTheme};
use yunari_model::credits::{CreditsAuthor, CreditsEntry, CreditsSource, ThemeCredits};
use yunari_model::document::{AssetKind, AssetReference, Node, ThemeDocument};

use crate::assets::{self, ImportCredit};
use crate::clipboard::Clipboard;
use crate::history::History;

/// Where the open document belongs: nowhere yet, or a stored theme and the
/// base its next save is written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Unsaved,
    Library { id: String, base: Option<String> },
}

#[derive(Debug, Clone)]
pub struct Editor {
    history: History,
    /// The canonical JSON of the stored bytes, when this document has been
    /// saved. `None` for a document that has never been saved.
    saved: Option<String>,
    source: Source,
    /// Set while a gesture (a drag, a nudge burst) is in flight: edits change
    /// the document but record nothing until the gesture ends.
    burst: Option<ThemeDocument>,
    /// The bytes of every asset the document declares, by path.
    assets: BTreeMap<String, Vec<u8>>,
    /// The attribution the theme's imported assets carry. Attribution is not
    /// part of the document an undo walks, so it lives beside the bytes.
    credits: Option<ThemeCredits>,
    /// The last copy, which a paste inserts and no document state holds.
    pub(crate) clipboard: Option<Clipboard>,
}

impl Editor {
    pub fn new(document: ThemeDocument) -> Self {
        Self {
            history: History::new(document),
            saved: None,
            source: Source::Unsaved,
            burst: None,
            assets: BTreeMap::new(),
            credits: None,
            clipboard: None,
        }
    }

    /// A document opened from the library, clean as stored.
    pub fn opened(stored: StoredTheme) -> Self {
        let saved = canonical(&stored.document);
        Self {
            history: History::new(stored.document),
            saved: Some(saved),
            source: Source::Library {
                id: stored.entry.id,
                base: Some(stored.base),
            },
            burst: None,
            assets: BTreeMap::new(),
            credits: stored.credits,
            clipboard: None,
        }
    }

    pub fn document(&self) -> &ThemeDocument {
        self.history.current()
    }

    pub fn credits(&self) -> Option<&ThemeCredits> {
        self.credits.as_ref()
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn assets(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.assets
    }

    /// Declares an asset's bytes. Storing the same bytes again changes nothing.
    pub fn put_asset(&mut self, path: impl Into<String>, bytes: Vec<u8>) {
        self.assets.insert(path.into(), bytes);
    }

    /// Takes a file this PC holds as a theme asset, refusing anything this
    /// editor will not draw. Returns the new asset's id; the bytes are held
    /// beside the document until a save writes them.
    pub fn import_asset(&mut self, name: &str, bytes: Vec<u8>) -> Result<String, String> {
        self.import_asset_with(name, bytes, None)
    }

    /// The same import, carrying the credit a finder row knows for its source.
    pub fn import_asset_with(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        credit: Option<ImportCredit>,
    ) -> Result<String, String> {
        let kind = assets::sniff(name, &bytes)?;
        if kind == AssetKind::Svg {
            let text =
                std::str::from_utf8(&bytes).map_err(|_| assets::NOT_A_FILE_MESSAGE.to_owned())?;
            assets::sanitize_svg(text)?;
        }
        let limit = assets::limit(kind);
        if bytes.len() as u64 > limit {
            return Err(format!(
                "That file is larger than the {} MiB an imported file may be.",
                limit / (1024 * 1024)
            ));
        }
        let path = free_asset_path(
            self.document(),
            &assets::stem(name),
            assets::extension(kind),
        );
        let id = free_asset_id(self.document());
        let source_url = credit.as_ref().and_then(|credit| credit.source_url.clone());
        let license = credit
            .as_ref()
            .map(|credit| yunari_model::document::AssetLicense {
                name: Some(credit.license.clone()),
                url: source_url.clone(),
                attribution: None,
            });
        let reference = AssetReference {
            id: id.clone(),
            kind,
            path: path.clone(),
            // The renderer trusts the bytes beside the document, so the digest
            // is recorded for whoever wants to check them later.
            sha256: Some(yunari_library::sha256_hex(&bytes)),
            source_url,
            license,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        };
        self.edit(|document| document.assets.push(reference));
        self.put_asset(path, bytes);
        if let Some(credit) = credit {
            let entry = CreditsEntry {
                kind: credit.kind,
                title: credit.title,
                author: credit.author.map(|name| CreditsAuthor {
                    name: Some(name),
                    steam_id: None,
                }),
                source: credit.source_url.map(|url| CreditsSource {
                    url: Some(url),
                    workshop_id: None,
                }),
                license: credit.license,
                changes: "Imported unchanged".to_owned(),
                asset_id: Some(id.clone()),
            };
            let credits = match self.credits.take() {
                Some(held) => held.merged(entry)?,
                None => ThemeCredits::new(Vec::new()).merged(entry)?,
            };
            self.credits = Some(credits);
        }
        Ok(id)
    }

    pub fn replace_assets(&mut self, assets: BTreeMap<String, Vec<u8>>) {
        self.assets = assets;
    }

    /// Mutates the document and records one history entry. Returns whether the
    /// document actually changed.
    pub fn edit(&mut self, change: impl FnOnce(&mut ThemeDocument)) -> bool {
        let mut next = self.history.current().clone();
        change(&mut next);
        if next == *self.history.current() {
            return false;
        }
        if self.burst.is_some() {
            // Inside a gesture: keep the latest state, record nothing yet.
            self.history.replace_current(next);
            return true;
        }
        self.history.commit(next)
    }

    /// Starts a gesture. Edits until [`Self::end_burst`] are one history entry.
    pub fn begin_burst(&mut self) {
        if self.burst.is_none() {
            self.burst = Some(self.history.current().clone());
        }
    }

    /// Ends a gesture, recording one entry if anything changed.
    pub fn end_burst(&mut self) -> bool {
        let Some(before) = self.burst.take() else {
            return false;
        };
        let after = self.history.current().clone();
        if before == after {
            return false;
        }
        // Replace the silently-updated state with one recorded entry: undo must
        // land on the state the gesture started from.
        self.history.replace_current(before);
        self.history.commit(after)
    }

    pub fn undo(&mut self) -> bool {
        self.history.undo().is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.history.redo().is_some()
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn is_dirty(&self) -> bool {
        match &self.saved {
            Some(saved) => *saved != canonical(self.document()),
            None => true,
        }
    }

    /// The identity a save adopted: the theme id and the base its bytes now
    /// have. Marks the document clean.
    pub fn mark_saved(&mut self, id: impl Into<String>, base: impl Into<String>) {
        self.saved = Some(canonical(self.document()));
        self.source = Source::Library {
            id: id.into(),
            base: Some(base.into()),
        };
    }

    /// Re-opens a theme in place, dropping the open document's history.
    pub fn open(&mut self, stored: StoredTheme) {
        *self = Self::opened(stored);
    }

    /// Adopts an id for the document and every state undo can reach. Identity
    /// is not content, so this records no history entry.
    pub fn adopt_id(&mut self, id: &str) {
        self.history.map_all(|document| document.id = id.to_owned());
    }

    pub fn add_node(&mut self, node: Node) -> bool {
        self.edit(|document| document.nodes.push(node))
    }

    /// Removes nodes and everything under them, along with the bindings their
    /// ids held. A binding keyed to a removed id would otherwise invalidate
    /// every later save.
    pub fn remove_nodes(&mut self, ids: &[String]) -> bool {
        if ids.is_empty() {
            return false;
        }
        let mut removed = Vec::new();
        self.edit(|document| {
            let mut kept = Vec::new();
            for node in std::mem::take(&mut document.nodes) {
                if ids.contains(&node.id) {
                    collect_ids(&node, &mut removed);
                } else {
                    let mut node = node;
                    prune_children(&mut node, ids, &mut removed);
                    kept.push(node);
                }
            }
            document.nodes = kept;
            for id in &removed {
                document.bindings.remove(id);
            }
        })
    }

    /// Replaces the transforms of the named nodes in one edit, which is what a
    /// drag step is.
    pub fn apply_transforms(
        &mut self,
        changes: &[(String, yunari_model::document::Transform)],
    ) -> bool {
        if changes.is_empty() {
            return false;
        }
        self.edit(|document| {
            for node in &mut document.nodes {
                apply_one(node, changes);
            }
        })
    }

    pub fn rename_theme(&mut self, name: &str) -> bool {
        let name = name.trim().to_owned();
        self.edit(|document| document.metadata.name = Some(name))
    }
}

/// The document as the clean test sees it: one canonical JSON spelling.
fn canonical(document: &ThemeDocument) -> String {
    serde_json::to_string(document).unwrap_or_default()
}

/// A path no declared asset holds, so importing a second file of the same name
/// lands beside the first rather than over it.
fn free_asset_path(document: &ThemeDocument, stem: &str, extension: &str) -> String {
    let mut path = format!("assets/{stem}.{extension}");
    let mut n = 2;
    while document.assets.iter().any(|asset| asset.path == path) {
        path = format!("assets/{stem}-{n}.{extension}");
        n += 1;
    }
    path
}

fn free_asset_id(document: &ThemeDocument) -> String {
    let mut id = "asset".to_owned();
    let mut n = 2;
    while document.assets.iter().any(|asset| asset.id == id) {
        id = format!("asset-{n}");
        n += 1;
    }
    id
}

fn apply_one(node: &mut Node, changes: &[(String, yunari_model::document::Transform)]) {
    if let Some((_, transform)) = changes.iter().find(|(id, _)| *id == node.id) {
        node.transform = *transform;
    }
    for child in &mut node.children {
        apply_one(child, changes);
    }
}

fn collect_ids(node: &Node, into: &mut Vec<String>) {
    into.push(node.id.clone());
    for child in &node.children {
        collect_ids(child, into);
    }
}

fn prune_children(node: &mut Node, ids: &[String], removed: &mut Vec<String>) {
    let mut kept = Vec::new();
    for child in std::mem::take(&mut node.children) {
        if ids.contains(&child.id) {
            collect_ids(&child, removed);
        } else {
            let mut child = child;
            prune_children(&mut child, ids, removed);
            kept.push(child);
        }
    }
    node.children = kept;
}

/// A save's identity, for a caller that only has the result.
pub fn saved_identity(saved: &Saved) -> (&str, &str) {
    (saved.entry.id.as_str(), saved.base.as_str())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::{
        Artboard, ArtboardShape, Background, ContentFit, Metadata, NodeKind, Transform,
    };

    use super::*;

    fn document() -> ThemeDocument {
        ThemeDocument {
            schema_version: "1.0.0".to_owned(),
            id: "alpha".to_owned(),
            metadata: Metadata {
                name: Some("Alpha".to_owned()),
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

    fn rectangle(id: &str) -> Node {
        Node {
            id: id.to_owned(),
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
    fn a_burst_is_one_entry() {
        let mut editor = Editor::new(document());
        editor.begin_burst();
        editor.edit(|document| document.metadata.description = Some("one".to_owned()));
        editor.edit(|document| document.metadata.description = Some("two".to_owned()));
        assert!(editor.end_burst());
        assert_eq!(
            editor.document().metadata.description.as_deref(),
            Some("two")
        );
        assert!(editor.undo());
        assert_eq!(editor.document().metadata.description, None);
        assert!(!editor.can_undo());
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut editor = Editor::new(document());
        let start = rectangle("a");
        editor.add_node(start.clone());
        editor.begin_burst();
        for step in 1..=3 {
            let mut moved = start.transform;
            moved.x = start.transform.x + f64::from(step);
            editor.apply_transforms(&[(start.id.clone(), moved)]);
        }
        assert!(editor.end_burst());
        assert_eq!(editor.document().nodes[0].transform.x, 13.0);
        assert!(editor.undo());
        assert_eq!(editor.document().nodes[0].transform.x, 10.0);
    }

    #[test]
    fn dirty_is_derived_from_the_saved_document() {
        let mut editor = Editor::new(document());
        assert!(editor.is_dirty(), "a new document is unsaved");
        editor.mark_saved("alpha", "base");
        assert!(!editor.is_dirty());
        editor.rename_theme("Renamed");
        assert!(editor.is_dirty());
        editor.undo();
        assert!(
            !editor.is_dirty(),
            "undoing back to the saved state is clean"
        );
    }

    #[test]
    fn save_as_adopts_the_new_identity() {
        let mut editor = Editor::new(document());
        editor.mark_saved("beta", "hash");
        assert_eq!(
            editor.source(),
            &Source::Library {
                id: "beta".to_owned(),
                base: Some("hash".to_owned()),
            }
        );
    }

    #[test]
    fn removing_a_group_takes_its_children_and_bindings() {
        let mut editor = Editor::new(document());
        let mut group = rectangle("group");
        group.children.push(rectangle("child"));
        editor.add_node(group);
        editor.edit(|document| {
            document.bindings.insert("child".to_owned(), Vec::new());
        });
        assert!(editor.remove_nodes(&["group".to_owned()]));
        assert!(editor.document().nodes.is_empty());
        assert!(editor.document().bindings.is_empty());
    }

    #[test]
    fn an_imported_picture_is_an_asset_with_its_bytes_beside_the_document() {
        const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        let mut editor = Editor::new(document());
        let id = editor
            .import_asset("My Photo.PNG", PNG.to_vec())
            .expect("an import");
        let asset = editor
            .document()
            .assets
            .iter()
            .find(|asset| asset.id == id)
            .expect("the asset");
        assert_eq!(asset.path, "assets/my-photo.png");
        assert_eq!(asset.kind, AssetKind::Image);
        assert!(asset.sha256.as_deref().is_some_and(|sha| sha.len() == 64));
        assert_eq!(
            editor.assets().get("assets/my-photo.png"),
            Some(&PNG.to_vec())
        );
        // A second file of the same name lands beside the first.
        let second = editor
            .import_asset("my photo.png", PNG.to_vec())
            .expect("a second import");
        assert_ne!(second, id);
        assert_eq!(editor.document().assets.len(), 2);
        assert_ne!(
            editor.document().assets[0].path,
            editor.document().assets[1].path
        );
    }

    #[test]
    fn a_file_that_is_not_a_picture_leaves_the_document_alone() {
        let mut editor = Editor::new(document());
        assert_eq!(
            editor.import_asset("a.png", b"PK\x03\x04".to_vec()),
            Err(assets::NOT_A_FILE_MESSAGE.to_owned())
        );
        assert_eq!(
            editor.import_asset("x.svg", b"<svg><script/></svg>".to_vec()),
            Err(assets::UNSAFE_SVG_MESSAGE.to_owned())
        );
        assert!(editor.document().assets.is_empty());
        assert!(editor.assets().is_empty());
        assert!(!editor.can_undo());
    }

    #[test]
    fn a_credited_import_lands_with_its_attribution() {
        const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        let mut editor = Editor::new(document());
        let id = editor
            .import_asset_with(
                "Dusk.png",
                PNG.to_vec(),
                Some(ImportCredit {
                    kind: "image".to_owned(),
                    title: "Dusk".to_owned(),
                    license: "CC0-1.0".to_owned(),
                    source_url: Some("https://example.test/dusk".to_owned()),
                    author: Some("A. Photographer".to_owned()),
                }),
            )
            .expect("a credited import");
        let asset = editor
            .document()
            .assets
            .iter()
            .find(|asset| asset.id == id)
            .expect("the asset");
        let license = asset.license.as_ref().expect("a licence");
        assert_eq!(license.name.as_deref(), Some("CC0-1.0"));
        assert_eq!(
            asset.source_url.as_deref(),
            Some("https://example.test/dusk")
        );
        let credits = editor.credits().expect("the credits the import carried");
        assert_eq!(credits.entries.len(), 1);
        let entry = &credits.entries[0];
        assert_eq!(entry.asset_id.as_deref(), Some(id.as_str()));
        assert_eq!(entry.license, "CC0-1.0");
        assert_eq!(
            entry
                .author
                .as_ref()
                .and_then(|author| author.name.as_deref()),
            Some("A. Photographer")
        );
    }

    #[test]
    fn opening_a_stored_theme_starts_clean() {
        let mut editor = Editor::new(document());
        editor.rename_theme("Dirty");
        editor.open(StoredTheme {
            entry: yunari_library::ThemeEntry {
                id: "alpha".to_owned(),
                name: "Alpha".to_owned(),
                author: None,
                version: None,
                updated_at: std::time::UNIX_EPOCH,
                node_count: 0,
            },
            document: document(),
            base: "base".to_owned(),
            credits: None,
        });
        assert!(!editor.is_dirty());
        assert!(!editor.can_undo());
    }
}
