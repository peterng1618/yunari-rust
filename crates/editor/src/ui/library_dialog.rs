//! The stored-theme list Open shows, with the actions that manage it.

use std::{collections::{BTreeMap, BTreeSet}, sync::Arc};

use gpui_kit::*;
use yunari_library::{Library, ThemeEntry};

use crate::ui::EditorWindow;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThemeSort {
    Name,
    Updated,
    Created,
    Downloads,
    Favorites,
}

impl ThemeSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Updated => "Updated",
            Self::Created => "Created",
            Self::Downloads => "Downloads",
            Self::Favorites => "Favorites",
        }
    }
}

/// The four gallery densities, smallest cards first, as the source names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridDensity {
    Compact,
    Comfortable,
    Roomy,
    Spacious,
}

impl GridDensity {
    pub const ALL: [Self; 4] = [
        Self::Compact,
        Self::Comfortable,
        Self::Roomy,
        Self::Spacious,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Roomy => "Roomy",
            Self::Spacious => "Spacious",
        }
    }

    fn target_width(self) -> f32 {
        match self {
            Self::Compact => 150.0,
            Self::Comfortable => 204.0,
            Self::Roomy => 270.0,
            Self::Spacious => 360.0,
        }
    }
}

/// How many columns `width` holds at `level`, clamped to the source's range.
fn columns_for(width: f32, density: GridDensity) -> usize {
    let gap = 12.0;
    let fit = ((width + gap) / (density.target_width() + gap)).floor() as usize;
    fit.clamp(2, 12)
}

/// The Library mode's navigation, in the source's order. One owner for the
/// column and the page body, so a word cannot name a page that cannot draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryPage {
    DashboardThemes,
    AppThemes,
    Widgets,
    Shapes,
    SensorProviders,
    AssetSources,
    PanelDrivers,
    Collections,
    Submissions,
}

impl LibraryPage {
    pub const ALL: [Self; 9] = [
        Self::DashboardThemes,
        Self::AppThemes,
        Self::Widgets,
        Self::Shapes,
        Self::SensorProviders,
        Self::AssetSources,
        Self::PanelDrivers,
        Self::Collections,
        Self::Submissions,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::DashboardThemes => "Dashboard themes",
            Self::AppThemes => "App themes",
            Self::Widgets => "Widgets",
            Self::Shapes => "Shapes",
            Self::SensorProviders => "Sensor providers",
            Self::AssetSources => "Asset sources",
            Self::PanelDrivers => "Panel drivers",
            Self::Collections => "Collections",
            Self::Submissions => "My submissions",
        }
    }
}

/// The source's installed-state filter. A local theme is on this PC; rows the
/// catalogue holds alone would answer Not installed once the online half is
/// read. Until then that choice is empty rather than a lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryState {
    All,
    OnThisPc,
    NotInstalled,
}

impl LibraryState {
    pub const ALL: [Self; 3] = [Self::All, Self::OnThisPc, Self::NotInstalled];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::OnThisPc => "On this PC",
            Self::NotInstalled => "Not installed",
        }
    }
}

#[derive(Clone)]
pub struct FacetOption {
    pub id: String,
    pub name: String,
    pub count: usize,
}

#[derive(Clone)]
pub struct Facet {
    pub id: &'static str,
    pub name: &'static str,
    pub options: Vec<FacetOption>,
}

#[derive(Clone)]
pub struct LibraryDialog {
    pub entries: Vec<ThemeEntry>,
    pub page: LibraryPage,
    pub state: LibraryState,
    pub query: String,
    pub sort: ThemeSort,
    pub density: GridDensity,
    /// Real artboard previews, rastered once when the list opens. Images the
    /// document names are absent because reading them per card would stall the
    /// mode switch; shapes and text still come from the authored frame.
    pub frames: BTreeMap<String, Arc<gpui_kit::Image>>,
    /// Each authored artboard's width/height, used to justify grid rows.
    pub ratios: BTreeMap<String, f32>,
    /// The five facets, read off the stored documents rather than invented.
    pub facets: Vec<Facet>,
    /// One entry's values per facet, so filtering never re-reads a document.
    pub entry_facets: BTreeMap<String, BTreeMap<&'static str, BTreeSet<String>>>,
    pub chosen: BTreeMap<&'static str, BTreeSet<String>>,
    pub active_facet: Option<&'static str>,
}

/// Adds one reading to a facet's option counts.
fn count(map: &mut BTreeMap<String, (String, usize)>, id: String, name: String) {
    match map.entry(id) {
        std::collections::btree_map::Entry::Occupied(mut slot) => slot.get_mut().1 += 1,
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert((name, 1));
        }
    }
}

/// The source's five facets, read off the stored documents. An unreadable
/// document contributes no values rather than inventing them.
fn build_facets(
    library: &Library,
    entries: &[ThemeEntry],
) -> (
    Vec<Facet>,
    BTreeMap<String, BTreeMap<&'static str, BTreeSet<String>>>,
) {
    let keys: [(&str, &str); 5] = [
        ("author", "Author"),
        ("colour-mode", "Color mode"),
        ("sensors", "Sensors"),
        ("shape", "Shape"),
        ("style", "Style"),
    ];
    let mut counts: BTreeMap<&str, BTreeMap<String, (String, usize)>> =
        keys.iter().map(|(id, _)| (*id, BTreeMap::new())).collect();
    let mut per_entry = BTreeMap::new();
    for entry in entries {
        let Ok(stored) = library.read(&entry.id) else {
            continue;
        };
        let document = &stored.document;
        let mut values: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        if let Some(author) = &entry.author {
            if !author.is_empty() {
                values.entry("author").or_default().insert(author.clone());
                count(counts.get_mut("author").unwrap(), author.clone(), author.clone());
            }
        }
        let tags = &document.metadata.tags;
        for tag in tags {
            let lower = tag.to_lowercase();
            if lower == "light" || lower == "dark" {
                values.entry("colour-mode").or_default().insert(lower.clone());
                count(counts.get_mut("colour-mode").unwrap(), lower.clone(), tag.clone());
            }
            values.entry("style").or_default().insert(tag.clone());
            count(counts.get_mut("style").unwrap(), tag.clone(), tag.clone());
        }
        for bindings in document.bindings.values() {
            for binding in bindings {
                let Some((family, _)) = binding.semantic_key.split_once('.') else {
                    continue;
                };
                values.entry("sensors").or_default().insert(family.to_owned());
                count(
                    counts.get_mut("sensors").unwrap(),
                    family.to_owned(),
                    family.to_owned(),
                );
            }
        }
        let board = &document.artboard;
        if board.height > 0.0 {
            let ratio = board.width / board.height;
            let orientation = if ratio > 1.2 { "landscape" } else { "portrait" };
            values.entry("shape").or_default().insert(orientation.to_owned());
            count(
                counts.get_mut("shape").unwrap(),
                orientation.to_owned(),
                orientation.to_owned(),
            );
            let bucket = if ratio > 2.5 { "ultrawide" } else { "standard" };
            values.entry("shape").or_default().insert(bucket.to_owned());
            count(counts.get_mut("shape").unwrap(), bucket.to_owned(), bucket.to_owned());
        }
        per_entry.insert(entry.id.clone(), values);
    }
    let facets = keys
        .iter()
        .map(|(id, name)| Facet {
            id,
            name,
            options: counts[id]
                .iter()
                .map(|(option_id, (option_name, count))| FacetOption {
                    id: option_id.clone(),
                    name: option_name.clone(),
                    count: *count,
                })
                .collect(),
        })
        .collect();
    (facets, per_entry)
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
        let (facets, entry_facets) = build_facets(library, &entries);
        Self {
            entries,
            page: LibraryPage::DashboardThemes,
            state: LibraryState::All,
            query: String::new(),
            sort: ThemeSort::Updated,
            density: GridDensity::Comfortable,
            frames,
            ratios,
            facets,
            entry_facets,
            chosen: BTreeMap::new(),
            active_facet: None,
        }
    }

    /// The listing the page draws: search by name, then the chosen order.
    /// The source opens on Updated; a local row has no catalogue counts, so
    /// Created/Downloads/Favorites leave the retained order alone.
    pub fn sorted(&self) -> Vec<&ThemeEntry> {
        let needle = self.query.trim().to_lowercase();
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| needle.is_empty() || entry.name.to_lowercase().contains(&needle))
            .filter(|_| self.state != LibraryState::NotInstalled)
            .filter(|entry| {
                self.chosen.iter().all(|(facet, wanted)| {
                    if wanted.is_empty() {
                        return true;
                    }
                    self.entry_facets
                        .get(&entry.id)
                        .and_then(|values| values.get(facet))
                        .is_some_and(|values| wanted.iter().any(|id| values.contains(id)))
                })
            })
            .collect();
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
            _ => {}
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
    search: Option<&gpui_kit::Entity<gpui_kit::component::input::InputState>>,
) -> impl IntoElement {
    use gpui_kit::component::{
        ActiveTheme,
        Sizable,
        button::{Button, ButtonVariants},
        scroll::ScrollableElement,
    };

    let nav = div()
        .w(px(232.0))
        .flex_shrink_0()
        .min_h(px(0.0))
        .p_3()
        .flex()
        .flex_col()
        .gap_1()
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .pb_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Browse"),
        );
    let mut nav = nav;
    for page in LibraryPage::ALL {
        if page == LibraryPage::Submissions {
            nav = nav.child(
                div()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Publish"),
            );
        }
        let selected = dialog.page == page;
        nav = nav.child(
            Button::new(format!("library-page-{:?}", page))
                .ghost()
                .small()
                .w_full()
                .with_variant(if selected {
                    gpui_kit::component::button::ButtonVariant::Primary
                } else {
                    gpui_kit::component::button::ButtonVariant::Ghost
                })
                .label(page.label())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(themes) = &mut this.themes {
                        themes.page = page;
                    }
                    cx.notify();
                })),
        );
    }
    let body: AnyElement = if dialog.page == LibraryPage::DashboardThemes {
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
            .child(toolbar(dialog, cx, width - px(232.0), search))
            .child(grid(dialog, cx, width - px(232.0)))
            .into_any_element()
    } else {
        div()
            .p_6()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(format!(
                "The {} page is not ported yet.",
                dialog.page.label()
            ))
            .into_any_element()
    };
    div()
        .id("themes-page")
        .size_full()
        .min_h(px(0.0))
        .flex()
        .child(nav)
        .child(div().flex_1().min_w_0().overflow_y_scrollbar().child(body))
}

/// The source's listing toolbar: search, facets' place, the sort well and the
/// density level. Facets are derived from catalogue rows and land with the
/// library halves; the controls that are already backed by real data are here.
fn toolbar(
    dialog: &LibraryDialog,
    cx: &mut Context<EditorWindow>,
    width: gpui_kit::Pixels,
    search: Option<&gpui_kit::Entity<gpui_kit::component::input::InputState>>,
) -> impl IntoElement {
    use gpui_kit::component::{
        ActiveTheme,
        Sizable,
        button::{Button, ButtonVariants},
        input::Input,
    };

    let mut row = div().flex().flex_wrap().items_center().gap_2();
    if let Some(state) = search {
        row = row.child(
            div()
                .id("library-search")
                .w(px(260.0))
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().input)
                .child(Input::new(state)),
        );
    }
    for state in LibraryState::ALL {
        let active = dialog.state == state;
        row = row.child(
            Button::new(format!("library-state-{:?}", state))
                .small()
                .with_variant(if active {
                    gpui_kit::component::button::ButtonVariant::Primary
                } else {
                    gpui_kit::component::button::ButtonVariant::Ghost
                })
                .label(state.label())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(themes) = &mut this.themes {
                        themes.state = state;
                    }
                    cx.notify();
                })),
        );
    }
    for facet in &dialog.facets {
        let id = facet.id;
        let active = dialog.active_facet == Some(id);
        let chosen = dialog.chosen.get(id).is_some_and(|set| !set.is_empty());
        row = row.child(
            Button::new(format!("facet-{id}"))
                .small()
                .with_variant(if active || chosen {
                    gpui_kit::component::button::ButtonVariant::Primary
                } else {
                    gpui_kit::component::button::ButtonVariant::Ghost
                })
                .label(format!("{} ▾", facet.name))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(themes) = &mut this.themes {
                        themes.active_facet = if themes.active_facet == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                    }
                    cx.notify();
                })),
        );
    }
    // The source keeps both controls in one well: a sort select and a density
    // slider. Pressing either cycles the offered values until their popovers
    // are ported with the kit's Select and Slider.
    let sorts = [
        ThemeSort::Name,
        ThemeSort::Updated,
        ThemeSort::Created,
        ThemeSort::Downloads,
        ThemeSort::Favorites,
    ];
    let sort_index = sorts
        .iter()
        .position(|sort| *sort == dialog.sort)
        .unwrap_or(0);
    row = row.child(
        Button::new("theme-sort-well")
            .small()
            .label(format!("Sort themes: {}", sorts[sort_index].label()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(themes) = &mut this.themes {
                    let current = sorts
                        .iter()
                        .position(|sort| *sort == themes.sort)
                        .unwrap_or(0);
                    themes.sort = sorts[(current + 1) % sorts.len()];
                }
                cx.notify();
            })),
    );
    let densities = GridDensity::ALL;
    let density_index = densities
        .iter()
        .position(|density| *density == dialog.density)
        .unwrap_or(0);
    row = row.child(
        Button::new("library-density-well")
            .small()
            .label(format!(
                "Density: {} · {}",
                densities[density_index].label(),
                columns_for(f32::from(width), densities[density_index])
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if let Some(themes) = &mut this.themes {
                    let current = densities
                        .iter()
                        .position(|density| *density == themes.density)
                        .unwrap_or(0);
                    themes.density = densities[(current + 1) % densities.len()];
                }
                cx.notify();
            })),
    );
    let mut root = div().flex().flex_col().gap_2().child(row);
    if let Some(active) = dialog.active_facet {
        if let Some(facet) = dialog.facets.iter().find(|facet| facet.id == active) {
            let mut options = div().flex().flex_wrap().gap_2().px_2();
            for option in &facet.options {
                let facet_id = facet.id;
                let option_id = option.id.clone();
                let picked = dialog
                    .chosen
                    .get(facet_id)
                    .is_some_and(|set| set.contains(&option_id));
                options = options.child(
                    Button::new(format!("facet-{facet_id}-{}", option.id))
                        .small()
                        .with_variant(if picked {
                            gpui_kit::component::button::ButtonVariant::Primary
                        } else {
                            gpui_kit::component::button::ButtonVariant::Ghost
                        })
                        .label(format!("{}  ·  {}", option.name, option.count))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(themes) = &mut this.themes {
                                let set = themes.chosen.entry(facet_id).or_default();
                                if set.contains(&option_id) {
                                    set.remove(&option_id);
                                } else {
                                    set.insert(option_id.clone());
                                }
                            }
                            cx.notify();
                        })),
                );
            }
            root = root.child(options);
        }
    }
    root
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
    let gap = 12.0;
    let columns = columns_for(f32::from(width), dialog.density);
    let target_height = (f32::from(width) - gap * (columns as f32 - 1.0)) / columns as f32;
    let rows = justify_rows(&ratios, f32::from(width), target_height, gap);
    if rows.is_empty() {
        return div()
            .p_3()
            .text_sm()
            .text_color(muted)
            .child(if dialog.state == LibraryState::NotInstalled {
                "The store catalogue is not loaded yet."
            } else {
                "No stored themes yet. Save one first."
            })
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
            let group = format!("theme-card-{}", entry.id);
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
            // The source keeps the author on a hover chip and puts the name,
            // day and actions on one line. The day is the resting reading;
            // attending the card replaces it with the actions in place.
            let date = time::OffsetDateTime::from(entry.updated_at).date();
            let byline = format!(
                "{:04}-{:02}-{:02}",
                date.year(),
                date.month() as u8,
                date.day()
            );
            let author = entry
                .author
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|value| format!("© {value}"))
                .unwrap_or_else(|| "No author".to_owned());
            let chip_group = group.clone();
            let action_group = group.clone();
            let day_group = group.clone();
            cards.push(
                div()
                    .id(("library-card", item.index))
                    .group(group)
                    .relative()
                    .w(px(item.width))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .right(px(8.0))
                            .rounded_full()
                            .bg(panel)
                            .px_2()
                            .py_0p5()
                            .text_xs()
                            .text_color(muted)
                            .opacity(0.0)
                            .group_hover(chip_group, |style| style.opacity(1.0))
                            .child(author),
                    )
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
                            .relative()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(ink)
                            .child(div().flex_1().truncate().child(entry.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(muted)
                                    .group_hover(day_group, |style| style.opacity(0.0))
                                    .child(byline),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .right(px(8.0))
                                    .flex()
                                    .gap_2()
                                    .text_color(muted)
                                    .opacity(0.0)
                                    .group_hover(action_group, |style| style.opacity(1.0))
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
    use super::{GridDensity, LibraryDialog, LibraryPage, LibraryState, ThemeSort};
    use std::collections::BTreeSet;
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
    fn not_installed_has_no_local_rows_to_show() {
        let mut dialog = LibraryDialog {
            entries: vec![entry("Zulu", "z", 1), entry("alpha", "a", 3)],
            page: LibraryPage::DashboardThemes,
            state: LibraryState::NotInstalled,
            query: String::new(),
            sort: ThemeSort::Updated,
            density: GridDensity::Comfortable,
            frames: BTreeMap::new(),
            ratios: BTreeMap::new(),
            facets: Vec::new(),
            entry_facets: BTreeMap::new(),
            chosen: BTreeMap::new(),
            active_facet: None,
        };
        assert!(dialog.sorted().is_empty());
        dialog.state = LibraryState::OnThisPc;
        assert_eq!(dialog.sorted().len(), 2);
    }

    #[test]
    fn the_library_nav_uses_the_source_pages_in_order() {
        assert_eq!(
            LibraryPage::ALL.map(LibraryPage::label),
            [
                "Dashboard themes",
                "App themes",
                "Widgets",
                "Shapes",
                "Sensor providers",
                "Asset sources",
                "Panel drivers",
                "Collections",
                "My submissions",
            ]
        );
    }

    #[test]
    fn a_facet_choice_narrows_the_listing() {
        let mut entry_facets = BTreeMap::new();
        let mut values = BTreeMap::new();
        values.insert("author", BTreeSet::from(["Yunari".to_owned()]));
        entry_facets.insert("a".to_owned(), values);
        let mut dialog = LibraryDialog {
            entries: vec![entry("Zulu", "z", 1), entry("alpha", "a", 3)],
            page: LibraryPage::DashboardThemes,
            state: LibraryState::All,
            query: String::new(),
            sort: ThemeSort::Name,
            density: GridDensity::Comfortable,
            frames: BTreeMap::new(),
            ratios: BTreeMap::new(),
            facets: Vec::new(),
            entry_facets,
            chosen: BTreeMap::from([(
                "author",
                BTreeSet::from(["Yunari".to_owned()]),
            )]),
            active_facet: None,
        };
        assert_eq!(dialog.sorted().len(), 1);
        assert_eq!(dialog.sorted()[0].id, "a");
        dialog.chosen.clear();
        assert_eq!(dialog.sorted().len(), 2);
    }

    #[test]
    fn the_search_narrows_the_listing_by_name() {
        let mut dialog = LibraryDialog {
            entries: vec![entry("Zulu", "z", 1), entry("alpha", "a", 3)],
            page: LibraryPage::DashboardThemes,
            state: LibraryState::All,
            query: "ALP".to_owned(),
            sort: ThemeSort::Updated,
            density: GridDensity::Comfortable,
            frames: BTreeMap::new(),
            ratios: BTreeMap::new(),
            facets: Vec::new(),
            entry_facets: BTreeMap::new(),
            chosen: BTreeMap::new(),
            active_facet: None,
        };
        let ids: Vec<_> = dialog.sorted().iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a"]);
        dialog.query.clear();
        assert_eq!(dialog.sorted().len(), 2);
    }

    #[test]
    fn the_listing_sorts_by_name_or_by_the_latest_update() {
        let mut dialog = LibraryDialog {
            entries: vec![entry("Zulu", "z", 1), entry("alpha", "a", 3)],
            page: LibraryPage::DashboardThemes,
            state: LibraryState::All,
            query: String::new(),
            sort: ThemeSort::Name,
            density: GridDensity::Comfortable,
            frames: BTreeMap::new(),
            ratios: BTreeMap::new(),
            facets: Vec::new(),
            entry_facets: BTreeMap::new(),
            chosen: BTreeMap::new(),
            active_facet: None,
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
