//! Undo and redo, as whole-document snapshots.
//!
//! The source editor undoes the scene only, so a background colour or a binding
//! change could not be taken back. A snapshot here is the whole document, which
//! is both simpler and what an author expects of undo. An unchanged document is
//! never recorded, and a new edit drops the redo tail.

use yunari_model::document::ThemeDocument;

/// How many states an author can walk back.
pub const MAX_ENTRIES: usize = 100;

#[derive(Debug, Clone)]
pub struct History {
    entries: Vec<ThemeDocument>,
    index: usize,
}

impl History {
    pub fn new(document: ThemeDocument) -> Self {
        Self {
            entries: vec![document],
            index: 0,
        }
    }

    pub fn current(&self) -> &ThemeDocument {
        &self.entries[self.index]
    }

    /// Records a state, unless it is the one already current.
    pub fn commit(&mut self, document: ThemeDocument) -> bool {
        if *self.current() == document {
            return false;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(document);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        self.index = self.entries.len() - 1;
        true
    }

    /// Replaces the current state without recording a new entry, for a gesture
    /// that will be committed once when it ends.
    pub fn replace_current(&mut self, document: ThemeDocument) {
        self.entries[self.index] = document;
    }

    /// Changes every recorded state alike. Identity is not content: a save that
    /// adopts a new id must not make undo walk back into the id it replaced.
    pub fn map_all(&mut self, change: impl Fn(&mut ThemeDocument)) {
        for entry in &mut self.entries {
            change(entry);
        }
    }

    pub fn undo(&mut self) -> Option<&ThemeDocument> {
        if self.index == 0 {
            return None;
        }
        self.index -= 1;
        Some(self.current())
    }

    pub fn redo(&mut self) -> Option<&ThemeDocument> {
        if self.index + 1 >= self.entries.len() {
            return None;
        }
        self.index += 1;
        Some(self.current())
    }

    pub fn can_undo(&self) -> bool {
        self.index > 0
    }

    pub fn can_redo(&self) -> bool {
        self.index + 1 < self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use yunari_model::document::{
        Artboard, ArtboardShape, Background, ContentFit, Metadata, ThemeDocument,
    };

    use super::*;

    fn document(name: &str) -> ThemeDocument {
        ThemeDocument {
            schema_version: "3.0.0".to_owned(),
            fabric_version: "7.4.0".to_owned(),
            id: "alpha".to_owned(),
            metadata: Metadata {
                name: Some(name.to_owned()),
                ..Metadata::default()
            },
            artboard: Artboard {
                width: 450.0,
                height: 675.0,
                content_fit: ContentFit::Contain,
                shape: ArtboardShape::Rect,
            },
            background: Background::default(),
            globals: Default::default(),
            scene: Default::default(),
            palette: BTreeMap::new(),
            type_tokens: BTreeMap::new(),
            assets: Vec::new(),
            nodes: Vec::new(),
            bindings: BTreeMap::new(),
            editor_metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn an_edit_is_one_entry_and_undo_restores_the_document() {
        let mut history = History::new(document("first"));
        assert!(history.commit(document("second")));
        assert_eq!(history.current().metadata.name.as_deref(), Some("second"));
        assert_eq!(
            history.undo().and_then(|doc| doc.metadata.name.as_deref()),
            Some("first")
        );
        assert!(history.can_redo());
    }

    #[test]
    fn an_unchanged_document_records_no_entry() {
        let mut history = History::new(document("first"));
        assert!(!history.commit(document("first")));
        assert_eq!(history.len(), 1);
        assert!(!history.can_undo());
    }

    #[test]
    fn redo_is_dropped_by_a_new_edit() {
        let mut history = History::new(document("first"));
        history.commit(document("second"));
        history.undo();
        assert!(history.commit(document("third")));
        assert!(!history.can_redo());
    }

    #[test]
    fn history_is_capped_at_one_hundred_entries() {
        let mut history = History::new(document("start"));
        for index in 0..150 {
            history.commit(document(&format!("edit {index}")));
        }
        assert_eq!(history.len(), MAX_ENTRIES);
        assert!(history.can_undo());
    }
}
