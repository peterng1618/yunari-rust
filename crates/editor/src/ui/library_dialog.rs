//! The stored-theme list Open shows, with the actions that manage it.

use std::{collections::BTreeMap, sync::Arc};

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
    /// Real artboard previews, rastered once when the list opens. Images the
    /// document names are absent because reading them per card would stall the
    /// mode switch; shapes and text still come from the authored frame.
    pub frames: BTreeMap<String, Arc<gpui_kit::Image>>,
    /// Each authored artboard's width/height, used to justify grid rows.
    pub ratios: BTreeMap<String, f32>,
}

impl LibraryDialog {
    pub fn new(library: &Library) -> Self {
        let entries = library.list();
        let mut frames = BTreeMap::new();
        let mut ratios = BTreeMap::new();
        for entry in &entries {
            if let Some((frame, ratio)) = frame_of(library, &entry.id) {
                ratios.insert(entry.id.clone(), ratio);
                frames.insert(entry.id.clone(), frame);
            }
        }
        Self {
            entries,
            sort: ThemeSort::Name,
            frames,
            ratios,
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
pub fn page(
    dialog: &LibraryDialog,
    cx: &mut Context<EditorWindow>,
    width: gpui_kit::Pixels,
) -> impl IntoElement {
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
                .child(grid(dialog, cx, width)),
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

/// A theme's own thumbnail, written by the source when the theme was saved.
/// It is the same picture the original app's library draws, so the port reads
/// it rather than inventing a preview from a partial render.
fn frame_of(library: &Library, id: &str) -> Option<(Arc<gpui_kit::Image>, f32)> {
    let bytes = std::fs::read(library.root().join(id).join("thumbnail.png")).ok()?;
    // The PNG IHDR carries the pixel size; reading it avoids decoding twice.
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?) as f32;
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?) as f32;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let image = Arc::new(gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Png,
        bytes,
    ));
    Some((image, width / height))
}

/// The source's justified rows: cards keep their artboard ratios, a row is
/// scaled to the container, and the final row stays short at its target height.
/// Ported from `store-mode/justified-rows.ts` at the pinned source.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RowItem {
    index: usize,
    width: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct GalleryRow {
    items: Vec<RowItem>,
    height: f32,
    short: bool,
}

fn justify_rows(ratios: &[f32], width: f32, target: f32, gap: f32) -> Vec<GalleryRow> {
    let total = ratios.len();
    if total == 0 {
        return Vec::new();
    }
    let target = target.max(1.0);
    let gap = gap.max(0.0);
    let ratio = |index: usize| {
        let value = ratios[index];
        if value.is_finite() && value > 0.0 {
            value
        } else {
            1.0
        }
    };
    if !(width > 0.0) {
        return ratios
            .iter()
            .enumerate()
            .map(|(index, _)| GalleryRow {
                items: vec![RowItem {
                    index,
                    width: (target * ratio(index)).max(1.0),
                }],
                height: target,
                short: true,
            })
            .collect();
    }
    let height_at = |natural: f32| {
        if natural > 0.0 {
            (target * width) / natural
        } else {
            target
        }
    };
    let natural_width = |start: usize, count: usize| {
        (start..start + count)
            .enumerate()
            .fold(0.0, |sum, (offset, index)| {
                sum + target * ratio(index) + if offset > 0 { gap } else { 0.0 }
            })
    };
    let mut rows = Vec::new();
    let mut index = 0;
    while index < total {
        let mut fitted = 0usize;
        let mut natural = 0.0;
        while index + fitted < total {
            let next = target * ratio(index + fitted);
            let with_gap = natural + if fitted > 0 { gap } else { 0.0 } + next;
            if fitted > 0 && with_gap > width {
                break;
            }
            natural = with_gap;
            fitted += 1;
        }
        let fit_height = height_at(natural);
        let overflow_height = (index + fitted < total)
            .then(|| height_at(natural + gap + target * ratio(index + fitted)));
        let last = index + fitted >= total;
        let count = if last {
            fitted
        } else {
            match overflow_height {
                None => fitted,
                Some(overflow) => {
                    if (overflow - target).abs() <= (fit_height - target).abs() {
                        fitted + 1
                    } else {
                        fitted
                    }
                }
            }
        };
        let is_tail = index + count >= total;
        let natural_width = natural_width(index, count);
        let ragged = is_tail && natural_width <= width;
        let raw_height = if is_tail {
            target.min(height_at(natural_width))
        } else {
            height_at(natural_width)
        };
        let widths: Vec<f32> = (0..count)
            .map(|offset| raw_height * ratio(index + offset))
            .collect();
        let fixed = width.round();
        let mut widths = widths;
        if !ragged && !widths.is_empty() {
            let used: f32 = widths.iter().sum::<f32>() + gap * (count as f32 - 1.0);
            let last_width = widths.last_mut().unwrap();
            *last_width = (*last_width + fixed - used.round()).max(1.0);
        }
        rows.push(GalleryRow {
            items: widths
                .into_iter()
                .enumerate()
                .map(|(offset, card_width)| RowItem {
                    index: index + offset,
                    width: card_width,
                })
                .collect(),
            height: raw_height,
            short: ragged,
        });
        index += count;
    }
    rows
}

/// The Themes mode's justified gallery. The card keeps the theme's own shape;
/// no uniform tile is imposed on a 4:1 wallpaper or a square clock.
fn grid(
    dialog: &LibraryDialog,
    cx: &mut Context<EditorWindow>,
    width: gpui_kit::Pixels,
) -> AnyElement {
    use gpui_kit::component::ActiveTheme;

    let (ink, muted, panel) = (
        cx.theme().foreground,
        cx.theme().muted_foreground,
        cx.theme().popover,
    );
    let entries = dialog.sorted();
    let ratios: Vec<f32> = entries
        .iter()
        .map(|entry| dialog.ratios.get(&entry.id).copied().unwrap_or(2.0))
        .collect();
    let rows = justify_rows(&ratios, f32::from(width), 150.0, 12.0);
    if rows.is_empty() {
        return div()
            .p_3()
            .text_sm()
            .text_color(muted)
            .child("No stored themes yet. Save one first.")
            .into_any_element();
    }
    let mut columns: Vec<AnyElement> = Vec::new();
    for row in rows {
        let mut cards: Vec<AnyElement> = Vec::new();
        for item in row.items {
            let entry = entries[item.index];
            let open_id = entry.id.clone();
            let duplicate_id = entry.id.clone();
            let delete_id = entry.id.clone();
            let preview_height = (row.height - 26.0).max(24.0);
            let picture = match dialog.frames.get(&entry.id) {
                Some(frame) => div()
                    .relative()
                    .h(px(preview_height))
                    .w_full()
                    .overflow_hidden()
                    .child(
                        img(gpui_kit::ImageSource::Image(frame.clone()))
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(gpui_kit::ObjectFit::Contain),
                    )
                    .into_any_element(),
                None => div()
                    .h(px(preview_height))
                    .w_full()
                    .bg(panel)
                    .into_any_element(),
            };
            cards.push(
                div()
                    .id(("library-card", item.index))
                    .w(px(item.width))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id(("library-card-open", item.index))
                            .cursor_pointer()
                            .child(picture)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.open_entry(&open_id, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .text_xs()
                            .text_color(ink)
                            .child(div().truncate().child(entry.name.clone()))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .text_color(muted)
                                    .child(action(
                                        ("library-card-duplicate", item.index),
                                        "Duplicate",
                                        move |this, _, _, cx| {
                                            this.duplicate_theme(&duplicate_id, cx)
                                        },
                                        cx,
                                    ))
                                    .child(action(
                                        ("library-card-delete", item.index),
                                        "Delete",
                                        move |this, _, _, cx| {
                                            this.confirm_delete_theme(delete_id.clone(), cx)
                                        },
                                        cx,
                                    )),
                            ),
                    )
                    .into_any_element(),
            );
        }
        columns.push(div().flex().gap_3().children(cards).into_any_element());
    }
    div()
        .flex()
        .flex_col()
        .gap_3()
        .children(columns)
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
    use super::{LibraryDialog, ThemeSort};
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
    fn justified_rows_keep_artboard_ratios_and_fill_the_row() {
        use super::justify_rows;

        let rows = justify_rows(&[2.0, 1.0, 4.0, 1.0], 800.0, 150.0, 12.0);
        assert!(!rows.is_empty());
        for row in &rows {
            let used: f32 = row.items.iter().map(|item| item.width).sum::<f32>()
                + 12.0 * (row.items.len() as f32 - 1.0);
            if row.short {
                assert!(used <= 800.0 + 1.0, "short row overflows: {used}");
            } else {
                assert!((used - 800.0).abs() < 1.0, "full row not flush: {used}");
            }
        }
        // A 4:1 wallpaper never becomes a uniform tile beside a square clock.
        let widths: Vec<f32> = rows
            .iter()
            .flat_map(|row| row.items.iter().map(|item| item.width))
            .collect();
        assert!(widths.iter().any(|width| *width > 150.0));
    }

    #[test]
    fn a_row_with_no_width_puts_one_card_per_row() {
        use super::justify_rows;

        let rows = justify_rows(&[2.0, 1.0], 0.0, 150.0, 12.0);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.short && row.items.len() == 1));
    }

    #[test]
    fn the_listing_sorts_by_name_or_by_the_latest_update() {
        let mut dialog = LibraryDialog {
            entries: vec![entry("Zulu", "z", 1), entry("alpha", "a", 3)],
            sort: ThemeSort::Name,
            frames: BTreeMap::new(),
            ratios: BTreeMap::new(),
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
