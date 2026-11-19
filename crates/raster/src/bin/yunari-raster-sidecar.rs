//! The native panel raster sidecar process.
//!
//! Reads one JSON request per line from stdin and writes one JSON response per
//! line to stdout. Diagnostics go to stderr so they can never be mistaken for a
//! protocol line. No sockets, credentials or writes outside the response stream.

use std::io::{BufRead, BufWriter, Write};

use yunari_render_raster::sidecar::{handle_line, Sessions};

fn main() {
    let mut state = Sessions::new();
    let stdin = std::io::stdin();
    let mut stdout = BufWriter::new(std::io::stdout().lock());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("sidecar: could not read a request line: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match handle_line(&mut state, &line) {
            Ok(Some(response)) => {
                let write = writeln!(stdout, "{response}").and_then(|()| stdout.flush());
                if let Err(error) = write {
                    eprintln!("sidecar: could not write a response line: {error}");
                    return;
                }
            }
            Ok(None) => return, // orderly shutdown
            Err(error) => {
                let response = error.to_response_line();
                let write = writeln!(stdout, "{response}").and_then(|()| stdout.flush());
                if let Err(error) = write {
                    eprintln!("sidecar: could not write an error response: {error}");
                    return;
                }
            }
        }
    }
}
