//! One sample stream every display shares.
//!
//! Connections are bounded, per display and in total, and each one owns a
//! queue of whole frames. A display that stops reading loses its oldest
//! pending frames instead of growing.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::http::Shared;

pub const SAMPLE_EVENT: &str = "samples";
pub const SAMPLE_STREAM_PATH: &str = "/ws";
pub const PROTOCOL_VERSION: u64 = 1;
pub const DEFAULT_SAMPLE_INTERVAL_MS: u64 = 1_000;
pub const MAX_STREAM_KEYS: usize = 64;
pub const MAX_STREAM_KEY_BYTES: usize = 128;
pub const MAX_STREAM_CONNECTIONS: usize = 64;
pub const MAX_STREAM_CONNECTIONS_PER_DISPLAY: usize = 8;
pub const SSE_FRAME_QUEUE_LIMIT: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamRefusal {
    TooManyKeys,
    KeyTooLong,
    Busy,
}

impl StreamRefusal {
    pub fn message(&self) -> String {
        match self {
            Self::TooManyKeys => format!("A stream may ask for {MAX_STREAM_KEYS} keys at most."),
            Self::KeyTooLong => {
                format!("A stream key may be {MAX_STREAM_KEY_BYTES} bytes at most.")
            }
            Self::Busy => {
                "This host is already streaming to as many displays as it will.".to_owned()
            }
        }
    }
}

#[derive(Clone)]
struct Connection {
    id: u64,
    display: String,
    keys: Vec<String>,
    #[allow(dead_code)]
    lang: Option<String>,
    sender: mpsc::Sender<String>,
}

/// Every live stream, keyed by the id its handler unregisters with.
#[derive(Default)]
pub struct Streams {
    next: AtomicU64,
    connections: Mutex<Vec<Connection>>,
}

impl Streams {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check a stream's keys and, if the host has room, register it. The
    /// receiver carries whole SSE frames.
    pub fn register(
        &self,
        display: String,
        keys: Vec<String>,
        lang: Option<String>,
    ) -> Result<(u64, mpsc::Receiver<String>), StreamRefusal> {
        if keys.len() > MAX_STREAM_KEYS {
            return Err(StreamRefusal::TooManyKeys);
        }
        if keys.iter().any(|key| key.len() > MAX_STREAM_KEY_BYTES) {
            return Err(StreamRefusal::KeyTooLong);
        }
        let mut connections = match self.connections.lock() {
            Ok(connections) => connections,
            Err(_) => return Err(StreamRefusal::Busy),
        };
        if connections.len() >= MAX_STREAM_CONNECTIONS
            || connections
                .iter()
                .filter(|connection| connection.display == display)
                .count()
                >= MAX_STREAM_CONNECTIONS_PER_DISPLAY
        {
            return Err(StreamRefusal::Busy);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel(SSE_FRAME_QUEUE_LIMIT);
        connections.push(Connection {
            id,
            display,
            keys,
            lang,
            sender: sender.clone(),
        });
        drop(connections);
        // The connected comment is the first thing every stream receives.
        let _ = sender.try_send(": connected\n\n".to_owned());
        Ok((id, receiver))
    }

    pub fn unregister(&self, id: u64) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.retain(|connection| connection.id != id);
        }
    }

    pub fn live(&self) -> usize {
        self.connections
            .lock()
            .map(|connections| connections.len())
            .unwrap_or(0)
    }

    /// Deliver a frame to each connection of a display, dropping the frame for
    /// a connection whose queue is full. A full queue means a display that
    /// stopped reading, and the next frame is a whole snapshot anyway.
    fn publish(&self, display: &str, frame: &str) {
        let Ok(mut connections) = self.connections.lock() else {
            return;
        };
        for connection in connections.iter_mut() {
            if connection.display != display {
                continue;
            }
            if connection.sender.try_send(frame.to_owned()).is_err() {
                // Full or closed; keep the connection, the next tick retries.
            }
        }
    }
}

/// Every distinct display's ask for this tick.
fn groups(app: &crate::http::App) -> Vec<(String, Vec<String>)> {
    let Ok(connections) = app.streams.connections.lock() else {
        return Vec::new();
    };
    let mut by_display: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for connection in connections.iter() {
        let keys = by_display.entry(connection.display.clone()).or_default();
        for key in &connection.keys {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    by_display.into_iter().collect()
}

/// Start the one tick that samples every display and publishes its frame.
pub fn spawn(app: Shared) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(DEFAULT_SAMPLE_INTERVAL_MS));
        let mut fonts = yunari_render::Fonts::new();
        let mut documents: BTreeMap<String, yunari_model::document::ThemeDocument> = BTreeMap::new();
        loop {
            interval.tick().await;
            if app.streams.live() == 0 {
                continue;
            }
            let groups = groups(&app);
            if groups.is_empty() {
                continue;
            }
            let asks: Vec<yunari_providers::registry::Ask> = groups
                .iter()
                .map(|(display, keys)| {
                    yunari_providers::registry::Ask::new(display.clone(), keys.clone())
                })
                .collect();
            let now = jiff::Timestamp::now().as_millisecond();
            let cycle = app
                .registry
                .sample(&asks, u64::try_from(now).unwrap_or_default());
            let assignment = crate::assignment::Assignment::load(&app.config.settings_dir());
            for (display, keys) in groups {
                let readings = crate::frame::readings_for(&cycle, &keys);
                let theme = assignment.resolve(&app.library, &display, None);
                let frame_json = theme.and_then(|id| {
                    if !documents.contains_key(&id)
                        && let Ok(stored) = app.library.read(&id)
                    {
                        documents.insert(id.clone(), stored.document);
                    }
                    let document = documents.get(&id)?;
                    Some(crate::frame::frame(document, &readings, &mut fonts))
                });
                let samples: Vec<serde_json::Value> = keys
                    .iter()
                    .filter_map(|key| readings.get(key))
                    .map(crate::frame::sample_json)
                    .collect();
                let payload = serde_json::json!({
                    "version": PROTOCOL_VERSION,
                    "sentAt": jiff::Timestamp::now().as_millisecond(),
                    "samples": samples,
                    "frame": frame_json,
                });
                let data = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
                app.streams.publish(&display, &format!("event: {SAMPLE_EVENT}\ndata: {data}\n\n"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_is_limited_by_key_count_and_key_length() {
        let streams = Streams::new();
        let too_many: Vec<String> = (0..65).map(|index| format!("key.{index}")).collect();
        assert_eq!(
            streams.register("phone".to_owned(), too_many, None).unwrap_err(),
            StreamRefusal::TooManyKeys
        );
        assert_eq!(
            streams
                .register("phone".to_owned(), vec!["k".repeat(129)], None)
                .unwrap_err(),
            StreamRefusal::KeyTooLong
        );
        assert!(streams.register("phone".to_owned(), vec!["cpu.usage".to_owned()], None).is_ok());
    }

    #[test]
    fn one_display_gets_eight_streams_and_the_host_sixty_four() {
        let streams = Streams::new();
        for index in 0..MAX_STREAM_CONNECTIONS_PER_DISPLAY {
            streams
                .register("phone".to_owned(), vec!["cpu.usage".to_owned()], None)
                .unwrap_or_else(|_| panic!("stream {index} of one display"));
        }
        assert_eq!(
            streams
                .register("phone".to_owned(), vec!["cpu.usage".to_owned()], None)
                .unwrap_err(),
            StreamRefusal::Busy
        );
        for index in 1..=(MAX_STREAM_CONNECTIONS - MAX_STREAM_CONNECTIONS_PER_DISPLAY) {
            streams
                .register(format!("display-{index}"), vec!["cpu.usage".to_owned()], None)
                .unwrap_or_else(|_| panic!("stream {index} of the host"));
        }
        assert_eq!(
            streams
                .register("one-more".to_owned(), vec!["cpu.usage".to_owned()], None)
                .unwrap_err(),
            StreamRefusal::Busy
        );
    }

    #[test]
    fn a_full_queue_drops_the_frame_rather_than_growing() {
        let streams = Streams::new();
        let (_id, mut receiver) = streams.register("phone".to_owned(), vec![], None).unwrap();
        // The connected comment is already queued; fill the rest.
        for index in 0..SSE_FRAME_QUEUE_LIMIT {
            streams.publish("phone", &format!("frame {index}\n\n"));
        }
        let mut frames = 0;
        while receiver.try_recv().is_ok() {
            frames += 1;
        }
        assert_eq!(frames, SSE_FRAME_QUEUE_LIMIT);
    }

    #[test]
    fn unregistering_frees_a_slot() {
        let streams = Streams::new();
        let (id, _receiver) = streams.register("phone".to_owned(), vec![], None).unwrap();
        assert_eq!(streams.live(), 1);
        streams.unregister(id);
        assert_eq!(streams.live(), 0);
    }
}
