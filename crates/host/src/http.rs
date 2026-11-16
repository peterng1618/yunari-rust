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
use crate::panels::PanelFrames;

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
    pub registry: Arc<yunari_providers::registry::Registry>,
    pub assignment: Mutex<crate::assignment::Assignment>,
    pub manager: Arc<yunari_panels::manager::Manager>,
    pub panel_frames: Arc<PanelFrames>,
    pub store: crate::store::Store,
    /// The store account this host holds, and the sign-ins it started.
    pub account: crate::store_routes::Account,
    /// Provider credentials, sealed by the OS protector.
    pub credentials: crate::credentials::Credentials,
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
        .route("/api/pairing/sessions", get(sessions).post(new_session))
        .route("/api/displays/panels", get(crate::panels_routes::panels))
        .route("/api/displays", get(displays).post(add_display))
        .route(
            "/api/displays/{id}/panel/command",
            post(crate::panels_routes::command),
        )
        .route(
            "/api/displays/{id}/panel/reset-device",
            post(crate::panels_routes::reset_device),
        )
        .route(
            "/api/displays/{id}",
            axum::routing::patch(patch_display).delete(remove_display),
        )
        .route(
            "/api/displays/autostart",
            get(autostart_enabled).post(set_autostart),
        )
        .route("/api/hosting", get(hosting).put(set_hosting))
        .route("/ws", get(ws))
        .route("/api/sensors", get(sensors))
        .route("/api/themes", get(crate::themes_routes::themes))
        .route(
            "/api/themes/active",
            get(crate::themes_routes::active).put(crate::themes_routes::set_active),
        )
        .route(
            "/api/themes/{id}/document",
            get(crate::themes_routes::document),
        )
        .route(
            "/api/themes/{id}/panel-preview",
            get(crate::panels_routes::preview),
        )
        .route(
            "/api/themes/{id}/assets/{*path}",
            get(crate::themes_routes::asset),
        )
        .route("/api/published", get(crate::themes_routes::published))
        .route("/", get(root))
        .route("/player.js", get(player_script))
        .route("/favicon.svg", get(favicon))
        // A display loads the icon a weather reading names; the slug is
        // allow-listed, so no other path is ever served.
        .route("/api/weather/icon/{file}", get(weather_icon))
        .route("/api/media/artwork/{file}", get(media_artwork))
        .route("/api/game/artwork/{app_id}", get(game_artwork))
        // The store catalogue is read here, so no page talks to a third origin.
        .route("/api/store/themes", get(store_themes))
        .route("/api/store/auth/start", post(crate::store_routes::start))
        .route("/api/store/auth/return", get(crate::store_routes::complete))
        .route(
            "/api/store/auth/sign-out",
            post(crate::store_routes::sign_out),
        )
        .route("/api/store/session", get(crate::store_routes::session))
        .route("/api/store/install", post(crate::store_install::route))
        .route("/api/store/mine", get(crate::store_routes::mine))
        .route("/api/themes/submit", post(crate::submit::route))
        .route(
            "/api/store/items/{id}/{action}",
            post(crate::store_routes::transition),
        )
        .route("/api/mcp", post(crate::mcp::post))
        .route(
            "/api/widget-packs",
            get(crate::packs_routes::list).post(crate::packs_routes::install),
        )
        .route("/manifest.webmanifest", get(manifest))
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
        if let Some(origin) = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            && let Some(host) = headers
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
            && host_of(origin) != host_of(host)
        {
            let _ = &app;
            return (
                StatusCode::FORBIDDEN,
                "That request is from another origin.",
            )
                .into_response();
        }
    }
    next.run(request).await
}

fn host_of(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    let without_scheme = value
        .split_once("://")
        .map_or(value.as_str(), |(_, rest)| rest);
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
        IpAddr::V6(address) => {
            address.is_loopback() || address.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// The token a request proves, from the cookie first and then the header.
/// The query string is never accepted here; `/?session=` has its own exchange.
pub fn request_token(headers: &HeaderMap) -> Option<String> {
    if let Some(cookie) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
    {
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

pub(crate) fn authenticated(app: &App, headers: &HeaderMap) -> Option<String> {
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
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

async fn certificate(State(app): State<Shared>) -> Response {
    let Some(authority) = &app.authority else {
        return (
            StatusCode::NOT_FOUND,
            "This host serves no certificate of its own.",
        )
            .into_response();
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

async fn session(
    State(app): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<SessionBody>,
) -> Response {
    let Some(_display) = app
        .displays
        .lock()
        .ok()
        .and_then(|mut store| store.authenticate(&body.token))
    else {
        return (
            StatusCode::FORBIDDEN,
            "That session is not one this host issued.",
        )
            .into_response();
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
            None => {
                return (StatusCode::BAD_REQUEST, "A display is a rect or a circle.")
                    .into_response();
            }
        },
    };
    if let (Some(degrees), Some(shape)) = (body.rotation, shape)
        && !shape.accepts_rotation(degrees)
    {
        return (
            StatusCode::BAD_REQUEST,
            "That rotation is not one of this display's.",
        )
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
        let wait = app
            .pairing
            .lock()
            .map(|mut pairing| pairing.miss(&remote))
            .unwrap_or(0);
        if wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                u64::try_from(wait).unwrap_or(4_000),
            ))
            .await;
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
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    let mut store = match app.displays.lock() {
        Ok(store) => store,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    match pairing.claim(
        &code,
        now_ms(),
        &mut store,
        &body.label,
        shape,
        body.rotation,
    ) {
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
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    Json(json!({"code": view.code, "expiresAt": view.expires_at})).into_response()
}

async fn mint_code(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let view = match app.pairing.lock() {
        Ok(mut pairing) => pairing.mint(now_ms()),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    Json(json!({"code": view.code, "expiresAt": view.expires_at})).into_response()
}

async fn sessions(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let displays = app
        .displays
        .lock()
        .map(|store| {
            store
                .list()
                .iter()
                .map(|record| record.peer_json())
                .collect::<Vec<_>>()
        })
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
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    if let Some(id) = query.display_id {
        let Some(token) = store.rotate(&id) else {
            return (StatusCode::NOT_FOUND, "No such display.").into_response();
        };
        let display = store.get(&id).map(|record| record.peer_json());
        return (
            StatusCode::CREATED,
            Json(json!({"display": display, "token": token})),
        )
            .into_response();
    }
    let label = query.label.unwrap_or_else(|| "display".to_owned());
    match store.adopt(&label, None, None) {
        Ok((record, token)) => (
            StatusCode::CREATED,
            Json(json!({"display": record.peer_json(), "token": token})),
        )
            .into_response(),
        Err(refusal) => (StatusCode::BAD_REQUEST, refusal.0).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct NewDisplay {
    label: String,
    /// `local` for a monitor, `panel` for a physical panel.
    kind: Option<String>,
    monitor: Option<Monitor>,
    device: Option<yunari_panels::manager::PanelDeviceId>,
    shape: Option<String>,
    rotation: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct Monitor {
    #[serde(rename = "deviceName")]
    device_name: String,
}

fn parse_presentation(
    shape: Option<&str>,
    rotation: Option<u16>,
) -> Result<Option<DisplayShape>, (StatusCode, &'static str)> {
    let shape = match shape {
        None => None,
        Some(name) => Some(
            DisplayShape::parse(name)
                .ok_or((StatusCode::BAD_REQUEST, "A display is a rect or a circle."))?,
        ),
    };
    if let (Some(degrees), Some(shape)) = (rotation, shape)
        && !shape.accepts_rotation(degrees)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "That rotation is not one of this display's.",
        ));
    }
    Ok(shape)
}

async fn add_display(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<NewDisplay>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    if body.kind.as_deref() == Some("panel") {
        return add_panel_display(&app, body).await;
    }
    let shape = match parse_presentation(body.shape.as_deref(), body.rotation) {
        Ok(shape) => shape,
        Err((status, message)) => return (status, message).into_response(),
    };
    let Some(monitor) = body.monitor else {
        return (StatusCode::BAD_REQUEST, "A local display needs a monitor.").into_response();
    };
    let mut store = match app.displays.lock() {
        Ok(store) => store,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    let uuid = monitor.device_name;
    if store.list().iter().any(|record| {
        record.kind == crate::displays::DisplayKind::Local
            && record.monitor.as_deref() == Some(uuid.as_str())
    }) {
        return (
            StatusCode::CONFLICT,
            "A local display already holds that monitor.",
        )
            .into_response();
    }
    match store.adopt_local(&body.label, Some(uuid), shape, body.rotation) {
        Ok(record) => (
            StatusCode::CREATED,
            Json(json!({"display": record.peer_json()})),
        )
            .into_response(),
        Err(refusal) => (StatusCode::BAD_REQUEST, refusal.0).into_response(),
    }
}

/// A panel record, and its runner started on the spot.
///
/// The attachment is checked here rather than trusted: a record naming a device
/// no driver claims would be a display that never shows anything.
pub(crate) async fn add_panel_display(app: &Shared, body: NewDisplay) -> Response {
    let Some(device) = body.device else {
        return (StatusCode::BAD_REQUEST, "A panel display needs a device.").into_response();
    };
    if yunari_panels::for_device(device.vendor_id, device.product_id).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "No driver on this host claims {:04x}:{:04x}.",
                device.vendor_id, device.product_id
            ),
        )
            .into_response();
    }
    let record = {
        let mut store = match app.displays.lock() {
            Ok(store) => store,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.")
                    .into_response();
            }
        };
        if store
            .list()
            .iter()
            .any(|record| record.device.as_ref() == Some(&device))
        {
            return (
                StatusCode::CONFLICT,
                "A panel display already holds that device.",
            )
                .into_response();
        }
        match store.adopt_panel(&body.label, device) {
            Ok(record) => record,
            Err(refusal) => return (StatusCode::BAD_REQUEST, refusal.0).into_response(),
        }
    };
    // The runner is attempted now so the answer reports the panel's state; a
    // failure is not the record's: the retry pass opens it when it appears.
    // Opening a panel is a USB conversation, not a request: it runs off the
    // async runtime so one slow device cannot hold another display's page.
    let manager = Arc::clone(&app.manager);
    let manager_record = crate::panels::manager_record(&record);
    let opened = tokio::task::spawn_blocking(move || manager.start(&manager_record))
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    let mut response = json!({
        "display": record.peer_json(),
        "panel": { "status": app.manager.status(&record.id) },
    });
    if let Err(message) = opened {
        response["panel"]["reason"] = Value::String(message);
    }
    (StatusCode::CREATED, Json(response)).into_response()
}

#[derive(Debug, Deserialize)]
struct DisplayPatch {
    label: Option<String>,
    shape: Option<String>,
    rotation: Option<u16>,
}

async fn patch_display(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<DisplayPatch>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let shape = match parse_presentation(body.shape.as_deref(), body.rotation) {
        Ok(shape) => shape,
        Err((status, message)) => return (status, message).into_response(),
    };
    let mut store = match app.displays.lock() {
        Ok(store) => store,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    match store.patch(&id, body.label.as_deref(), shape, body.rotation) {
        Ok(record) => Json(json!({"display": record.peer_json()})).into_response(),
        Err(refusal) => (StatusCode::NOT_FOUND, refusal.0).into_response(),
    }
}

async fn remove_display(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    if id == crate::displays::DEFAULT_DISPLAY {
        return (StatusCode::CONFLICT, "The default is not a paired client.").into_response();
    }
    let removed = app
        .displays
        .lock()
        .map(|mut store| store.remove(&id))
        .unwrap_or(false);
    if removed {
        Json(json!({"ok": true})).into_response()
    } else {
        (StatusCode::NOT_FOUND, "No such display.").into_response()
    }
}

async fn autostart_enabled(ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    Json(json!({"enabled": crate::autostart::enabled()})).into_response()
}

#[derive(Debug, Deserialize)]
struct AutostartWrite {
    enabled: bool,
}

async fn set_autostart(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<AutostartWrite>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    match crate::autostart::set(body.enabled) {
        Ok(message) => Json(json!({"enabled": crate::autostart::enabled(), "message": message}))
            .into_response(),
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

async fn hosting(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let hosting = crate::hosting::Hosting::load(&app.config.settings_dir());
    Json(json!({"lan": hosting.lan})).into_response()
}

#[derive(Debug, Deserialize)]
struct HostingWrite {
    lan: bool,
}

async fn set_hosting(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<HostingWrite>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let hosting = crate::hosting::Hosting { lan: body.lan };
    hosting.save(&app.config.settings_dir());
    Json(json!({"lan": hosting.lan, "restartRequired": true})).into_response()
}

async fn displays(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback_only(peer) {
        return refusal;
    }
    let displays = app
        .displays
        .lock()
        .map(|store| {
            store
                .list()
                .iter()
                .map(|record| record.peer_json())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Json(json!({"displays": displays})).into_response()
}

async fn root(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !is_loopback(peer.ip()) && authenticated(&app, &headers).is_none() {
        let mut response = (StatusCode::FOUND, "").into_response();
        response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_static("/pair?reason=unpaired"),
        );
        return response;
    }
    crate::pages::player_page()
}

async fn player_script() -> Response {
    crate::pages::player_script()
}

async fn favicon() -> Response {
    crate::pages::favicon()
}

async fn weather_icon(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    crate::pages::weather_icon(&file)
}

/// Track artwork, read through the same credential as theme asset bytes; this
/// PC needs none.
async fn media_artwork(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::extract::Path(file): axum::extract::Path<String>,
) -> Response {
    if !is_loopback(peer.ip()) && authenticated(&app, &headers).is_none() {
        return (
            StatusCode::FORBIDDEN,
            "This display is not paired with the host.",
        )
            .into_response();
    }
    crate::pages::media_artwork(&app.config.settings_dir().join("media-artwork"), &file)
}

/// The store catalogue, for this PC only: a display reads it through the host,
/// which is the one place the configured origin is known.
async fn store_themes(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if !is_loopback(peer.ip()) {
        return (
            StatusCode::FORBIDDEN,
            "The theme store is available on this PC only.",
        )
            .into_response();
    }
    match app.store.themes(now_ms()) {
        Ok(catalogue) => Json(catalogue).into_response(),
        Err(message) => (StatusCode::SERVICE_UNAVAILABLE, message).into_response(),
    }
}

/// The running game's cover, read through the same credential as track artwork.
async fn game_artwork(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::extract::Path(app_id): axum::extract::Path<String>,
) -> Response {
    if !is_loopback(peer.ip()) && authenticated(&app, &headers).is_none() {
        return (
            StatusCode::FORBIDDEN,
            "This display is not paired with the host.",
        )
            .into_response();
    }
    crate::pages::game_artwork(&app_id)
}

async fn manifest() -> Response {
    crate::pages::manifest()
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
            Some(
                query
                    .display
                    .clone()
                    .unwrap_or_else(|| crate::displays::DEFAULT_DISPLAY.to_owned()),
            )
        } else {
            None
        }
    });
    let Some(display) = display else {
        return (
            StatusCode::UNAUTHORIZED,
            "This display is not paired with the host.",
        )
            .into_response();
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
    let stream =
        futures_util::stream::unfold((receiver, cleanup), |(mut receiver, cleanup)| async move {
            receiver
                .recv()
                .await
                .map(|text| (Ok::<_, std::convert::Infallible>(text), (receiver, cleanup)))
        });
    let mut response = Response::new(Body::from_stream(stream));
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
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
        return (
            StatusCode::UNAUTHORIZED,
            "This display is not paired with the host.",
        )
            .into_response();
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
        assert!(plain.starts_with(
            "yunari_session=abc; Max-Age=34560000; HttpOnly; SameSite=Strict; Path=/"
        ));
        assert!(!plain.contains("Secure"));
        assert!(session_cookie("abc", true).ends_with("; Secure"));
    }

    fn app(name: &str) -> Shared {
        let app_dir = std::env::temp_dir().join(format!(
            "yunari-http-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        crate::server::app(HostConfig::resolve(Some(app_dir), Some(0), false))
    }

    async fn ask(app: Shared, uri: &str, peer: SocketAddr) -> (StatusCode, String) {
        request(app, axum::http::Method::GET, uri, peer).await
    }

    async fn request(
        app: Shared,
        method: axum::http::Method,
        uri: &str,
        peer: SocketAddr,
    ) -> (StatusCode, String) {
        use tower::ServiceExt;
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost:5227")
            .body(Body::empty())
            .unwrap();
        let (mut parts, body) = request.into_parts();
        parts.extensions.insert(ConnectInfo(peer));
        let response = router(app)
            .oneshot(Request::from_parts(parts, body))
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn the_store_session_and_the_authors_items_answer_signed_out_and_local_only() {
        let app = app("store-session");
        let local = SocketAddr::from(([127, 0, 0, 1], 50_000));
        let (status, body) = ask(app.clone(), "/api/store/session", local).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"signedIn":false}"#);

        let (status, body) = ask(app.clone(), "/api/store/mine", local).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains(crate::store_routes::SIGNED_OUT_MESSAGE));

        let phone = SocketAddr::from(([203, 0, 113, 9], 50_000));
        let (status, body) = ask(app, "/api/store/session", phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, crate::store_routes::NOT_LOOPBACK_MESSAGE);
    }

    fn app_with_store(name: &str) -> Shared {
        let app_dir = std::env::temp_dir().join(format!(
            "yunari-http-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        let settings = app_dir.join("settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("store.json"),
            r#"{"origin":"https://store.example"}"#,
        )
        .unwrap();
        crate::server::app(HostConfig::resolve(Some(app_dir), Some(0), false))
    }

    #[tokio::test]
    async fn a_sign_in_starts_at_the_configured_worker_for_a_loopback_return() {
        let local = SocketAddr::from(([127, 0, 0, 1], 50_000));
        // No store.json: nothing to sign in to.
        let (status, body) = request(
            app("store-start"),
            axum::http::Method::POST,
            "/api/store/auth/start",
            local,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, crate::store_routes::NO_STORE_MESSAGE);

        let (status, body) = request(
            app_with_store("store-start-origin"),
            axum::http::Method::POST,
            "/api/store/auth/start",
            local,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.contains("https://store.example/auth/start?provider=github&host=http%3A%2F%2Flocalhost%3A5227"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn a_store_install_is_loopback_only_and_needs_the_stores_facts() {
        let app = app("store-install");
        let local = SocketAddr::from(([127, 0, 0, 1], 50_000));
        let phone = SocketAddr::from(([203, 0, 113, 9], 50_000));
        let (status, _) = request(
            app.clone(),
            axum::http::Method::POST,
            "/api/store/install",
            phone,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) =
            request(app, axum::http::Method::POST, "/api/store/install", local).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, crate::store_install::NOT_JSON_MESSAGE);
    }

    #[tokio::test]
    async fn a_submission_is_local_and_needs_a_sign_in() {
        let app = app_with_store("submit");
        let local = SocketAddr::from(([127, 0, 0, 1], 50_000));
        let phone = SocketAddr::from(([203, 0, 113, 9], 50_000));
        let (status, _) = request(
            app.clone(),
            axum::http::Method::POST,
            "/api/themes/submit",
            phone,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) =
            request(app, axum::http::Method::POST, "/api/themes/submit", local).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains(crate::submit::NO_SESSION_MESSAGE));
    }

    #[tokio::test]
    async fn a_sign_in_return_that_was_not_started_here_is_refused() {
        let app = app_with_store("store-return");
        let local = SocketAddr::from(([127, 0, 0, 1], 50_000));
        let (status, body) = ask(
            app,
            "/api/store/auth/return?state=made-up&token=one-time",
            local,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("This sign-in has expired, or was not started here."));
    }

    #[tokio::test]
    async fn a_weather_icon_is_served_only_for_a_vendored_slug() {
        let app = app("weather-icons");
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        // A paired display loads the picture itself, so a phone may read it.
        let (status, body) = ask(app.clone(), "/api/weather/icon/rain.svg", phone).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<svg"));
        // A slug this build did not vendor, and a path that is not a slug.
        let (status, _) = ask(app.clone(), "/api/weather/icon/evil.svg", phone).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = ask(app.clone(), "/api/weather/icon/rain.png", phone).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = ask(app, "/api/weather/icon/..%2Fsecrets.svg", phone).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn media_artwork_is_served_to_this_pc_and_only_a_paired_phone() {
        const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
        let app = app("media-artwork");
        let cache = yunari_providers::media::artwork::Artwork::new(
            app.config.settings_dir().join("media-artwork"),
        );
        let url = cache.put(PNG, "png").expect("a picture");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let (status, _) = ask(app.clone(), &url, here).await;
        assert_eq!(status, StatusCode::OK);
        // A display reads it through its credential.
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        let (status, _) = ask(app.clone(), &url, phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // The name is the entire allow-list.
        let (status, _) = ask(app.clone(), "/api/media/artwork/evil.png", here).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = ask(app, "/api/media/artwork/..%2Fsecrets.png", here).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn game_artwork_is_only_a_steam_cache_name() {
        let app = app("game-artwork");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        // An id that is not an app id is a 404, never a filesystem probe.
        let (status, _) = ask(app.clone(), "/api/game/artwork/..%2Fsecrets", here).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // A phone reads it through its credential first.
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        let (status, _) = ask(app, "/api/game/artwork/730", phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    async fn ask_post(
        app: Shared,
        uri: &str,
        peer: SocketAddr,
        body: &str,
    ) -> (StatusCode, String) {
        use tower::ServiceExt;
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::HOST, "localhost:5227")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        let (mut parts, body) = request.into_parts();
        parts.extensions.insert(ConnectInfo(peer));
        let response = router(app)
            .oneshot(Request::from_parts(parts, body))
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// One theme in the app's library, saved through the same path the editor
    /// uses.
    fn save_fixture_theme(app: &Shared, id: &str) {
        let document = yunari_model::document::ThemeDocument {
            schema_version: yunari_model::document::SCHEMA_VERSION.to_owned(),
            id: id.to_owned(),
            metadata: yunari_model::document::Metadata::default(),
            artboard: yunari_model::document::Artboard {
                width: 100.0,
                height: 100.0,
                content_fit: yunari_model::document::ContentFit::Contain,
                shape: yunari_model::document::ArtboardShape::Rect,
            },
            background: yunari_model::document::Background::default(),
            palette: std::collections::BTreeMap::new(),
            type_tokens: std::collections::BTreeMap::new(),
            assets: Vec::new(),
            nodes: Vec::new(),
            bindings: std::collections::BTreeMap::new(),
            editor_metadata: std::collections::BTreeMap::new(),
        };
        app.library
            .save(&yunari_library::SaveRequest {
                id,
                document: &document,
                assets: &std::collections::BTreeMap::new(),
                base: None,
                overwrite: true,
            })
            .expect("the fixture theme saves");
    }

    #[tokio::test]
    async fn mcp_answers_this_pc_with_the_librarys_themes_and_refuses_a_phone() {
        let app = app("mcp");
        save_fixture_theme(&app, "fixture");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        let list =
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"themes_list"}}"#;
        let (status, body) = ask_post(app.clone(), "/api/mcp", here, list).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("fixture"), "{body}");
        // A phone does not get the host's own files.
        let (status, _) = ask_post(app.clone(), "/api/mcp", phone, list).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // An unknown method is a JSON-RPC error, not a page.
        let unknown = r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#;
        let (status, body) = ask_post(app.clone(), "/api/mcp", here, unknown).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("-32601"), "{body}");
        // A request that is not JSON-RPC is a parse error.
        let (status, body) = ask_post(app, "/api/mcp", here, "not json").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("-32700"), "{body}");
    }

    /// One widget package on disk, so the install route is driven with a real
    /// file rather than bytes in a test's imagination.
    fn widget_package(dir: &std::path::Path) -> std::path::PathBuf {
        use std::io::Write;

        let manifest = serde_json::json!({
            "format": "yunari-widget-package",
            "version": 1,
            "widget": "widget.json",
            "units": [{"id": "card", "label": "Card", "objectId": "card"}],
        });
        let widget = serde_json::json!({
            "schemaVersion": "3.0.0",
            "id": "cards",
            "metadata": {"name": "Cards"},
            "scene": {"objects": [{"type": "Rect", "id": "card"}]},
        });
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&manifest).unwrap())
            .unwrap();
        writer.start_file("widget.json", options).unwrap();
        writer
            .write_all(&serde_json::to_vec(&widget).unwrap())
            .unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let path = dir.join("cards.yunari-widget-package");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[tokio::test]
    async fn widget_packs_are_installed_and_listed_for_this_pc_only() {
        let app = app("widget-packs");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        let path = widget_package(&app.config.settings_dir());
        let body = format!(
            r#"{{"path":{}}}"#,
            serde_json::json!(path.to_string_lossy())
        );
        // A phone neither lists the host's packs nor installs over them.
        let (status, _) = ask(app.clone(), "/api/widget-packs", phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = ask_post(app.clone(), "/api/widget-packs", phone, &body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, answer) = ask_post(app.clone(), "/api/widget-packs", here, &body).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert!(answer.contains("\"card\""), "{answer}");
        let (status, listing) = ask(app, "/api/widget-packs", here).await;
        assert_eq!(status, StatusCode::OK);
        assert!(listing.contains("Cards"), "{listing}");
        assert!(listing.contains("\"card\""), "{listing}");
    }

    #[tokio::test]
    async fn a_fresh_host_starts_with_the_committed_card_pack() {
        let app = app("starter-pack");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let (status, listing) = ask(app, "/api/widget-packs", here).await;
        assert_eq!(status, StatusCode::OK);
        assert!(listing.contains("Starter cards"), "{listing}");
        assert!(listing.contains("CPU card"), "{listing}");
    }

    #[tokio::test]
    async fn a_package_the_reader_refuses_is_a_400_with_its_sentence() {
        let app = app("widget-packs-bad");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let path = app
            .config
            .settings_dir()
            .join("broken.yunari-widget-package");
        std::fs::create_dir_all(app.config.settings_dir()).unwrap();
        std::fs::write(&path, b"not a package").unwrap();
        let body = format!(
            r#"{{"path":{}}}"#,
            serde_json::json!(path.to_string_lossy())
        );
        let (status, answer) = ask_post(app, "/api/widget-packs", here, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer, yunari_packs::package::UNREADABLE);
    }

    #[tokio::test]
    async fn the_store_catalogue_is_for_this_pc_only() {
        let app = app("store");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        let (status, _) = ask(app.clone(), "/api/store/themes", phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Nothing configured is a sentence, not an empty catalogue.
        let (status, body) = ask(app, "/api/store/themes", here).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, crate::store::NO_STORE_MESSAGE);
    }

    #[tokio::test]
    async fn a_panel_display_for_an_unknown_family_is_refused() {
        let app = app("panel-unknown");
        let response = add_panel_display(
            &app,
            NewDisplay {
                label: "Odd panel".to_owned(),
                kind: Some("panel".to_owned()),
                monitor: None,
                device: Some(yunari_panels::manager::PanelDeviceId {
                    vendor_id: 0x1234,
                    product_id: 0x5678,
                    serial: String::new(),
                    device_id: None,
                }),
                shape: None,
                rotation: None,
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = body_text(response.into_body()).await.unwrap_or_default();
        assert_eq!(body, "No driver on this host claims 1234:5678.");
        // Nothing was stored: a record no driver claims would never show.
        assert!(app.displays.lock().unwrap().list().is_empty());
    }

    #[tokio::test]
    async fn a_phone_cannot_reach_the_local_surfaces() {
        let app = app("remote");
        let phone = SocketAddr::from(([203, 0, 113, 9], 50000));
        let (status, _) = ask(app.clone(), "/api/displays", phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = ask(app.clone(), "/api/pairing/code", phone).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) = ask(app.clone(), "/api/host", phone).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"id\""), "{body}");
        let (status, _) = ask(app, "/", phone).await;
        assert_eq!(status, StatusCode::FOUND);
    }

    #[tokio::test]
    async fn this_pc_reaches_its_own_surfaces() {
        let app = app("local");
        let here = SocketAddr::from(([127, 0, 0, 1], 50000));
        let (status, body) = ask(app.clone(), "/api/displays", here).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, _) = ask(app, "/", here).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn a_token_is_read_from_the_cookie_before_the_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=b; yunari_session=cookie; d=e"),
        );
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
