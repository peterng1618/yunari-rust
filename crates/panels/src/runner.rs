//! One panel's life: open the device and send a frame only when the pixels
//! would be different.
//!
//! **The scheduler is the interesting half.** A panel is a wall screen with no
//! reader touching it, and every frame costs wire time the machine cannot get
//! back — so a frame nobody would notice is not sent. The comparison is over
//! the encoded bytes rather than a rendered hash: the encoder is the last step
//! before the wire, so equal bytes are the strongest evidence available that the
//! panel would look the same.

use std::sync::Mutex;

use crate::family::{CommandKind, PanelCommand, PanelDevice};

/// The panel's refresh cadence. A wall screen costs the machine it reports on.
pub const DEFAULT_INTERVAL_MS: u32 = 1_000;

/// Consecutive failures that mean the device is gone rather than one bad frame.
///
/// A lost port fails every write, so three ticks (about 3 s at the default
/// cadence) is long enough to outlast a single hiccup and short enough that the
/// owner sees the panel come back soon after the manager's retry pass.
pub const FAILURES_BEFORE_LOST: u32 = 3;

/// One encoded still for the panel, from its own geometry.
pub type Capture = Box<dyn Fn(u32, u32) -> Result<Vec<u8>, String> + Send + Sync>;

/// The clip a device plays itself, in the form its decoder takes.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoClip {
    Mp4(Vec<u8>),
    H264 { bytes: Vec<u8>, seconds: f64 },
}

impl VideoClip {
    fn borrowed(&self) -> crate::family::VideoClip<'_> {
        match self {
            Self::Mp4(bytes) => crate::family::VideoClip::Mp4(bytes),
            Self::H264 { bytes, seconds } => crate::family::VideoClip::H264 {
                bytes,
                seconds: *seconds,
            },
        }
    }
}

/// Resolves the clip a display's theme plays itself; `None` is a still panel.
pub type ClipSource = Box<dyn Fn() -> Result<Option<VideoClip>, String> + Send + Sync>;

/// One overlay capture: the UI alone, as a transparent PNG over the clip.
pub type OverlaySource = Box<dyn Fn(u32, u32) -> Result<Vec<u8>, String> + Send + Sync>;

/// Attempts at the device's video mode before the panel is a still panel.
///
/// A save of the assigned theme restarts the runner, and that race can make one
/// clip start fail on a busy device. Three attempts span it and are bounded, so
/// a clip the device really will not play costs three uploads, not an endless
/// one.
const VIDEO_ATTEMPTS_BEFORE_STILLS: u32 = 3;

struct State {
    /// True while ticks are allowed; cleared the moment a stop is asked for.
    running: bool,
    /// The device was given up on, so no further tick may read or write it.
    lost: bool,
    /// A tick is inside a capture or a send.
    in_flight: bool,
    /// The timer fired mid-tick; one follow-up runs when that tick settles.
    queued: bool,
    /// True once the device actually closed, so a failed close can be retried.
    closed: bool,
    /// Consecutive failed captures or sends; a success clears the count.
    failures: u32,
    /// The frame the panel is holding, so an identical capture is not sent.
    last: Option<Vec<u8>>,
    /// The clip not yet started, dropped once the device has it or refused it.
    pending: Option<VideoClip>,
    /// The device is playing the clip, so stop releases it.
    playing: bool,
    /// The overlay path still works; a refused overlay drops to stills.
    overlay: bool,
    start_failures: u32,
    overlay_failures: u32,
    /// The last overlay sent, so an unchanged UI is not sent again.
    last_overlay: Option<Vec<u8>>,
}

pub struct Runner<D: PanelDevice> {
    /// The only handle to the device: a control and a frame must never share it.
    device: Mutex<D>,
    capture: Capture,
    interval_ms: u32,
    state: Mutex<State>,
    /// The clip resolver and overlay capture, when this runner has a video
    /// theme. Both are absent or both present.
    video: Option<ClipSource>,
    overlay: Option<OverlaySource>,
    /// Called for every failed capture or send.
    on_error: Box<dyn Fn(&str) + Send + Sync>,
    /// Called once when the device has refused three frames in a row — a lost
    /// port, an unplugged cable, a re-enumerated interface.
    on_lost: Box<dyn Fn(&str) + Send + Sync>,
    /// Called when a frame the panel did not already hold has been sent.
    on_frame: Box<dyn Fn() + Send + Sync>,
}

impl<D: PanelDevice> Runner<D> {
    pub fn new(device: D, capture: Capture, interval_ms: u32) -> Self {
        Self {
            device: Mutex::new(device),
            capture,
            interval_ms,
            state: Mutex::new(State {
                running: true,
                lost: false,
                in_flight: false,
                queued: false,
                closed: false,
                failures: 0,
                last: None,
                pending: None,
                playing: false,
                overlay: false,
                start_failures: 0,
                overlay_failures: 0,
                last_overlay: None,
            }),
            video: None,
            overlay: None,
            on_error: Box::new(|_| {}),
            on_lost: Box::new(|_| {}),
            on_frame: Box::new(|| {}),
        }
    }

    /// Gives this runner a video theme: a clip it starts on the device and the
    /// UI-only capture that rides over it.
    pub fn with_video(mut self, video: ClipSource, overlay: OverlaySource) -> Self {
        self.video = Some(video);
        self.overlay = Some(overlay);
        self
    }

    pub fn on_error(mut self, report: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_error = Box::new(report);
        self
    }

    pub fn on_lost(mut self, report: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_lost = Box::new(report);
        self
    }

    pub fn on_frame(mut self, report: impl Fn() + Send + Sync + 'static) -> Self {
        self.on_frame = Box::new(report);
        self
    }

    /// Whether the device running now takes this control at all.
    pub fn accepts(&self, kind: CommandKind) -> bool {
        self.device.lock().expect("the panel").accepts(kind)
    }

    /// Runs one operation on the device under the runner's own lock, so a
    /// lighting write cannot interleave with a frame's reports.
    pub fn call<T>(&self, work: impl FnOnce(&mut D) -> Result<T, String>) -> Result<T, String> {
        let opened = {
            let state = self.state.lock().expect("the runner state");
            state.running && !state.lost && !state.closed
        };
        if !opened {
            return Err("The panel is not connected.".to_owned());
        }
        let mut device = self.device.lock().expect("the panel");
        work(&mut device)
    }

    pub fn interval_ms(&self) -> u32 {
        self.interval_ms
    }

    /// Opens the device. Refused with the driver's own sentence, which names
    /// what the owner has to do.
    pub fn open(&self) -> Result<(), String> {
        self.device.lock().expect("the panel").open()
    }

    pub fn running(&self) -> bool {
        let state = self.state.lock().expect("the runner state");
        state.running && !state.lost
    }

    /// True while a tick is inside a capture or a send.
    pub fn busy(&self) -> bool {
        self.state.lock().expect("the runner state").in_flight
    }

    /// One capture and one send, if the pixels moved. Answers whether a frame
    /// went out; a tick that found the previous one still running is skipped and
    /// answers `false` without starting a second capture.
    pub fn tick(&self) -> bool {
        {
            let mut state = self.state.lock().expect("the runner state");
            if !state.running || state.lost || state.closed {
                return false;
            }
            if state.in_flight {
                // A capture slow enough to still be running when the timer
                // fires again is not a reason to start a second one: two sends
                // racing for the same wire is worse than a skipped frame. The
                // tick it displaced is not dropped either — one follow-up runs
                // as soon as this one settles, which keeps a slow panel at its
                // own rate instead of the timer's.
                state.queued = true;
                return false;
            }
            state.in_flight = true;
        }

        let mut sent = false;
        loop {
            sent |= self.pass();
            let mut state = self.state.lock().expect("the runner state");
            state.in_flight = false;
            let again = state.queued && state.running && !state.lost && !state.closed;
            state.queued = false;
            if !again {
                return sent;
            }
            state.in_flight = true;
        }
    }

    /// One capture, and one send only when the bytes moved.
    fn pass(&self) -> bool {
        let geometry = self.device.lock().expect("the panel").geometry();
        if self.video.is_some() && self.device.lock().expect("the panel").plays_video() {
            if self.video_pass(geometry.width, geometry.height) {
                return true;
            }
            // While the overlay path is live an unchanged UI is the whole
            // frame: a still would paint the theme's background over the clip.
            // A failed start or a refused overlay has dropped out of it, and
            // stills are then the honest fallback.
            let active = {
                let state = self.state.lock().expect("the runner state");
                state.playing && state.overlay
            };
            if active {
                return false;
            }
        }
        self.still_pass(geometry.width, geometry.height)
    }

    /// Starts the clip when it is still pending, then sends only the UI over
    /// it. Answers whether a frame went out; `false` falls through to stills.
    fn video_pass(&self, width: u32, height: u32) -> bool {
        let (playing, start_failures) = {
            let state = self.state.lock().expect("the runner state");
            (state.playing, state.start_failures)
        };
        let mut pending = self.state.lock().expect("the runner state").pending.take();
        if pending.is_none() && !playing && start_failures < VIDEO_ATTEMPTS_BEFORE_STILLS {
            match self.video.as_ref().map(|video| video()) {
                Some(Ok(Some(clip))) => pending = Some(clip),
                Some(Ok(None)) => {
                    // No clip: the panel stays on stills and never retries.
                    self.state.lock().expect("the runner state").start_failures =
                        VIDEO_ATTEMPTS_BEFORE_STILLS;
                    return false;
                }
                Some(Err(error)) => {
                    (self.on_error)(&error);
                    self.state.lock().expect("the runner state").start_failures =
                        VIDEO_ATTEMPTS_BEFORE_STILLS;
                    return false;
                }
                None => return false,
            }
        }
        if let Some(clip) = pending {
            match self
                .device
                .lock()
                .expect("the panel")
                .start_video(clip.borrowed())
            {
                Ok(()) => {
                    let mut state = self.state.lock().expect("the runner state");
                    state.playing = true;
                    state.overlay = true;
                    state.start_failures = 0;
                    state.last_overlay = None;
                }
                Err(error) => {
                    (self.on_error)(&error);
                    let mut state = self.state.lock().expect("the runner state");
                    state.start_failures += 1;
                    // A refusal can be the restart the save just caused; only
                    // a device that refuses every attempt lacks the mode.
                    if state.start_failures < VIDEO_ATTEMPTS_BEFORE_STILLS {
                        state.pending = Some(clip);
                    }
                }
            }
        }

        let (playing, overlay) = {
            let state = self.state.lock().expect("the runner state");
            (state.playing, state.overlay)
        };
        if !playing || !overlay {
            return false;
        }
        let Some(capture) = self.overlay.as_ref() else {
            return false;
        };
        let frame = match capture(width, height) {
            Ok(frame) => frame,
            Err(error) => {
                self.failed(&error);
                return false;
            }
        };
        if self
            .state
            .lock()
            .expect("the runner state")
            .last_overlay
            .as_ref()
            .is_some_and(|last| *last == frame)
        {
            return false;
        }
        match self
            .device
            .lock()
            .expect("the panel")
            .present_overlay(&frame)
        {
            Ok(()) => {
                let mut state = self.state.lock().expect("the runner state");
                state.failures = 0;
                state.overlay_failures = 0;
                state.last_overlay = Some(frame);
                drop(state);
                (self.on_frame)();
                true
            }
            Err(error) => {
                self.failed(&error);
                let mut state = self.state.lock().expect("the runner state");
                state.overlay_failures += 1;
                // A video mode the device cannot compose: stills are the
                // honest fallback, and the clip keeps playing underneath.
                if state.overlay_failures >= VIDEO_ATTEMPTS_BEFORE_STILLS {
                    state.overlay = false;
                }
                false
            }
        }
    }

    fn still_pass(&self, width: u32, height: u32) -> bool {
        let frame = match (self.capture)(width, height) {
            Ok(frame) => frame,
            Err(error) => {
                self.failed(&error);
                return false;
            }
        };

        if self
            .state
            .lock()
            .expect("the runner state")
            .last
            .as_ref()
            .is_some_and(|last| *last == frame)
        {
            return false;
        }

        if let Err(error) = self.device.lock().expect("the panel").present(&frame) {
            // A send that failed did not put these pixels on the panel, so the
            // last frame is not this one: keeping it would suppress the retry.
            self.failed(&error);
            return false;
        }

        {
            let mut state = self.state.lock().expect("the runner state");
            state.failures = 0;
            state.last = Some(frame);
        }
        (self.on_frame)();
        true
    }

    /// Counts one failed capture or send, and gives the device up after the
    /// threshold. `on_lost` runs once; the message is the failure that tripped
    /// it, so the manager's log names the cause rather than a status sentence.
    fn failed(&self, message: &str) {
        (self.on_error)(message);
        let mut state = self.state.lock().expect("the runner state");
        state.failures += 1;
        if state.lost || state.failures < FAILURES_BEFORE_LOST {
            return;
        }
        state.lost = true;
        // The same flag a stop clears, so a status read answers "not connected"
        // before the manager's detach lands rather than after it.
        state.running = false;
        drop(state);
        (self.on_lost)(message);
    }

    /// Runs one control on the device, waiting out a frame in flight so the
    /// control cannot land inside one.
    pub fn control(&self, command: PanelCommand) -> Result<String, String> {
        let opened = {
            let state = self.state.lock().expect("the runner state");
            state.running && !state.lost && !state.closed
        };
        if !opened {
            return Err("The panel is not connected.".to_owned());
        }
        self.wait_idle();
        let mut device = self.device.lock().expect("the panel");
        if !device.accepts(command.kind()) {
            return Err(format!(
                "The panel does not have a {} control.",
                command.kind().as_str()
            ));
        }
        device.run(command)
    }

    fn wait_idle(&self) {
        while self.busy() {
            std::thread::yield_now();
        }
    }

    /// Stops feeding the panel and closes the device once.
    pub fn stop(&self) -> Result<(), String> {
        {
            let mut state = self.state.lock().expect("the runner state");
            state.running = false;
        }
        self.wait_idle();
        let playing = {
            let mut state = self.state.lock().expect("the runner state");
            if state.closed {
                return Ok(());
            }
            state.closed = true;
            state.playing
        };
        let mut device = self.device.lock().expect("the panel");
        // The clip belongs to the device, so it is stopped with it rather than
        // left playing behind a port the next runner wants.
        if playing {
            device.stop_video();
        }
        device.close()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    use super::*;
    use crate::family::{CommandKind, Geometry, StillFormat};

    struct FakePanel {
        opens: AtomicU32,
        presents: Arc<Mutex<Vec<Vec<u8>>>>,
        closes: AtomicU32,
        fail_close: bool,
        video: bool,
        starts: Arc<Mutex<Vec<usize>>>,
        overlays: Arc<Mutex<Vec<Vec<u8>>>>,
        fail_starts: AtomicU32,
        fail_overlays: AtomicU32,
        stops: Arc<AtomicU32>,
    }

    impl FakePanel {
        fn new() -> (Self, Arc<Mutex<Vec<Vec<u8>>>>) {
            let presents = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    opens: AtomicU32::new(0),
                    presents: Arc::clone(&presents),
                    closes: AtomicU32::new(0),
                    fail_close: false,
                    video: false,
                    starts: Arc::new(Mutex::new(Vec::new())),
                    overlays: Arc::new(Mutex::new(Vec::new())),
                    fail_starts: AtomicU32::new(0),
                    fail_overlays: AtomicU32::new(0),
                    stops: Arc::new(AtomicU32::new(0)),
                },
                presents,
            )
        }

        /// A panel whose decoder takes video, with what it was handed.
        #[allow(clippy::type_complexity)]
        fn video_panel() -> (Self, Arc<Mutex<Vec<usize>>>, Arc<Mutex<Vec<Vec<u8>>>>) {
            let (mut panel, _) = Self::new();
            panel.video = true;
            let starts = Arc::clone(&panel.starts);
            let overlays = Arc::clone(&panel.overlays);
            (panel, starts, overlays)
        }
    }

    impl PanelDevice for FakePanel {
        fn geometry(&self) -> Geometry {
            Geometry {
                width: 4,
                height: 2,
            }
        }

        fn open(&mut self) -> Result<(), String> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn present(&mut self, still: &[u8]) -> Result<(), String> {
            self.presents.lock().unwrap().push(still.to_vec());
            Ok(())
        }

        fn still_format(&self) -> StillFormat {
            StillFormat::Rgba
        }

        fn plays_video(&self) -> bool {
            self.video
        }

        fn start_video(&mut self, clip: crate::family::VideoClip<'_>) -> Result<(), String> {
            let bytes = match clip {
                crate::family::VideoClip::Mp4(bytes) => bytes.len(),
                crate::family::VideoClip::H264 { bytes, .. } => bytes.len(),
            };
            self.starts.lock().unwrap().push(bytes);
            let left = self.fail_starts.load(Ordering::SeqCst);
            if left > 0 {
                self.fail_starts.store(left - 1, Ordering::SeqCst);
                return Err("The panel refused the clip.".to_owned());
            }
            Ok(())
        }

        fn present_overlay(&mut self, png: &[u8]) -> Result<(), String> {
            let left = self.fail_overlays.load(Ordering::SeqCst);
            if left > 0 {
                self.fail_overlays.store(left - 1, Ordering::SeqCst);
                return Err("The panel refused the overlay.".to_owned());
            }
            self.overlays.lock().unwrap().push(png.to_vec());
            Ok(())
        }

        fn stop_video(&mut self) {
            self.stops.fetch_add(1, Ordering::SeqCst);
        }

        fn accepts(&self, kind: CommandKind) -> bool {
            kind == CommandKind::Brightness
        }

        fn run(&mut self, command: PanelCommand) -> Result<String, String> {
            Ok(format!("{command:?}"))
        }

        fn close(&mut self) -> Result<(), String> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            if self.fail_close {
                return Err("The port was already gone.".to_owned());
            }
            Ok(())
        }
    }

    fn fixed(frame: Vec<u8>) -> Capture {
        Box::new(move |_, _| Ok(frame.clone()))
    }

    #[test]
    fn an_unchanged_capture_is_not_sent_twice() {
        let (panel, presents) = FakePanel::new();
        let runner = Runner::new(panel, fixed(vec![1, 2, 3]), DEFAULT_INTERVAL_MS);
        assert!(runner.tick());
        assert!(!runner.tick());
        assert_eq!(presents.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_changed_capture_is_sent() {
        let (panel, presents) = FakePanel::new();
        let frame = AtomicU32::new(1);
        let runner = Runner::new(
            panel,
            Box::new(move |_, _| Ok(vec![frame.fetch_add(1, Ordering::SeqCst) as u8])),
            DEFAULT_INTERVAL_MS,
        );
        assert!(runner.tick());
        assert!(runner.tick());
        assert_eq!(presents.lock().unwrap().len(), 2);
    }

    #[test]
    fn three_failures_in_a_row_report_the_panel_lost_once() {
        let (panel, _) = FakePanel::new();
        let lost = Arc::new(AtomicU32::new(0));
        let reported = Arc::clone(&lost);
        let runner = Runner::new(
            panel,
            Box::new(|_, _| Err("The port vanished.".to_owned())),
            DEFAULT_INTERVAL_MS,
        )
        .on_lost(move |_| {
            reported.fetch_add(1, Ordering::SeqCst);
        });
        for _ in 0..5 {
            assert!(!runner.tick());
        }
        assert_eq!(lost.load(Ordering::SeqCst), 1);
        assert!(!runner.running());
    }

    #[test]
    fn a_stop_after_a_failure_still_closes_the_device_once() {
        let (panel, _) = FakePanel::new();
        let runner = Runner::new(
            panel,
            Box::new(|_, _| Err("One bad frame.".to_owned())),
            DEFAULT_INTERVAL_MS,
        );
        assert!(!runner.tick());
        runner.stop().expect("the device closes");
        runner.stop().expect("a second stop is a no-op");
        assert_eq!(
            runner.device.lock().unwrap().closes.load(Ordering::SeqCst),
            1
        );
        assert!(!runner.tick());
    }

    #[test]
    fn a_tick_during_an_in_flight_tick_queues_exactly_one_follow_up() {
        let (panel, presents) = FakePanel::new();
        let (started, arrived) = mpsc::channel::<()>();
        let (release, waiting) = mpsc::channel::<()>();
        let waiting = Mutex::new(waiting);
        let captures = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&captures);

        let runner = Arc::new(Runner::new(
            panel,
            Box::new(move |_, _| {
                let count = counted.fetch_add(1, Ordering::SeqCst);
                if count == 0 {
                    started.send(()).expect("the test is listening");
                    waiting.lock().expect("the gate").recv().expect("released");
                }
                Ok(vec![(count + 1) as u8])
            }),
            DEFAULT_INTERVAL_MS,
        ));

        let first = {
            let runner = Arc::clone(&runner);
            std::thread::spawn(move || runner.tick())
        };
        arrived.recv().expect("the first capture started");

        // Two ticks arrive while the first is in flight; one follow-up runs.
        assert!(!runner.tick());
        assert!(!runner.tick());
        release.send(()).expect("the first capture is released");

        assert!(
            first.join().expect("the ticking thread"),
            "a frame went out"
        );
        assert_eq!(captures.load(Ordering::SeqCst), 2);
        assert_eq!(presents.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_control_waits_for_the_frame_in_flight() {
        let (panel, presents) = FakePanel::new();
        let runner = Runner::new(panel, fixed(vec![7]), DEFAULT_INTERVAL_MS);
        assert_eq!(
            runner.control(PanelCommand::Brightness(40)).unwrap(),
            "Brightness(40)"
        );
        assert!(
            runner.control(PanelCommand::Restart).is_err(),
            "the panel has no restart control"
        );
        // A control neither sends nor suppresses a frame.
        assert!(runner.tick());
        assert_eq!(presents.lock().unwrap().len(), 1);
    }

    fn video_runner(panel: FakePanel, overlay: Vec<u8>) -> Runner<FakePanel> {
        Runner::new(panel, fixed(vec![9, 9]), DEFAULT_INTERVAL_MS).with_video(
            Box::new(|| Ok(Some(VideoClip::Mp4(vec![1, 2, 3])))),
            Box::new(move |_, _| Ok(overlay.clone())),
        )
    }

    #[test]
    fn a_clip_starts_once_and_an_unchanged_overlay_is_not_sent_twice() {
        let (panel, starts, overlays) = FakePanel::video_panel();
        let runner = video_runner(panel, vec![4, 5]);
        // One tick resolves the clip, starts it and paints the UI over it.
        assert!(runner.tick());
        // The UI has not moved, so there is nothing left to send over it.
        assert!(!runner.tick());
        assert_eq!(*starts.lock().unwrap(), vec![3]);
        assert_eq!(overlays.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_refused_start_is_retried_three_times_then_left_to_stills() {
        let (panel, starts, overlays) = FakePanel::video_panel();
        panel.fail_starts.store(3, Ordering::SeqCst);
        let runner = video_runner(panel, vec![4, 5]);
        for _ in 0..5 {
            runner.tick();
        }
        assert_eq!(starts.lock().unwrap().len(), 3);
        assert!(overlays.lock().unwrap().is_empty());
    }

    #[test]
    fn a_refused_overlay_falls_back_to_stills_over_the_playing_clip() {
        let (panel, starts, overlays) = FakePanel::video_panel();
        panel.fail_overlays.store(3, Ordering::SeqCst);
        let presents = Arc::clone(&panel.presents);
        let runner = video_runner(panel, vec![4, 5]);
        runner.tick();
        runner.tick();
        runner.tick();
        runner.tick();
        assert_eq!(starts.lock().unwrap().len(), 1);
        assert!(overlays.lock().unwrap().is_empty());
        assert!(!presents.lock().unwrap().is_empty());
    }

    #[test]
    fn stopping_the_runner_stops_the_clip_before_it_closes() {
        let (panel, _, _) = FakePanel::video_panel();
        let stops = Arc::clone(&panel.stops);
        let runner = video_runner(panel, vec![4]);
        runner.tick();
        runner.tick();
        runner.stop().unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
    }
}
