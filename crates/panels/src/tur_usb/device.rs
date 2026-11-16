//! One Lian Li panel: open, push a still, answer a control.
//!
//! The transport is a seam so a test can hand the driver a fake and read back
//! exactly what it wrote; on a real machine it is `UsbBulk` over libusb.

use std::time::{Duration, Instant};

use crate::family::{CommandKind, Geometry, PanelCommand, PanelDevice, StillFormat};
use crate::tur_usb::protocol::{self, Fields, opcode};

/// The panel can take this long to answer a command. A shorter read does not
/// fail politely: the transfer is cancelled, and enough cancellations make the
/// endpoint refuse writes until the cable is replugged.
pub const READ_TIMEOUT_MS: u64 = 2_000;
/// A write is allowed to wait: the panel NAKs bulk OUT while it drains, and a
/// timeout cancels the URB mid-block, after which the panel's USB stack stops
/// answering altogether.
pub const WRITE_TIMEOUT_MS: u64 = 5_000;
/// The device refuses a payload past this; measured in both drivers.
pub const MAX_PAYLOAD: usize = 512_000;
/// One vendor interface, one endpoint, no alternate settings.
pub const CONFIGURATION: i32 = 1;
pub const INTERFACE: i32 = 0;
pub const ENDPOINT: u8 = 1;
/// The panel's own quarter turn. It is sticky across power cycles, so the host
/// pins it rather than inheriting whatever the vendor app left behind. Mode 2
/// is the one measured upright with the display record's 90° rotation.
pub const ROTATE_MODE: u8 = 2;
/// Legacy brightness the vendor app uses on every frame.
const FRAME_RATE: u8 = 30;
const MAX_QUEUED_REPLIES: usize = 16;
/// The vendor's own read timeout, used to flush queued replies cheaply.
const VENDOR_READ_TIMEOUT_MS: u64 = 100;
/// The depth the vendor drains above, and the depth it drains to.
const ACK_DEPTH: u8 = 3;
const DRAIN_DEPTH: u8 = 2;
const DRAIN_POLL_MS: u64 = 50;
const DRAIN_WAIT_MS: u64 = 30_000;

/// The bulk endpoint, as the driver needs it.
pub trait Bulk: Send {
    fn open(&mut self, configuration: i32, interface: i32) -> Result<(), String>;
    fn write(&mut self, bytes: &[u8], timeout_ms: u64) -> Result<(), String>;
    fn read(&mut self, length: usize, timeout_ms: u64) -> Result<Vec<u8>, String>;
    /// Clears a stalled endpoint in both directions; a failure here is not
    /// worth reporting above the stall it is recovering from.
    fn clear_halt(&mut self) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
}

pub struct TurUsbPanel<B: Bulk> {
    transport: B,
    label: &'static str,
    geometry: Geometry,
    started: Instant,
    opened: bool,
    /// The level a screen-on restores: the last brightness this host wrote.
    level: u8,
    off: bool,
    /// The panel keeps the PNG layer across a push, so it is enabled once.
    overlay_shown: bool,
}

impl<B: Bulk> TurUsbPanel<B> {
    pub fn new(transport: B, label: &'static str, geometry: Geometry) -> Self {
        Self {
            transport,
            label,
            geometry,
            started: Instant::now(),
            opened: false,
            level: 100,
            off: false,
            overlay_shown: false,
        }
    }

    fn elapsed_ms(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn request(&mut self, opcode: u8, fields: &Fields) -> Result<Option<Vec<u8>>, String> {
        let write = protocol::packet(opcode, self.elapsed_ms(), fields, None);
        self.transport.write(&write, WRITE_TIMEOUT_MS)?;
        self.read(READ_TIMEOUT_MS)
    }

    fn parameter(&mut self, opcode: u8, byte: u8) -> Result<Option<Vec<u8>>, String> {
        self.request(
            opcode,
            &Fields {
                parameter: Some(byte),
                ..Fields::default()
            },
        )
    }

    /// One reply, then whatever else the panel already queued.
    ///
    /// A reply left unread backs up the IN queue, and once it is full the panel
    /// stops accepting writes — a freeze that reads as a panel fault and only a
    /// replug clears. The vendor driver flushes after every read for the same
    /// reason; a read with nothing waiting answers with a zero-length transfer.
    fn read(&mut self, timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
        let first = self.read_one(timeout_ms);
        self.flush_replies();
        first
    }

    fn flush_replies(&mut self) {
        for _ in 0..MAX_QUEUED_REPLIES {
            match self.read_one(VENDOR_READ_TIMEOUT_MS) {
                Ok(Some(reply)) if !reply.is_empty() => {}
                _ => return,
            }
        }
    }

    /// A read the device never answers is the common case for a frame, not a
    /// failure: the write already landed.
    fn read_one(&mut self, timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
        match self.transport.read(512, timeout_ms) {
            Ok(reply) if !reply.is_empty() => Ok(Some(reply)),
            _ => Ok(None),
        }
    }

    /// Ends any playback still running. The vendor stops playback before
    /// **every** frame: a panel still playing keeps ingesting into a buffer it
    /// never drains, and after about thirty frames every transfer is cancelled.
    fn stop_play(&mut self) -> Result<(), String> {
        self.request(opcode::STOP_PLAY, &Fields::default())?;
        Ok(())
    }

    /// Recovering a stalled endpoint the way the vendor does: clear both halts
    /// and write again. A device reset is avoided because it takes the LED MCU
    /// beside the panel down with it.
    fn write_frame(&mut self, write: &[u8]) -> Result<(), String> {
        if self.transport.write(write, WRITE_TIMEOUT_MS).is_ok() {
            return Ok(());
        }
        let _ = self.transport.clear_halt();
        self.transport.write(write, WRITE_TIMEOUT_MS)
    }

    fn drain(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(DRAIN_WAIT_MS);
        while Instant::now() < deadline {
            let reply = self.request(opcode::BUFFER_LEVEL, &Fields::default())?;
            let depth = reply.as_ref().and_then(|reply| reply.get(8)).copied();
            if depth.is_none_or(|depth| depth <= DRAIN_DEPTH) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(DRAIN_POLL_MS));
        }
        Ok(())
    }

    /// Empties the panel's PNG layer, where L-Connect leaves its sensor
    /// dashboard: the last picture survives L-Connect exiting and every JPEG
    /// this driver pushes, so a theme otherwise renders under a ghost.
    fn clear_overlay(&mut self) -> Result<(), String> {
        self.parameter(opcode::OVERLAY_LAYER, 1)?;
        self.overlay_shown = true;
        let png = transparent_png(self.geometry.width, self.geometry.height);
        let write = protocol::image(opcode::PUSH_PNG, &png, self.elapsed_ms());
        self.transport.write(&write, WRITE_TIMEOUT_MS)?;
        self.read(READ_TIMEOUT_MS)?;
        Ok(())
    }

    fn firmware(&mut self) -> Result<String, String> {
        let reply = self.request(opcode::VERSION, &Fields::default())?;
        let body = reply
            .as_ref()
            .map(|reply| reply[8.min(reply.len())..].to_vec());
        let name = body.map(|body| {
            let end = body
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(body.len());
            String::from_utf8_lossy(&body[..end]).trim().to_owned()
        });
        match name {
            Some(name) if !name.is_empty() => Ok(name),
            _ => Err("The panel did not answer its firmware query.".to_owned()),
        }
    }
}

impl<B: Bulk> PanelDevice for TurUsbPanel<B> {
    fn geometry(&self) -> Geometry {
        self.geometry
    }

    fn open(&mut self) -> Result<(), String> {
        self.transport.open(CONFIGURATION, INTERFACE)?;
        self.opened = true;
        // The sync exchange is what tells a panel it has a host. Every driver
        // sends it first, and a frame without it is ignored by some firmware.
        self.request(opcode::VERSION, &Fields::default())?;
        self.parameter(opcode::FRAME_RATE, FRAME_RATE)?;
        self.parameter(opcode::ROTATE, ROTATE_MODE)?;
        self.clear_overlay()
    }

    fn present(&mut self, still: &[u8]) -> Result<(), String> {
        if !self.opened {
            return Err(format!("{} is not open.", self.label));
        }
        if still.len() > MAX_PAYLOAD {
            return Err(format!(
                "A {}-byte frame exceeds the panel's {MAX_PAYLOAD}-byte limit.",
                still.len()
            ));
        }
        self.stop_play()?;
        let write = protocol::image(opcode::PUSH_JPEG, still, self.elapsed_ms());
        self.write_frame(&write)?;
        // Flow control: byte 8 of the ack is the device's buffer level. A frame
        // commonly renders with no readable ack, so a timeout here is normal.
        if let Some(ack) = self.read(READ_TIMEOUT_MS)?
            && ack.get(8).is_some_and(|depth| *depth > ACK_DEPTH)
        {
            self.drain()?;
        }
        Ok(())
    }

    fn still_format(&self) -> StillFormat {
        StillFormat::Jpeg
    }

    fn accepts(&self, kind: CommandKind) -> bool {
        matches!(
            kind,
            CommandKind::Brightness | CommandKind::Screen | CommandKind::Status
        )
    }

    fn run(&mut self, command: PanelCommand) -> Result<String, String> {
        match command {
            // Opcode 14's parameter is the percent itself, 0..100 — not a
            // 0..255 weight. L-Connect's own constants are exactly 0 and 100.
            PanelCommand::Brightness(level) => {
                self.parameter(opcode::BRIGHTNESS, level.min(100))?;
                self.level = level.min(100);
                self.off = false;
                // The ack carries no text, so a host sentence would be read as
                // the panel's reply.
                Ok(String::new())
            }
            PanelCommand::Screen(on) => {
                // Brightness is the only off switch this panel documents: zero
                // draws nothing and leaves the cable open.
                self.off = !on;
                self.parameter(opcode::BRIGHTNESS, if self.off { 0 } else { self.level })?;
                Ok(String::new())
            }
            PanelCommand::Status => self.firmware(),
            _ => Err("That is not a command this panel accepts.".to_owned()),
        }
    }

    fn close(&mut self) -> Result<(), String> {
        if !self.opened {
            return Ok(());
        }
        self.opened = false;
        // A panel that went away cannot be closed again; the unplug is the
        // report, not this failure.
        let _ = self.transport.clear_halt();
        self.transport.close()
    }
}

/// A transparent PNG of the panel's own size, for clearing the overlay layer.
fn transparent_png(width: u32, height: u32) -> Vec<u8> {
    let buffer =
        image::RgbaImage::from_pixel(width.max(1), height.max(1), image::Rgba([0, 0, 0, 0]));
    let mut png = Vec::new();
    if image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_err()
    {
        return png;
    }
    png
}

/// The real transport: libusb, the panel's one vendor interface, endpoint 1.
pub struct UsbBulk {
    handle: Option<rusb::DeviceHandle<rusb::GlobalContext>>,
    endpoint_out: u8,
    endpoint_in: u8,
    interface: i32,
}

impl UsbBulk {
    /// The first attached panel with these ids. `None` is "not plugged in",
    /// which the route turns into its own sentence.
    pub fn find(vendor_id: u16, product_id: u16) -> Option<Self> {
        let devices = rusb::devices().ok()?;
        for device in devices.iter() {
            let descriptor = device.device_descriptor().ok()?;
            if descriptor.vendor_id() == vendor_id && descriptor.product_id() == product_id {
                let handle = device.open().ok()?;
                return Some(Self {
                    handle: Some(handle),
                    endpoint_out: ENDPOINT,
                    endpoint_in: ENDPOINT | 0x80,
                    interface: INTERFACE,
                });
            }
        }
        None
    }
}

impl Bulk for UsbBulk {
    fn open(&mut self, configuration: i32, interface: i32) -> Result<(), String> {
        let handle = self.handle.as_ref().ok_or("The panel is not open.")?;
        handle
            .set_active_configuration(configuration as u8)
            .map_err(|error| error.to_string())?;
        handle
            .claim_interface(interface as u8)
            .map_err(|error| error.to_string())?;
        self.interface = interface;
        Ok(())
    }

    fn write(&mut self, bytes: &[u8], timeout_ms: u64) -> Result<(), String> {
        let handle = self.handle.as_ref().ok_or("The panel is not open.")?;
        handle
            .write_bulk(self.endpoint_out, bytes, Duration::from_millis(timeout_ms))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn read(&mut self, length: usize, timeout_ms: u64) -> Result<Vec<u8>, String> {
        let handle = self.handle.as_ref().ok_or("The panel is not open.")?;
        let mut buffer = vec![0_u8; length];
        let read = handle
            .read_bulk(
                self.endpoint_in,
                &mut buffer,
                Duration::from_millis(timeout_ms),
            )
            .map_err(|error| error.to_string())?;
        buffer.truncate(read);
        Ok(buffer)
    }

    fn clear_halt(&mut self) -> Result<(), String> {
        let handle = self.handle.as_ref().ok_or("The panel is not open.")?;
        handle
            .clear_halt(self.endpoint_out)
            .and_then(|_| handle.clear_halt(self.endpoint_in))
            .map_err(|error| error.to_string())
    }

    fn close(&mut self) -> Result<(), String> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let interface = self.interface as u8;
        handle.release_interface(interface).ok();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tur_usb::protocol::PACKET_SIZE;

    /// A panel that records writes and answers from a script.
    struct FakeBulk {
        writes: Vec<Vec<u8>>,
        replies: std::collections::VecDeque<Vec<u8>>,
        /// The write that fails, counting from one, so a test can fail the
        /// frame and not the command before it.
        fail_at_write: usize,
        cleared: usize,
    }

    impl FakeBulk {
        fn new() -> Self {
            Self {
                writes: Vec::new(),
                replies: std::collections::VecDeque::new(),
                fail_at_write: 0,
                cleared: 0,
            }
        }

        /// The plaintext of each write, in order.
        fn plain(&self) -> Vec<[u8; crate::tur_usb::protocol::COMMAND_SIZE]> {
            self.writes
                .iter()
                .map(|write| crate::tur_usb::protocol::decrypt_block(write))
                .collect()
        }

        fn opcodes(&self) -> Vec<u8> {
            self.plain().iter().map(|block| block[0]).collect()
        }

        fn parameter(&self, index: usize) -> u8 {
            self.plain()[index][8]
        }
    }

    impl Bulk for FakeBulk {
        fn open(&mut self, _configuration: i32, _interface: i32) -> Result<(), String> {
            Ok(())
        }

        fn write(&mut self, bytes: &[u8], _timeout_ms: u64) -> Result<(), String> {
            if self.writes.len() + 1 == self.fail_at_write {
                self.fail_at_write = 0;
                return Err("cancelled".to_owned());
            }
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        fn read(&mut self, _length: usize, timeout_ms: u64) -> Result<Vec<u8>, String> {
            // The driver flushes with the vendor's short timeout; a reply the
            // device has not sent yet does not answer one of those.
            if timeout_ms < READ_TIMEOUT_MS {
                return Ok(Vec::new());
            }
            Ok(self.replies.pop_front().unwrap_or_default())
        }

        fn clear_halt(&mut self) -> Result<(), String> {
            self.cleared += 1;
            Ok(())
        }

        fn close(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    fn panel(transport: FakeBulk) -> TurUsbPanel<FakeBulk> {
        TurUsbPanel::new(
            transport,
            "Lian Li 9.2\" LCD",
            Geometry {
                width: 464,
                height: 1920,
            },
        )
    }

    #[test]
    fn open_announces_the_host_and_pins_the_turn() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        let opcodes = {
            let transport: &FakeBulk = &panel.transport;
            transport.opcodes()
        };
        // version, frame rate, rotate, overlay enable, transparent PNG.
        assert_eq!(
            opcodes,
            vec![
                opcode::VERSION,
                opcode::FRAME_RATE,
                opcode::ROTATE,
                opcode::OVERLAY_LAYER,
                opcode::PUSH_PNG
            ]
        );
        assert_eq!(panel.transport.parameter(1), FRAME_RATE);
        assert_eq!(panel.transport.parameter(2), ROTATE_MODE);
    }

    #[test]
    fn a_still_stops_playback_before_it_pushes() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        panel.transport.writes.clear();
        panel.present(&[0xff, 0xd8, 0xff]).unwrap();
        let opcodes: Vec<u8> = panel.transport.opcodes();
        assert_eq!(opcodes, vec![opcode::STOP_PLAY, opcode::PUSH_JPEG]);
        let push = panel.transport.writes.last().unwrap();
        assert!(push.len() > PACKET_SIZE);
    }

    #[test]
    fn a_frame_over_the_payload_limit_is_refused() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        let error = panel.present(&vec![0_u8; MAX_PAYLOAD + 1]).unwrap_err();
        assert_eq!(
            error,
            "A 512001-byte frame exceeds the panel's 512000-byte limit."
        );
    }

    /// A reply whose byte 8 is the device's buffer depth.
    fn depth(depth: u8) -> Vec<u8> {
        let mut reply = vec![0_u8; 8];
        reply.push(depth);
        reply
    }

    #[test]
    fn a_full_buffer_is_drained_until_it_is_not() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        panel.transport.writes.clear();
        // stop-play reads nothing; the push's ack reports a full buffer, then
        // the poll answers 9 and then 2, which is the drain's own floor.
        panel.transport.replies.push_back(Vec::new());
        panel.transport.replies.push_back(depth(9));
        panel.transport.replies.push_back(depth(9));
        panel.transport.replies.push_back(depth(2));
        panel.present(&[0xff]).unwrap();
        let polls = panel
            .transport
            .opcodes()
            .iter()
            .filter(|code| **code == opcode::BUFFER_LEVEL)
            .count();
        assert_eq!(polls, 2);
    }

    #[test]
    fn a_cancelled_write_is_retried_after_the_halts_are_cleared() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        panel.transport.writes.clear();
        // The frame is the second write of a push; the stop-play before it
        // lands, which is what a panel wedged mid-frame looks like.
        panel.transport.fail_at_write = 2;
        panel.present(&[0xff]).unwrap();
        assert_eq!(panel.transport.cleared, 1);
        assert_eq!(
            panel.transport.opcodes(),
            vec![opcode::STOP_PLAY, opcode::PUSH_JPEG]
        );
    }

    #[test]
    fn brightness_is_a_percent_and_screen_off_restores_it() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        panel.transport.writes.clear();
        assert_eq!(panel.run(PanelCommand::Brightness(70)).unwrap(), "");
        assert_eq!(panel.transport.parameter(0), 70);
        panel.transport.writes.clear();
        panel.run(PanelCommand::Screen(false)).unwrap();
        assert_eq!(panel.transport.parameter(0), 0);
        panel.transport.writes.clear();
        panel.run(PanelCommand::Screen(true)).unwrap();
        assert_eq!(panel.transport.parameter(0), 70);
    }

    #[test]
    fn status_reads_the_firmware_and_refuses_silence() {
        let mut panel = self::panel(FakeBulk::new());
        panel.open().unwrap();
        let mut reply = vec![0_u8; 8];
        reply.extend_from_slice(b"lianli92_0001\0");
        panel.transport.replies.push_back(reply);
        assert_eq!(panel.run(PanelCommand::Status).unwrap(), "lianli92_0001");

        let mut silent = self::panel(FakeBulk::new());
        silent.open().unwrap();
        assert_eq!(
            silent.run(PanelCommand::Status).unwrap_err(),
            "The panel did not answer its firmware query."
        );
    }

    #[test]
    fn a_control_the_panel_does_not_have_is_refused() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        assert_eq!(
            panel.run(PanelCommand::Restart).unwrap_err(),
            "That is not a command this panel accepts."
        );
        assert!(panel.accepts(CommandKind::Brightness));
        assert!(!panel.accepts(CommandKind::Restart));
    }

    #[test]
    fn a_closed_panel_is_not_presentable() {
        let mut panel = panel(FakeBulk::new());
        assert_eq!(
            panel.present(&[]).unwrap_err(),
            "Lian Li 9.2\" LCD is not open."
        );
    }
}
