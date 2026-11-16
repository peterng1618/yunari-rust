//! The displays this host serves, and the credentials that prove a phone is
//! one of them.
//!
//! The file is `display-clients.json`, not `displays.json`: `display.json`
//! already holds how readings are presented, and two files one character apart
//! with different owners is a defect waiting for a tired reader.

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use yunari_panels::manager::PanelDeviceId;

use crate::config::{read_json, write_json};

/// The key an unpaired client is served as, and the fallback for a display
/// with no assignment of its own. A key rather than a record: never minted.
pub const DEFAULT_DISPLAY: &str = "default";
/// The machine's old theme key. Never minted, so no display can inherit it.
pub const RETIRED_MACHINE_DISPLAY_ID: &str = "this-pc";
pub const MAX_LABEL_LENGTH: usize = 64;
pub const MAX_ID_LENGTH: usize = 64;
/// How often `last_seen_at` is written; a poll every second would rewrite the
/// file every second.
pub const TOUCH_INTERVAL_MS: i64 = 5_000;

const FILE: &str = "display-clients.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayKind {
    Paired,
    Local,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayShape {
    Rect,
    Circle,
}

impl DisplayShape {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "rect" => Some(Self::Rect),
            "circle" => Some(Self::Circle),
            _ => None,
        }
    }

    /// A rectangle turns in quarter steps; a circle takes any whole degree.
    pub fn accepts_rotation(self, degrees: u16) -> bool {
        match self {
            Self::Rect => degrees <= 270 && degrees.is_multiple_of(90),
            Self::Circle => degrees <= 359,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayRecord {
    pub id: String,
    pub label: String,
    pub kind: DisplayKind,
    pub created_at: String,
    pub last_seen_at: String,
    /// Stored hash of a paired display's token; absent on every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_hash: Option<String>,
    /// The monitor uuid a local display holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<DisplayShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<u16>,
    /// The physical panel this record drives, when it is a panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<PanelDeviceId>,
}

impl DisplayRecord {
    /// What a settings surface may show: the record without the credential it
    /// proves itself with. A hash is not a token, but it is not for display
    /// either.
    pub fn peer_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "label": self.label,
            "kind": self.kind,
            "createdAt": self.created_at,
            "lastSeenAt": self.last_seen_at,
            "monitor": self.monitor,
            "shape": self.shape,
            "rotation": self.rotation,
            "device": self.device,
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    displays: Vec<DisplayRecord>,
}

/// A label the store refuses, in the store's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRefusal(pub String);

/// The displays on this machine.
#[derive(Debug, Clone)]
pub struct DisplayStore {
    path: PathBuf,
    records: Vec<DisplayRecord>,
}

impl DisplayStore {
    /// Read the file; a missing or unreadable one is an empty store, because
    /// one broken settings file must not stop the host from starting.
    pub fn load(settings_dir: &Path) -> Self {
        let path = settings_dir.join(FILE);
        let stored: File = read_json(&path);
        let mut seen = std::collections::BTreeSet::new();
        let records = stored
            .displays
            .into_iter()
            .filter(|record| is_stable_id(&record.id) && seen.insert(record.id.clone()))
            .collect();
        Self { path, records }
    }

    pub fn list(&self) -> &[DisplayRecord] {
        &self.records
    }

    pub fn get(&self, id: &str) -> Option<&DisplayRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    /// The record an unpaired page is served as.
    pub fn default_record(&self) -> DisplayRecord {
        let exists = self.get(DEFAULT_DISPLAY);
        if let Some(record) = exists {
            return record.clone();
        }
        DisplayRecord {
            id: DEFAULT_DISPLAY.to_owned(),
            label: "This PC".to_owned(),
            kind: DisplayKind::Local,
            created_at: now(),
            last_seen_at: now(),
            token_hash: None,
            monitor: None,
            shape: None,
            rotation: None,
            device: None,
        }
    }

    /// Add a paired display, minting its id from the label and its token from
    /// the platform's randomness. Returns the token, which is never stored.
    pub fn adopt(
        &mut self,
        label: &str,
        shape: Option<DisplayShape>,
        rotation: Option<u16>,
    ) -> Result<(DisplayRecord, String), LabelRefusal> {
        let label = normalize_label(label)?;
        let id = self.free_id(&label);
        let token = mint_token();
        let record = DisplayRecord {
            id,
            label,
            kind: DisplayKind::Paired,
            created_at: now(),
            last_seen_at: now(),
            token_hash: Some(hash_token(&token)),
            monitor: None,
            shape,
            rotation,
            device: None,
        };
        self.records.push(record.clone());
        self.save();
        Ok((record, token))
    }

    /// Add a display this PC drives itself: a monitor window, with no token.
    pub fn adopt_local(
        &mut self,
        label: &str,
        monitor: Option<String>,
        shape: Option<DisplayShape>,
        rotation: Option<u16>,
    ) -> Result<DisplayRecord, LabelRefusal> {
        let label = normalize_label(label)?;
        let id = self.free_id(&label);
        let record = DisplayRecord {
            id,
            label,
            kind: DisplayKind::Local,
            created_at: now(),
            last_seen_at: now(),
            token_hash: None,
            monitor,
            shape,
            rotation,
            device: None,
        };
        self.records.push(record.clone());
        self.save();
        Ok(record)
    }

    /// Add a physical panel this host drives. Its shape comes from the family,
    /// because a panel's glass is a property of the hardware, not the record.
    pub fn adopt_panel(
        &mut self,
        label: &str,
        device: PanelDeviceId,
    ) -> Result<DisplayRecord, LabelRefusal> {
        let label = normalize_label(label)?;
        let id = self.free_id(&label);
        let shape =
            yunari_panels::for_device(device.vendor_id, device.product_id).map(
                |family| match family.shape {
                    yunari_panels::PanelShape::Rect => DisplayShape::Rect,
                    yunari_panels::PanelShape::Circle => DisplayShape::Circle,
                },
            );
        let record = DisplayRecord {
            id,
            label,
            kind: DisplayKind::Panel,
            created_at: now(),
            last_seen_at: now(),
            token_hash: None,
            monitor: None,
            shape,
            rotation: None,
            device: Some(device),
        };
        self.records.push(record.clone());
        self.save();
        Ok(record)
    }

    /// Change what a record says about itself. A missing field is left alone.
    pub fn patch(
        &mut self,
        id: &str,
        label: Option<&str>,
        shape: Option<DisplayShape>,
        rotation: Option<u16>,
    ) -> Result<DisplayRecord, LabelRefusal> {
        let label = match label {
            Some(label) => Some(normalize_label(label)?),
            None => None,
        };
        let Some(record) = self.records.iter_mut().find(|record| record.id == id) else {
            return Err(LabelRefusal("No such display.".to_owned()));
        };
        if let Some(label) = label {
            record.label = label;
        }
        if shape.is_some() {
            record.shape = shape;
        }
        if rotation.is_some() {
            record.rotation = rotation;
        }
        let patched = record.clone();
        self.save();
        Ok(patched)
    }

    /// The display id a token proves, touching `last_seen_at` at most once per
    /// `TOUCH_INTERVAL_MS`.
    pub fn authenticate(&mut self, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        let wanted = hash_token(token);
        let index = self
            .records
            .iter()
            .position(|record| record.token_hash.as_deref() == Some(wanted.as_str()))?;
        let now = now();
        let stale = self.records[index].last_seen_at.is_empty()
            || jiff::Timestamp::now().as_millisecond()
                - parse_ms(&self.records[index].last_seen_at).unwrap_or(0)
                >= TOUCH_INTERVAL_MS;
        if stale {
            self.records[index].last_seen_at = now;
            self.save();
        }
        Some(self.records[index].id.clone())
    }

    /// Issue a new token for a paired display, retiring the old one.
    pub fn rotate(&mut self, id: &str) -> Option<String> {
        let token = mint_token();
        let hash = hash_token(&token);
        let record = self
            .records
            .iter_mut()
            .find(|record| record.id == id && record.kind == DisplayKind::Paired)?;
        record.token_hash = Some(hash);
        self.save();
        Some(token)
    }

    pub fn set_label(&mut self, id: &str, label: &str) -> Result<(), LabelRefusal> {
        let label = normalize_label(label)?;
        let Some(record) = self.records.iter_mut().find(|record| record.id == id) else {
            return Err(LabelRefusal("No such display.".to_owned()));
        };
        record.label = label;
        self.save();
        Ok(())
    }

    /// The default is a key, not a record: nothing removes it.
    pub fn remove(&mut self, id: &str) -> bool {
        if id == DEFAULT_DISPLAY {
            return false;
        }
        let before = self.records.len();
        self.records.retain(|record| record.id != id);
        if self.records.len() != before {
            self.save();
        }
        self.records.len() != before
    }

    fn free_id(&self, label: &str) -> String {
        let base = slug(label);
        let base = if base.is_empty() {
            "display".to_owned()
        } else {
            base
        };
        let base: String = base.chars().take(MAX_ID_LENGTH - 5).collect();
        if !self.records.iter().any(|record| record.id == base)
            && base != RETIRED_MACHINE_DISPLAY_ID
        {
            return base;
        }
        loop {
            let mut bytes = [0u8; 2];
            rand::rng().fill(&mut bytes);
            let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            let candidate = format!("{base}-{suffix}");
            if !self.records.iter().any(|record| record.id == candidate) {
                return candidate;
            }
        }
    }

    fn save(&self) {
        let file = File {
            displays: self.records.clone(),
        };
        // A settings directory that cannot be written still serves this run.
        write_json(&self.path, &file).ok();
    }
}

/// The label a person sees: trimmed, 1 to 64 characters. Deliberately not the
/// slug rule — "Living Room (TV)" is a name someone may have.
pub fn normalize_label(input: &str) -> Result<String, LabelRefusal> {
    let label = input.trim();
    let length = label.chars().count();
    if !(1..=MAX_LABEL_LENGTH).contains(&length) {
        return Err(LabelRefusal(format!(
            "A display label must be 1 to {MAX_LABEL_LENGTH} characters, not {length}."
        )));
    }
    Ok(label.to_owned())
}

pub fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_token(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

fn is_stable_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LENGTH
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn slug(label: &str) -> String {
    let mut slug = String::new();
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_owned()
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn parse_ms(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (PathBuf, DisplayStore) {
        let dir =
            std::env::temp_dir().join(format!("yunari-displays-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let store = DisplayStore::load(&dir);
        (dir, store)
    }

    #[test]
    fn a_panel_record_names_its_device_and_takes_the_family_shape() {
        let (dir, mut store) = store("panel");
        let record = store
            .adopt_panel(
                "Kraken glass",
                PanelDeviceId {
                    vendor_id: 0x1e71,
                    product_id: 0x3008,
                    serial: "kraken-1".to_owned(),
                    device_id: None,
                },
            )
            .unwrap();
        assert_eq!(record.kind, DisplayKind::Panel);
        assert_eq!(record.shape, Some(DisplayShape::Circle));
        assert_eq!(
            record.device.as_ref().map(|device| device.serial.as_str()),
            Some("kraken-1")
        );

        // The record survives a reload with its device, or a panel would have
        // to be re-picked after every restart.
        let reloaded = DisplayStore::load(&dir);
        let stored = reloaded.get(&record.id).expect("the panel is stored");
        assert_eq!(stored.device, record.device);
        assert_eq!(stored.peer_json()["device"]["serial"], "kraken-1");
    }

    #[test]
    fn an_adopted_display_authenticates_and_stores_only_its_hash() {
        let (dir, mut store) = store("adopt");
        let (record, token) = store
            .adopt("Living Room (TV)", Some(DisplayShape::Rect), Some(90))
            .unwrap();
        assert_eq!(record.id, "living-room-tv");
        assert_eq!(record.kind, DisplayKind::Paired);
        assert_eq!(
            store.authenticate(&token).as_deref(),
            Some("living-room-tv")
        );
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!text.contains(&token), "the token is never written");
        assert!(text.contains("tokenHash"));
        assert_eq!(store.authenticate("nope"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotating_retires_the_old_token() {
        let (dir, mut store) = store("rotate");
        let (record, first) = store.adopt("Phone", None, None).unwrap();
        let second = store.rotate(&record.id).expect("a paired display");
        assert_eq!(store.authenticate(&first), None);
        assert_eq!(
            store.authenticate(&second).as_deref(),
            Some(record.id.as_str())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_default_is_a_key_that_is_never_removed() {
        let (dir, mut store) = store("default");
        assert!(!store.remove(DEFAULT_DISPLAY));
        assert_eq!(store.default_record().id, DEFAULT_DISPLAY);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ids_collide_into_a_suffix_and_stay_short() {
        let (dir, mut store) = store("ids");
        let first = store.adopt("Phone", None, None).unwrap().0;
        let second = store.adopt("phone", None, None).unwrap().0;
        assert_eq!(first.id, "phone");
        assert_ne!(second.id, first.id);
        assert!(second.id.starts_with("phone-"));
        let long = store.adopt(&"x".repeat(64), None, None).unwrap().0;
        assert!(long.id.len() <= MAX_ID_LENGTH);
        assert_eq!(
            normalize_label("   ").unwrap_err().0,
            "A display label must be 1 to 64 characters, not 0."
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_stored_record_round_trips() {
        let (dir, _) = store("round");
        let mut store = DisplayStore::load(&dir);
        let (record, _) = store
            .adopt("Desk", Some(DisplayShape::Circle), Some(45))
            .unwrap();
        let reloaded = DisplayStore::load(&dir);
        assert_eq!(
            reloaded.get(&record.id).unwrap().shape,
            Some(DisplayShape::Circle)
        );
        assert_eq!(reloaded.get(&record.id).unwrap().rotation, Some(45));
        assert_eq!(reloaded.get(&record.id).unwrap().kind, DisplayKind::Paired);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_rectangle_turns_in_quarters() {
        assert!(DisplayShape::Rect.accepts_rotation(90));
        assert!(!DisplayShape::Rect.accepts_rotation(45));
        assert!(DisplayShape::Circle.accepts_rotation(359));
        assert!(!DisplayShape::Circle.accepts_rotation(360));
    }
}
