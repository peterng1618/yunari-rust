//! The TURZX 5" panel's wire format: every write padded to a 250-byte multiple,
//! pixels sent as BGRA in 249-byte chunks, and a text HELLO that names the
//! panel.
//!
//! Pure, so the verified probe bytes are testable without hardware.

/// The frame size every write is padded to; a shorter command is ignored.
pub const FRAME_SIZE: usize = 250;
/// A 249-byte chunk plus its `0x00` join is exactly one frame.
const PAYLOAD_CHUNK: usize = 249;

/// The panel's HELLO answer, once its trailing NULs and whitespace are trimmed.
pub struct Hello {
    /// `chs_` plus the firmware's model name, as the reply wrote it.
    pub model: String,
    pub device: String,
    pub firmware: String,
}

/// The panel's own verdict on its overlay, as QUERY_STATUS answers it.
#[derive(Debug, PartialEq, Eq)]
pub struct PanelStatus {
    /// True when the panel dropped pixels and wants a full repaint.
    pub resend: bool,
    /// Advances on its own while the panel renders; two answers carrying the
    /// same count are a panel whose render task has stopped.
    pub render_cnt: u32,
}

/// `bytes` padded to a 250-byte multiple with zeroes.
pub fn frame(bytes: &[u8]) -> Vec<u8> {
    padded(bytes, 0x00)
}

/// `bytes` padded to a 250-byte multiple with the bitmap fill.
pub fn bitmap_frame(bytes: &[u8]) -> Vec<u8> {
    padded(bytes, 0x2c)
}

fn padded(bytes: &[u8], fill: u8) -> Vec<u8> {
    let size = bytes.len().div_ceil(FRAME_SIZE) * FRAME_SIZE;
    let mut framed = vec![fill; size];
    framed[..bytes.len()].copy_from_slice(bytes);
    framed
}

/// The identity a HELLO reply carries, or `None` for any other answer.
pub fn parse_hello(reply: &[u8]) -> Option<Hello> {
    let text = String::from_utf8_lossy(reply);
    parse_hello_text(text.trim_end_matches(['\0', ' ', '\t', '\r', '\n']))
}

/// The identity an already-decoded HELLO answer carries, or `None`.
pub fn parse_hello_text(text: &str) -> Option<Hello> {
    let rest = text.strip_prefix("chs_")?;
    let (model, rest) = rest.split_once('.')?;
    let (device, firmware) = rest.split_once("_rom")?;
    let model_ok = !model.is_empty()
        && model
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    let device_ok = device
        .strip_prefix("dev")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()));
    let firmware_ok = !firmware.is_empty()
        && firmware
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.');
    (model_ok && device_ok && firmware_ok).then(|| Hello {
        model: format!("chs_{model}"),
        device: device.to_owned(),
        firmware: firmware.to_owned(),
    })
}

/// The status an already-decoded reply carries, or `None`.
pub fn parse_panel_status(text: &str) -> Option<PanelStatus> {
    let (_, after) = text.split_once("needReSend:")?;
    let (resend, after) = after.split_once('|')?;
    let (_, render) = after.split_once("renderCnt:")?;
    let digits = |text: &str| -> String { text.chars().take_while(char::is_ascii_digit).collect() };
    let resend = digits(resend);
    let render_cnt = digits(render).parse().ok()?;
    (!resend.is_empty()).then(|| PanelStatus {
        resend: resend != "0",
        render_cnt,
    })
}

/// Straight RGBA as the panel's payload: each pixel reordered to BGRA, chunked
/// at 249 bytes with one `0x00` between chunks, then padded to a frame multiple.
///
/// The chunked form is the wire's, not a convenience: a full 800x480 frame is
/// 1,542,250 bytes, which is exactly this join plus the final padding.
pub fn bgra_payload(rgba: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let chunks = bgra.len().div_ceil(PAYLOAD_CHUNK);
    let mut joined = Vec::with_capacity(bgra.len() + chunks.saturating_sub(1));
    for (index, chunk) in bgra.chunks(PAYLOAD_CHUNK).enumerate() {
        if index > 0 {
            joined.push(0x00);
        }
        joined.extend_from_slice(chunk);
    }
    frame(&joined)
}

/// A text reply, cut at the first NUL.
///
/// The wire pads replies with NULs, and a burst can carry a drained reply's
/// tail followed by this command's answer.
pub fn decode_reply(reply: &[u8]) -> String {
    let text = String::from_utf8_lossy(reply);
    text.split('\0')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

/// The verified command table. Only non-destructive commands are named.
pub mod command {
    use super::FRAME_SIZE;

    pub const HELLO: &[u8] = &[
        0x01, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xc5, 0xd3,
    ];
    pub const STOP_VIDEO: &[u8] = &[0x79, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01];
    pub const STOP_MEDIA: &[u8] = &[0x96, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01];
    /// The level byte is the caller's, appended to this prefix.
    pub const BRIGHTNESS: &[u8] = &[0x7b, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
    /// Five settings bytes follow: brightness weight, start mode, a zero the
    /// vendor never varies, image flip and sleep delay.
    pub const PANEL_SETTINGS: &[u8] = &[0x7d, 0xef, 0x69, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00];
    pub const RESTART: &[u8] = &[0x84, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01];
    pub const PRE_UPDATE: &[u8] = &[0x86, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01];
    /// The six-byte constant plus the big-endian width²/64 field (480*480/64
    /// = 3600): a zero field is a full-image send the panel drops.
    pub const DISPLAY_BITMAP_5INCH: &[u8] = &[0xc8, 0xef, 0x69, 0x00, 0x17, 0x70, 0x0e, 0x10];
    pub const DISPLAY_ON_VIDEO: &[u8] = &[0xca, 0xef, 0x69, 0x00, 0x17, 0x70];
    pub const QUERY_STATUS: &[u8] = &[0xcf, 0xef, 0x69, 0x00, 0x00, 0x00, 0x01];

    /// `START_DISPLAY_BITMAP`: one frame of `0x2c`, the padding the panel
    /// expects before a bitmap and the only command that is already framed.
    pub fn start_display_bitmap() -> Vec<u8> {
        vec![0x2c; FRAME_SIZE]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_is_a_frame_multiple() {
        assert_eq!(frame(command::HELLO).len(), FRAME_SIZE);
        assert_eq!(frame(&[0x01]).len(), FRAME_SIZE);
        assert!(frame(&vec![0x00; FRAME_SIZE]).len() == FRAME_SIZE);
        assert_eq!(frame(&vec![0x00; FRAME_SIZE + 1]).len(), FRAME_SIZE * 2);
        assert_eq!(command::start_display_bitmap().len(), FRAME_SIZE);
        assert_eq!(
            command::start_display_bitmap(),
            frame(&command::start_display_bitmap())
        );
    }

    #[test]
    fn a_still_is_a_prefixed_bitmap() {
        let mut write = Vec::new();
        write.extend_from_slice(&frame(command::PRE_UPDATE));
        write.extend_from_slice(&frame(&command::start_display_bitmap()));
        write.extend_from_slice(&frame(command::DISPLAY_BITMAP_5INCH));
        write.extend_from_slice(&frame(&[0x00; 4]));
        assert_eq!(write.len(), FRAME_SIZE * 4);
    }

    #[test]
    fn a_full_frame_is_one_megabyte_and_a_half_of_chunked_bgra() {
        let rgba = vec![0x11_u8; 800 * 480 * 4];
        assert_eq!(bgra_payload(&rgba).len(), 1_542_250);
    }

    #[test]
    fn a_pixel_gets_its_blue_and_red_swapped() {
        let payload = bgra_payload(&[10, 20, 30, 40]);
        assert_eq!(&payload[..4], &[30, 20, 10, 40]);
    }

    #[test]
    fn hello_names_the_panel_and_refuses_anything_else() {
        let hello = parse_hello(b"chs_5inch.dev1_rom1.88\0\0\0\0\0").expect("a hello");
        assert_eq!(hello.model, "chs_5inch");
        assert_eq!(hello.device, "dev1");
        assert_eq!(hello.firmware, "1.88");

        assert!(parse_hello(b"chs_5inch_rom1.88").is_none());
        assert!(parse_hello(b"OK").is_none());
        assert!(parse_hello(b"chs_5inch.dev1_rom").is_none());
        assert!(parse_hello(b"").is_none());
    }

    #[test]
    fn a_status_reply_carries_the_counters_the_panel_keeps() {
        let status =
            parse_panel_status("QUERY_STATUS needReSend:0|renderCnt:1576").expect("a status");
        assert!(!status.resend);
        assert_eq!(status.render_cnt, 1576);

        let resend = parse_panel_status("needReSend:1|renderCnt:9").expect("a status");
        assert!(resend.resend);

        assert!(parse_panel_status("seq_png_init_sucess").is_none());
    }

    #[test]
    fn a_reply_ends_at_its_first_nul() {
        assert_eq!(decode_reply(b"OK\0stale tail"), "OK");
        assert_eq!(decode_reply(b"OK  \r\n"), "OK");
        assert_eq!(decode_reply(b""), "");
    }
}
