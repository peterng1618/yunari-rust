//! The path-preset picker: the built-ins, the author's own rows and an
//! installed pack's rows, all read from the host's settings file.

use gpui_kit::*;
use yunari_host::path_presets::{self, Preset};

use crate::ui::EditorWindow;

pub struct PathPresetDialog {
    pub presets: Vec<Preset>,
}

impl PathPresetDialog {
    pub fn new(settings_dir: &std::path::Path) -> Self {
        Self {
            presets: path_presets::Store::at(settings_dir).read().presets,
        }
    }
}

/// One clickable row per preset, or where to put one.
pub fn view(dialog: &PathPresetDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let rows: Vec<_> = dialog
        .presets
        .iter()
        .enumerate()
        .map(|(index, preset)| {
            let preset = preset.clone();
            div()
                .id(("path-preset-row", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(0x22222a))
                .child(preset.label.clone())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.insert_path_preset(preset.clone(), cx);
                }))
        })
        .collect();
    let body = if rows.is_empty() {
        div()
            .p_3()
            .text_color(rgb(0x8a8a94))
            .child("No shape presets are stored.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .children(rows)
            .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("Insert a shape preset"),
        )
        .child(body)
        .child(
            div()
                .id("path-presets-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}
