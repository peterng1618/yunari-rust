//! The installed-widget picker.

use std::path::Path;

use gpui_kit::*;
use yunari_packs::InstalledPack;

use crate::ui::EditorWindow;

pub struct WidgetDialog {
    pub packs: Vec<InstalledPack>,
}

impl WidgetDialog {
    pub fn new(directory: &Path) -> Self {
        Self {
            packs: yunari_packs::installed(directory),
        }
    }
}

/// One clickable row per unit of every installed pack, or how to install one.
pub fn view(dialog: &WidgetDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let mut rows = Vec::new();
    for pack in &dialog.packs {
        for unit in &pack.units {
            let unit = unit.clone();
            let label = format!(
                "{}  ·  {}",
                pack.name.clone().unwrap_or(pack.id.clone()),
                unit.label
            );
            rows.push(
                div()
                    .id(("widget-row", rows.len()))
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgb(0x22222a))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.insert_widget(unit.clone(), cx);
                    })),
            );
        }
    }
    let body = if rows.is_empty() {
        div()
            .p_3()
            .text_color(rgb(0x8a8a94))
            .child(
                "No widget packs are installed. Put a .yunari-widget-package in the host's settings/widget-packs folder.",
            )
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
                .child("Insert a widget"),
        )
        .child(body)
        .child(
            div()
                .id("widgets-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}
