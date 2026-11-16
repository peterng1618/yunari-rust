//! This PC's settings: what the host does, not what the document holds.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use yunari_host::displays::DisplayKind;
use yunari_host::hosting::Hosting;
use yunari_host::http::Shared;

use crate::ui::EditorWindow;

/// One monitor this PC has, and whether a display record follows it.
pub struct MonitorRow {
    pub uuid: String,
    pub label: String,
    pub shown: bool,
}

/// The switches, read once when the dialog opens.
pub struct SettingsDialog {
    pub settings: PathBuf,
    /// Whether phones on this network may reach the host.
    pub lan: bool,
    /// Whether the app starts with Windows.
    pub autostart: bool,
    pub monitors: Vec<MonitorRow>,
    /// The last result of flipping a switch, for the row under it.
    pub message: Option<String>,
}

impl SettingsDialog {
    pub fn new(settings: PathBuf, monitors: Vec<MonitorRow>) -> Self {
        Self {
            lan: Hosting::load(&settings).lan,
            autostart: yunari_host::autostart::enabled(),
            monitors,
            settings,
            message: None,
        }
    }
}

/// The monitors GPUI sees, each marked when a local display record holds it.
pub fn monitor_rows(host: Option<&Shared>, cx: &App) -> Vec<MonitorRow> {
    let records = host
        .and_then(|host| host.displays.lock().ok().map(|store| store.list().to_vec()))
        .unwrap_or_default();
    cx.displays()
        .iter()
        .map(|display| {
            let uuid = display
                .uuid()
                .map(|uuid| uuid.to_string())
                .unwrap_or_default();
            let bounds = display.bounds();
            let label = format!(
                "Monitor {}×{}",
                f32::from(bounds.size.width).round() as i64,
                f32::from(bounds.size.height).round() as i64
            );
            let shown = !uuid.is_empty()
                && records.iter().any(|record| {
                    record.kind == DisplayKind::Local
                        && record.monitor.as_deref() == Some(uuid.as_str())
                });
            MonitorRow { uuid, label, shown }
        })
        .collect()
}

/// Writes the LAN choice where the host reads it on its next bind.
pub fn set_lan(settings: &Path, lan: bool) -> bool {
    let mut hosting = Hosting::load(settings);
    hosting.lan = lan;
    hosting.save(settings);
    Hosting::load(settings).lan
}

/// Writes the autostart entry; returns the state Windows now reports.
pub fn set_autostart(on: bool) -> Result<bool, String> {
    yunari_host::autostart::set(on)?;
    Ok(yunari_host::autostart::enabled())
}

pub fn view(dialog: &SettingsDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let lan = if dialog.lan {
        "On  ·  phones on this network can pair and play"
    } else {
        "Off  ·  this PC only"
    };
    let autostart = if dialog.autostart { "On" } else { "Off" };
    let rows = [
        ("settings-lan", format!("Phones on this network: {lan}")),
        (
            "settings-autostart",
            format!("Start with Windows: {autostart}"),
        ),
    ];
    let mut column = div().flex().flex_col().gap_1();
    for (id, label) in rows {
        column = column.child(
            div()
                .id(id)
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xe8e8ea))
                .bg(rgb(0x22222a))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.flip_setting(id, cx);
                })),
        );
    }
    for (index, monitor) in dialog.monitors.iter().enumerate() {
        let uuid = monitor.uuid.clone();
        let state = if monitor.shown {
            "showing the active theme"
        } else {
            "not used"
        };
        let label = format!("{}  ·  {state}", monitor.label);
        column = column.child(
            div()
                .id(("settings-monitor", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xe8e8ea))
                .bg(rgb(0x22222a))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_monitor(&uuid, cx);
                })),
        );
    }
    let mut footer = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .text_color(rgb(0x8a8a94));
    footer = footer.child(format!("Settings: {}", dialog.settings.display()));
    if let Some(message) = &dialog.message {
        footer = footer.child(message.clone());
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("This PC"),
        )
        .child(column.p_1())
        .child(footer)
        .child(
            div()
                .id("settings-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Close  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}

#[cfg(test)]
mod tests {
    // Named imports, not a glob: GPUI exports its own `test` attribute.
    use std::path::PathBuf;

    use yunari_host::hosting::Hosting;

    use super::set_lan;

    fn temp_settings(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yunari-settings-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn the_lan_switch_lands_where_the_host_reads_it() {
        let settings = temp_settings("lan");
        assert!(!Hosting::load(&settings).lan);
        assert!(set_lan(&settings, true));
        assert!(Hosting::load(&settings).lan);
        assert!(!set_lan(&settings, false));
        std::fs::remove_dir_all(&settings).ok();
    }
}
