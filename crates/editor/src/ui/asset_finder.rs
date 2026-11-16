//! The asset finder: pictures this PC already holds, and the wallpaper
//! library's previews. Picking a row puts the bytes in the theme through the
//! editor's own import, so the same sniffing and SVG refusal apply.

use std::path::PathBuf;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;

use crate::ui::EditorWindow;

/// One row's source. The bytes are read on the click, not when the dialog
/// opens: a library of wallpapers can be much larger than its listing.
#[derive(Debug, Clone)]
pub enum Pick {
    File(PathBuf),
    /// A Wallpaper Engine item's id; its preview is the picture.
    Wallpaper(String),
    /// A subscribed scene to write out as an editable project.
    Project(String),
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub label: String,
    pub pick: Pick,
}

pub struct AssetFinder {
    pub choices: Vec<Choice>,
    pub note: Option<String>,
}

/// The picture extensions a local row may name. Bytes still decide what the
/// file is; the extension only decides what is listed.
const PICTURES: [&str; 7] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"];
/// A listing is a picker, not a filesystem walk.
const MAX_LOCAL: usize = 200;

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
        Self { choices, note }
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

/// The bytes a picked row stands for, or a sentence saying why not.
pub fn bytes_of(pick: &Pick) -> Result<(String, Vec<u8>), String> {
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
    }
}

pub fn view(dialog: &AssetFinder, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let rows: Vec<_> = dialog
        .choices
        .iter()
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
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("Place a picture"),
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
    use super::PICTURES;

    #[test]
    fn only_picture_extensions_are_listed() {
        assert!(PICTURES.contains(&"PNG".to_ascii_lowercase().as_str()));
        assert!(!PICTURES.contains(&"exe"));
        assert!(!PICTURES.contains(&"mp4"));
    }
}
