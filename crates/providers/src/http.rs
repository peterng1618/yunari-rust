//! One bounded GET, for the programs and services this host reads.
//!
//! Every remote read goes through here, so the deadline, the body cap and the
//! refusal to follow a redirect exist once. A reason never carries the URL: it
//! reaches every display on the network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
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

/// Percent-encodes everything outside the unreserved set, so a query value
/// cannot add a parameter of its own. A credential read from this PC's store
/// goes through it too: the key never becomes part of the URL's grammar.
pub fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The reasons a user-authored address is refused, in the source app's wording.
pub const BLOCKED_ADDRESS_REASON: &str = "the source resolves to a link-local or metadata address";
pub const PRIVATE_ADDRESS_REASON: &str = "the source resolves to a private or loopback address";

/// One GET with the headers a user-authored source names. The headers are the
/// same ones the source app kept in plain text beside the source's address; no
/// credential is stored here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Get {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub timeout_ms: u64,
    /// A user-authored address is refused when it resolves onto this PC's own
    /// network or to a metadata service. Weather's own endpoints leave it off.
    pub public_only: bool,
}

/// One GET, as its body text.
pub fn get(url: &str, timeout_ms: u64) -> Result<String, String> {
    get_with(&Get {
        url: url.to_owned(),
        headers: Vec::new(),
        timeout_ms,
        public_only: false,
    })
}

/// One GET, as its body bytes, for a picture the host will cache.
pub fn get_bytes(url: &str, timeout_ms: u64, limit: u64) -> Result<Vec<u8>, String> {
    bytes_with(
        &Get {
            url: url.to_owned(),
            headers: Vec::new(),
            timeout_ms,
            public_only: false,
        },
        limit,
    )
}

/// One GET, with the source's own headers.
pub fn get_with(request: &Get) -> Result<String, String> {
    let bytes = bytes_with(request, MAX_BODY_BYTES)?;
    String::from_utf8(bytes).map_err(|_| "the answer is not UTF-8 text.".to_owned())
}

fn bytes_with(request: &Get, limit: u64) -> Result<Vec<u8>, String> {
    let url = request.url.as_str();
    if let Some(refusal) = refusal(url) {
        return Err(refusal);
    }
    if request.public_only {
        public_destination(url)?;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(request.timeout_ms)))
        // A redirect is not the address this host asked.
        .max_redirects(0)
        // The status is read here, so a 404 says what it was rather than
        // turning into a client error with no code.
        .http_status_as_error(false)
        .build()
        .into();

    let mut call = agent.get(url);
    for (name, value) in &request.headers {
        call = call.header(name.as_str(), value.as_str());
    }
    let mut response = call.call().map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("the server answered {status}"));
    }
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => {
                "the server answered more than it should.".to_owned()
            }
            other => other.to_string(),
        })
}

/// A URL's host and port, enough to look its addresses up.
fn host_and_port(url: &str) -> Option<(String, u16)> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?.rsplit('@').next()?;
    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None => 80,
        };
        return Some((host.to_owned(), port));
    }
    match authority.split_once(':') {
        Some((host, port)) => Some((host.to_ascii_lowercase(), port.parse().ok()?)),
        None => Some((authority.to_ascii_lowercase(), 80)),
    }
}

/// Refuses a destination on this PC's own network, a link-local or a metadata
/// address. This is the source app's policy for a user-authored URL; the same
/// answer is never invented for a service the host itself names.
///
/// ponytail: the name is resolved here and then dialled by name, so a name that
/// answers differently on the second lookup is not caught. The upgrade is a
/// resolver pinned to the address that was checked.
pub fn public_destination(url: &str) -> Result<(), String> {
    let Some((host, port)) = host_and_port(url) else {
        return Err("the source URL has no host".to_owned());
    };
    if host.is_empty() {
        return Err("the source URL has no host".to_owned());
    }
    let literal: Option<IpAddr> = host.parse().ok();
    let addresses: Vec<IpAddr> = match literal {
        Some(address) => vec![address],
        None => (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| "the source host could not be resolved".to_owned())?
            .map(|address| address.ip())
            .collect(),
    };
    if addresses.is_empty() {
        return Err("the source host has no address".to_owned());
    }
    if addresses.iter().any(|address| is_blocked(*address)) {
        return Err(BLOCKED_ADDRESS_REASON.to_owned());
    }
    if addresses.iter().any(|address| is_private(*address)) {
        return Err(PRIVATE_ADDRESS_REASON.to_owned());
    }
    Ok(())
}

fn is_blocked(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => link_local_v4(v4),
        IpAddr::V6(v6) => match embedded_v4(v6) {
            Some(v4) => link_local_v4(v4),
            None => (v6.segments()[0] & 0xffc0) == 0xfe80,
        },
    }
}

fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = embedded_v4(v6) {
                return private_v4(v4);
            }
            let segments = v6.segments();
            v6 == Ipv6Addr::LOCALHOST
                || segments.iter().all(|segment| *segment == 0)
                || (segments[0] & 0xfe00) == 0xfc00
        }
    }
}

fn link_local_v4(address: Ipv4Addr) -> bool {
    address.octets()[0] == 169 && address.octets()[1] == 254
}

/// RFC 1918, loopback, CGNAT (`100.64.0.0/10`) and `0.0.0.0/8`. Link-local is
/// refused on its own, whatever the source asked for.
fn private_v4(address: Ipv4Addr) -> bool {
    let [a, b, ..] = address.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
}

/// The IPv4 an address carries inside a mapped, translated, NAT64, 6to4 or
/// Teredo form. Teredo hides the client in the last 32 bits XORed with all
/// ones, so it is decoded before any range test.
fn embedded_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return Some(mapped);
    }
    let groups = address.segments();
    let low = Ipv4Addr::new(
        (groups[6] >> 8) as u8,
        groups[6] as u8,
        (groups[7] >> 8) as u8,
        groups[7] as u8,
    );
    let zeros = |until: usize| groups[..until].iter().all(|group| *group == 0);
    if zeros(4) && groups[4] == 0xffff && groups[5] == 0 {
        return Some(low);
    }
    if groups[0] == 0x64 && groups[1] == 0xff9b {
        let well_known = groups[2..6].iter().all(|group| *group == 0);
        let local_use = groups[2] == 1 && groups[3..6].iter().all(|group| *group == 0);
        if well_known || local_use {
            return Some(low);
        }
    }
    if groups[0] == 0x2002 {
        return Some(Ipv4Addr::new(
            (groups[1] >> 8) as u8,
            groups[1] as u8,
            (groups[2] >> 8) as u8,
            groups[2] as u8,
        ));
    }
    if groups[0] == 0x2001 && groups[1] == 0 {
        return Some(Ipv4Addr::from(u32::MAX - u32::from(low)));
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn a_user_authored_address_on_this_pc_s_own_network_is_refused() {
        for url in [
            "http://127.0.0.1:8085/data.json",
            "http://10.1.2.3/data.json",
            "http://192.168.1.5/x",
            "http://172.16.4.4/x",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/x",
            "http://[::1]/x",
            "http://[fd00::1]/x",
            "http://[::ffff:127.0.0.1]/x",
        ] {
            assert!(public_destination(url).is_err(), "{url}");
        }
        assert_eq!(
            public_destination("http://169.254.169.254/").unwrap_err(),
            BLOCKED_ADDRESS_REASON
        );
        assert_eq!(
            public_destination("http://10.0.0.1/").unwrap_err(),
            PRIVATE_ADDRESS_REASON
        );
        // A public literal passes; no name is resolved, so the test needs no
        // network.
        assert!(public_destination("https://1.1.1.1/").is_ok());
    }

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
