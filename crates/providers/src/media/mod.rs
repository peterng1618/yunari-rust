//! What Windows says is playing, read through the embedded helper.
//!
//! The helper is one PowerShell process started only while a display asks for a
//! media or audio key, and stopped once none has for the grace period. `sample`
//! drains what it has said since the last poll, so a reading is never older
//! than the poll that asked for it.
//!
//! A session is keyed by app, title, artist and album: a track change is a new
//! session, and the previous one is dropped once it stops being reported, so a
//! frozen position is never shown as a reading.

pub mod artwork;
pub mod helper;
pub mod script;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;
use crate::vocabulary;
use helper::{Audio, Event, Miss, Session, Stop};

pub const MEDIA_KEYS: &[&str] = &[
    "media.title",
    "media.artist",
    "media.album",
    "media.artwork",
    "media.state",
    "media.position",
    "media.duration",
    "media.app",
];
pub const AUDIO_KEYS: &[&str] = &["audio.volume", "audio.muted"];

/// Media first, then the independent audio readings.
pub const KEYS: &[&str] = &[
    "media.title",
    "media.artist",
    "media.album",
    "media.artwork",
    "media.state",
    "media.position",
    "media.duration",
    "media.app",
    "audio.volume",
    "audio.muted",
];

pub const NO_SESSION_MESSAGE: &str = "No media session is playing.";
pub const NO_AUDIO_ENDPOINT_MESSAGE: &str = "The Host reports no output volume.";
pub const WAITING_MESSAGE: &str = "Waiting for the media helper.";
pub const WAITING_ARTWORK_MESSAGE: &str = "Waiting for this track's artwork.";
pub const ARTWORK_TOO_LARGE_MESSAGE: &str =
    "This track's artwork is too large for the media helper to read.";
pub const ARTWORK_UNREADABLE_MESSAGE: &str =
    "This track's artwork could not be read from the media session.";
pub const MEDIA_READ_FAILURE: &str = "The media session could not be read.";
pub const HELPER_STOPPED_MESSAGE: &str = "The media helper stopped reporting.";
const NO_KEY_MESSAGE: &str = "This host does not read that media key.";

/// No display asked for a media or audio key for this long, so the helper stops.
const DEFAULT_GRACE_MS: u64 = 30_000;
/// A failed helper is not retried before this, so a broken one cannot spin.
const DEFAULT_RETRY_MS: u64 = 60_000;
/// A session line not refreshed for three polls is gone.
const SESSION_TTL_MS: u64 = 3_000;
/// One line a second is the contract; a minute without one is a wedged helper.
const LIVENESS_MS: u64 = 60_000;
/// Held artwork URLs: the handful of tracks a display switches between.
const ARTWORK_TRACKS_CAP: usize = 32;

/// Launches one helper, given whether its script should read artwork.
pub type Spawn = Arc<dyn Fn(bool) -> Result<(Receiver<Event>, Stop), String> + Send + Sync>;

struct Live {
    session: Session,
    seen_at_ms: u64,
}

struct Helper {
    events: Receiver<Event>,
    stop: Option<Stop>,
    /// The script this helper was spawned with; a mode change is a restart.
    artwork: bool,
}

#[derive(Default)]
struct State {
    sessions: Vec<Live>,
    /// The session id the system calls current, from the latest poll.
    current_id: Option<String>,
    audio: Option<Audio>,
    /// A helper that has not answered yet cannot be told from one with no
    /// output endpoint; the first audio line settles it.
    audio_seen: bool,
    /// Redacted; may reach a browser through `health()` and a reading.
    failure: Option<String>,
    helper: Option<Helper>,
    last_demand_ms: Option<u64>,
    last_artwork_demand_ms: Option<u64>,
    retry_at_ms: u64,
    last_line_ms: u64,
    track_key: Option<String>,
    artwork_url: Option<String>,
    /// Held artwork and definitive misses, by track identity, newest last.
    artwork_by_track: Vec<(String, String)>,
    misses: Vec<(String, Miss)>,
}

pub struct MediaProvider {
    cache: artwork::Artwork,
    spawn: Spawn,
    grace_ms: u64,
    retry_ms: u64,
    state: Mutex<State>,
}

impl MediaProvider {
    pub fn new(artwork_dir: impl Into<PathBuf>) -> Self {
        Self::with_spawn(
            artwork_dir,
            Arc::new(helper::spawn),
            DEFAULT_GRACE_MS,
            DEFAULT_RETRY_MS,
        )
    }

    pub fn with_spawn(
        artwork_dir: impl Into<PathBuf>,
        spawn: Spawn,
        grace_ms: u64,
        retry_ms: u64,
    ) -> Self {
        Self {
            cache: artwork::Artwork::new(artwork_dir),
            spawn,
            grace_ms,
            retry_ms,
            state: Mutex::new(State::default()),
        }
    }

    /// Ends the helper; the host calls this when it shuts down, so no
    /// `powershell.exe` is left polling. A later sample starts it again.
    pub fn stop(&self) {
        let mut state = self.state.lock().expect("the media state");
        teardown(&mut state);
    }

    /// Starts, restarts or stops the helper so its script matches demand.
    fn reconcile(&self, state: &mut State, now_ms: u64) {
        let artwork = state
            .last_artwork_demand_ms
            .is_some_and(|last| now_ms.saturating_sub(last) <= self.grace_ms);
        if let Some(helper) = state.helper.as_ref() {
            if helper.artwork == artwork {
                return;
            }
            // The script is fixed at spawn, so a change in demand is a restart.
            // Readings survive it: only a teardown clears them.
            stop_helper(state);
        }
        self.start(state, now_ms, artwork);
    }

    fn start(&self, state: &mut State, now_ms: u64, artwork: bool) {
        if state.helper.is_some() || now_ms < state.retry_at_ms {
            return;
        }
        match (self.spawn)(artwork) {
            Ok((events, stop)) => {
                state.helper = Some(Helper {
                    events,
                    stop: Some(stop),
                    artwork,
                });
                state.last_line_ms = now_ms;
            }
            Err(reason) => fail(state, now_ms, reason, self.retry_ms),
        }
    }

    /// Drains what the helper has reported since the last poll.
    fn collect(&self, state: &mut State, now_ms: u64) {
        let Some(helper) = state.helper.take() else {
            return;
        };
        let mut events = Vec::new();
        let mut disconnected = false;
        loop {
            match helper.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        state.helper = Some(helper);

        for event in events {
            ingest(&self.cache, state, now_ms, event);
        }

        if disconnected {
            // A deliberate stop takes the helper out before stopping it, so a
            // disconnect seen here is the helper ending on its own.
            stop_helper(state);
            fail(
                state,
                now_ms,
                HELPER_STOPPED_MESSAGE.to_owned(),
                self.retry_ms,
            );
            return;
        }
        if state.helper.is_some() && now_ms.saturating_sub(state.last_line_ms) > LIVENESS_MS {
            stop_helper(state);
            fail(
                state,
                now_ms,
                HELPER_STOPPED_MESSAGE.to_owned(),
                self.retry_ms,
            );
        }
    }
}

impl Provider for MediaProvider {
    fn id(&self) -> &str {
        "media"
    }

    fn label(&self) -> &str {
        "Media"
    }

    fn describe(&self) -> Vec<Descriptor> {
        KEYS.iter()
            .map(|key| {
                Descriptor::new(*key, vocabulary::describe(key).and_then(|entry| entry.unit))
            })
            .collect()
    }

    fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String> {
        let wanted: Vec<&String> = keys
            .iter()
            .filter(|key| KEYS.contains(&key.as_str()))
            .collect();
        let mut state = self.state.lock().expect("the media state");

        // A gap this long is a new demand window: tear down before anything
        // else, so a poll that asks for nothing still ends the helper.
        if state
            .last_demand_ms
            .is_none_or(|last| now_ms.saturating_sub(last) > self.grace_ms)
        {
            teardown(&mut state);
        }
        if wanted.is_empty() {
            return Ok(Vec::new());
        }

        state.last_demand_ms = Some(now_ms);
        if wanted.iter().any(|key| key.as_str() == "media.artwork") {
            state.last_artwork_demand_ms = Some(now_ms);
        }
        self.reconcile(&mut state, now_ms);
        self.collect(&mut state, now_ms);

        let session = selected(&mut state, now_ms);
        Ok(wanted
            .into_iter()
            .map(|key| reading_of(&state, session.as_ref(), key))
            .collect())
    }

    fn health(&self) -> Health {
        match self.state.lock().expect("the media state").failure.clone() {
            Some(reason) => Health::Unavailable(reason),
            None => Health::Ok,
        }
    }
}

/// One line from the helper, applied to the state it describes.
fn ingest(cache: &artwork::Artwork, state: &mut State, now_ms: u64, event: Event) {
    state.last_line_ms = now_ms;
    match event {
        Event::Session(session) => {
            let id = session.session_id.clone();
            let current = session.current;
            match state
                .sessions
                .iter_mut()
                .find(|live| live.session.session_id == id)
            {
                Some(live) => {
                    live.session = session;
                    live.seen_at_ms = now_ms;
                }
                None => state.sessions.push(Live {
                    session,
                    seen_at_ms: now_ms,
                }),
            }
            if current {
                state.current_id = Some(id);
            }
            state.failure = None;
            selected(state, now_ms);
        }
        Event::Audio(audio) => {
            state.audio = audio;
            state.audio_seen = true;
        }
        Event::NoSessions => {
            state.sessions.clear();
            state.failure = None;
            selected(state, now_ms);
        }
        Event::Error => state.failure = Some(MEDIA_READ_FAILURE.to_owned()),
        Event::Artwork {
            session_id,
            ext,
            bytes,
        } => {
            // Artwork is held for the session that emitted it, not only for the
            // one selected now: another track's picture must already be known
            // when a selection change makes it current.
            let Some(key) = state
                .sessions
                .iter()
                .find(|live| live.session.session_id == session_id)
                .map(|live| track_key(&live.session))
            else {
                return;
            };
            let Ok(url) = cache.put(&bytes, &ext) else {
                return;
            };
            remember(&mut state.artwork_by_track, key.clone(), url.clone());
            // A picture arrived for a track the helper had called unreadable.
            state.misses.retain(|(missed, _)| *missed != key);
            if state.track_key.as_deref() == Some(key.as_str()) {
                state.artwork_url = Some(url);
            }
        }
        Event::ArtworkMiss { session_id, miss } => {
            let Some(key) = state
                .sessions
                .iter()
                .find(|live| live.session.session_id == session_id)
                .map(|live| track_key(&live.session))
            else {
                return;
            };
            remember_miss(&mut state.misses, key, miss);
        }
    }
}

fn remember(held: &mut Vec<(String, String)>, key: String, url: String) {
    held.retain(|(held_key, _)| *held_key != key);
    held.push((key, url));
    if held.len() > ARTWORK_TRACKS_CAP {
        held.remove(0);
    }
}

fn remember_miss(misses: &mut Vec<(String, Miss)>, key: String, miss: Miss) {
    misses.retain(|(held_key, _)| *held_key != key);
    misses.push((key, miss));
    if misses.len() > ARTWORK_TRACKS_CAP {
        misses.remove(0);
    }
}

/// The session every media key answers from: the most recently seen playing
/// one, else the most recently seen current one, else nothing.
fn selected(state: &mut State, now_ms: u64) -> Option<Session> {
    state
        .sessions
        .retain(|live| now_ms.saturating_sub(live.seen_at_ms) <= SESSION_TTL_MS);

    let mut playing: Option<&Live> = None;
    let mut current: Option<&Live> = None;
    for live in &state.sessions {
        if live.session.state == "playing"
            && playing.is_none_or(|best| live.seen_at_ms >= best.seen_at_ms)
        {
            playing = Some(live);
        }
        if state.current_id.as_deref() == Some(live.session.session_id.as_str())
            && current.is_none_or(|best| live.seen_at_ms >= best.seen_at_ms)
        {
            current = Some(live);
        }
    }

    let chosen = playing.or(current).map(|live| live.session.clone());
    let key = chosen.as_ref().map(track_key);
    if key != state.track_key {
        state.track_key = key.clone();
        // A picture already emitted for this exact track is reused; only a
        // track nothing was ever read for is a wait.
        state.artwork_url = key.and_then(|key| {
            state
                .artwork_by_track
                .iter()
                .find(|(held_key, _)| *held_key == key)
                .map(|(_, url)| url.clone())
        });
    }
    chosen
}

fn track_key(session: &Session) -> String {
    format!(
        "{}\u{0}{}\u{0}{}\u{0}{}",
        session.session_id, session.title, session.artist, session.album
    )
}

/// One key's reading, from the session selected for this poll.
fn reading_of(state: &State, session: Option<&Session>, key: &str) -> Reading {
    let unit = || {
        vocabulary::describe(key)
            .and_then(|entry| entry.unit)
            .unwrap_or_default()
    };

    if AUDIO_KEYS.contains(&key) {
        if !state.audio_seen {
            return Reading::missing(key, WAITING_MESSAGE);
        }
        let Some(audio) = state.audio else {
            return Reading::missing(key, NO_AUDIO_ENDPOINT_MESSAGE);
        };
        if key == "audio.volume" {
            return Reading::number(key, audio.volume.unwrap_or_default(), unit());
        }
        // The source's renderer shows a boolean as these two words.
        return Reading::text(
            key,
            if audio.muted.unwrap_or(false) {
                "on"
            } else {
                "off"
            },
        );
    }

    let Some(session) = session else {
        return match &state.failure {
            Some(reason) => Reading::failed(key, reason.clone()),
            None => Reading::missing(key, NO_SESSION_MESSAGE),
        };
    };

    match key {
        "media.title" => Reading::text(key, session.title.clone()),
        "media.artist" => Reading::text(key, session.artist.clone()),
        "media.album" => Reading::text(key, session.album.clone()),
        "media.state" => Reading::text(key, session.state.clone()),
        "media.app" => Reading::text(key, session.app.clone()),
        "media.position" => Reading::number(key, session.position_s, unit()),
        "media.duration" => Reading::number(key, session.duration_s, unit()),
        "media.artwork" => {
            if let Some(url) = &state.artwork_url {
                return Reading::text(key, url.clone());
            }
            // A definitive miss is this track's answer, not a wait: the helper
            // will not read it again.
            let miss = state.track_key.as_ref().and_then(|track| {
                state
                    .misses
                    .iter()
                    .find(|(key, _)| key == track)
                    .map(|(_, miss)| *miss)
            });
            match miss {
                None => Reading::missing(key, WAITING_ARTWORK_MESSAGE),
                Some(Miss::Over) => Reading::missing(key, ARTWORK_TOO_LARGE_MESSAGE),
                Some(Miss::Read) => Reading::missing(key, ARTWORK_UNREADABLE_MESSAGE),
            }
        }
        _ => Reading::missing(key, NO_KEY_MESSAGE),
    }
}

fn stop_helper(state: &mut State) {
    if let Some(helper) = state.helper.take()
        && let Some(stop) = helper.stop
    {
        stop();
    }
}

/// A helper that ended or failed: its readings are not trusted again until a
/// new one reports.
fn fail(state: &mut State, now_ms: u64, reason: String, retry_ms: u64) {
    clear_readings(state);
    state.failure = Some(reason);
    state.retry_at_ms = now_ms.saturating_add(retry_ms);
}

fn clear_readings(state: &mut State) {
    state.sessions.clear();
    state.current_id = None;
    state.audio = None;
    state.audio_seen = false;
    state.track_key = None;
    state.artwork_url = None;
    state.artwork_by_track.clear();
    state.misses.clear();
}

fn teardown(state: &mut State) {
    stop_helper(state);
    clear_readings(state);
    state.failure = None;
    state.retry_at_ms = 0;
    state.last_demand_ms = None;
    state.last_artwork_demand_ms = None;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use super::*;
    use crate::sample::Status;

    struct Fixture {
        provider: MediaProvider,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        directory: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn fixture(name: &str, script: Vec<Vec<Event>>) -> Fixture {
        let directory = std::env::temp_dir().join(format!(
            "yunari-media-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let queued = Arc::new(Mutex::new(script));
        let counted = Arc::clone(&starts);
        let counted_stops = Arc::clone(&stops);
        let spawn: Spawn = Arc::new(move |_artwork| {
            counted.fetch_add(1, Ordering::SeqCst);
            let events = {
                let mut script = queued.lock().unwrap();
                if script.is_empty() {
                    Vec::new()
                } else {
                    script.remove(0)
                }
            };
            let (sender, receiver) = mpsc::channel();
            for event in events {
                let _ = sender.send(event);
            }
            let stops = Arc::clone(&counted_stops);
            Ok((
                receiver,
                Box::new(move || {
                    // The sender stays alive until the helper stops, as a real
                    // one's process does.
                    drop(sender);
                    stops.fetch_add(1, Ordering::SeqCst);
                }) as Stop,
            ))
        });
        Fixture {
            provider: MediaProvider::with_spawn(&directory, spawn, 1_000, 0),
            starts,
            stops,
            directory,
        }
    }

    fn session_line(state: &str, current: bool) -> Event {
        Event::Session(Session {
            app: "Spotify".to_owned(),
            session_id: "spotify:1".to_owned(),
            title: "Title".to_owned(),
            artist: "Artist".to_owned(),
            album: "Album".to_owned(),
            state: state.to_owned(),
            current,
            position_s: 12.5,
            duration_s: 200.0,
        })
    }

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn reading<'a>(readings: &'a [Reading], key: &str) -> &'a Reading {
        readings
            .iter()
            .find(|reading| reading.semantic_key == key)
            .expect("a reading for the key")
    }

    #[test]
    fn a_helper_line_maps_to_every_media_reading() {
        let fixture = fixture("readings", vec![vec![session_line("playing", true)]]);
        let readings = fixture
            .provider
            .sample(&keys(MEDIA_KEYS), 1_000)
            .expect("a cycle");
        assert_eq!(
            reading(&readings, "media.title").text.as_deref(),
            Some("Title")
        );
        assert_eq!(
            reading(&readings, "media.artist").text.as_deref(),
            Some("Artist")
        );
        assert_eq!(
            reading(&readings, "media.album").text.as_deref(),
            Some("Album")
        );
        assert_eq!(
            reading(&readings, "media.state").text.as_deref(),
            Some("playing")
        );
        assert_eq!(
            reading(&readings, "media.app").text.as_deref(),
            Some("Spotify")
        );
        assert_eq!(reading(&readings, "media.position").value, Some(12.5));
        assert_eq!(
            reading(&readings, "media.position").unit.as_deref(),
            Some("s")
        );
        assert_eq!(reading(&readings, "media.duration").value, Some(200.0));
        // Nothing has read this track's picture yet.
        assert_eq!(
            reading(&readings, "media.artwork").message.as_deref(),
            Some(WAITING_ARTWORK_MESSAGE)
        );
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn no_session_playing_is_missing_with_the_reason() {
        let fixture = fixture("none", vec![vec![Event::NoSessions]]);
        let readings = fixture
            .provider
            .sample(&keys(&["media.title"]), 1_000)
            .expect("a cycle");
        assert_eq!(readings[0].status, Status::Missing);
        assert_eq!(readings[0].value, None);
        assert_eq!(readings[0].message.as_deref(), Some(NO_SESSION_MESSAGE));
    }

    #[test]
    fn a_helper_that_cannot_start_says_why_and_is_not_retried_at_once() {
        let spawn: Spawn = Arc::new(|_artwork| Err("PowerShell is not installed.".to_owned()));
        let provider = MediaProvider::with_spawn(std::env::temp_dir(), spawn, 30_000, 60_000);
        let readings = provider
            .sample(&keys(&["media.title"]), 1_000)
            .expect("a cycle");
        assert_eq!(readings[0].status, Status::Error);
        assert_eq!(
            readings[0].message.as_deref(),
            Some("PowerShell is not installed.")
        );
        assert!(matches!(provider.health(), Health::Unavailable(_)));
        // Not before the retry window.
        let readings = provider
            .sample(&keys(&["media.title"]), 2_000)
            .expect("a cycle");
        assert_eq!(
            readings[0].message.as_deref(),
            Some("PowerShell is not installed.")
        );
    }

    #[test]
    fn audio_reads_its_endpoint_or_says_there_is_none() {
        let audible = fixture(
            "audio",
            vec![vec![
                Event::Audio(Some(Audio {
                    volume: Some(42.0),
                    muted: Some(false),
                })),
                session_line("paused", false),
            ]],
        );
        let readings = audible
            .provider
            .sample(&keys(AUDIO_KEYS), 1_000)
            .expect("a cycle");
        assert_eq!(reading(&readings, "audio.volume").value, Some(42.0));
        assert_eq!(
            reading(&readings, "audio.volume").unit.as_deref(),
            Some("%")
        );
        assert_eq!(
            reading(&readings, "audio.muted").text.as_deref(),
            Some("off")
        );

        let silent = fixture("no-endpoint", vec![vec![Event::Audio(None)]]);
        let readings = silent
            .provider
            .sample(&keys(AUDIO_KEYS), 1_000)
            .expect("a cycle");
        assert_eq!(
            reading(&readings, "audio.volume").message.as_deref(),
            Some(NO_AUDIO_ENDPOINT_MESSAGE)
        );
        assert_eq!(reading(&readings, "audio.muted").status, Status::Missing);
    }

    #[test]
    fn artwork_bytes_arrive_as_the_track_picture() {
        const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        let fixture = fixture(
            "artwork",
            vec![vec![
                session_line("playing", true),
                Event::Artwork {
                    session_id: "spotify:1".to_owned(),
                    ext: "png".to_owned(),
                    bytes: PNG.to_vec(),
                },
            ]],
        );
        let readings = fixture
            .provider
            .sample(&keys(&["media.artwork"]), 1_000)
            .expect("a cycle");
        let name = format!("{}.png", artwork::sha256_hex(PNG));
        assert_eq!(readings[0].text, Some(format!("/api/media/artwork/{name}")));
        // The file the reading names is on disk and is the picture.
        assert!(fixture.directory.join(&name).exists());
    }

    #[test]
    fn a_definitive_miss_is_the_tracks_answer_not_a_wait() {
        let fixture = fixture(
            "miss",
            vec![vec![
                session_line("playing", true),
                Event::ArtworkMiss {
                    session_id: "spotify:1".to_owned(),
                    miss: Miss::Over,
                },
            ]],
        );
        let readings = fixture
            .provider
            .sample(&keys(&["media.artwork"]), 1_000)
            .expect("a cycle");
        assert_eq!(
            readings[0].message.as_deref(),
            Some(ARTWORK_TOO_LARGE_MESSAGE)
        );
    }

    #[test]
    fn a_lapsed_demand_window_stops_the_helper_and_a_new_ask_starts_one() {
        let fixture = fixture(
            "grace",
            vec![vec![session_line("playing", true)], Vec::new()],
        );
        fixture
            .provider
            .sample(&keys(&["media.title"]), 1_000)
            .expect("a cycle");
        assert_eq!(fixture.stops.load(Ordering::SeqCst), 0);
        // Nothing asked for two seconds: the helper stops and the readings go.
        fixture.provider.sample(&[], 3_500).expect("a cycle");
        assert_eq!(fixture.stops.load(Ordering::SeqCst), 1);
        let readings = fixture
            .provider
            .sample(&keys(&["media.title"]), 3_600)
            .expect("a cycle");
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 2);
        assert_eq!(readings[0].message.as_deref(), Some(NO_SESSION_MESSAGE));
    }

    #[test]
    fn only_the_keys_asked_for_are_answered() {
        let fixture = fixture("asked", vec![vec![session_line("playing", true)]]);
        let readings = fixture
            .provider
            .sample(&keys(&["media.title", "cpu.load"]), 1_000)
            .expect("a cycle");
        assert_eq!(readings.len(), 1);
        assert_eq!(fixture.provider.sample(&[], 1_000).unwrap().len(), 0);
    }

    #[test]
    fn a_playing_session_wins_over_a_current_one_at_a_tie() {
        let mut other = Session {
            session_id: "browser:2".to_owned(),
            app: "Browser".to_owned(),
            title: "Other".to_owned(),
            ..match session_line("paused", true) {
                Event::Session(session) => session,
                _ => unreachable!(),
            }
        };
        other.current = true;
        let fixture = fixture(
            "selection",
            vec![vec![Event::Session(other), session_line("playing", false)]],
        );
        let readings = fixture
            .provider
            .sample(&keys(&["media.title"]), 1_000)
            .expect("a cycle");
        assert_eq!(readings[0].text.as_deref(), Some("Title"));
    }
}
