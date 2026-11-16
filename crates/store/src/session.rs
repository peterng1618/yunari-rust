//! The store session this host holds, beside the other settings.
//!
//! The file is the protected bytes, not JSON: the token inside it is what a
//! request from here would carry, so a copy of the file is not a copy of the
//! account. The browser never reads it, and neither does a display.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::secret;

pub const FILE: &str = "store-session.bin";

/// One sign-in, as the Worker reported it and this host holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The Worker's session token. Never log it.
    pub token: String,
    /// Better Auth's `user.id`.
    pub user_id: String,
    pub email: String,
    /// An ISO instant.
    pub obtained_at: String,
}

impl Session {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "token": self.token,
            "userId": self.user_id,
            "email": self.email,
            "obtainedAt": self.obtained_at,
        })
    }

    /// Four usable values, or nothing: a token without the account it acts as
    /// cannot be reported honestly, so a bad record reads as no session.
    fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let session = Self {
            token: text("token")?,
            user_id: text("userId")?,
            email: text("email")?,
            obtained_at: text("obtainedAt")?,
        };
        session.obtained_at.parse::<jiff::Timestamp>().ok()?;
        Some(session)
    }
}

pub struct SessionStore {
    directory: PathBuf,
}

impl SessionStore {
    pub fn at(directory: impl AsRef<Path>) -> Self {
        Self {
            directory: directory.as_ref().to_path_buf(),
        }
    }

    fn file(&self) -> PathBuf {
        self.directory.join(FILE)
    }

    /// The held session, or nothing when there is none this build can use.
    pub fn read(&self) -> Option<Session> {
        let sealed = std::fs::read(self.file()).ok()?;
        let plain = secret::unprotect(&sealed).ok()?;
        let value: Value = serde_json::from_slice(&plain).ok()?;
        Session::from_json(&value)
    }

    /// Holds `session`, replacing whatever was there. A write that fails leaves
    /// the previous session untouched.
    pub fn write(&self, session: &Session) -> Result<(), String> {
        let plain = serde_json::to_vec(&session.to_json()).map_err(|error| error.to_string())?;
        let sealed = secret::protect(&plain)?;
        std::fs::create_dir_all(&self.directory).map_err(|error| error.to_string())?;
        let temp = self.directory.join(format!("{FILE}.new"));
        std::fs::write(&temp, &sealed).map_err(|error| error.to_string())?;
        std::fs::rename(&temp, self.file()).map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            error.to_string()
        })
    }

    /// Forgets the session, on sign-out or a refused token.
    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(self.file()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunari-store-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session() -> Session {
        Session {
            token: "token-value".to_owned(),
            user_id: "user-1".to_owned(),
            email: "person@example.com".to_owned(),
            obtained_at: "2026-11-16T10:00:00Z".to_owned(),
        }
    }

    #[test]
    fn a_session_survives_a_write_and_clears() {
        let dir = directory("round-trip");
        let store = SessionStore::at(&dir);
        assert_eq!(store.read(), None);
        store.write(&session()).expect("a written session");
        assert_eq!(store.read(), Some(session()));
        store.clear().expect("a cleared session");
        assert_eq!(store.read(), None);
        // Clearing twice is not an error.
        store.clear().expect("an already clear session");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_file_does_not_carry_the_token_in_plain_text() {
        let dir = directory("sealed");
        let store = SessionStore::at(&dir);
        store.write(&session()).expect("a written session");
        let bytes = std::fs::read(dir.join(FILE)).expect("the session file");
        let found = bytes
            .windows(session().token.len())
            .any(|window| window == session().token.as_bytes());
        assert!(!found, "the token was readable in the file");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_this_build_cannot_use_reads_as_no_session() {
        let dir = directory("corrupt");
        let store = SessionStore::at(&dir);
        std::fs::write(dir.join(FILE), b"not the protected bytes").unwrap();
        assert_eq!(store.read(), None);
        // A protected record missing an account is no session either.
        let sealed = secret::protect(br#"{"token":"t"}"#).expect("protect");
        std::fs::write(dir.join(FILE), sealed).unwrap();
        assert_eq!(store.read(), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
