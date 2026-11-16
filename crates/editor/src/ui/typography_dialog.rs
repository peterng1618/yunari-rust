//! The typography picker: the pairing presets a theme can take.

use gpui_kit::*;

use crate::typography::{TRIOS, Trio};
use crate::ui::EditorWindow;

pub struct TypographyDialog;

impl TypographyDialog {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TypographyDialog {
    fn default() -> Self {
        Self::new()
    }
}

/// One clickable row per pairing, with the three families named beside it.
pub fn view(_dialog: &TypographyDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let mut rows = div().flex().flex_col().gap_1().p_1();
    for (index, trio) in TRIOS.iter().enumerate() {
        let trio: Trio = *trio;
        rows = rows.child(
            div()
                .id(("typography-row", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(0x22222a))
                .flex()
                .flex_col()
                .child(trio.name)
                .child(div().text_color(rgb(0x8a8a94)).child(format!(
                    "{}  ·  {}  ·  {}",
                    trio.heading.family, trio.body.family, trio.mono.family
                )))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.apply_typography(trio, cx);
                })),
        );
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
                .child("Apply a typography preset"),
        )
        .child(rows)
        .child(
            div()
                .id("typography-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}
