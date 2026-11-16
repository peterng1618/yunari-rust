#![deny(unsafe_code)]
//! RivaTuner Statistics Server's shared memory, read and nothing else.
//!
//! RTSS publishes a named mapping the host may read; this crate opens it, hands
//! back the bytes, and decodes the header and application entries. It never
//! starts RTSS, injects into anything, or writes to the mapping.
//!
//! The one `unsafe` call in the workspace lives in [`windows`]: Win32 has no
//! safe way to map a named section. Everything that decides a value is safe and
//! tested against the byte layout.

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;

/// The mapping RTSS publishes for overlay-aware readers.
pub const MAP_NAME: &str = "RTSSSharedMemoryV2";

/// A mapping this host cannot use, in words a display can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// RTSS is not running, or its mapping is gone.
    NotRunning,
    /// The mapping is shorter than its own header.
    TooSmall,
    /// The mapping is not RTSS's at all.
    Signature,
    /// RTSS older than 2.0 predates the fields read here.
    Version,
    /// The application entries are laid out differently than this reader knows.
    Layout,
}

impl Status {
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Ok => None,
            Self::NotRunning => Some("RivaTuner Statistics Server is not running."),
            Self::TooSmall => Some("RTSS shared memory is too small to read."),
            Self::Signature => Some("RTSS shared memory has an unexpected signature."),
            Self::Version => Some("RTSS shared memory is older than version 2.0."),
            Self::Layout => Some("RTSS shared memory has an unexpected application entry layout."),
        }
    }
}

/// The mapping's contents, once decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub version: u32,
    /// `None` when RTSS has no 3D application hooked right now.
    pub app: Option<App>,
}

/// The 3D application RTSS is reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub pid: u32,
    /// The executable's file name, never a path.
    pub name: String,
    /// The next ring slot RTSS will overwrite.
    pub ring_pos: u32,
    /// The last 1024 frametimes, in microseconds, oldest slot first.
    pub ring: Vec<u32>,
}

/// Offsets into `RTSSSharedMemory.h` (SDK v2.21). The header's own size fields
/// drive traversal, so a version that appends fields does not move a field read
/// here.
mod header {
    /// `'R' | 'T' << 8 | 'S' << 16 | 'S' << 24`
    pub const SIGNATURE: u32 = 0x5353_5452;
    pub const VERSION_AT: usize = 4;
    pub const APP_ENTRY_SIZE_AT: usize = 8;
    pub const APP_ARR_OFFSET_AT: usize = 12;
    pub const APP_ARR_SIZE_AT: usize = 16;
    /// From this version on, RTSS names the foreground process itself.
    pub const FOREGROUND_VERSION: u32 = 0x0002_0016;
    pub const FOREGROUND_PID_AT: usize = 68;
    pub const SIZE: usize = 72;
}

/// Offsets into one application entry. `dwStatFrameTimeBuf` is a 1024-slot ring
/// of recent frames in microseconds; RTSS's own overlay reads it as
/// `buf[(i + pos) & 1023]`, so the newest frame is `(pos - 1) & 1023`.
mod entry {
    pub const PID_AT: usize = 0;
    pub const NAME_AT: usize = 4;
    pub const NAME_BYTES: usize = 260;
    pub const TIME1_AT: usize = 272;
    pub const RING_AT: usize = 924;
    pub const RING_SLOTS: usize = 1_024;
    pub const RING_POS_AT: usize = RING_AT + RING_SLOTS * 4;
    pub const SIZE: usize = RING_POS_AT + 8;
}

/// Decodes a mapping, or says why it is not one this reader can use.
pub fn decode(bytes: &[u8]) -> Result<Snapshot, Status> {
    if bytes.len() < header::SIZE {
        return Err(Status::TooSmall);
    }
    if u32_at(bytes, 0) != header::SIGNATURE {
        return Err(Status::Signature);
    }
    let version = u32_at(bytes, header::VERSION_AT);
    if version < 0x0002_0000 {
        return Err(Status::Version);
    }

    let entry_size = u32_at(bytes, header::APP_ENTRY_SIZE_AT) as usize;
    let array_offset = u32_at(bytes, header::APP_ARR_OFFSET_AT) as usize;
    let array_size = u32_at(bytes, header::APP_ARR_SIZE_AT) as usize;
    if entry_size < entry::SIZE || array_offset == 0 || array_size == 0 || array_size > 4_096 {
        return Err(Status::Layout);
    }

    let foreground = if version >= header::FOREGROUND_VERSION {
        Some(u32_at(bytes, header::FOREGROUND_PID_AT))
    } else {
        None
    };

    let mut best: Option<(usize, u32)> = None;
    for index in 0..array_size {
        let at = array_offset + index * entry_size;
        if at + entry::SIZE > bytes.len() {
            break;
        }
        let pid = u32_at(bytes, at + entry::PID_AT);
        if pid == 0 {
            continue;
        }
        if foreground == Some(pid) {
            best = Some((at, pid));
            break;
        }
        let time1 = u32_at(bytes, at + entry::TIME1_AT);
        // The first entry, then the one RTSS touched most recently.
        if best.is_none_or(|(_, best_time)| time1 > best_time) {
            best = Some((at, time1));
        }
    }

    Ok(Snapshot {
        version,
        app: best.map(|(at, _)| app_at(bytes, at)),
    })
}

fn app_at(bytes: &[u8], at: usize) -> App {
    let ring_start = at + entry::RING_AT;
    App {
        pid: u32_at(bytes, at + entry::PID_AT),
        name: name_at(bytes, at + entry::NAME_AT),
        ring_pos: u32_at(bytes, at + entry::RING_POS_AT),
        ring: (0..entry::RING_SLOTS)
            .map(|slot| u32_at(bytes, ring_start + slot * 4))
            .collect(),
    }
}

/// An executable name, trimmed to its file name.
fn name_at(bytes: &[u8], at: usize) -> String {
    let raw = &bytes[at..at + entry::NAME_BYTES];
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    let text = String::from_utf8_lossy(&raw[..end]).trim().to_owned();
    text.rsplit(['/', '\\']).next().unwrap_or("").to_owned()
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The newest slot the ring has been written to.
pub fn newest_index(ring_pos: u32) -> usize {
    (ring_pos.wrapping_sub(1) & 1023) as usize
}

/// Frametimes, in microseconds, written since `last_newest`. A zero slot is
/// uninitialised and a frametime over a second is not a frame.
pub fn frames_since(ring: &[u32], ring_pos: u32, last_newest: usize) -> Vec<u32> {
    if ring.len() < entry::RING_SLOTS || last_newest > 1023 {
        return Vec::new();
    }
    let newest = newest_index(ring_pos);
    if newest == last_newest {
        return Vec::new();
    }
    let steps = (newest + 1024 - last_newest) % 1024;
    (1..=steps)
        .map(|step| ring[(last_newest + step) & 1023])
        .filter(|microseconds| *microseconds != 0 && *microseconds <= 1_000_000)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One entry with a name, the given frames and a ring position.
    fn entry(pid: u32, name: &str, time1: u32, frames: &[u32], ring_pos: u32) -> Vec<u8> {
        let mut bytes = vec![0_u8; entry::SIZE];
        bytes[entry::PID_AT..entry::PID_AT + 4].copy_from_slice(&pid.to_le_bytes());
        bytes[entry::NAME_AT..entry::NAME_AT + name.len()].copy_from_slice(name.as_bytes());
        bytes[entry::TIME1_AT..entry::TIME1_AT + 4].copy_from_slice(&time1.to_le_bytes());
        for (slot, frame) in frames.iter().enumerate() {
            let at = entry::RING_AT + slot * 4;
            bytes[at..at + 4].copy_from_slice(&frame.to_le_bytes());
        }
        bytes[entry::RING_POS_AT..entry::RING_POS_AT + 4].copy_from_slice(&ring_pos.to_le_bytes());
        bytes
    }

    /// A header followed by `entries`, with the paren'ed-off header fields.
    fn mapping(version: u32, foreground_pid: u32, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0_u8; header::SIZE];
        bytes[0..4].copy_from_slice(&header::SIGNATURE.to_le_bytes());
        bytes[header::VERSION_AT..header::VERSION_AT + 4].copy_from_slice(&version.to_le_bytes());
        bytes[header::APP_ENTRY_SIZE_AT..header::APP_ENTRY_SIZE_AT + 4]
            .copy_from_slice(&(entry::SIZE as u32).to_le_bytes());
        bytes[header::APP_ARR_OFFSET_AT..header::APP_ARR_OFFSET_AT + 4]
            .copy_from_slice(&(header::SIZE as u32).to_le_bytes());
        bytes[header::APP_ARR_SIZE_AT..header::APP_ARR_SIZE_AT + 4]
            .copy_from_slice(&(entries.len() as u32).to_le_bytes());
        bytes[header::FOREGROUND_PID_AT..header::FOREGROUND_PID_AT + 4]
            .copy_from_slice(&foreground_pid.to_le_bytes());
        for item in entries {
            bytes.extend_from_slice(item);
        }
        bytes
    }

    #[test]
    fn the_offsets_are_the_sdk_headers() {
        // An independent golden table: a wrong offset silently reads a
        // different field, so it is pinned here rather than derived.
        assert_eq!(header::SIZE, 72);
        assert_eq!(entry::PID_AT, 0);
        assert_eq!(entry::NAME_AT, 4);
        assert_eq!(entry::TIME1_AT, 272);
        assert_eq!(entry::RING_AT, 924);
        assert_eq!(entry::RING_POS_AT, 5_020);
        assert_eq!(entry::SIZE, 5_028);
    }

    #[test]
    fn a_mapping_that_is_not_rtss_says_why() {
        assert_eq!(decode(&[]), Err(Status::TooSmall));
        let mut wrong = mapping(0x0002_0016, 0, &[]);
        wrong[0] = b'X';
        assert_eq!(decode(&wrong), Err(Status::Signature));
        assert_eq!(decode(&mapping(0x0001_0000, 0, &[])), Err(Status::Version));

        let mut tiny_entries = mapping(0x0002_0016, 0, &[]);
        tiny_entries[header::APP_ENTRY_SIZE_AT..header::APP_ENTRY_SIZE_AT + 4]
            .copy_from_slice(&64_u32.to_le_bytes());
        assert_eq!(decode(&tiny_entries), Err(Status::Layout));
    }

    #[test]
    fn the_foreground_application_wins_over_the_busiest() {
        let idle = entry(4, "idle.exe", 900, &[], 0);
        let game = entry(7, "game.exe", 10, &[], 0);
        let bytes = mapping(0x0002_0016, 7, &[idle, game]);

        let snapshot = decode(&bytes).expect("a readable mapping");
        assert_eq!(snapshot.app.expect("an app").name, "game.exe");
    }

    #[test]
    fn without_a_foreground_name_the_most_recent_entry_wins() {
        let older = entry(4, "older.exe", 10, &[], 0);
        let newer = entry(7, "newer.exe", 20, &[], 0);
        let bytes = mapping(0x0002_0010, 0, &[older, newer]);

        let snapshot = decode(&bytes).expect("a readable mapping");
        assert_eq!(snapshot.app.expect("an app").pid, 7);
    }

    #[test]
    fn no_application_is_no_application_rather_than_an_error() {
        // A running RTSS reserves its slots; every one is empty here.
        let snapshot = decode(&mapping(0x0002_0016, 0, &[entry(0, "", 0, &[], 0)]))
            .expect("a readable mapping");
        assert_eq!(snapshot.app, None);
    }

    #[test]
    fn a_path_in_the_name_is_reduced_to_its_file_name() {
        let bytes = mapping(
            0x0002_0016,
            0,
            &[entry(1, "C:\\Games\\a\\game.exe", 1, &[], 0)],
        );
        assert_eq!(
            decode(&bytes)
                .expect("a readable mapping")
                .app
                .expect("an app")
                .name,
            "game.exe"
        );
    }

    #[test]
    fn only_slots_written_since_the_last_poll_are_frames() {
        let mut ring = vec![0_u32; 1_024];
        ring[10] = 16_667;
        ring[11] = 8_333;
        // The writer is at 12 now, so slots 10 and 11 are new.
        assert_eq!(frames_since(&ring, 12, 9), vec![16_667, 8_333]);
        // Nothing written since the last poll is nothing, not a repeat.
        assert_eq!(frames_since(&ring, 12, 11), Vec::<u32>::new());
        // An uninitialised slot and an absurd frametime are not frames.
        ring[12] = 0;
        ring[13] = 2_000_000;
        assert_eq!(frames_since(&ring, 14, 11), Vec::<u32>::new());
    }

    #[test]
    fn the_ring_wraps_around_its_last_slot() {
        let mut ring = vec![0_u32; 1_024];
        ring[1_023] = 5_000;
        ring[0] = 6_000;
        // Written at 1023 then 0: the writer is at 1.
        assert_eq!(frames_since(&ring, 1, 1_022), vec![5_000, 6_000]);
    }
}
