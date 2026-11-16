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

use crate::family::{PanelCommand, PanelDevice};

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
}

pub struct Runner<D: PanelDevice> {
    /// The only handle to the device: a control and a frame must never share it.
    device: Mutex<D>,
    capture: Capture,
    interval_ms: u32,
    state: Mutex<State>,
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
            }),
            on_error: Box::new(|_| {}),
            on_lost: Box::new(|_| {}),
            on_frame: Box::new(|| {}),
        }
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
        let frame = match (self.capture)(geometry.width, geometry.height) {
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
        {
            let mut state = self.state.lock().expect("the runner state");
            if state.closed {
                return Ok(());
            }
            state.closed = true;
        }
        let mut device = self.device.lock().expect("the panel");
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
                },
                presents,
            )
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
}
