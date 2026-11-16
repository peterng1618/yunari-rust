//! A plain HTTP/1.1 reader: enough for a program on this machine that publishes
//! JSON, and nothing more.
//!
//! ponytail: plain `http://` only, no TLS, no redirects, no chunked bodies. The
//! only caller today is LibreHardwareMonitor's loopback server. A remote source
//! that needs TLS gets a real client when it lands, not a hand-rolled TLS.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// A URL this client will not read, in words a display can show.
pub fn refusal(url: &str) -> Option<String> {
    let Some((scheme, _)) = url.split_once("://") else {
        return Some(format!("{url} is not an http address."));
    };
    match scheme.to_ascii_lowercase().as_str() {
        "http" => None,
        other => Some(format!(
            "A {other} address cannot be read by this client; only http is."
        )),
    }
}

/// One GET, as its body text.
pub fn get(url: &str, timeout_ms: u64) -> Result<String, String> {
    if let Some(refusal) = refusal(url) {
        return Err(refusal);
    }
    let rest = &url["http://".len()..];
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .map_err(|_| format!("{authority} is not a host and port."))?,
        ),
        None => (authority, 80),
    };
    let timeout = Duration::from_millis(timeout_ms);
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?;
    let mut last_error = format!("{host}:{port} could not be resolved.");
    for address in addresses {
        let mut stream = match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => stream,
            Err(error) => {
                last_error = error.to_string();
                continue;
            }
        };
        stream.set_read_timeout(Some(timeout)).ok();
        stream.set_write_timeout(Some(timeout)).ok();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        return parse_response(&bytes);
    }
    Err(last_error)
}

/// The body of a response, and the status that must be a success.
fn parse_response(bytes: &[u8]) -> Result<String, String> {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "the server answered without headers.".to_owned())?;
    let head = String::from_utf8_lossy(&bytes[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "the server answered without a status.".to_owned())?;
    if !(200..300).contains(&status) {
        return Err(format!("the server answered {status}"));
    }
    let body = &bytes[split + 4..];
    Ok(String::from_utf8_lossy(body).into_owned())
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn only_http_addresses_are_read() {
        assert!(refusal("https://example.com/x").is_some());
        assert!(refusal("file:///etc/passwd").is_some());
        assert!(refusal("C:\\secrets").is_some());
        assert!(refusal("http://127.0.0.1:8085/data.json").is_none());
    }

    #[test]
    fn a_loopback_answer_is_read_and_a_refusal_is_not() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let answer = std::thread::spawn(move || {
            for body in ["{\"ok\":true}", ""] {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0_u8; 512];
                let _ = stream.read(&mut request);
                let status = if body.is_empty() {
                    "500 Oops"
                } else {
                    "200 OK"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        assert_eq!(
            get(&format!("http://127.0.0.1:{port}/data.json"), 2_000).unwrap(),
            "{\"ok\":true}"
        );
        assert_eq!(
            get(&format!("http://127.0.0.1:{port}/data.json"), 2_000).unwrap_err(),
            "the server answered 500"
        );
        answer.join().ok();
    }
}
