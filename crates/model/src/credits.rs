//! `credits.json`: the attribution an imported asset travels with.
//!
//! The reader is total — a member that is not one, or an entry that is not
//! usable, answers nothing or is dropped — so one bad line cannot keep a theme
//! from opening. The writer restates the same cap it reads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_CREDITS_ENTRIES: usize = 256;
pub const FORMAT: &str = "yunari-theme-credits";
pub const VERSION: u32 = 1;

/// Who made the work: a plain name, a Steam account, or both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditsAuthor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_id: Option<String>,
}

/// Where a person reads the terms.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditsSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workshop_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreditsEntry {
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<CreditsAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CreditsSource>,
    pub license: String,
    pub changes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
}

impl CreditsEntry {
    /// The entry an import writes: the source's own licence and address, and
    /// the change an untouched copy records.
    pub fn imported(
        kind: impl Into<String>,
        title: impl Into<String>,
        license: impl Into<String>,
        source: Option<CreditsSource>,
        asset_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            author: None,
            source,
            license: license.into(),
            changes: "Imported unchanged".to_owned(),
            asset_id: Some(asset_id.into()),
        }
    }

    fn read(value: &Value) -> Option<Self> {
        let field = |key: &str| value.get(key).and_then(Value::as_str);
        Some(Self {
            kind: field("kind").filter(|text| !text.is_empty())?.to_owned(),
            title: field("title")?.to_owned(),
            author: value.get("author").and_then(|author| {
                let author = CreditsAuthor {
                    name: author
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    steam_id: author
                        .get("steamId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                };
                (author.name.is_some() || author.steam_id.is_some()).then_some(author)
            }),
            source: value.get("source").and_then(|source| {
                let source = CreditsSource {
                    url: source.get("url").and_then(Value::as_str).map(str::to_owned),
                    workshop_id: source
                        .get("workshopId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                };
                (source.url.is_some() || source.workshop_id.is_some()).then_some(source)
            }),
            license: field("license")?.to_owned(),
            changes: field("changes")?.to_owned(),
            asset_id: field("assetId")
                .filter(|id| !id.is_empty())
                .map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeCredits {
    pub format: String,
    pub version: u32,
    pub entries: Vec<CreditsEntry>,
}

impl ThemeCredits {
    pub fn new(entries: Vec<CreditsEntry>) -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            entries,
        }
    }

    /// The member's contents, or nothing when the bytes are not one. An entry
    /// that is not usable is dropped, not fatal to the rest.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        Self::normalize(&serde_json::from_slice::<Value>(bytes).ok()?)
    }

    pub fn normalize(value: &Value) -> Option<Self> {
        if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
            return None;
        }
        if value.get("version").and_then(Value::as_u64) != Some(u64::from(VERSION)) {
            return None;
        }
        let raw = value.get("entries")?.as_array()?;
        if raw.len() > MAX_CREDITS_ENTRIES {
            return None;
        }
        Some(Self::new(
            raw.iter().filter_map(CreditsEntry::read).collect(),
        ))
    }

    /// Adds one entry, replacing an earlier credit for the same asset. The
    /// writer refuses a list past the cap, so the import that would cross it is
    /// refused here rather than leaving a theme that cannot be saved.
    pub fn merged(&self, entry: CreditsEntry) -> Result<Self, String> {
        let mut entries: Vec<CreditsEntry> = self
            .entries
            .iter()
            .filter(|held| held.asset_id != entry.asset_id)
            .cloned()
            .collect();
        entries.push(entry);
        if entries.len() > MAX_CREDITS_ENTRIES {
            return Err(format!(
                "A theme carries at most {MAX_CREDITS_ENTRIES} credits; this import would add one more."
            ));
        }
        Ok(Self::new(entries))
    }

    /// The list without the credit an asset carried, when it had one.
    pub fn without(&self, asset_id: &str) -> Self {
        Self::new(
            self.entries
                .iter()
                .filter(|entry| entry.asset_id.as_deref() != Some(asset_id))
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_is_read_tolerantly_and_a_bad_entry_is_dropped() {
        let member = br#"{
            "format": "yunari-theme-credits",
            "version": 1,
            "entries": [
                {"kind": "image", "title": "Peak", "license": "Unsplash", "changes": "Imported unchanged", "assetId": "peak-1"},
                {"kind": "image", "title": "No licence"}
            ]
        }"#;
        let credits = ThemeCredits::read(member).expect("a member");
        assert_eq!(credits.entries.len(), 1);
        assert_eq!(credits.entries[0].asset_id.as_deref(), Some("peak-1"));
        assert!(ThemeCredits::read(b"{}").is_none());
        assert!(ThemeCredits::read(b"not json").is_none());
        let wrong_version = br#"{"format":"yunari-theme-credits","version":2,"entries":[]}"#;
        assert!(ThemeCredits::read(wrong_version).is_none());
    }

    #[test]
    fn an_import_replaces_its_asset_s_old_credit_and_the_cap_is_refused() {
        let first = ThemeCredits::new(vec![CreditsEntry::imported(
            "image", "Peak", "Unsplash", None, "peak-1",
        )]);
        let replaced = first
            .merged(CreditsEntry::imported(
                "image", "Peak", "Unsplash", None, "peak-1",
            ))
            .unwrap();
        assert_eq!(replaced.entries.len(), 1);

        let full = ThemeCredits::new(
            (0..MAX_CREDITS_ENTRIES)
                .map(|n| CreditsEntry::imported("image", "x", "Own work", None, format!("a{n}")))
                .collect(),
        );
        assert!(
            full.merged(CreditsEntry::imported(
                "image", "y", "Own work", None, "new"
            ))
            .is_err()
        );
        assert!(full.without("a0").entries.len() == MAX_CREDITS_ENTRIES - 1);
    }
}
