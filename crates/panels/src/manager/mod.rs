//! The panels this host is driving: one runner per display record, the
//! controls that reach it, and the retry pass that reopens a panel which came
//! back.
//!
//! Everything here is synchronous and lock-short: the map is read under the
//! manager's lock and every runner call happens after it is released, so a slow
//! panel can never hold the settings surface still.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::family::{CommandKind, PanelCommand, PanelDevice, PanelFamily, for_device};
use crate::open_device;
use crate::runner::{Capture, Runner};

/// What identifies one physical panel, for a record and for finding it again
/// after a reboot. A bus address moves; the serial does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelDeviceId {
    pub vendor_id: u16,
    pub product_id: u16,
    /// Empty when the firmware reports none — the address is then all there is.
    #[serde(default)]
    pub serial: String,
    /// The OS instance id, for an action that has to name the device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

/// What the manager is doing with one display, in the vocabulary its port and
/// the settings surface share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelConnectionStatus {
    Connected,
    Connecting,
    Disconnected,
}

impl PanelConnectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Connecting => "connecting",
            Self::Disconnected => "disconnected",
        }
    }
}

/// A control the manager refused, in the manager's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelManagerError {
    NotRunning(String),
    NoCommands(String),
    NoDriver(u16, u16),
    NoDevice,
    /// Another record already drives this device.
    Held,
}

impl std::fmt::Display for PanelManagerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning(message) | Self::NoCommands(message) => formatter.write_str(message),
            Self::NoDriver(vendor, product) => {
                write!(
                    formatter,
                    "No driver on this host claims {vendor:04x}:{product:04x}."
                )
            }
            Self::NoDevice => formatter
                .write_str("That panel display names no device; give it one before starting it."),
            Self::Held => formatter.write_str("A panel display already holds that device."),
        }
    }
}

impl std::error::Error for PanelManagerError {}

/// One encoded still for a record's panel, from its own geometry.
pub type FrameSource =
    Arc<dyn Fn(&DisplayRecord, u32, u32) -> Result<Vec<u8>, String> + Send + Sync>;

/// How a panel device is opened. The seam keeps hardware out of tests.
pub type OpenDevice =
    Box<dyn Fn(&PanelFamily) -> Result<Box<dyn PanelDevice>, String> + Send + Sync>;

/// One panel fact, with the display it belongs to.
pub type Report = Box<dyn Fn(&str, &str) + Send + Sync>;

/// The part of a display record the manager needs. The host's record satisfies
/// it, so the manager never depends on the host's storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayRecord {
    pub id: String,
    pub label: String,
    /// True when this record drives a panel.
    pub panel: bool,
    pub device: Option<PanelDeviceId>,
}

struct Entry {
    runner: Runner<Box<dyn PanelDevice>>,
    device: PanelDeviceId,
}

struct Inner {
    entries: HashMap<String, Arc<Entry>>,
    /// Displays the owner disconnected: the retry pass skips them.
    disconnected: HashSet<String>,
    /// The last message reported for each display, so a retry loop that keeps
    /// failing the same way says so once rather than every pass.
    last_reported: HashMap<String, String>,
    stopping: bool,
}

pub struct Manager {
    inner: Mutex<Inner>,
    open: OpenDevice,
    frame_source: FrameSource,
    on_error: Report,
}

impl Manager {
    pub fn new(frame_source: FrameSource) -> Self {
        Self::with_device_source(frame_source, Box::new(open_device))
    }

    pub fn with_device_source(frame_source: FrameSource, open: OpenDevice) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                disconnected: HashSet::new(),
                last_reported: HashMap::new(),
                stopping: false,
            }),
            open,
            frame_source,
            on_error: Box::new(|_, _| {}),
        }
    }

    pub fn on_error(mut self, report: impl Fn(&str, &str) + Send + Sync + 'static) -> Self {
        self.on_error = Box::new(report);
        self
    }

    /// Logs one panel fact, skipping the same sentence twice in a row.
    ///
    /// The retry loop earns this: an unplugged panel fails identically every
    /// pass for as long as it stays unplugged, and a line per pass would bury
    /// the change that matters. A different message is always written.
    fn report(&self, display: &str, message: &str) {
        let mut inner = self.inner.lock().expect("the manager");
        if inner
            .last_reported
            .get(display)
            .is_some_and(|last| last == message)
        {
            return;
        }
        inner
            .last_reported
            .insert(display.to_owned(), message.to_owned());
        drop(inner);
        (self.on_error)(display, message);
    }

    fn entry(&self, display: &str) -> Option<Arc<Entry>> {
        self.inner
            .lock()
            .expect("the manager")
            .entries
            .get(display)
            .cloned()
    }

    /// Starts a display's runner, or does nothing when one is already running.
    pub fn start(&self, record: &DisplayRecord) -> Result<(), String> {
        let device = record
            .device
            .clone()
            .ok_or_else(|| PanelManagerError::NoDevice.to_string())?;
        let family = for_device(device.vendor_id, device.product_id).ok_or_else(|| {
            PanelManagerError::NoDriver(device.vendor_id, device.product_id).to_string()
        })?;

        {
            let inner = self.inner.lock().expect("the manager");
            if inner.stopping || inner.entries.contains_key(&record.id) {
                return Ok(());
            }
            if inner.entries.values().any(|entry| entry.device == device) {
                return Err(PanelManagerError::Held.to_string());
            }
        }

        let mut panel = (self.open)(family)?;
        panel.open()?;
        let interval = panel.frame_interval_ms();
        let capture: Capture = {
            let source = Arc::clone(&self.frame_source);
            let record = record.clone();
            Box::new(move |width, height| source(&record, width, height))
        };
        let entry = Arc::new(Entry {
            runner: Runner::new(panel, capture, interval),
            device,
        });

        let mut inner = self.inner.lock().expect("the manager");
        // A stop or shutdown that landed while the device was opening owns it
        // now: nothing would ever tick or close this runner if it were
        // installed.
        if inner.stopping || inner.entries.contains_key(&record.id) {
            drop(inner);
            entry.runner.stop().ok();
            return Ok(());
        }
        inner.entries.insert(record.id.clone(), entry);
        // A runner is connected: the next failure of any kind is news again.
        inner.last_reported.remove(&record.id);
        Ok(())
    }

    /// Stops a display's runner, answering once the device is released. A
    /// record removed without this leaves it driving.
    pub fn stop(&self, display: &str) -> Result<(), String> {
        let entry = self
            .inner
            .lock()
            .expect("the manager")
            .entries
            .remove(display);
        match entry {
            Some(entry) => entry.runner.stop(),
            None => Ok(()),
        }
    }

    /// Sends one hardware command to a display's running device.
    ///
    /// A display with no runner is refused rather than opened or queued for
    /// later: starting a panel is the owner's decision, and a command that
    /// waited for a start could reach hardware the caller never asked to drive.
    pub fn command(
        &self,
        display: &str,
        command: PanelCommand,
    ) -> Result<String, PanelManagerError> {
        let Some(entry) = self.entry(display) else {
            return Err(PanelManagerError::NotRunning(format!(
                "Panel display {display} is not running; start it before sending a command."
            )));
        };
        // A runner whose stop already began is `running() == false`: its port is
        // on the way down, so a write now is a race with the close.
        if !entry.runner.running() {
            return Err(PanelManagerError::NotRunning(format!(
                "Panel display {display} is not running; start it before sending a command."
            )));
        }
        if !entry.runner.accepts(command.kind()) {
            return Err(PanelManagerError::NoCommands(format!(
                "Panel display {display} does not accept commands."
            )));
        }

        let restart = command.kind() == CommandKind::Restart;
        let reply = entry
            .runner
            .control(command)
            .map_err(PanelManagerError::NotRunning)?;

        if restart {
            // **A restart is a planned disconnection.** The interface drops for
            // about fifteen seconds, so writing to this port again is writing
            // to a stream the device has already torn down. Detach on purpose —
            // even when the write failed — and let the retry pass open the
            // panel when it comes back.
            self.stop(display).ok();
            self.report(display, "restarting; reconnecting when the panel returns.");
        }
        Ok(reply)
    }

    /// What this display's device is doing, for a surface that reports it.
    pub fn status(&self, display: &str) -> PanelConnectionStatus {
        let inner = self.inner.lock().expect("the manager");
        if inner.disconnected.contains(display) {
            return PanelConnectionStatus::Disconnected;
        }
        match inner.entries.get(display) {
            Some(entry) if entry.runner.running() => PanelConnectionStatus::Connected,
            _ => PanelConnectionStatus::Disconnected,
        }
    }

    /// One tick for every running panel. Answers how many frames went out.
    pub fn tick(&self) -> usize {
        let entries: Vec<(String, Arc<Entry>)> = {
            let inner = self.inner.lock().expect("the manager");
            inner
                .entries
                .iter()
                .map(|(id, entry)| (id.clone(), Arc::clone(entry)))
                .collect()
        };
        let mut sent = 0;
        for (id, entry) in entries {
            if entry.runner.tick() {
                sent += 1;
            }
            if !entry.runner.running() {
                // The device refused three frames in a row: remove this runner
                // and let the retry pass open the panel again.
                self.stop(&id).ok();
                self.report(&id, "disconnected; retrying.");
            }
        }
        sent
    }

    /// Every attached panel that has a record and no runner yet, and the end of
    /// any runner whose record vanished.
    pub fn reconcile(&self, records: &[DisplayRecord]) {
        let running: Vec<String> = {
            let inner = self.inner.lock().expect("the manager");
            inner.entries.keys().cloned().collect()
        };
        for id in running {
            if !records.iter().any(|record| record.id == id) {
                self.stop(&id).ok();
            }
        }

        for record in records.iter().filter(|record| record.panel) {
            let skip = {
                let inner = self.inner.lock().expect("the manager");
                inner.stopping
                    || inner.disconnected.contains(&record.id)
                    || inner.entries.contains_key(&record.id)
            };
            if skip {
                continue;
            }
            if let Err(message) = self.start(record) {
                self.report(&record.id, &message);
            }
        }
    }

    /// Stops the panel and suppresses its automatic reopen until this process
    /// exits. A manual reconnect or a start clears the suppression.
    pub fn disconnect(&self, display: &str) -> Result<(), String> {
        self.stop(display)?;
        self.inner
            .lock()
            .expect("the manager")
            .disconnected
            .insert(display.to_owned());
        Ok(())
    }

    /// Stops and starts one display now: the manual retake.
    pub fn reconnect(&self, record: &DisplayRecord) -> Result<(), String> {
        {
            let mut inner = self.inner.lock().expect("the manager");
            inner.disconnected.remove(&record.id);
            inner.last_reported.remove(&record.id);
        }
        self.stop(&record.id)?;
        self.start(record)
    }

    /// Whether a disconnect, not a missing device, keeps this panel stopped.
    pub fn stopped(&self, display: &str) -> bool {
        self.inner
            .lock()
            .expect("the manager")
            .disconnected
            .contains(display)
    }

    /// The displays a runner is installed for.
    pub fn running(&self) -> Vec<String> {
        let inner = self.inner.lock().expect("the manager");
        inner.entries.keys().cloned().collect()
    }

    pub fn stop_all(&self) {
        self.inner.lock().expect("the manager").stopping = true;
        let ids = self.running();
        for id in ids {
            self.stop(&id).ok();
        }
    }
}

#[cfg(test)]
mod tests;
