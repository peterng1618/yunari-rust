//! Settings mode: the source's settings map, with the one page the port has.
//! Every other destination names itself instead of pretending to hold controls.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, list::ListItem};
use gpui_kit::*;

use crate::ui::EditorWindow;
use crate::ui::settings_dialog::{self, SettingsDialog};

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
    let label = label_at(selected);
    let content = match dialog {
        None => div()
            .text_color(cx.theme().muted_foreground)
            .child("Open the settings dialog from a host to see this PC's settings.")
            .into_any_element(),
        Some(dialog) if selected == 0 => settings_dialog::view(dialog, cx).into_any_element(),
        Some(_) => div()
            .text_color(cx.theme().muted_foreground)
            .child(format!("The {label} page is not ported yet."))
            .into_any_element(),
    };
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
        .child(content)
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
