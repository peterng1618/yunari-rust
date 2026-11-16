//! Frame statistics for the active 3D application.
//!
//! Sources are tried in order: RTSS first, PresentMon only when RTSS cannot
//! report. This provider owns the rolling window, so the numbers a theme sees
//! are computed here rather than trusted from either source. A key with no
//! frames behind it is `missing` — never zero.
//!
//! A source runs a helper; frames arrive on a channel and are stamped with the
//! poll that drained them, which is the receipt time as far as the window is
//! concerned.

pub mod presentmon;
pub mod rolling;
pub mod rtss;

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;
use crate::vocabulary;
use rolling::FrameWindow;

/// No display asked for a frame key for this long, so the helper stops.
const DEFAULT_GRACE_MS: u64 = 30_000;
/// A source that failed is not retried before this, so it cannot spin.
const DEFAULT_RETRY_MS: u64 = 60_000;

pub const KEYS: &[&str] = &["fps.current", "frametime.current", "fps.low1", "frame.app"];

pub const NO_FRAMES_MESSAGE: &str =
    "No frame timing has been reported by an available frame source.";
pub const WAITING_LOW1_MESSAGE: &str = "Not enough frames yet for a 1% low.";
pub const NO_APP_MESSAGE: &str = "No game process is reporting frame statistics.";
pub const SOURCE_STOPPED_MESSAGE: &str = "The frame source stopped reporting.";

/// One thing a source reports.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameEvent {
    Frame {
        frametime_ms: f64,
        /// The executable RTSS or PresentMon names, when it names one.
        app: Option<String>,
    },
    /// The source cannot report, and the reason is what a display shows.
    Unavailable(String),
}

/// Ends a helper and waits for its thread to stop.
pub type Stop = Box<dyn FnOnce() + Send>;

/// Where frames come from. A source is started only while a display asks.
pub trait FrameSource: Send + Sync {
    fn id(&self) -> &'static str;
    fn start(&self) -> Result<(Receiver<FrameEvent>, Stop), String>;
}

struct Helper {
    events: Receiver<FrameEvent>,
    stop: Option<Stop>,
}

#[derive(Default)]
struct State {
    window: FrameWindow,
    app: Option<String>,
    /// Redacted; may reach a browser through `health()` and a reading.
    failure: Option<String>,
    helper: Option<Helper>,
    source_index: usize,
    last_demand_ms: Option<u64>,
    retry_at_ms: u64,
    /// Every source failed; the order is retried only in a new demand window.
    window_failed: bool,
}

pub struct FrameProvider {
    sources: Vec<Box<dyn FrameSource>>,
    grace_ms: u64,
    retry_ms: u64,
    state: Mutex<State>,
}

impl FrameProvider {
    pub fn new(sources: Vec<Box<dyn FrameSource>>) -> Self {
        Self::with_timings(sources, DEFAULT_GRACE_MS, DEFAULT_RETRY_MS)
    }

    pub fn with_timings(sources: Vec<Box<dyn FrameSource>>, grace_ms: u64, retry_ms: u64) -> Self {
        Self {
            sources,
            grace_ms,
            retry_ms,
            state: Mutex::new(State::default()),
        }
    }

    /// Ends every helper; the host calls this when it shuts down.
    pub fn stop(&self) {
        let mut state = self.state.lock().expect("the frame state");
        teardown(&mut state);
    }

    fn reconcile(&self, state: &mut State, now_ms: u64) {
        if state.window_failed || state.helper.is_some() || now_ms < state.retry_at_ms {
            return;
        }
        self.start_current(state, now_ms);
    }

    /// Starts the current source, or moves on when it cannot start.
    fn start_current(&self, state: &mut State, now_ms: u64) {
        let Some(source) = self.sources.get(state.source_index) else {
            state.window_failed = true;
            return;
        };
        match source.start() {
            Ok((events, stop)) => {
                state.helper = Some(Helper {
                    events,
                    stop: Some(stop),
                });
            }
            Err(reason) => self.advance(state, now_ms, reason),
        }
    }

    /// Reports a reason once, then falls back to the next source or closes the
    /// window.
    fn advance(&self, state: &mut State, now_ms: u64, reason: String) {
        stop_helper(state);
        state.window.clear();
        state.app = None;
        state.failure = Some(reason);
        state.source_index += 1;

        if state.source_index < self.sources.len() {
            // Fall back inside the same demand window; the next source may work.
            state.failure = None;
            self.start_current(state, now_ms);
            return;
        }

        if self.sources.len() > 1 {
            // A denied trace session must not spawn a process per poll. The
            // window stays failed until demand lapses and returns.
            state.window_failed = true;
            return;
        }

        state.source_index = 0;
        state.retry_at_ms = now_ms.saturating_add(self.retry_ms);
    }
}

impl Provider for FrameProvider {
    fn id(&self) -> &str {
        "frame"
    }

    fn label(&self) -> &str {
        "Frames"
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
        let mut state = self.state.lock().expect("the frame state");

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
        self.reconcile(&mut state, now_ms);
        self.collect(&mut state, now_ms);

        Ok(wanted
            .into_iter()
            .map(|key| reading_of(&mut state, key, now_ms))
            .collect())
    }

    fn health(&self) -> Health {
        match self.state.lock().expect("the frame state").failure.clone() {
            Some(reason) => Health::Unavailable(reason),
            None => Health::Ok,
        }
    }
}

impl FrameProvider {
    /// Drains what the running helper has reported since the last poll.
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

        let mut failure = None;
        for event in events {
            match event {
                FrameEvent::Frame { frametime_ms, app } => {
                    state.window.add(frametime_ms, now_ms);
                    state.app = app;
                    state.failure = None;
                }
                FrameEvent::Unavailable(reason) => {
                    failure = Some(reason);
                    break;
                }
            }
        }
        if failure.is_none() && disconnected {
            failure = Some(SOURCE_STOPPED_MESSAGE.to_owned());
        }
        if let Some(reason) = failure {
            self.advance(state, now_ms, reason);
        }
    }
}

/// One key's reading from the window the source filled.
fn reading_of(state: &mut State, key: &str, now_ms: u64) -> Reading {
    let absent = |state: &State, message: &str| {
        Reading::missing(key, state.failure.as_deref().unwrap_or(message))
    };

    if key == "frame.app" {
        return match (&state.app, &state.failure) {
            (Some(app), _) => Reading::text(key, app.clone()),
            (None, _) => absent(state, NO_APP_MESSAGE),
        };
    }

    let Some(fps) = state.window.fps(now_ms) else {
        return absent(state, NO_FRAMES_MESSAGE);
    };
    let unit = vocabulary::describe(key)
        .and_then(|entry| entry.unit)
        .unwrap_or_default();

    match key {
        "fps.low1" => match state.window.low1(now_ms) {
            Some(low) => Reading::number(key, low, unit),
            None => absent(state, WAITING_LOW1_MESSAGE),
        },
        "fps.current" => Reading::number(key, fps, unit),
        // The mean frametime over the same second `fps.current` averages.
        _ => Reading::number(key, 1000.0 / fps, unit),
    }
}

fn stop_helper(state: &mut State) {
    if let Some(helper) = state.helper.take()
        && let Some(stop) = helper.stop
    {
        stop();
    }
}

fn teardown(state: &mut State) {
    stop_helper(state);
    state.window.clear();
    state.app = None;
    state.failure = None;
    // The next demand is a new window: order and backoff start clean.
    state.source_index = 0;
    state.window_failed = false;
    state.retry_at_ms = 0;
    state.last_demand_ms = None;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use super::*;
    use crate::sample::Status;

    /// One start's answer: events, or the reason it could not start.
    type Scripted = Result<Vec<FrameEvent>, String>;

    /// A source the test drives directly: it hands back a scripted set of
    /// events per start, and records how often it was started.
    struct Fake {
        id: &'static str,
        starts: Arc<AtomicUsize>,
        script: Arc<Mutex<Vec<Scripted>>>,
        stops: Arc<AtomicUsize>,
    }

    impl Fake {
        fn new(
            id: &'static str,
            script: Vec<Scripted>,
        ) -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let starts = Arc::new(AtomicUsize::new(0));
            let stops = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    id,
                    starts: Arc::clone(&starts),
                    script: Arc::new(Mutex::new(script)),
                    stops: Arc::clone(&stops),
                },
                starts,
                stops,
            )
        }
    }

    impl FrameSource for Fake {
        fn id(&self) -> &'static str {
            self.id
        }

        fn start(&self) -> Result<(Receiver<FrameEvent>, Stop), String> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            let scripted = {
                let mut script = self.script.lock().unwrap();
                if script.is_empty() {
                    Ok(Vec::new())
                } else {
                    script.remove(0)
                }
            };
            let stops = Arc::clone(&self.stops);
            match scripted {
                Ok(events) => {
                    let (sender, receiver) = mpsc::channel();
                    for event in events {
                        let _ = sender.send(event);
                    }
                    Ok((
                        receiver,
                        Box::new(move || {
                            // The sender stays alive until the helper stops, as
                            // a real source's thread does.
                            drop(sender);
                            stops.fetch_add(1, Ordering::SeqCst);
                        }) as Stop,
                    ))
                }
                Err(reason) => Err(reason),
            }
        }
    }

    fn frames(frametimes: &[f64], app: Option<&str>) -> Vec<FrameEvent> {
        frametimes
            .iter()
            .map(|frametime_ms| FrameEvent::Frame {
                frametime_ms: *frametime_ms,
                app: app.map(str::to_owned),
            })
            .collect()
    }

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn asked() -> Vec<String> {
        keys(KEYS)
    }

    #[test]
    fn a_source_that_answers_gives_fps_frametime_and_the_game() {
        let (source, starts, _) = Fake::new(
            "rtss",
            vec![Ok(frames(&[10.0, 10.0, 10.0, 10.0], Some("game.exe")))],
        );
        let provider = FrameProvider::new(vec![Box::new(source)]);

        let readings = provider.sample(&asked(), 1_000).expect("a cycle");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(readings[0].value, Some(100.0));
        assert_eq!(readings[0].unit.as_deref(), Some("fps"));
        assert!((readings[1].value.expect("a frametime") - 10.0).abs() < 1e-9);
        assert_eq!(readings[2].status, Status::Missing);
        assert!(
            readings[2]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("1% low"))
        );
        assert_eq!(readings[3].text.as_deref(), Some("game.exe"));
    }

    #[test]
    fn no_frames_is_missing_not_zero() {
        let (source, ..) = Fake::new("rtss", vec![Ok(Vec::new())]);
        let provider = FrameProvider::new(vec![Box::new(source)]);

        let readings = provider.sample(&asked(), 1_000).expect("a cycle");
        assert_eq!(readings[0].status, Status::Missing);
        assert_eq!(readings[0].value, None);
        assert_eq!(readings[0].message.as_deref(), Some(NO_FRAMES_MESSAGE));
        assert_eq!(readings[3].message.as_deref(), Some(NO_APP_MESSAGE));
        assert_eq!(provider.health(), Health::Ok);
    }

    #[test]
    fn a_failing_source_falls_back_to_the_next_one() {
        let (rtss, ..) = Fake::new(
            "rtss",
            vec![Err("RivaTuner Statistics Server is not running.".to_owned())],
        );
        let (presentmon, starts, _) = Fake::new("presentmon", vec![Ok(frames(&[20.0], None))]);
        let provider = FrameProvider::new(vec![Box::new(rtss), Box::new(presentmon)]);

        let readings = provider.sample(&asked(), 1_000).expect("a cycle");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(readings[0].value, Some(50.0));
    }

    #[test]
    fn every_source_failing_reports_the_reason_and_does_not_spin() {
        let (rtss, rtss_starts, _) = Fake::new(
            "rtss",
            vec![Err("RivaTuner Statistics Server is not running.".to_owned())],
        );
        let (presentmon, pm_starts, _) = Fake::new(
            "presentmon",
            vec![Err("PresentMon is not vendored.".to_owned())],
        );
        let provider = FrameProvider::new(vec![Box::new(rtss), Box::new(presentmon)]);

        let readings = provider.sample(&asked(), 1_000).expect("a cycle");
        assert_eq!(rtss_starts.load(Ordering::SeqCst), 1);
        assert_eq!(pm_starts.load(Ordering::SeqCst), 1);
        assert_eq!(
            readings[0].message.as_deref(),
            Some("PresentMon is not vendored.")
        );
        assert!(matches!(provider.health(), Health::Unavailable(_)));

        // The same window does not start them again on every poll.
        provider.sample(&asked(), 2_000).expect("a cycle");
        assert_eq!(rtss_starts.load(Ordering::SeqCst), 1);
        assert_eq!(pm_starts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_lapsed_demand_window_stops_the_helper_and_clears_the_failure() {
        let (source, starts, stops) = Fake::new(
            "rtss",
            vec![
                Ok(frames(&[10.0], None)),
                Err("RivaTuner Statistics Server is not running.".to_owned()),
            ],
        );
        let provider = FrameProvider::with_timings(vec![Box::new(source)], 1_000, 0);
        provider.sample(&asked(), 1_000).expect("a cycle");
        assert_eq!(stops.load(Ordering::SeqCst), 0);

        // Nothing asked for two seconds: the helper stops and the next ask is
        // a new window, which starts the source again.
        provider.sample(&[], 3_500).expect("a cycle");
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        let readings = provider.sample(&asked(), 3_600).expect("a cycle");
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert!(matches!(provider.health(), Health::Unavailable(_)));
        assert!(
            readings[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("not running"))
        );
    }

    #[test]
    fn a_source_that_reports_itself_unavailable_hands_over() {
        let (rtss, ..) = Fake::new(
            "rtss",
            vec![
                Ok(vec![
                    FrameEvent::Frame {
                        frametime_ms: 10.0,
                        app: None,
                    },
                    FrameEvent::Unavailable("RTSS shared memory is too small to read.".to_owned()),
                ]),
                Ok(Vec::new()),
            ],
        );
        let (presentmon, starts, _) = Fake::new(
            "presentmon",
            vec![Ok(frames(&[5.0], None)), Ok(frames(&[5.0], None))],
        );
        let provider = FrameProvider::new(vec![Box::new(rtss), Box::new(presentmon)]);

        let readings = provider.sample(&asked(), 1_000).expect("a cycle");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        // The first source's frames are gone with it; the fallback's own
        // frames answer from the next poll.
        assert_eq!(readings[0].status, Status::Missing);
        assert_eq!(readings[0].message.as_deref(), Some(NO_FRAMES_MESSAGE));

        let readings = provider.sample(&asked(), 2_000).expect("a cycle");
        assert_eq!(readings[0].value, Some(200.0));
    }

    #[test]
    fn only_the_keys_asked_for_are_answered() {
        let (source, ..) = Fake::new("rtss", vec![Ok(frames(&[4.0], None))]);
        let provider = FrameProvider::new(vec![Box::new(source)]);

        let readings = provider
            .sample(&keys(&["fps.current", "cpu.load"]), 1_000)
            .expect("a cycle");
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].semantic_key, "fps.current");
    }

    #[test]
    fn stopping_ends_the_helper() {
        let (source, _, stops) = Fake::new("rtss", vec![Ok(frames(&[4.0], None))]);
        let provider = FrameProvider::new(vec![Box::new(source)]);
        provider.sample(&asked(), 1_000).expect("a cycle");

        provider.stop();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}
