#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;
use yunari_host::config::HostConfig;
use yunari_host::http::Shared;
use yunari_host::lock::HostLock;

mod displays;

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

/// `yunari-app [theme.json]` plays a theme; with no theme it opens the editor.
fn main() {
    let path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            let host = start_host();
            if let Some(app) = &host {
                displays::supervise(app.clone(), cx);
            }
            match path.as_deref() {
                Some(path) => yunari_player::open(Some(path), cx),
                None => yunari_editor::open_with(host, cx),
            }
        });
}
