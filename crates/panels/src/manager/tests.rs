use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::family::{Geometry, StillFormat};

const FAMILY_VID: u16 = 0x1cbe;
const FAMILY_PID: u16 = 0xa092;

struct FakePanel {
    presents: Arc<AtomicU32>,
    closes: Arc<AtomicU32>,
    fail_present: bool,
    accepts: bool,
}

impl PanelDevice for FakePanel {
    fn geometry(&self) -> Geometry {
        Geometry {
            width: 464,
            height: 1920,
        }
    }

    fn open(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn present(&mut self, _still: &[u8]) -> Result<(), String> {
        self.presents.fetch_add(1, Ordering::SeqCst);
        if self.fail_present {
            return Err("The port vanished.".to_owned());
        }
        Ok(())
    }

    fn still_format(&self) -> StillFormat {
        StillFormat::Jpeg
    }

    fn accepts(&self, _kind: CommandKind) -> bool {
        self.accepts
    }

    fn run(&mut self, command: PanelCommand) -> Result<String, String> {
        Ok(format!("{command:?}"))
    }

    fn close(&mut self) -> Result<(), String> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Harness {
    manager: Manager,
    opens: Arc<AtomicU32>,
    closes: Arc<AtomicU32>,
    reports: Arc<Mutex<Vec<(String, String)>>>,
}

fn harness(fail_present: bool, accepts: bool) -> Harness {
    let opens = Arc::new(AtomicU32::new(0));
    let closes = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&opens);
    let closed = Arc::clone(&closes);
    let frames = Arc::new(AtomicU32::new(0));
    let counted_frames = Arc::clone(&frames);
    let reports = Arc::new(Mutex::new(Vec::new()));
    let reported = Arc::clone(&reports);

    let manager = Manager::with_device_source(
        Arc::new(
            move |_record: &DisplayRecord, _w, _h, _format: StillFormat| {
                Ok(vec![counted_frames.fetch_add(1, Ordering::SeqCst) as u8])
            },
        ),
        Box::new(move |_family: &PanelFamily| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakePanel {
                presents: Arc::new(AtomicU32::new(0)),
                closes: Arc::clone(&closed),
                fail_present,
                accepts,
            }) as Box<dyn PanelDevice>)
        }),
    )
    .on_error(move |display, message| {
        reported
            .lock()
            .unwrap()
            .push((display.to_owned(), message.to_owned()));
    });
    Harness {
        manager,
        opens,
        closes,
        reports,
    }
}

fn record(id: &str) -> DisplayRecord {
    DisplayRecord {
        id: id.to_owned(),
        label: id.to_owned(),
        panel: true,
        rotation: None,
        device: Some(PanelDeviceId {
            vendor_id: FAMILY_VID,
            product_id: FAMILY_PID,
            serial: format!("{id}-serial"),
            device_id: None,
        }),
        render_mode: None,
    }
}

#[test]
fn reconcile_starts_every_panel_record_and_stops_a_vanished_one() {
    let harness = harness(false, true);
    let first = record("one");
    let second = record("two");
    harness.manager.reconcile(&[first.clone(), second.clone()]);
    assert_eq!(harness.manager.running().len(), 2);
    assert_eq!(harness.opens.load(Ordering::SeqCst), 2);

    // The second record is gone; its device must be released.
    harness.manager.reconcile(&[first]);
    assert_eq!(harness.manager.running(), vec!["one".to_owned()]);
    assert_eq!(harness.closes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_lost_runner_is_detached_and_reported_retrying() {
    let harness = harness(true, true);
    harness.manager.reconcile(&[record("one")]);
    for _ in 0..3 {
        harness.manager.tick();
    }
    assert!(harness.manager.running().is_empty());
    assert_eq!(
        harness.manager.status("one"),
        PanelConnectionStatus::Disconnected
    );
    let reports = harness.reports.lock().unwrap();
    assert!(
        reports
            .iter()
            .any(|(_, message)| message == "disconnected; retrying."),
        "{reports:?}"
    );
}

#[test]
fn a_command_on_a_stopped_display_is_refused_in_words() {
    let harness = harness(false, true);
    let error = harness
        .manager
        .command("gone", PanelCommand::Brightness(50))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Panel display gone is not running; start it before sending a command."
    );
}

#[test]
fn a_device_without_controls_is_refused_in_words() {
    let harness = harness(false, false);
    harness.manager.reconcile(&[record("one")]);
    let error = harness
        .manager
        .command("one", PanelCommand::Brightness(50))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Panel display one does not accept commands."
    );
}

#[test]
fn a_restart_detaches_the_panel_and_says_when_it_returns() {
    let harness = harness(false, true);
    harness.manager.reconcile(&[record("one")]);
    assert_eq!(
        harness.manager.status("one"),
        PanelConnectionStatus::Connected
    );

    assert_eq!(
        harness
            .manager
            .command("one", PanelCommand::Restart)
            .unwrap(),
        "Restart"
    );
    assert!(harness.manager.running().is_empty());
    assert_eq!(
        harness.manager.status("one"),
        PanelConnectionStatus::Disconnected
    );
    let reports = harness.reports.lock().unwrap();
    assert!(
        reports
            .iter()
            .any(|(_, message)| { message == "restarting; reconnecting when the panel returns." })
    );
}

#[test]
fn two_records_naming_one_device_refuse_the_second() {
    let harness = harness(false, true);
    let first = record("one");
    let mut second = record("two");
    second.device = first.device.clone();

    harness
        .manager
        .start(&first)
        .expect("the first panel starts");
    let error = harness.manager.start(&second).unwrap_err();
    assert_eq!(error, "A panel display already holds that device.");
}

#[test]
fn a_disconnected_panel_is_not_reopened_by_the_retry_pass() {
    let harness = harness(false, true);
    let record = record("one");
    harness.manager.reconcile(std::slice::from_ref(&record));
    harness.manager.disconnect("one").expect("it stops");
    assert!(harness.manager.stopped("one"));

    harness.manager.reconcile(std::slice::from_ref(&record));
    assert!(harness.manager.running().is_empty());

    harness
        .manager
        .reconnect(&record)
        .expect("a manual reconnect opens it");
    assert_eq!(harness.manager.running(), vec!["one".to_owned()]);
}

#[test]
fn a_panel_with_no_device_is_refused_before_it_opens() {
    let harness = harness(false, true);
    let mut record = record("one");
    record.device = None;
    let error = harness.manager.start(&record).unwrap_err();
    assert_eq!(
        error,
        "That panel display names no device; give it one before starting it."
    );
    assert_eq!(harness.opens.load(Ordering::SeqCst), 0);
}

#[test]
fn an_unknown_family_is_refused_with_its_ids() {
    let harness = harness(false, true);
    let mut record = record("one");
    record.device = Some(PanelDeviceId {
        vendor_id: 0x1234,
        product_id: 0x5678,
        serial: String::new(),
        device_id: None,
    });
    let error = harness.manager.start(&record).unwrap_err();
    assert_eq!(error, "No driver on this host claims 1234:5678.");
}

#[test]
fn stop_all_releases_every_device_and_nothing_reopens() {
    let harness = harness(false, true);
    harness.manager.reconcile(&[record("one"), record("two")]);
    assert_eq!(harness.manager.running().len(), 2);

    harness.manager.stop_all();
    assert!(harness.manager.running().is_empty());
    harness.manager.reconcile(&[record("one"), record("two")]);
    assert!(harness.manager.running().is_empty());
    assert_eq!(harness.closes.load(Ordering::SeqCst), 2);
}
