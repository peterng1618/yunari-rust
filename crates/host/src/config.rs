//! Where the host keeps its files and how it is told to listen.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The port the source host has always used, and the nine after it.
pub const DEFAULT_PORT: u16 = 5227;
pub const MAX_PORT_ATTEMPTS: u16 = 10;

/// Everything the host was told on the command line, with the source's
/// defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    pub app_dir: PathBuf,
    /// The first port to try; the next nine are tried on `EADDRINUSE`.
    pub port: u16,
    /// Whether phones on the LAN may reach the host.
    pub lan: bool,
    /// Where LibreHardwareMonitor's web server answers.
    pub lhm_url: String,
    /// PresentMon's console program, the frame source after RTSS.
    pub presentmon_path: PathBuf,
}

impl HostConfig {
    pub fn resolve(app_dir: Option<PathBuf>, port: Option<u16>, lan: bool) -> Self {
        Self {
            // An explicit directory wins, then the environment's, then the
            // source's default. The environment is what a test or a second
            // install uses to keep its settings apart.
            app_dir: app_dir
                .or_else(|| std::env::var_os("YUNARI_APP_DIR").map(PathBuf::from))
                .unwrap_or_else(default_app_dir),
            port: port.unwrap_or(DEFAULT_PORT),
            lan,
            lhm_url: lhm_url(&std::env::args().collect::<Vec<_>>()),
            presentmon_path: presentmon_path(),
        }
    }

    pub fn settings_dir(&self) -> PathBuf {
        self.app_dir.join("settings")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.app_dir.join("themes")
    }

    /// The ports to try in order: the asked-for one and the nine after it.
    pub fn ports(&self) -> impl Iterator<Item = u16> {
        (0..MAX_PORT_ATTEMPTS).map(move |step| self.port.saturating_add(step))
    }
}

/// `--lhm-url <url>` from the command line, or LibreHardwareMonitor's default
/// port. A missing value is the default rather than an empty address.
fn lhm_url(args: &[String]) -> String {
    args.iter()
        .position(|arg| arg == "--lhm-url")
        .and_then(|at| args.get(at + 1))
        .filter(|url| !url.trim().is_empty())
        .map_or_else(
            || yunari_providers::lhm::DEFAULT_URL.to_owned(),
            |url| url.trim_end_matches('/').to_owned(),
        )
}

/// `YUNARI_PRESENTMON`, else `PresentMon.exe` beside the app. The file may be
/// absent: the frame provider then reports that it is not installed.
fn presentmon_path() -> PathBuf {
    if let Some(path) = std::env::var_os("YUNARI_PRESENTMON") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
        .join("PresentMon.exe")
}

/// `%USERPROFILE%\.yunari`, or `$HOME`'s equivalent.
pub fn default_app_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".yunari")
}

/// Write JSON through a temporary file in the same directory, so a crash
/// half-way leaves the old file intact.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a file needs a directory",
        ));
    };
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

/// Read a JSON file, or the default when it is absent or unreadable. A broken
/// settings file must not stop the host from starting.
pub(crate) fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let text = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_layout_is_the_sources() {
        let config = HostConfig::resolve(Some(PathBuf::from("C:/app")), None, false);
        assert_eq!(config.settings_dir(), Path::new("C:/app/settings"));
        assert_eq!(config.themes_dir(), Path::new("C:/app/themes"));
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.ports().next(), Some(5227));
        assert_eq!(config.ports().last(), Some(5236));
    }

    #[test]
    fn the_lhm_address_is_the_source_default_unless_it_is_passed() {
        let none = vec!["yunari-host".to_owned()];
        assert_eq!(lhm_url(&none), "http://127.0.0.1:8085");
        let passed = vec![
            "yunari-host".to_owned(),
            "--lhm-url".to_owned(),
            "http://127.0.0.1:9000/".to_owned(),
        ];
        assert_eq!(lhm_url(&passed), "http://127.0.0.1:9000");
        // A flag with no value is the default, not an empty address.
        let dangling = vec!["yunari-host".to_owned(), "--lhm-url".to_owned()];
        assert_eq!(lhm_url(&dangling), "http://127.0.0.1:8085");
    }

    #[test]
    fn presentmon_is_looked_for_beside_the_app() {
        // The environment may point it elsewhere on a test machine, which is
        // exactly what the override is for.
        let path = presentmon_path();
        if std::env::var_os("YUNARI_PRESENTMON").is_none() {
            assert!(path.ends_with("PresentMon.exe"));
        }
    }

    #[test]
    fn a_written_file_is_read_back() {
        let dir = std::env::temp_dir().join(format!("yunari-config-{}", std::process::id()));
        let path = dir.join("value.json");
        write_json(&path, &vec!["one", "two"]).expect("a writable directory");
        let read: Vec<String> = read_json(&path);
        assert_eq!(read, ["one", "two"]);
        let missing: Vec<String> = read_json(&dir.join("absent.json"));
        assert!(missing.is_empty());
        fs::remove_dir_all(&dir).ok();
    }
}
