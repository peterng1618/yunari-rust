//! Frame timing from Intel PresentMon's ETW trace, the fallback after RTSS.
//!
//! PresentMon is a separate console program, not a helper script: it writes CSV
//! to stdout. Yunari never passes `--restart_as_admin` (which prompts for UAC
//! per launch) or `--stop_existing_session` (which would kill a session the user
//! started); a denied trace session and a name conflict are reported as
//! unavailable and retried only at the next demand window.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use super::{FrameEvent, FrameSource, Stop};

pub const PRESENTMON_ARGS: &[&str] = &["--output_stdout", "--no_csv", "--session_name", "Yunari"];

pub const PRESENTMON_MISSING: &str =
    "PresentMon is not installed. Put PresentMon.exe beside the Yunari app, or turn on RTSS.";
pub const PRESENTMON_ACCESS_DENIED: &str = "PresentMon access denied: add the account to the Performance Log Users group, or run Yunari elevated.";
pub const PRESENTMON_SESSION_BUSY: &str = "A PresentMon trace session named \"Yunari\" is already running. It will be retried at the next demand window.";
pub const PRESENTMON_STOPPED: &str = "PresentMon stopped reporting frames.";

/// PresentMon 2.x writes this column for the frame time; the metric name varies
/// with the version, the column name does not.
const FRAMETIME_COLUMN: &str = "msbetweenpresents";
const APP_COLUMN: &str = "application";
const MISSING_APP: &str = "<unknown>";
const STDERR_CAP: usize = 2_000;

/// One data row, read against the header it arrived under.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub frametime_ms: f64,
    /// `None` when PresentMon names no process for the frame.
    pub app: Option<String>,
}

/// The column names from a PresentMon header line, lowercased and trimmed, or
/// `None` for a line that is not a header.
pub fn parse_header(line: &str) -> Option<Vec<String>> {
    let names: Vec<String> = split_csv(line)?
        .into_iter()
        .map(|field| field.trim().to_lowercase())
        .collect();
    names
        .iter()
        .any(|name| name == FRAMETIME_COLUMN)
        .then_some(names)
}

/// One data row read against an explicit column list.
///
/// A row whose field count differs from the header is not that header's row —
/// the check is what makes a stale header return `None` rather than a value from
/// the wrong column. Header lines, `NA`, and non-positive frametimes are all not
/// readings.
pub fn parse_row(line: &str, columns: &[String]) -> Option<Row> {
    let fields = split_csv(line)?;
    if fields.len() != columns.len() {
        return None;
    }
    let frametime = columns.iter().position(|name| name == FRAMETIME_COLUMN)?;
    let raw = fields[frametime].trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("NA") {
        return None;
    }
    let frametime_ms = raw.parse::<f64>().ok().filter(|value| *value > 0.0)?;

    let app = columns
        .iter()
        .position(|name| name == APP_COLUMN)
        .map(|index| fields[index].trim())
        .filter(|name| {
            !name.is_empty()
                && !name.eq_ignore_ascii_case("NA")
                && !name.eq_ignore_ascii_case(MISSING_APP)
        })
        .map(str::to_owned);
    Some(Row { frametime_ms, app })
}

/// Splits one CSV line, honouring quoted fields.
fn split_csv(line: &str) -> Option<Vec<String>> {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        if quoted {
            match character {
                '"' if characters.peek() == Some(&'"') => {
                    field.push('"');
                    characters.next();
                }
                '"' => quoted = false,
                other => field.push(other),
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            ',' => fields.push(std::mem::take(&mut field)),
            other => field.push(other),
        }
    }
    // An unterminated quote is not a row.
    if quoted {
        return None;
    }
    fields.push(field);
    Some(fields)
}

/// What PresentMon's stderr means, in words a display can show.
fn stderr_reason(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("access denied") || lower.contains("performance log users") {
        return PRESENTMON_ACCESS_DENIED.to_owned();
    }
    if lower.contains("already running") && lower.contains("session") {
        return PRESENTMON_SESSION_BUSY.to_owned();
    }
    let tail = stderr.trim();
    if tail.is_empty() {
        PRESENTMON_STOPPED.to_owned()
    } else {
        format!(
            "{PRESENTMON_STOPPED} {}",
            tail.rsplit('\n').next().unwrap_or("")
        )
    }
}

pub struct PresentMonSource {
    executable: PathBuf,
}

impl PresentMonSource {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }
}

impl FrameSource for PresentMonSource {
    fn id(&self) -> &'static str {
        "presentmon"
    }

    fn start(&self) -> Result<(Receiver<FrameEvent>, Stop), String> {
        if !self.executable.is_file() {
            return Err(PRESENTMON_MISSING.to_owned());
        }
        let mut command = Command::new(&self.executable);
        command
            .args(PRESENTMON_ARGS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("PresentMon could not be started: {error}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let running = Arc::new(Mutex::new(Some(child)));
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || read(stdout, stderr, sender));

        let killer = Arc::clone(&running);
        Ok((
            receiver,
            Box::new(move || {
                // Taking the child needs the lock; a wait under it cannot block
                // the reader, which owns only the pipes.
                if let Ok(mut slot) = killer.lock()
                    && let Some(mut child) = slot.take()
                {
                    child.kill().ok();
                    child.wait().ok();
                }
                handle.join().ok();
            }) as Stop,
        ))
    }
}

/// Reads PresentMon's stdout to the end, then says why it ended.
fn read(stdout: Option<ChildStdout>, stderr: Option<ChildStderr>, sender: Sender<FrameEvent>) {
    let collected = Arc::new(Mutex::new(String::new()));
    let stderr_thread = stderr.map(|mut stderr| {
        let collected = Arc::clone(&collected);
        thread::spawn(move || {
            let mut text = String::new();
            stderr.read_to_string(&mut text).ok();
            if let Ok(mut held) = collected.lock() {
                let keep = text.len().saturating_sub(STDERR_CAP);
                held.push_str(&text[keep..]);
            }
        })
    });

    if let Some(stdout) = stdout {
        let mut columns: Option<Vec<String>> = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(header) = parse_header(&line) {
                columns = Some(header);
                continue;
            }
            let Some(columns) = &columns else {
                continue;
            };
            let Some(row) = parse_row(&line, columns) else {
                continue;
            };
            let event = FrameEvent::Frame {
                frametime_ms: row.frametime_ms,
                app: row.app,
            };
            if sender.send(event).is_err() {
                return;
            }
        }
    }

    if let Some(stderr_thread) = stderr_thread {
        stderr_thread.join().ok();
    }
    let text = collected
        .lock()
        .map(|held| held.clone())
        .unwrap_or_default();
    // No liveness timeout: PresentMon writes its header only with the first
    // frame, so a silent capture is a healthy one waiting for a game. The
    // process ending is the failure signal.
    let _ = sender.send(FrameEvent::Unavailable(stderr_reason(&text)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "Application,ProcessID,SwapChainAddress,Runtime,SyncInterval,PresentFlags,AllowsTearing,PresentMode,msBetweenPresents";

    #[test]
    fn a_header_is_lowercased_and_must_carry_the_frametime_column() {
        let header = parse_header(HEADER).expect("a header");
        assert_eq!(header[0], "application");
        assert_eq!(header[8], "msbetweenpresents");
        assert!(parse_header("not,a,header").is_none());
        assert!(parse_header("").is_none());
    }

    #[test]
    fn a_row_reads_the_frametime_and_the_application() {
        let columns = parse_header(HEADER).expect("a header");
        let row = parse_row(
            "game.exe,4242,0x0,1,0,0,0,Hardware Independent,8.5",
            &columns,
        )
        .expect("a row");
        assert!((row.frametime_ms - 8.5).abs() < 1e-9);
        assert_eq!(row.app.as_deref(), Some("game.exe"));
    }

    #[test]
    fn a_row_that_does_not_match_its_header_is_not_a_reading() {
        let columns = parse_header(HEADER).expect("a header");
        // One fewer field: a stale header would read the wrong column.
        assert_eq!(parse_row("game.exe,4242,0x0,1,0,0,0,8.5", &columns), None);
        assert_eq!(parse_row(HEADER, &columns), None);
    }

    #[test]
    fn a_missing_application_or_frametime_is_no_frame() {
        let columns = parse_header(HEADER).expect("a header");
        assert_eq!(
            parse_row("<unknown>,1,0x0,1,0,0,0,mode,8.5", &columns)
                .expect("a row")
                .app,
            None
        );
        assert_eq!(parse_row("game.exe,1,0x0,1,0,0,0,mode,NA", &columns), None);
        assert_eq!(parse_row("game.exe,1,0x0,1,0,0,0,mode,0", &columns), None);
        assert_eq!(parse_row("game.exe,1,0x0,1,0,0,0,mode,-2", &columns), None);
    }

    #[test]
    fn a_quoted_field_keeps_its_comma() {
        assert_eq!(
            split_csv("a,\"b,c\",d"),
            Some(vec!["a".to_owned(), "b,c".to_owned(), "d".to_owned()])
        );
        // An unterminated quote is not a row.
        assert_eq!(split_csv("a,\"b"), None);
    }

    #[test]
    fn stderr_names_the_action_a_denied_trace_needs() {
        assert_eq!(
            stderr_reason("Error: access is denied. Add the user to Performance Log Users."),
            PRESENTMON_ACCESS_DENIED
        );
        assert_eq!(
            stderr_reason("A trace session named Yunari is already running."),
            PRESENTMON_SESSION_BUSY
        );
        assert_eq!(stderr_reason(""), PRESENTMON_STOPPED);
        assert!(stderr_reason("boom\ndetail").ends_with("detail"));
    }

    #[test]
    fn a_missing_executable_is_refused_before_a_process_starts() {
        let source = PresentMonSource::new("definitely/not/here/PresentMon.exe");
        assert_eq!(source.start().err().as_deref(), Some(PRESENTMON_MISSING));
    }
}
