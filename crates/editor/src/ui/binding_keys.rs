//! The binding-key picker: every key the host's providers catalogue, with the
//! ones this PC answers marked, so a binding is chosen rather than typed.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;

use yunari_host::http::Shared;
use yunari_render::Readings;

use crate::ui::EditorWindow;

#[derive(Debug, Clone)]
pub struct KeyRow {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub available: bool,
}

/// The key the editor's own polling has answered, which is the whole catalogue
/// when the document binds every one of them.
pub fn rows(host: Option<&Shared>, readings: &Readings) -> Vec<KeyRow> {
    let Some(host) = host else {
        return Vec::new();
    };
    let mut rows: Vec<KeyRow> = host
        .registry
        .describe()
        .into_iter()
        .map(|descriptor| {
            let described = yunari_providers::vocabulary::describe(&descriptor.semantic_key);
            let label = described.as_ref().map_or_else(
                || descriptor.semantic_key.clone(),
                |described| described.label.clone(),
            );
            let unit = described
                .and_then(|described| described.unit.map(str::to_owned))
                .or(descriptor.unit)
                .unwrap_or_default();
            KeyRow {
                available: readings.get(&descriptor.semantic_key).is_some(),
                label,
                unit,
                key: descriptor.semantic_key,
            }
        })
        .collect();
    rows.sort_by(|left, right| left.key.cmp(&right.key));
    rows
}

pub fn view(dialog: &[KeyRow], cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let rows: Vec<_> = dialog
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let key = row.key.clone();
            let text = if row.unit.is_empty() {
                format!("{}  ·  {}", row.label, row.key)
            } else {
                format!("{}  ·  {}  ·  {}", row.label, row.key, row.unit)
            };
            let entry = div()
                .id(("binding-key-row", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .bg(rgb(0x22222a))
                .text_color(if row.available {
                    rgb(0xe8e8ea)
                } else {
                    rgb(0x6a6a74)
                })
                .child(if row.available {
                    text
                } else {
                    format!("{text}  ·  unavailable")
                });
            if row.available {
                entry
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.choose_binding_key(&key, cx);
                    }))
            } else {
                entry
            }
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("Bind a measured key"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_1()
                .max_h(px(420.0))
                .overflow_y_scrollbar()
                .children(rows),
        )
        .child(
            div()
                .id("binding-key-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}

#[cfg(test)]
mod tests {
    use super::rows;

    #[test]
    fn a_host_without_a_registry_lists_nothing() {
        assert!(rows(None, &yunari_render::Readings::default()).is_empty());
    }
}
