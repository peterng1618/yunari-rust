//! The asset finder: pictures this PC already holds, the wallpaper library's
//! previews, and the stock sources this host serves. Picking a row puts the
//! bytes in the theme through the editor's own import, so the same sniffing
//! and SVG refusal apply.
//!
//! A stock search runs off the UI thread: a slow catalogue must not freeze the
//! window it is answering into.

use std::collections::BTreeSet;
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
    /// The rail tab this row belongs to, so a tab shows only its own rows.
    pub tab: FinderTab,
    /// The source that offered this row, for the source chips.
    pub source: String,
    /// A local picture's own bytes, shown in the row. Remote and wallpaper
    /// rows carry no preview until the host answers with one.
    pub preview: Option<std::sync::Arc<gpui_kit::Image>>,
}

/// The source's finder rail, in its declared order. Icons is the opening tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderTab {
    Colors,
    Fonts,
    Icons,
    Images,
    Videos,
    WallpaperEngine,
}

impl FinderTab {
    pub const ALL: [Self; 6] = [
        Self::Colors,
        Self::Fonts,
        Self::Icons,
        Self::Images,
        Self::Videos,
        Self::WallpaperEngine,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Colors => "Colors",
            Self::Fonts => "Fonts",
            Self::Icons => "Icons",
            Self::Images => "Images",
            Self::Videos => "Videos",
            Self::WallpaperEngine => "Wallpaper Engine",
        }
    }
}

pub struct AssetFinder {
    pub tab: FinderTab,
    pub disabled: BTreeSet<String>,
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
            let preview = preview_of(&path);
            Choice {
                label: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string()),
                pick: Pick::File(path),
                tab: FinderTab::Images,
                source: "local".to_owned(),
                preview,
            }
        }));

        match yunari_host::wallpaper_engine::locate() {
            Some(install) => {
                let scan = yunari_host::wallpaper_engine::scan(&install);
                choices.extend(scan.items.into_iter().flat_map(|item| {
                    let mut rows = vec![Choice {
                        label: format!("{}  ·  wallpaper", item.title),
                        pick: Pick::Wallpaper(item.workshop_id.clone()),
                        tab: FinderTab::WallpaperEngine,
                        source: "wallpaper-engine".to_owned(),
                        preview: None,
                    }];
                    // A scene is a package, not a picture: its own action is
                    // writing it out as a project the engine can edit.
                    if item.kind == yunari_host::wallpaper_engine::Kind::Scene {
                        rows.push(Choice {
                            label: format!("{}  ·  convert to a project", item.title),
                            pick: Pick::Project(item.workshop_id),
                            tab: FinderTab::WallpaperEngine,
                            source: "wallpaper-engine".to_owned(),
                            preview: None,
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
            tab: FinderTab::Icons,
            disabled: BTreeSet::new(),
            choices,
            remote: Vec::new(),
            query: Prompt::new("Search", "Search the stock sources", ""),
            searched: false,
            busy: false,
            note,
        }
    }
}

/// Reads a local picture into the image element the kit draws. An unreadable
/// or unknown file simply has no preview; the row still names it and can place
/// it, so a broken thumbnail never becomes a broken action.
fn preview_of(path: &std::path::Path) -> Option<std::sync::Arc<gpui_kit::Image>> {
    const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;
    if std::fs::metadata(path).ok()?.len() > MAX_PREVIEW_BYTES {
        return None;
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let format = match extension.as_str() {
        "png" => gpui_kit::ImageFormat::Png,
        "jpg" | "jpeg" => gpui_kit::ImageFormat::Jpeg,
        "gif" => gpui_kit::ImageFormat::Gif,
        "webp" => gpui_kit::ImageFormat::Webp,
        "bmp" => gpui_kit::ImageFormat::Bmp,
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    Some(std::sync::Arc::new(gpui_kit::Image::from_bytes(
        format, bytes,
    )))
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
/// Fetches one stock row's bytes to draw its preview. Called on the search's
/// background thread; a refusal leaves the row without a picture rather than
/// holding the search open on one bad asset.
fn stock_preview(
    source: &str,
    id: &str,
    credential: Option<&str>,
) -> Option<std::sync::Arc<gpui_kit::Image>> {
    let (bytes, format, _) = yunari_host::assets::bytes_with(source, id, credential)
        .and_then(Result::ok)?;
    let format = match format {
        "png" => gpui_kit::ImageFormat::Png,
        "jpg" | "jpeg" => gpui_kit::ImageFormat::Jpeg,
        "gif" => gpui_kit::ImageFormat::Gif,
        "webp" => gpui_kit::ImageFormat::Webp,
        "svg" => gpui_kit::ImageFormat::Svg,
        _ => return None,
    };
    Some(std::sync::Arc::new(gpui_kit::Image::from_bytes(
        format, bytes,
    )))
}

pub fn search_pictures(query: &str, credential: Option<&str>) -> (Vec<Choice>, Option<String>) {
    search_sources(query, credential, &PICTURE_SOURCES, kinds_for(FinderTab::Images), FinderTab::Images)
}

/// The sources a tab may ask. Icons and images share one host search, but the
/// finder keeps each tab's answer to itself so switching back restores its
/// rows. Wallpaper Engine's local library has no remote search here.
pub fn search_for(
    tab: FinderTab,
    query: &str,
    credential: Option<&str>,
) -> (Vec<Choice>, Option<String>) {
    match tab {
        FinderTab::Icons => search_sources(
            query,
            credential,
            &["lucide", "lineicons", "fontawesome-free"],
            kinds_for(FinderTab::Icons),
            FinderTab::Icons,
        ),
        FinderTab::Images => search_sources(query, credential, &["unsplash"], kinds_for(FinderTab::Images), FinderTab::Images),
        FinderTab::Colors => search_sources(
            query,
            credential,
            &["webgradients", "uigradients", "colorswoop", "colorhunt"],
            kinds_for(FinderTab::Colors),
            FinderTab::Colors,
        ),
        FinderTab::Fonts => {
            search_sources(
                query,
                credential,
                &["google-fonts", "curated-fonts"],
                kinds_for(FinderTab::Fonts),
                FinderTab::Fonts,
            )
        }
        FinderTab::WallpaperEngine => (
            Vec::new(),
            Some("Wallpaper Engine searches this PC's library, not a remote source.".to_owned()),
        ),
        _ => (Vec::new(), None),
    }
}

/// The result kinds each tab can place.
fn kinds_for(tab: FinderTab) -> &'static [&'static str] {
    match tab {
        FinderTab::Colors => &["gradient", "palette"],
        FinderTab::Fonts => &["font"],
        FinderTab::Videos => &["video"],
        _ => &["image", "svg", "gif"],
    }
}

fn search_sources(
    query: &str,
    credential: Option<&str>,
    sources: &[&str],
    kinds: &[&str],
    tab: FinderTab,
) -> (Vec<Choice>, Option<String>) {
    let mut choices = Vec::new();
    let mut notes = Vec::new();
    for id in sources {
        let Some(stock) = yunari_host::assets::stock(id) else {
            continue;
        };
        let key = stock.requires.and(credential);
        match yunari_host::assets::search_with(id, query, 1, key) {
            Ok(page) => {
                for result in page.results {
                    let kind = result["kind"].as_str().unwrap_or_default();
                    if !kinds.contains(&kind) {
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
                    let preview = stock_preview(stock.id, id, credential);
                    choices.push(Choice {
                        label: format!("{name}  ·  {}", stock.label),
                        tab,
                        source: stock.id.to_owned(),
                        preview,
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
    use gpui_kit::component::{
        ActiveTheme,
        Sizable,
        button::{Button, ButtonVariants},
    };

    let tabs = div().flex().flex_wrap().gap_1().children(FinderTab::ALL.map(|tab| {
        Button::new(format!("asset-tab-{:?}", tab))
            .small()
            .with_variant(if dialog.tab == tab {
                gpui_kit::component::button::ButtonVariant::Primary
            } else {
                gpui_kit::component::button::ButtonVariant::Ghost
            })
            .label(tab.label())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(super::Overlay::AssetFinder(finder)) = &mut this.overlay {
                    finder.tab = tab;
                }
                cx.notify();
            }))
    }));
    let mut sources: Vec<String> = PICTURE_SOURCES.iter().map(|id| (*id).to_owned()).collect();
    sources.push("local".to_owned());
    sources.push("wallpaper-engine".to_owned());
    let chips = div().flex().flex_wrap().gap_1().children(sources.into_iter().map(|source| {
        let id = source.clone();
        Button::new(format!("asset-source-{source}"))
            .small()
            .with_variant(if dialog.disabled.contains(&source) {
                gpui_kit::component::button::ButtonVariant::Ghost
            } else {
                gpui_kit::component::button::ButtonVariant::Primary
            })
            .label(source)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(super::Overlay::AssetFinder(finder)) = &mut this.overlay {
                    if finder.disabled.contains(&id) {
                        finder.disabled.remove(&id);
                    } else {
                        finder.disabled.insert(id.clone());
                    }
                }
                cx.notify();
            }))
    }));
    let rows: Vec<_> = dialog
        .choices
        .iter()
        .chain(dialog.remote.iter())
        .filter(|choice| choice.tab == dialog.tab)
        .filter(|choice| !dialog.disabled.contains(&choice.source))
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(match &choice.preview {
                            Some(image) => div()
                                .size(px(36.0))
                                .flex_shrink_0()
                                .rounded_sm()
                                .overflow_hidden()
                                .child(
                                    img(gpui_kit::ImageSource::Image(image.clone()))
                                        .size_full()
                                        .object_fit(gpui_kit::ObjectFit::Cover),
                                )
                                .into_any_element(),
                            None => div().w(px(36.0)).into_any_element(),
                        })
                        .child(choice.label.clone()),
                )
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
    let unported = match dialog.tab {
        FinderTab::Videos => Some(format!(
            "The {} tab is not ported yet. Its sources need the host's asset registry.",
            dialog.tab.label()
        )),
        FinderTab::Icons
        | FinderTab::Images
        | FinderTab::Colors
        | FinderTab::Fonts
        | FinderTab::WallpaperEngine => None,
    };
    let body: AnyElement = if let Some(message) = unported {
        div()
            .p_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
            .into_any_element()
    } else if rows.is_empty() {
        div()
            .p_3()
            .text_color(rgb(0x8a8a94))
            .child(match dialog.tab {
                FinderTab::Icons => "Nothing to browse here yet. Try a word.".to_owned(),
                FinderTab::Images => dialog
                    .note
                    .clone()
                    .unwrap_or_else(|| "No pictures are available.".to_owned()),
                FinderTab::WallpaperEngine => "Wallpaper Engine is not installed on this PC."
                    .to_owned(),
                _ => "No results. Try another word.".to_owned(),
            })
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
    } else if let (Some(note), FinderTab::Images | FinderTab::WallpaperEngine) =
        (&dialog.note, dialog.tab)
    {
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
                .child(div().text_color(cx.theme().foreground).child("Asset Finder"))
                .child(tabs)
                .child(chips)
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
    use super::{FinderTab, PICTURE_SOURCES, PICTURES, Pick, bytes_of};

    #[test]
    fn the_finder_rail_uses_the_source_order_and_opens_on_icons() {
        assert_eq!(
            FinderTab::ALL.map(FinderTab::label),
            ["Colors", "Fonts", "Icons", "Images", "Videos", "Wallpaper Engine"]
        );
        assert_eq!(FinderTab::Icons.label(), "Icons");
    }

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
