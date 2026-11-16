//! The panel routes: what this PC can drive, the controls that reach it, and
//! the preview a settings surface shows before an owner commits.
//!
//! Every one of them is loopback-only. A panel is a physical device on this
//! machine, and a paired phone has no business opening, driving or restarting
//! it.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use yunari_panels::manager::PanelManagerError;
use yunari_panels::{PanelCommand, PanelFamily, families};

use crate::displays::DisplayKind;
use crate::http::{Shared, is_loopback};

pub const LOCAL_ONLY: &str = "Panel controls are local to this PC only.";

fn loopback(peer: SocketAddr) -> Option<Response> {
    if is_loopback(peer.ip()) {
        None
    } else {
        Some((StatusCode::FORBIDDEN, LOCAL_ONLY).into_response())
    }
}

fn family_json(family: &PanelFamily) -> Value {
    json!({
        "id": family.id,
        "label": family.label,
        "vendorId": family.vendor_id,
        "productId": family.product_id,
        "width": family.geometry.width,
        "height": family.geometry.height,
        "commands": family.commands.iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
        "support": {
            "maker": family.support.maker,
            "hardwareRevision": family.support.hardware_revision,
            "supportedFirmware": family.support.supported_firmware,
            "updatedAt": family.support.updated_at,
        },
    })
}

/// The families this host can drive, and the panels of each that are attached
/// right now.
pub async fn panels(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = loopback(peer) {
        return refusal;
    }
    let attached: Vec<Value> = families()
        .iter()
        .flat_map(|family| {
            yunari_panels::attached(family)
                .into_iter()
                .map(move |hint| {
                    json!({
                        "vendorId": hint.vendor_id,
                        "productId": hint.product_id,
                        "serial": hint.serial,
                        "label": hint.label,
                        "family": family.id,
                        "deviceId": hint.device_id,
                    })
                })
        })
        .collect();
    let _ = &app;
    Json(json!({
        "families": families().iter().map(family_json).collect::<Vec<_>>(),
        "attached": attached,
    }))
    .into_response()
}

/// A command as the route received it; the device's driver validates the rest.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CommandBody {
    Brightness {
        level: f64,
    },
    Screen {
        on: bool,
    },
    PanelSettings {
        #[serde(rename = "startMode")]
        start_mode: f64,
        flip: bool,
        #[serde(rename = "sleepDelay")]
        sleep_delay: f64,
    },
    Restart,
    Status,
    Hello,
}

impl CommandBody {
    fn into_command(self) -> Result<PanelCommand, (StatusCode, String)> {
        let whole = |value: f64| -> Option<u8> {
            (value.fract() == 0.0 && (0.0..=255.0).contains(&value)).then_some(value as u8)
        };
        match self {
            Self::Brightness { level } => {
                let Some(level) = whole(level) else {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("The brightness must be a whole percent; received {level}."),
                    ));
                };
                if level > 100 {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("The brightness must be from 0 to 100; received {level}."),
                    ));
                }
                Ok(PanelCommand::Brightness(level))
            }
            Self::Screen { on } => Ok(PanelCommand::Screen(on)),
            Self::PanelSettings {
                start_mode,
                flip,
                sleep_delay,
            } => {
                let Some(start_mode) = whole(start_mode) else {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("The start mode must be 0, 1 or 2; received {start_mode}."),
                    ));
                };
                let Some(sleep_delay) = whole(sleep_delay) else {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!(
                            "The sleep delay must be a whole number from 0 to 10; received {sleep_delay}."
                        ),
                    ));
                };
                if start_mode > 2 {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("The start mode must be 0, 1 or 2; received {start_mode}."),
                    ));
                }
                if sleep_delay > 10 {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!(
                            "The sleep delay must be a whole number from 0 to 10; received {sleep_delay}."
                        ),
                    ));
                }
                Ok(PanelCommand::PanelSettings {
                    start_mode,
                    flip,
                    sleep_delay,
                })
            }
            Self::Restart => Ok(PanelCommand::Restart),
            Self::Status => Ok(PanelCommand::Status),
            Self::Hello => Ok(PanelCommand::Hello),
        }
    }
}

fn status_for(error: &PanelManagerError) -> StatusCode {
    match error {
        // A display that is not running, or a device with no control channel,
        // is a state conflict rather than a fault of this host.
        PanelManagerError::NotRunning(_) | PanelManagerError::NoCommands(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// One control, on the wire to the device or refused with the manager's words.
pub async fn command(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    Json(body): Json<CommandBody>,
) -> Response {
    if let Some(refusal) = loopback(peer) {
        return refusal;
    }
    let command = match body.into_command() {
        Ok(command) => command,
        Err((status, message)) => return (status, message).into_response(),
    };
    match app.manager.command(&id, command) {
        Ok(reply) => Json(json!({"ok": true, "reply": reply})).into_response(),
        Err(error) => (status_for(&error), error.to_string()).into_response(),
    }
}

/// A USB instance id Windows reports for a panel's parent device.
///
/// Deliberately strict: no quote, whitespace or separator outside the set a
/// Windows instance id actually uses, so nothing from a request can become
/// script text even before it is quoted.
const USB_INSTANCE_ID: &str =
    r"^USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}(&MI_[0-9A-F]{2})?\\[A-Za-z0-9_&+.-]{1,64}$";

/// Whether a string is a USB instance id this host will hand to Windows.
pub fn is_usb_instance_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix(r"USB\VID_") else {
        return false;
    };
    let Some(rest) = strip_hex(rest, 4) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix("&PID_") else {
        return false;
    };
    let Some(rest) = strip_hex(rest, 4) else {
        return false;
    };
    let rest = match rest.strip_prefix("&MI_") {
        Some(after) => match strip_hex(after, 2) {
            Some(after) => after,
            None => return false,
        },
        None => rest,
    };
    let Some(suffix) = rest.strip_prefix('\\') else {
        return false;
    };
    (1..=64).contains(&suffix.len())
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_&+.-".contains(&byte))
}

/// `text` after `count` hexadecimal digits, or `None` when they are not hex.
fn strip_hex(text: &str, count: usize) -> Option<&str> {
    let (digits, rest) = text.split_at_checked(count)?;
    digits
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit())
        .then_some(rest)
}

/// Restarts the USB node a wedged panel hangs off.
///
/// A panel whose endpoint stops consuming data is not absent and not held by
/// another program: only a USB restart clears it. The parent composite device
/// is the one that works — restarting the COM child leaves the wedge. Both the
/// lookup and `pnputil` need elevation, so the owner sees a prompt and can
/// decline; the host holds no standing privilege.
pub async fn reset_device(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refusal) = loopback(peer) {
        return refusal;
    }
    let instance = {
        let store = match app.displays.lock() {
            Ok(store) => store,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.")
                    .into_response();
            }
        };
        let Some(record) = store.get(&id) else {
            return (StatusCode::NOT_FOUND, "No such display.").into_response();
        };
        if record.kind != DisplayKind::Panel {
            return (StatusCode::BAD_REQUEST, "That display is not a panel.").into_response();
        }
        record
            .device
            .as_ref()
            .and_then(|device| device.device_id.clone())
    };
    let Some(instance) = instance else {
        return (
            StatusCode::BAD_REQUEST,
            "This panel has no USB device id to restart. Replug it instead.",
        )
            .into_response();
    };
    if !is_usb_instance_id(&instance) {
        return (
            StatusCode::BAD_REQUEST,
            "That is not a USB device this host will restart.",
        )
            .into_response();
    }
    match restart_usb_device(&instance) {
        Ok(message) => Json(json!({"ok": true, "detail": message})).into_response(),
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

/// `pnputil /restart-device <parent>` behind the owner's own prompt.
#[cfg(windows)]
fn restart_usb_device(instance: &str) -> Result<String, String> {
    // The instance id was validated above and the rule admits no quote, so it
    // is safe as a single-quoted PowerShell literal.
    let script = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $parent = (Get-PnpDeviceProperty -InstanceId '{instance}' -KeyName 'DEVPKEY_Device_Parent').Data\n\
         if ($parent -notmatch '{USB_INSTANCE_ID}') {{ throw 'The parent device is not a USB device.' }}\n\
         Start-Process -FilePath pnputil.exe -ArgumentList @('/restart-device', $parent) -Verb RunAs -Wait\n\
         'The USB device restarted.'"
    );
    let encoded: String = script.encode_utf16().fold(String::new(), |mut text, unit| {
        use std::fmt::Write;
        let _ = write!(text, "{unit:04X}");
        text
    });
    // PowerShell's own decoding wants base64 of the UTF-16LE bytes, not hex.
    let bytes: Vec<u8> = encoded
        .as_bytes()
        .chunks(4)
        .filter_map(|chunk| u16::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok())
        .flat_map(u16::to_le_bytes)
        .collect();
    let command = base64::engine::general_purpose::STANDARD;
    let payload = {
        use base64::Engine;
        command.encode(bytes)
    };
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &payload,
        ])
        .output()
        .map_err(|error| format!("The USB device could not be restarted: {error}."))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let first = detail.lines().next().unwrap_or_default().trim();
    Err(if first.is_empty() {
        format!(
            "The USB device could not be restarted (exit {}).",
            output.status.code().unwrap_or(-1)
        )
    } else {
        format!("The USB device could not be restarted: {first}")
    })
}

#[cfg(not(windows))]
fn restart_usb_device(_instance: &str) -> Result<String, String> {
    Err("Restarting a USB device is a Windows action.".to_owned())
}

#[derive(Debug, Deserialize)]
pub struct PreviewQuery {
    display: Option<String>,
}

/// One display's panel, painted with a theme, at its family's own geometry.
/// The device is never opened.
pub async fn preview(
    State(app): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(theme): Path<String>,
    Query(query): Query<PreviewQuery>,
) -> Response {
    if let Some(refusal) = loopback(peer) {
        return refusal;
    }
    let Some(display) = query.display else {
        return (StatusCode::BAD_REQUEST, "A preview needs a display.").into_response();
    };
    let (record, family) = {
        let store = match app.displays.lock() {
            Ok(store) => store,
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "The host is not ready.")
                    .into_response();
            }
        };
        let Some(record) = store.get(&display) else {
            return (StatusCode::NOT_FOUND, "No such display.").into_response();
        };
        if record.kind != DisplayKind::Panel {
            return (StatusCode::BAD_REQUEST, "That display is not a panel.").into_response();
        }
        let family = record
            .device
            .as_ref()
            .and_then(|device| yunari_panels::for_device(device.vendor_id, device.product_id));
        let Some(family) = family else {
            return (
                StatusCode::BAD_REQUEST,
                "No driver on this host claims that panel.",
            )
                .into_response();
        };
        (record.clone(), family)
    };
    let _ = family;
    if app.library.read(&theme).is_err() {
        return (StatusCode::NOT_FOUND, "No such theme.").into_response();
    }

    // The preview paints the requested theme without touching the assignment:
    // a preview is what an owner is considering, not what the panel shows.
    let manager_record = crate::panels::manager_record(&record);
    match app.panel_frames.preview_of(&theme, &manager_record) {
        Ok(png) => {
            let mut response = png.into_response();
            let headers = response.headers_mut();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            headers.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use axum::extract::{Path, Query, State};

    use super::*;

    fn app(name: &str) -> Shared {
        let dir = std::env::temp_dir().join(format!(
            "yunari-panel-routes-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_millisecond()
        ));
        std::fs::create_dir_all(&dir).ok();
        crate::server::app(crate::config::HostConfig::resolve(
            Some(dir),
            Some(0),
            false,
        ))
    }

    fn here() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 5_000))
    }

    fn phone() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::new(203, 0, 113, 7), 5_000))
    }

    async fn text(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let body = crate::http::body_text(response.into_body())
            .await
            .unwrap_or_default();
        (status, body)
    }

    #[tokio::test]
    async fn every_panel_route_refuses_a_phone() {
        let app = app("phone");
        let (status, body) = text(panels(State(app.clone()), ConnectInfo(phone())).await).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, LOCAL_ONLY);

        let (status, _) = text(
            command(
                State(app.clone()),
                ConnectInfo(phone()),
                Path("x".to_owned()),
                Json(CommandBody::Status),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = text(
            reset_device(
                State(app.clone()),
                ConnectInfo(phone()),
                Path("x".to_owned()),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = text(
            preview(
                State(app),
                ConnectInfo(phone()),
                Path("theme".to_owned()),
                Query(PreviewQuery { display: None }),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_preview_of_a_display_that_is_not_a_panel_is_refused() {
        let app = app("not-panel");
        let (record, _token) = app
            .displays
            .lock()
            .unwrap()
            .adopt("Living room", None, None)
            .unwrap();
        let (status, body) = text(
            preview(
                State(app),
                ConnectInfo(here()),
                Path("theme".to_owned()),
                Query(PreviewQuery {
                    display: Some(record.id),
                }),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, "That display is not a panel.");
    }

    #[tokio::test]
    async fn a_command_to_a_stopped_panel_is_a_conflict_with_the_managers_sentence() {
        let app = app("stopped");
        let record = app
            .displays
            .lock()
            .unwrap()
            .adopt_panel(
                "Kraken",
                yunari_panels::manager::PanelDeviceId {
                    vendor_id: 0x1e71,
                    product_id: 0x3008,
                    serial: "k".to_owned(),
                    device_id: None,
                },
            )
            .unwrap();
        let (status, body) = text(
            command(
                State(app),
                ConnectInfo(here()),
                Path(record.id.clone()),
                Json(CommandBody::Brightness { level: 50.0 }),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body,
            format!(
                "Panel display {} is not running; start it before sending a command.",
                record.id
            )
        );
    }

    #[test]
    fn only_a_usb_instance_id_reaches_windows() {
        assert!(is_usb_instance_id(
            r"USB\VID_1D6B&PID_0106&MI_00\6&E9576A&5&0000"
        ));
        assert!(is_usb_instance_id(
            r"USB\VID_1cbe&PID_a092\7&1B2C3D4E&0&0000"
        ));
        assert!(!is_usb_instance_id(r"USB\VID_1D6B&PID_0106"));
        assert!(!is_usb_instance_id(r"USB\VID_1D6BPID_0106\6&E9576A"));
        assert!(!is_usb_instance_id(
            r"USB\VID_1D6B&PID_0106\6&E9576A'; whoami"
        ));
        assert!(!is_usb_instance_id(r"USB\VID_1D6B&PID_0106\"));
        assert!(!is_usb_instance_id(r"PCI\VEN_8086"));
    }

    #[test]
    fn a_command_body_is_validated_before_it_reaches_a_device() {
        assert!(matches!(
            CommandBody::Brightness { level: 50.5 }.into_command(),
            Err((StatusCode::BAD_REQUEST, _))
        ));
        assert!(matches!(
            CommandBody::Brightness { level: 101.0 }.into_command(),
            Err((StatusCode::BAD_REQUEST, _))
        ));
        assert_eq!(
            CommandBody::Brightness { level: 50.0 }
                .into_command()
                .unwrap(),
            yunari_panels::PanelCommand::Brightness(50)
        );
        assert!(
            CommandBody::PanelSettings {
                start_mode: 3.0,
                flip: false,
                sleep_delay: 0.0
            }
            .into_command()
            .is_err()
        );
    }
}
