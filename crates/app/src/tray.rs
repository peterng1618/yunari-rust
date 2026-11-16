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
                    yunari_tray::Action::Themes => {
                        open_editor_command(&host, yunari_editor::Command::Store, cx);
                    }
                    yunari_tray::Action::Settings => {
                        open_editor_command(&host, yunari_editor::Command::Settings, cx);
                    }
                    yunari_tray::Action::Mcp => {
                        open_editor_command(&host, yunari_editor::Command::Mcp, cx);
                    }
                    yunari_tray::Action::StartExternal(index) => {
                        if let Some(external) = externals(&host).get(index) {
                            yunari_tray::start_external(&external.executable);
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

/// Opens the editor already showing one of its dialogs. The tray never keeps a
/// second window over the same library, so this is a fresh window only when
/// no editor was opened at launch.
fn open_editor_command(host: &Shared, command: yunari_editor::Command, cx: &mut AsyncApp) {
    let host = host.clone();
    cx.update(|app| yunari_editor::open_command(Some(host), command, app));
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
        // The bridge ships beside the app, so the page is always offered.
        mcp: true,
        externals: externals(host),
    }
}

/// The external programs this PC can offer the tray: each row is a program the
/// app reads but never bundles. A program that is neither installed nor running
/// has no row, and one that is running is disabled rather than hidden.
fn externals(host: &Shared) -> Vec<yunari_tray::External> {
    let status = host.registry.status();
    let healthy = |id: &str| {
        status
            .iter()
            .find(|(provider, _, _)| *provider == id)
            .is_some_and(|(_, _, health)| health.reason().is_none())
    };
    let mut rows = Vec::new();
    // LHM answers through its web server, so "healthy" is the running signal.
    let lhm = find(
        "YUNARI_LHM",
        "LibreHardwareMonitor/LibreHardwareMonitor.exe",
    );
    if let Some(executable) = lhm {
        rows.push(yunari_tray::External {
            label: "LibreHardwareMonitor".to_owned(),
            running: healthy("lhm"),
            executable,
        });
    }
    let rtss = find("YUNARI_RTSS", "RivaTuner Statistics Server/RTSS.exe");
    if let Some(executable) = rtss {
        rows.push(yunari_tray::External {
            label: "RivaTuner Statistics Server".to_owned(),
            running: yunari_host::frame::rtss_running(),
            executable,
        });
    }
    rows
}

/// The first place `relative` exists under an override or this PC's program
/// directories. The override is a full path; the rest are standard installs.
fn find(variable: &str, relative: &str) -> Option<std::path::PathBuf> {
    let named = std::env::var_os(variable).map(std::path::PathBuf::from);
    let standard = ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| std::path::PathBuf::from(root).join(relative));
    named
        .into_iter()
        .chain(standard)
        .find(|path| path.is_file())
}
