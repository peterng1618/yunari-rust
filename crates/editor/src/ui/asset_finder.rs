//! The asset finder: pictures this PC already holds, the wallpaper library's
//! previews, and the stock sources this host serves. Picking a row puts the
//! bytes in the theme through the editor's own import, so the same sniffing
//! and SVG refusal apply.
//!
//! A stock search runs off the UI thread: a slow catalogue must not freeze the
//! window it is answering into.

use std::path::PathBuf;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;

use crate::ui::EditorWindow;
use crate::ui::prompt::Prompt;

/// One row's source. The bytes are read on the click, not when the dialog
/// opens: a library of wallpapers can be much larger than its listing.
#[derive(Debug, Clone)]
pub enum Pick {
    File(PathBuf),
    /// A Wallpaper Engine item's id; its preview is the picture.
    Wallpaper(String),
    /// A subscribed scene to write out as an editable project.
    Project(String),
    /// One stock row: the source it came from, its own id within it, and the
    /// credit the row knows.
    Stock {
        source: String,
        id: String,
        credit: crate::assets::ImportCredit,
    },
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub label: String,
    pub pick: Pick,
}

pub struct AssetFinder {
    pub choices: Vec<Choice>,
    /// Rows a stock search returned. Replaced by each answer, so a stale page
    /// cannot outlive the question that asked for it.
    pub remote: Vec<Choice>,
    /// The search box. Enter asks; Escape closes the finder from the shell.
    pub query: Prompt,
    pub searched: bool,
    pub busy: bool,
    pub note: Option<String>,
}

/// The picture extensions a local row may name. Bytes still decide what the
/// file is; the extension only decides what is listed.
const PICTURES: [&str; 7] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"];
/// A listing is a picker, not a filesystem walk.
const MAX_LOCAL: usize = 200;
/// The stock sources whose rows are pictures this finder can place. A font or
/// a gradient belongs to the typography and palette surfaces, not here.
const PICTURE_SOURCES: [&str; 4] = ["lucide", "lineicons", "fontawesome-free", "unsplash"];

impl Default for AssetFinder {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetFinder {
    pub fn new() -> Self {
        let mut choices = Vec::new();
        let mut note = None;
        let pictures = local_pictures();
        if pictures.is_empty() {
            note = Some("No pictures are in this PC's Pictures folder.".to_owned());
        }
        choices.extend(pictures.into_iter().map(|path| {
            Choice {
                label: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                pick: Pick::File(path),
            }
        }));

        match yunari_host::wallpaper_engine::locate() {
            Some(install) => {
                let scan = yunari_host::wallpaper_engine::scan(&install);
                choices.extend(scan.items.into_iter().flat_map(|item| {
                    let mut rows = vec![Choice {
                        label: format!("{}  ·  wallpaper", item.title),
                        pick: Pick::Wallpaper(item.workshop_id.clone()),
                    }];
                    // A scene is a package, not a picture: its own action is
                    // writing it out as a project the engine can edit.
                    if item.kind == yunari_host::wallpaper_engine::Kind::Scene {
                        rows.push(Choice {
                            label: format!("{}  ·  convert to a project", item.title),
                            pick: Pick::Project(item.workshop_id),
                        });
                    }
                    rows
                }));
            }
            None => {
                if note.is_none() {
                    note = Some("Wallpaper Engine is not installed on this PC.".to_owned());
                }
            }
        }
        Self {
            choices,
            remote: Vec::new(),
            query: Prompt::new("Search", "Search the stock sources", ""),
            searched: false,
            busy: false,
            note,
        }
    }
}

fn local_pictures() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return Vec::new();
    };
    let folder = PathBuf::from(home).join("Pictures");
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        PICTURES.contains(&extension.to_ascii_lowercase().as_str())
                    })
        })
        .take(MAX_LOCAL)
        .collect();
    paths.sort();
    paths
}

/// Asks every picture source. One source that cannot answer is a sentence
/// beside the others' rows, never an empty finder.
pub fn search_pictures(query: &str, credential: Option<&str>) -> (Vec<Choice>, Option<String>) {
    let mut choices = Vec::new();
    let mut notes = Vec::new();
    for id in PICTURE_SOURCES {
        let Some(stock) = yunari_host::assets::stock(id) else {
            continue;
        };
        let key = stock.requires.and(credential);
        match yunari_host::assets::search_with(id, query, 1, key) {
            Ok(page) => {
                for result in page.results {
                    let kind = result["kind"].as_str().unwrap_or_default();
                    if !matches!(kind, "image" | "svg" | "gif") {
                        continue;
                    }
                    let Some(id) = result["id"].as_str() else {
                        continue;
                    };
                    let name = result["name"].as_str().unwrap_or(id);
                    let field = |key: &str| {
                        result[key]
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .map(str::to_owned)
                    };
                    choices.push(Choice {
                        label: format!("{name}  ·  {}", stock.label),
                        pick: Pick::Stock {
                            source: stock.id.to_owned(),
                            id: id.to_owned(),
                            credit: crate::assets::ImportCredit {
                                kind: kind.to_owned(),
                                title: name.to_owned(),
                                license: field("licence").unwrap_or_else(|| "Unknown".to_owned()),
                                source_url: field("sourceUrl"),
                                // An image row's subtitle is its photographer.
                                author: (kind == "image").then(|| field("subtitle")).flatten(),
                            },
                        },
                    });
                }
            }
            Err(message) => notes.push(format!("{}: {message}", stock.label)),
        }
    }
    (choices, (!notes.is_empty()).then(|| notes.join("\n")))
}

/// The bytes a picked row stands for, or a sentence saying why not.
pub fn bytes_of(pick: &Pick, credential: Option<&str>) -> Result<(String, Vec<u8>), String> {
    match pick {
        Pick::File(path) => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "picture".to_owned());
            let bytes = std::fs::read(path).map_err(|error| format!("{error}"))?;
            Ok((name, bytes))
        }
        // A project is not bytes to place; it is a write on the host.
        Pick::Project(_) => Err("That row is not a picture.".to_owned()),
        Pick::Wallpaper(id) => {
            let (bytes, _) = yunari_host::wallpaper_engine::bytes(id, true)
                .ok_or_else(|| "That wallpaper has no preview to place.".to_owned())?;
            Ok((format!("wallpaper-{id}.jpg"), bytes))
        }
        Pick::Stock { source, id, .. } => {
            let keyed =
                yunari_host::assets::stock(source).is_some_and(|stock| stock.requires.is_some());
            let key = keyed.then_some(credential).flatten();
            let (bytes, _, name) = yunari_host::assets::bytes_with(source, id, key)
                .ok_or_else(|| "That row has no file to place.".to_owned())??;
            Ok((name, bytes))
        }
    }
}

pub fn view(dialog: &AssetFinder, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let rows: Vec<_> = dialog
        .choices
        .iter()
        .chain(dialog.remote.iter())
        .enumerate()
        .map(|(index, choice)| {
            let choice = choice.clone();
            div()
                .id(("asset-row", index))
                .w_full()
                .px_3()
                .py_2()
                .rounded_sm()
                .cursor_pointer()
                .bg(rgb(0x22222a))
                .child(choice.label.clone())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.place_asset(&choice, cx);
                }))
        })
        .collect();
    let search_text = if dialog.query.value.is_empty() {
        dialog.query.hint.clone()
    } else {
        dialog.query.value.clone()
    };
    let body: AnyElement = if rows.is_empty() {
        div()
            .p_3()
            .text_color(rgb(0x8a8a94))
            .child(
                dialog
                    .note
                    .clone()
                    .unwrap_or_else(|| "No pictures are available.".to_owned()),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .max_h(px(420.0))
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    };
    let status = if dialog.busy {
        "Searching…".to_owned()
    } else if let Some(note) = &dialog.note {
        note.clone()
    } else if dialog.searched {
        format!("{} stock rows", dialog.remote.len())
    } else {
        "Type a search and press Enter.".to_owned()
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(rgb(0xe8e8ea)).child("Place a picture"))
                .child(
                    div()
                        .id("asset-finder-search")
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(0x191920))
                        .text_color(rgb(0x8a8a94))
                        .child(search_text),
                )
                .child(div().text_color(rgb(0x8a8a94)).child(status)),
        )
        .child(body)
        .child(
            div()
                .id("asset-finder-cancel")
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
    use super::{PICTURE_SOURCES, PICTURES, Pick, bytes_of};

    #[test]
    fn only_picture_extensions_are_listed() {
        assert!(PICTURES.contains(&"PNG".to_ascii_lowercase().as_str()));
        assert!(!PICTURES.contains(&"exe"));
        assert!(!PICTURES.contains(&"mp4"));
    }

    #[test]
    fn a_preset_row_has_no_file_to_place() {
        let refused = bytes_of(
            &Pick::Stock {
                source: "colorhunt".to_owned(),
                id: "1234".to_owned(),
                credit: crate::assets::ImportCredit {
                    kind: "gradient".to_owned(),
                    title: "Dusk".to_owned(),
                    license: "CC0-1.0".to_owned(),
                    source_url: None,
                    author: None,
                },
            },
            None,
        )
        .unwrap_err();
        assert!(refused.contains("no file"), "{refused}");
    }

    #[test]
    fn the_picture_sources_are_the_picture_bodies() {
        for id in PICTURE_SOURCES {
            let stock = yunari_host::assets::stock(id).expect("a stock");
            assert_ne!(stock.category, "colour", "{id}");
            assert_eq!(stock.body, "grid", "{id}");
        }
    }
}
