//! One Lian Li panel: open, push a still, answer a control.
//!
//! The transport is a seam so a test can hand the driver a fake and read back
//! exactly what it wrote; on a real machine it is `UsbBulk` over libusb.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::family::{CommandKind, Geometry, PanelCommand, PanelDevice, StillFormat, VideoClip};
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
/// The block size the vendor driver falls back to when the panel answers
/// nothing (`WinUsb.CheckH264Block`).
pub const H264_BLOCK_BYTES: usize = 202_752;
/// The vendor's own cadence: a block goes out, the driver sleeps 30 ms, then
/// reads the acknowledgement. Nothing else paces the feed.
const BLOCK_PAUSE_MS: u64 = 30;
/// Blocks the feed keeps queued ahead of the decoder. The vendor's loop holds
/// 2–4; a feed that only matches the play time keeps one and the picture holds
/// at every block boundary.
const LOOKAHEAD_DEPTH: u8 = 2;

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
    /// Shared with the feed thread: one writer at a time on the endpoint.
    transport: Arc<Mutex<B>>,
    label: &'static str,
    geometry: Geometry,
    started: Instant,
    opened: bool,
    /// The level a screen-on restores: the last brightness this host wrote.
    level: u8,
    off: bool,
    /// The panel keeps the PNG layer across a push, so it is enabled once.
    overlay_shown: bool,
    /// The clip being fed, when one is.
    video: Option<Feed>,
}

/// A running feed, and the flag that ends it.
struct Feed {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl<B: Bulk + 'static> TurUsbPanel<B> {
    pub fn new(transport: B, label: &'static str, geometry: Geometry) -> Self {
        Self {
            transport: Arc::new(Mutex::new(transport)),
            label,
            geometry,
            started: Instant::now(),
            opened: false,
            level: 100,
            off: false,
            overlay_shown: false,
            video: None,
        }
    }

    /// Ends the feed thread before touching the wire again: a block written
    /// after the stop command starts a playback nobody is feeding.
    fn stop_feed(&mut self) {
        let Some(feed) = self.video.take() else {
            return;
        };
        feed.stop.store(true, Ordering::Relaxed);
        let _ = feed.handle.join();
    }

    fn request(&mut self, opcode: u8, fields: &Fields) -> Result<Option<Vec<u8>>, String> {
        let started = self.started;
        let mut transport = lock(&self.transport);
        request(&mut *transport, started, opcode, fields)
    }

    fn parameter(&mut self, opcode: u8, byte: u8) -> Result<Option<Vec<u8>>, String> {
        let started = self.started;
        let mut transport = lock(&self.transport);
        parameter(&mut *transport, started, opcode, byte)
    }

    /// Empties the panel's PNG layer, where L-Connect leaves its sensor
    /// dashboard: the last picture survives L-Connect exiting and every JPEG
    /// this driver pushes, so a theme otherwise renders under a ghost.
    fn clear_overlay(&mut self) -> Result<(), String> {
        let started = self.started;
        let mut transport = lock(&self.transport);
        parameter(&mut *transport, started, opcode::OVERLAY_LAYER, 1)?;
        self.overlay_shown = true;
        let png = transparent_png(self.geometry.width, self.geometry.height);
        let write = protocol::image(opcode::PUSH_PNG, &png, elapsed_ms(started));
        transport.write(&write, WRITE_TIMEOUT_MS)?;
        read(&mut *transport, READ_TIMEOUT_MS)?;
        Ok(())
    }

    fn firmware(&mut self) -> Result<String, String> {
        let started = self.started;
        let mut transport = lock(&self.transport);
        let reply = request(
            &mut *transport,
            started,
            opcode::VERSION,
            &Fields::default(),
        )?;
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

fn lock<B>(transport: &Arc<Mutex<B>>) -> MutexGuard<'_, B> {
    transport
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn elapsed_ms(started: Instant) -> u32 {
    started.elapsed().as_millis() as u32
}

fn request(
    transport: &mut impl Bulk,
    started: Instant,
    opcode: u8,
    fields: &Fields,
) -> Result<Option<Vec<u8>>, String> {
    let write = protocol::packet(opcode, elapsed_ms(started), fields, None);
    transport.write(&write, WRITE_TIMEOUT_MS)?;
    read(transport, READ_TIMEOUT_MS)
}

fn parameter(
    transport: &mut impl Bulk,
    started: Instant,
    opcode: u8,
    byte: u8,
) -> Result<Option<Vec<u8>>, String> {
    request(
        transport,
        started,
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
fn read(transport: &mut impl Bulk, timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
    let first = read_one(transport, timeout_ms);
    flush_replies(transport);
    first
}

fn flush_replies(transport: &mut impl Bulk) {
    for _ in 0..MAX_QUEUED_REPLIES {
        match read_one(transport, VENDOR_READ_TIMEOUT_MS) {
            Ok(Some(reply)) if !reply.is_empty() => {}
            _ => return,
        }
    }
}

/// A read the device never answers is the common case for a frame, not a
/// failure: the write already landed.
fn read_one(transport: &mut impl Bulk, timeout_ms: u64) -> Result<Option<Vec<u8>>, String> {
    match transport.read(512, timeout_ms) {
        Ok(reply) if !reply.is_empty() => Ok(Some(reply)),
        _ => Ok(None),
    }
}

/// Ends any playback still running. The vendor stops playback before **every**
/// frame: a panel still playing keeps ingesting into a buffer it never drains,
/// and after about thirty frames every transfer is cancelled.
fn stop_play(transport: &mut impl Bulk, started: Instant) -> Result<(), String> {
    request(transport, started, opcode::STOP_PLAY, &Fields::default())?;
    Ok(())
}

/// Recovering a stalled endpoint the way the vendor does: clear both halts and
/// write again. A device reset is avoided because it takes the LED MCU beside
/// the panel down with it.
fn write_frame(transport: &mut impl Bulk, write: &[u8]) -> Result<(), String> {
    if transport.write(write, WRITE_TIMEOUT_MS).is_ok() {
        return Ok(());
    }
    let _ = transport.clear_halt();
    transport.write(write, WRITE_TIMEOUT_MS)
}

/// Waits until the decoder has room, polling the way `waitBuffer(2)` does. A
/// panel that stops answering stops the drain as well: silence is not
/// permission to keep writing.
fn drain(
    transport: &mut impl Bulk,
    started: Instant,
    stop: Option<&AtomicBool>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(DRAIN_WAIT_MS);
    while !stop.is_some_and(|stop| stop.load(Ordering::Relaxed)) && Instant::now() < deadline {
        let depth = level(transport, started)?;
        if depth.is_none_or(|depth| depth <= DRAIN_DEPTH) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(DRAIN_POLL_MS));
    }
    Ok(())
}

/// The decoder's latest reported depth. Flush first, then ask: every block
/// queues an ack, so a plain read answers with a depth that can be stale.
fn level(transport: &mut impl Bulk, started: Instant) -> Result<Option<u8>, String> {
    flush_replies(transport);
    let write = protocol::packet(
        opcode::BUFFER_LEVEL,
        elapsed_ms(started),
        &Fields::default(),
        None,
    );
    transport.write(&write, WRITE_TIMEOUT_MS)?;
    Ok(read_one(transport, VENDOR_READ_TIMEOUT_MS)?.and_then(|reply| reply.get(8).copied()))
}

/// The block size this panel wants, with the vendor's own default when it
/// answers zero or not at all (`WinUsb.CheckH264Block`).
///
/// **Match the reply by opcode.** A stale reply left by an earlier command
/// answers this query with its own bytes; reading a block size out of a version
/// string once produced a 1.8 GB block, which the panel refused.
fn query_block_bytes(transport: &mut impl Bulk, started: Instant) -> Result<usize, String> {
    let Ok(Some(reply)) = request(transport, started, opcode::H264_BLOCK, &Fields::default())
    else {
        return Ok(H264_BLOCK_BYTES);
    };
    if reply.len() < 12 || reply[0] != opcode::H264_BLOCK {
        return Ok(H264_BLOCK_BYTES);
    }
    let size = u32::from_be_bytes([reply[8], reply[9], reply[10], reply[11]]) as usize;
    Ok(if size > 0 && size <= MAX_PAYLOAD {
        size
    } else {
        H264_BLOCK_BYTES
    })
}

/// One block onto the wire; answers the decoder's ring depth it reported.
///
/// The stream never ends: the feed wraps and keeps sending while a clip is
/// wanted, the way the vendor driver rewinds. The final block of each pass
/// carries the vendor's end-of-file flag so the panel can finish the stream it
/// was handed.
fn write_video_block(
    transport: &mut impl Bulk,
    started: Instant,
    clip: &[u8],
    at: usize,
    block_bytes: usize,
    stop: Option<&AtomicBool>,
) -> Result<Option<u8>, String> {
    let end = (at + block_bytes).min(clip.len());
    let block = &clip[at..end];
    let write = protocol::packet(
        opcode::PLAY_BLOCK,
        elapsed_ms(started),
        &Fields {
            payload_length: Some(block.len() as u32),
            last_block: Some(end >= clip.len()),
            ..Fields::default()
        },
        Some(block),
    );
    transport.write(&write, WRITE_TIMEOUT_MS)?;

    // **The ack is the flow control.** Its byte 8 is the decoder's depth: at
    // or below the vendor's threshold the ring is draining, and above it the
    // decoder has stopped reading — another block there is an overrun, which
    // the panel plays out as a freeze.
    thread::sleep(Duration::from_millis(BLOCK_PAUSE_MS));
    let depth =
        read_one(transport, VENDOR_READ_TIMEOUT_MS)?.and_then(|reply| reply.get(8).copied());
    if depth.is_some_and(|depth| depth > ACK_DEPTH) {
        drain(transport, started, stop)?;
    }
    Ok(depth)
}

/// Recovers from a cancelled block transfer. The block is not rewritten: a
/// cancelled bulk OUT can leave the decoder holding part of one, and writing
/// the same bytes again duplicates them. Starting again from the first block
/// hands the decoder a fresh keyframe.
fn resync(transport: &mut impl Bulk, started: Instant) {
    let _ = transport.clear_halt();
    let _ = stop_play(transport, started);
}

impl<B: Bulk + 'static> PanelDevice for TurUsbPanel<B> {
    fn geometry(&self) -> Geometry {
        self.geometry
    }

    fn open(&mut self) -> Result<(), String> {
        lock(&self.transport).open(CONFIGURATION, INTERFACE)?;
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
        let started = self.started;
        let mut transport = lock(&self.transport);
        stop_play(&mut *transport, started)?;
        let write = protocol::image(opcode::PUSH_JPEG, still, elapsed_ms(started));
        write_frame(&mut *transport, &write)?;
        // Flow control: byte 8 of the ack is the device's buffer level. A frame
        // commonly renders with no readable ack, so a timeout here is normal.
        if let Some(ack) = read(&mut *transport, READ_TIMEOUT_MS)?
            && ack.get(8).is_some_and(|depth| *depth > ACK_DEPTH)
        {
            drain(&mut *transport, started, None)?;
        }
        Ok(())
    }

    fn start_video(&mut self, clip: VideoClip<'_>) -> Result<(), String> {
        if !self.opened {
            return Err(format!("{} is not open.", self.label));
        }
        let VideoClip::H264 { bytes, seconds } = clip else {
            return Err("That panel takes a raw H.264 stream, not a stored clip.".to_owned());
        };
        if bytes.is_empty() {
            return Err("The clip is empty.".to_owned());
        }
        self.stop_feed();

        // The first block is sent here so an encoder or wire failure still
        // reaches the runner, which then falls back to still frames; the rest
        // is fed in the background.
        let started = self.started;
        let block_bytes = {
            let mut transport = lock(&self.transport);
            // No frame-rate command here: a 15 sent just before the blocks
            // makes this panel's decoder ghost moving parts.
            stop_play(&mut *transport, started)?;
            let block_bytes = query_block_bytes(&mut *transport, started)?;
            write_video_block(&mut *transport, started, bytes, 0, block_bytes, None)?;
            block_bytes
        };

        let stop = Arc::new(AtomicBool::new(false));
        let handle = spawn_feed(
            Arc::clone(&self.transport),
            started,
            bytes.to_vec(),
            seconds,
            block_bytes,
            Arc::clone(&stop),
        );
        self.video = Some(Feed { stop, handle });
        Ok(())
    }

    fn present_overlay(&mut self, png: &[u8]) -> Result<(), String> {
        if !self.opened {
            return Err(format!("{} is not open.", self.label));
        }
        if png.len() > MAX_PAYLOAD {
            return Err(format!(
                "A {}-byte overlay exceeds the panel's {MAX_PAYLOAD}-byte limit.",
                png.len()
            ));
        }
        let started = self.started;
        let mut transport = lock(&self.transport);
        if !self.overlay_shown {
            parameter(&mut *transport, started, opcode::OVERLAY_LAYER, 1)?;
            self.overlay_shown = true;
        }
        let write = protocol::image(opcode::PUSH_PNG, png, elapsed_ms(started));
        write_frame(&mut *transport, &write)?;
        read(&mut *transport, READ_TIMEOUT_MS)?;
        Ok(())
    }

    fn stop_video(&mut self) {
        self.stop_feed();
        if !self.opened {
            return;
        }
        let started = self.started;
        let mut transport = lock(&self.transport);
        let _ = stop_play(&mut *transport, started);
        // A stopped clip leaves the layer off; the next overlay enables it.
        self.overlay_shown = false;
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
        self.stop_feed();
        self.opened = false;
        // A panel that went away cannot be closed again; the unplug is the
        // report, not this failure.
        let mut transport = lock(&self.transport);
        let _ = transport.clear_halt();
        transport.close()
    }
}

/// Feeds the clip's blocks forever, paced by the clip's own playing time.
fn spawn_feed<B: Bulk + 'static>(
    transport: Arc<Mutex<B>>,
    started: Instant,
    clip: Vec<u8>,
    seconds: f64,
    block_bytes: usize,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let clip_ms = seconds * 1000.0;
        let mut at = block_bytes;
        while !stop.load(Ordering::Relaxed) {
            if at >= clip.len() {
                at = 0;
            }
            let sent = (at + block_bytes).min(clip.len()) - at;
            let began = Instant::now();
            let result = {
                let mut guard = lock(&transport);
                write_video_block(&mut *guard, started, &clip, at, block_bytes, Some(&stop))
            };
            let mut depth = None;
            match result {
                Ok(reported) => {
                    depth = reported;
                    at += block_bytes;
                }
                Err(error) => {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    // Nothing above is awaiting this loop, so a failure has to
                    // be said here: the panel keeps whatever it decoded last.
                    eprintln!("Panel video: {error}. Restarting the clip.");
                    let mut guard = lock(&transport);
                    resync(&mut *guard, started);
                    at = 0;
                }
            }

            // **The clip's playing time is the pace, kept outside the lock.**
            // Handing this panel blocks faster than it plays makes its decoder
            // leave ghost trails. Sleeping under the lock would hold the
            // overlay push for a whole block instead.
            //
            // Except while the ring is short: below `LOOKAHEAD_DEPTH` the next
            // block goes without waiting, which fills the buffer.
            let play_ms = if clip.is_empty() {
                0.0
            } else {
                (sent as f64 / clip.len() as f64) * clip_ms
            };
            let spent = began.elapsed().as_secs_f64() * 1000.0;
            let wait = match depth {
                Some(depth) if depth < LOOKAHEAD_DEPTH => 0.0,
                _ => (play_ms - spent).max(BLOCK_PAUSE_MS as f64),
            };
            if wait > 0.0 {
                pause(wait, &stop);
            }
        }
    })
}

/// Sleeps `wait_ms` in slices, so stopping the feed does not wait for a whole
/// block's playing time.
fn pause(wait_ms: f64, stop: &AtomicBool) {
    let mut left = wait_ms;
    while left > 0.0 && !stop.load(Ordering::Relaxed) {
        let step = left.min(50.0);
        thread::sleep(Duration::from_secs_f64(step / 1000.0));
        left -= step;
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
        /// Depths a `buffer-level` poll answers, one per command, and whether
        /// the reply is waiting for the short read that follows the command.
        depths: std::collections::VecDeque<u8>,
        short_reply: bool,
        /// The size an `h264-block` query answers, when it answers at all.
        block_size: Option<u32>,
    }

    impl FakeBulk {
        fn new() -> Self {
            Self {
                writes: Vec::new(),
                replies: std::collections::VecDeque::new(),
                fail_at_write: 0,
                cleared: 0,
                depths: std::collections::VecDeque::new(),
                short_reply: false,
                block_size: None,
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
            // A poll's depth and a block query's size are answers to the
            // command that was just written, so they arrive with it.
            let opcode = crate::tur_usb::protocol::decrypt_block(bytes)[0];
            if opcode == opcode::BUFFER_LEVEL
                && let Some(depth) = self.depths.pop_front()
            {
                let mut reply = vec![0_u8; 8];
                reply.push(depth);
                self.replies.push_back(reply);
                self.short_reply = true;
            }
            if opcode == opcode::H264_BLOCK
                && let Some(size) = self.block_size
            {
                let mut reply = vec![0_u8; 12];
                reply[0] = opcode::H264_BLOCK;
                reply[8..12].copy_from_slice(&size.to_be_bytes());
                self.replies.push_back(reply);
            }
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        fn read(&mut self, _length: usize, timeout_ms: u64) -> Result<Vec<u8>, String> {
            // The driver flushes with the vendor's short timeout; only a reply
            // that arrived with the command just written answers one of those.
            if timeout_ms < READ_TIMEOUT_MS && !self.short_reply {
                return Ok(Vec::new());
            }
            let reply = self.replies.pop_front().unwrap_or_default();
            if self.replies.is_empty() {
                self.short_reply = false;
            }
            Ok(reply)
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

    fn bulk(panel: &TurUsbPanel<FakeBulk>) -> MutexGuard<'_, FakeBulk> {
        lock(&panel.transport)
    }

    fn h264(bytes: &[u8], seconds: f64) -> VideoClip<'_> {
        VideoClip::H264 { bytes, seconds }
    }

    #[test]
    fn open_announces_the_host_and_pins_the_turn() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        let opcodes = {
            let transport = bulk(&panel);
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
        assert_eq!(bulk(&panel).parameter(1), FRAME_RATE);
        assert_eq!(bulk(&panel).parameter(2), ROTATE_MODE);
    }

    #[test]
    fn a_still_stops_playback_before_it_pushes() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        bulk(&panel).writes.clear();
        panel.present(&[0xff, 0xd8, 0xff]).unwrap();
        let opcodes: Vec<u8> = bulk(&panel).opcodes();
        assert_eq!(opcodes, vec![opcode::STOP_PLAY, opcode::PUSH_JPEG]);
        let push = {
            let transport = bulk(&panel);
            transport.writes.last().unwrap().len()
        };
        assert!(push > PACKET_SIZE);
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
        bulk(&panel).writes.clear();
        // stop-play reads nothing; the push's ack reports a full buffer, then
        // the poll answers 9 and then 2, which is the drain's own floor.
        bulk(&panel).replies.push_back(Vec::new());
        bulk(&panel).replies.push_back(depth(9));
        bulk(&panel).depths.push_back(9);
        bulk(&panel).depths.push_back(2);
        panel.present(&[0xff]).unwrap();
        let polls = bulk(&panel)
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
        bulk(&panel).writes.clear();
        // The frame is the second write of a push; the stop-play before it
        // lands, which is what a panel wedged mid-frame looks like.
        bulk(&panel).fail_at_write = 2;
        panel.present(&[0xff]).unwrap();
        assert_eq!(bulk(&panel).cleared, 1);
        assert_eq!(
            bulk(&panel).opcodes(),
            vec![opcode::STOP_PLAY, opcode::PUSH_JPEG]
        );
    }

    #[test]
    fn brightness_is_a_percent_and_screen_off_restores_it() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        bulk(&panel).writes.clear();
        assert_eq!(panel.run(PanelCommand::Brightness(70)).unwrap(), "");
        assert_eq!(bulk(&panel).parameter(0), 70);
        bulk(&panel).writes.clear();
        panel.run(PanelCommand::Screen(false)).unwrap();
        assert_eq!(bulk(&panel).parameter(0), 0);
        bulk(&panel).writes.clear();
        panel.run(PanelCommand::Screen(true)).unwrap();
        assert_eq!(bulk(&panel).parameter(0), 70);
    }

    #[test]
    fn status_reads_the_firmware_and_refuses_silence() {
        let mut panel = self::panel(FakeBulk::new());
        panel.open().unwrap();
        let mut reply = vec![0_u8; 8];
        reply.extend_from_slice(b"lianli92_0001\0");
        bulk(&panel).replies.push_back(reply);
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
        assert_eq!(
            panel.start_video(h264(&[0], 1.0)).unwrap_err(),
            "Lian Li 9.2\" LCD is not open."
        );
        assert_eq!(
            panel.present_overlay(&[0]).unwrap_err(),
            "Lian Li 9.2\" LCD is not open."
        );
    }

    #[test]
    fn a_clip_is_announced_as_blocks_after_stop_play() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        bulk(&panel).writes.clear();
        panel
            .start_video(h264(&[0x00, 0x00, 0x01, 0x65], 1.0))
            .unwrap();
        panel.stop_video();
        let transport = bulk(&panel);
        let opcodes = transport.opcodes();
        assert_eq!(
            &opcodes[..3],
            &[opcode::STOP_PLAY, opcode::H264_BLOCK, opcode::PLAY_BLOCK]
        );
        let first = transport.plain()[2];
        assert_eq!(u32::from_be_bytes(first[8..12].try_into().unwrap()), 4);
        // The clip is one block, so that block ends the pass.
        assert_eq!(first[12], 1);
    }

    #[test]
    fn the_panels_block_size_is_used_and_a_zero_answer_keeps_the_default() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        bulk(&panel).writes.clear();
        bulk(&panel).block_size = Some(4_096);
        panel.start_video(h264(&[7_u8; 10_000], 2.0)).unwrap();
        panel.stop_video();
        let first = bulk(&panel).plain()[2];
        assert_eq!(u32::from_be_bytes(first[8..12].try_into().unwrap()), 4_096);
        // More blocks follow, so this one is not the end of the stream.
        assert_eq!(first[12], 0);

        let mut fallback = self::panel(FakeBulk::new());
        fallback.open().unwrap();
        bulk(&fallback).writes.clear();
        bulk(&fallback).block_size = Some(0);
        fallback
            .start_video(h264(&vec![7_u8; 300_000], 2.0))
            .unwrap();
        fallback.stop_video();
        let first = bulk(&fallback).plain()[2];
        assert_eq!(
            u32::from_be_bytes(first[8..12].try_into().unwrap()) as usize,
            H264_BLOCK_BYTES
        );
    }

    #[test]
    fn an_empty_clip_is_refused() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        assert_eq!(
            panel.start_video(h264(&[], 1.0)).unwrap_err(),
            "The clip is empty."
        );
    }

    #[test]
    fn an_overlay_enables_its_layer_once_and_is_pushed_as_png() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        bulk(&panel).writes.clear();
        // The open already enabled the layer for its transparent clear.
        panel.overlay_shown = false;
        panel.present_overlay(&[1, 2, 3]).unwrap();
        panel.present_overlay(&[4, 5]).unwrap();
        let transport = bulk(&panel);
        assert_eq!(
            transport.opcodes(),
            vec![opcode::OVERLAY_LAYER, opcode::PUSH_PNG, opcode::PUSH_PNG]
        );
        assert_eq!(transport.parameter(0), 1);
    }

    #[test]
    fn a_stopped_clip_re_enables_the_layer_on_the_next_overlay() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        panel.present_overlay(&[1]).unwrap();
        bulk(&panel).writes.clear();
        panel.stop_video();
        panel.present_overlay(&[2]).unwrap();
        assert_eq!(
            bulk(&panel).opcodes(),
            vec![opcode::STOP_PLAY, opcode::OVERLAY_LAYER, opcode::PUSH_PNG]
        );
    }

    #[test]
    fn an_overlay_over_the_payload_limit_is_refused_by_its_size() {
        let mut panel = panel(FakeBulk::new());
        panel.open().unwrap();
        let error = panel
            .present_overlay(&vec![0_u8; MAX_PAYLOAD + 1])
            .unwrap_err();
        assert_eq!(
            error,
            "A 512001-byte overlay exceeds the panel's 512000-byte limit."
        );
    }
}
