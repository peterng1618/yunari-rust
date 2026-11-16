//! Reading the library, and which theme each display holds.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::assignment::Assignment;
use crate::http::{Shared, authenticated, is_loopback};

fn readable(app: &Shared, peer: SocketAddr, headers: &HeaderMap) -> Option<Response> {
    if is_loopback(peer.ip()) || authenticated(app, headers).is_some() {
        None
    } else {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                "This display is not paired with the host.",
            )
                .into_response(),
        )
    }
}

fn loopback(peer: SocketAddr) -> Option<Response> {
    if is_loopback(peer.ip()) {
        None
    } else {
        Some(
            (
                StatusCode::FORBIDDEN,
                "This theme surface is local to this PC only.",
            )
                .into_response(),
        )
    }
}

pub async fn themes(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Some(refusal) = readable(&app, peer, &headers) {
        return refusal;
    }
    let themes: Vec<Value> = app
        .library
        .list()
        .into_iter()
        .map(|entry| {
            let mut value = json!({
                "id": entry.id,
                "name": entry.name,
                "origin": "local",
                "author": entry.author,
                "version": entry.version,
                "nodeCount": entry.node_count,
            });
            if !is_loopback(peer.ip()) {
                value = json!({"id": entry.id, "name": entry.name});
            }
            value
        })
        .collect();
    Json(json!({"themes": themes})).into_response()
}

pub async fn document(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(refusal) = readable(&app, peer, &headers) {
        return refusal;
    }
    match app.library.read(&id) {
        Ok(stored) => match serde_json::to_value(&stored.document) {
            Ok(value) => Json(value).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "The host could not answer that request.",
            )
                .into_response(),
        },
        Err(_) => (StatusCode::NOT_FOUND, "Theme not found.").into_response(),
    }
}

pub async fn asset(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    if let Some(refusal) = readable(&app, peer, &headers) {
        return refusal;
    }
    if !is_safe_asset_path(&path) {
        return (StatusCode::NOT_FOUND, "No such asset.").into_response();
    }
    let Ok(stored) = app.library.read(&id) else {
        return (StatusCode::NOT_FOUND, "Theme not found.").into_response();
    };
    let assets = app.library.assets(&id, &stored.document);
    let Some(bytes) = assets.get(&path) else {
        return (StatusCode::NOT_FOUND, "No such asset.").into_response();
    };
    let mut response = bytes.clone().into_response();
    let out = response.headers_mut();
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    out.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'"),
    );
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// A declared asset path, never one that climbs out of the theme directory.
pub fn is_safe_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub async fn active(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback(peer) {
        return refusal;
    }
    let assignment = match app.assignment.lock() {
        Ok(assignment) => assignment,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    Json(active_value(&app, &assignment)).into_response()
}

fn active_value(app: &Shared, assignment: &Assignment) -> Value {
    let displays: Vec<Value> = app
        .displays
        .lock()
        .map(|store| {
            store
                .list()
                .iter()
                .map(|record| {
                    json!({
                        "id": record.id,
                        "label": record.label,
                        "kind": record.kind,
                        "theme": assignment.get(&record.id),
                        "shape": record.shape,
                        "rotation": record.rotation,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let themes: Vec<Value> = app
        .library
        .list()
        .into_iter()
        .map(|entry| json!({"id": entry.id, "name": entry.name}))
        .collect();
    json!({
        "default": assignment.get(crate::displays::DEFAULT_DISPLAY),
        "displays": displays,
        "themes": themes,
    })
}

#[derive(Debug, Deserialize)]
pub struct ActiveWrite {
    pub display: Option<String>,
    pub id: Option<String>,
}

pub async fn set_active(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<ActiveWrite>,
) -> Response {
    if let Some(refusal) = loopback(peer) {
        return refusal;
    }
    let display = body
        .display
        .unwrap_or_else(|| crate::displays::DEFAULT_DISPLAY.to_owned());
    if let Some(id) = &body.id
        && app.library.read(id).is_err()
    {
        return (StatusCode::NOT_FOUND, "Theme not found.").into_response();
    }
    if display != crate::displays::DEFAULT_DISPLAY
        && app
            .displays
            .lock()
            .map(|store| store.get(&display).is_none())
            .unwrap_or(true)
    {
        return (StatusCode::NOT_FOUND, "No such display.").into_response();
    }
    let mut value = {
        let mut assignment = match app.assignment.lock() {
            Ok(assignment) => assignment,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.")
                    .into_response();
            }
        };
        assignment.set(&display, body.id.as_deref());
        active_value(&app, &assignment)
    };
    value["ok"] = Value::Bool(true);
    Json(value).into_response()
}

/// What a playing page polls: the revision it is rendering and the theme its
/// display holds now.
pub async fn published(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<PublishedQuery>,
) -> Response {
    let loopback = is_loopback(peer.ip());
    let display = authenticated(&app, &headers).or_else(|| {
        loopback.then(|| {
            query
                .display
                .clone()
                .unwrap_or_else(|| crate::displays::DEFAULT_DISPLAY.to_owned())
        })
    });
    if display.is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            "This display is not paired with the host.",
        )
            .into_response();
    }
    let display = display.unwrap_or_else(|| crate::displays::DEFAULT_DISPLAY.to_owned());
    let assignment = match app.assignment.lock() {
        Ok(assignment) => assignment,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.").into_response();
        }
    };
    let theme = assignment.resolve(&app.library, &display, None);
    let revision = theme
        .as_deref()
        .and_then(|id| app.library.read(id).ok())
        .map(|stored| stored.base)
        .unwrap_or_default();
    let record = app
        .displays
        .lock()
        .ok()
        .and_then(|store| store.get(&display).cloned());
    Json(json!({
        "id": app.identity.id,
        "revision": revision,
        "display": {
            "theme": theme,
            "shape": record.as_ref().and_then(|record| record.shape),
            "rotation": record.as_ref().and_then(|record| record.rotation),
        }
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct PublishedQuery {
    pub display: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_asset_path_never_climbs_out() {
        assert!(is_safe_asset_path("icons/power.svg"));
        assert!(!is_safe_asset_path("../secrets"));
        assert!(!is_safe_asset_path("icons/../../secrets"));
        assert!(!is_safe_asset_path("/absolute"));
        assert!(!is_safe_asset_path("a\\b"));
        assert!(!is_safe_asset_path(""));
    }
}
