//! The store's account routes: sign in, who is signed in, and the author's own
//! items.
//!
//! The browser never sees the session token. A sign-in starts at the Worker's
//! social flow, the Worker's return page hands this host a one-time token, and
//! the host redeems it here — writing the session it buys to the settings
//! directory and answering the popup with a page that has nothing in it but a
//! sentence. Every route that acts as the author's account is loopback only.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::{Value, json};

use yunari_store::client::{Method, Request, Transport};
use yunari_store::{Client, ClientError, Session, SessionStore};

use crate::http::{Shared, is_loopback};

/// How long a started sign-in may take before its state is spent.
const STATE_TTL_MS: i64 = 10 * 60 * 1000;
const STATE_LIMIT: usize = 64;

pub const NO_STORE_MESSAGE: &str = "This host has no store to sign in to.";
pub const SIGNED_OUT_MESSAGE: &str = "Sign in to the Yunari store first.";
pub const NOT_LOOPBACK_MESSAGE: &str = "Store sign-in is local to this PC only.";
pub const NOT_LOCAL_THEME_MESSAGE: &str = "Theme submission is local to this PC only.";

/// One started sign-in, and the loopback origin it was started for.
#[derive(Default)]
pub struct PendingStates {
    pending: Vec<(String, Pending)>,
}

struct Pending {
    host: String,
    at_ms: i64,
}

impl PendingStates {
    /// Remembers a sign-in for `host` and answers its state.
    pub fn remember(&mut self, host: &str, now_ms: i64) -> String {
        self.pending
            .retain(|(_, entry)| now_ms.saturating_sub(entry.at_ms) <= STATE_TTL_MS);
        while self.pending.len() >= STATE_LIMIT {
            self.pending.remove(0);
        }
        let state = uuid();
        self.pending.push((
            state.clone(),
            Pending {
                host: host.to_owned(),
                at_ms: now_ms,
            },
        ));
        state
    }

    /// Spends a state: true only when this server started it, for this host,
    /// and it has not expired. One attempt whatever it answers.
    pub fn spend(&mut self, state: &str, host: &str, now_ms: i64) -> bool {
        let Some(at) = self.pending.iter().position(|(key, _)| key == state) else {
            return false;
        };
        let (_, entry) = self.pending.remove(at);
        entry.host == host && now_ms.saturating_sub(entry.at_ms) <= STATE_TTL_MS
    }
}

/// A random uuid-shaped state; `randomUUID` is exactly this, and the Worker
/// only passes it back.
fn uuid() -> String {
    use rand::prelude::*;
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The store account this host holds, and the sign-ins it started.
pub struct Account {
    pub sessions: SessionStore,
    pub pending: Mutex<PendingStates>,
    /// The Worker's origin; absent when this host has no store.
    pub origin: Option<String>,
    pub transport: Transport,
}

impl Account {
    pub fn new(directory: &Path, origin: Option<String>, transport: Transport) -> Self {
        Self {
            sessions: SessionStore::at(directory),
            pending: Mutex::new(PendingStates::default()),
            origin,
            transport,
        }
    }

    /// The client for this host's held session, or none.
    pub fn client(&self) -> Option<Client> {
        let origin = self.origin.as_deref()?;
        let session = self.sessions.read()?;
        Some(Client::with_send(
            origin,
            session.token,
            Arc::clone(&self.transport),
        ))
    }

    fn forget_on_refusal(&self, error: &ClientError) {
        if error.status == 401 {
            let _ = self.sessions.clear();
        }
    }

    /// One POST of JSON to the Worker, as the sign-in exchange needs it.
    fn post_json(
        &self,
        origin: &str,
        path: &str,
        body: &Value,
    ) -> Result<yunari_store::Reply, String> {
        (self.transport)(&Request {
            method: Method::Post,
            url: format!("{}{path}", origin.trim_end_matches('/')),
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: Some(body.to_string().into_bytes()),
            timeout_ms: yunari_store::client::TIMEOUT_MS,
        })
    }

    fn get(&self, origin: &str, path: &str, token: &str) -> Result<yunari_store::Reply, String> {
        (self.transport)(&Request {
            method: Method::Get,
            url: format!("{}{path}", origin.trim_end_matches('/')),
            headers: vec![("authorization".to_owned(), format!("Bearer {token}"))],
            body: None,
            timeout_ms: yunari_store::client::TIMEOUT_MS,
        })
    }
}

/// The origin this request was made at, when it is a loopback host on a numeric
/// port, otherwise nothing. `[::1]` is sent as `127.0.0.1`, because the Worker
/// accepts only `127.0.0.1` or `localhost` as a return host.
pub fn loopback_origin(headers: &HeaderMap) -> Option<String> {
    let authority = headers.get(header::HOST)?.to_str().ok()?;
    let (name, port) = authority.trim().rsplit_once(':')?;
    let name = name.to_ascii_lowercase();
    if !matches!(name.as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    let name = if name == "localhost" {
        "localhost"
    } else {
        "127.0.0.1"
    };
    Some(format!("http://{name}:{port}"))
}

/// The loopback guard every account route shares.
fn local_peer(peer: SocketAddr) -> Option<Response> {
    if is_loopback(peer.ip()) {
        None
    } else {
        Some((StatusCode::FORBIDDEN, NOT_LOOPBACK_MESSAGE).into_response())
    }
}

/// The session token the exchange bought: the bearer plugin's `set-auth-token`
/// header where the Worker sends one, else the session cookie's own value.
pub fn session_token_of(headers: &[(String, String)]) -> Option<String> {
    const COOKIES: [&str; 2] = [
        "better-auth.session_token",
        "__Secure-better-auth.session_token",
    ];
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("set-auth-token") && !value.is_empty() {
            return percent_decode(value);
        }
    }
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("set-cookie") {
            continue;
        }
        for pair in value.split(';') {
            let pair = pair.trim();
            let Some((cookie, value)) = pair.split_once('=') else {
                continue;
            };
            if COOKIES.contains(&cookie) && !value.is_empty() {
                return percent_decode(value);
            }
        }
    }
    None
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub async fn start(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Some(refusal) = local_peer(peer) {
        return refusal;
    }
    let Some(host) = loopback_origin(&headers) else {
        return (
            StatusCode::BAD_REQUEST,
            "A sign-in can only return to a loopback host.",
        )
            .into_response();
    };
    match start_url(&app, &host) {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(message) if message == NO_STORE_MESSAGE => {
            (StatusCode::SERVICE_UNAVAILABLE, message).into_response()
        }
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

/// The address a browser opens to sign in: the Worker's start page, carrying
/// the loopback host the answer returns to and a state this host remembers
/// once. The editor needs the same exchange the route performs, without
/// making an HTTP request to itself.
pub fn start_url(app: &crate::http::App, host: &str) -> Result<String, String> {
    let Some(origin) = app.account.origin.as_deref() else {
        return Err(NO_STORE_MESSAGE.to_owned());
    };
    let state = app
        .account
        .pending
        .lock()
        .map_err(|_| "The host is not ready.".to_owned())?
        .remember(host, crate::http::now_ms());
    Ok(format!(
        "{}/auth/start?provider=github&host={}&state={}",
        origin.trim_end_matches('/'),
        encode(host),
        encode(&state)
    ))
}

pub async fn complete(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(refusal) = local_peer(peer) {
        return refusal.into_response();
    }
    let Some(origin) = app.account.origin.as_deref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, NO_STORE_MESSAGE).into_response();
    };
    let Some(host) = loopback_origin(&headers) else {
        return sign_in_page(400, "This sign-in did not come back to a loopback host.");
    };
    let state = query.get("state").map(String::as_str).unwrap_or_default();
    let token = query.get("token").map(String::as_str).unwrap_or_default();
    let started = app
        .account
        .pending
        .lock()
        .map(|mut pending| pending.spend(state, &host, crate::http::now_ms()))
        .unwrap_or(false);
    if !started || token.is_empty() {
        return sign_in_page(400, "This sign-in has expired, or was not started here.");
    }

    let exchanged = match app.account.post_json(
        origin,
        "/api/auth/one-time-token/verify",
        &json!({ "token": token }),
    ) {
        Ok(reply) => reply,
        Err(_) => return sign_in_page(502, "The store could not be reached."),
    };
    if !(200..300).contains(&exchanged.status) {
        return sign_in_page(401, "The store did not accept that sign-in.");
    }
    let Some(session_token) = session_token_of(&exchanged.headers) else {
        return sign_in_page(401, "The store answered no session for that sign-in.");
    };

    let answered = match app.account.get(origin, "/api/me", &session_token) {
        Ok(reply) => reply,
        Err(_) => return sign_in_page(502, "The store could not be reached."),
    };
    if !(200..300).contains(&answered.status) {
        return sign_in_page(401, "The store did not accept that session.");
    }
    let Ok(me) = serde_json::from_slice::<Value>(&answered.body) else {
        return sign_in_page(401, "The store answered no account for that sign-in.");
    };
    let Some(session) = session_of(&me, &session_token) else {
        return sign_in_page(401, "The store answered no account for that sign-in.");
    };
    if app.account.sessions.write(&session).is_err() {
        return sign_in_page(500, "This host could not keep that sign-in.");
    }
    sign_in_page(
        200,
        "Signed in to the Yunari store. You can close this window.",
    )
}

fn session_of(me: &Value, token: &str) -> Option<Session> {
    let user_id = me.get("userId")?.as_str().filter(|text| !text.is_empty())?;
    let email = me.get("email")?.as_str().filter(|text| !text.is_empty())?;
    Some(Session {
        token: token.to_owned(),
        user_id: user_id.to_owned(),
        email: email.to_owned(),
        obtained_at: jiff::Timestamp::now().to_string(),
    })
}

fn sign_in_page(status: u16, message: &str) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        Html(format!(
            "<!doctype html>\n<html lang=\"en\">\n<meta charset=\"utf-8\">\n<title>Yunari sign-in</title>\n<p>{message}</p>"
        )),
    )
        .into_response()
}

pub async fn session(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = local_peer(peer) {
        return refusal;
    }
    // The signed-out answer is exact, and a signed-in one never carries token
    // or account id: this route exists to say signed in or not.
    match app.account.sessions.read() {
        Some(session) => Json(json!({ "signedIn": true, "email": session.email })).into_response(),
        None => Json(json!({ "signedIn": false })).into_response(),
    }
}

pub async fn sign_out(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = local_peer(peer) {
        return refusal;
    }
    // The Worker's own session is left to expire: this forgets the token this
    // host holds, which is the one a request from here would carry.
    let _ = app.account.sessions.clear();
    Json(json!({ "ok": true })).into_response()
}

fn signed_out() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "ok": false, "message": SIGNED_OUT_MESSAGE })),
    )
        .into_response()
}

/// One proxied author call: the Worker's refusal keeps its status, an
/// unreachable store is a 502, and a 401 forgets the held session.
fn proxied<F>(app: &Shared, work: F) -> Response
where
    F: FnOnce(&Client) -> Result<Value, ClientError>,
{
    let Some(client) = app.account.client() else {
        return signed_out();
    };
    match work(&client) {
        Ok(value) => Json(value).into_response(),
        Err(error) => {
            app.account.forget_on_refusal(&error);
            let status = if error.status == 401 {
                StatusCode::UNAUTHORIZED
            } else if error.status == 0 {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::from_u16(error.status).unwrap_or(StatusCode::BAD_GATEWAY)
            };
            (
                status,
                Json(json!({ "ok": false, "message": error.message })),
            )
                .into_response()
        }
    }
}

pub async fn mine(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = local_peer(peer) {
        return refusal;
    }
    proxied(&app, Client::mine)
}

pub async fn transition(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path((id, action)): axum::extract::Path<(String, String)>,
) -> Response {
    if let Some(refusal) = local_peer(peer) {
        return refusal;
    }
    // The path id is checked against the catalogue's shape first, so anything
    // else is a 400 here and never a request to the Worker.
    if !is_catalogue_id(&id) {
        return (StatusCode::BAD_REQUEST, "That is not a catalogue id.").into_response();
    }
    match action.as_str() {
        "publish" => proxied(&app, |client| {
            client
                .publish(&id)
                .map(|answer| json!({ "ok": true, "status": answer.status }))
        }),
        "unpublish" => proxied(&app, |client| {
            client
                .unpublish(&id)
                .map(|answer| json!({ "ok": true, "status": answer.status }))
        }),
        _ => (StatusCode::NOT_FOUND, "No such store action.").into_response(),
    }
}

/// The catalogue's id shape: a lowercase UUID, not any slug.
pub fn is_catalogue_id(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(length, part)| {
            part.len() == *length
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

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

    #[test]
    fn a_state_is_bound_to_its_loopback_origin_and_spent_once() {
        let mut pending = PendingStates::default();
        let state = pending.remember("http://127.0.0.1:5229", 1_000);
        assert!(!pending.spend(&state, "http://localhost:5229", 1_001));
        // The failed attempt spent it, so the right origin cannot replay it.
        assert!(!pending.spend(&state, "http://127.0.0.1:5229", 1_002));

        let state = pending.remember("http://127.0.0.1:5229", 1_000);
        assert!(pending.spend(&state, "http://127.0.0.1:5229", 2_000));
    }

    #[test]
    fn an_old_state_is_not_redeemable() {
        let mut pending = PendingStates::default();
        let state = pending.remember("http://127.0.0.1:5229", 1_000);
        assert!(!pending.spend(&state, "http://127.0.0.1:5229", 1_000 + STATE_TTL_MS + 1));
    }

    #[test]
    fn the_loopback_origin_comes_from_the_host_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "localhost:5227".parse().unwrap());
        assert_eq!(
            loopback_origin(&headers).as_deref(),
            Some("http://localhost:5227")
        );
        headers.insert(header::HOST, "[::1]:5227".parse().unwrap());
        assert_eq!(
            loopback_origin(&headers).as_deref(),
            Some("http://127.0.0.1:5227")
        );
        // A name another machine could own is not a return host.
        headers.insert(header::HOST, "evil.example:5227".parse().unwrap());
        assert_eq!(loopback_origin(&headers), None);
        headers.insert(header::HOST, "127.0.0.1".parse().unwrap());
        assert_eq!(loopback_origin(&headers), None);
    }

    #[test]
    fn a_session_token_is_read_from_the_header_or_the_cookie() {
        let header = vec![("set-auth-token".to_owned(), "abc%2F123".to_owned())];
        assert_eq!(session_token_of(&header).as_deref(), Some("abc/123"));
        let cookie = vec![(
            "set-cookie".to_owned(),
            "__Secure-better-auth.session_token=abc.def; Path=/; HttpOnly".to_owned(),
        )];
        assert_eq!(session_token_of(&cookie).as_deref(), Some("abc.def"));
        let other = vec![("set-cookie".to_owned(), "theme=blue; Path=/".to_owned())];
        assert_eq!(session_token_of(&other), None);
    }

    #[test]
    fn a_catalogue_id_is_a_lowercase_uuid_and_nothing_else() {
        assert!(is_catalogue_id("0123abcd-4567-89ab-cdef-0123456789ab"));
        assert!(!is_catalogue_id("aurora"));
        assert!(!is_catalogue_id("0123ABCD-4567-89ab-cdef-0123456789ab"));
        assert!(!is_catalogue_id("0123abcd-4567-89ab-cdef-0123456789a"));
    }
}
