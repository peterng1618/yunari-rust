//! The Win32 half: mapping a named section read-only.
//!
//! Nothing here decides a value; it copies bytes and hands them to
//! [`crate::decode`].

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Memory::{
    FILE_MAP_READ, MEMORY_BASIC_INFORMATION, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile,
    VirtualQuery,
};

/// Nothing larger than this is a mapping RTSS could have published.
const MAX_MAPPING_BYTES: usize = 64 * 1024 * 1024;

/// The whole mapping, or `None` when RTSS is not running or it cannot be read.
pub fn read(name: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the name outlives the call, and both handles are closed and both
    // pointers unmapped on every path out of this function.
    unsafe {
        let mapping: HANDLE = OpenFileMappingW(FILE_MAP_READ, 0, wide.as_ptr());
        if mapping.is_null() {
            return None;
        }
        let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(mapping);
            return None;
        }
        let mut region = std::mem::zeroed::<MEMORY_BASIC_INFORMATION>();
        let queried = VirtualQuery(
            view.Value,
            &mut region,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        let length = if queried == 0 { 0 } else { region.RegionSize };
        let bytes = if length == 0 || length > MAX_MAPPING_BYTES {
            None
        } else {
            Some(std::slice::from_raw_parts(view.Value.cast::<u8>(), length).to_vec())
        };
        UnmapViewOfFile(view);
        CloseHandle(mapping);
        bytes
    }
}
