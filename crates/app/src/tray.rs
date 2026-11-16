//! The tray icon, driven by the same records the app shows.
//!
//! The tray itself is a Win32 window on its own thread; this module turns its
//! actions into app calls and keeps its tooltip current. A machine where the
//! icon cannot be created loses one surface, not the app.

use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::*;
use yunari_host::http::Shared;

/// How often the tooltip is refreshed. A port or a LAN flip lands within a tick.
const TICK: Duration = Duration::from_millis(500);

/// Starts the tray and keeps it in step with the host. `editor` is the window the
/// app already opened, so "Theme Editor" brings that one back instead of a
/// second copy over the same library.
pub fn run(host: Shared, editor: Option<AnyWindowHandle>, cx: &mut App) {
    let Some(tray) = yunari_tray::spawn(status(&host)) else {
        return;
    };
    cx.spawn(async move |cx| {
        let mut editor = editor;
        loop {
            cx.background_executor().timer(TICK).await;
            yunari_tray::update(status(&host));
            while let Ok(action) = tray.actions().try_recv() {
                match action {
                    yunari_tray::Action::AutoStart => {
                        let _ = yunari_host::autostart::set(!yunari_host::autostart::enabled());
                    }
                    yunari_tray::Action::Pair => {
                        let port = host.bound_port.load(Ordering::Relaxed);
                        yunari_tray::open_url(&format!("http://127.0.0.1:{port}/pair"));
                    }
                    yunari_tray::Action::Folder => {
                        yunari_tray::open_folder(&host.config.settings_dir());
                    }
                    yunari_tray::Action::Editor => {
                        let raised = editor.is_some_and(|handle| {
                            cx.update_window(handle, |_, window, _| window.activate_window())
                                .is_ok()
                        });
                        if !raised {
                            let host = host.clone();
                            editor =
                                Some(cx.update(|app| yunari_editor::open_with(Some(host), app)));
                        }
                    }
                    yunari_tray::Action::Exit => {
                        cx.update(|app| app.quit());
                        return;
                    }
                }
            }
        }
    })
    .detach();
}

fn status(host: &Shared) -> yunari_tray::Status {
    let port = host.bound_port.load(Ordering::Relaxed);
    let lan = yunari_host::hosting::Hosting::load(&host.config.settings_dir()).lan;
    yunari_tray::Status {
        tooltip: format!(
            "Yunari — localhost:{port} · {}",
            if lan { "LAN on" } else { "local only" }
        ),
        autostart: yunari_host::autostart::enabled(),
    }
}
