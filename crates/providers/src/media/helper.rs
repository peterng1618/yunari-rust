//! One line a second out of the PowerShell helper, as typed events.
//!
//! The helper is a process, not a protocol: a line that is not one of the six
//! shapes is ignored rather than trusted, and a line larger than the buffer is
//! dropped rather than buffered without bound.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use super::artwork;
use super::script;

pub const POWERSHELL: &str = "powershell.exe";
/// A helper that cannot be launched keeps its reason with the reading.
pub const START_FAILURE: &str = "The media helper could not be started: ";

/// Ends the helper and waits for it to exit.
pub type Stop = Box<dyn FnOnce() + Send>;

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

/// `-EncodedCommand` takes the script as base64 of its UTF-16LE bytes, which is
/// how a script with quotes and braces travels without a shell in between.
pub fn encode(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Starts the helper and hands back its events and its stop.
pub fn spawn(artwork: bool) -> Result<(Receiver<Event>, Stop), String> {
    let encoded = encode(&script::script(artwork));
    let mut child = Command::new(POWERSHELL)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{START_FAILURE}{error}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err(format!("{START_FAILURE}its output could not be read."));
    };
    let (sender, events) = mpsc::channel();
    thread::spawn(move || read(stdout, sender));
    let child = Arc::new(Mutex::new(child));
    let stopper = Arc::clone(&child);
    Ok((
        events,
        Box::new(move || {
            let mut child = stopper
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }),
    ))
}

fn read(stdout: std::process::ChildStdout, sender: mpsc::Sender<Event>) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        match read_line(&mut reader, &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let Some(event) = parse(text.trim()) else {
            continue;
        };
        if sender.send(event).is_err() {
            return;
        }
    }
}

/// Reads one line, keeping at most [`script::LINE_CAP`] bytes of it. A longer
/// line is drained and dropped, so a wedged script cannot grow this buffer.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<usize> {
    line.clear();
    let mut total = 0;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(total);
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let end = newline.unwrap_or(available.len());
        if line.len() < script::LINE_CAP {
            let room = script::LINE_CAP - line.len();
            line.extend_from_slice(&available[..end.min(room)]);
        }
        let consumed = end + usize::from(newline.is_some());
        reader.consume(consumed);
        total += consumed;
        if newline.is_some() {
            return Ok(total);
        }
    }
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

    #[test]
    fn an_encoded_script_is_the_script() {
        use base64::Engine;
        let script = "$x = 'y'";
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encode(script))
            .expect("base64");
        let units: Vec<u16> = decoded
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), script);
    }

    #[test]
    fn a_line_longer_than_the_cap_is_dropped_not_buffered() {
        let mut input = Vec::new();
        input.extend(std::iter::repeat_n(b'x', script::LINE_CAP + 64));
        input.push(b'\n');
        input.extend_from_slice(b"{\"kind\":\"none\"}\n");
        let mut reader = std::io::BufReader::new(input.as_slice());
        let mut line = Vec::new();
        assert!(read_line(&mut reader, &mut line).unwrap() > script::LINE_CAP);
        assert_eq!(line.len(), script::LINE_CAP);
        read_line(&mut reader, &mut line).unwrap();
        assert_eq!(line, b"{\"kind\":\"none\"}");
    }
}
