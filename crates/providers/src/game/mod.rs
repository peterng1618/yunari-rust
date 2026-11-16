//! The game the user is playing, identified from local evidence.
//!
//! One PowerShell helper reports the foreground process once a second while a
//! display binds a `game.*` key; Steam's installed-app catalogue is read once
//! per demand window. A match names the game and serves the cover Steam already
//! cached — never a network fetch.
//!
//! ponytail: identification is the Steam install folder. The catalogue of
//! launcher games (Epic/GOG/Itch/Xbox) and the engine-window classifier are not
//! ported yet; a non-Steam game is `missing`, not the previous game.

pub mod script;
pub mod steam;
pub mod vdf;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;
use crate::vocabulary;
use crate::winps;
use script::Foreground;
use steam::SteamApp;

pub const KEYS: &[&str] = &["game.name", "game.artwork", "game.running", "game.app"];

pub const WAITING_GAME_MESSAGE: &str = "Waiting for the game helper.";
pub const NO_GAME_MESSAGE: &str = "No game process is running.";
pub const NO_PROCESS_NAME_MESSAGE: &str = "The foreground process could not be named.";
pub const NO_ARTWORK_MESSAGE: &str = "Game artwork is not available yet.";
pub const HELPER_STOPPED_MESSAGE: &str = "The game helper stopped reporting.";
pub const START_FAILURE: &str = "The game helper could not be started: ";

/// No display asked for a game key for this long, so the helper stops.
const DEFAULT_GRACE_MS: u64 = 30_000;
/// A helper that failed is not retried before this, so it cannot spin.
const DEFAULT_RETRY_MS: u64 = 60_000;
/// A helper silent for this long is wedged; its readings are dropped.
const LIVENESS_MS: u64 = 60_000;
/// The installed catalogue is read once per demand window, not per poll.
const LIBRARY_TTL_MS: u64 = 300_000;

/// What one read of Steam's library found. The root is kept so a cover's
/// existence can be checked without reading the catalogue again.
pub struct Library {
    pub root: Option<PathBuf>,
    pub apps: Vec<SteamApp>,
}

/// Launches the foreground helper. A seam so a test scripts its lines.
pub type Spawn = Arc<dyn Fn() -> Result<(Receiver<Foreground>, winps::Stop), String> + Send + Sync>;
/// Reads the installed catalogue. A seam so a test owns the file system.
pub type LibraryLoader = Arc<dyn Fn() -> Library + Send + Sync>;

fn spawn_helper() -> Result<(Receiver<Foreground>, winps::Stop), String> {
    let (lines, stop) =
        winps::spawn(script::SCRIPT).map_err(|error| format!("{START_FAILURE}{error}"))?;
    let (sender, events) = mpsc::channel();
    thread::spawn(move || {
        for line in lines {
            let Some(foreground) = script::parse(line.trim()) else {
                continue;
            };
            if sender.send(foreground).is_err() {
                return;
            }
        }
    });
    Ok((events, stop))
}

fn read_library() -> Library {
    let root = steam::root();
    let apps = root.as_deref().map(steam::apps).unwrap_or_default();
    Library { root, apps }
}

struct Helper {
    events: Receiver<Foreground>,
    stop: Option<winps::Stop>,
}

#[derive(Default)]
struct State {
    foreground: Option<Foreground>,
    library: Option<Library>,
    library_read_at_ms: Option<u64>,
    /// Redacted; may reach a browser through `health()` and a reading.
    failure: Option<String>,
    helper: Option<Helper>,
    last_demand_ms: Option<u64>,
    retry_at_ms: u64,
    last_line_ms: u64,
}

pub struct GameProvider {
    spawn: Spawn,
    library: LibraryLoader,
    grace_ms: u64,
    retry_ms: u64,
    state: Mutex<State>,
}

impl Default for GameProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GameProvider {
    pub fn new() -> Self {
        Self::with_parts(
            Arc::new(spawn_helper),
            Arc::new(read_library),
            DEFAULT_GRACE_MS,
            DEFAULT_RETRY_MS,
        )
    }

    pub fn with_parts(spawn: Spawn, library: LibraryLoader, grace_ms: u64, retry_ms: u64) -> Self {
        Self {
            spawn,
            library,
            grace_ms,
            retry_ms,
            state: Mutex::new(State::default()),
        }
    }

    /// Ends the helper; a later sample starts it again.
    pub fn stop(&self) {
        let mut state = self.state.lock().expect("the game state");
        teardown(&mut state);
    }

    fn start(&self, state: &mut State, now_ms: u64) {
        if state.helper.is_some() || now_ms < state.retry_at_ms {
            return;
        }
        match (self.spawn)() {
            Ok((events, stop)) => {
                state.helper = Some(Helper {
                    events,
                    stop: Some(stop),
                });
                state.last_line_ms = now_ms;
            }
            Err(reason) => fail(state, now_ms, reason, self.retry_ms),
        }
    }

    /// Drains what the helper reported since the last poll.
    fn collect(&self, state: &mut State, now_ms: u64) {
        let Some(helper) = state.helper.take() else {
            return;
        };
        let mut latest = None;
        let mut disconnected = false;
        loop {
            match helper.events.try_recv() {
                Ok(foreground) => latest = Some(foreground),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        state.helper = Some(helper);

        if let Some(foreground) = latest {
            state.foreground = Some(foreground);
            state.failure = None;
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

    /// Reads the catalogue once per demand window.
    fn refresh_library(&self, state: &mut State, now_ms: u64) {
        let fresh = state
            .library_read_at_ms
            .is_some_and(|read_at| now_ms.saturating_sub(read_at) < LIBRARY_TTL_MS);
        if state.library.is_some() && fresh {
            return;
        }
        state.library = Some((self.library)());
        state.library_read_at_ms = Some(now_ms);
    }
}

impl Provider for GameProvider {
    fn id(&self) -> &str {
        "game"
    }

    fn label(&self) -> &str {
        "Game"
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
        let mut state = self.state.lock().expect("the game state");

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
        if state.helper.is_none() {
            self.start(&mut state, now_ms);
        }
        self.collect(&mut state, now_ms);
        self.refresh_library(&mut state, now_ms);

        Ok(wanted
            .into_iter()
            .map(|key| reading_of(&state, key))
            .collect())
    }

    fn health(&self) -> Health {
        match self.state.lock().expect("the game state").failure.clone() {
            Some(reason) => Health::Unavailable(reason),
            None => Health::Ok,
        }
    }
}

/// The app whose install folder holds the foreground executable.
fn matched_app(state: &State) -> Option<&SteamApp> {
    let executable = state.foreground.as_ref()?.path.as_deref()?;
    let library = state.library.as_ref()?;
    steam::app_for(&library.apps, Path::new(executable))
}

fn reading_of(state: &State, key: &str) -> Reading {
    let failure = state.failure.as_deref();
    let app = matched_app(state);

    match key {
        "game.app" => match state.foreground.as_ref().and_then(Foreground::executable) {
            Some(name) => Reading::text(key, name.to_owned()),
            None => Reading::missing(key, failure.unwrap_or(NO_PROCESS_NAME_MESSAGE)),
        },
        // A fullscreen D3D app is running even when its install folder is not
        // one Steam wrote; the name stays a gap.
        "game.running" => {
            let running =
                app.is_some() || state.foreground.as_ref().is_some_and(|f| f.d3d_fullscreen);
            Reading::text(key, if running { "on" } else { "off" })
        }
        _ => {
            let Some(app) = app else {
                return match failure {
                    Some(reason) => Reading::failed(key, reason.to_owned()),
                    None => Reading::missing(key, NO_GAME_MESSAGE),
                };
            };
            if key == "game.name" {
                return Reading::text(key, app.name.clone());
            }
            // The cover is named only when Steam's cache actually holds it, so
            // a display never draws a broken picture.
            let cover = state
                .library
                .as_ref()
                .and_then(|library| library.root.as_deref())
                .is_some_and(|root| steam::box_art(root, &app.app_id).is_some());
            if cover {
                Reading::text(key, format!("/api/game/artwork/{}", app.app_id))
            } else {
                Reading::missing(key, NO_ARTWORK_MESSAGE)
            }
        }
    }
}

fn stop_helper(state: &mut State) {
    if let Some(helper) = state.helper.take()
        && let Some(stop) = helper.stop
    {
        stop();
    }
}

fn fail(state: &mut State, now_ms: u64, reason: String, retry_ms: u64) {
    state.foreground = None;
    state.failure = Some(reason);
    state.retry_at_ms = now_ms.saturating_add(retry_ms);
}

fn teardown(state: &mut State) {
    stop_helper(state);
    state.foreground = None;
    state.library = None;
    state.library_read_at_ms = None;
    state.failure = None;
    state.retry_at_ms = 0;
    state.last_demand_ms = None;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::sample::Status;

    /// The helper a test drives directly: one channel per start, and a handle
    /// to push lines at whichever start is current.
    struct Fixture {
        provider: GameProvider,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        sender: Arc<Mutex<Option<mpsc::Sender<Foreground>>>>,
    }

    impl Fixture {
        fn new(apps: Vec<SteamApp>) -> Self {
            let starts = Arc::new(AtomicUsize::new(0));
            let stops = Arc::new(AtomicUsize::new(0));
            let sender = Arc::new(Mutex::new(None));
            let counted = Arc::clone(&starts);
            let counted_stops = Arc::clone(&stops);
            let current = Arc::clone(&sender);
            let spawn: Spawn = Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                let (lines, receiver) = mpsc::channel();
                *current.lock().unwrap() = Some(lines);
                let stops = Arc::clone(&counted_stops);
                let current = Arc::clone(&current);
                Ok((
                    receiver,
                    Box::new(move || {
                        *current.lock().unwrap() = None;
                        stops.fetch_add(1, Ordering::SeqCst);
                    }) as winps::Stop,
                ))
            });
            // The library carries no root, so artwork is a gap rather than a
            // disk lookup, which is what a machine without Steam sees.
            let library: LibraryLoader = Arc::new(move || Library {
                root: None,
                apps: apps.clone(),
            });
            Self {
                provider: GameProvider::with_parts(spawn, library, 1_000, 0),
                starts,
                stops,
                sender,
            }
        }

        fn push(&self, foreground: Foreground) {
            if let Some(sender) = self.sender.lock().unwrap().as_ref() {
                sender.send(foreground).ok();
            }
        }

        fn sample(&self, names: &[&str], now_ms: u64) -> Vec<Reading> {
            self.provider.sample(&keys(names), now_ms).expect("a cycle")
        }
    }

    fn steam_app(app_id: &str, name: &str, install: &str) -> SteamApp {
        SteamApp {
            app_id: app_id.to_owned(),
            name: name.to_owned(),
            install_dir: PathBuf::from(install),
        }
    }

    fn foreground(path: &str) -> Foreground {
        Foreground {
            pid: 4242,
            path: Some(path.to_owned()),
            name: None,
            class: None,
            fullscreen: true,
            d3d_fullscreen: true,
        }
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
    fn a_steam_app_is_named_from_the_foreground_executable() {
        let fixture = Fixture::new(vec![steam_app(
            "730",
            "Counter-Strike 2",
            r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive",
        )]);
        // The first sample starts the helper; the lines come on later polls.
        fixture.sample(KEYS, 1_000);
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 1);
        fixture.push(foreground(
            r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive\game\bin\win64\cs2.exe",
        ));
        let readings = fixture.sample(KEYS, 1_100);
        assert_eq!(
            reading(&readings, "game.name").text.as_deref(),
            Some("Counter-Strike 2")
        );
        assert_eq!(
            reading(&readings, "game.app").text.as_deref(),
            Some("cs2.exe")
        );
        assert_eq!(
            reading(&readings, "game.running").text.as_deref(),
            Some("on")
        );
        // No cached cover on this machine is a gap, not a broken picture.
        assert_eq!(
            reading(&readings, "game.artwork").message.as_deref(),
            Some(NO_ARTWORK_MESSAGE)
        );
    }

    #[test]
    fn an_unknown_foreground_executable_is_missing_not_the_last_game() {
        let fixture = Fixture::new(vec![steam_app(
            "730",
            "Counter-Strike 2",
            r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive",
        )]);
        fixture.sample(KEYS, 1_000);
        fixture.push(foreground(
            r"D:\SteamLibrary\steamapps\common\Counter-Strike Global Offensive\cs2.exe",
        ));
        fixture.sample(KEYS, 1_100);
        fixture.push(foreground(r"C:\Windows\notepad.exe"));
        let readings = fixture.sample(KEYS, 1_200);
        assert_eq!(reading(&readings, "game.name").status, Status::Missing);
        assert_eq!(
            reading(&readings, "game.name").message.as_deref(),
            Some(NO_GAME_MESSAGE)
        );
        assert_eq!(
            reading(&readings, "game.app").text.as_deref(),
            Some("notepad.exe")
        );
    }

    #[test]
    fn no_foreground_window_is_no_game_rather_than_the_last_one() {
        let fixture = Fixture::new(vec![steam_app("730", "Counter-Strike 2", r"D:\Steam\game")]);
        fixture.sample(KEYS, 1_000);
        fixture.push(Foreground::default());
        let readings = fixture.sample(KEYS, 1_100);
        assert_eq!(
            reading(&readings, "game.running").text.as_deref(),
            Some("off")
        );
        assert_eq!(
            reading(&readings, "game.name").message.as_deref(),
            Some(NO_GAME_MESSAGE)
        );
        assert_eq!(
            reading(&readings, "game.app").message.as_deref(),
            Some(NO_PROCESS_NAME_MESSAGE)
        );
    }

    #[test]
    fn a_helper_that_cannot_start_says_why_and_is_not_retried_at_once() {
        let spawn: Spawn = Arc::new(|| Err("PowerShell is not installed.".to_owned()));
        let library: LibraryLoader = Arc::new(|| Library {
            root: None,
            apps: Vec::new(),
        });
        let provider = GameProvider::with_parts(spawn, library, 30_000, 60_000);
        let readings = provider
            .sample(&keys(&["game.name"]), 1_000)
            .expect("a cycle");
        assert_eq!(readings[0].status, Status::Error);
        assert_eq!(
            readings[0].message.as_deref(),
            Some("PowerShell is not installed.")
        );
        assert!(matches!(provider.health(), Health::Unavailable(_)));
        let readings = provider
            .sample(&keys(&["game.name"]), 2_000)
            .expect("a cycle");
        assert_eq!(
            readings[0].message.as_deref(),
            Some("PowerShell is not installed.")
        );
    }

    #[test]
    fn a_lapsed_demand_window_stops_the_helper_and_a_new_ask_starts_one() {
        let fixture = Fixture::new(Vec::new());
        fixture.sample(&["game.app"], 1_000);
        fixture.push(foreground(r"C:\Windows\notepad.exe"));
        assert_eq!(
            fixture.sample(&["game.app"], 1_100)[0].text.as_deref(),
            Some("notepad.exe")
        );
        assert_eq!(fixture.stops.load(Ordering::SeqCst), 0);
        fixture.provider.sample(&[], 3_500).expect("a cycle");
        assert_eq!(fixture.stops.load(Ordering::SeqCst), 1);
        let readings = fixture.sample(&["game.app"], 3_600);
        assert_eq!(fixture.starts.load(Ordering::SeqCst), 2);
        assert_eq!(
            readings[0].message.as_deref(),
            Some(NO_PROCESS_NAME_MESSAGE)
        );
    }

    #[test]
    fn only_the_keys_asked_for_are_answered() {
        let fixture = Fixture::new(Vec::new());
        let readings = fixture.sample(&["game.app", "cpu.load"], 1_000);
        assert_eq!(readings.len(), 1);
        assert_eq!(fixture.provider.sample(&[], 1_000).unwrap().len(), 0);
    }
}
