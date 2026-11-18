//! The monitors this PC shows, driven the way a panel is: the host's record of
//! a local display plus its assignment decides, and this thread keeps a window
//! per record rather than opening one per click.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use yunari_host::displays::DisplayKind;
use yunari_host::http::Shared;

/// How often the records are compared with the windows. A settings write lands
/// within a tick, so a monitor follows its assignment without a restart.
const TICK: Duration = Duration::from_secs(2);

/// What one window is meant to show. A save changes the stamp, so the window is
/// replaced rather than left painting the theme as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Want {
    monitor: Option<String>,
    theme: Option<PathBuf>,
    stamp_ms: i64,
}

/// Keeps one fullscreen window per local display record.
pub fn supervise(app: Shared, cx: &mut App) {
    let registry = Arc::clone(&app.registry);
    cx.spawn(async move |cx| {
        let mut open: BTreeMap<String, (Want, AnyWindowHandle)> = BTreeMap::new();
        loop {
            cx.background_executor().timer(TICK).await;
            let wanted = desired(&app);
            let stale: Vec<String> = open
                .iter()
                .filter(|(id, (want, _))| wanted.get(*id) != Some(want))
                .map(|(id, _)| id.clone())
                .collect();
            for id in stale {
                if let Some((_, handle)) = open.remove(&id) {
                    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
                }
            }
            for (id, want) in &wanted {
                if open.contains_key(id) {
                    continue;
                }
                let theme = want.theme.clone();
                let monitor = want.monitor.clone();
                let registry = Arc::clone(&registry);
                let handle = cx.update(|app| {
                    yunari_player::open_supervised(
                        registry,
                        theme.as_deref(),
                        monitor.as_deref(),
                        app,
                    )
                });
                if let Some(handle) = handle {
                    open.insert(id.clone(), (want.clone(), handle));
                }
            }
        }
    })
    .detach();
}

/// The windows that should be open, by display id: every local record that
/// names a monitor, with the theme its assignment resolves to.
fn desired(app: &Shared) -> BTreeMap<String, Want> {
    let Ok(displays) = app.displays.lock() else {
        return BTreeMap::new();
    };
    let Ok(assignment) = app.assignment.lock() else {
        return BTreeMap::new();
    };
    let mut wanted = BTreeMap::new();
    for record in displays.list() {
        let (DisplayKind::Local, Some(monitor)) = (record.kind, record.monitor.clone()) else {
            continue;
        };
        // A display with no theme to resolve stays dark rather than borrowing
        // whatever happens to be in the library.
        let Some(theme) = assignment.resolve(&app.library, &record.id, None) else {
            continue;
        };
        let path = app.library.root().join(&theme).join("theme.json");
        let stamp_ms = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        wanted.insert(
            record.id.clone(),
            Want {
                monitor: Some(monitor),
                theme: Some(path),
                stamp_ms,
            },
        );
    }
    wanted
}

#[cfg(test)]
mod tests {
    use yunari_host::config::HostConfig;

    use super::{Shared, desired};

    fn app(name: &str) -> Shared {
        let dir = std::env::temp_dir().join(format!(
            "yunari-displays-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0)
        ));
        let config = HostConfig::resolve(Some(dir), Some(0), false);
        yunari_host::server::app(config)
    }

    fn save_theme(app: &Shared, id: &str) {
        let directory = app.library.root().join(id);
        std::fs::create_dir_all(&directory).unwrap();
        let document = format!(
            r#"{{"schemaVersion":"3.0.0","id":"{id}","metadata":{{"name":"{id}"}},"artboard":{{"width":1920,"height":1080}},"nodes":[]}}"#
        );
        std::fs::write(directory.join("theme.json"), document).unwrap();
    }

    #[test]
    fn only_a_local_record_with_a_resolved_theme_wants_a_window() {
        let app = app("wanted");
        save_theme(&app, "shown");
        let mut displays = app.displays.lock().unwrap();
        let shown = displays
            .adopt_local("Monitor", Some("uuid-shown".to_owned()), None, None)
            .expect("a local record");
        let bare = displays
            .adopt_local("Bare", Some("uuid-bare".to_owned()), None, None)
            .expect("a second record");
        let unheld = displays
            .adopt_local("No monitor", None, None, None)
            .expect("a record with no monitor");
        drop(displays);
        app.assignment.lock().unwrap().set(&shown.id, Some("shown"));

        let wanted = desired(&app);
        // A record with no assignment of its own falls back to the only theme
        // in the library, so both monitors want a window; a record with no
        // monitor, and a record whose theme does not resolve, want none.
        assert_eq!(wanted.len(), 2, "{wanted:?}");
        let want = &wanted[&shown.id];
        assert_eq!(want.monitor.as_deref(), Some("uuid-shown"));
        assert_eq!(wanted[&bare.id].monitor.as_deref(), Some("uuid-bare"));
        assert!(
            want.theme
                .as_deref()
                .is_some_and(|path| path.ends_with("theme.json"))
        );
        assert!(want.stamp_ms > 0);
        assert!(!wanted.contains_key(&unheld.id));

        // Deleting the theme leaves nothing to resolve, so no window.
        std::fs::remove_dir_all(app.library.root().join("shown")).ok();
        assert!(desired(&app).is_empty());
    }
}
