//! The overlay a panel draws over its own playing clip: a per-frame diff of the
//! theme's UI, packed into the vendor's UPDATE_BITMAP payload.
//!
//! A cleared pixel is not re-sent — it leaves the changed list, and its absence
//! from the visible list is what the device erases. Segments, pixel packing and
//! the alignment trailer are the verified probe's, ported byte for byte.

use super::protocol::{FRAME_SIZE, frame};

/// A 249-byte chunk plus its `0x00` join is one frame, as on the bitmap wire.
const PAYLOAD_CHUNK: usize = 249;

/// UPDATE_BITMAP's prefix; the encoder appends the size, count and visible
/// fields.
const UPDATE_BITMAP: [u8; 4] = [0xcc, 0xef, 0x69, 0x00];
/// The raw two-byte terminator every overlay payload closes with.
const TERMINATOR: [u8; 2] = [0xef, 0x69];
/// The seven-byte form an update's trailer takes when the padded payload ends
/// flush: the panel's own re-clear marker, then the ordinary terminator.
const ALIGNED_TRAILER: [u8; 8] = [0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xef, 0x69];

/// One overlay update, ready to frame.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayUpdate {
    /// The value the header's imageSize field carries.
    pub image_size: usize,
    /// The value the header's visiblePixels field carries.
    pub visible_bytes: usize,
    /// The unframed payload, including its alignment trailer.
    pub body: Vec<u8>,
}

/// The trailer an unframed payload of `unframed` bytes closes with, and the
/// aligned form's addition to `imageSize`.
pub fn overlay_trailer(unframed: usize) -> (Vec<u8>, usize) {
    let remainder = unframed % FRAME_SIZE;
    if unframed > FRAME_SIZE && matches!(remainder, 0 | 248 | 249) {
        return (ALIGNED_TRAILER.to_vec(), 7);
    }
    (TERMINATOR.to_vec(), 0)
}

/// The 18-byte UPDATE_BITMAP header, unframed.
pub fn overlay_header(image_size: usize, count: u32, visible_bytes: usize) -> Vec<u8> {
    let mut header = vec![0_u8; 18];
    header[..4].copy_from_slice(&UPDATE_BITMAP);
    write_uint24(&mut header, 4, image_size);
    header[10..14].copy_from_slice(&count.to_be_bytes());
    header[14..18].copy_from_slice(&(visible_bytes as u32).to_be_bytes());
    header
}

/// The per-frame UPDATE_BITMAP payload: the pixels that changed since
/// `previous`, then the spans that are visible at all.
pub fn overlay_update(
    previous: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
) -> Result<OverlayUpdate, String> {
    let expected = width * height * 4;
    if previous.len() != expected {
        return Err(format!(
            "The previous overlay is {} bytes; a {width}x{height} RGBA frame is {expected}.",
            previous.len()
        ));
    }
    if current.len() != expected {
        return Err(format!(
            "The decoded overlay is {} bytes; a {width}x{height} RGBA frame is {expected}.",
            current.len()
        ));
    }

    let mut changed: Vec<Segment> = Vec::new();
    let mut visible: Vec<Segment> = Vec::new();
    for y in 0..height {
        let row = y * width;
        for span in segments_in_row(width, |x| {
            let at = (row + x) * 4;
            if changed_at(previous, current, at) {
                current[at + 3]
            } else {
                0
            }
        }) {
            changed.push(Segment {
                offset: row + span.x,
                width: span.width,
                at: (row + span.x) * 4,
            });
        }
        for span in segments_in_row(width, |x| current[(row + x) * 4 + 3]) {
            visible.push(Segment {
                offset: row + span.x,
                width: span.width,
                at: (row + span.x) * 4,
            });
        }
    }

    let mut message = vec![
        0_u8;
        changed
            .iter()
            .map(|segment| 5 + segment.width * 3)
            .sum::<usize>()
            + visible.len() * 5
    ];
    let mut at = 0;
    for segment in &changed {
        write_uint24(&mut message, at, segment.offset);
        message[at + 3..at + 5].copy_from_slice(&(segment.width as u16).to_be_bytes());
        at += 5;
        for pixel in 0..segment.width {
            let index = segment.at + pixel * 4;
            let alpha = (u32::from(current[index + 3]) * 15 / 255) as u8;
            write_pixel(
                &mut message,
                at,
                current[index],
                current[index + 1],
                current[index + 2],
                alpha,
            );
            at += 3;
        }
    }
    for segment in &visible {
        write_uint24(&mut message, at, segment.offset);
        message[at + 3..at + 5].copy_from_slice(&(segment.width as u16).to_be_bytes());
        at += 5;
    }

    let payload = if message.len() > FRAME_SIZE {
        join_chunks(&message, PAYLOAD_CHUNK)
    } else {
        message.clone()
    };
    let (trailer, extra) = overlay_trailer(payload.len());
    let mut body = payload;
    body.extend_from_slice(&trailer);

    Ok(OverlayUpdate {
        // The size field counts the message before chunk separators, not the
        // body; the aligned form reports +7 even though its trailer is eight
        // bytes long.
        image_size: message.len() + if extra == 0 { 2 } else { 7 },
        visible_bytes: visible.len() * 5 + if extra == 0 { 0 } else { 5 },
        body,
    })
}

/// A visible span of one row, as the reference's segment walk finds it.
#[derive(Debug, Clone, Copy)]
struct Segment {
    /// Pixel index, `y * width + x`.
    offset: usize,
    width: usize,
    /// Byte index of the span's first pixel in an RGBA frame.
    at: usize,
}

#[derive(Debug, Clone, Copy)]
struct Span {
    x: usize,
    width: usize,
}

/// Whether the four RGBA bytes at `at` differ between the two frames.
fn changed_at(previous: &[u8], current: &[u8], at: usize) -> bool {
    previous[at..at + 4] != current[at..at + 4]
}

/// The reference's `_get_visible_segments_numba` walk, literally: a span starts
/// where `alpha > 0` and keeps extending while the next pixel is opaque **or
/// the pixel after it is**, so a one-pixel gap inside a run is kept.
fn segments_in_row(width: usize, alpha: impl Fn(usize) -> u8) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < width {
        if alpha(i) > 0 {
            let x = i;
            let mut j = i + 1;
            while j < width && (alpha(j) > 0 || (j + 1 < width && alpha(j + 1) > 0)) {
                j += 1;
            }
            spans.push(Span { x, width: j - x });
            i = j;
        }
        i += 1;
    }
    spans
}

/// One pixel as three bytes: **blue 6 bits, green 6 bits, red 8 bits, alpha 4**
/// — the packing the vendor's own app sends.
fn write_pixel(out: &mut [u8], at: usize, red: u8, green: u8, blue: u8, alpha: u8) {
    out[at] = (blue & 0xfc) | ((alpha & 0x0c) >> 2);
    out[at + 1] = (green & 0xfc) | (alpha & 0x03);
    out[at + 2] = red;
}

/// `value` as three big-endian bytes at `at`.
fn write_uint24(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at] = ((value >> 16) & 0xff) as u8;
    bytes[at + 1] = ((value >> 8) & 0xff) as u8;
    bytes[at + 2] = (value & 0xff) as u8;
}

/// A message joined at `size` bytes with one `0x00` between chunks.
fn join_chunks(bytes: &[u8], size: usize) -> Vec<u8> {
    let chunks = bytes.len().div_ceil(size);
    let mut joined = Vec::with_capacity(bytes.len() + chunks.saturating_sub(1));
    for (index, chunk) in bytes.chunks(size).enumerate() {
        if index > 0 {
            joined.push(0x00);
        }
        joined.extend_from_slice(chunk);
    }
    joined
}

/// A path command: the six-byte command, a one-byte path length, three nulls,
/// the ASCII path, and — for an upload — the file size as int32 little-endian.
/// Framed separately, because every write the panel reads is 250 bytes.
pub fn file_header(
    command: &[u8],
    remote_path: &str,
    size: Option<u32>,
) -> Result<Vec<u8>, String> {
    let path = remote_path.as_bytes();
    let length = command.len() + 4 + path.len() + usize::from(size.is_some()) * 4;
    if length > FRAME_SIZE {
        return Err(format!("The panel path is too long: {remote_path}."));
    }
    let mut header = vec![0_u8; length];
    header[..command.len()].copy_from_slice(command);
    header[command.len()] = path.len() as u8;
    header[command.len() + 4..command.len() + 4 + path.len()].copy_from_slice(path);
    if let Some(size) = size {
        let at = command.len() + 4 + path.len();
        header[at..at + 4].copy_from_slice(&(size as i32).to_le_bytes());
    }
    Ok(frame(&header))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 4;

    fn blank(width: usize, height: usize) -> Vec<u8> {
        vec![0_u8; width * height * 4]
    }

    fn red(width: usize, height: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
        let mut pixels = blank(width, height);
        for row in y..y + h {
            for column in x..x + w {
                let at = (row * width + column) * 4;
                pixels[at] = 255;
                pixels[at + 3] = 255;
            }
        }
        pixels
    }

    fn grey(width: usize, height: usize, value: u8) -> Vec<u8> {
        let mut pixels = blank(width, height);
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[0] = value;
            pixel[1] = value;
            pixel[2] = value;
            pixel[3] = 255;
        }
        pixels
    }

    #[test]
    fn a_new_rectangle_is_its_pixels_plus_a_visible_segment() {
        let update = overlay_update(&blank(WIDTH, 2), &red(WIDTH, 2, 1, 0, 2, 1), WIDTH, 2)
            .expect("encoded");
        // One row-0 changed segment: offset 3 bytes, width 2, then two pixels.
        assert_eq!(&update.body[..5], &[0, 0, 1, 0, 2]);
        // A fully opaque red pixel: blue 0, green 0, red 255, alpha 15.
        assert_eq!(&update.body[5..11], &[0x03, 0x03, 0xff, 0x03, 0x03, 0xff]);
        // The visible list repeats that segment after the changed pixels.
        assert_eq!(&update.body[11..16], &[0, 0, 1, 0, 2]);
        assert_eq!(update.visible_bytes, 5);
        assert_eq!(update.image_size, 5 + 6 + 5 + 2);
    }

    #[test]
    fn sub_nibble_colour_is_kept_rather_than_quantized() {
        let update =
            overlay_update(&blank(WIDTH, 4), &grey(WIDTH, 4, 128), WIDTH, 4).expect("encoded");
        let mut at = 0;
        for _ in 0..4 {
            at += 5;
            for _ in 0..WIDTH {
                assert_eq!(&update.body[at..at + 3], &[0x83, 0x83, 0x80]);
                at += 3;
            }
        }
    }

    #[test]
    fn a_cleared_pixel_leaves_the_visible_list_and_is_not_resent() {
        let update = overlay_update(
            &red(WIDTH, 2, 0, 0, 2, 1),
            &red(WIDTH, 2, 2, 0, 2, 1),
            WIDTH,
            2,
        )
        .expect("encoded");
        assert_eq!(&update.body[..5], &[0, 0, 2, 0, 2]);
        assert_eq!(&update.body[11..16], &[0, 0, 2, 0, 2]);
    }

    #[test]
    fn a_frame_that_is_not_the_geometrys_rgba_size_is_refused_by_its_size() {
        let error = overlay_update(&[0; 3], &blank(WIDTH, 2), WIDTH, 2).unwrap_err();
        assert_eq!(
            error,
            "The previous overlay is 3 bytes; a 4x2 RGBA frame is 32."
        );
        let error = overlay_update(&blank(WIDTH, 2), &[0; 3], WIDTH, 2).unwrap_err();
        assert_eq!(
            error,
            "The decoded overlay is 3 bytes; a 4x2 RGBA frame is 32."
        );
    }

    #[test]
    fn a_large_message_is_chunked_and_reports_the_aligned_size() {
        let update = overlay_update(&blank(800, 480), &red(800, 480, 0, 0, 800, 221), 800, 480)
            .expect("encoded");
        assert_eq!(update.image_size, 532_610 + 7);
        assert_eq!(update.visible_bytes, 221 * 5 + 5);
        assert_eq!(update.body.len(), 534_748 + 8);
        assert_eq!(update.body[249], 0x00);
        assert_eq!(update.body[499], 0x00);
        assert_eq!(
            &update.body[update.body.len() - 8..],
            &[0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xef, 0x69]
        );
    }

    #[test]
    fn a_message_of_exactly_250_bytes_is_not_chunked() {
        let update = overlay_update(&blank(800, 480), &red(800, 480, 0, 0, 80, 1), 800, 480)
            .expect("encoded");
        // 5 + 80*3 changed bytes + 5 visible = 250, then its two-byte trailer.
        assert_eq!(update.image_size, 252);
        assert_eq!(update.body.len(), 252);
        assert_eq!(
            &update.body[245..],
            &[0x00, 0x00, 0x00, 0x00, 0x50, 0xef, 0x69]
        );
    }

    #[test]
    fn offsets_and_widths_are_big_endian() {
        let update =
            overlay_update(&blank(800, 1), &red(800, 1, 300, 0, 260, 1), 800, 1).expect("encoded");
        assert_eq!(&update.body[..5], &[0x00, 0x01, 0x2c, 0x01, 0x04]);
    }

    #[test]
    fn a_one_pixel_transparent_gap_is_kept_through_the_lookahead() {
        let mut current = blank(WIDTH, 2);
        for x in [0, 2] {
            current[x * 4] = 255;
            current[x * 4 + 3] = 255;
        }
        let update = overlay_update(&blank(WIDTH, 2), &current, WIDTH, 2).expect("encoded");
        assert_eq!(&update.body[..5], &[0, 0, 0, 0, 3]);
        assert_eq!(
            &update.body[5..14],
            &[0x03, 0x03, 0xff, 0x00, 0x00, 0x00, 0x03, 0x03, 0xff]
        );
        assert_eq!(&update.body[14..19], &[0, 0, 0, 0, 3]);
        assert_eq!(update.body.len(), 19 + 2);
        assert_eq!(update.visible_bytes, 5);
    }

    #[test]
    fn a_row_end_does_not_look_into_the_next_row() {
        let mut current = blank(WIDTH, 2);
        for index in [3 * 4, 4 * 4] {
            current[index] = 255;
            current[index + 3] = 255;
        }
        let update = overlay_update(&blank(WIDTH, 2), &current, WIDTH, 2).expect("encoded");
        assert_eq!(&update.body[..5], &[0, 0, 3, 0, 1]);
        assert_eq!(&update.body[8..13], &[0, 0, 4, 0, 1]);
        assert_eq!(&update.body[16..21], &[0, 0, 3, 0, 1]);
        assert_eq!(&update.body[21..26], &[0, 0, 4, 0, 1]);
        assert_eq!(update.body.len(), 26 + 2);
    }

    #[test]
    fn a_segments_offset_uses_the_row_stride() {
        let update = overlay_update(&blank(WIDTH, 4), &red(WIDTH, 4, 1, 2, 1, 1), WIDTH, 4)
            .expect("encoded");
        // Pixel (1,2) is offset 2 * 4 + 1 = 9.
        assert_eq!(&update.body[..5], &[0, 0, 9, 0, 1]);
    }

    #[test]
    fn the_trailer_aligns_only_a_payload_that_ends_flush() {
        for (length, size, extra) in [
            (250_usize, 2_usize, 0_usize),
            (251, 2, 0),
            (300, 2, 0),
            (500, 8, 7),
            (498, 8, 7),
            (499, 8, 7),
        ] {
            let (bytes, reported) = overlay_trailer(length);
            assert_eq!(bytes.len(), size, "{length}");
            assert_eq!(reported, extra, "{length}");
            assert_eq!(&bytes[bytes.len() - 2..], &[0xef, 0x69]);
        }
    }

    #[test]
    fn the_header_carries_the_size_count_and_visible_bytes() {
        let header = overlay_header(532_610, 7, 1_110);
        assert_eq!(&header[..4], &[0xcc, 0xef, 0x69, 0x00]);
        assert_eq!(&header[4..7], &[0x08, 0x20, 0x82]);
        assert_eq!(&header[10..14], &7_u32.to_be_bytes());
        assert_eq!(&header[14..18], &1_110_u32.to_be_bytes());
    }

    #[test]
    fn a_file_header_names_its_path_and_size_inside_one_frame() {
        let header = file_header(
            &[0x6f, 0xef, 0x69, 0x00, 0x00, 0x00],
            "/mnt/SDCARD/video/a.mp4",
            Some(9),
        )
        .expect("fits");
        assert_eq!(header.len() % FRAME_SIZE, 0);
        assert_eq!(header[6], "/mnt/SDCARD/video/a.mp4".len() as u8);
        assert_eq!(&header[10..10 + 23], b"/mnt/SDCARD/video/a.mp4");
        assert_eq!(&header[33..37], &9_i32.to_le_bytes());
        let too_long = "x".repeat(FRAME_SIZE);
        assert!(file_header(&[0x6f, 0xef, 0x69, 0x00, 0x00, 0x00], &too_long, None).is_err());
    }
}
