//! The stored-theme list Open shows, with the actions that manage it.

use gpui_kit::*;
use yunari_library::{Library, ThemeEntry};

use crate::ui::EditorWindow;

#[derive(Clone)]
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

/// A small clickable word in a row's action strip.
fn action(
    id: (&'static str, usize),
    label: &str,
    on_click: impl Fn(&mut EditorWindow, &ClickEvent, &mut Window, &mut Context<EditorWindow>) + 'static,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .text_color(rgb(0x8a8a94))
        .child(label.to_owned())
        .on_click(cx.listener(on_click))
}

/// The Themes mode's surface: the stored themes under the mode's own heading,
/// with no dismiss footer because nothing was opened over the editor.
pub fn page(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    use gpui_kit::component::scroll::ScrollableElement;

    div()
        .id("themes-page")
        .size_full()
        .min_h(px(0.0))
        .overflow_y_scrollbar()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .child(
                    div()
                        .text_color(rgb(0xe8e8ea))
                        .child("Stored themes"),
                )
                .child(list(dialog, cx)),
        )
}

/// The list's contents: one row per stored theme, or why there are none. A
/// theme that cannot be read is already skipped by the listing.
fn list(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> AnyElement {
    let rows: Vec<_> = dialog
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let open_id = entry.id.clone();
            let duplicate_id = entry.id.clone();
            let delete_id = entry.id.clone();
            div()
                .id(("library-row", index))
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .py_2()
                .rounded_sm()
                .bg(rgb(0x22222a))
                .child(
                    div()
                        .id(("library-open", index))
                        .flex_1()
                        .cursor_pointer()
                        .child(format!("{}  ·  {} nodes", entry.name, entry.node_count))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.open_entry(&open_id, cx);
                        })),
                )
                .child(action(
                    ("library-duplicate", index),
                    "Duplicate",
                    move |this, _, _, cx| this.duplicate_theme(&duplicate_id, cx),
                    cx,
                ))
                .child(action(
                    ("library-delete", index),
                    "Delete",
                    move |this, _, _, cx| this.confirm_delete_theme(delete_id.clone(), cx),
                    cx,
                ))
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
    body
}

/// The `Open` dialog: the list, then the way out of it.
pub fn view(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("Stored themes"),
        )
        .child(list(dialog, cx))
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
