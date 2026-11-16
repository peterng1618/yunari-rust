//! One PowerShell helper process, its lines and its stop.
//!
//! Every PowerShell helper this host runs goes through here, so the line cap,
//! the process handle and the kill exist once. The script travels as
//! `-EncodedCommand`, base64 of its UTF-16LE bytes, which is how a script with
//! quotes and braces reaches PowerShell without a shell in between.

use std::io::{BufRead, BufReader};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const POWERSHELL: &str = "powershell.exe";
/// A one-shot helper still running after this is killed; the catalogue moves
/// on without it.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(15);
/// A line larger than this is drained and dropped rather than buffered without
/// bound; the media helper's artwork line is derived from it.
pub const LINE_CAP: usize = 1_048_576;

/// Ends the helper and waits for it to exit.
pub type Stop = Box<dyn FnOnce() + Send>;

pub fn encode(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Starts `script`; its stdout lines arrive on the channel until it exits or is
/// stopped. The error is the operating system's, for the caller to prefix.
pub fn spawn(script: &str) -> Result<(Receiver<String>, Stop), String> {
    let encoded = encode(script);
    let mut child = Command::new(POWERSHELL)
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err("its output could not be read.".to_owned());
    };
    let (sender, lines) = mpsc::channel();
    thread::spawn(move || read(stdout, sender));
    let child = Arc::new(Mutex::new(child));
    let stopper = Arc::clone(&child);
    Ok((
        lines,
        Box::new(move || {
            let mut child = stopper
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }),
    ))
}

/// Runs `script` to completion and answers everything it printed. A script
/// that outlives [`RUN_TIMEOUT`] is killed, so a catalogue read cannot wedge
/// the caller.
pub fn run(script: &str) -> Result<String, String> {
    let (lines, stop) = spawn(script)?;
    let mut stop = Some(stop);
    let mut output = String::new();
    loop {
        match lines.recv_timeout(RUN_TIMEOUT) {
            Ok(line) => {
                output.push_str(&line);
                output.push('\n');
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(output),
            Err(RecvTimeoutError::Timeout) => {
                if let Some(stop) = stop.take() {
                    stop();
                }
                return Err(format!(
                    "it did not finish within {} seconds.",
                    RUN_TIMEOUT.as_secs()
                ));
            }
        }
    }
}

fn read(stdout: ChildStdout, sender: mpsc::Sender<String>) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        match read_line(&mut reader, &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line).into_owned();
        if sender.send(text).is_err() {
            return;
        }
    }
}

/// Reads one line, keeping at most [`LINE_CAP`] bytes of it. A longer line is
/// drained and dropped, so a wedged script cannot grow this buffer.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<usize> {
    line.clear();
    let mut total = 0;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(total);
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let end = newline.unwrap_or(available.len());
        if line.len() < LINE_CAP {
            let room = LINE_CAP - line.len();
            line.extend_from_slice(&available[..end.min(room)]);
        }
        let consumed = end + usize::from(newline.is_some());
        reader.consume(consumed);
        total += consumed;
        if newline.is_some() {
            return Ok(total);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_encoded_script_is_the_script() {
        use base64::Engine;
        let script = "$x = 'y'";
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encode(script))
            .expect("base64");
        let units: Vec<u16> = decoded
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), script);
    }

    #[test]
    fn a_line_longer_than_the_cap_is_dropped_not_buffered() {
        let mut input = Vec::new();
        input.extend(std::iter::repeat_n(b'x', LINE_CAP + 64));
        input.push(b'\n');
        input.extend_from_slice(b"{\"kind\":\"none\"}\n");
        let mut reader = std::io::BufReader::new(input.as_slice());
        let mut line = Vec::new();
        assert!(read_line(&mut reader, &mut line).unwrap() > LINE_CAP);
        assert_eq!(line.len(), LINE_CAP);
        read_line(&mut reader, &mut line).unwrap();
        assert_eq!(line, b"{\"kind\":\"none\"}");
    }
}
