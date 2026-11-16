//! One Kraken: HID reports for control, bulk pixels for the glass.
//!
//! Two interfaces on one device, and neither alone is enough: control is the
//! HID class interface (64-byte reports) and data is bulk endpoint 2 on
//! interface 0. On Windows the HID interface belongs to the class driver, so it
//! is opened through HIDAPI rather than claimed through libusb.

use std::time::{Duration, Instant};

use crate::family::{
    CommandKind, Controls, Geometry, Lighting, PanelCommand, PanelDevice, RenderMode, StillFormat,
    Telemetry,
};

pub const SIZE: u32 = 320;
/// The transfer header every upload begins with.
pub const HEADER_MAGIC: [u8; 12] = [
    0x12, 0xfa, 0x01, 0xe8, 0xab, 0xcd, 0xef, 0x98, 0x76, 0x54, 0x32, 0x10,
];
/// Upload type `0x02`: one still, 32 bits per pixel, alpha ignored.
pub const RGBA_MODE: u8 = 0x02;
/// Upload type `0x03`: a 256-entry RGBA palette, then 8-bit indices.
pub const PALETTED_MODE: u8 = 0x03;
/// The unit a bucket's start and size are counted in, both on the device.
pub const SLOT_BYTES: u32 = 1024;
/// The device's bucket table; every index in it can hold one frame.
pub const BUCKET_COUNT: u8 = 16;
const FRAME_BYTES: u32 = SIZE * SIZE * 4;
const HEADER_BYTES: u32 = HEADER_MAGIC.len() as u32 + 8;
const BUCKET_SLOTS: u16 = ((HEADER_BYTES + FRAME_BYTES) / SLOT_BYTES) as u16;
/// CAM's paletted bucket is exactly its payload: 101 slots cover the palette
/// and the indices, and the header written anyway lands in the next region,
/// which the next frame deletes and recreates.
pub const PALETTED_SLOTS: u16 = 101;
/// The glass honours a mode-A switch only about every 140 ms: the firmware
/// answers every command with status 1 and silently drops a switch that
/// arrives before the previous bucket is on the glass.
pub const KRAKEN_FRAME_INTERVAL_MS: u32 = 140;
/// A paletted switch costs the glass about 110 ms, which is CAM's own cadence.
pub const KRAKEN_PALETTED_FRAME_INTERVAL_MS: u32 = 110;
/// A single blocking read's timeout.
const READ_TIMEOUT_MS: u32 = 120;
/// How long a command waits for its own reply. A window in time, not a count of
/// reports: the firmware broadcasts unsolicited status reports, and counting
/// reads would spend the budget on them.
const REPLY_WINDOW_MS: u64 = 1_000;
/// How long the firmware gets to finish storing a bucket before it is shown.
const UPLOAD_SETTLE_MS: u64 = 10;
/// One HID report, exactly. A 65-byte array is read as "report id + 64 bytes"
/// and Windows refuses it with 0x57.
pub(crate) const REPORT_BYTES: usize = 64;
const STATUS: usize = 14;

/// The HID control interface.
pub trait Hid: Send {
    fn write(&mut self, report: &[u8; REPORT_BYTES]) -> Result<(), String>;
    /// `None` for a read that drew no report, which is the normal answer to a
    /// command the device does not reply to.
    fn read(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, String>;
    fn close(&mut self);
}

/// The bulk half: interface 0, endpoint 2.
pub trait BulkStream: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn close(&mut self);
}

/// A frame the device would not store. `status` is the firmware's byte 14, or
/// `None` when the command drew no reply at all — the two need different
/// repairs, so the caller has to see which it was. `stage` names the command,
/// because a refused bucket setup and a refused switch are different faults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRefused {
    pub status: Option<u8>,
    pub stage: &'static str,
}

impl FrameRefused {
    fn new(status: Option<u8>, stage: &'static str) -> Self {
        Self { status, stage }
    }

    pub fn message(&self) -> String {
        let status = self
            .status
            .map_or_else(|| "none".to_owned(), |status| status.to_string());
        format!(
            "The Kraken refused a frame upload ({}, status {status}).",
            self.stage
        )
    }
}

impl std::fmt::Display for FrameRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

pub struct KrakenZ3<H: Hid, B: BulkStream> {
    control: H,
    bulk: B,
    /// Whether the bucket table has been cleared for this driver.
    ready: bool,
    /// The bucket on the glass, or `None` before the first frame.
    shown: Option<u8>,
    /// The level a screen-on restores.
    level: u8,
    off: bool,
    /// How frames are laid out on the wire.
    render_mode: RenderMode,
}

impl<H: Hid, B: BulkStream> KrakenZ3<H, B> {
    pub fn new(control: H, bulk: B) -> Self {
        Self {
            control,
            bulk,
            ready: false,
            shown: None,
            level: 100,
            off: false,
            render_mode: RenderMode::ModeA,
        }
    }

    /// One 64-byte report, padded; the report id is part of the layout.
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut report = [0_u8; REPORT_BYTES];
        let copy = bytes.len().min(REPORT_BYTES);
        report[..copy].copy_from_slice(&bytes[..copy]);
        self.control.write(&report)
    }

    /// Read until this command's own reply arrives. Commands do not answer in
    /// order: the firmware also emits status reports on its own, so anything
    /// that is not the reply is dropped and the wait continues.
    fn read_until(&mut self, first: u8, second: u8) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(REPLY_WINDOW_MS);
        while Instant::now() < deadline {
            let Ok(Some(report)) = self.control.read(READ_TIMEOUT_MS) else {
                continue;
            };
            if report.first() == Some(&first) && report.get(1) == Some(&second) {
                return Some(report);
            }
        }
        None
    }

    fn drain(&mut self) {
        for _ in 0..80 {
            if !matches!(self.control.read(READ_TIMEOUT_MS), Ok(Some(_))) {
                return;
            }
        }
    }

    fn firmware(&mut self) -> Option<[u8; 3]> {
        self.write(&[0x10, 0x01]).ok()?;
        let reply = self.read_until(0x11, 0x01)?;
        Some([
            reply.get(0x11).copied().unwrap_or(0),
            reply.get(0x12).copied().unwrap_or(0),
            reply.get(0x13).copied().unwrap_or(0),
        ])
    }

    /// The `31 01` reply: brightness at 0x18, orientation at 0x1A.
    fn lcd_info(&mut self) -> Option<Vec<u8>> {
        self.write(&[0x30, 0x01]).ok()?;
        self.read_until(0x31, 0x01)
    }

    fn status_of(&mut self, first: u8, second: u8) -> Option<u8> {
        self.read_until(first, second)?.get(STATUS).copied()
    }

    fn delete_bucket(&mut self, index: u8) -> Option<u8> {
        self.write(&[0x32, 0x02, index]).ok()?;
        self.status_of(0x33, 0x02)
    }

    fn create_bucket(&mut self, index: u8, slots: u16) -> Option<u8> {
        let start = (index as u32 * slots as u32) as u16;
        self.write(&[
            0x32,
            0x01,
            index,
            index + 1,
            (start & 0xff) as u8,
            (start >> 8) as u8,
            (slots & 0xff) as u8,
            (slots >> 8) as u8,
            0x01,
        ])
        .ok()?;
        self.status_of(0x33, 0x01)
    }

    fn switch_bucket(&mut self, index: u8, mode: u8) -> Option<u8> {
        self.write(&[0x38, 0x01, mode, index]).ok()?;
        self.status_of(0x39, 0x01)
    }

    fn end_transfer(&mut self, index: u8) {
        let _ = self.write(&[0x36, 0x02, index]);
        let _ = self.read_until(0x37, 0x02);
    }

    /// Liquefy the glass, then empty every bucket so the ring has its range.
    /// It leaves bucket 0 on the glass, so `shown` is set here.
    fn prepare(&mut self) -> Result<(), String> {
        // Read `37 03`, not "the next report": the firmware emits `75 01` on
        // its own, and taking one as this reply shifts every read after it.
        self.write(&[0x36, 0x03])?;
        self.read_until(0x37, 0x03);
        // Liquid mode first: the firmware refuses to delete the bucket it plays.
        self.switch_bucket(0, 0x02);
        self.shown = Some(0);
        for index in 0..BUCKET_COUNT {
            self.delete_bucket(index);
        }
        Ok(())
    }

    /// The bucket this frame goes into: the one not on the glass. Mode A keeps
    /// a two-bucket ring because the firmware's 140 ms glass only fits two;
    /// mode B uses CAM's full 16, each holding one paletted frame.
    fn next_bucket(&self) -> u8 {
        if self.render_mode == RenderMode::ModeA {
            return match self.shown {
                Some(0) => 1,
                _ => 0,
            };
        }
        Self::bucket_after(self.render_mode, self.shown.unwrap_or(BUCKET_COUNT - 1))
    }

    fn bucket_after(mode: RenderMode, index: u8) -> u8 {
        let count = if mode == RenderMode::ModeA {
            2
        } else {
            BUCKET_COUNT
        };
        (index + 1) % count
    }

    fn transfer_header(mode: u8, payload_bytes: u32) -> [u8; HEADER_BYTES as usize] {
        let mut header = [0_u8; HEADER_BYTES as usize];
        header[..HEADER_MAGIC.len()].copy_from_slice(&HEADER_MAGIC);
        header[12] = mode;
        header[16..20].copy_from_slice(&payload_bytes.to_le_bytes());
        header
    }

    fn write_frame(&mut self, index: u8, rgba: &[u8]) -> Result<(), FrameRefused> {
        let paletted = self.render_mode == RenderMode::ModeB;
        let mode = if paletted { PALETTED_MODE } else { RGBA_MODE };
        let slots = if paletted {
            PALETTED_SLOTS
        } else {
            BUCKET_SLOTS
        };
        let payload = if paletted {
            crate::kraken::palette::to_paletted_frame(rgba)
        } else {
            rgba.to_vec()
        };

        // A refused delete must never be written over: the firmware keeps the
        // first graphic a bucket stored. A second delete is the retry for a
        // refusal the firmware answers once, and CAM deletes twice on every
        // paletted frame regardless.
        let mut status = self.delete_bucket(index);
        if paletted || status != Some(0x01) {
            status = self.delete_bucket(index);
        }
        if status != Some(0x01) {
            return Err(FrameRefused::new(status, "delete"));
        }
        // Never stream into a bucket the firmware refused to set up: on the
        // 2024 firmware that produced wrong pictures and wedged the device.
        let status = self.create_bucket(index, slots);
        if status != Some(0x01) {
            return Err(FrameRefused::new(status, "create"));
        }

        self.write(&[0x36, 0x01, index])
            .map_err(|_| FrameRefused::new(None, "begin"))?;
        let begin = self.status_of(0x37, 0x01);
        if begin != Some(0x01) {
            // A begin whose reply was missed still opened the transfer: end it
            // so the next tick starts clean and one frame is lost, not the
            // panel.
            if begin.is_none() || begin == Some(0x02) {
                self.end_transfer(index);
            }
            return Err(FrameRefused::new(begin, "begin"));
        }

        let header = Self::transfer_header(mode, payload.len() as u32);
        self.bulk
            .write(&header)
            .and_then(|()| self.bulk.write(&payload))
            .map_err(|_| FrameRefused::new(None, "write"))?;

        let _ = self.write(&[0x36, 0x02, index]);
        self.read_until(0x37, 0x02);
        // The switch races the write: `36 02` answers before the glass has the
        // bucket's last tiles, and switching on its heels shows a tear. CAM's
        // paletted stream carries a quarter of the tiles and needs no settle.
        if !paletted {
            std::thread::sleep(Duration::from_millis(UPLOAD_SETTLE_MS));
        }
        let status = self.switch_bucket(index, 0x04);
        if status != Some(0x01) {
            return Err(FrameRefused::new(status, "switch"));
        }
        Ok(())
    }

    /// liquidctl's brightness write: the current orientation travels with the
    /// level, because the firmware takes both fields in one report.
    fn write_brightness(&mut self, level: u8) -> Result<(), String> {
        let Some(lcd) = self.lcd_info() else {
            return Err("The Kraken did not answer its LCD query.".to_owned());
        };
        let orientation = lcd.get(0x1a).copied().unwrap_or(0);
        self.write(&[0x30, 0x02, 0x01, level, 0x00, 0x00, 0x01, orientation])
    }

    fn present_rgba(&mut self, rgba: &[u8]) -> Result<(), String> {
        if !self.ready {
            self.prepare()?;
            self.ready = true;
        }
        // `prepare` leaves bucket 0 on the glass, so every frame targets the
        // bucket `shown` is not.
        let target = self.next_bucket();
        match self.write_frame(target, rgba) {
            Ok(()) => {
                self.shown = Some(target);
                Ok(())
            }
            // Status 9 means `shown` is wrong, and the ring can repair itself:
            // the firmware refuses the bucket it is displaying, so the bucket
            // `shown` names is not the one on the glass. Write it instead.
            Err(refusal) if refusal.status == Some(9) => {
                let other = Self::bucket_after(self.render_mode, target);
                match self.write_frame(other, rgba) {
                    Ok(()) => {
                        self.shown = Some(other);
                        Ok(())
                    }
                    // Neither bucket takes a frame: the table changed under us,
                    // so the next frame rebuilds it.
                    Err(retry) => {
                        self.ready = false;
                        Err(retry.message())
                    }
                }
            }
            Err(refusal) => Err(refusal.message()),
        }
    }
}

impl<H: Hid, B: BulkStream> PanelDevice for KrakenZ3<H, B> {
    fn geometry(&self) -> Geometry {
        Geometry {
            width: SIZE,
            height: SIZE,
        }
    }

    fn open(&mut self) -> Result<(), String> {
        self.drain();
        // The two queries are the check, not a record: what a panel says about
        // its firmware and its LCD is the only way to know a handle which
        // opened is a handle the firmware is listening on.
        if self.firmware().is_none() {
            return Err("The Kraken did not answer its firmware query.".to_owned());
        }
        if self.lcd_info().is_none() {
            return Err("The Kraken did not answer its LCD query.".to_owned());
        }
        Ok(())
    }

    fn present(&mut self, still: &[u8]) -> Result<(), String> {
        let image = image::load_from_memory(still)
            .map_err(|error| format!("The panel frame could not be read: {error}"))?
            .into_rgba8();
        if image.width() != SIZE || image.height() != SIZE {
            // The firmware accepts only its own geometry; a bigger frame would
            // overrun the bucket it is stored in.
            let resized =
                image::imageops::resize(&image, SIZE, SIZE, image::imageops::FilterType::Triangle);
            return self.present_rgba(&resized.into_raw());
        }
        self.present_rgba(&image.into_raw())
    }

    fn still_format(&self) -> StillFormat {
        StillFormat::Png
    }

    fn frame_interval_ms(&self) -> u32 {
        if self.render_mode == RenderMode::ModeB {
            KRAKEN_PALETTED_FRAME_INTERVAL_MS
        } else {
            KRAKEN_FRAME_INTERVAL_MS
        }
    }

    fn render_modes(&self) -> &'static [RenderMode] {
        &[RenderMode::ModeA, RenderMode::ModeB]
    }

    fn set_render_mode(&mut self, mode: RenderMode) -> Result<(), String> {
        self.render_mode = mode;
        Ok(())
    }

    fn accepts(&self, kind: CommandKind) -> bool {
        matches!(
            kind,
            CommandKind::Brightness | CommandKind::Screen | CommandKind::Status
        )
    }

    fn run(&mut self, command: PanelCommand) -> Result<String, String> {
        match command {
            // Asking for a level is also asking for the screen back.
            PanelCommand::Brightness(level) => {
                self.level = level.min(100);
                self.off = false;
                self.write_brightness(self.level)?;
                // The write draws no reply, so there is no device text to show.
                Ok(String::new())
            }
            PanelCommand::Screen(on) => {
                self.off = !on;
                self.write_brightness(if self.off { 0 } else { self.level })?;
                Ok(String::new())
            }
            PanelCommand::Status => {
                let Some(firmware) = self.firmware() else {
                    return Err("The Kraken did not answer its firmware query.".to_owned());
                };
                let mut parts = vec![format!(
                    "Firmware {}.{}.{}",
                    firmware[0], firmware[1], firmware[2]
                )];
                if let Some(lcd) = self.lcd_info() {
                    let brightness = lcd.get(0x18).copied().unwrap_or(0);
                    let orientation = lcd.get(0x1a).copied().unwrap_or(0);
                    parts.push(format!("LCD brightness {brightness}%"));
                    parts.push(format!("orientation {}°", u16::from(orientation) * 90));
                }
                Ok(parts.join(", "))
            }
            _ => Err("That is not a command this panel accepts.".to_owned()),
        }
    }

    fn close(&mut self) -> Result<(), String> {
        self.bulk.close();
        // A HID handle is a file; closing it is the release.
        self.control.close();
        Ok(())
    }

    fn lighting(&self) -> Option<Lighting> {
        Some(crate::kraken::lighting::lighting())
    }

    fn write_colors(&mut self, rgb: &[u8]) -> Result<(), String> {
        // Validate and pack first: a short frame is refused before a report
        // reaches the wire.
        let reports = crate::kraken::lighting::lighting_reports(rgb)?;
        for report in &reports {
            self.write(report)?;
        }
        self.write(&crate::kraken::lighting::lighting_commit())
    }

    fn controls(&self) -> Option<Controls> {
        Some(Controls {
            pump: true,
            fans: true,
            telemetry: true,
        })
    }

    fn set_controls(&mut self, pump: Option<f64>, fan: Option<f64>) -> Result<(), String> {
        // Both reports are packed before either is written, so a bad value
        // cannot leave half a request on the wire.
        let pump = pump.map(crate::kraken::lighting::pump_report).transpose()?;
        let fan = fan.map(crate::kraken::lighting::fan_report).transpose()?;
        if pump.is_none() && fan.is_none() {
            return Err("A controls request needs pumpPercent or fanPercent.".to_owned());
        }
        if let Some(report) = pump {
            self.write(&report)?;
        }
        if let Some(report) = fan {
            self.write(&report)?;
        }
        Ok(())
    }

    fn telemetry(&mut self) -> Result<Telemetry, String> {
        self.write(&[0x74, 0x01])?;
        let reply = self
            .read_until(0x75, 0x01)
            .ok_or_else(|| "The Kraken did not answer its status query.".to_owned())?;
        crate::kraken::lighting::parse_status(&reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakeHid {
        writes: Vec<[u8; REPORT_BYTES]>,
        replies: VecDeque<Vec<u8>>,
        /// A report every reply cycle resembles; the firmware broadcasts one.
        broadcast: Option<Vec<u8>>,
        /// Whether a command has been written since the panel opened. A device
        /// only answers writes, which is what makes the open-time drain empty.
        armed: bool,
    }

    impl FakeHid {
        fn new() -> Self {
            Self {
                writes: Vec::new(),
                replies: VecDeque::new(),
                broadcast: None,
                armed: false,
            }
        }
    }

    impl Hid for FakeHid {
        fn write(&mut self, report: &[u8; REPORT_BYTES]) -> Result<(), String> {
            self.writes.push(*report);
            self.armed = true;
            Ok(())
        }

        fn read(&mut self, _timeout_ms: u32) -> Result<Option<Vec<u8>>, String> {
            if !self.armed {
                return Ok(None);
            }
            if self.broadcast.is_some() {
                let report = self.broadcast.clone();
                self.broadcast = None;
                return Ok(report);
            }
            Ok(self.replies.pop_front())
        }

        fn close(&mut self) {}
    }

    struct FakeBulk {
        writes: Vec<Vec<u8>>,
    }

    impl BulkStream for FakeBulk {
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        fn close(&mut self) {}
    }

    fn reply(first: u8, second: u8, status: u8) -> Vec<u8> {
        let mut report = vec![0_u8; REPORT_BYTES];
        report[0] = first;
        report[1] = second;
        report[STATUS] = status;
        report
    }

    fn firmware_reply() -> Vec<u8> {
        let mut report = reply(0x11, 0x01, 1);
        report[0x11] = 5;
        report[0x12] = 11;
        report[0x13] = 0;
        report
    }

    fn lcd_reply(brightness: u8, orientation: u8) -> Vec<u8> {
        let mut report = reply(0x31, 0x01, 1);
        report[0x18] = brightness;
        report[0x1a] = orientation;
        report
    }

    fn opened(mut hid: FakeHid) -> KrakenZ3<FakeHid, FakeBulk> {
        hid.replies.push_back(firmware_reply());
        hid.replies.push_back(lcd_reply(80, 1));
        let mut panel = KrakenZ3::new(hid, FakeBulk { writes: Vec::new() });
        panel.open().unwrap();
        panel
    }

    /// The replies one mode-A frame expects after `open`.
    fn frame_replies(panel: &mut KrakenZ3<FakeHid, FakeBulk>) {
        panel.control.replies.push_back(reply(0x37, 0x03, 1)); // liquid mode
        panel.control.replies.push_back(reply(0x39, 0x01, 1)); // switch bucket 0
        for _ in 0..BUCKET_COUNT {
            panel.control.replies.push_back(reply(0x33, 0x02, 1)); // delete
        }
        // Two deletes, create, begin, end, switch — the target bucket first.
        panel.control.replies.push_back(reply(0x33, 0x02, 1));
        panel.control.replies.push_back(reply(0x33, 0x01, 1));
        panel.control.replies.push_back(reply(0x37, 0x01, 1));
        panel.control.replies.push_back(reply(0x37, 0x02, 1));
        panel.control.replies.push_back(reply(0x39, 0x01, 1));
    }

    fn still() -> Vec<u8> {
        let png = image::RgbaImage::from_pixel(SIZE, SIZE, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(png)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn open_proves_the_control_channel_answers() {
        let mut hid = FakeHid::new();
        hid.replies.push_back(firmware_reply());
        hid.replies.push_back(lcd_reply(80, 0));
        let mut panel = KrakenZ3::new(hid, FakeBulk { writes: Vec::new() });
        panel.open().unwrap();
        assert_eq!(panel.control.writes[0][..2], [0x10, 0x01]);
        assert_eq!(panel.control.writes[1][..2], [0x30, 0x01]);
    }

    #[test]
    fn a_kraken_that_says_nothing_is_refused_by_name() {
        let mut panel = KrakenZ3::new(FakeHid::new(), FakeBulk { writes: Vec::new() });
        // A read that never answers must not hang the open; the fake answers
        // nothing and the reply window closes.
        assert_eq!(
            panel.open().unwrap_err(),
            "The Kraken did not answer its firmware query."
        );
    }

    #[test]
    fn a_broadcast_status_report_does_not_answer_a_command() {
        let mut hid = FakeHid::new();
        hid.replies.push_back(reply(0x75, 0x01, 1));
        hid.replies.push_back(firmware_reply());
        hid.replies.push_back(reply(0x75, 0x01, 1));
        hid.replies.push_back(lcd_reply(50, 2));
        let mut panel = KrakenZ3::new(hid, FakeBulk { writes: Vec::new() });
        panel.open().unwrap();
        // The firmware report was dropped, not read as this command's answer.
        assert_eq!(panel.control.writes.len(), 2);
    }

    #[test]
    fn a_frame_deletes_creates_streams_and_switches() {
        let mut panel = opened(FakeHid::new());
        panel.control.writes.clear();
        frame_replies(&mut panel);
        panel.present(&still()).unwrap();
        let commands: Vec<[u8; 2]> = panel
            .control
            .writes
            .iter()
            .map(|report| [report[0], report[1]])
            .collect();
        assert_eq!(commands[0], [0x36, 0x03]);
        assert_eq!(commands[1], [0x38, 0x01]);
        // Deleting, creating, beginning, ending and switching the target.
        assert!(commands.contains(&[0x32, 0x02]));
        assert!(commands.contains(&[0x32, 0x01]));
        assert!(commands.contains(&[0x36, 0x01]));
        assert!(commands.contains(&[0x36, 0x02]));
        assert_eq!(*commands.last().unwrap(), [0x38, 0x01]);
        // Header then pixels, one bulk write each.
        assert_eq!(panel.bulk.writes.len(), 2);
        assert_eq!(panel.bulk.writes[0][..12], HEADER_MAGIC);
        assert_eq!(panel.bulk.writes[0][12], RGBA_MODE);
        assert_eq!(&panel.bulk.writes[0][16..20], &FRAME_BYTES.to_le_bytes());
        assert_eq!(panel.bulk.writes[1].len() as u32, FRAME_BYTES);
        assert_eq!(panel.shown, Some(1));
    }

    #[test]
    fn a_paletted_frame_writes_cam_s_own_payload_shape() {
        let mut panel = opened(FakeHid::new());
        panel.control.writes.clear();
        panel
            .set_render_mode(RenderMode::ModeB)
            .expect("mode B is this driver's own");
        assert_eq!(panel.frame_interval_ms(), KRAKEN_PALETTED_FRAME_INTERVAL_MS);

        panel.control.replies.push_back(reply(0x37, 0x03, 1)); // liquid mode
        panel.control.replies.push_back(reply(0x39, 0x01, 1)); // switch bucket 0
        for _ in 0..BUCKET_COUNT {
            panel.control.replies.push_back(reply(0x33, 0x02, 1)); // prepare deletes
        }
        // Two deletes, create, begin, end, switch: CAM deletes twice a frame.
        panel.control.replies.push_back(reply(0x33, 0x02, 1));
        panel.control.replies.push_back(reply(0x33, 0x02, 1));
        panel.control.replies.push_back(reply(0x33, 0x01, 1));
        panel.control.replies.push_back(reply(0x37, 0x01, 1));
        panel.control.replies.push_back(reply(0x37, 0x02, 1));
        panel.control.replies.push_back(reply(0x39, 0x01, 1));
        panel.present(&still()).expect("a paletted frame");

        // Header then a palette-plus-indices payload, one bulk write each.
        assert_eq!(panel.bulk.writes[0][12], PALETTED_MODE);
        assert_eq!(
            &panel.bulk.writes[0][16..20],
            &(crate::kraken::palette::FRAME_BYTES as u32).to_le_bytes()
        );
        assert_eq!(
            panel.bulk.writes[1].len(),
            crate::kraken::palette::FRAME_BYTES
        );
        // CAM's bucket is exactly its payload: 101 slots, not the RGBA count.
        let create = panel
            .control
            .writes
            .iter()
            .find(|report| report[0] == 0x32 && report[1] == 0x01)
            .expect("a create");
        assert_eq!(
            u16::from_le_bytes([create[6], create[7]]),
            PALETTED_SLOTS,
            "the paletted bucket is 101 slots"
        );
        // Mode A leaves bucket 0 on the glass from `prepare`; the frame lands
        // in the next bucket of the 16-bucket ring.
        assert_eq!(panel.shown, Some(1));
    }

    #[test]
    fn a_refused_delete_never_streams_into_the_bucket() {
        let mut panel = opened(FakeHid::new());
        // prepare: liquid mode, the switch to bucket 0, then sixteen deletes.
        panel.control.replies.push_back(reply(0x37, 0x03, 1));
        panel.control.replies.push_back(reply(0x39, 0x01, 1));
        for _ in 0..BUCKET_COUNT {
            panel.control.replies.push_back(reply(0x33, 0x02, 1));
        }
        // Both buckets are refused: the firmware keeps whatever a refused
        // bucket holds, so nothing may be streamed into either. The ring turns
        // a status 9 into a second attempt at the other bucket, and that is
        // refused as well.
        for _ in 0..4 {
            panel.control.replies.push_back(reply(0x33, 0x02, 9));
        }
        let error = panel.present(&still()).unwrap_err();
        assert_eq!(
            error,
            "The Kraken refused a frame upload (delete, status 9)."
        );
        assert!(panel.bulk.writes.is_empty());
    }

    #[test]
    fn a_status_nine_repoints_the_next_frame() {
        let mut panel = opened(FakeHid::new());
        panel.ready = true;
        panel.shown = Some(0);
        // Both deletes refused with 9: the bucket `shown` names is on the glass,
        // so the other bucket is written instead and takes the frame.
        panel.control.replies.push_back(reply(0x33, 0x02, 9));
        panel.control.replies.push_back(reply(0x33, 0x02, 9));
        panel.control.replies.push_back(reply(0x33, 0x02, 1));
        panel.control.replies.push_back(reply(0x33, 0x01, 1));
        panel.control.replies.push_back(reply(0x37, 0x01, 1));
        panel.control.replies.push_back(reply(0x37, 0x02, 1));
        panel.control.replies.push_back(reply(0x39, 0x01, 1));
        panel.present(&still()).unwrap();
        assert_eq!(panel.shown, Some(0));
        assert_eq!(panel.bulk.writes.len(), 2);
    }

    #[test]
    fn a_missed_begin_ends_the_transfer_before_giving_up() {
        let mut panel = opened(FakeHid::new());
        panel.ready = true;
        panel.shown = Some(0);
        panel.control.replies.push_back(reply(0x33, 0x02, 1));
        panel.control.replies.push_back(reply(0x33, 0x01, 1));
        // Begin answers "already transferring" — the command landed, its reply
        // did not, and every later begin would answer the same way.
        panel.control.replies.push_back(reply(0x37, 0x01, 2));
        panel.control.replies.push_back(reply(0x37, 0x02, 1));
        let error = panel.present(&still()).unwrap_err();
        assert_eq!(
            error,
            "The Kraken refused a frame upload (begin, status 2)."
        );
        let commands: Vec<[u8; 2]> = panel
            .control
            .writes
            .iter()
            .map(|report| [report[0], report[1]])
            .collect();
        assert!(commands.contains(&[0x36, 0x02]));
        assert!(panel.bulk.writes.is_empty());
    }

    #[test]
    fn brightness_carries_the_orientation_it_read() {
        let mut panel = opened(FakeHid::new());
        panel.control.replies.push_back(lcd_reply(80, 3));
        panel.run(PanelCommand::Brightness(40)).unwrap();
        let report = panel.control.writes.last().unwrap();
        assert_eq!(report[..8], [0x30, 0x02, 0x01, 40, 0x00, 0x00, 0x01, 3]);
    }

    #[test]
    fn status_names_the_firmware_lcd_and_orientation() {
        let mut panel = opened(FakeHid::new());
        panel.control.replies.push_back(firmware_reply());
        panel.control.replies.push_back(lcd_reply(70, 2));
        assert_eq!(
            panel.run(PanelCommand::Status).unwrap(),
            "Firmware 5.11.0, LCD brightness 70%, orientation 180°"
        );
    }

    #[test]
    fn a_control_the_kraken_lacks_is_refused() {
        let mut panel = opened(FakeHid::new());
        assert_eq!(
            panel.run(PanelCommand::Hello).unwrap_err(),
            "That is not a command this panel accepts."
        );
        assert!(!panel.accepts(CommandKind::PanelSettings));
    }
}
