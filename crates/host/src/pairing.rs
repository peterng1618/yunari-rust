//! One short code pairs one phone.
//!
//! The code is drawn from an alphabet without the letters and digits people
//! confuse, it lives ten minutes, and a phone that guesses wrong waits longer
//! each time. Guessing is per address, so one noisy phone cannot slow down a
//! person at another.

use std::collections::HashMap;

use rand::prelude::*;

use crate::displays::{DisplayRecord, DisplayShape, DisplayStore, LabelRefusal};

pub const PAIRING_CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const PAIRING_CODE_LENGTH: usize = 6;
pub const PAIRING_CODE_TTL_MS: i64 = 10 * 60 * 1000;
pub const BASE_PENALTY_MS: i64 = 250;
pub const MAX_PENALTY_MS: i64 = 4_000;

/// The one sentence every wrong, expired or spent code gets, so a probe cannot
/// tell them apart.
pub const NO_PAIRING_CODE: &str = "That pairing code is not the one this host is showing.";

#[derive(Debug, Clone)]
struct Live {
    code: String,
    expires_ms: i64,
}

#[derive(Debug, Default)]
pub struct Pairing {
    live: Option<Live>,
    /// Wrong guesses per remote address since the last mint or success.
    misses: HashMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeView {
    pub code: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// The code was wrong, spent or expired. The message is fixed.
    Refused,
    Label(LabelRefusal),
}

impl Pairing {
    /// A fresh code, which also forgives every address its misses.
    pub fn mint(&mut self, now_ms: i64) -> CodeView {
        let mut rng = rand::rng();
        let code: String = (0..PAIRING_CODE_LENGTH)
            .map(|_| {
                let index = rng.random_range(0..PAIRING_CODE_ALPHABET.len());
                PAIRING_CODE_ALPHABET.as_bytes()[index] as char
            })
            .collect();
        self.misses.clear();
        self.live = Some(Live {
            code: code.clone(),
            expires_ms: now_ms + PAIRING_CODE_TTL_MS,
        });
        CodeView {
            code,
            expires_at: self.live.as_ref().map_or(0, |live| live.expires_ms),
        }
    }

    /// The live code, minting one when there is none or it has expired.
    pub fn current(&mut self, now_ms: i64) -> CodeView {
        match &self.live {
            Some(live) if live.expires_ms > now_ms => CodeView {
                code: live.code.clone(),
                expires_at: live.expires_ms,
            },
            _ => self.mint(now_ms),
        }
    }

    /// How long this address must wait before its next attempt. The first
    /// wrong guess is free; each later one doubles the wait, up to four
    /// seconds.
    pub fn miss(&mut self, remote: &str) -> i64 {
        let missed = self.misses.entry(remote.to_owned()).or_insert(0);
        *missed += 1;
        penalty(*missed)
    }

    /// Whether `code` is the live one right now.
    pub fn matches(&self, code: &str, now_ms: i64) -> bool {
        self.live
            .as_ref()
            .is_some_and(|live| live.expires_ms > now_ms && live.code == code)
    }

    /// The code was used; the next phone needs a new one.
    pub fn spend(&mut self) {
        self.live = None;
    }

    /// Claim a display for `code`. The caller decides the penalty; this only
    /// answers whether the code was right.
    pub fn claim(
        &mut self,
        code: &str,
        now_ms: i64,
        store: &mut DisplayStore,
        label: &str,
        shape: Option<DisplayShape>,
        rotation: Option<u16>,
    ) -> Result<(DisplayRecord, String), ClaimError> {
        if !self.matches(code, now_ms) {
            return Err(ClaimError::Refused);
        }
        let adopted = store.adopt(label, shape, rotation).map_err(ClaimError::Label)?;
        self.spend();
        Ok(adopted)
    }
}

pub fn penalty(missed: u32) -> i64 {
    match missed {
        0 | 1 => 0,
        _ => {
            let steps = i32::try_from(missed - 1).unwrap_or(i32::MAX).min(20);
            (BASE_PENALTY_MS.saturating_mul(1 << steps)).min(MAX_PENALTY_MS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn store(name: &str) -> (PathBuf, DisplayStore) {
        let dir = std::env::temp_dir().join(format!("yunari-pairing-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let store = DisplayStore::load(&dir);
        (dir, store)
    }

    #[test]
    fn a_code_is_six_unambiguous_characters() {
        let mut pairing = Pairing::default();
        let code = pairing.mint(0);
        assert_eq!(code.code.len(), 6);
        for character in code.code.chars() {
            assert!(PAIRING_CODE_ALPHABET.contains(character), "{character}");
            assert!(!"ILO01".contains(character));
        }
        assert_eq!(code.expires_at, PAIRING_CODE_TTL_MS);
    }

    #[test]
    fn the_live_code_is_reused_until_it_expires() {
        let mut pairing = Pairing::default();
        let first = pairing.current(0);
        assert_eq!(pairing.current(1_000).code, first.code);
        let later = pairing.current(PAIRING_CODE_TTL_MS);
        assert_ne!(later.code, first.code);
    }

    #[test]
    fn misses_are_free_once_then_double_to_the_cap() {
        assert_eq!(penalty(0), 0);
        assert_eq!(penalty(1), 0);
        assert_eq!(penalty(2), 500);
        assert_eq!(penalty(3), 1_000);
        assert_eq!(penalty(4), 2_000);
        assert_eq!(penalty(5), 4_000);
        assert_eq!(penalty(6), 4_000);
        assert_eq!(penalty(30), 4_000);
    }

    #[test]
    fn a_claim_spends_the_code_and_mints_a_token() {
        let (dir, mut store) = store("claim");
        let mut pairing = Pairing::default();
        let view = pairing.mint(0);
        let (record, token) = pairing
            .claim(&view.code, 1, &mut store, "Phone", Some(DisplayShape::Rect), Some(90))
            .expect("the live code");
        assert_eq!(store.authenticate(&token).as_deref(), Some(record.id.as_str()));
        assert_eq!(
            pairing.claim(&view.code, 2, &mut store, "Again", None, None),
            Err(ClaimError::Refused)
        );
        let fresh = pairing.current(3);
        assert_ne!(fresh.code, view.code);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_expired_code_is_refused_and_a_bad_label_is_told_apart() {
        let (dir, mut store) = store("expired");
        let mut pairing = Pairing::default();
        let view = pairing.mint(0);
        assert_eq!(
            pairing.claim(&view.code, PAIRING_CODE_TTL_MS, &mut store, "Phone", None, None),
            Err(ClaimError::Refused)
        );
        let view = pairing.mint(0);
        assert!(matches!(
            pairing.claim(&view.code, 1, &mut store, "  ", None, None),
            Err(ClaimError::Label(_))
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn minting_forgives_past_misses() {
        let mut pairing = Pairing::default();
        assert_eq!(pairing.miss("phone"), 0);
        assert_eq!(pairing.miss("phone"), 500);
        pairing.mint(0);
        assert_eq!(pairing.miss("phone"), 0);
    }
}
