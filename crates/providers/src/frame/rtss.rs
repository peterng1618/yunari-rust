//! Frame timing from RivaTuner Statistics Server's shared memory.
//!
//! RTSS is a third-party overlay that hooks the game; this source never starts
//! it, injects into anything, or reads a file it does not publish. When RTSS is
//! not running the helper says so once and the provider falls back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::{FrameEvent, FrameSource, Stop};

/// How often the mapping is re-read. RTSS itself refreshes on the game's frame.
const POLL: Duration = Duration::from_millis(100);

pub struct RtssSource;

impl FrameSource for RtssSource {
    fn id(&self) -> &'static str {
        "rtss"
    }

    fn start(&self) -> Result<(Receiver<FrameEvent>, Stop), String> {
        let (sender, receiver) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        let handle = thread::spawn(move || poll(sender, flag));
        Ok((
            receiver,
            Box::new(move || {
                stopped.store(true, Ordering::SeqCst);
                handle.join().ok();
            }) as Stop,
        ))
    }
}

/// Whether RTSS is running: it publishes its mapping only while it is, and
/// reading it changes nothing.
pub fn running() -> bool {
    #[cfg(windows)]
    {
        yunari_rtss::windows::read(yunari_rtss::MAP_NAME).is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn poll(sender: Sender<FrameEvent>, stopped: Arc<AtomicBool>) {
    use yunari_rtss::{MAP_NAME, Status, decode, frames_since, newest_index, windows};

    let mut reported_down = false;
    let mut last_pid = 0_u32;
    let mut last_newest: Option<usize> = None;

    while !stopped.load(Ordering::SeqCst) {
        let snapshot = windows::read(MAP_NAME)
            .ok_or(Status::NotRunning)
            .and_then(|bytes| decode(&bytes));

        match snapshot {
            Err(status) => {
                // One line per absence episode: the provider falls back once.
                if !reported_down {
                    let reason = status.reason().unwrap_or("RTSS cannot be read.").to_owned();
                    let _ = sender.send(FrameEvent::Unavailable(reason));
                    reported_down = true;
                }
                last_pid = 0;
                last_newest = None;
            }
            Ok(snapshot) => {
                reported_down = false;
                match snapshot.app {
                    Some(app) => {
                        let newest = newest_index(app.ring_pos);
                        // A new process or the first poll: the ring may predate
                        // us or hold another game's frames, so only frames
                        // observed from now on count.
                        let frames = match (app.pid == last_pid, last_newest) {
                            (true, Some(last)) => frames_since(&app.ring, app.ring_pos, last),
                            _ => Vec::new(),
                        };
                        last_pid = app.pid;
                        last_newest = Some(newest);
                        let name = (!app.name.is_empty()).then_some(app.name);
                        for microseconds in frames {
                            let event = FrameEvent::Frame {
                                frametime_ms: f64::from(microseconds) / 1000.0,
                                app: name.clone(),
                            };
                            if sender.send(event).is_err() {
                                return;
                            }
                        }
                    }
                    None => {
                        last_pid = 0;
                        last_newest = None;
                    }
                }
            }
        }

        thread::sleep(POLL);
    }
}

#[cfg(not(windows))]
fn poll(sender: Sender<FrameEvent>, _stopped: Arc<AtomicBool>) {
    let _ = sender.send(FrameEvent::Unavailable(
        "RTSS frame timing is read only on Windows.".to_owned(),
    ));
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_host_without_rtss_reports_the_reason_and_stops() {
        // RTSS may be installed on the machine running the tests, so this only
        // pins that the helper always settles rather than spinning forever.
        let source = RtssSource;
        let (events, stop) = source.start().expect("a helper");
        // The first event, if any, is either a frame or the absence reason.
        if let Ok(event) = events.recv_timeout(Duration::from_millis(500)) {
            match event {
                FrameEvent::Unavailable(reason) => {
                    assert!(
                        reason.contains("RivaTuner") || reason.contains("RTSS"),
                        "{reason}"
                    )
                }
                FrameEvent::Frame { frametime_ms, .. } => assert!(frametime_ms > 0.0),
            }
        }
        stop();
    }
}
