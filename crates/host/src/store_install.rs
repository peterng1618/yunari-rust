//! One catalogue item's Install action, end to end, for every kind this build
//! runs.
//!
//! The trust boundary is one place: the package's address is held to the
//! configured store's own host over https, no redirect is followed, the store's
//! declared size and SHA-256 are checked, and each kind's owner reads the
//! archive before anything is written. A package that fails any stage leaves
//! this machine exactly as it was.
//!
//! The editor posts the store's facts; it never sends bytes.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use yunari_store::client::{Method, Request, Transport, UPLOAD_TIMEOUT_MS};

use crate::http::Shared;
use crate::origins::{Origin, Origins};

pub const NOT_JSON_MESSAGE: &str = "That install request is not JSON.";
pub const NEEDS_FACTS_MESSAGE: &str =
    "An install needs packageUrl, catalogueId, kind, version, sha256 and bytes.";
pub const NO_PACKS_MESSAGE: &str = "This host keeps no widget packs.";
pub const NO_PLUGINS_MESSAGE: &str = "This host keeps no plugins.";

/// The package's own cap follows the theme reader's.
pub const MAX_PACKAGE_BYTES: u64 = yunari_library::package::MAX_ARCHIVE_BYTES as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Address,
    Download,
    Integrity,
    Package,
    Store,
}

impl Stage {
    pub fn status(self) -> u16 {
        match self {
            Stage::Download => 502,
            Stage::Store => 500,
            Stage::Address | Stage::Integrity | Stage::Package => 400,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub stage: Stage,
    pub message: String,
}

impl Refusal {
    fn at(stage: Stage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    DashboardTheme,
    WidgetPack,
    EditorTheme,
    PathPresetPack,
    Plugin,
}

impl Kind {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "dashboard-theme" => Some(Self::DashboardTheme),
            "widget-pack" => Some(Self::WidgetPack),
            "editor-theme" => Some(Self::EditorTheme),
            "path-preset-pack" => Some(Self::PathPresetPack),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    pub package_url: String,
    pub catalogue_id: String,
    pub kind: Kind,
    pub version: String,
    pub sha256: String,
    pub bytes: u64,
}

/// The route's body, or nothing when it is not the request this dispatcher
/// accepts.
pub fn parse_request(value: &Value) -> Option<InstallRequest> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let catalogue_id = text("catalogueId")?;
    if !crate::store_routes::is_catalogue_id(&catalogue_id) {
        return None;
    }
    let sha256 = text("sha256")?;
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let bytes = value.get("bytes")?.as_u64().filter(|bytes| *bytes > 0)?;
    Some(InstallRequest {
        package_url: text("packageUrl")?,
        catalogue_id,
        kind: Kind::parse(&text("kind")?)?,
        version: text("version")?,
        sha256,
        bytes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
}

/// One package fetch. A seam so a test installs without a network.
pub type Download = Arc<dyn Fn(&str) -> Result<Vec<u8>, Refusal> + Send + Sync>;

/// The package the configured store serves, held to its address, its size and
/// its digest.
pub fn download(
    origin: Option<&str>,
    url: &str,
    transport: &Transport,
) -> Result<Vec<u8>, Refusal> {
    let Some(origin) = origin else {
        return Err(Refusal::at(Stage::Address, "No theme store is configured."));
    };
    let Some(target) = host_of(url) else {
        return Err(Refusal::at(
            Stage::Address,
            "That is not a URL a catalogue can produce.",
        ));
    };
    if !url.to_ascii_lowercase().starts_with("https://") {
        return Err(Refusal::at(
            Stage::Address,
            "A catalogue package must be downloaded over https.",
        ));
    }
    if target != host_of(origin).unwrap_or_default() {
        return Err(Refusal::at(
            Stage::Address,
            "That host cannot serve a catalogue package.",
        ));
    }
    let reply = (transport)(&Request {
        method: Method::Get,
        url: url.to_owned(),
        headers: Vec::new(),
        body: None,
        timeout_ms: UPLOAD_TIMEOUT_MS * 2,
    })
    .map_err(|message| Refusal::at(Stage::Download, message))?;
    if (300..400).contains(&reply.status) {
        return Err(Refusal::at(
            Stage::Download,
            "The catalogue redirected a package the store serves directly.",
        ));
    }
    if !(200..300).contains(&reply.status) {
        return Err(Refusal::at(
            Stage::Download,
            format!("The store answered HTTP {}.", reply.status),
        ));
    }
    Ok(reply.body)
}

/// The host a URL names, without its port, lowercased.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Installs one catalogue item, or refuses at the stage that stopped it.
pub fn install(
    app: &Shared,
    request: &InstallRequest,
    download: &Download,
) -> Result<Installed, Refusal> {
    let bytes = download(&request.package_url)?;
    if bytes.len() as u64 != request.bytes || yunari_library::sha256_hex(&bytes) != request.sha256 {
        return Err(Refusal::at(
            Stage::Integrity,
            "The downloaded package does not match the size and SHA-256 the store declared.",
        ));
    }
    match request.kind {
        Kind::DashboardTheme => dashboard_theme(app, request, &bytes),
        Kind::WidgetPack => widget_pack(app, &bytes),
        Kind::EditorTheme => editor_theme(app, request, &bytes),
        Kind::PathPresetPack => path_presets(app, &bytes),
        Kind::Plugin => Err(Refusal::at(Stage::Store, NO_PLUGINS_MESSAGE)),
    }
}

fn dashboard_theme(
    app: &Shared,
    request: &InstallRequest,
    bytes: &[u8],
) -> Result<Installed, Refusal> {
    let package = yunari_library::package::read_package(bytes)
        .map_err(|message| Refusal::at(Stage::Package, message))?;
    let origins = Origins::at(app.config.settings_dir());
    let existing = origins.theme_of(&request.catalogue_id);
    let (id, fresh) = match existing {
        Some(id) => {
            let mut document = package.document.clone();
            document.id.clone_from(&id);
            app.library
                .save(&yunari_library::SaveRequest {
                    id: &id,
                    document: &document,
                    assets: &package.assets,
                    credits: package.credits.as_ref(),
                    base: None,
                    overwrite: true,
                })
                .map_err(store_refusal)?;
            (id, false)
        }
        None => {
            let saved = yunari_library::package::import_into(&app.library, &package)
                .map_err(store_refusal)?;
            (saved.entry.id, true)
        }
    };
    let origin = Origin {
        catalogue_id: request.catalogue_id.clone(),
        version: request.version.clone(),
        installed_at: jiff::Timestamp::now().to_string(),
    };
    if let Err(message) = origins.write(&id, &origin) {
        // A refused origin write undoes a fresh import rather than leaving a
        // copy the catalogue cannot see; an update keeps the copy the person
        // already had.
        if fresh {
            let _ = app.library.remove(&id);
        }
        return Err(Refusal::at(Stage::Store, message));
    }
    Ok(Installed { id })
}

fn widget_pack(app: &Shared, bytes: &[u8]) -> Result<Installed, Refusal> {
    let directory = yunari_packs::directory(&app.config.settings_dir());
    let installed = yunari_packs::install(&directory, bytes)
        .map_err(|message| Refusal::at(Stage::Package, message))?;
    Ok(Installed { id: installed.id })
}

fn path_presets(app: &Shared, bytes: &[u8]) -> Result<Installed, Refusal> {
    let (id, _) = crate::path_presets::Store::at(app.config.settings_dir())
        .add_pack(bytes)
        .map_err(|message| Refusal::at(Stage::Package, message))?;
    Ok(Installed { id })
}

fn editor_theme(
    app: &Shared,
    request: &InstallRequest,
    bytes: &[u8],
) -> Result<Installed, Refusal> {
    // The reader owns the archive's shape; this host owns the file.
    yunari_library::package::read_package(bytes)
        .map_err(|message| Refusal::at(Stage::Package, message))?;
    let directory = app.config.settings_dir().join("editor-themes");
    std::fs::create_dir_all(&directory).map_err(store_io)?;
    std::fs::write(
        directory.join(format!("{}.yunari-theme", request.catalogue_id)),
        bytes,
    )
    .map_err(store_io)?;
    Ok(Installed {
        id: request.catalogue_id.clone(),
    })
}

fn store_refusal(message: yunari_library::LibraryError) -> Refusal {
    Refusal::at(Stage::Store, message.to_string())
}

fn store_io(message: std::io::Error) -> Refusal {
    Refusal::at(Stage::Store, message.to_string())
}

/// The route's own answer: an install pressed from the loopback page.
pub async fn route(
    axum::extract::State(app): axum::extract::State<Shared>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    if !crate::http::is_loopback(peer.ip()) {
        return (StatusCode::FORBIDDEN, "Store installs are loopback only.").into_response();
    }
    let value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, NOT_JSON_MESSAGE).into_response(),
    };
    let Some(request) = parse_request(&value) else {
        return (StatusCode::BAD_REQUEST, NEEDS_FACTS_MESSAGE).into_response();
    };
    let origin = app.account.origin.clone();
    let transport = Arc::clone(&app.account.transport);
    let download: Download = Arc::new(move |url| download(origin.as_deref(), url, &transport));
    let app = Arc::clone(&app);
    let outcome = tokio::task::spawn_blocking(move || install(&app, &request, &download))
        .await
        .unwrap_or_else(|error| Err(Refusal::at(Stage::Store, error.to_string())));
    match outcome {
        Ok(installed) => axum::Json(serde_json::json!({
            "ok": true,
            "id": installed.id,
        }))
        .into_response(),
        Err(refusal) => {
            let status = StatusCode::from_u16(refusal.stage.status())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, refusal.message).into_response()
        }
    }
}

/// The file a package lands under, for the settings import control.
pub fn editor_theme_path(settings: &Path, catalogue_id: &str) -> std::path::PathBuf {
    settings
        .join("editor-themes")
        .join(format!("{catalogue_id}.yunari-theme"))
}

/// One package's theme document, for a test that needs a valid envelope.
#[cfg(test)]
pub(crate) fn test_document(id: &str) -> yunari_model::document::ThemeDocument {
    use std::collections::BTreeMap;

    use yunari_model::document::{
        Artboard, ArtboardShape, Background, ContentFit, Metadata, Paint, PaletteEntry, StyleRef,
        StyleValue, ThemeDocument, TypeToken, TypeWeight,
    };

    let mut type_tokens = BTreeMap::new();
    for (id, family, weight) in [
        ("heading", "Segoe UI, sans-serif", 600.0),
        ("body", "Segoe UI, sans-serif", 400.0),
        ("mono", "Consolas, monospace", 400.0),
    ] {
        type_tokens.insert(
            id.to_owned(),
            TypeToken {
                family: family.to_owned(),
                weight: Some(TypeWeight::Number(weight)),
                face: None,
            },
        );
    }
    let mut palette = BTreeMap::new();
    for (id, name, colour) in [
        ("none", "None", "#00000000"),
        ("text", "Text", "#ecf5ff"),
        ("chartTrack", "Chart track", "#223047"),
    ] {
        palette.insert(
            id.to_owned(),
            PaletteEntry {
                name: name.to_owned(),
                value: Paint::Solid {
                    color: colour.to_owned(),
                },
            },
        );
    }
    ThemeDocument {
        schema_version: "3.0.0".to_owned(),
        fabric_version: "7.4.0".to_owned(),
        id: id.to_owned(),
        metadata: Metadata {
            name: Some(format!("Theme {id}")),
            theme_language: Some("en".to_owned()),
            ..Metadata::default()
        },
        artboard: Artboard {
            width: 1920.0,
            height: 1080.0,
            content_fit: ContentFit::Contain,
            shape: ArtboardShape::Rect,
        },
        background: Background {
            mode: Default::default(),
            color: Some(StyleValue::Ref(StyleRef {
                r#ref: "palette.chartTrack".to_owned(),
            })),
            media: None,
        },
        globals: Default::default(),
        scene: Default::default(),
        palette,
        type_tokens,
        assets: Vec::new(),
        nodes: Vec::new(),
        bindings: BTreeMap::new(),
        editor_metadata: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn app(name: &str) -> Shared {
        let dir = std::env::temp_dir().join(format!(
            "yunari-install-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        crate::server::app(crate::config::HostConfig::resolve(
            Some(dir),
            Some(0),
            false,
        ))
    }

    const CATALOGUE: &str = "0123abcd-4567-89ab-cdef-0123456789ab";

    fn request(kind: Kind, bytes: &[u8]) -> InstallRequest {
        InstallRequest {
            package_url: "https://store.example/api/upload/x".to_owned(),
            catalogue_id: CATALOGUE.to_owned(),
            kind,
            version: "1.0.0".to_owned(),
            sha256: yunari_library::sha256_hex(bytes),
            bytes: bytes.len() as u64,
        }
    }

    fn served(bytes: Vec<u8>) -> Download {
        Arc::new(move |_| Ok(bytes.clone()))
    }

    #[test]
    fn a_widget_pack_installs() {
        let app = app("widget-pack");
        let bytes = yunari_packs::STARTER.to_vec();
        let installed = install(
            &app,
            &request(Kind::WidgetPack, &bytes),
            &served(bytes.clone()),
        )
        .expect("an installed pack");
        let packs = yunari_packs::directory(&app.config.settings_dir());
        assert!(
            yunari_packs::installed(&packs)
                .iter()
                .any(|pack| pack.id == installed.id)
        );
    }

    #[test]
    fn a_wrong_digest_writes_nothing() {
        let app = app("digest");
        let bytes = yunari_packs::STARTER.to_vec();
        let mut request = request(Kind::WidgetPack, &bytes);
        request.sha256 = "0".repeat(64);
        let packs = yunari_packs::directory(&app.config.settings_dir());
        let before: Vec<String> = yunari_packs::installed(&packs)
            .into_iter()
            .map(|pack| pack.id)
            .collect();
        let refusal = install(&app, &request, &served(bytes)).expect_err("a refusal");
        assert_eq!(refusal.stage, Stage::Integrity);
        let after: Vec<String> = yunari_packs::installed(&packs)
            .into_iter()
            .map(|pack| pack.id)
            .collect();
        assert_eq!(after, before);
    }

    #[test]
    fn a_dashboard_theme_lands_in_the_library_with_its_origin() {
        let app = app("dashboard");
        let package = yunari_library::package::write_package(
            &test_document("aurora"),
            &BTreeMap::new(),
            None,
            None,
        )
        .expect("a written package");
        let installed = install(
            &app,
            &request(Kind::DashboardTheme, &package),
            &served(package.clone()),
        )
        .expect("an installed theme");
        assert_eq!(installed.id, "aurora");
        let entry = app
            .library
            .list()
            .into_iter()
            .find(|entry| entry.id == "aurora")
            .expect("the stored theme");
        assert_eq!(entry.name, "Theme aurora");
        let origins = Origins::at(app.config.settings_dir());
        assert_eq!(
            origins
                .read()
                .get("aurora")
                .map(|origin| origin.catalogue_id.as_str()),
            Some(CATALOGUE)
        );

        // A second install of the same catalogue item updates that copy in
        // place rather than creating a sibling.
        let mut updated = test_document("aurora");
        updated.metadata.name = Some("Aurora".to_owned());
        let package =
            yunari_library::package::write_package(&updated, &BTreeMap::new(), None, None)
                .expect("a written package");
        let installed = install(
            &app,
            &request(Kind::DashboardTheme, &package),
            &served(package.clone()),
        )
        .expect("an updated theme");
        assert_eq!(installed.id, "aurora");
        assert_eq!(app.library.list().len(), 1);
        assert_eq!(
            app.library
                .read("aurora")
                .expect("the theme")
                .document
                .metadata
                .name
                .as_deref(),
            Some("Aurora")
        );
    }

    #[test]
    fn an_editor_theme_lands_as_a_file_and_a_bad_archive_does_not() {
        let app = app("editor-theme");
        let package = yunari_library::package::write_package(
            &test_document("aurora"),
            &BTreeMap::new(),
            None,
            None,
        )
        .expect("a written package");
        install(
            &app,
            &request(Kind::EditorTheme, &package),
            &served(package.clone()),
        )
        .expect("an installed editor theme");
        assert!(editor_theme_path(&app.config.settings_dir(), CATALOGUE).is_file());

        let refusal = install(
            &app,
            &request(Kind::EditorTheme, b"not a zip"),
            &served(b"not a zip".to_vec()),
        )
        .expect_err("a refusal");
        assert_eq!(refusal.stage, Stage::Package);
    }

    #[test]
    fn a_plugin_is_refused_as_this_hosts_own_gap() {
        let app = app("dropped-kinds");
        let bytes = b"whatever".to_vec();
        let refusal =
            install(&app, &request(Kind::Plugin, &bytes), &served(bytes)).expect_err("a refusal");
        assert_eq!(refusal.stage, Stage::Store);
        assert_eq!(refusal.message, NO_PLUGINS_MESSAGE);
    }

    #[test]
    fn a_path_preset_pack_is_installed_into_the_presets_store() {
        let app = app("preset-pack");
        let pack = serde_json::json!({
            "format": "yunari-path-preset-pack",
            "version": 1,
            "id": "space",
            "name": "Space shapes",
            "presets": [
                {"id": "orbit", "label": "Orbit", "data": "M 0 0 L 10 10 Z", "paint": "fill"}
            ]
        })
        .to_string()
        .into_bytes();
        let installed = install(
            &app,
            &request(Kind::PathPresetPack, &pack),
            &served(pack.clone()),
        )
        .expect("an installed pack");
        assert_eq!(installed.id, "space");
        assert!(
            crate::path_presets::Store::at(app.config.settings_dir())
                .read()
                .presets
                .iter()
                .any(|preset| preset.id == "pack:space:orbit")
        );
        // A pack that is not one is the package's refusal, not a store fault.
        let refusal = install(
            &app,
            &request(Kind::PathPresetPack, b"not a pack"),
            &served(b"not a pack".to_vec()),
        )
        .expect_err("a refusal");
        assert_eq!(refusal.stage, Stage::Package);
    }

    #[test]
    fn a_request_must_carry_every_fact_and_a_catalogue_uuid() {
        let bytes = yunari_packs::STARTER.to_vec();
        let request = request(Kind::WidgetPack, &bytes);
        let value = serde_json::json!({
            "packageUrl": request.package_url,
            "catalogueId": request.catalogue_id,
            "kind": "widget-pack",
            "version": request.version,
            "sha256": request.sha256,
            "bytes": request.bytes,
        });
        assert_eq!(parse_request(&value), Some(request.clone()));

        let mut bad = value.clone();
        bad["catalogueId"] = serde_json::json!("aurora");
        assert_eq!(parse_request(&bad), None);
        let mut bad = value.clone();
        bad["kind"] = serde_json::json!("theme");
        assert_eq!(parse_request(&bad), None);
        let mut bad = value.clone();
        bad["sha256"] = serde_json::json!("abc");
        assert_eq!(parse_request(&bad), None);
        let mut bad = value;
        bad["bytes"] = serde_json::json!(0);
        assert_eq!(parse_request(&bad), None);
    }

    #[test]
    fn a_package_must_come_from_the_stores_own_host_over_https() {
        let transport: Transport = Arc::new(|_| {
            Ok(yunari_store::Reply {
                status: 200,
                headers: Vec::new(),
                body: b"package".to_vec(),
            })
        });
        let reached = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reached);
        let counting: Transport = Arc::new(move |request| {
            counted.fetch_add(1, Ordering::SeqCst);
            (transport)(request)
        });
        let origin = Some("https://store.example");
        assert_eq!(
            download(origin, "http://store.example/x", &counting)
                .expect_err("a refusal")
                .stage,
            Stage::Address
        );
        assert_eq!(
            download(origin, "https://elsewhere.example/x", &counting)
                .expect_err("a refusal")
                .stage,
            Stage::Address
        );
        assert_eq!(reached.load(Ordering::SeqCst), 0);
        assert_eq!(
            download(origin, "https://store.example/x", &counting).expect("a package"),
            b"package"
        );
    }
}
