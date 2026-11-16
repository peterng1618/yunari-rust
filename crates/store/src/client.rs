//! The author's half of the store, as this host calls it.
//!
//! Every method returns the Worker's payload and fails on a non-2xx except
//! [`Client::baseline`], whose 404 is a first submission rather than a refusal.
//! The token goes in the `Authorization` header on every call, the upload
//! route included. One transport function is used for all of them, so the
//! deadline, the body cap and the refusal to follow a redirect exist once.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::session::Session;

/// A body larger than this is not an answer the store sends.
pub const MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;
pub const TIMEOUT_MS: u64 = 5_000;
pub const UPLOAD_TIMEOUT_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The transport. A seam so a test scripts the Worker's answers.
pub type Transport = Arc<dyn Fn(&Request) -> Result<Reply, String> + Send + Sync>;

/// One HTTP call with this build's bounds.
pub fn send(request: &Request) -> Result<Reply, String> {
    let Some((scheme, _)) = request.url.split_once("://") else {
        return Err(format!("{} is not an http address.", request.url));
    };
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return Err(format!(
            "A {scheme} address cannot be read; only http and https are."
        ));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(request.timeout_ms)))
        // A redirect is not the address the store declared.
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into();
    let url = request.url.as_str();
    let mut response = if matches!(request.method, Method::Post | Method::Put) {
        let mut call = if request.method == Method::Post {
            agent.post(url)
        } else {
            agent.put(url)
        };
        for (name, value) in &request.headers {
            call = call.header(name.as_str(), value.as_str());
        }
        let body = request.body.as_deref().unwrap_or_default();
        call.send(body)
    } else {
        let mut call = if request.method == Method::Get {
            agent.get(url)
        } else {
            agent.delete(url)
        };
        for (name, value) in &request.headers {
            call = call.header(name.as_str(), value.as_str());
        }
        call.call()
    }
    .map_err(|error| error.to_string())?;

    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_vec()
        .map_err(|error| error.to_string())?;
    Ok(Reply {
        status,
        headers,
        body,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientError {
    /// The Worker's HTTP status; 0 when it could not be reached at all.
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Baseline {
    pub version: String,
    pub created_at: String,
    pub envelope: Value,
    pub credits: Option<Value>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateInput {
    pub envelope: Value,
    pub name: String,
    pub credits: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub version: String,
    pub status: String,
    pub upload_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finalized {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transitioned {
    pub status: String,
}

pub struct Client {
    origin: String,
    token: String,
    send: Transport,
}

impl Client {
    pub fn new(origin: impl Into<String>, token: impl Into<String>) -> Self {
        Self::with_send(origin, token, Arc::new(send))
    }

    pub fn with_send(origin: impl Into<String>, token: impl Into<String>, send: Transport) -> Self {
        Self {
            origin: origin.into(),
            token: token.into(),
            send,
        }
    }

    /// The client for a held session, or none when this host holds no token.
    pub fn for_session(origin: &str, session: Option<&Session>) -> Option<Self> {
        session.map(|session| Self::new(origin, session.token.clone()))
    }

    fn item_path(id: &str) -> String {
        format!("/api/me/items/{}", encode(id))
    }

    fn call(&self, path: &str, method: Method, body: Option<Value>) -> Result<Reply, ClientError> {
        let mut headers = vec![("authorization".to_owned(), format!("Bearer {}", self.token))];
        let body = match body {
            Some(value) => {
                headers.push(("content-type".to_owned(), "application/json".to_owned()));
                Some(value.to_string().into_bytes())
            }
            None => None,
        };
        let request = Request {
            method,
            url: format!("{}{path}", self.origin.trim_end_matches('/')),
            headers,
            body,
            timeout_ms: TIMEOUT_MS,
        };
        let reply = (self.send)(&request).map_err(|message| ClientError { status: 0, message })?;
        if !(200..300).contains(&reply.status) {
            return Err(ClientError {
                status: reply.status,
                message: sentence_of(&reply.body, reply.status),
            });
        }
        Ok(reply)
    }

    fn json(&self, path: &str, method: Method, body: Option<Value>) -> Result<Value, ClientError> {
        let reply = self.call(path, method, body)?;
        parse(&reply.body, reply.status)
    }

    /// The latest stored version in any state, or none when there is none.
    pub fn baseline(&self, id: &str) -> Result<Option<Baseline>, ClientError> {
        let reply = match self.call(
            &format!("{}/baseline", Self::item_path(id)),
            Method::Get,
            None,
        ) {
            Ok(reply) => reply,
            Err(error) if error.status == 404 => return Ok(None),
            Err(error) => return Err(error),
        };
        let value = parse(&reply.body, reply.status)?;
        Ok(Some(Baseline {
            version: text(&value, "version").ok_or_else(unreadable)?,
            created_at: text(&value, "createdAt").unwrap_or_default(),
            envelope: value.get("envelope").cloned().unwrap_or(Value::Null),
            credits: value
                .get("credits")
                .cloned()
                .filter(|value| !value.is_null()),
            status: text(&value, "status").unwrap_or_default(),
        }))
    }

    pub fn create_version(&self, id: &str, input: &CreateInput) -> Result<Plan, ClientError> {
        let mut body = serde_json::json!({
            "envelope": input.envelope,
            "name": input.name,
        });
        if let Some(credits) = &input.credits {
            body["credits"] = credits.clone();
        }
        let value = self.json(
            &format!("{}/versions", Self::item_path(id)),
            Method::Post,
            Some(body),
        )?;
        Ok(Plan {
            version: text(&value, "version").ok_or_else(unreadable)?,
            status: text(&value, "status").unwrap_or_default(),
            upload_url: text(&value, "uploadUrl").ok_or_else(unreadable)?,
        })
    }

    /// PUTs the package through the Worker's authenticated upload route.
    pub fn upload(&self, url: &str, bytes: &[u8]) -> Result<(), ClientError> {
        let request = Request {
            method: Method::Put,
            url: url.to_owned(),
            headers: vec![
                ("authorization".to_owned(), format!("Bearer {}", self.token)),
                ("content-length".to_owned(), bytes.len().to_string()),
            ],
            body: Some(bytes.to_vec()),
            timeout_ms: UPLOAD_TIMEOUT_MS,
        };
        let reply = (self.send)(&request).map_err(|message| ClientError { status: 0, message })?;
        if !(200..300).contains(&reply.status) {
            return Err(ClientError {
                status: reply.status,
                message: sentence_of(&reply.body, reply.status),
            });
        }
        Ok(())
    }

    /// The package's version, from when the upload ended. A 404 here is not a
    /// first submission: the version was planned before the upload.
    pub fn finalize(
        &self,
        id: &str,
        version: &str,
        thumbnail: &[u8],
        luminance: Option<f64>,
    ) -> Result<Finalized, ClientError> {
        use base64::Engine;
        let mut body = serde_json::json!({
            "thumbnail": base64::engine::general_purpose::STANDARD.encode(thumbnail),
        });
        if let Some(luminance) = luminance {
            body["thumbnailLuminance"] = serde_json::json!(luminance);
        }
        let value = self.json(
            &format!(
                "{}/versions/{}/finalize",
                Self::item_path(id),
                encode(version)
            ),
            Method::Post,
            Some(body),
        )?;
        Ok(Finalized {
            status: text(&value, "status").unwrap_or_default(),
            version: text(&value, "version").unwrap_or_else(|| version.to_owned()),
        })
    }

    pub fn unpublish(&self, id: &str) -> Result<Transitioned, ClientError> {
        self.transition(id, "unpublish")
    }

    pub fn publish(&self, id: &str) -> Result<Transitioned, ClientError> {
        self.transition(id, "publish")
    }

    fn transition(&self, id: &str, action: &str) -> Result<Transitioned, ClientError> {
        let value = self.json(
            &format!("{}/{}", Self::item_path(id), action),
            Method::Post,
            None,
        )?;
        Ok(Transitioned {
            status: text(&value, "status").unwrap_or_default(),
        })
    }

    /// The author's own rows, in the Worker's own order.
    pub fn mine(&self) -> Result<Value, ClientError> {
        self.json("/api/me/items", Method::Get, None)
    }

    pub fn favorite(&self, id: &str, on: bool) -> Result<Value, ClientError> {
        self.json(
            &format!("{}/favorite", Self::item_path(id)),
            if on { Method::Put } else { Method::Delete },
            None,
        )
    }
}

/// The Worker's sentence for a refusal, whatever shape it arrived in.
fn sentence_of(body: &[u8], status: u16) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        for key in ["message", "error"] {
            if let Some(text) = value.get(key).and_then(Value::as_str)
                && !text.is_empty()
            {
                return text.to_owned();
            }
        }
    }
    let trimmed = String::from_utf8_lossy(body);
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        format!("The store answered HTTP {status}.")
    } else {
        trimmed.to_owned()
    }
}

fn unreadable() -> ClientError {
    ClientError {
        status: 200,
        message: "The store answered something unexpected.".to_owned(),
    }
}

fn parse(body: &[u8], status: u16) -> Result<Value, ClientError> {
    serde_json::from_slice(body).map_err(|_| ClientError {
        status,
        message: "The store answered something that is not JSON.".to_owned(),
    })
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Percent-encodes everything outside the unreserved set, so an id cannot add
/// a path segment of its own.
fn encode(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fixture {
        client: Client,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    fn with_reply(reply: Reply) -> Fixture {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let send: Transport = Arc::new(move |request| {
            seen.lock().unwrap().push(request.clone());
            Ok(reply.clone())
        });
        Fixture {
            client: Client::with_send("https://store.example/", "the-token", send),
            requests,
        }
    }

    fn answer(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn every_call_carries_the_token_and_the_origin() {
        let fixture = with_reply(answer(200, r#"{"entries":[]}"#));
        fixture.client.mine().expect("the author's rows");
        let requests = fixture.requests.lock().unwrap();
        let request = &requests[0];
        assert_eq!(request.url, "https://store.example/api/me/items");
        assert_eq!(request.method, Method::Get);
        assert!(
            request
                .headers
                .contains(&("authorization".to_owned(), "Bearer the-token".to_owned()))
        );
    }

    #[test]
    fn a_first_submission_has_no_baseline_and_a_missing_one_is_no_error() {
        let fixture = with_reply(answer(404, ""));
        assert_eq!(fixture.client.baseline("aurora"), Ok(None));

        let fixture = with_reply(answer(
            200,
            r#"{"version":"1.2.0","createdAt":"2026-11-01T00:00:00Z",
                "envelope":{},"status":"published"}"#,
        ));
        let baseline = fixture
            .client
            .baseline("aurora")
            .expect("a lookup")
            .expect("a stored version");
        assert_eq!(baseline.version, "1.2.0");
        assert_eq!(baseline.status, "published");
    }

    #[test]
    fn a_refusal_is_the_stores_own_sentence() {
        let fixture = with_reply(answer(401, r#"{"message":"Sign in again."}"#));
        assert_eq!(
            fixture.client.mine(),
            Err(ClientError {
                status: 401,
                message: "Sign in again.".to_owned(),
            })
        );

        let fixture = with_reply(answer(502, "the store is down"));
        assert_eq!(
            fixture.client.mine(),
            Err(ClientError {
                status: 502,
                message: "the store is down".to_owned(),
            })
        );

        let fixture = with_reply(answer(500, ""));
        assert_eq!(
            fixture.client.mine().unwrap_err().message,
            "The store answered HTTP 500."
        );
    }

    #[test]
    fn a_payload_that_is_not_json_is_refused_rather_than_guessed() {
        let fixture = with_reply(answer(200, "<html>hooray</html>"));
        assert_eq!(
            fixture.client.mine().unwrap_err().message,
            "The store answered something that is not JSON."
        );
    }

    #[test]
    fn an_upload_sends_the_bytes_and_its_length() {
        let fixture = with_reply(answer(200, ""));
        fixture
            .client
            .upload("https://store.example/api/upload/x", b"package")
            .expect("an upload");
        let requests = fixture.requests.lock().unwrap();
        let request = &requests[0];
        assert_eq!(request.method, Method::Put);
        assert_eq!(request.body.as_deref(), Some(b"package".as_slice()));
        assert!(
            request
                .headers
                .contains(&("content-length".to_owned(), "7".to_owned()))
        );
    }

    #[test]
    fn an_id_cannot_add_a_path_segment_of_its_own() {
        assert_eq!(encode("a/b"), "a%2Fb");
        let fixture = with_reply(answer(404, ""));
        fixture.client.baseline("a/b").expect("a lookup");
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://store.example/api/me/items/a%2Fb/baseline"
        );
    }
}
