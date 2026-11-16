//! The stored-theme picker Open shows.

use gpui_kit::*;
use yunari_library::{Library, ThemeEntry};

use crate::ui::EditorWindow;

pub struct LibraryDialog {
    pub entries: Vec<ThemeEntry>,
}

impl LibraryDialog {
    pub fn new(library: &Library) -> Self {
        Self {
            entries: library.list(),
        }
    }
}

/// The picker's contents: one clickable row per stored theme, or why there are
/// none. A theme that cannot be read is already skipped by the listing.
pub fn view(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let rows: Vec<_> = dialog
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let id = entry.id.clone();
            div()
                .id(("library-row", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(0x22222a))
                .child(format!("{}  ·  {} nodes", entry.name, entry.node_count))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.open_entry(&id, cx);
                }))
        })
        .collect();
    let body = if rows.is_empty() {
        div()
            .p_3()
            .text_color(rgb(0x8a8a94))
            .child("No stored themes yet. Save one first.")
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
                .child("Open a stored theme"),
        )
        .child(body)
        .child(
            div()
                .id("library-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}
