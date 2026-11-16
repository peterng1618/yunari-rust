//! This PC's settings: what the host does, not what the document holds.

use std::path::{Path, PathBuf};

use gpui_kit::*;
use yunari_host::hosting::Hosting;

use crate::ui::EditorWindow;

/// Both switches, read once when the dialog opens.
pub struct SettingsDialog {
    pub settings: PathBuf,
    /// Whether phones on this network may reach the host.
    pub lan: bool,
    /// Whether the app starts with Windows.
    pub autostart: bool,
    /// The last result of flipping a switch, for the row under it.
    pub message: Option<String>,
}

impl SettingsDialog {
    pub fn new(settings: PathBuf) -> Self {
        Self {
            lan: Hosting::load(&settings).lan,
            autostart: yunari_host::autostart::enabled(),
            settings,
            message: None,
        }
    }
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
