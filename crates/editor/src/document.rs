//! The document an author is working on.
//!
//! Every mutation goes through [`Editor::edit`], so undo, redo and the dirty
//! flag cannot disagree about what changed. Dirty is derived by comparing the
//! document to the bytes last saved, never from an edit counter.

use yunari_library::{Saved, StoredTheme};
use yunari_model::document::{Node, ThemeDocument};

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
}

impl Editor {
    pub fn new(document: ThemeDocument) -> Self {
        Self {
            history: History::new(document),
            saved: None,
            source: Source::Unsaved,
            burst: None,
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
        }
    }

    pub fn document(&self) -> &ThemeDocument {
        self.history.current()
    }

    pub fn source(&self) -> &Source {
        &self.source
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

    pub fn rename_theme(&mut self, name: &str) -> bool {
        let name = name.trim().to_owned();
        self.edit(|document| document.metadata.name = Some(name))
    }
}

/// The document as the clean test sees it: one canonical JSON spelling.
fn canonical(document: &ThemeDocument) -> String {
    serde_json::to_string(document).unwrap_or_default()
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
    fn dirty_is_derived_from_the_saved_document() {
        let mut editor = Editor::new(document());
        assert!(editor.is_dirty(), "a new document is unsaved");
        editor.mark_saved("alpha", "base");
        assert!(!editor.is_dirty());
        editor.rename_theme("Renamed");
        assert!(editor.is_dirty());
        editor.undo();
        assert!(!editor.is_dirty(), "undoing back to the saved state is clean");
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
            document
                .bindings
                .insert("child".to_owned(), Vec::new());
        });
        assert!(editor.remove_nodes(&["group".to_owned()]));
        assert!(editor.document().nodes.is_empty());
        assert!(editor.document().bindings.is_empty());
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
        });
        assert!(!editor.is_dirty());
        assert!(!editor.can_undo());
    }
}
