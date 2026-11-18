//! Settings mode: the source's settings map, with the one page the port has.
//! Every other destination names itself instead of pretending to hold controls.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, list::ListItem};
use gpui_kit::*;

use crate::ui::EditorWindow;
use crate::ui::settings_dialog::SettingsDialog;

/// The destinations in the source's order, split by its two groups.
const THIS_PC: [&str; 5] = [
    "Displays",
    "Hardware",
    "SignalRGB",
    "Units & time",
    "Readings",
];
const EDITOR: [&str; 6] = [
    "Appearance",
    "Preview ratios",
    "Custom shape presets",
    "Keyboard",
    "Asset sources",
    "About",
];

/// Which destination is showing. Kept here rather than on `EditorWindow`: it is
/// view state of this page, not of the document or the host.
#[derive(Default)]
struct SettingsNav {
    selected: usize,
}

impl Global for SettingsNav {}

fn label_at(index: usize) -> &'static str {
    if let Some(label) = THIS_PC.get(index) {
        label
    } else {
        EDITOR.get(index - THIS_PC.len()).unwrap_or(&THIS_PC[0])
    }
}

/// The Settings mode's surface. `dialog` is owned by the host that opened it;
/// only the first page reuses that dialog's existing content.
pub fn page(dialog: Option<&SettingsDialog>, cx: &mut Context<EditorWindow>) -> AnyElement {
    let selected = cx.update_default_global::<SettingsNav, _>(|nav, _| nav.selected);
    div()
        .id("settings-mode")
        .size_full()
        .min_h(px(0.0))
        .overflow_y_scrollbar()
        .child(
            div()
                .flex()
                .min_h(px(0.0))
                .child(nav(cx, selected))
                .child(body(dialog, cx, selected)),
        )
        .into_any_element()
}

fn nav(cx: &mut Context<EditorWindow>, selected: usize) -> impl IntoElement {
    let row = |label: &'static str, index: usize| {
        ListItem::new(("settings-mode-nav", index))
            .w_full()
            .accessibility_label(label)
            .selected(index == selected)
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.update_default_global::<SettingsNav, _>(|nav, _| nav.selected = index);
                cx.notify();
            }))
            .child(label)
    };
    let this_pc = THIS_PC.into_iter().enumerate().map(|(i, l)| row(l, i));
    let editor = EDITOR
        .into_iter()
        .enumerate()
        .map(|(i, l)| row(l, THIS_PC.len() + i));

    div()
        .w_64()
        .flex_shrink_0()
        .min_h(px(0.0))
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().border)
        .p_4()
        .child(
            div()
                .pb_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child("This PC"),
        )
        .children(this_pc)
        .child(
            div()
                .pt_4()
                .pb_2()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child("Editor"),
        )
        .children(editor)
}

fn body(
    dialog: Option<&SettingsDialog>,
    cx: &mut Context<EditorWindow>,
    selected: usize,
) -> AnyElement {
    use gpui_kit::component::button::{Button, ButtonVariants};

    let label = label_at(selected);
    let hint = match selected {
        0 => "Each display shows its own theme. Assign one from the theme's row in Themes.",
        1 => "Which hardware the Host reports. These apply to every theme.",
        2 => "The colour receiver SignalRGB sends to, and the devices it publishes back.",
        3 => "How the Host's readings are shown, including weather place and units.",
        4 => "Where this PC's weather and custom readings come from.",
        5 => "The editor's own palette.",
        6 => "The display menu offers each preview ratio in both orientations.",
        7 => "The shapes the Add pane's Custom Shape section offers.",
        8 => "Every product action, and the chord that runs it.",
        9 => "Keys stay on this machine. Stored values are never shown again.",
        _ => "Yunari, a dashboard theme editor.",
    };
    let mut content: Vec<AnyElement> = Vec::new();
    content.push(
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(hint)
            .into_any_element(),
    );
    if selected == 8 {
        for (index, row) in crate::ui::menus::SHORTCUT_REFERENCE.iter().enumerate() {
            content.push(
                div()
                    .id(("settings-shortcut", index))
                    .flex()
                    .justify_between()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(row.label)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(row.chord),
                    )
                    .into_any_element(),
            );
        }
    }
    match (selected, dialog) {
        (_, None) => content.push(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Open the settings dialog from a host to see this PC's settings.")
                .into_any_element(),
        ),
        (0, Some(settings)) => {
            for monitor in &settings.monitors {
                let uuid = monitor.uuid.clone();
                let state = if monitor.shown {
                    "showing the active theme"
                } else {
                    "not used"
                };
                content.push(
                    Button::new(format!("settings-monitor-{}", monitor.uuid))
                        .ghost()
                        .label(format!("{}  ·  {}", monitor.label, state))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.toggle_monitor(&uuid, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        (1, Some(settings)) => {
            for (slot, name) in [("gpu", "GPU"), ("system-disk", "System disk"), ("data-disk", "Data disk")] {
                let chosen = match slot {
                    "gpu" => settings.devices.gpu.as_deref(),
                    "system-disk" => settings.devices.system_disk.as_deref(),
                    _ => settings.devices.data_disk.as_deref(),
                };
                let state = chosen.unwrap_or("first device this PC reports");
                content.push(
                    Button::new(format!("settings-device-{slot}"))
                        .ghost()
                        .label(format!("{name}: {state}  ·  click to change"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.cycle_device_slot(slot, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        (2, Some(settings)) => {
            let bridge = match (settings.bridge_listening, settings.bridge_error.as_deref()) {
                (Some(true), _) => format!("listening on UDP {}", settings.bridge_port),
                (Some(false), Some(error)) => format!("not listening — {error}"),
                _ => format!("UDP port {}", settings.bridge_port),
            };
            content.push(
                Button::new("settings-bridge")
                    .ghost()
                    .label(format!("SignalRGB bridge: {bridge}  ·  click to change"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.edit_bridge_port()))
                    .into_any_element(),
            );
        }
        (3, Some(settings)) => {
            let place = if settings.weather.label.is_empty() {
                "Windows location".to_owned()
            } else {
                format!("{} ({}, {})", settings.weather.label, settings.weather.latitude, settings.weather.longitude)
            };
            content.push(
                Button::new("settings-weather-backend")
                    .ghost()
                    .label(format!("Weather backend: {}", settings.weather.provider))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.cycle_weather_backend(cx)
                    }))
                    .into_any_element(),
            );
            content.push(
                Button::new("settings-weather-place")
                    .ghost()
                    .label(format!("Weather place: {place}"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.edit_weather_place()))
                    .into_any_element(),
            );
            content.push(
                Button::new("settings-weather-location")
                    .ghost()
                    .label(format!(
                        "Windows location: {}",
                        if settings.weather.use_windows_location { "On" } else { "Off" }
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_weather_location(cx)
                    }))
                    .into_any_element(),
            );
        }
        (4, Some(settings)) => {
            for (index, (name, state)) in settings.providers.iter().enumerate() {
                content.push(
                    div()
                        .id(("settings-provider", index))
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(format!("{name}: {state}"))
                        .into_any_element(),
                );
            }
        }
        (5, Some(_)) => content.push(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Graphite is the prioritized editor palette; light themes land later.")
                .into_any_element(),
        ),
        (6, Some(_)) => {
            // The source ships these five tokens. Editing the catalogue lives
            // with the host's lens store and is not wired here yet.
            for (index, token) in ["16:9", "19.5:9", "4:3", "1:1", "Circle"]
                .iter()
                .enumerate()
            {
                content.push(
                    div()
                        .id(("settings-preview-ratio", index))
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(*token)
                        .into_any_element(),
                );
            }
        }
        (7, Some(_)) => {
            let settings = yunari_host::config::HostConfig::resolve(None, None, false).settings_dir();
            let presets = yunari_host::path_presets::Store::at(settings).read();
            if presets.presets.is_empty() {
                content.push(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No custom shape presets on this PC.")
                        .into_any_element(),
                );
            }
            for (index, preset) in presets.presets.iter().enumerate() {
                content.push(
                    div()
                        .id(("settings-path-preset", index))
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(preset.label.clone())
                        .into_any_element(),
                );
            }
        }
        (9, Some(settings)) => {
            for (id, set) in &settings.source_credentials {
                let credential = id.clone();
                content.push(
                    Button::new(format!("settings-credential-{id}"))
                        .ghost()
                        .label(format!("Source credential {id}: {}", if *set { "stored" } else { "not set" }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, _| {
                            this.edit_credential(credential.clone())
                        }))
                        .into_any_element(),
                );
            }
        }
        (10, Some(_)) => content.push(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("Yunari v{} — dashboard theme editor.", env!("CARGO_PKG_VERSION")))
                .into_any_element(),
        ),
        (_, Some(_)) => content.push(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format!("The {label} page is not ported yet."))
                .into_any_element(),
        ),
    }
    div()
        .flex_1()
        .min_w_0()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(label),
        )
        .children(content)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{EDITOR, THIS_PC, label_at};

    #[test]
    fn navigation_follows_the_source_groups() {
        assert_eq!(
            THIS_PC,
            [
                "Displays",
                "Hardware",
                "SignalRGB",
                "Units & time",
                "Readings"
            ]
        );
        assert_eq!(
            EDITOR,
            [
                "Appearance",
                "Preview ratios",
                "Custom shape presets",
                "Keyboard",
                "Asset sources",
                "About",
            ]
        );
    }

    #[test]
    fn the_first_destination_is_the_page_the_port_has() {
        assert_eq!(label_at(0), "Displays");
        assert_eq!(label_at(THIS_PC.len()), "Appearance");
        assert_eq!(label_at(THIS_PC.len() + EDITOR.len() - 1), "About");
        assert_eq!(label_at(usize::MAX), "Displays");
    }
}
