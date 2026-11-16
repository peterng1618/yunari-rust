//! The agent surface: MCP over one loopback POST.
//!
//! The tools answer from the surfaces this host already owns — the theme
//! library, the theme documents and the sensor catalogue — so an agent reads
//! the same data a display does, and nothing here duplicates a store or a
//! renderer.

use std::net::SocketAddr;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::http::{Shared, is_loopback};

/// The MCP revision this surface answers with.
const PROTOCOL_VERSION: &str = "2025-06-18";
pub const NO_AGENTS: &str = "The agent surface is available on this PC only.";
/// One request is a tool call, not a document upload.
const MAX_REQUEST_BYTES: usize = 64 * 1024;

fn tools() -> Value {
    let mut list = json!([
        {
            "name": "themes_list",
            "description": "List the themes in this host's library.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "themes_get",
            "description": "Read one theme's document by id.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"],
                "additionalProperties": false
            }
        },
        {
            "name": "sensors_list",
            "description": "List every semantic key this host can read, with its unit.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "displays_list",
            "description": "List the displays this host serves, without their credentials.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "monitors_list",
            "description": "List this PC's monitors with the uuid a local display holds.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "path_presets_list",
            "description": "List the path presets the Add pane offers, built-ins first.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "providers_list",
            "description": "List the sensor providers and why any of them is unavailable.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }
    ]);
    if let (Some(list), Value::Array(compose)) = (list.as_array_mut(), crate::mcp_compose::tools())
    {
        list.extend(compose);
    }
    list
}

pub async fn post(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    if !is_loopback(peer.ip()) {
        return (StatusCode::FORBIDDEN, NO_AGENTS).into_response();
    }
    if body.len() > MAX_REQUEST_BYTES {
        return rpc_error(Value::Null, -32600, "That request is too large.");
    }
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return rpc_error(Value::Null, -32700, "Parse error.");
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return rpc_error(id, -32600, "Invalid Request.");
    }
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return rpc_error(id, -32600, "Invalid Request.");
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    // A notification expects no answer at all.
    if method.starts_with("notifications/") {
        return StatusCode::ACCEPTED.into_response();
    }
    let answer = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "yunari-host", "version": env!("CARGO_PKG_VERSION")},
        })),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => call(&app, &params),
        _ => Err((-32601, "Method not found.")),
    };
    match answer {
        Ok(result) => Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response(),
        Err((code, message)) => rpc_error(id, code, message),
    }
}

/// One tool call, as MCP's text content.
fn call(app: &Shared, params: &Value) -> Result<Value, (i64, &'static str)> {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Err((-32602, "Invalid params."));
    };
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    if let Some(result) = crate::mcp_compose::call(app, name, &arguments) {
        return Ok(match result {
            Ok(value) => text(&value.to_string(), false),
            Err(message) => text(&message, true),
        });
    }
    let answer = match name {
        "themes_list" => {
            let themes: Vec<Value> = app
                .library
                .list()
                .into_iter()
                .map(|entry| {
                    json!({
                        "id": entry.id,
                        "name": entry.name,
                        "author": entry.author,
                        "version": entry.version,
                        "nodeCount": entry.node_count,
                    })
                })
                .collect();
            Ok(json!({"themes": themes}))
        }
        "themes_get" => {
            let Some(id) = arguments.get("id").and_then(Value::as_str) else {
                return Ok(text("A theme id is required.", true));
            };
            match app.library.read(id) {
                Ok(stored) => serde_json::to_value(&stored.document)
                    .map_err(|_| (-32603, "The host could not answer that request.")),
                Err(_) => Ok(text("Theme not found.", true)),
            }
        }
        "sensors_list" => {
            let sensors: Vec<Value> = app
                .registry
                .describe()
                .into_iter()
                .map(|descriptor| {
                    json!({"semanticKey": descriptor.semantic_key, "unit": descriptor.unit})
                })
                .collect();
            Ok(json!({"sensors": sensors}))
        }
        "displays_list" => {
            let displays: Vec<Value> = app
                .displays
                .lock()
                .map(|store| {
                    store
                        .list()
                        .iter()
                        .map(|record| record.peer_json())
                        .collect()
                })
                .unwrap_or_default();
            Ok(json!({"displays": displays}))
        }
        "monitors_list" => match crate::monitors::list() {
            Ok(monitors) => Ok(json!({
                "monitors": monitors.iter().map(crate::monitors::Monitor::json).collect::<Vec<_>>(),
            })),
            Err(reason) => Ok(text(&reason, true)),
        },
        "path_presets_list" => Ok(crate::path_presets::Store::at(app.config.settings_dir())
            .read()
            .json()),
        "providers_list" => {
            let providers: Vec<Value> = app
                .registry
                .status()
                .into_iter()
                .map(|(id, label, health)| {
                    let mut row = json!({"id": id, "label": label});
                    match health.reason() {
                        Some(reason) => row["reason"] = json!(reason),
                        None => row["ready"] = json!(true),
                    }
                    row
                })
                .collect();
            Ok(json!({"providers": providers}))
        }
        _ => Ok(text("No such tool.", true)),
    };
    answer.map(|value| {
        if value.get("content").is_some() {
            value
        } else {
            text(&value.to_string(), false)
        }
    })
}

/// MCP's one content shape here: the answer as text, plus whether it is an
/// error a caller should read rather than parse.
fn text(body: &str, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": body}],
        "isError": is_error,
    })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message},
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_surface_this_host_owns_is_a_declared_tool() {
        let declared = tools().to_string();
        for name in [
            "themes_list",
            "themes_get",
            "sensors_list",
            "displays_list",
            "monitors_list",
            "path_presets_list",
            "providers_list",
            "create_theme",
            "save_theme",
            "add_text",
            "add_shape",
            "import_asset",
            "list_objects",
        ] {
            assert!(declared.contains(name), "{name} is not declared");
        }
    }
}
