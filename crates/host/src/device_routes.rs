//! The SignalRGB bridge's HTTP surface: the device descriptor, controls,
//! telemetry and lighting mode, plus the device assignments beside them.
//!
//! Every route here is loopback-only, like the editor's own: a LAN client has
//! no business driving this PC's LEDs or fans. No handler opens hardware; every
//! write reaches the one panel runner the manager holds.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use yunari_panels::signalrgb::{BridgeError, SignalRgbBridge};
use yunari_providers::devices::Assignment;

use crate::http::{Shared, is_loopback};

pub const LOCAL_ONLY: &str = "Device control is available on this PC only.";
pub const SETTINGS_LOCAL_ONLY: &str = "SignalRGB bridge settings are available on this PC only.";

fn loopback(peer: SocketAddr, message: &'static str) -> Option<Response> {
    if is_loopback(peer.ip()) {
        None
    } else {
        Some((StatusCode::FORBIDDEN, message).into_response())
    }
}

const NO_BRIDGE: &str = "The SignalRGB bridge is not running.";

fn bridge_or_missing(app: &Shared) -> Option<&SignalRgbBridge> {
    app.signalrgb.as_deref()
}

/// The descriptor, the assignments and the bridge's own state, in one answer.
pub async fn devices(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback(peer, LOCAL_ONLY) {
        return refusal;
    }
    let settings = app.config.settings_dir();
    let available: Vec<Value> = yunari_panels::families()
        .iter()
        .flat_map(|family| {
            yunari_panels::attached(family)
                .into_iter()
                .map(move |hint| {
                    json!({
                        "id": format!("{:04x}:{:04x}:{}", hint.vendor_id, hint.product_id, hint.serial),
                        "kind": family.id,
                        "name": hint.label,
                        "vendorId": hint.vendor_id,
                        "productId": hint.product_id,
                    })
                })
        })
        .collect();
    let assigned = Assignment::load(&settings).to_json();
    let configured = crate::signalrgb::Settings::load(&settings).port();
    let mut body = match &app.signalrgb {
        Some(bridge) => bridge.describe(),
        None => json!({
            "hub": { "name": "Yunari", "vendor": "Yunari" },
            "devices": [],
            "stats": { "received": 0, "malformed": 0, "dropped": 0 },
        }),
    };
    body["available"] = json!(available);
    body["assigned"] = assigned;
    body["bridge"] = json!({
        "listening": app.signalrgb.is_some(),
        "port": app.signalrgb.as_ref().map(|bridge| bridge.port()),
        "configuredPort": configured,
        "error": app.signalrgb_error,
    });
    Json(body).into_response()
}

/// Replaces the GPU/disk device assignment.
pub async fn assign(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(refusal) = loopback(peer, LOCAL_ONLY) {
        return refusal;
    }
    let assignment = Assignment::from_json(&body);
    if let Err(error) = assignment.save(&app.config.settings_dir()) {
        return (StatusCode::BAD_REQUEST, error).into_response();
    }
    Json(json!({ "ok": true, "assigned": assignment.to_json() })).into_response()
}

/// `POST /api/devices/{id}/controls` and `POST /api/devices/{id}/lighting-mode`.
pub async fn act(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(refusal) = loopback(peer, LOCAL_ONLY) {
        return refusal;
    }
    let Some(bridge) = bridge_or_missing(&app) else {
        return (StatusCode::NOT_FOUND, NO_BRIDGE).into_response();
    };
    let outcome = match action.as_str() {
        "controls" => bridge.controls(&id, &body),
        "lighting-mode" => bridge.lighting_mode(&id, &body),
        _ => return (StatusCode::NOT_FOUND, "No such device action.").into_response(),
    };
    match outcome {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => answer(error),
    }
}

/// `GET /api/devices/{id}/telemetry`.
pub async fn telemetry(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refusal) = loopback(peer, LOCAL_ONLY) {
        return refusal;
    }
    let Some(bridge) = bridge_or_missing(&app) else {
        return (StatusCode::NOT_FOUND, NO_BRIDGE).into_response();
    };
    match bridge.telemetry(&id) {
        Ok(value) => Json(value).into_response(),
        Err(error) => answer(error),
    }
}

fn answer(error: BridgeError) -> Response {
    match error {
        BridgeError::Request(message) => (StatusCode::BAD_REQUEST, message).into_response(),
        BridgeError::NotFound(message) => (StatusCode::NOT_FOUND, message).into_response(),
    }
}

/// `PUT /api/signalrgb-bridge`: the port the next start binds.
pub async fn port(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(refusal) = loopback(peer, SETTINGS_LOCAL_ONLY) {
        return refusal;
    }
    let mut settings = crate::signalrgb::Settings::load(&app.config.settings_dir());
    match settings.write_udp_port(body.get("udpPort").unwrap_or(&Value::Null)) {
        Ok(udp_port) => Json(json!({ "udpPort": udp_port })).into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}
