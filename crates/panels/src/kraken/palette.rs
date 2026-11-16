//! One Kraken Z frame in the format its streaming transfer takes: a 256-entry
//! `R,G,B,A` palette, then one 8-bit palette index per pixel.
//!
//! **This is the wire format, not an image format.** CAM's own stream is this
//! layout, 1,024 palette bytes plus 102,400 indices, and the firmware ignores
//! the fourth palette byte; it is written as 0.
//!
//! **The palette is rebuilt for every frame and the quantization error is
//! diffused.** A fixed palette would band on gradients and could not follow a
//! photograph. Floyd–Steinberg is applied in the capture's own orientation,
//! because the display record owns rotation.
//!
//! **A 5-bit histogram is the whole search space.** Two colours within 8 of
//! each other share a bin. Median cut splits the populated bins into at most
//! 256 boxes, and each box's weighted mean becomes a palette entry. A bin maps
//! straight to its box; a dithered colour outside every populated bin searches
//! the palette once and caches the answer for the rest of the frame.

use crate::kraken::device::SIZE;

/// The frame's own side, as a `usize` for the pixel plane.
pub const KRAKEN_SIZE: usize = SIZE as usize;
/// 256 entries of four bytes; the firmware ignores the fourth.
pub const PALETTE_BYTES: usize = 256 * 4;
pub const FRAME_BYTES: usize = PALETTE_BYTES + KRAKEN_SIZE * KRAKEN_SIZE;

/// 5 bits per channel is the histogram's whole key space.
const BIN_COUNT: usize = 32 * 32 * 32;
const BOX_COUNT: usize = 256;
const NO_ENTRY: i32 = -1;

#[derive(Debug, Clone, Copy)]
struct Point {
    key: usize,
    count: u32,
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Debug, Clone, Copy)]
struct Box {
    /// Half-open range into the points array.
    start: usize,
    end: usize,
    count: u64,
    /// 0 = red, 1 = green, 2 = blue.
    axis: usize,
    range: u8,
}

/// The frame the device reads: the palette, then one index per pixel.
pub fn to_paletted_frame(rgba: &[u8]) -> Vec<u8> {
    let mut histogram = vec![0_u32; BIN_COUNT];
    // **Exact channel sums, not the bin centre.** Floyd–Steinberg's weights sum
    // to 1, so an entry off by one ramp amplifies the error until the nearest
    // entry flips, which shows as speckles in a flat fill. Summing the real
    // bytes makes a flat region's mean its own colour.
    let mut red_sum = vec![0_u64; BIN_COUNT];
    let mut green_sum = vec![0_u64; BIN_COUNT];
    let mut blue_sum = vec![0_u64; BIN_COUNT];

    for pixel in 0..KRAKEN_SIZE * KRAKEN_SIZE {
        let at = pixel * 4;
        let red = rgba.get(at).copied().unwrap_or(0);
        let green = rgba.get(at + 1).copied().unwrap_or(0);
        let blue = rgba.get(at + 2).copied().unwrap_or(0);
        let bin = bin_of(red, green, blue);
        histogram[bin] += 1;
        red_sum[bin] += u64::from(red);
        green_sum[bin] += u64::from(green);
        blue_sum[bin] += u64::from(blue);
    }

    let mut points: Vec<Point> = Vec::new();
    for (key, count) in histogram.iter().enumerate() {
        let count = *count;
        if count == 0 {
            continue;
        }
        points.push(Point {
            key,
            count,
            red: ((key >> 10) & 0x1f) as u8,
            green: ((key >> 5) & 0x1f) as u8,
            blue: (key & 0x1f) as u8,
        });
    }

    let boxes = median_cut(&mut points);
    let mut frame = vec![0_u8; FRAME_BYTES];
    let mut lookup = vec![NO_ENTRY; BIN_COUNT];

    for (index, taken) in boxes.iter().enumerate() {
        let at = index * 4;
        let (red, green, blue) = mean_of(&points, taken, &red_sum, &green_sum, &blue_sum);
        frame[at] = red;
        frame[at + 1] = green;
        frame[at + 2] = blue;
        // Alpha is the firmware's padding, not the captured alpha.
        frame[at + 3] = 0;

        for point in taken.start..taken.end {
            lookup[points[point].key] = index as i32;
        }
    }

    let (palette, plane) = frame.split_at_mut(PALETTE_BYTES);
    diffuse(rgba, plane, palette, &mut lookup, boxes.len());
    frame
}

/// Every pixel's palette index, after Floyd–Steinberg diffusion.
fn diffuse(rgba: &[u8], plane: &mut [u8], palette: &[u8], lookup: &mut [i32], palette_size: usize) {
    let mut error = vec![0_f32; KRAKEN_SIZE * KRAKEN_SIZE * 3];

    for y in 0..KRAKEN_SIZE {
        for x in 0..KRAKEN_SIZE {
            let pixel = y * KRAKEN_SIZE + x;
            let at = pixel * 4;
            let carry = pixel * 3;
            let red = clamp(f32::from(rgba.get(at).copied().unwrap_or(0)) + error[carry]);
            let green = clamp(f32::from(rgba.get(at + 1).copied().unwrap_or(0)) + error[carry + 1]);
            let blue = clamp(f32::from(rgba.get(at + 2).copied().unwrap_or(0)) + error[carry + 2]);
            let bin = bin_of(red as u8, green as u8, blue as u8);

            let mut index = lookup[bin];
            if index == NO_ENTRY {
                index = nearest(palette, palette_size, red, green, blue) as i32;
                lookup[bin] = index;
            }
            plane[pixel] = index as u8;

            let entry = index as usize * 4;
            diffuse_error(
                &mut error,
                pixel,
                x,
                y,
                red - f32::from(palette[entry]),
                green - f32::from(palette[entry + 1]),
                blue - f32::from(palette[entry + 2]),
            );
        }
    }
}

/// Clamps a channel to a byte after the diffusion error is added.
fn clamp(value: f32) -> f32 {
    value.clamp(0.0, 255.0)
}

/// Floyd–Steinberg's 7/16, 3/16, 5/16 and 1/16 weights, in place.
fn diffuse_error(
    error: &mut [f32],
    pixel: usize,
    x: usize,
    y: usize,
    red: f32,
    green: f32,
    blue: f32,
) {
    if x + 1 < KRAKEN_SIZE {
        add(error, (pixel + 1) * 3, red, green, blue, 7.0 / 16.0);
    }
    if y + 1 >= KRAKEN_SIZE {
        return;
    }
    if x > 0 {
        add(
            error,
            (pixel + KRAKEN_SIZE - 1) * 3,
            red,
            green,
            blue,
            3.0 / 16.0,
        );
    }
    add(
        error,
        (pixel + KRAKEN_SIZE) * 3,
        red,
        green,
        blue,
        5.0 / 16.0,
    );
    if x + 1 < KRAKEN_SIZE {
        add(
            error,
            (pixel + KRAKEN_SIZE + 1) * 3,
            red,
            green,
            blue,
            1.0 / 16.0,
        );
    }
}

fn add(error: &mut [f32], at: usize, red: f32, green: f32, blue: f32, weight: f32) {
    error[at] += red * weight;
    error[at + 1] += green * weight;
    error[at + 2] += blue * weight;
}

/// The nearest palette entry, by squared distance in RGB.
fn nearest(palette: &[u8], palette_size: usize, red: f32, green: f32, blue: f32) -> usize {
    let mut best = 0;
    let mut distance = f32::INFINITY;

    for index in 0..palette_size {
        let at = index * 4;
        let dr = red - f32::from(palette[at]);
        let dg = green - f32::from(palette[at + 1]);
        let db = blue - f32::from(palette[at + 2]);
        let candidate = dr * dr + dg * dg + db * db;
        if candidate < distance {
            distance = candidate;
            best = index;
        }
    }
    best
}

fn bin_of(red: u8, green: u8, blue: u8) -> usize {
    (usize::from(red >> 3) << 10) | (usize::from(green >> 3) << 5) | usize::from(blue >> 3)
}

/// The box's widest channel and the population it holds.
fn measure(points: &[Point], start: usize, end: usize) -> Box {
    let mut red_low = 31_u8;
    let mut red_high = 0_u8;
    let mut green_low = 31_u8;
    let mut green_high = 0_u8;
    let mut blue_low = 31_u8;
    let mut blue_high = 0_u8;
    let mut count = 0_u64;

    for point in &points[start..end] {
        red_low = red_low.min(point.red);
        red_high = red_high.max(point.red);
        green_low = green_low.min(point.green);
        green_high = green_high.max(point.green);
        blue_low = blue_low.min(point.blue);
        blue_high = blue_high.max(point.blue);
        count += u64::from(point.count);
    }

    let spans = [
        red_high.saturating_sub(red_low),
        green_high.saturating_sub(green_low),
        blue_high.saturating_sub(blue_low),
    ];
    let axis = spans
        .iter()
        .enumerate()
        .max_by_key(|(_, span)| **span)
        .map(|(index, _)| index)
        .unwrap_or(0);
    Box {
        start,
        end,
        count,
        axis,
        range: spans[axis],
    }
}

/// Median cut: split the box with the most pixels along its widest channel
/// until there are 256 boxes, or nothing can be split.
fn median_cut(points: &mut [Point]) -> Vec<Box> {
    let mut boxes = vec![measure(points, 0, points.len())];

    while boxes.len() < BOX_COUNT {
        let mut target: Option<usize> = None;
        for (index, item) in boxes.iter().enumerate() {
            if item.range == 0 || item.end - item.start < 2 {
                continue;
            }
            if target.is_none_or(|best| {
                item.count * u64::from(item.range)
                    > boxes[best].count * u64::from(boxes[best].range)
            }) {
                target = Some(index);
            }
        }
        let Some(target) = target else {
            break;
        };

        let item = boxes[target];
        // A box holding one dominant colour can reach half its population at
        // its last point, which is still a valid cut: keep one point aside
        // rather than leaving the box whole.
        let split = split_at(points, item).clamp(item.start + 1, item.end - 1);
        let first = measure(points, item.start, split);
        let second = measure(points, split, item.end);
        boxes.splice(target..=target, [first, second]);
    }
    boxes
}

/// Sorts the box along its widest channel and returns the weighted middle.
fn split_at(points: &mut [Point], item: Box) -> usize {
    let channel = |point: &Point| match item.axis {
        0 => point.red,
        1 => point.green,
        _ => point.blue,
    };
    points[item.start..item.end].sort_by_key(channel);

    let half = item.count as f64 / 2.0;
    let mut seen = 0_u64;
    for (offset, point) in points[item.start..item.end].iter().enumerate() {
        seen += u64::from(point.count);
        if seen as f64 >= half {
            return (item.start + offset + 1).min(item.end);
        }
    }
    item.end
}

/// The box's colour as the weighted mean of its real pixels, rounded to 8 bits
/// per channel. The 5-bit bin only groups; the sums are the true bytes.
fn mean_of(
    points: &[Point],
    item: &Box,
    red_sum: &[u64],
    green_sum: &[u64],
    blue_sum: &[u64],
) -> (u8, u8, u8) {
    let mut red = 0_u64;
    let mut green = 0_u64;
    let mut blue = 0_u64;
    let mut count = 0_u64;

    for point in &points[item.start..item.end] {
        red += red_sum[point.key];
        green += green_sum[point.key];
        blue += blue_sum[point.key];
        count += u64::from(point.count);
    }
    if count == 0 {
        return (0, 0, 0);
    }
    let mean = |sum: u64| (sum as f64 / count as f64).round() as u8;
    (mean(red), mean(green), mean(blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One flat colour, all pixels with the same RGB.
    fn flat(colour: [u8; 3]) -> Vec<u8> {
        let mut rgba = vec![0_u8; KRAKEN_SIZE * KRAKEN_SIZE * 4];
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel[..3].copy_from_slice(&colour);
            pixel[3] = 255;
        }
        rgba
    }

    #[test]
    fn a_flat_colour_is_its_own_palette_entry_with_no_error() {
        let frame = to_paletted_frame(&flat([10, 20, 30]));
        assert_eq!(frame.len(), FRAME_BYTES);
        // The single populated box's mean is the colour itself.
        assert_eq!(&frame[..3], &[10, 20, 30]);
        assert_eq!(frame[3], 0, "the firmware's alpha padding");
        let plane = &frame[PALETTE_BYTES..];
        assert!(plane.iter().all(|index| *index == 0));
    }

    #[test]
    fn a_two_colour_frame_spends_only_the_entries_it_needs() {
        let mut rgba = flat([0, 0, 0]);
        for (pixel, chunk) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if pixel % 2 == 0 {
                chunk[..3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let frame = to_paletted_frame(&rgba);
        let plane = &frame[PALETTE_BYTES..];
        let mut seen = [false; 256];
        for index in plane {
            seen[usize::from(*index)] = true;
        }
        let used = seen.iter().filter(|seen| **seen).count();
        assert!(used <= 2, "{used} entries used");
        assert!(used >= 2, "both colours appear");
        // A neutral ramp stays neutral: every entry is a grey.
        for entry in frame[..PALETTE_BYTES].as_chunks::<4>().0.iter().take(used) {
            assert_eq!(entry[0], entry[1]);
            assert_eq!(entry[1], entry[2]);
        }
    }

    #[test]
    fn a_short_frame_is_read_as_black_rather_than_panicking() {
        let frame = to_paletted_frame(&[255, 0, 0]);
        assert_eq!(frame.len(), FRAME_BYTES);
        assert_eq!(&frame[..3], &[0, 0, 0]);
    }
}
