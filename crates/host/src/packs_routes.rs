//! Installed widget packs, for this PC only.
//!
//! The folder is the state: a listing reads it, and an install validates a
//! package before it is written. Both are loopback surfaces, because a phone
//! has no use for the host's own authoring library.

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::{Shared, is_loopback};

pub const NO_PACKS: &str = "Widget packs are available on this PC only.";

pub fn directory(app: &Shared) -> PathBuf {
    yunari_packs::directory(&app.config.settings_dir())
}

/// Every pack this host has installed, with the units it offers.
pub async fn list(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if !is_loopback(peer.ip()) {
        return (StatusCode::FORBIDDEN, NO_PACKS).into_response();
    }
    let packs: Vec<Value> = yunari_packs::installed(&directory(&app))
        .into_iter()
        .map(|pack| {
            let units: Vec<Value> = pack
                .units
                .iter()
                .map(|unit| json!({"id": unit.id, "label": unit.label}))
                .collect();
            json!({
                "id": pack.id,
                "name": pack.name,
                "version": pack.version,
                "units": units,
            })
        })
        .collect();
    Json(json!({"packs": packs})).into_response()
}

#[derive(Deserialize)]
pub struct InstallRequest {
    pub path: String,
}

/// Installs a package file this PC holds. The bytes are checked before they
/// are copied, so a bad package never lands.
pub async fn install(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<InstallRequest>,
) -> Response {
    if !is_loopback(peer.ip()) {
        return (StatusCode::FORBIDDEN, NO_PACKS).into_response();
    }
    let path = request.path.trim();
    let bytes = match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() <= yunari_packs::MAX_ARCHIVE_BYTES as u64 => {
            match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
                }
            }
        }
        Ok(_) => {
            return (StatusCode::BAD_REQUEST, yunari_packs::package::TOO_LARGE).into_response();
        }
        Err(error) => {
            return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
        }
    };
    match yunari_packs::install(&directory(&app), &bytes) {
        Ok(pack) => Json(json!({
            "id": pack.id,
            "name": pack.name,
            "version": pack.version,
            "units": pack.units.iter().map(|unit| json!({"id": unit.id, "label": unit.label})).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}
