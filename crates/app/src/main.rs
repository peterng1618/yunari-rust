#![forbid(unsafe_code)]

use std::path::PathBuf;

use gpui_kit::*;
use yunari_host::config::HostConfig;
use yunari_host::lock::HostLock;

/// The host runs on its own thread for as long as the app does. A second app
/// over the same folder keeps its editor and prints why its host did not start.
fn start_host() {
    let mut config = HostConfig::resolve(None, None, false);
    config.lan = yunari_host::hosting::Hosting::load(&config.settings_dir()).lan;
    let lock = match HostLock::acquire(&config.settings_dir(), config.port) {
        Ok(lock) => lock,
        Err(held) => {
            eprintln!("{}", held.message());
            return;
        }
    };
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
            let lan = config.lan;
            let app = yunari_host::server::app(config);
            match yunari_host::server::start(app, lan).await {
                Ok(server) => {
                    println!(
                        "Yunari host at {} (pairing code on /api/pairing/code)",
                        server.local_url
                    );
                    server.wait().await;
                }
                Err(error) => eprintln!("{error}"),
            }
        });
    });
}

/// `yunari-app [theme.json]` plays a theme; with no theme it opens the editor.
fn main() {
    let path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            start_host();
            match path.as_deref() {
                Some(path) => yunari_player::open(Some(path), cx),
                None => yunari_editor::open(cx),
            }
        });
}
