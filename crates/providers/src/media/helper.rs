//! One line a second out of the PowerShell helper, as typed events.
//!
//! The helper is a process, not a protocol: a line that is not one of the six
//! shapes is ignored rather than trusted, and a line larger than the buffer is
//! dropped rather than buffered without bound.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

use super::artwork;
use super::script;
use crate::winps;

/// A helper that cannot be launched keeps its reason with the reading.
pub const START_FAILURE: &str = "The media helper could not be started: ";

/// Ends the helper and waits for it to exit.
pub type Stop = winps::Stop;

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub app: String,
    pub session_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `playing`, `paused` or `stopped`.
    pub state: String,
    /// The session Windows calls current, which is not always the playing one.
    pub current: bool,
    pub position_s: f64,
    pub duration_s: f64,
}

/// Endpoint volume and mute. Neither value means no output endpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Audio {
    pub volume: Option<f64>,
    pub muted: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// The picture is larger than one helper line can carry.
    Over,
    /// The helper gave up after its retries.
    Read,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Session(Session),
    Audio(Option<Audio>),
    /// No session is open, as Windows says outright.
    NoSessions,
    /// The session list could not be read this poll.
    Error,
    Artwork {
        session_id: String,
        ext: String,
        bytes: Vec<u8>,
    },
    ArtworkMiss {
        session_id: String,
        miss: Miss,
    },
}

/// One line of the helper's output, when it is one this provider reads.
pub fn parse(line: &str) -> Option<Event> {
    let payload: Value = serde_json::from_str(line).ok()?;
    let text = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match text("kind").as_str() {
        "session" => Some(Event::Session(Session {
            app: text("app"),
            session_id: text("sessionId"),
            title: text("title"),
            artist: text("artist"),
            album: text("album"),
            state: text("state"),
            current: payload
                .get("current")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            position_s: number(&payload, "positionS").unwrap_or_default(),
            duration_s: number(&payload, "durationS").unwrap_or_default(),
        })),
        "audio" => {
            let volume = number(&payload, "volume");
            let muted = payload.get("muted").and_then(Value::as_bool);
            let audio = (volume.is_some() && muted.is_some()).then_some(Audio { volume, muted });
            Some(Event::Audio(audio))
        }
        "none" => Some(Event::NoSessions),
        "error" => Some(Event::Error),
        "artwork" => {
            let session_id = text("sessionId");
            let ext = text("ext");
            let bytes = artwork::decode_base64(&text("base64"))?;
            Some(Event::Artwork {
                session_id,
                ext,
                bytes,
            })
        }
        "artwork-miss" => Some(Event::ArtworkMiss {
            session_id: text("sessionId"),
            miss: if text("reason") == "over" {
                Miss::Over
            } else {
                Miss::Read
            },
        }),
        _ => None,
    }
}

fn number(record: &Value, field: &str) -> Option<f64> {
    record
        .get(field)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

/// Starts the helper and hands back its events and its stop.
pub fn spawn(artwork: bool) -> Result<(Receiver<Event>, Stop), String> {
    let (lines, stop) = winps::spawn(&script::script(artwork))
        .map_err(|error| format!("{START_FAILURE}{error}"))?;
    let (sender, events) = mpsc::channel();
    // The script's lines are typed here, once, so the provider never sees JSON.
    thread::spawn(move || {
        for line in lines {
            let Some(event) = parse(line.trim()) else {
                continue;
            };
            if sender.send(event).is_err() {
                return;
            }
        }
    });
    Ok((events, stop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_line_is_every_field_the_readings_need() {
        let line = r#"{"kind":"session","app":"Spotify","sessionId":"spotify:1","title":"Title","artist":"Artist","album":"Album","state":"playing","current":true,"positionS":12.5,"durationS":200,"hasArtwork":true}"#;
        let Some(Event::Session(session)) = parse(line) else {
            panic!("a session line");
        };
        assert_eq!(session.app, "Spotify");
        assert_eq!(session.session_id, "spotify:1");
        assert_eq!(session.title, "Title");
        assert_eq!(session.artist, "Artist");
        assert_eq!(session.album, "Album");
        assert_eq!(session.state, "playing");
        assert!(session.current);
        assert_eq!(session.position_s, 12.5);
        assert_eq!(session.duration_s, 200.0);
    }

    #[test]
    fn audio_with_no_endpoint_is_not_zero_volume_unmuted() {
        let Some(Event::Audio(Some(audio))) = parse(r#"{"kind":"audio","volume":42,"muted":true}"#)
        else {
            panic!("an audio line");
        };
        assert_eq!(audio.volume, Some(42.0));
        assert_eq!(audio.muted, Some(true));
        let Some(Event::Audio(None)) = parse(r#"{"kind":"audio"}"#) else {
            panic!("an audio line without an endpoint");
        };
    }

    #[test]
    fn the_plain_shapes_map_to_their_own_events() {
        assert_eq!(parse(r#"{"kind":"none"}"#), Some(Event::NoSessions));
        assert_eq!(parse(r#"{"kind":"error"}"#), Some(Event::Error));
        assert_eq!(parse("not json"), None);
        assert_eq!(parse(r#"{"kind":"odd"}"#), None);
        assert_eq!(parse("[1,2,3]"), None);
    }

    #[test]
    fn artwork_bytes_are_decoded_and_a_miss_carries_its_reason() {
        use base64::Engine;
        let bytes = [0x89, 0x50, 0x4e, 0x47, 1, 2, 3];
        let line = format!(
            r#"{{"kind":"artwork","sessionId":"s","sha256":"x","ext":"png","base64":"{}"}}"#,
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        let Some(Event::Artwork {
            session_id,
            ext,
            bytes: decoded,
        }) = parse(&line)
        else {
            panic!("an artwork line");
        };
        assert_eq!(session_id, "s");
        assert_eq!(ext, "png");
        assert_eq!(decoded, bytes);
        assert!(parse(r#"{"kind":"artwork","sessionId":"s","base64":"!!"}"#).is_none());

        assert_eq!(
            parse(r#"{"kind":"artwork-miss","sessionId":"s","reason":"over"}"#),
            Some(Event::ArtworkMiss {
                session_id: "s".to_owned(),
                miss: Miss::Over
            })
        );
        assert_eq!(
            parse(r#"{"kind":"artwork-miss","sessionId":"s","reason":"exhausted"}"#),
            Some(Event::ArtworkMiss {
                session_id: "s".to_owned(),
                miss: Miss::Read
            })
        );
    }
}
