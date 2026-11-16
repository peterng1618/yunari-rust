//! The routes and the state they answer from.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::HostConfig;
use crate::displays::{DisplayShape, DisplayStore};
use crate::identity::HostIdentity;
use crate::pairing::{ClaimError, NO_PAIRING_CODE, Pairing};

pub const SESSION_HEADER: &str = "x-yunari-session";
pub const SESSION_COOKIE: &str = "yunari_session";
/// Roughly 400 days, as the source's cookie.
pub const SESSION_COOKIE_MAX_AGE: i64 = 34_560_000;
pub const NO_DISPLAYS: &str = "Displays are available on this PC only.";

/// Everything a handler may need, shared by every connection.
pub struct App {
    pub config: HostConfig,
    pub identity: HostIdentity,
    pub displays: Mutex<DisplayStore>,
    pub pairing: Mutex<Pairing>,
    pub authority: Option<crate::tls::Authority>,
    pub library: yunari_library::Library,
    /// The port the listener actually took, which the first attempt may not be.
    pub bound_port: std::sync::atomic::AtomicU16,
    pub streams: crate::stream::Streams,
    pub registry: yunari_providers::registry::Registry,
}

pub type Shared = Arc<App>;

pub fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

pub fn router(app: Shared) -> axum::Router {
    let guarded = axum::Router::new()
        .route("/api/session", post(session))
        .route("/api/pairing/claim", post(claim))
        .route("/yunari-ca.crt", get(certificate))
        .route("/api/host", get(host))
        .route("/api/health", get(health))
        .route("/pair", get(pair_page))
        .route("/api/pairing/code", get(code).post(mint_code))
        .route(
            "/api/pairing/sessions",
            get(sessions).post(new_session),
        )
        .route("/api/displays", get(displays))
        .route("/ws", get(ws))
        .route("/api/sensors", get(sensors))
        .with_state(app.clone());
    guarded.layer(axum::middleware::from_fn_with_state(
        app,
        refuse_foreign_loopback_api,
    ))
}

/// A loopback page may only call `/api/*` from its own origin. The host
/// itself is exempt: a phone reads it before it is paired.
async fn refuse_foreign_loopback_api(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path.starts_with("/api/") && path != "/api/host" && peer.ip().is_loopback() {
        let headers = request.headers();
        if let Some(origin) = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok())
            && let Some(host) = headers.get(header::HOST).and_then(|value| value.to_str().ok())
            && host_of(origin) != host_of(host)
        {
            let _ = &app;
            return (StatusCode::FORBIDDEN, "That request is from another origin.").into_response();
        }
    }
    next.run(request).await
}

fn host_of(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    let without_scheme = value.split_once("://").map_or(value.as_str(), |(_, rest)| rest);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    authority
        .rsplit_once(':')
        .filter(|(name, port)| !name.ends_with(']') || *port != "]")
        .map_or(authority, |(name, _)| name)
        .to_owned()
}

pub fn is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => address.is_loopback(),
        IpAddr::V6(address) => address.is_loopback() || address.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
    }
}

/// The token a request proves, from the cookie first and then the header.
/// The query string is never accepted here; `/?session=` has its own exchange.
pub fn request_token(headers: &HeaderMap) -> Option<String> {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|value| value.to_str().ok()) {
        for pair in cookie.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name == SESSION_COOKIE && !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    headers
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn authenticated(app: &App, headers: &HeaderMap) -> Option<String> {
    let token = request_token(headers)?;
    app.displays.lock().ok()?.authenticate(&token)
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={token}; Max-Age={SESSION_COOKIE_MAX_AGE}; HttpOnly; SameSite=Strict; Path=/{secure}"
    )
}

async fn health() -> impl IntoResponse {
    Json(json!({"ok": true}))
}

async fn host(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let mut body = json!({
        "id": app.identity.id,
        "name": app.identity.name,
        "port": app.bound_port.load(std::sync::atomic::Ordering::Relaxed),
    });
    if authenticated(&app, &headers).is_some()
        && let Some(token) = request_token(&headers)
    {
        body["token"] = Value::String(token);
    }
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

async fn certificate(State(app): State<Shared>) -> Response {
    let Some(authority) = &app.authority else {
        return (StatusCode::NOT_FOUND, "This host serves no certificate of its own.").into_response();
    };
    let mut response = authority.certificate_pem().to_owned().into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-x509-ca-cert"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"yunari-ca.crt\""),
    );
    if let Ok(value) = HeaderValue::from_str(&authority.fingerprint_hex()) {
        headers.insert("x-yunari-ca-sha256", value);
    }
    response
}

async fn pair_page(State(app): State<Shared>, Query(query): Query<PairQuery>) -> Response {
    crate::pages::pair_page(&app, query.code.as_deref(), query.reason.as_deref())
}

#[derive(Debug, Deserialize)]
struct PairQuery {
    code: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionBody {
    token: String,
}

async fn session(State(app): State<Shared>, headers: HeaderMap, Json(body): Json<SessionBody>) -> Response {
    let Some(_display) = app
        .displays
        .lock()
        .ok()
        .and_then(|mut store| store.authenticate(&body.token))
    else {
        return (StatusCode::FORBIDDEN, "That session is not one this host issued.").into_response();
    };
    let secure = app.authority.is_some();
    let mut response = StatusCode::FOUND.into_response();
    let out = response.headers_mut();
    out.insert(header::LOCATION, HeaderValue::from_static("/"));
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&session_cookie(&body.token, secure)) {
        out.insert(header::SET_COOKIE, value);
    }
    let _ = headers;
    response
}

#[derive(Debug, Deserialize)]
struct ClaimBody {
    code: String,
    label: String,
    shape: Option<String>,
    rotation: Option<u16>,
}

async fn claim(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<ClaimBody>,
) -> Response {
    let code: String = body
        .code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    let shape = match body.shape.as_deref() {
        None => None,
        Some(name) => match DisplayShape::parse(name) {
            Some(shape) => Some(shape),
            None => return (StatusCode::BAD_REQUEST, "A display is a rect or a circle.").into_response(),
        },
    };
    if let (Some(degrees), Some(shape)) = (body.rotation, shape)
        && !shape.accepts_rotation(degrees)
    {
        return (StatusCode::BAD_REQUEST, "That rotation is not one of this display's.")
            .into_response();
    }
    let remote = peer.ip().to_string();
    let now = now_ms();
    let matched = app
        .pairing
        .lock()
        .map(|pairing| pairing.matches(&code, now))
        .unwrap_or(false);
    if !matched {
        let wait = app.pairing.lock().map(|mut pairing| pairing.miss(&remote)).unwrap_or(0);
        if wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(u64::try_from(wait).unwrap_or(4_000))).await;
        }
        let now = now_ms();
        let matched = app
            .pairing
            .lock()
            .map(|pairing| pairing.matches(&code, now))
            .unwrap_or(false);
        if !matched {
            return (StatusCode::FORBIDDEN, NO_PAIRING_CODE).into_response();
        }
    }
    let mut pairing = match app.pairing.lock() {
        Ok(pairing) => pairing,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response(),
    };
    let mut store = match app.displays.lock() {
        Ok(store) => store,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response(),
    };
    match pairing.claim(&code, now_ms(), &mut store, &body.label, shape, body.rotation) {
        Ok((record, token)) => Json(json!({
            "displayId": record.id,
            "token": token,
        }))
        .into_response(),
        Err(ClaimError::Refused) => (StatusCode::FORBIDDEN, NO_PAIRING_CODE).into_response(),
        Err(ClaimError::Label(refusal)) => (StatusCode::BAD_REQUEST, refusal.0).into_response(),
    }
}

/// These routes are reachable from any paired phone, so each checks the
/// address itself rather than trusting a middleware order.
fn loopback_only(peer: SocketAddr) -> Option<Response> {
    if is_loopback(peer.ip()) {
        None
    } else {
        Some((StatusCode::FORBIDDEN, NO_DISPLAYS).into_response())
    }
}

async fn code(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let view = match app.pairing.lock() {
        Ok(mut pairing) => pairing.current(now_ms()),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response(),
    };
    Json(json!({"code": view.code, "expiresAt": view.expires_at})).into_response()
}

async fn mint_code(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let view = match app.pairing.lock() {
        Ok(mut pairing) => pairing.mint(now_ms()),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response(),
    };
    Json(json!({"code": view.code, "expiresAt": view.expires_at})).into_response()
}

async fn sessions(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let displays = app
        .displays
        .lock()
        .map(|store| store.list().to_vec())
        .unwrap_or_default();
    Json(json!({"displays": displays})).into_response()
}

#[derive(Debug, Deserialize)]
struct NewSession {
    #[serde(rename = "displayId")]
    display_id: Option<String>,
    label: Option<String>,
}

/// Rotate a paired display's token, or add one.
async fn new_session(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(query): Query<NewSession>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let mut store = match app.displays.lock() {
        Ok(store) => store,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response(),
    };
    if let Some(id) = query.display_id {
        let Some(token) = store.rotate(&id) else {
            return (StatusCode::NOT_FOUND, "No such display.").into_response();
        };
        let display = store.get(&id).cloned();
        return (StatusCode::CREATED, Json(json!({"display": display, "token": token}))).into_response();
    }
    let label = query.label.unwrap_or_else(|| "display".to_owned());
    match store.adopt(&label, None, None) {
        Ok((record, token)) => {
            (StatusCode::CREATED, Json(json!({"display": record, "token": token}))).into_response()
        }
        Err(refusal) => (StatusCode::BAD_REQUEST, refusal.0).into_response(),
    }
}

async fn displays(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let displays = app
        .displays
        .lock()
        .map(|store| store.list().to_vec())
        .unwrap_or_default();
    Json(json!({"displays": displays})).into_response()
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    keys: Option<String>,
    lang: Option<String>,
    display: Option<String>,
}

async fn ws(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Response {
    let loopback = is_loopback(peer.ip());
    let display = authenticated(&app, &headers).or_else(|| {
        if loopback {
            Some(query.display.clone().unwrap_or_else(|| crate::displays::DEFAULT_DISPLAY.to_owned()))
        } else {
            None
        }
    });
    let Some(display) = display else {
        return (StatusCode::UNAUTHORIZED, "This display is not paired with the host.").into_response();
    };
    let keys: Vec<String> = query
        .keys
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect();
    let lang = query.lang.filter(|lang| {
        lang.len() <= 64
            && lang
                .split('-')
                .next()
                .is_some_and(|part| (2..=8).contains(&part.len()))
    });
    let (id, receiver) = match app.streams.register(display, keys, lang) {
        Ok(stream) => stream,
        Err(refusal) => return (StatusCode::TOO_MANY_REQUESTS, refusal.message()).into_response(),
    };
    let cleanup = Unregister {
        app: app.clone(),
        id,
    };
    let stream = futures_util::stream::unfold((receiver, cleanup), |(mut receiver, cleanup)| async move {
        receiver
            .recv()
            .await
            .map(|text| (Ok::<_, std::convert::Infallible>(text), (receiver, cleanup)))
    });
    let mut response = Response::new(Body::from_stream(stream));
    let out = response.headers_mut();
    out.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    out.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    out.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    out.insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

/// The stream's slot is released when its body is dropped, which happens when
/// the phone goes away.
struct Unregister {
    app: Shared,
    id: u64,
}

impl Drop for Unregister {
    fn drop(&mut self) {
        self.app.streams.unregister(self.id);
    }
}

async fn sensors(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !is_loopback(peer.ip()) && authenticated(&app, &headers).is_none() {
        return (StatusCode::UNAUTHORIZED, "This display is not paired with the host.").into_response();
    }
    let sensors: Vec<Value> = app
        .registry
        .describe()
        .into_iter()
        .map(|descriptor| {
            json!({
                "semanticKey": descriptor.semantic_key,
                "unit": descriptor.unit,
            })
        })
        .collect();
    Json(json!({"sensors": sensors})).into_response()
}

/// Serve one request body, used by the player page for its own JSON.
pub async fn body_text(body: Body) -> Option<String> {
    let bytes = axum::body::to_bytes(body, 64 * 1024).await.ok()?;
    String::from_utf8(bytes.to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_header_and_an_origin_name_the_same_host() {
        assert_eq!(host_of("http://localhost:5227"), host_of("localhost:5227"));
        assert_eq!(host_of("https://127.0.0.1:5227"), "127.0.0.1");
        assert_eq!(host_of("[::1]:5227"), "[::1]");
        assert_ne!(host_of("http://evil.example"), host_of("localhost:5227"));
    }

    #[test]
    fn the_session_cookie_is_httponly_strict_and_secure_over_tls() {
        let plain = session_cookie("abc", false);
        assert!(plain.starts_with("yunari_session=abc; Max-Age=34560000; HttpOnly; SameSite=Strict; Path=/"));
        assert!(!plain.contains("Secure"));
        assert!(session_cookie("abc", true).ends_with("; Secure"));
    }

    #[test]
    fn a_token_is_read_from_the_cookie_before_the_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("a=b; yunari_session=cookie; d=e"));
        headers.insert(SESSION_HEADER, HeaderValue::from_static("header"));
        assert_eq!(request_token(&headers).as_deref(), Some("cookie"));
        let mut only_header = HeaderMap::new();
        only_header.insert(SESSION_HEADER, HeaderValue::from_static("header"));
        assert_eq!(request_token(&only_header).as_deref(), Some("header"));
        let mut empty = HeaderMap::new();
        empty.insert(header::COOKIE, HeaderValue::from_static("yunari_session=;"));
        assert_eq!(request_token(&empty), None);
    }
}
