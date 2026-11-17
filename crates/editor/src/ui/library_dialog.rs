//! The stored-theme list Open shows, with the actions that manage it.

use std::collections::BTreeMap;

use gpui_kit::*;
use yunari_library::{Library, ThemeEntry};

use crate::ui::EditorWindow;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeSort {
    Name,
    Updated,
}

#[derive(Clone)]
pub struct LibraryDialog {
    pub entries: Vec<ThemeEntry>,
    pub sort: ThemeSort,
    /// A small palette sample per theme, read from its stored document. Full
    /// artboard previews need a raster pass and are not faked here.
    pub previews: BTreeMap<String, Vec<[u8; 4]>>,
}

impl LibraryDialog {
    pub fn new(library: &Library) -> Self {
        let entries = library.list();
        let mut previews = BTreeMap::new();
        for entry in &entries {
            let Ok(stored) = library.read(&entry.id) else {
                continue;
            };
            let colors: Vec<[u8; 4]> = stored
                .document
                .palette
                .values()
                .filter_map(|token| match &token.value {
                    yunari_model::document::Paint::Solid { color } => {
                        yunari_render::parse_color(color).map(|parsed| parsed.channels())
                    }
                    yunari_model::document::Paint::Gradient { .. } => None,
                })
                .take(6)
                .collect();
            if !colors.is_empty() {
                previews.insert(entry.id.clone(), colors);
            }
        }
        Self {
            entries,
            sort: ThemeSort::Name,
            previews,
        }
    }

    /// The listing the page draws: sorted by the chosen column, newest first
    /// when the author asked for time. The source lands on Name.
    pub fn sorted(&self) -> Vec<&ThemeEntry> {
        let mut rows: Vec<_> = self.entries.iter().collect();
        match self.sort {
            ThemeSort::Name => rows.sort_by(|left, right| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
                    .then_with(|| left.id.cmp(&right.id))
            }),
            ThemeSort::Updated => rows.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.cmp(&right.id))
            }),
        }
        rows
    }
}

/// A small clickable word in a row's action strip.
fn action(
    id: (&'static str, usize),
    label: &str,
    on_click: impl Fn(&mut EditorWindow, &ClickEvent, &mut Window, &mut Context<EditorWindow>) + 'static,
    cx: &mut Context<EditorWindow>,
) -> impl IntoElement {
    use gpui_kit::component::ActiveTheme;

    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .text_color(cx.theme().muted_foreground)
        .child(label.to_owned())
        .on_click(cx.listener(on_click))
}

/// The Themes mode's surface: the stored themes under the mode's own heading,
/// with no dismiss footer because nothing was opened over the editor.
pub fn page(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    use gpui_kit::component::{ActiveTheme, scroll::ScrollableElement};

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
                        .text_color(cx.theme().foreground)
                        .child("Stored themes"),
                )
                .child(sort_row(dialog, cx))
                .child(grid(dialog, cx)),
        )
}

/// The source's sort well, in its order: Name, then Updated. One surface owns
/// the choice, so the page and the `Open` dialog agree on what "first" means.
fn sort_row(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    use gpui_kit::component::{
        Sizable,
        button::{Button, ButtonVariants},
    };

    div().flex().gap_2().children(
        [(ThemeSort::Name, "Name"), (ThemeSort::Updated, "Updated")].map(|(sort, label)| {
            Button::new(format!("theme-sort-{label}"))
                .small()
                .with_variant(if dialog.sort == sort {
                    gpui_kit::component::button::ButtonVariant::Primary
                } else {
                    gpui_kit::component::button::ButtonVariant::Ghost
                })
                .label(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(themes) = &mut this.themes {
                        themes.sort = sort;
                    }
                    cx.notify();
                }))
        }),
    )
}

/// A palette strip: a small, truthful sample of the theme's own tokens. A
/// stored theme with unreadable colours answers an empty box, not a fake.
fn card_preview(dialog: &LibraryDialog, entry: &ThemeEntry) -> AnyElement {
    let Some(colors) = dialog.previews.get(&entry.id) else {
        return div()
            .h(px(64.0))
            .rounded_sm()
            .bg(cx_fallback())
            .into_any_element();
    };
    div()
        .h(px(64.0))
        .rounded_sm()
        .flex()
        .items_end()
        .children(colors.iter().map(|channels| {
            div()
                .flex_1()
                .h(px(16.0))
                .bg(rgba(channels_to_hex(*channels)))
        }))
        .into_any_element()
}

/// Packs a renderer colour into GPUI's RRGGBBAA form.
fn channels_to_hex(channels: [u8; 4]) -> u32 {
    (u32::from(channels[0]) << 24)
        | (u32::from(channels[1]) << 16)
        | (u32::from(channels[2]) << 8)
        | u32::from(channels[3])
}

/// The neutral fill used when a theme has no readable palette. Kept local so
/// no call site has to invent a colour.
fn cx_fallback() -> gpui_kit::Rgba {
    rgb(0x22222a)
}

/// Width of one library card. Three columns fit the 1024x700 logical minimum
/// with the source's comfortable gutters.
const CARD_WIDTH: Pixels = px(220.0);
const CARD_HEIGHT: Pixels = px(180.0);

/// The Themes mode's grid. Real theme previews need a raster pass per stored
/// theme, which the port does not do yet; the card still keeps the source's
/// shape — a tile per theme, name and author together, actions at the foot.
fn grid(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> AnyElement {
    use gpui_kit::component::ActiveTheme;

    let (ink, muted, border, panel) = (
        cx.theme().foreground,
        cx.theme().muted_foreground,
        cx.theme().border,
        cx.theme().popover,
    );
    let cards: Vec<_> = dialog
        .sorted()
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let open_id = entry.id.clone();
            let duplicate_id = entry.id.clone();
            let delete_id = entry.id.clone();
            let author = entry
                .author
                .as_deref()
                .filter(|author| !author.is_empty())
                .unwrap_or("Unknown author");
            div()
                .id(("library-card", index))
                .w(CARD_WIDTH)
                .h(CARD_HEIGHT)
                .flex()
                .flex_col()
                .justify_between()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(panel)
                .child(
                    div()
                        .id(("library-card-open", index))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .cursor_pointer()
                        .child(div().text_sm().text_color(ink).child(entry.name.clone()))
                        .child(div().text_xs().text_color(muted).child(author.to_owned()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("{} nodes", entry.node_count)),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.open_entry(&open_id, cx)
                        })),
                )
                .child(self::card_preview(dialog, entry))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(action(
                            ("library-card-duplicate", index),
                            "Duplicate",
                            move |this, _, _, cx| this.duplicate_theme(&duplicate_id, cx),
                            cx,
                        ))
                        .child(action(
                            ("library-card-delete", index),
                            "Delete",
                            move |this, _, _, cx| this.confirm_delete_theme(delete_id.clone(), cx),
                            cx,
                        )),
                )
        })
        .collect();
    if cards.is_empty() {
        return div()
            .p_3()
            .text_sm()
            .text_color(muted)
            .child("No stored themes yet. Save one first.")
            .into_any_element();
    }
    div()
        .flex()
        .flex_wrap()
        .gap_3()
        .children(cards)
        .into_any_element()
}

/// The list's contents: one row per stored theme, or why there are none. A
/// theme that cannot be read is already skipped by the listing.
fn list(dialog: &LibraryDialog, cx: &mut Context<EditorWindow>) -> AnyElement {
    use gpui_kit::component::ActiveTheme;

    let rows: Vec<_> = dialog
        .sorted()
        .into_iter()
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
                .bg(cx.theme().popover)
                .child(
                    div()
                        .id(("library-open", index))
                        .flex_1()
                        .cursor_pointer()
                        .child(format!(
                            "{}  ·  {} nodes{}",
                            entry.name,
                            entry.node_count,
                            entry
                                .author
                                .as_ref()
                                .filter(|author| !author.is_empty())
                                .map(|author| format!("  ·  {author}"))
                                .unwrap_or_default()
                        ))
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
            .text_color(cx.theme().muted_foreground)
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
    use gpui_kit::component::ActiveTheme;

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(cx.theme().foreground)
                .child("Stored themes"),
        )
        .child(list(dialog, cx))
        .child(
            div()
                .id("library-cancel")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(cx.theme().muted_foreground)
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}

#[cfg(test)]
mod tests {
    use super::{LibraryDialog, ThemeSort, channels_to_hex};
    use std::collections::BTreeMap;
    use yunari_library::ThemeEntry;

    fn entry(name: &str, id: &str, seconds: u64) -> ThemeEntry {
        ThemeEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            author: None,
            version: None,
            updated_at: std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
            node_count: 1,
        }
    }

    #[test]
    fn the_listing_sorts_by_name_or_by_the_latest_update() {
        let mut dialog = LibraryDialog {
            entries: vec![entry("Zulu", "z", 1), entry("alpha", "a", 3)],
            sort: ThemeSort::Name,
            previews: BTreeMap::new(),
        };
        assert_eq!(
            dialog
                .sorted()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        dialog.sort = ThemeSort::Updated;
        assert_eq!(
            dialog
                .sorted()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        dialog.entries[1].updated_at = std::time::UNIX_EPOCH;
        assert_eq!(
            dialog
                .sorted()
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["z", "a"]
        );
    }
}

#[cfg(test)]
mod preview_tests {
    use super::channels_to_hex;

    #[test]
    fn palette_channels_pack_as_rrggbbaa() {
        assert_eq!(channels_to_hex([0x12, 0x34, 0x56, 0x78]), 0x12345678);
        assert_eq!(channels_to_hex([255, 0, 0, 255]), 0xff0000ff);
    }
}
