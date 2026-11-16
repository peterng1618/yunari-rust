//! The TURZX 5" serial panel: a USB CDC gadget that takes a full BGRA frame per
//! update, not a JPEG stream.

use std::time::{Duration, Instant};

use super::overlay;
use super::protocol::{self, command, frame, parse_hello_text, parse_panel_status};
use crate::family::{CommandKind, Geometry, PanelCommand, PanelDevice, StillFormat, VideoClip};

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
/// A file command's answer may follow a whole upload, not just its header.
const FILE_TIMEOUT_MS: u64 = 10_000;
/// The stored-size probe's own cap: a missing file has no verified reply, and
/// a missed answer only means "upload".
const SIZE_PROBE_TIMEOUT_MS: u64 = 2_000;
/// The panel dropped an overlay update sent 8 ms after init; 1.5 s worked.
const OVERLAY_SETTLE_MS: u64 = 1_500;
/// One QUERY_STATUS answer may take this long before it counts as unanswered.
const OVERLAY_STATUS_TIMEOUT_MS: u64 = 1_000;
/// Consecutive unanswered status queries that mean the panel stopped talking.
const UNANSWERED_QUERIES_BEFORE_REINIT: u32 = 3;
/// A file payload is cut into 249-byte chunks, each its own 250-byte frame.
const FILE_CHUNK_BYTES: usize = 249;
/// Frames per write: the device reads a byte stream, so one write per frame
/// costs a round trip per 249 bytes for no change on the wire.
const UPLOAD_BATCH_FRAMES: usize = 64;
/// A file command's answer loop gives up after this long.
const FILE_REPLY_DEADLINE_MS: u64 = 30_000;
/// The overlay init's two answers; the bare prefix is not one of them.
const OVERLAY_INIT_OK: &str = "seq_png_init_sucess";
const OVERLAY_INIT_FAILED: &str = "seq_png_init_failed";
/// The upload's two acks: the file was opened, then the panel finished it.
const UPLOAD_ACKS: [&str; 2] = ["create_success", "file_rev_done"];
/// Storage the vendor firmware reads clips from.
const VIDEO_DIRECTORY: &str = "/mnt/SDCARD/video";

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
    /// The clip the panel is looping, so the next start can delete it.
    previous_video_path: Option<String>,
    /// The last overlay sent, the base for the next diff.
    previous_overlay: Option<Vec<u8>>,
    overlay_count: u32,
    /// The panel's own render counter, which advances while its overlay runs.
    last_render_cnt: Option<u32>,
    unanswered_queries: u32,
}

impl<S: Serial> TurzxPanel<S> {
    pub fn new(serial: S, geometry: Geometry) -> Self {
        Self {
            serial,
            geometry,
            opened: false,
            brightness: 255,
            screen_off: false,
            previous_video_path: None,
            previous_overlay: None,
            overlay_count: 0,
            last_render_cnt: None,
            unanswered_queries: 0,
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

    /// A file command: its header, then the path (and size) inside one frame.
    fn send_file(
        &mut self,
        command: &[u8],
        remote_path: &str,
        size: Option<u32>,
    ) -> Result<(), String> {
        self.serial
            .write(&overlay::file_header(command, remote_path, size)?)
    }

    /// Reads until `accept` answers, an empty read ends the wait, or the file
    /// deadline passes. A non-empty reply that does not match is a stale ack
    /// and is skipped; the last one is returned so the caller can name it.
    fn read_replies<T>(
        &mut self,
        mut accept: impl FnMut(&str) -> Option<T>,
        read_timeout_ms: u64,
    ) -> (Option<T>, String) {
        let deadline = Instant::now() + Duration::from_millis(FILE_REPLY_DEADLINE_MS);
        let mut last = String::new();
        while Instant::now() < deadline {
            let Ok(bytes) = self.serial.read(read_timeout_ms) else {
                break;
            };
            let text = protocol::decode_reply(&bytes);
            if text.is_empty() {
                break;
            }
            if let Some(accepted) = accept(&text) {
                return (Some(accepted), text);
            }
            last = text;
        }
        (None, last)
    }

    fn read_until_accept<T>(
        &mut self,
        wanted: &str,
        accept: impl FnMut(&str) -> Option<T>,
        read_timeout_ms: u64,
    ) -> Result<T, String> {
        let (accepted, last) = self.read_replies(accept, read_timeout_ms);
        accepted.ok_or_else(|| {
            let last = if last.is_empty() { "no reply" } else { &last };
            format!("The panel did not answer with {wanted}; last reply: {last}.")
        })
    }

    /// The device's stored size for `remote_path`, or 0 when it holds nothing.
    fn file_size(&mut self, remote_path: &str) -> Result<usize, String> {
        self.send_file(command::GET_FILE_SIZE, remote_path, None)?;
        Ok(self
            .read_replies(protocol::parse_file_size, SIZE_PROBE_TIMEOUT_MS)
            .0
            .unwrap_or(0))
    }

    /// Uploads `bytes`, waiting for the file to be opened before the first
    /// chunk: a chunk written before the panel's `create_success` is dropped,
    /// and the panel then answers neither ack.
    fn upload(&mut self, remote_path: &str, bytes: &[u8]) -> Result<(), String> {
        self.send_file(command::UPLOAD_FILE, remote_path, Some(bytes.len() as u32))?;

        let mut created = false;
        let mut done = false;
        let (accepted, _) = self.read_replies(
            |text| {
                for phrase in UPLOAD_ACKS {
                    if text.contains(phrase) {
                        if phrase == UPLOAD_ACKS[0] {
                            created = true;
                        } else {
                            done = true;
                        }
                    }
                }
                created.then_some(())
            },
            FILE_TIMEOUT_MS,
        );
        if accepted.is_none() {
            return Err("The panel did not answer with \"create_success\".".to_owned());
        }

        // Whole frames batch into one write. Each still ends on its own
        // 250-byte boundary, so the stream the panel reads is unchanged; only
        // the number of host-side drains falls.
        let batch_bytes = UPLOAD_BATCH_FRAMES * protocol::FRAME_SIZE;
        let mut batch = Vec::with_capacity(batch_bytes);
        for chunk in bytes.chunks(FILE_CHUNK_BYTES) {
            batch.extend_from_slice(&frame(chunk));
            if batch.len() == batch_bytes {
                self.serial.write(&batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            self.serial.write(&batch)?;
        }

        if !done {
            self.read_until_accept(
                "\"file_rev_done\"",
                |text| text.contains(UPLOAD_ACKS[1]).then_some(()),
                FILE_TIMEOUT_MS,
            )?;
        }
        Ok(())
    }

    /// Puts the overlay back to a transparent, initialized state. A re-init
    /// does not wait for the init's ack: the panel whose overlay is wedged is
    /// exactly the one that will not answer, and waiting would fail the update.
    fn init_overlay(&mut self, clear: bool, expect_ack: bool) -> Result<(), String> {
        self.send(command::PRE_UPDATE)?;
        self.send(&command::start_display_bitmap())?;
        self.send(command::DISPLAY_ON_VIDEO)?;

        // The overlay starts fully transparent: every pixel the UI does not
        // paint shows the playing video underneath. Only a clip start pays for
        // that frame; a re-init is cleared by the repaint that follows it.
        let transparent =
            vec![0_u8; self.geometry.width as usize * self.geometry.height as usize * 4];
        if clear {
            self.send(&protocol::bgra_payload(&transparent))?;
        }
        self.send(command::INIT_VIDEO_OVERLAY)?;
        self.send(command::OVERLAY_TERMINATE)?;

        let accept = |text: &str| {
            (text.contains(OVERLAY_INIT_OK) || text.contains(OVERLAY_INIT_FAILED))
                .then(|| text.to_owned())
        };
        let init = if expect_ack {
            Some(self.read_until_accept("\"seq_png_init_\"", accept, FILE_TIMEOUT_MS)?)
        } else {
            self.read_replies(accept, OVERLAY_STATUS_TIMEOUT_MS).0
        };
        if let Some(text) = init
            && text.contains(OVERLAY_INIT_FAILED)
        {
            return Err(format!(
                "The panel could not initialize its overlay: {text}."
            ));
        }

        // An update sent 8 ms after init was dropped; 1.5 s reached the glass.
        std::thread::sleep(Duration::from_millis(OVERLAY_SETTLE_MS));
        self.send(command::QUERY_STATUS)?;

        self.previous_overlay = Some(transparent);
        // A fresh init re-renders from scratch, so a count carried over from
        // the last session is meaningless.
        self.overlay_count = 0;
        self.last_render_cnt = None;
        self.unanswered_queries = 0;
        Ok(())
    }

    /// Asks the panel how its overlay is doing; the only channel on which a
    /// stopped overlay says so.
    fn ask_status(&mut self) -> Option<protocol::PanelStatus> {
        if self.send(command::QUERY_STATUS).is_err() {
            return None;
        }
        self.read_replies(parse_panel_status, OVERLAY_STATUS_TIMEOUT_MS)
            .0
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

    fn plays_video(&self) -> bool {
        true
    }

    /// Uploads a clip the panel does not already hold, plays it, and
    /// initializes the transparent overlay a UI is drawn over.
    fn start_video(&mut self, clip: VideoClip<'_>) -> Result<(), String> {
        if !self.opened {
            return Err(format!("{LABEL} is not open."));
        }
        let VideoClip::Mp4(bytes) = clip else {
            return Err("That panel takes a stored MP4 clip, not a raw stream.".to_owned());
        };
        let remote_path = format!("{VIDEO_DIRECTORY}/yunari-{}.mp4", short_hash(bytes));

        let stored = self.file_size(&remote_path)?;
        if stored != bytes.len() {
            if stored > 0 {
                self.send_file(command::DELETE_FILE, &remote_path, None)?;
            }
            self.upload(&remote_path, bytes)?;
        }
        self.send_file(command::START_VIDEO, &remote_path, None)?;

        // The panel is looping the clip from here. A failure after this point
        // would leave it playing with no host behind it, so stop it and keep
        // the original error.
        let failed = |panel: &mut Self| {
            let _ = panel.send(command::STOP_VIDEO);
        };
        if let Err(error) = self.read_until_accept(
            "\"play_video_success\"",
            |text| text.contains("play_video_success").then_some(()),
            FILE_TIMEOUT_MS,
        ) {
            failed(self);
            return Err(error);
        }
        if let Err(error) = self.init_overlay(true, true) {
            failed(self);
            return Err(error);
        }

        if let Some(previous) = self.previous_video_path.clone()
            && previous != remote_path
        {
            self.send_file(command::DELETE_FILE, &previous, None)?;
        }
        self.previous_video_path = Some(remote_path);
        Ok(())
    }

    /// One overlay diff over the playing video, followed by the panel's own
    /// verdict: its status is the brake, and it is how a stalled overlay says
    /// so before the glass freezes for good.
    fn present_overlay(&mut self, png: &[u8]) -> Result<(), String> {
        if !self.opened {
            return Err(format!("{LABEL} is not open."));
        }
        let Some(previous) = self.previous_overlay.clone() else {
            return Err(
                "The panel's overlay is not initialized; start the video before sending an update."
                    .to_owned(),
            );
        };
        let current = decode_overlay(png, self.geometry)?;
        let width = self.geometry.width as usize;
        let height = self.geometry.height as usize;
        let update = overlay::overlay_update(&previous, &current, width, height)?;

        let count = self.overlay_count;
        self.send(&overlay::overlay_header(
            update.image_size,
            count,
            update.visible_bytes,
        ))?;
        self.send(&update.body)?;

        let status = self.ask_status();
        self.unanswered_queries = if status.is_none() {
            self.unanswered_queries + 1
        } else {
            0
        };
        let mut stall = None;
        match status {
            None if self.unanswered_queries >= UNANSWERED_QUERIES_BEFORE_REINIT => {
                stall = Some(format!(
                    "{} unanswered status queries",
                    self.unanswered_queries
                ));
            }
            None => {}
            Some(status) => {
                if status.resend {
                    stall = Some("the panel asked for a full repaint".to_owned());
                } else if self.last_render_cnt == Some(status.render_cnt) {
                    stall = Some(format!(
                        "its render counter stopped at {}",
                        status.render_cnt
                    ));
                }
                self.last_render_cnt = Some(status.render_cnt);
            }
        }

        if let Some(reason) = stall {
            eprintln!("TURZX overlay re-init: {reason}.");
            self.init_overlay(false, false)?;
            // The repaint is part of the recovery: the base the init restored
            // is transparent, so this frame carries the whole UI.
            std::thread::sleep(Duration::from_millis(OVERLAY_SETTLE_MS));
            let full = overlay::overlay_update(&previous, &current, width, height)?;
            self.send(&overlay::overlay_header(
                full.image_size,
                0,
                full.visible_bytes,
            ))?;
            self.send(&full.body)?;
            self.overlay_count = 1;
            self.previous_overlay = Some(current);
            return Ok(());
        }

        // Advance only once both writes landed, so a failed update does not
        // skip a number.
        self.overlay_count = count + 1;
        self.previous_overlay = Some(current);
        Ok(())
    }

    fn stop_video(&mut self) {
        let _ = self.send(command::STOP_VIDEO);
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

/// The first eight bytes of the clip's SHA-256, as the name the panel keeps
/// it under: the same bytes reuse the stored file, different bytes replace it.
fn short_hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The overlay PNG as straight RGBA of the panel's own size; anything else is
/// not the frame the panel expects.
fn decode_overlay(png: &[u8], geometry: Geometry) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| format!("That frame is not a PNG: {error}."))?;
    let rgba = image.to_rgba8();
    if rgba.width() != geometry.width || rgba.height() != geometry.height {
        return Err(format!(
            "That frame is {}x{}, not the panel's {}x{}.",
            rgba.width(),
            rgba.height(),
            geometry.width,
            geometry.height
        ));
    }
    Ok(rgba.into_raw())
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

    fn mp4_clip(bytes: &[u8]) -> VideoClip<'_> {
        VideoClip::Mp4(bytes)
    }

    fn position(writes: &[Vec<u8>], command: &[u8]) -> Option<usize> {
        writes.iter().position(|write| write.starts_with(command))
    }

    #[test]
    fn a_clip_is_uploaded_then_started_and_its_overlay_initialized() {
        let bytes = b"an mp4 the panel does not hold";
        let mut panel = TurzxPanel::new(
            FakeSerial::new(&[
                b"chs_5inch.dev1_rom1.88\0",
                b"needReSend:0|renderCnt:1\0",
                b"0\0",
                b"create_success\0",
                b"file_rev_done\0",
                b"play_video_success\0",
                b"seq_png_init_sucessneedReSend:0|renderCnt:3\0",
            ]),
            GEOMETRY,
        );
        panel.open().unwrap();
        panel.serial.writes.clear();
        panel.start_video(mp4_clip(bytes)).unwrap();

        let writes = &panel.serial.writes;
        let size = position(writes, command::GET_FILE_SIZE).expect("a size probe");
        let upload = position(writes, command::UPLOAD_FILE).expect("an upload");
        let start = position(writes, command::START_VIDEO).expect("a start");
        let init = position(writes, command::INIT_VIDEO_OVERLAY).expect("an init");
        assert!(size < upload && upload < start && start < init);

        let header = &writes[upload];
        let length = header[6] as usize;
        let path = std::str::from_utf8(&header[10..10 + length]).unwrap();
        assert_eq!(
            path,
            format!("{VIDEO_DIRECTORY}/yunari-{}.mp4", short_hash(bytes))
        );
        let declared = u32::from_le_bytes(header[10 + length..14 + length].try_into().unwrap());
        assert_eq!(declared as usize, bytes.len());
        assert!(position(writes, command::OVERLAY_TERMINATE).is_some());
        assert!(position(writes, command::DISPLAY_ON_VIDEO).is_some());
    }

    #[test]
    fn a_clip_the_panel_already_holds_is_not_uploaded_again() {
        let bytes = b"an mp4 already stored";
        let stored = format!("{}\0", bytes.len());
        let mut panel = TurzxPanel::new(
            FakeSerial::new(&[
                b"chs_5inch.dev1_rom1.88\0",
                b"needReSend:0|renderCnt:1\0",
                stored.as_bytes(),
                b"play_video_success\0",
                b"seq_png_init_sucess\0",
            ]),
            GEOMETRY,
        );
        panel.open().unwrap();
        panel.serial.writes.clear();
        panel.start_video(mp4_clip(bytes)).unwrap();

        assert!(position(&panel.serial.writes, command::UPLOAD_FILE).is_none());
        assert!(position(&panel.serial.writes, command::DELETE_FILE).is_none());
        assert!(position(&panel.serial.writes, command::START_VIDEO).is_some());
    }

    /// An overlay-sized transparent frame with one red pixel.
    fn overlay_png(geometry: Geometry) -> Vec<u8> {
        let mut image = image::RgbaImage::from_pixel(
            geometry.width,
            geometry.height,
            image::Rgba([0, 0, 0, 0]),
        );
        image.put_pixel(1, 2, image::Rgba([255, 0, 0, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    fn panel_with_overlay(replies: &[&[u8]]) -> TurzxPanel<FakeSerial> {
        let mut panel = opened(replies);
        panel.previous_overlay = Some(vec![
            0_u8;
            GEOMETRY.width as usize * GEOMETRY.height as usize * 4
        ]);
        panel.serial.writes.clear();
        panel
    }

    #[test]
    fn one_overlay_diff_carries_its_changed_pixel_and_the_count() {
        let mut panel =
            panel_with_overlay(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        panel.present_overlay(&overlay_png(GEOMETRY)).unwrap();

        let header = panel
            .serial
            .writes
            .iter()
            .find(|write| write.starts_with(&[0xcc, 0xef, 0x69, 0x00]))
            .expect("an update header");
        assert_eq!(&header[10..14], &0_u32.to_be_bytes());
        // The visible list has the one red pixel's segment, after its pixel.
        assert_eq!(&header[14..18], &5_u32.to_be_bytes());
        assert_eq!(panel.overlay_count, 1);
    }

    #[test]
    fn an_overlay_of_the_wrong_size_is_refused_before_it_reaches_the_wire() {
        let mut panel =
            panel_with_overlay(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        let mut image = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 0]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(std::mem::take(&mut image))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            panel.present_overlay(&png).unwrap_err(),
            "That frame is 4x4, not the panel's 800x480."
        );
        assert!(panel.serial.writes.is_empty());
    }

    #[test]
    fn an_overlay_that_is_not_initialized_is_refused_by_that_fact() {
        let mut panel = opened(&[b"chs_5inch.dev1_rom1.88", b"needReSend:0|renderCnt:1"]);
        assert_eq!(
            panel.present_overlay(&overlay_png(GEOMETRY)).unwrap_err(),
            "The panel's overlay is not initialized; start the video before sending an update."
        );
    }

    #[test]
    fn a_panel_asking_for_a_repaint_is_reinitialized_and_repainted() {
        let mut panel = panel_with_overlay(&[
            b"chs_5inch.dev1_rom1.88",
            b"needReSend:0|renderCnt:1",
            b"needReSend:1|renderCnt:9",
            b"seq_png_init_sucess\0",
        ]);
        panel.present_overlay(&overlay_png(GEOMETRY)).unwrap();

        let headers: Vec<&Vec<u8>> = panel
            .serial
            .writes
            .iter()
            .filter(|write| write.starts_with(&[0xcc, 0xef, 0x69, 0x00]))
            .collect();
        assert_eq!(headers.len(), 2);
        assert_eq!(&headers[0][10..14], &0_u32.to_be_bytes());
        assert_eq!(&headers[1][10..14], &0_u32.to_be_bytes());
        assert!(position(&panel.serial.writes, command::INIT_VIDEO_OVERLAY).is_some());
        assert_eq!(panel.overlay_count, 1);
    }
}
