//! One bounded GET, for the programs and services this host reads.
//!
//! Every remote read goes through here, so the deadline, the body cap and the
//! refusal to follow a redirect exist once. A reason never carries the URL: it
//! reaches every display on the network.

use std::time::Duration;

/// A body larger than this is not an answer this host asked for.
pub const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// A URL this client will not read, in words a display can show.
pub fn refusal(url: &str) -> Option<String> {
    let Some((scheme, _)) = url.split_once("://") else {
        return Some(format!("{url} is not an http address."));
    };
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => None,
        other => Some(format!(
            "A {other} address cannot be read by this client; only http and https are."
        )),
    }
}

/// One GET, as its body text.
pub fn get(url: &str, timeout_ms: u64) -> Result<String, String> {
    if let Some(refusal) = refusal(url) {
        return Err(refusal);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(timeout_ms)))
        // A redirect is not the address this host asked.
        .max_redirects(0)
        // The status is read here, so a 404 says what it was rather than
        // turning into a client error with no code.
        .http_status_as_error(false)
        .build()
        .into();

    let mut response = agent.get(url).call().map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("the server answered {status}"));
    }
    response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => {
                "the server answered more than it should.".to_owned()
            }
            other => other.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn only_http_addresses_are_read() {
        assert!(refusal("file:///etc/passwd").is_some());
        assert!(refusal("C:\\secrets").is_some());
        assert!(refusal("ftp://example.com/x").is_some());
        assert!(refusal("http://127.0.0.1:8085/data.json").is_none());
        assert!(refusal("https://api.open-meteo.com/v1/forecast").is_none());
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
            get(&format!("http://127.0.0.1:{port}/data.json"), 5_000).unwrap(),
            "{\"ok\":true}"
        );
        assert_eq!(
            get(&format!("http://127.0.0.1:{port}/data.json"), 5_000).unwrap_err(),
            "the server answered 500"
        );
        answer.join().ok();
    }
}
