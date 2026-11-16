//! In-memory theme drafts the agent surface edits.
//!
//! A draft is a document the agent composes before anything is written: the
//! tools mutate it, `save_theme` stores it, and a draft that is never saved
//! leaves the library untouched. A host restart drops every draft; a draft is
//! a working board, not a record.

use std::collections::{BTreeMap, HashMap};

use yunari_model::document::ThemeDocument;

/// One working theme and the asset bytes it carries.
#[derive(Debug, Clone)]
pub struct Draft {
    pub document: ThemeDocument,
    pub assets: BTreeMap<String, Vec<u8>>,
    /// What a save was written against, when the draft came from the library.
    pub base: Option<String>,
}

#[derive(Debug, Default)]
pub struct Drafts {
    next: u64,
    open: HashMap<String, Draft>,
    /// The draft a tool edits when its call names none: the newest one.
    last: Option<String>,
}

impl Drafts {
    /// Starts a draft and returns its id. The document's own id is the draft
    /// id until a save gives it a stored identity.
    pub fn create(&mut self, document: ThemeDocument) -> String {
        self.next += 1;
        let id = format!("draft-{}", self.next);
        let mut document = document;
        document.id = id.clone();
        self.open.insert(
            id.clone(),
            Draft {
                document,
                assets: BTreeMap::new(),
                base: None,
            },
        );
        self.last = Some(id.clone());
        id
    }

    /// Takes an already-built draft (a stored theme being edited) under a new
    /// draft key, keeping the document's own identity.
    pub fn adopt(&mut self, draft: Draft) -> String {
        self.next += 1;
        let key = format!("draft-{}", self.next);
        self.last = Some(key.clone());
        self.open.insert(key.clone(), draft);
        key
    }

    /// The named draft, or the newest when the call names none.
    pub fn draft(&self, theme_id: Option<&str>) -> Option<&Draft> {
        match theme_id {
            Some(id) => self.open.get(id),
            None => self.last.as_ref().and_then(|id| self.open.get(id)),
        }
    }

    pub fn draft_mut(&mut self, theme_id: Option<&str>) -> Option<&mut Draft> {
        let id = match theme_id {
            Some(id) => Some(id.to_owned()),
            None => self.last.clone(),
        }?;
        self.open.get_mut(&id)
    }

    pub fn remove(&mut self, theme_id: Option<&str>) -> Option<Draft> {
        let id = match theme_id {
            Some(id) => Some(id.to_owned()),
            None => self.last.clone(),
        }?;
        if self.last.as_deref() == Some(id.as_str()) {
            self.last = None;
        }
        self.open.remove(&id)
    }

    pub fn ids(&self) -> Vec<&str> {
        self.open.keys().map(String::as_str).collect()
    }
}
