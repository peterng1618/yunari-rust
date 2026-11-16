//! Starting the host when the user signs in.
//!
//! Windows runs a script from the Startup folder; the script launches the app
//! with no console window. The file is UTF-16LE with a byte-order mark, which
//! is what the shell's own tools write.

use std::path::PathBuf;

pub const STARTED: &str = "Yunari will start hidden at sign-in.";
pub const STOPPED: &str = "Yunari will not start at sign-in.";
pub const FILE_NAME: &str = "Yunari.vbs";

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\Yunari.vbs`.
pub fn path() -> Option<PathBuf> {
    path_in(&std::env::var_os("APPDATA").map(PathBuf::from)?)
}

fn path_in(appdata: &std::path::Path) -> Option<PathBuf> {
    Some(
        appdata
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(FILE_NAME),
    )
}

pub fn enabled() -> bool {
    path().is_some_and(|path| path.exists())
}

/// Turn the startup entry on or off. The command is the app's own executable;
/// quotes inside it are doubled for the script's string literal.
pub fn set(turning_on: bool) -> Result<String, String> {
    let path = path().ok_or_else(|| "This PC has no startup folder.".to_owned())?;
    set_at(&path, turning_on)
}

fn set_at(path: &std::path::Path, turning_on: bool) -> Result<String, String> {
    if turning_on {
        let command = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('"', "\"\"");
        let script = format!(
            "Set WshShell = CreateObject(\"WScript.Shell\")\r\nWshShell.Run \"\"\"{command}\"\"\", 0, False\r\n"
        );
        let mut bytes = vec![0xFF, 0xFE];
        for unit in script.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, bytes).map_err(|error| error.to_string())?;
        Ok(STARTED.to_owned())
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(STOPPED.to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(STOPPED.to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_written_in_the_shells_own_encoding() {
        let dir = std::env::temp_dir().join(format!("yunari-startup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        let entry = path_in(&dir).expect("a startup folder");
        assert!(!entry.exists());
        assert_eq!(set_at(&entry, true), Ok(STARTED.to_owned()));
        let bytes = std::fs::read(&entry).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let needle: Vec<u8> = "WshShell.Run"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(bytes.windows(needle.len()).any(|window| window == needle));
        assert_eq!(set_at(&entry, false), Ok(STOPPED.to_owned()));
        assert!(!entry.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
