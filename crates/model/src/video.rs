//! The facts a theme's videos are judged by, read from an MP4's own boxes.
//!
//! **This is the publish-time envelope, not a player.** Only `ftyp`, `moov`
//! and its tables are read, and every read is bounds-checked under a box and
//! entry budget, so a crafted upload cannot hang or allocate without bound. An
//! unreadable file answers `None` rather than throwing.

/// The declared limits a theme video must fit.
pub const MAX_DURATION_MS: u64 = 60_000;
pub const MAX_LONG_EDGE: u32 = 4096;
pub const MAX_FRAME_RATE: f64 = 30.0;

/// A trimmed loop's dissolve, and the shortest span that leaves a body after
/// it. The editor's trim surface and the encoder both read these.
pub const TRIM_BLEND_MS: u64 = 1_000;
pub const TRIM_MIN_SPAN_MS: u64 = 2_000;

const MAX_BOXES: u32 = 1024;
const MAX_DEPTH: u32 = 8;
const MAX_STSD_ENTRIES: u32 = 32;
const MAX_STTS_ENTRIES: u32 = 100_000;
const MAX_ELST_ENTRIES: u32 = 256;
const MAX_SPS_BYTES: u16 = 4096;
const VISUAL_SAMPLE_ENTRY_BYTES: usize = 78;

/// Profile families whose SPS carries `chroma_format_idc`; the rest are 4:2:0.
const CHROMA_PROFILES: [u8; 13] = [44, 83, 86, 100, 110, 118, 122, 128, 134, 135, 138, 139, 244];

/// What one video declares about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub width: u32,
    pub height: u32,
    /// The longest of media, edit-list, track and movie duration, so a clip
    /// that repeats or holds its media cannot present past the cap.
    pub duration_ms: u64,
    /// Measured average; a cadence that is not constant says so in
    /// `constant_frame_rate`.
    pub frame_rate: f64,
    pub constant_frame_rate: bool,
    /// Sample entry fourcc, e.g. `avc1`.
    pub codec: Option<String>,
    /// `avcC`'s `profile_idc`.
    pub profile: Option<u8>,
    /// SPS `chroma_format_idc`: 1 is 4:2:0, 2 is 4:2:2, 3 is 4:4:4, 0 is
    /// monochrome.
    pub chroma_format: Option<u8>,
    pub has_audio: bool,
    /// `moov` appears before the first `mdat`.
    pub faststart: bool,
}

impl Facts {
    pub fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }
}

/// The read of one file: the facts, or why they could not be read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Read {
    pub facts: Option<Facts>,
    /// An MP4-shaped file whose walk stopped before its declared end: it is
    /// incomplete or too complex, which is not the claim "not an H.264 MP4".
    pub truncated: bool,
}

/// The facts of an MP4's own boxes, or `None` when the bytes are not one this
/// parser can read.
pub fn read(bytes: &[u8]) -> Option<Facts> {
    read_detailed(bytes).facts
}

/// The facts and whether the walk was cut short, so a refusal can say
/// "truncated" rather than blame the codec.
pub fn read_detailed(bytes: &[u8]) -> Read {
    let mut budget = Budget {
        left: MAX_BOXES,
        truncated: false,
    };
    let top = boxes(bytes, 0, bytes.len(), &mut budget, 0);
    // A top-level `ftyp` is what separates an incomplete MP4 from bytes that
    // are not an MP4 at all.
    let mp4 = top.iter().any(|span| span.kind == *b"ftyp");
    // A walk that stopped early has not seen the whole file, so its silence
    // about a track means nothing.
    if budget.truncated {
        return unreadable(&budget, mp4);
    }
    let Some(moov) = top.iter().find(|span| span.kind == *b"moov") else {
        return unreadable(&budget, mp4);
    };
    let mdat = top.iter().find(|span| span.kind == *b"mdat");
    let faststart = mdat.is_some_and(|mdat| moov.start < mdat.start);

    let mut tracks: Vec<Track> = Vec::new();
    let mut movie_timescale: Option<f64> = None;
    let mut movie_ticks: Option<f64> = None;
    for child in boxes(bytes, moov.start, moov.end, &mut budget, 1) {
        if child.kind == *b"trak" {
            tracks.push(read_trak(bytes, &child, &mut budget));
        } else if child.kind == *b"mvhd"
            && let Some((timescale, duration)) = timescale_duration(bytes, &child)
        {
            movie_timescale = Some(timescale);
            movie_ticks = Some(duration);
        }
    }
    if budget.truncated {
        return unreadable(&budget, mp4);
    }
    let Some(video) = tracks
        .iter()
        .find(|track| track.handler.as_deref() == Some("vide"))
    else {
        return unreadable(&budget, mp4);
    };

    // Timing the parser cannot read is unreadable, not a zero-duration video:
    // a cap must never be satisfied by an absent measurement.
    let Some(timescale) = video.timescale else {
        return unreadable(&budget, mp4);
    };
    if timescale <= 0.0 {
        return unreadable(&budget, mp4);
    }

    let mdhd_ms = video
        .duration
        .map(|duration| (duration / timescale) * 1000.0);
    let sample_ms = (video.sample_ticks > 0.0).then(|| (video.sample_ticks / timescale) * 1000.0);
    // Edit-list and track durations are stated on the movie timeline, so a
    // conversion with no movie timescale cannot bound the presentation.
    let edit_ms = ticks_to_ms(video.edit_ticks, movie_timescale);
    let track_ms = ticks_to_ms(video.track_ticks, movie_timescale);
    let movie_ms = ticks_to_ms(movie_ticks, movie_timescale);
    if (video.edit_ticks.is_some() && edit_ms.is_none())
        || (video.track_ticks.is_some() && track_ms.is_none())
    {
        return unreadable(&budget, mp4);
    }
    // The fields can lie about each other. For a cap the larger wins.
    let duration_ms = mdhd_ms
        .unwrap_or(0.0)
        .max(sample_ms.unwrap_or(0.0))
        .max(edit_ms.unwrap_or(0.0))
        .max(track_ms.unwrap_or(0.0))
        .max(movie_ms.unwrap_or(0.0));

    let uniform_delta = (video.deltas.len() == 1)
        .then(|| video.deltas.first().copied())
        .flatten();
    // A zero delta is not a cadence. Left alone it would read as 0 fps and a
    // constant rate while the table carries no time at all.
    if uniform_delta == Some(0) {
        return unreadable(&budget, mp4);
    }
    let sample_seconds = sample_ms.map(|ms| ms / 1000.0);
    let frame_rate = if video.samples == 0 {
        0.0
    } else if let Some(delta) = uniform_delta {
        // A uniform table's cadence is exact, where samples/seconds merely
        // averages and can under-report a rounded `mdhd` duration.
        timescale / f64::from(delta)
    } else if let Some(seconds) = sample_seconds {
        if seconds > 0.0 {
            video.samples as f64 / seconds
        } else {
            0.0
        }
    } else {
        0.0
    };

    Read {
        facts: Some(Facts {
            width: video.width,
            height: video.height,
            duration_ms: cap_ceil_u64(duration_ms),
            frame_rate: cap_ceil_f64(frame_rate * 1000.0) / 1000.0,
            constant_frame_rate: video.samples > 0 && video.deltas.len() == 1,
            codec: video.codec.clone(),
            profile: video.profile,
            chroma_format: video.chroma_format,
            has_audio: tracks
                .iter()
                .any(|track| track.handler.as_deref() == Some("soun")),
            faststart,
        }),
        truncated: false,
    }
}

fn unreadable(budget: &Budget, mp4: bool) -> Read {
    Read {
        facts: None,
        truncated: mp4 && budget.truncated,
    }
}

/// Rounds a bound up, tolerating only floating-point noise. A rounded display
/// value must not let 30.0004 s or fps pass a 30 cap.
fn cap_ceil_u64(value: f64) -> u64 {
    cap_ceil_f64(value).max(0.0) as u64
}

fn cap_ceil_f64(value: f64) -> f64 {
    let nearest = value.round();
    if (value - nearest).abs() < 1e-6 {
        nearest
    } else {
        value.ceil()
    }
}

/// Ticks to milliseconds, or `None` when either side is missing. A zero
/// timescale is not a unit, so it converts nothing.
fn ticks_to_ms(ticks: Option<f64>, timescale: Option<f64>) -> Option<f64> {
    match (ticks, timescale) {
        (Some(ticks), Some(timescale)) if timescale > 0.0 => Some((ticks / timescale) * 1000.0),
        _ => None,
    }
}

/// Every way `facts` breaks the envelope, in a fixed order. Empty means
/// conforming.
pub fn refusals(path: &str, facts: &Facts) -> Vec<String> {
    let mut out = Vec::new();
    if !facts.faststart {
        out.push(format!(
            "{path}: moov comes after mdat; +faststart is required"
        ));
    }
    if facts.codec.as_deref() != Some("avc1") {
        out.push(format!(
            "{path}: video codec {}; H.264 (avc1) is required",
            printable(facts.codec.as_deref().unwrap_or("none"))
        ));
    }
    if facts.profile != Some(100) {
        out.push(format!(
            "{path}: H.264 profile {}; High (100) is required",
            facts
                .profile
                .map_or("unknown".to_owned(), |value| value.to_string())
        ));
    }
    if facts.chroma_format != Some(1) {
        out.push(format!(
            "{path}: chroma format {}; 4:2:0 is required",
            facts
                .chroma_format
                .map_or("unknown".to_owned(), |value| value.to_string())
        ));
    }
    if !facts.width.is_multiple_of(2) || !facts.height.is_multiple_of(2) {
        out.push(format!(
            "{path}: odd dimensions {}x{}; even width and height are required",
            facts.width, facts.height
        ));
    }
    if !facts.constant_frame_rate {
        out.push(format!(
            "{path}: variable frame rate; a constant frame rate is required"
        ));
    }
    if facts.frame_rate > MAX_FRAME_RATE {
        out.push(format!(
            "{path}: frame rate {} fps exceeds the {} fps maximum",
            facts.frame_rate, MAX_FRAME_RATE
        ));
    }
    if facts.has_audio {
        out.push(format!("{path}: audio track present; video must be silent"));
    }
    if facts.duration_ms > MAX_DURATION_MS {
        out.push(format!(
            "{path}: duration {} ms exceeds the {} ms maximum",
            facts.duration_ms, MAX_DURATION_MS
        ));
    }
    if facts.long_edge() > MAX_LONG_EDGE {
        out.push(format!(
            "{path}: long edge {} px exceeds the {} px maximum",
            facts.long_edge(),
            MAX_LONG_EDGE
        ));
    }
    out
}

/// A fourcc from untrusted bytes, made safe to print in a refusal sentence.
fn printable(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if (' '..='~').contains(&character) {
                character
            } else {
                '?'
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct BoxSpan {
    kind: [u8; 4],
    start: usize,
    end: usize,
}

#[derive(Debug, Clone)]
struct Budget {
    left: u32,
    /// A walk stopped before its declared end, so nothing it returned is
    /// complete.
    truncated: bool,
}

#[derive(Debug, Clone, Default)]
struct Track {
    handler: Option<String>,
    timescale: Option<f64>,
    /// The `mdhd` duration in `timescale` ticks.
    duration: Option<f64>,
    samples: u64,
    /// `stts`'s total media time in `timescale` ticks.
    sample_ticks: f64,
    /// Sum of `elst` segment durations in movie-timescale ticks.
    edit_ticks: Option<f64>,
    /// `tkhd` duration in movie-timescale ticks.
    track_ticks: Option<f64>,
    deltas: Vec<u32>,
    codec: Option<String>,
    width: u32,
    height: u32,
    profile: Option<u8>,
    chroma_format: Option<u8>,
}

fn boxes(bytes: &[u8], start: usize, end: usize, budget: &mut Budget, depth: u32) -> Vec<BoxSpan> {
    let mut spans = Vec::new();
    if depth > MAX_DEPTH {
        budget.truncated = true;
        return spans;
    }
    let end = end.min(bytes.len());
    let mut offset = start;
    while offset + 8 <= end {
        if budget.left == 0 {
            budget.truncated = true;
            return spans;
        }
        budget.left -= 1;
        let mut size = be32(bytes, offset) as usize;
        let kind = fourcc(bytes, offset + 4);
        let mut header = 8;
        if size == 1 {
            if offset + 16 > end {
                budget.truncated = true;
                return spans;
            }
            // A payload past 4 GiB cannot fit in memory, so a non-zero high
            // word is malformed for our purposes.
            if be32(bytes, offset + 8) != 0 {
                budget.truncated = true;
                return spans;
            }
            size = be32(bytes, offset + 12) as usize;
            header = 16;
        } else if size == 0 {
            size = end - offset;
        }
        if size < header || size > end - offset {
            budget.truncated = true;
            return spans;
        }
        spans.push(BoxSpan {
            kind,
            start: offset + header,
            end: offset + size,
        });
        offset += size;
    }
    spans
}

fn fourcc(bytes: &[u8], offset: usize) -> [u8; 4] {
    match bytes.get(offset..offset + 4) {
        Some(slice) => [slice[0], slice[1], slice[2], slice[3]],
        None => [0; 4],
    }
}

fn code_of(bytes: &[u8], offset: usize) -> String {
    String::from_utf8_lossy(&fourcc(bytes, offset)).into_owned()
}

fn be16(bytes: &[u8], offset: usize) -> u16 {
    bytes
        .get(offset..offset + 2)
        .map_or(0, |slice| u16::from_be_bytes([slice[0], slice[1]]))
}

fn be32(bytes: &[u8], offset: usize) -> u32 {
    bytes.get(offset..offset + 4).map_or(0, |slice| {
        u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]])
    })
}

fn read_trak(bytes: &[u8], trak: &BoxSpan, budget: &mut Budget) -> Track {
    let mut track = Track::default();
    for child in boxes(bytes, trak.start, trak.end, budget, 1) {
        if child.kind == *b"edts" {
            if let Some(ticks) = read_edts(bytes, &child, budget) {
                track.edit_ticks = Some(ticks);
            }
            continue;
        }
        if child.kind == *b"tkhd" {
            if let Some((_, duration)) = timescale_duration(bytes, &child) {
                track.track_ticks = Some(duration);
            }
            continue;
        }
        if child.kind != *b"mdia" {
            continue;
        }
        for span in boxes(bytes, child.start, child.end, budget, 2) {
            if span.kind == *b"hdlr" {
                if span.start + 12 <= span.end {
                    track.handler = Some(code_of(bytes, span.start + 8));
                }
            } else if span.kind == *b"mdhd" {
                if let Some((timescale, duration)) = timescale_duration(bytes, &span) {
                    track.timescale = Some(timescale);
                    track.duration = Some(duration);
                }
            } else if span.kind == *b"minf" {
                for stbl in boxes(bytes, span.start, span.end, budget, 3) {
                    if stbl.kind != *b"stbl" {
                        continue;
                    }
                    for entry in boxes(bytes, stbl.start, stbl.end, budget, 4) {
                        if entry.kind == *b"stsd" {
                            read_stsd(bytes, &entry, budget, &mut track);
                        } else if entry.kind == *b"stts" {
                            read_stts(bytes, &entry, budget, &mut track);
                        }
                    }
                }
            }
        }
    }
    track
}

/// The sum of an `edts`'s edit-list segment durations, in movie-timescale
/// ticks. An edit can hold or repeat media, so this presentation length can
/// exceed the media the sample table carries.
fn read_edts(bytes: &[u8], edts: &BoxSpan, budget: &mut Budget) -> Option<f64> {
    for span in boxes(bytes, edts.start, edts.end, budget, 2) {
        if span.kind == *b"elst" {
            return read_elst(bytes, &span, budget);
        }
    }
    None
}

fn read_elst(bytes: &[u8], span: &BoxSpan, budget: &mut Budget) -> Option<f64> {
    if span.start + 8 > span.end {
        budget.truncated = true;
        return None;
    }
    let version = bytes.get(span.start).copied().unwrap_or(0);
    // Only the two defined layouts can be read; an unknown one may claim any
    // presentation length, so it is unreadable rather than ignored.
    if version != 0 && version != 1 {
        budget.truncated = true;
        return None;
    }
    let entry_bytes = if version == 1 { 20 } else { 12 };
    let declared = be32(bytes, span.start + 4);
    let count = u64::from(declared)
        .min(((span.end - span.start - 8) / entry_bytes) as u64)
        .min(u64::from(MAX_ELST_ENTRIES));
    // Entries the budget dropped would under-report the presentation, so the
    // untidy list is unreadable rather than short.
    if count < u64::from(declared) {
        budget.truncated = true;
        return None;
    }
    let mut total = 0.0_f64;
    for index in 0..count as usize {
        let at = span.start + 8 + index * entry_bytes;
        total += if version == 1 {
            f64::from(be32(bytes, at)) * 2f64.powi(32) + f64::from(be32(bytes, at + 4))
        } else {
            f64::from(be32(bytes, at))
        };
        if !total.is_finite() || total > 9_007_199_254_740_991.0 {
            budget.truncated = true;
            return None;
        }
    }
    (count > 0).then_some(total)
}

/// `mvhd`, `tkhd` and `mdhd` share the version/creation/modification/timescale/
/// duration prefix.
fn timescale_duration(bytes: &[u8], span: &BoxSpan) -> Option<(f64, f64)> {
    if span.start + 4 > span.end {
        return None;
    }
    let version = bytes.get(span.start).copied().unwrap_or(0);
    if version == 1 {
        if span.start + 32 > span.end {
            return None;
        }
        return Some((
            f64::from(be32(bytes, span.start + 20)),
            f64::from(be32(bytes, span.start + 24)) * 2f64.powi(32)
                + f64::from(be32(bytes, span.start + 28)),
        ));
    }
    if span.start + 20 > span.end {
        return None;
    }
    Some((
        f64::from(be32(bytes, span.start + 12)),
        f64::from(be32(bytes, span.start + 16)),
    ))
}

fn read_stsd(bytes: &[u8], span: &BoxSpan, budget: &mut Budget, track: &mut Track) {
    if span.start + 8 > span.end {
        budget.truncated = true;
        return;
    }
    let count = be32(bytes, span.start + 4).min(MAX_STSD_ENTRIES);
    // The first sample entry is the one the player decodes, so the walk stops
    // there the way the source's does.
    if count == 0 {
        return;
    }
    let offset = span.start + 8;
    if offset + 8 > span.end {
        budget.truncated = true;
        return;
    }
    let size = be32(bytes, offset) as usize;
    if size < 8 || size > span.end - offset {
        budget.truncated = true;
        return;
    }
    let codec = code_of(bytes, offset + 4);
    let payload = offset + 8;
    let payload_end = offset + size;
    track.codec = Some(codec.clone());
    // VisualSampleEntry: width/height sit 24/26 bytes into the payload.
    if payload + 28 <= payload_end {
        track.width = u32::from(be16(bytes, payload + 24));
        track.height = u32::from(be16(bytes, payload + 26));
    }
    if (codec == "avc1" || codec == "avc3") && payload + VISUAL_SAMPLE_ENTRY_BYTES <= payload_end {
        for child in boxes(
            bytes,
            payload + VISUAL_SAMPLE_ENTRY_BYTES,
            payload_end,
            budget,
            5,
        ) {
            if child.kind != *b"avcC" {
                continue;
            }
            track.profile = (child.start + 1 < child.end)
                .then(|| bytes.get(child.start + 1).copied())
                .flatten();
            track.chroma_format = read_avcc_chroma(bytes, &child);
        }
    }
}

/// First SPS's `chroma_format_idc`; `None` when the avcC carries none we can
/// read.
fn read_avcc_chroma(bytes: &[u8], avcc: &BoxSpan) -> Option<u8> {
    if avcc.start + 6 > avcc.end {
        return None;
    }
    let profile_id = bytes.get(avcc.start + 1).copied()?;
    let sps_count = (bytes.get(avcc.start + 5).copied()? & 0x1f).min(32);
    let mut offset = avcc.start + 6;
    for _ in 0..sps_count {
        if offset + 2 > avcc.end {
            return None;
        }
        let length = usize::from(be16(bytes, offset));
        let start = offset + 2;
        if length == 0 || length > usize::from(MAX_SPS_BYTES) || start + length > avcc.end {
            return None;
        }
        if let Some(chroma) = read_sps_chroma(bytes, start, start + length, profile_id) {
            return Some(chroma);
        }
        offset = start + length;
    }
    None
}

fn read_sps_chroma(bytes: &[u8], start: usize, end: usize, profile_id: u8) -> Option<u8> {
    // RBSP: drop the NAL header byte and every emulation-prevention 0x03.
    let mut rbsp = Vec::with_capacity(end.saturating_sub(start));
    let mut zeros = 0_u32;
    for index in start + 1..end {
        let byte = bytes.get(index).copied()?;
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        rbsp.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    let mut bits = BitReader::new(&rbsp);
    // H.264 SPS RBSP: profile_idc, constraint flags and level_idc, then the
    // parameter-set id; `chroma_format_idc` follows only for the High families.
    if !bits.skip(24) {
        return None;
    }
    bits.ue()?;
    if !CHROMA_PROFILES.contains(&profile_id) {
        return Some(1);
    }
    bits.ue()
        .and_then(|chroma| (chroma <= 3).then_some(chroma as u8))
}

fn read_stts(bytes: &[u8], span: &BoxSpan, budget: &mut Budget, track: &mut Track) {
    if span.start + 8 > span.end {
        budget.truncated = true;
        return;
    }
    let declared = be32(bytes, span.start + 4);
    let count = u64::from(declared)
        .min(((span.end - span.start - 8) / 8) as u64)
        .min(u64::from(MAX_STTS_ENTRIES));
    // Entries the budget dropped would under-report the media length, so the
    // untidy table is unreadable rather than short.
    if count < u64::from(declared) {
        budget.truncated = true;
    }
    let mut offset = span.start + 8;
    for _ in 0..count {
        let samples = be32(bytes, offset);
        let delta = be32(bytes, offset + 4);
        track.samples += u64::from(samples);
        track.sample_ticks += f64::from(samples) * f64::from(delta);
        if !track.deltas.contains(&delta) {
            track.deltas.push(delta);
        }
        offset += 8;
    }
}

/// Bounded exp-Golomb reader; every read past the end answers `None`.
struct BitReader {
    bytes: Vec<u8>,
    bit: usize,
}

impl BitReader {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            bit: 0,
        }
    }

    fn end(&self) -> usize {
        self.bytes.len() * 8
    }

    fn skip(&mut self, count: usize) -> bool {
        if self.bit + count > self.end() {
            return false;
        }
        self.bit += count;
        true
    }

    fn u(&mut self, count: usize) -> Option<u32> {
        if count > 32 || self.bit + count > self.end() {
            return None;
        }
        let mut value = 0_u32;
        for _ in 0..count {
            value = value * 2 + self.read_bit()?;
        }
        Some(value)
    }

    /// Unsigned exp-Golomb, bounded to 31 leading zeros.
    fn ue(&mut self) -> Option<u32> {
        let mut leading = 0_u32;
        loop {
            if self.u(1)? == 1 {
                break;
            }
            leading += 1;
            if leading > 31 {
                return None;
            }
        }
        if leading == 0 {
            return Some(0);
        }
        let rest = self.u(leading as usize)?;
        Some(2_u32.pow(leading) - 1 + rest)
    }

    fn read_bit(&mut self) -> Option<u32> {
        let byte = self.bytes.get(self.bit >> 3).copied()?;
        let bit = u32::from((byte >> (7 - (self.bit & 7))) & 1);
        self.bit += 1;
        Some(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFORMING: &[u8] = include_bytes!("../tests/video-fixtures/conforming.mp4");
    const AUDIO: &[u8] = include_bytes!("../tests/video-fixtures/audio.mp4");
    const NOFASTSTART: &[u8] = include_bytes!("../tests/video-fixtures/nofaststart.mp4");
    const CHROMA422: &[u8] = include_bytes!("../tests/video-fixtures/chroma422.mp4");
    const VFR: &[u8] = include_bytes!("../tests/video-fixtures/vfr.mp4");
    const WEBM: &[u8] = include_bytes!("../tests/video-fixtures/other.webm");
    const TRUNCATED: &[u8] = include_bytes!("../tests/video-fixtures/truncated.mp4");
    const LONG: &[u8] = include_bytes!("../tests/video-fixtures/too-long.mp4");
    const ODD: &[u8] = include_bytes!("../tests/video-fixtures/odd.mp4");
    const WIDE: &[u8] = include_bytes!("../tests/video-fixtures/wide.mp4");

    const PATH: &str = "assets/bg.mp4";

    #[test]
    fn reads_duration_dimensions_cadence_and_faststart_from_a_conforming_file() {
        let facts = read(CONFORMING).expect("readable");
        assert_eq!(
            facts,
            Facts {
                width: 320,
                height: 180,
                duration_ms: 2_000,
                frame_rate: 30.0,
                constant_frame_rate: true,
                codec: Some("avc1".to_owned()),
                profile: Some(100),
                chroma_format: Some(1),
                has_audio: false,
                faststart: true,
            }
        );
        assert!(refusals(PATH, &facts).is_empty());
    }

    #[test]
    fn sees_an_audio_track_and_moov_after_mdat() {
        let facts = read(AUDIO).expect("readable");
        assert!(facts.has_audio);
        assert_eq!((facts.width, facts.height), (320, 180));
        assert!(!read(NOFASTSTART).expect("readable").faststart);
        assert!(read(CONFORMING).expect("readable").faststart);
    }

    #[test]
    fn reads_a_4_2_2_chroma_format_from_the_sps() {
        let facts = read(CHROMA422).expect("readable");
        assert_eq!(facts.chroma_format, Some(2));
        assert_eq!(facts.profile, Some(122));
    }

    #[test]
    fn marks_a_variable_cadence_as_not_constant_and_averages_its_rate() {
        let facts = read(VFR).expect("readable");
        assert!(!facts.constant_frame_rate);
        // 12 frames at 9000 ticks plus 28 at 3000 is 40 frames over 192000
        // ticks (2.133 s): the sample table, not mdhd's rounded 2 s, is the
        // media length.
        assert_eq!(facts.duration_ms, 2_134);
        assert_eq!(facts.frame_rate, 18.75);
    }

    #[test]
    fn refuses_a_webm_a_truncated_mp4_and_hostile_bytes() {
        assert!(read(WEBM).is_none());
        assert!(read(TRUNCATED).is_none());
        assert!(read_detailed(TRUNCATED).truncated);

        assert!(read(&[]).is_none());
        assert!(read(&[0; 4096]).is_none());
        // A box whose declared size runs past the buffer.
        assert!(read(&[0xff, 0xff, 0xff, 0xff, b'm', b'o', b'o', b'v']).is_none());
        // A top-level box that claims to be everything, with nothing inside.
        let mut empty = [0_u8; 8];
        empty[4..].copy_from_slice(b"moov");
        assert!(read(&empty).is_none());
        // A 64-bit box whose high word is non-zero cannot fit in memory.
        let mut sixty_four = [0_u8; 16];
        sixty_four[..4].copy_from_slice(&1_u32.to_be_bytes());
        sixty_four[4..8].copy_from_slice(b"moov");
        sixty_four[8..12].copy_from_slice(&1_u32.to_be_bytes());
        assert!(read(&sixty_four).is_none());
    }

    #[test]
    fn an_odd_size_mp4_is_named_in_its_own_words() {
        let mut odd = Conforming::patched();
        // The first sample entry starts eight bytes into `stsd`'s payload, and
        // its width sits 24 bytes into the entry's own payload.
        let stsd = odd
            .box_at(&["moov", "trak", "mdia", "minf", "stbl", "stsd"])
            .1;
        odd.set_u32(stsd + 8 + 8 + 24, 319);
        let bytes = odd.finish();
        let facts = read(&bytes).expect("still readable");
        let refusals = refusals(PATH, &facts);
        assert!(
            refusals.iter().any(|line| line.contains("odd dimensions")),
            "{refusals:?}"
        );
    }

    #[test]
    fn each_broken_fixture_is_refused_in_its_own_words() {
        let cases: [(&[u8], &str); 3] = [
            (NOFASTSTART, "moov comes after mdat"),
            (ODD, "odd dimensions"),
            (WIDE, "long edge"),
        ];
        for (bytes, expected) in cases {
            let facts = read(bytes).expect("readable");
            let refusals = refusals(PATH, &facts);
            assert!(
                refusals.iter().any(|line| line.contains(expected)),
                "expected {expected} in {refusals:?}"
            );
        }
    }

    #[test]
    fn a_doubled_media_claim_cannot_hide_a_sixty_fps_sample_table() {
        let mut crafted = Conforming::patched();
        let mdhd = crafted.box_at(&["moov", "trak", "mdia", "mdhd"]).1;
        crafts_patch(
            &mut crafted,
            &["moov", "trak", "mdia", "minf", "stbl", "stts"],
            12,
            256,
        );
        crafted.set_u32(mdhd + 16, 61_440);
        let facts = read(&crafted.finish()).expect("readable");
        assert!(facts.constant_frame_rate);
        assert_eq!(
            refusals(PATH, &facts),
            vec![format!(
                "{PATH}: frame rate 60 fps exceeds the 30 fps maximum"
            )]
        );
    }

    #[test]
    fn the_largest_duration_wins_between_the_header_the_table_and_the_edits() {
        // A shortened `mdhd` over 61.5 s of samples is still 61.5 s.
        let mut samples = Conforming::with(LONG);
        let mdhd = samples.box_at(&["moov", "trak", "mdia", "mdhd"]).1;
        samples.set_u32(mdhd + 16, 241_920);
        assert_eq!(
            read(&samples.finish()).expect("readable").duration_ms,
            61_500
        );

        // An edit list that holds the last frame bounds the presentation.
        let mut held = Conforming::patched();
        let elst = held.box_at(&["moov", "trak", "edts", "elst"]).1;
        held.set_u32(elst + 8, 60_500);
        assert_eq!(
            refusals(PATH, &read(&held.finish()).expect("readable")),
            vec![format!(
                "{PATH}: duration 60500 ms exceeds the 60000 ms maximum"
            )]
        );

        // A shorter edit cannot hide the media still decoded.
        let mut shorter = Conforming::patched();
        let elst = shorter.box_at(&["moov", "trak", "edts", "elst"]).1;
        shorter.set_u32(elst + 8, 1_000);
        let facts = read(&shorter.finish()).expect("readable");
        assert_eq!(facts.duration_ms, 2_000);
        assert!(refusals(PATH, &facts).is_empty());

        // The movie header is a presentation bound too.
        let mut movie = Conforming::patched();
        let mvhd = movie.box_at(&["moov", "mvhd"]).1;
        movie.set_u32(mvhd + 16, 60_500);
        assert_eq!(
            refusals(PATH, &read(&movie.finish()).expect("readable")),
            vec![format!(
                "{PATH}: duration 60500 ms exceeds the 60000 ms maximum"
            )]
        );
    }

    #[test]
    fn a_zero_sample_delta_or_a_zero_timescale_is_unreadable() {
        let mut zero_delta = Conforming::patched();
        let mdhd = zero_delta.box_at(&["moov", "trak", "mdia", "mdhd"]).1;
        crafts_patch(
            &mut zero_delta,
            &["moov", "trak", "mdia", "minf", "stbl", "stts"],
            12,
            0,
        );
        zero_delta.set_u32(mdhd + 16, 15_360);
        assert!(read(&zero_delta.finish()).is_none());

        let mut zero_timescale = Conforming::patched();
        let mdhd = zero_timescale.box_at(&["moov", "trak", "mdia", "mdhd"]).1;
        zero_timescale.set_u32(mdhd + 12, 0);
        assert!(read(&zero_timescale.finish()).is_none());
    }

    fn crafts_patch(frame: &mut Conforming, path: &[&str], offset: usize, value: u32) {
        let at = frame.box_at(path).1 + offset;
        frame.set_u32(at, value);
    }

    /// A mutable copy of the conforming fixture, with the same box walking the
    /// tests use to patch one field.
    struct Conforming(Vec<u8>);

    impl Conforming {
        fn patched() -> Self {
            Self::with(CONFORMING)
        }

        fn with(bytes: &[u8]) -> Self {
            Self(bytes.to_vec())
        }

        fn set_u32(&mut self, at: usize, value: u32) {
            self.0[at..at + 4].copy_from_slice(&value.to_be_bytes());
        }

        /// The span of the box at a path of nested container types.
        fn box_at(&self, path: &[&str]) -> (usize, usize, usize, usize) {
            let mut start = 0;
            let mut end = self.0.len();
            let mut found = (0, 0, 0, 0);
            for kind in path {
                let mut offset = start;
                let mut located = None;
                while offset + 8 <= end {
                    let size = be32(&self.0, offset) as usize;
                    let header = if size == 1 { 16 } else { 8 };
                    let actual = if size == 0 { end - offset } else { size };
                    if actual < header || actual > end - offset {
                        break;
                    }
                    if &self.0[offset + 4..offset + 8] == kind.as_bytes() {
                        located = Some((offset, offset + header, offset + actual));
                        break;
                    }
                    offset += actual;
                }
                let (box_start, payload, box_end) = located.expect("the box path exists");
                found = (box_start, payload, box_end, 0);
                start = payload;
                end = box_end;
            }
            found
        }

        fn finish(self) -> Vec<u8> {
            self.0
        }
    }
}
