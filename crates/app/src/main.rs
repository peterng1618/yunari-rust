#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use yunari_host::config::HostConfig;
use yunari_host::http::Shared;
use yunari_host::lock::HostLock;

mod displays;
mod tray;

/// How often the host looks for a changed phone setting. A flip rebinds the
/// listener, so the switch takes effect without restarting the app.
const SETTING_CHECK: Duration = Duration::from_secs(1);

/// The host runs on its own thread for as long as the app does. A second app
/// over the same folder keeps its editor and prints why its host did not start.
fn start_host() -> Option<Shared> {
    let config = HostConfig::resolve(None, None, false);
    let settings = config.settings_dir();
    let lock = match HostLock::acquire(&settings, config.port) {
        Ok(lock) => lock,
        Err(held) => {
            eprintln!("{}", held.message());
            return None;
        }
    };
    // The app is built once and shared with the main thread, so the records a
    // monitor window follows are the same records the host writes.
    let app = yunari_host::server::app(config);
    let shared = app.clone();
    std::thread::spawn(move || {
        let _lock = lock;
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("the host could not start: {error}");
                return;
            }
        };
        runtime.block_on(async move {
            let mut lan = yunari_host::hosting::Hosting::load(&settings).lan;
            loop {
                let server = match yunari_host::server::start(app.clone(), lan).await {
                    Ok(server) => server,
                    Err(error) => {
                        eprintln!("{error}");
                        return;
                    }
                };
                println!(
                    "Yunari host at {} ({})",
                    server.local_url,
                    if lan {
                        "phones on this network may pair"
                    } else {
                        "this PC only"
                    }
                );
                loop {
                    tokio::time::sleep(SETTING_CHECK).await;
                    let wanted = yunari_host::hosting::Hosting::load(&settings).lan;
                    if wanted != lan {
                        lan = wanted;
                        server.stop().await;
                        break;
                    }
                }
            }
        });
    });
    Some(shared)
}

/// The source shell's Graphite palette, role for role from
/// `editor-shell.css` (`[data-shell-palette="graphite"]`). Graphite is the
/// shell's prioritized theme; other palettes come later.
fn apply_graphite(cx: &mut App) {
    use gpui_kit::component::{Theme, ThemeMode};

    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        let ink = |hex: u32| Hsla::from(rgba(hex));
        theme.background = ink(0x0b0e12ff);
        theme.foreground = ink(0xe8ecf3ff);
        theme.border = ink(0x242b34ff);
        theme.title_bar = ink(0x12161cff);
        theme.title_bar_border = ink(0x242b34ff);
        theme.status_bar = ink(0x12161cff);
        theme.status_bar_border = ink(0x242b34ff);
        theme.sidebar = ink(0x12161cff);
        theme.sidebar_foreground = ink(0xe8ecf3ff);
        theme.sidebar_border = ink(0x242b34ff);
        theme.sidebar_accent = ink(0x171c23ff);
        theme.sidebar_accent_foreground = ink(0xe8ecf3ff);
        theme.muted = ink(0x171c23ff);
        theme.muted_foreground = ink(0x8a97abff);
        theme.accent = ink(0x171c23ff);
        theme.accent_foreground = ink(0xe8ecf3ff);
        theme.selection = ink(0x72e0c033);
        theme.ring = ink(0x72e0c0ff);
        theme.caret = ink(0x72e0c0ff);
        theme.primary = ink(0x72e0c0ff);
        theme.primary_foreground = ink(0x06131aff);
        theme.primary_hover = ink(0x8ae8ceff);
        theme.primary_active = ink(0x5ccbadff);
        theme.secondary = ink(0x171c23ff);
        theme.secondary_foreground = ink(0xe8ecf3ff);
        theme.secondary_hover = ink(0x1c222aff);
        theme.secondary_active = ink(0x1f2731ff);
        theme.popover = ink(0x12161cff);
        theme.popover_foreground = ink(0xe8ecf3ff);
        theme.input = ink(0x171c23ff);
        theme.danger = ink(0xff7e6eff);
        theme.success = ink(0x6fdc9cff);
        theme.warning = ink(0xf3aa64ff);
    });
}

/// `yunari-app [theme.json]` plays a theme; with no theme it opens the editor.
fn main() {
    let path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            apply_graphite(cx);
            let host = start_host();
            if let Some(app) = &host {
                displays::supervise(app.clone(), cx);
            }
            let editor = match path.as_deref() {
                Some(path) => {
                    yunari_player::open(Some(path), cx);
                    None
                }
                None => Some(yunari_editor::open_with(host.clone(), cx)),
            };
            if let Some(app) = &host {
                tray::run(app.clone(), editor, cx);
            }
        });
}
