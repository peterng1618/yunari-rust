//! The TURZX 5" serial panel: a USB CDC gadget that takes a full BGRA frame per
//! update, not a JPEG stream.

use std::time::{Duration, Instant};

use super::protocol::{self, command, frame, parse_hello_text, parse_panel_status};
use crate::family::{CommandKind, Geometry, PanelCommand, PanelDevice, StillFormat};

pub const LABEL: &str = "TURZX 5\" Panel";
/// The ids the family's firmware reports. They are the generic Linux gadget
/// pair, so they do not *identify* a panel — HELLO does — but they are what a
/// port must report to be a candidate at all.
pub const VENDOR_ID: u16 = 0x1d6b;
pub const PRODUCT_ID: u16 = 0x0106;
/// Long enough for a 23-byte HELLO reply at 115200, short enough to probe many.
const PROBE_TIMEOUT_MS: u64 = 500;
const REPLY_TIMEOUT_MS: u64 = 2_000;
const REPLY_DEADLINE_MS: u64 = 30_000;

/// The bytes a serial port carries, as the driver needs them.
pub trait Serial: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Every byte available within `timeout_ms`; empty when the port said
    /// nothing.
    fn read(&mut self, timeout_ms: u64) -> Result<Vec<u8>, String>;
    /// Whatever is already queued, dropped: a stale ack must not be read as
    /// this command's answer.
    fn drain(&mut self);
    fn close(&mut self) -> Result<(), String>;
}

pub struct TurzxPanel<S: Serial> {
    serial: S,
    geometry: Geometry,
    opened: bool,
    /// The brightness byte this host last wrote, full by default. The settings
    /// packet carries it and the panel offers no read-back, so a mode or flip
    /// change would otherwise rewrite the level with a value nobody chose.
    brightness: u8,
    /// True between a screen-off and a screen-on, so every later brightness
    /// write stays dark until the owner asks for light again.
    screen_off: bool,
}

impl<S: Serial> TurzxPanel<S> {
    pub fn new(serial: S, geometry: Geometry) -> Self {
        Self {
            serial,
            geometry,
            opened: false,
            brightness: 255,
            screen_off: false,
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let framed = frame(bytes);
        debug_assert_eq!(framed.len() % protocol::FRAME_SIZE, 0);
        self.serial.write(&framed)
    }

    /// The operations that make an already-identifying panel ready: its clip
    /// and media stopped, and one status query so the first reply read by a
    /// control is not a stale ack.
    fn prepare(&mut self) -> Result<(), String> {
        self.send(command::STOP_VIDEO)?;
        self.send(command::STOP_MEDIA)?;
        self.send(command::QUERY_STATUS)?;
        if self.serial.read(REPLY_TIMEOUT_MS)?.is_empty() {
            return Err(not_answering("QUERY_STATUS was not answered."));
        }
        Ok(())
    }

    /// The first reply `accept` takes, skipping stale acks; the empty read or
    /// the deadline ends the wait.
    fn read_until(&mut self, accepted: impl Fn(&str) -> bool) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_millis(REPLY_DEADLINE_MS);
        loop {
            if Instant::now() >= deadline {
                return Err("The panel did not answer before the read deadline.".to_owned());
            }
            let reply = protocol::decode_reply(&self.serial.read(REPLY_TIMEOUT_MS)?);
            if reply.is_empty() {
                return Err("The panel did not answer.".to_owned());
            }
            if accepted(&reply) {
                return Ok(reply);
            }
        }
    }

    /// The brightness byte the panel should be showing right now.
    fn brightness_byte(&self) -> u8 {
        if self.screen_off { 0 } else { self.brightness }
    }

    fn set_brightness(&mut self, weight: u8) -> Result<(), String> {
        let mut write = command::BRIGHTNESS.to_vec();
        write.push(weight);
        self.send(&write)
    }
}

impl<S: Serial> PanelDevice for TurzxPanel<S> {
    fn geometry(&self) -> Geometry {
        self.geometry
    }

    fn open(&mut self) -> Result<(), String> {
        self.send(command::HELLO)?;
        let reply = self.serial.read(PROBE_TIMEOUT_MS)?;
        if parse_hello_text(String::from_utf8_lossy(&reply).trim_end_matches('\0')).is_none() {
            return Err(not_answering("HELLO was not answered."));
        }
        self.prepare()?;
        self.opened = true;
        Ok(())
    }

    /// Raw RGBA of the panel's own geometry: the still is reordered to BGRA and
    /// chunked on the wire, so a lossy intermediate would be loss for nothing.
    fn present(&mut self, still: &[u8]) -> Result<(), String> {
        if !self.opened {
            return Err(format!("{LABEL} is not open."));
        }
        let expected = self.geometry.width as usize * self.geometry.height as usize * 4;
        if still.len() != expected {
            return Err(format!(
                "A {}-byte frame is not a {}x{} RGBA still ({expected} bytes).",
                still.len(),
                self.geometry.width,
                self.geometry.height
            ));
        }
        self.send(command::PRE_UPDATE)?;
        self.send(&command::start_display_bitmap())?;
        self.send(command::DISPLAY_BITMAP_5INCH)?;
        let payload = protocol::bgra_payload(still);
        self.serial.write(&payload)
    }

    fn still_format(&self) -> StillFormat {
        StillFormat::Rgba
    }

    fn accepts(&self, kind: CommandKind) -> bool {
        matches!(
            kind,
            CommandKind::Brightness
                | CommandKind::Screen
                | CommandKind::PanelSettings
                | CommandKind::Restart
                | CommandKind::Status
                | CommandKind::Hello
        )
    }

    fn run(&mut self, command: PanelCommand) -> Result<String, String> {
        match command {
            PanelCommand::Brightness(level) => {
                if level > 100 {
                    return Err(format!(
                        "The brightness must be from 0 to 100; received {level}."
                    ));
                }
                let weight = ((u32::from(level) * 255 + 50) / 100) as u8;
                self.set_brightness(weight)?;
                // Asking for a level is also asking for the screen back.
                self.brightness = weight;
                self.screen_off = false;
                Ok(String::new())
            }
            PanelCommand::Screen(on) => {
                self.screen_off = !on;
                let byte = self.brightness_byte();
                self.set_brightness(byte)?;
                Ok(String::new())
            }
            PanelCommand::PanelSettings {
                start_mode,
                flip,
                sleep_delay,
            } => {
                if start_mode > 2 {
                    return Err(format!(
                        "The start mode must be 0, 1 or 2; received {start_mode}."
                    ));
                }
                if sleep_delay > 10 {
                    return Err(format!(
                        "The sleep delay must be a whole number from 0 to 10; received {sleep_delay}."
                    ));
                }
                let mut write = command::PANEL_SETTINGS.to_vec();
                write.extend_from_slice(&[
                    self.brightness_byte(),
                    start_mode,
                    0,
                    u8::from(flip),
                    sleep_delay,
                ]);
                self.send(&write)?;
                Ok(String::new())
            }
            PanelCommand::Restart => {
                self.send(command::RESTART)?;
                Ok(String::new())
            }
            PanelCommand::Status => {
                self.serial.drain();
                self.send(command::QUERY_STATUS)?;
                self.read_until(|text| parse_panel_status(text).is_some())
            }
            PanelCommand::Hello => {
                self.serial.drain();
                self.send(command::HELLO)?;
                self.read_until(|text| parse_hello_text(text).is_some())
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
        self.opened = false;
        self.serial.close()
    }
}

/// A device that opened and then stopped taking data is neither absent nor held
/// by another program: it needs its USB node reset.
pub fn not_answering(detail: &str) -> String {
    format!(
        "{LABEL} is attached but did not answer: {detail} Unplug and replug it, or restart its USB device."
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    const GEOMETRY: Geometry = Geometry {
        width: 800,
        height: 480,
    };

    struct FakeSerial {
        writes: Vec<Vec<u8>>,
        /// Acks already sitting in the port: `drain` drops these and nothing
        /// else, because a control must not read them as its own answer.
        stale: VecDeque<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
        closed: usize,
    }

    impl FakeSerial {
        fn new(replies: &[&[u8]]) -> Self {
            Self {
                writes: Vec::new(),
                stale: VecDeque::new(),
                replies: replies.iter().map(|reply| reply.to_vec()).collect(),
                closed: 0,
            }
        }

        fn assert_frames(&self) {
            for write in &self.writes {
                assert_eq!(
                    write.len() % protocol::FRAME_SIZE,
                    0,
                    "a {}-byte write is not frame aligned",
                    write.len()
                );
            }
        }
    }

    impl Serial for FakeSerial {
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        fn read(&mut self, _timeout_ms: u64) -> Result<Vec<u8>, String> {
            Ok(self
                .stale
                .pop_front()
                .or_else(|| self.replies.pop_front())
                .unwrap_or_default())
        }

        fn drain(&mut self) {
            self.stale.clear();
        }

        fn close(&mut self) -> Result<(), String> {
            self.closed += 1;
            Ok(())
        }
    }

    fn opened(replies: &[&[u8]]) -> TurzxPanel<FakeSerial> {
        let mut panel = TurzxPanel::new(FakeSerial::new(replies), GEOMETRY);
        panel.open().expect("the panel opens");
        panel
    }

    #[test]
    fn an_open_announces_itself_and_stops_whatever_was_playing() {
        let panel = opened(&[
            b"chs_5inch.dev1_rom1.88\0\0\0",
            b"needReSend:0|renderCnt:1\0",
        ]);
        assert_eq!(panel.serial.writes.len(), 4);
        assert_eq!(panel.serial.writes[0], frame(command::HELLO));
        assert_eq!(panel.serial.writes[1], frame(command::STOP_VIDEO));
        assert_eq!(panel.serial.writes[2], frame(command::STOP_MEDIA));
        assert_eq!(panel.serial.writes[3], frame(command::QUERY_STATUS));
        panel.serial.assert_frames();
    }

    #[test]
    fn a_panel_that_does_not_answer_hello_says_what_to_do() {
        let mut panel = TurzxPanel::new(FakeSerial::new(&[b"OK"]), GEOMETRY);
        assert_eq!(
            panel.open().unwrap_err(),
            "TURZX 5\" Panel is attached but did not answer: HELLO was not answered. \
             Unplug and replug it, or restart its USB device."
        );
    }

    #[test]
    fn a_panel_that_answers_hello_but_not_status_says_what_to_do() {
        let mut panel = TurzxPanel::new(FakeSerial::new(&[b"chs_5inch.dev1_rom1.88"]), GEOMETRY);
        assert_eq!(
            panel.open().unwrap_err(),
            "TURZX 5\" Panel is attached but did not answer: QUERY_STATUS was not answered. \
             Unplug and replug it, or restart its USB device."
        );
    }

    #[test]
    fn a_still_is_the_prefixed_bitmap() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel
            .present(&vec![0x11_u8; 800 * 480 * 4])
            .expect("the still presents");
        let writes = &panel.serial.writes;
        assert_eq!(writes.len(), 8);
        assert_eq!(writes[4], frame(command::PRE_UPDATE));
        assert_eq!(writes[5], frame(&command::start_display_bitmap()));
        assert_eq!(writes[6], frame(command::DISPLAY_BITMAP_5INCH));
        assert_eq!(writes[7].len(), 1_542_250);
        panel.serial.assert_frames();
    }

    #[test]
    fn a_still_of_the_wrong_size_is_refused_before_the_wire() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        let before = panel.serial.writes.len();
        assert!(panel.present(&[0x00; 4]).is_err());
        assert_eq!(panel.serial.writes.len(), before);
    }

    #[test]
    fn a_brightness_is_a_weight_out_of_255() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        assert_eq!(panel.run(PanelCommand::Brightness(50)).unwrap(), "");
        let write = panel.serial.writes.last().unwrap();
        assert_eq!(write[0], 0x7b);
        assert_eq!(write[2], 0x69);
        assert_eq!(write[FRAME_LEVEL], 128);
        panel.serial.assert_frames();
    }

    /// The level byte lands where the vendor's prefix ends.
    const FRAME_LEVEL: usize = command::BRIGHTNESS.len();

    #[test]
    fn a_brightness_out_of_range_is_refused() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        let before = panel.serial.writes.len();
        assert!(panel.run(PanelCommand::Brightness(101)).is_err());
        assert_eq!(panel.serial.writes.len(), before);
    }

    #[test]
    fn a_screen_off_is_zero_brightness_and_on_restores_the_level() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel.run(PanelCommand::Brightness(50)).unwrap();
        panel.run(PanelCommand::Screen(false)).unwrap();
        assert_eq!(panel.serial.writes.last().unwrap()[FRAME_LEVEL], 0);
        panel.run(PanelCommand::Screen(true)).unwrap();
        assert_eq!(panel.serial.writes.last().unwrap()[FRAME_LEVEL], 128);
    }

    #[test]
    fn the_settings_packet_carries_the_level_this_host_last_wrote() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel.run(PanelCommand::Brightness(20)).unwrap();
        panel
            .run(PanelCommand::PanelSettings {
                start_mode: 2,
                flip: true,
                sleep_delay: 5,
            })
            .unwrap();
        let write = panel.serial.writes.last().unwrap();
        assert_eq!(write[0], 0x7d);
        assert_eq!(write[6], 0x05);
        assert_eq!(write[FRAME_LEVEL], 51);
        assert_eq!(&write[FRAME_LEVEL + 1..FRAME_LEVEL + 5], &[2, 0, 1, 5]);
    }

    #[test]
    fn a_control_drops_the_acks_already_sitting_in_the_port() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel
            .serial
            .stale
            .push_back(b"needReSend:1|renderCnt:999".to_vec());
        panel
            .serial
            .replies
            .push_back(b"needReSend:0|renderCnt:2".to_vec());
        assert_eq!(
            panel.run(PanelCommand::Status).unwrap(),
            "needReSend:0|renderCnt:2"
        );
    }

    #[test]
    fn a_status_control_answers_with_the_panels_own_counters() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel
            .serial
            .replies
            .push_back(b"needReSend:1|renderCnt:77".to_vec());
        assert_eq!(
            panel.run(PanelCommand::Status).unwrap(),
            "needReSend:1|renderCnt:77"
        );
    }

    #[test]
    fn a_stale_ack_is_skipped_for_the_reply_it_asked_for() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel.serial.replies.push_back(b"OK".to_vec());
        panel
            .serial
            .replies
            .push_back(b"chs_5inch.dev1_rom1.88".to_vec());
        assert_eq!(
            panel.run(PanelCommand::Hello).unwrap(),
            "chs_5inch.dev1_rom1.88"
        );
    }
}
