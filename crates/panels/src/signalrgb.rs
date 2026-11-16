//! The SignalRGB seam's host half: the loopback UDP colour receiver, the
//! newest-frame rule, the lighting modes and the counters a descriptor
//! publishes.
//!
//! **It never opens hardware.** Every frame is handed to a running panel's own
//! runner, which serialises the write with its LCD commands. The bridge owns
//! only the network side and the pacing.
//!
//! **A frame is never queued.** One pending frame is kept per device; a newer
//! datagram replaces it and counts as dropped. While a write is in flight the
//! newest frame waits for the next tick, so the device is driven at its own
//! rate rather than at the sender's.

use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::family::Telemetry;
use crate::manager::SinkDevice;

pub const DEFAULT_SIGNALRGB_UDP_PORT: u16 = 7788;
/// One tick at ~30 Hz.
pub const FRAME_INTERVAL_MS: u64 = 33;
/// Repeating a forced colour costs no more than one write a second.
const FORCED_INTERVAL: Duration = Duration::from_secs(1);
/// A status reading is cached this long: the contract polls at ≤1 Hz and a
/// burst of GETs must not become a burst of `74 01` reports.
const TELEMETRY_TTL: Duration = Duration::from_secs(1);

const MAGIC: [u8; 4] = [0x59, 0x52, 0x47, 0x42];
const VERSION: u8 = 1;

/// One parsed colour datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColourFrame {
    pub device_index: usize,
    pub led_count: u32,
    /// RGB triples, one per LED, in channel order.
    pub rgb: Vec<u8>,
}

/// A refusal the seam names: the request itself is wrong, or the device is not
/// running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// The request is wrong: a bad body, mode or colour.
    Request(String),
    /// No running device answers to that id.
    NotFound(String),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(message) | Self::NotFound(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for BridgeError {}

/// What the bridge reads and writes on the panels: the manager in production,
/// a fake in tests.
pub trait Sinks: Send + Sync {
    fn describe(&self) -> Vec<SinkDevice>;
    fn write_colors(&self, id: &str, rgb: &[u8]) -> Result<(), String>;
    fn set_controls(&self, id: &str, pump: Option<f64>, fan: Option<f64>) -> Result<(), String>;
    fn telemetry(&self, id: &str) -> Result<Telemetry, String>;
}

/// Parse one `YRGB` datagram, refusing anything a sender has to fix.
pub fn parse_colour_frame(datagram: &[u8]) -> Result<ColourFrame, String> {
    if datagram.len() < 8 {
        return Err("A colour frame is at least 8 bytes.".to_owned());
    }
    if datagram[..4] != MAGIC {
        return Err("A colour frame begins with YRGB.".to_owned());
    }
    if datagram[4] != VERSION {
        return Err(format!("Unsupported colour frame version {}.", datagram[4]));
    }
    let device_index = usize::from(datagram[5]);
    let led_count = u32::from(datagram[6]) | (u32::from(datagram[7]) << 8);
    let expected = 8 + led_count as usize * 3;
    if datagram.len() != expected {
        return Err(format!(
            "A colour frame declares {led_count} LEDs and is {} bytes.",
            datagram.len()
        ));
    }
    Ok(ColourFrame {
        device_index,
        led_count,
        rgb: datagram[8..].to_vec(),
    })
}

/// `#rrggbb`, case-insensitive, into one RGB triple.
pub fn parse_hex_colour(value: &Value) -> Result<[u8; 3], BridgeError> {
    let Some(text) = value.as_str() else {
        return Err(BridgeError::Request(
            "The colour must be #rrggbb.".to_owned(),
        ));
    };
    let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6);
    let Some(hex) = hex else {
        return Err(BridgeError::Request(
            "The colour must be #rrggbb.".to_owned(),
        ));
    };
    let channel = |from: usize| u8::from_str_radix(&hex[from..from + 2], 16);
    match (channel(0), channel(2), channel(4)) {
        (Ok(red), Ok(green), Ok(blue)) => Ok([red, green, blue]),
        _ => Err(BridgeError::Request(
            "The colour must be #rrggbb.".to_owned(),
        )),
    }
}

fn solid_frame(colour: [u8; 3], led_count: u32) -> Vec<u8> {
    let mut frame = Vec::with_capacity(led_count as usize * 3);
    for _ in 0..led_count {
        frame.extend_from_slice(&colour);
    }
    frame
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Canvas,
    Forced,
    Off,
}

struct DeviceState {
    pending: Option<Vec<u8>>,
    mode: Mode,
    colour: Option<[u8; 3]>,
    last_forced: Option<Instant>,
    received: u64,
    dropped: u64,
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            pending: None,
            mode: Mode::Canvas,
            colour: None,
            last_forced: None,
            received: 0,
            dropped: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Stats {
    /// Valid frames that arrived, including ones that named no device.
    received: u64,
    /// Valid frames that were not written, including ones naming no device.
    dropped: u64,
    /// Datagrams the parser refused.
    malformed: u64,
}

struct Inner {
    sinks: Arc<dyn Sinks>,
    port: u16,
    states: Mutex<HashMap<String, DeviceState>>,
    stats: Mutex<Stats>,
    telemetry: Mutex<HashMap<String, (Telemetry, Instant)>>,
    stop: AtomicBool,
}

pub struct SignalRgbBridge {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SignalRgbBridge {
    /// Binds the loopback receiver and starts its tick. An `error` callback is
    /// told about failures after binding; a bind failure is the `Err`.
    pub fn start(
        port: u16,
        sinks: Arc<dyn Sinks>,
        on_error: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let socket = UdpSocket::bind(("127.0.0.1", port))
            .map_err(|error| format!("The bridge could not listen on 127.0.0.1:{port}: {error}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(FRAME_INTERVAL_MS)))
            .map_err(|error| error.to_string())?;
        let bound = socket
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();

        let inner = Arc::new(Inner {
            sinks,
            port: bound,
            states: Mutex::new(HashMap::new()),
            stats: Mutex::new(Stats::default()),
            telemetry: Mutex::new(HashMap::new()),
            stop: AtomicBool::new(false),
        });
        let bridge = Self {
            inner: Arc::clone(&inner),
            thread: Mutex::new(None),
        };

        let ticker = Arc::clone(&inner);
        let report: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(on_error);
        let thread = std::thread::Builder::new()
            .name("signalrgb".to_owned())
            .spawn(move || tick(&ticker, &socket, &report))
            .map_err(|error| format!("The bridge's receiver could not start: {error}"))?;
        *bridge.thread.lock().expect("the bridge thread") = Some(thread);
        Ok(bridge)
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    /// The device descriptor, exactly as the seam's contract states it.
    pub fn describe(&self) -> Value {
        let devices = self.inner.sinks.describe();
        let states = self.inner.states.lock().expect("the bridge states");
        let stats = *self.inner.stats.lock().expect("the bridge stats");
        let rows: Vec<Value> = devices
            .iter()
            .map(|device| {
                let state = states.get(&device.id);
                let mut row = json!({
                    "id": device.id,
                    "kind": device.kind,
                    "name": device.name,
                });
                if let Some(lighting) = &device.lighting {
                    row["lighting"] = json!({
                        "ledCount": lighting.led_count,
                        "udpPort": self.inner.port,
                        "channels": lighting.channels,
                        "layout": lighting.layout,
                        "stats": {
                            "received": state.map_or(0, |state| state.received),
                            "dropped": state.map_or(0, |state| state.dropped),
                        },
                    });
                }
                if let Some(controls) = &device.controls {
                    row["controls"] = json!({
                        "pump": controls.pump,
                        "fans": controls.fans,
                        "telemetry": controls.telemetry,
                    });
                }
                row["display"] = json!({
                    "kind": "panel",
                    "width": device.display.width,
                    "height": device.display.height,
                });
                row
            })
            .collect();

        json!({
            "hub": { "name": "Yunari", "vendor": "Yunari" },
            "devices": rows,
            "stats": {
                "received": stats.received,
                "malformed": stats.malformed,
                "dropped": stats.dropped,
            },
        })
    }

    /// Writes pump/fan percentages through the device's own clamp rules.
    pub fn controls(&self, id: &str, input: &Value) -> Result<(), BridgeError> {
        let device = self.device(id)?;
        if device.controls.is_none() {
            return Err(BridgeError::NotFound(format!(
                "Device {id} accepts no controls."
            )));
        }
        let pump = input.get("pumpPercent").map(percent_of).transpose()?;
        let fan = input.get("fanPercent").map(percent_of).transpose()?;
        if pump.is_none() && fan.is_none() {
            return Err(BridgeError::Request(
                "A controls request needs pumpPercent or fanPercent.".to_owned(),
            ));
        }
        self.inner
            .sinks
            .set_controls(id, pump, fan)
            .map_err(BridgeError::Request)
    }

    /// What the device is doing, cached for the contract's poll rate.
    pub fn telemetry(&self, id: &str) -> Result<Telemetry, BridgeError> {
        let device = self.device(id)?;
        if device.controls.is_none() {
            return Err(BridgeError::NotFound(format!(
                "Device {id} reports no telemetry."
            )));
        }
        {
            let cache = self.inner.telemetry.lock().expect("the bridge cache");
            if let Some((value, at)) = cache.get(id)
                && at.elapsed() < TELEMETRY_TTL
            {
                return Ok(*value);
            }
        }
        let value = self
            .inner
            .sinks
            .telemetry(id)
            .map_err(BridgeError::Request)?;
        self.inner
            .telemetry
            .lock()
            .expect("the bridge cache")
            .insert(id.to_owned(), (value, Instant::now()));
        Ok(value)
    }

    pub fn lighting_mode(&self, id: &str, input: &Value) -> Result<(), BridgeError> {
        let device = self.device(id)?;
        let Some(lighting) = device.lighting else {
            return Err(BridgeError::NotFound(format!(
                "Device {id} has no lighting."
            )));
        };
        let mode = match input.get("mode").and_then(Value::as_str) {
            Some("canvas") => Mode::Canvas,
            Some("forced") => Mode::Forced,
            Some("off") => Mode::Off,
            _ => {
                return Err(BridgeError::Request(
                    "The lighting mode must be canvas, forced or off.".to_owned(),
                ));
            }
        };
        let colour = match mode {
            Mode::Forced => Some(parse_hex_colour(
                input.get("color").unwrap_or(&Value::Null),
            )?),
            _ => None,
        };

        let mut states = self.inner.states.lock().expect("the bridge states");
        let state = states.entry(id.to_owned()).or_default();
        state.mode = mode;
        state.pending = None;
        state.colour = colour;
        state.last_forced = None;
        drop(states);

        match mode {
            Mode::Canvas => Ok(()),
            Mode::Off => self
                .inner
                .sinks
                .write_colors(id, &solid_frame([0, 0, 0], lighting.led_count))
                .map_err(BridgeError::Request),
            Mode::Forced => {
                let colour = colour.expect("a forced colour was parsed");
                self.inner
                    .sinks
                    .write_colors(id, &solid_frame(colour, lighting.led_count))
                    .map_err(BridgeError::Request)?;
                if let Some(state) = self
                    .inner
                    .states
                    .lock()
                    .expect("the bridge states")
                    .get_mut(id)
                {
                    state.last_forced = Some(Instant::now());
                }
                Ok(())
            }
        }
    }

    fn device(&self, id: &str) -> Result<SinkDevice, BridgeError> {
        self.inner
            .sinks
            .describe()
            .into_iter()
            .find(|device| device.id == id)
            .ok_or_else(|| BridgeError::NotFound(format!("No device {id} is open.")))
    }

    /// Stops the receiver and waits for its tick to finish.
    pub fn close(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.lock().expect("the bridge thread").take() {
            thread.join().ok();
        }
    }
}

impl Drop for SignalRgbBridge {
    fn drop(&mut self) {
        // The receiver thread would otherwise outlive its bridge.
        self.inner.stop.store(true, Ordering::SeqCst);
    }
}

fn percent_of(value: &Value) -> Result<f64, BridgeError> {
    match value.as_f64() {
        Some(number) if number.is_finite() => Ok(number),
        _ => Err(BridgeError::Request(
            "A pump or fan percent must be a number from 0 to 100.".to_owned(),
        )),
    }
}

fn tick(inner: &Arc<Inner>, socket: &UdpSocket, report: &Arc<dyn Fn(&str) + Send + Sync>) {
    let mut datagram = [0_u8; 4096];
    while !inner.stop.load(Ordering::SeqCst) {
        // Drain what arrived, keeping the newest frame per device.
        while let Ok(size) = socket.recv(&mut datagram) {
            receive(inner, &datagram[..size]);
        }
        flush(inner, report);
    }
}

fn receive(inner: &Arc<Inner>, datagram: &[u8]) {
    let Ok(frame) = parse_colour_frame(datagram) else {
        inner.stats.lock().expect("the bridge stats").malformed += 1;
        return;
    };
    inner.stats.lock().expect("the bridge stats").received += 1;

    let devices = inner.sinks.describe();
    let Some(device) = devices.get(frame.device_index) else {
        inner.stats.lock().expect("the bridge stats").dropped += 1;
        return;
    };
    let Some(lighting) = &device.lighting else {
        inner.stats.lock().expect("the bridge stats").dropped += 1;
        return;
    };
    if lighting.led_count != frame.led_count {
        inner.stats.lock().expect("the bridge stats").dropped += 1;
        return;
    }

    let mut states = inner.states.lock().expect("the bridge states");
    let state = states.entry(device.id.clone()).or_default();
    state.received += 1;
    if state.mode != Mode::Canvas {
        state.dropped += 1;
        inner.stats.lock().expect("the bridge stats").dropped += 1;
        return;
    }
    if state.pending.is_some() {
        state.dropped += 1;
        inner.stats.lock().expect("the bridge stats").dropped += 1;
    }
    state.pending = Some(frame.rgb);
}

fn flush(inner: &Arc<Inner>, report: &Arc<dyn Fn(&str) + Send + Sync>) {
    let devices = inner.sinks.describe();
    let now = Instant::now();

    // Pending canvas frames, each written once.
    let pending: Vec<(String, Vec<u8>)> = {
        let mut states = inner.states.lock().expect("the bridge states");
        states
            .iter_mut()
            .filter_map(|(id, state)| state.pending.take().map(|frame| (id.clone(), frame)))
            .collect()
    };
    for (id, frame) in pending {
        if let Err(error) = inner.sinks.write_colors(&id, &frame) {
            report(&format!("A lighting frame could not be written: {error}"));
        }
    }

    // A forced colour is repeated at the contract's one-second cadence.
    let forced: Vec<(String, [u8; 3], u32)> = {
        let mut states = inner.states.lock().expect("the bridge states");
        states
            .iter_mut()
            .filter_map(|(id, state)| {
                if state.mode != Mode::Forced {
                    return None;
                }
                if state
                    .last_forced
                    .is_some_and(|at| now.duration_since(at) < FORCED_INTERVAL)
                {
                    return None;
                }
                let colour = state.colour?;
                state.last_forced = Some(now);
                let leds = devices
                    .iter()
                    .find(|device| &device.id == id)?
                    .lighting
                    .as_ref()?
                    .led_count;
                Some((id.clone(), colour, leds))
            })
            .collect()
    };
    for (id, colour, leds) in forced {
        if let Err(error) = inner.sinks.write_colors(&id, &solid_frame(colour, leds)) {
            report(&format!("A forced colour could not be written: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::{Controls, Geometry, Lighting, LightingChannel, LightingLayout};
    use std::sync::atomic::AtomicUsize;

    const TEST_CHANNELS: [LightingChannel; 1] = [LightingChannel {
        name: "Test",
        leds: 4,
    }];

    fn lighting(leds: u32) -> Lighting {
        Lighting {
            led_count: leds,
            channels: &TEST_CHANNELS,
            layout: LightingLayout {
                shape: "rings",
                width: 1,
                height: 1,
                count: 1,
            },
        }
    }

    struct FakeSinks {
        writes: Mutex<Vec<(String, Vec<u8>)>>,
        colours: AtomicUsize,
    }

    impl Sinks for FakeSinks {
        fn describe(&self) -> Vec<SinkDevice> {
            vec![SinkDevice {
                id: "kraken".to_owned(),
                kind: "kraken-z3".to_owned(),
                name: "NZXT Kraken Z".to_owned(),
                lighting: Some(lighting(4)),
                controls: Some(Controls {
                    pump: true,
                    fans: true,
                    telemetry: true,
                }),
                display: Geometry {
                    width: 320,
                    height: 320,
                },
            }]
        }

        fn write_colors(&self, id: &str, rgb: &[u8]) -> Result<(), String> {
            self.writes
                .lock()
                .unwrap()
                .push((id.to_owned(), rgb.to_vec()));
            Ok(())
        }

        fn set_controls(
            &self,
            _id: &str,
            _pump: Option<f64>,
            _fan: Option<f64>,
        ) -> Result<(), String> {
            Ok(())
        }

        fn telemetry(&self, _id: &str) -> Result<Telemetry, String> {
            self.colours.fetch_add(1, Ordering::SeqCst);
            Ok(Telemetry {
                liquid_temp_c: 30.0,
                pump_rpm: 1,
                pump_percent: 2,
                fan_rpm: 3,
                fan_percent: 4,
            })
        }
    }

    fn frame(index: u8, leds: u16, rgb: &[u8]) -> Vec<u8> {
        let mut datagram = vec![0x59, 0x52, 0x47, 0x42, 1, index];
        datagram.extend_from_slice(&leds.to_le_bytes());
        datagram.extend_from_slice(rgb);
        datagram
    }

    #[test]
    fn a_frame_is_checked_before_it_is_read() {
        let parsed = parse_colour_frame(&frame(0, 2, &[1, 2, 3, 4, 5, 6])).expect("a frame");
        assert_eq!(parsed.device_index, 0);
        assert_eq!(parsed.led_count, 2);
        assert_eq!(parsed.rgb, vec![1, 2, 3, 4, 5, 6]);
        assert!(parse_colour_frame(&[0x59, 0x52]).is_err());
        assert!(parse_colour_frame(b"XXXX\x01\x00\x00\x00").is_err());
        assert!(parse_colour_frame(&frame(0, 2, &[1, 2, 3])).is_err());
    }

    #[test]
    fn the_descriptor_publishes_the_port_and_the_counters() {
        let sinks = Arc::new(FakeSinks {
            writes: Mutex::new(Vec::new()),
            colours: AtomicUsize::new(0),
        });
        let bridge = SignalRgbBridge::start(0, sinks, |_| {}).expect("a bridge");
        let descriptor = bridge.describe();
        assert_eq!(descriptor["hub"]["name"], "Yunari");
        assert_eq!(descriptor["devices"][0]["lighting"]["ledCount"], 4);
        assert_eq!(
            descriptor["devices"][0]["lighting"]["udpPort"],
            bridge.port()
        );
        assert_eq!(descriptor["devices"][0]["display"]["width"], 320);
        assert_eq!(descriptor["stats"]["received"], 0);
        bridge.close();
    }

    #[test]
    fn a_sender_frame_reaches_the_device_and_a_stale_one_is_dropped() {
        let sinks = Arc::new(FakeSinks {
            writes: Mutex::new(Vec::new()),
            colours: AtomicUsize::new(0),
        });
        let bridge = SignalRgbBridge::start(0, Arc::clone(&sinks) as Arc<dyn Sinks>, |_| {})
            .expect("a bridge");
        let socket = UdpSocket::bind(("127.0.0.1", 0)).expect("a sender");
        // Two frames back to back: the second replaces the first.
        socket
            .send_to(
                &frame(0, 4, &[1, 2, 3].repeat(4)),
                ("127.0.0.1", bridge.port()),
            )
            .expect("a frame");
        socket
            .send_to(
                &frame(0, 4, &[9, 9, 9].repeat(4)),
                ("127.0.0.1", bridge.port()),
            )
            .expect("a frame");

        // The tick runs at 30 Hz; one pass writes the newest frame only.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && sinks.writes.lock().expect("the writes").is_empty() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let writes = sinks.writes.lock().expect("the writes");
        let (id, rgb) = writes.first().expect("one write");
        assert_eq!(id, "kraken");
        assert_eq!(rgb, &[9, 9, 9].repeat(4));
        bridge.close();
    }

    #[test]
    fn a_mode_other_than_canvas_refuses_sender_frames() {
        let sinks = Arc::new(FakeSinks {
            writes: Mutex::new(Vec::new()),
            colours: AtomicUsize::new(0),
        });
        let bridge = SignalRgbBridge::start(0, Arc::clone(&sinks) as Arc<dyn Sinks>, |_| {})
            .expect("a bridge");
        bridge
            .lighting_mode("kraken", &json!({ "mode": "forced", "color": "#112233" }))
            .expect("a forced colour");
        // The forced write is one solid frame.
        assert_eq!(
            sinks.writes.lock().unwrap().first().unwrap().1,
            [0x11, 0x22, 0x33].repeat(4)
        );
        let refused = bridge.lighting_mode("kraken", &json!({ "mode": "x" }));
        assert!(matches!(refused, Err(BridgeError::Request(_))));
        let missing = bridge.lighting_mode("none", &json!({ "mode": "off" }));
        assert!(matches!(missing, Err(BridgeError::NotFound(_))));
        bridge.close();
    }

    #[test]
    fn telemetry_is_cached_for_the_contract_s_poll_rate() {
        let sinks = Arc::new(FakeSinks {
            writes: Mutex::new(Vec::new()),
            colours: AtomicUsize::new(0),
        });
        let bridge = SignalRgbBridge::start(0, Arc::clone(&sinks) as Arc<dyn Sinks>, |_| {})
            .expect("a bridge");
        assert_eq!(bridge.telemetry("kraken").unwrap().liquid_temp_c, 30.0);
        assert_eq!(bridge.telemetry("kraken").unwrap().liquid_temp_c, 30.0);
        assert_eq!(sinks.colours.load(Ordering::SeqCst), 1);
        bridge.close();
    }
}
