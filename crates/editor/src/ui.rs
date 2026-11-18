//! The editor window: a command bar, the artboard, and the dialogs a shell
//! needs to open, save, export and import a theme.
//!
//! The window owns the authoring state; every mutation goes through [`Editor`],
//! so undo, the dirty dot and what a save writes cannot disagree.

pub mod asset_finder;
pub mod binding_keys;
pub mod library_dialog;
pub mod mcp_dialog;
pub mod menus;
pub mod path_preset_dialog;
pub mod prompt;
pub mod settings_dialog;
pub mod settings_mode;
pub mod shell;
pub mod typography_dialog;
pub mod widget_dialog;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, TitleBar};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use yunari_host::http::Shared;
use yunari_library::{Library, LibraryError, default_root};
use yunari_model::document::{
    ChartContent, ChartFamily, ImageContent, ImageFit, Node, NodeKind, StyleLiteral, StyleMap,
    StyleRef, StyleValue, TextAlign, TextContent, TextOverflow, TextRun, ThemeDocument, Transform,
    VerticalAlign,
};
use yunari_model::presets::starter;
use yunari_providers::clock::ClockProvider;
use yunari_providers::registry::Registry;
use yunari_providers::sampler::Sampler;
use yunari_providers::system::SystemProvider;
use yunari_providers::units::UnitPreferences;
use yunari_render::plan::PlanIssue;
use yunari_render::{DisplayList, Fonts, Readings};
use yunari_render_raster::Images;

use self::asset_finder::AssetFinder;
use self::library_dialog::LibraryDialog;
use self::mcp_dialog::McpDialog;
use self::menus::{Command, SHORTCUT_REFERENCE};
use self::path_preset_dialog::PathPresetDialog;
use self::prompt::{Outcome, Prompt};
use self::settings_dialog::SettingsDialog;
use self::shell::{LeftPane, ShellMode};
use self::typography_dialog::TypographyDialog;
use self::widget_dialog::WidgetDialog;
use crate::arrange;
use crate::assets;
use crate::document::{Editor, Source};
use crate::inspector;
use crate::layers::{self, Move as LayerMove};
use crate::palette;
use crate::persistence::import_package;
use crate::selection::{self, Selection};
use crate::snap;
use crate::tokens;
use crate::transform::{self, Handle};
use crate::viewport::{LINE_HEIGHT, Viewport, ZOOM_STEP, wheel_factor};

/// How often the window looks at the sampler; the sampler decides when a poll
/// is actually due, so this only bounds how late a new reading can be.
const TICK: Duration = Duration::from_millis(250);
/// A playing clip repaints at its own cadence between sampler polls.
const FRAME: Duration = Duration::from_millis(33);

const ACCENT: u32 = 0x4f9cf9;
/// Arrow-key steps, in artboard units.
const NUDGE_STEP: f64 = 1.0;
const NUDGE_STEP_LARGE: f64 = 10.0;
/// How long the arrow-key burst waits for another press.
const NUDGE_IDLE_MS: u64 = 300;
const PANEL: u32 = 0x1d1d24;
const INK: u32 = 0xe8e8ea;
const MUTED: u32 = 0x8a8a94;
/// The gap a newly added node keeps from the artboard's edge, and the step
/// each further node takes so two adds never land on top of each other.
const INSET: f64 = 40.0;
const STEP: f64 = 80.0;

/// A message for the status line.
struct Status {
    text: String,
    error: bool,
}

impl Status {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

/// What sits over the artboard.
enum Overlay {
    SaveAs(Prompt),
    Import(Prompt),
    ImportAsset(Prompt),
    Library(LibraryDialog),
    Widgets(WidgetDialog),
    AssetFinder(AssetFinder),
    PathPresets(PathPresetDialog),
    Typography(TypographyDialog),
    /// How an agent reaches this host.
    Mcp(McpDialog),
    Settings(Box<SettingsDialog>),
    /// The `?` key reference: every command and its chord.
    Shortcuts,
    /// The header's one accent button: assign this theme to a display,
    /// then the publishing switch below the choice.
    Publish,
    /// A collection's install sheet: the includes the host's package names,
    /// and the optional ones this author chose to keep.
    CollectionConsent {
        row: serde_json::Value,
        includes: Vec<serde_json::Value>,
        chosen: BTreeSet<String>,
    },
    /// The canvas's menu, opened where the pointer pressed. Right-pressing an
    /// object selects it and offers its commands; a press on empty canvas
    /// offers what can be inserted.
    ContextMenu {
        at: Point<Pixels>,
        object: Option<String>,
    },
    /// The editor's own list of opens, newest first, under the toolbar button
    /// that asked for it.
    Recent {
        at: Point<Pixels>,
    },
    Rename(Prompt),
    Field(Prompt),
    /// A binding's key chosen from the host's measured catalogue.
    BindingKey {
        field: String,
        rows: Vec<binding_keys::KeyRow>,
    },
    /// The SignalRGB receiver's UDP port, typed in.
    BridgePort {
        dialog: Box<SettingsDialog>,
        prompt: Prompt,
    },
    /// A weather place typed as `label|latitude|longitude`, with the settings
    /// dialog it was opened from.
    WeatherPlace {
        dialog: Box<SettingsDialog>,
        prompt: Prompt,
    },
    /// A credential named by the settings (a weather backend's key or a source's
    /// `auth`), stored under the OS protector.
    Credential {
        dialog: Box<SettingsDialog>,
        id: String,
        prompt: Prompt,
    },
    /// A stored display's label, typed in.
    DisplayName {
        dialog: Box<SettingsDialog>,
        id: String,
        prompt: Prompt,
    },
    /// An action that would throw unsaved work away, waiting for an answer.
    Confirm {
        message: String,
        resume: Resume,
    },
}

/// The preview dragged with a layer row. It carries the row's name so the
/// pointer never loses track of what it is moving.
struct LayerDragPreview(String);

impl Render for LayerDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(self.0.clone())
    }
}

/// One of the editor's opens, by the id and the name it had when it opened.
#[derive(Clone)]
struct RecentTheme {
    id: String,
    name: String,
}

/// Reads opens from `~/.yunari/recents.json`. A missing or broken file is an
/// empty list: recents are a convenience, never a reason to fail a launch.
fn load_recents(library: &Library) -> Vec<RecentTheme> {
    let Some(parent) = library.root().parent() else {
        return Vec::new();
    };
    let bytes = match std::fs::read(parent.join("recents.json")) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let mut recents = Vec::new();
    if let Some(rows) = value.as_array() {
        for row in rows {
            let (Some(id), Some(name)) = (row["id"].as_str(), row["name"].as_str()) else {
                continue;
            };
            recents.push(RecentTheme {
                id: id.to_owned(),
                name: name.to_owned(),
            });
        }
    }
    recents.truncate(RECENT_LIMIT);
    recents
}

/// Writes opens back. A failed write leaves the in-memory list intact and is
/// not reported: losing a convenience list must not interrupt editing.
fn save_recents(library: &Library, recents: &[RecentTheme]) {
    let Some(parent) = library.root().parent() else {
        return;
    };
    let rows: Vec<_> = recents
        .iter()
        .map(|recent| serde_json::json!({ "id": recent.id, "name": recent.name }))
        .collect();
    if let Ok(bytes) = serde_json::to_vec_pretty(&serde_json::json!(rows)) {
        let _ = std::fs::create_dir_all(parent);
        let _ = std::fs::write(parent.join("recents.json"), bytes);
    }
}

/// The five ratio tokens the source ships until an author edits the list.
const DEFAULT_PREVIEW_RATIOS: [&str; 5] = ["16:9", "19.5:9", "4:3", "1:1", "Circle"];

fn preview_ratio_id(token: &str) -> Option<String> {
    let text = token.trim();
    if text.eq_ignore_ascii_case("circle") {
        return Some("circle".to_owned());
    }
    let (width, height) = text.split_once(':')?;
    let width: f64 = width.parse().ok()?;
    let height: f64 = height.parse().ok()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(format!("{:.6}", width / height))
}

fn preview_ratios_path() -> PathBuf {
    yunari_host::config::HostConfig::resolve(None, None, false)
        .settings_dir()
        .join("preview-ratios.json")
}

fn load_preview_ratios() -> Vec<String> {
    let Ok(bytes) = std::fs::read(preview_ratios_path()) else {
        return DEFAULT_PREVIEW_RATIOS.iter().map(|token| (*token).to_owned()).collect();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return DEFAULT_PREVIEW_RATIOS.iter().map(|token| (*token).to_owned()).collect();
    };
    let Some(rows) = value.as_array() else {
        return DEFAULT_PREVIEW_RATIOS.iter().map(|token| (*token).to_owned()).collect();
    };
    let mut seen = BTreeSet::new();
    let tokens: Vec<String> = rows
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter_map(|token| preview_ratio_id(token).map(|id| (token.to_owned(), id)))
        .filter(|(_, id)| seen.insert(id.clone()))
        .map(|(token, _)| token)
        .collect();
    if tokens.is_empty() {
        DEFAULT_PREVIEW_RATIOS.iter().map(|token| (*token).to_owned()).collect()
    } else {
        tokens
    }
}

/// The action a guard is holding.
#[derive(Clone, PartialEq, Eq)]
enum Resume {
    New,
    /// The reference composition the product ships, copied into a new theme.
    Template,
    Library,
    Import(String),
    /// A stored theme the author asked to delete, waiting for an answer.
    DeleteTheme(String),
    /// The window close the guard is holding.
    Quit,
}

/// An artboard box: x, y, width, height.
type Rect = (f64, f64, f64, f64);

/// A pointer gesture in flight. Each one records one history entry.
enum Gesture {
    /// The pressed node was already selected (or just became it).
    Move {
        start: (f64, f64),
        anchor: (f64, f64),
        starts: Vec<(String, Transform)>,
    },
    Resize {
        id: String,
        handle: Handle,
        start: Transform,
        rect: (f64, f64, f64, f64),
    },
    Rotate {
        id: String,
        start: Transform,
        angle: f64,
        centre: (f64, f64),
    },
}

pub struct EditorWindow {
    editor: Editor,
    library: Library,
    fonts: Fonts,
    /// What each key measured in recent polls, so a line chart draws a window.
    history: yunari_render::History,
    /// The moment of the last poll, which places the newest sample on a line.
    now_ms: u64,
    images: Rc<RefCell<Images>>,
    readings: Readings,
    preferences: UnitPreferences,
    sampler: Sampler,
    list: DisplayList,
    issues: Vec<PlanIssue>,
    selection: Selection,
    /// Canvas-local pixels the selection marquee started at and has reached.
    marquee: Option<((f32, f32), (f32, f32))>,
    gesture: Option<Gesture>,
    /// Lines the last drag step was pulled to, for painting.
    guides: Vec<snap::Guide>,
    /// An acquired equal-spacing line, held while the pointer stays within
    /// the source's wider release window.
    spacing_hold: Option<(snap::Axis, f64)>,
    /// Gaps between the first two selected nodes, held while their selection
    /// is dragged so an authored pattern survives the gesture.
    internal_spacing: Vec<(snap::Axis, String, String, f64)>,
    /// Bumped per arrow press so a stale idle timer ends nothing.
    nudge_epoch: u64,
    /// Group rows the layers panel is showing the children of.
    expanded: BTreeSet<String>,
    /// The row a rename prompt is editing.
    renaming: Option<String>,
    /// The row the Layers pane last focused, so arrow keys navigate the tree
    /// instead of nudging canvas objects until the canvas takes focus again.
    layer_focus: Option<String>,
    /// The inspector key a field prompt is editing.
    editing: Option<String>,
    overlay: Option<Overlay>,
    status: Option<Status>,
    focus: FocusHandle,
    /// Where the canvas was last painted, so a click can be turned into an
    /// artboard point.
    canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The window's height, so a menu opened near the foot can be kept on
    /// screen. Refreshed every render.
    window_height: Pixels,
    window_width: Pixels,
    /// The author's camera. Changes never enter history.
    viewport: Viewport,
    /// Whether the author has moved the camera; until they do the artboard is
    /// kept fitted as the canvas resizes.
    camera_touched: bool,
    /// Canvas pixel the current pan drag started at.
    panning: Option<Point<Pixels>>,
    /// Space holds pan mode, as in the source.
    space_held: bool,
    polling: bool,
    /// The running host, when this editor is attached to one.
    host: Option<Shared>,
    /// The Themes mode's listing, reloaded when the mode is entered so a save
    /// or a delete made elsewhere is not shown stale.
    themes: Option<library_dialog::LibraryDialog>,
    /// The catalogue halves the Library's Collections and Submissions pages
    /// read. Filled once from the host when the mode is first drawn.
    library_catalogue: Vec<serde_json::Value>,
    library_mine: Vec<serde_json::Value>,
    library_catalogue_loaded: bool,
    library_error: Option<String>,
    /// The display menu's ratio tokens, kept beside this PC's settings as the
    /// source keeps them in browser storage.
    preview_ratios: Vec<String>,
    /// The Themes mode's search box. The kit binds an `Input` to an entity, so
    /// the state is created with the first window that renders this editor.
    library_search: Option<Entity<InputState>>,
    /// The readings source toggle at the end of the source toolbar.
    readings_live: bool,
    /// The themes this editor has opened, newest first. The source keeps five.
    recents: Vec<RecentTheme>,
    /// Which top-level surface the header is showing. The editor body stays
    /// mounted while another mode shows, so the open document survives.
    mode: ShellMode,
    /// The left and right panes' collapsed state. View state, never document
    /// state.
    left_collapsed: bool,
    right_collapsed: bool,
    /// The rail slot the left pane is showing.
    left_pane: LeftPane,
}

/// Everything the product can insert, grouped as the source's Add pane groups
/// them. The pane and the empty-canvas context menu both read this, so a
/// right-click cannot offer a poorer list than the pane one gesture away.
pub(super) const INSERT_GROUPS: [(&str, &[Command]); 4] = [
    (
        "Text",
        &[
            Command::AddText,
            Command::AddVerticalText,
            Command::TypographyPreset,
        ],
    ),
    (
        "Shapes",
        &[
            Command::AddRectangle,
            Command::AddEllipse,
            Command::AddLine,
            Command::AddPath,
            Command::AddPathPreset,
            Command::AddArc,
            Command::AddWedge,
        ],
    ),
    (
        "Charts",
        &[
            Command::AddGauge,
            Command::AddLineChart,
            Command::AddBarChart,
            Command::AddPieChart,
            Command::AddRadarChart,
            Command::AddPolarBar,
        ],
    ),
    ("Media", &[Command::AddImage]),
];

/// One row of the canvas menu: a command, or a restack the registry does not
/// name. The words are the source's `object-actions.ts` labels.
#[derive(Clone, Copy, PartialEq)]
enum CanvasMenuEntry {
    Command(Command),
    Restack(LayerMove),
}

impl CanvasMenuEntry {
    fn label(self) -> &'static str {
        match self {
            Self::Command(command) => command.label(),
            Self::Restack(LayerMove::Front) => "Bring to front",
            Self::Restack(LayerMove::Forward) => "Bring forward",
            Self::Restack(LayerMove::Backward) => "Send backward",
            Self::Restack(LayerMove::Back) => "Send to back",
        }
    }
}

/// The canvas dock's rows: the registry's object commands, without the eight
/// arrange actions, which the source keeps in the selection inspector.
const DOCK_ACTIONS: [CanvasMenuEntry; 11] = [
    CanvasMenuEntry::Command(Command::Duplicate),
    CanvasMenuEntry::Command(Command::Copy),
    CanvasMenuEntry::Command(Command::Cut),
    CanvasMenuEntry::Command(Command::Paste),
    CanvasMenuEntry::Command(Command::DeleteSelected),
    CanvasMenuEntry::Command(Command::Group),
    CanvasMenuEntry::Command(Command::Ungroup),
    CanvasMenuEntry::Restack(LayerMove::Front),
    CanvasMenuEntry::Restack(LayerMove::Forward),
    CanvasMenuEntry::Restack(LayerMove::Backward),
    CanvasMenuEntry::Restack(LayerMove::Back),
];

/// The dock's icon for a row. The full catalog is embedded, so the source's
/// own marks are available rather than a nearest-available substitute.
fn dock_icon(entry: CanvasMenuEntry) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName as Glyph;
    match entry {
        CanvasMenuEntry::Command(Command::Duplicate) => Glyph::CopyPlus,
        CanvasMenuEntry::Command(Command::Copy) => Glyph::Copy,
        CanvasMenuEntry::Command(Command::Cut) => Glyph::Scissors,
        CanvasMenuEntry::Command(Command::Paste) => Glyph::ClipboardPaste,
        CanvasMenuEntry::Command(Command::DeleteSelected) => Glyph::Trash,
        CanvasMenuEntry::Command(Command::Group) => Glyph::Group,
        CanvasMenuEntry::Command(Command::Ungroup) => Glyph::Ungroup,
        CanvasMenuEntry::Restack(LayerMove::Front) => Glyph::ArrowUpToLine,
        CanvasMenuEntry::Restack(LayerMove::Forward) => Glyph::ArrowUp,
        CanvasMenuEntry::Restack(LayerMove::Backward) => Glyph::ArrowDown,
        CanvasMenuEntry::Restack(LayerMove::Back) => Glyph::ArrowDownToLine,
        _ => Glyph::Ellipsis,
    }
}

/// Runs a menu or dock row: a command through the registry, a restack on the
/// selected node.
fn run_canvas_entry(
    this: &mut EditorWindow,
    entry: CanvasMenuEntry,
    window: &mut Window,
    cx: &mut Context<EditorWindow>,
) {
    match entry {
        CanvasMenuEntry::Command(command) => this.apply(command, window, cx),
        CanvasMenuEntry::Restack(step) => {
            if let Some(id) = this.selection.first().map(str::to_owned) {
                this.editor.raise(&id, step);
                this.refresh();
                cx.notify();
            }
        }
    }
}

/// How many opens the recent list keeps, as the source's store does.
const RECENT_LIMIT: usize = 5;

/// Records an open: newest first, once each, capped as the source's store does.
fn note_recent(recents: &mut Vec<RecentTheme>, id: &str, name: String) {
    recents.retain(|recent| recent.id != id);
    recents.insert(
        0,
        RecentTheme {
            id: id.to_owned(),
            name,
        },
    );
    recents.truncate(RECENT_LIMIT);
}

/// The arrange actions, in the source's order. The source puts them in the
/// selection inspector, one row of six alignments and one of two distributions.
pub(super) const ARRANGE_ACTIONS: [Command; 8] = [
    Command::AlignLeft,
    Command::AlignCenterX,
    Command::AlignRight,
    Command::AlignTop,
    Command::AlignCenterY,
    Command::AlignBottom,
    Command::DistributeX,
    Command::DistributeY,
];

/// The object menu's groups, in the source's registry order.
const OBJECT_MENU: [&[CanvasMenuEntry]; 5] = [
    &[
        CanvasMenuEntry::Command(Command::Duplicate),
        CanvasMenuEntry::Command(Command::Copy),
        CanvasMenuEntry::Command(Command::Cut),
        CanvasMenuEntry::Command(Command::Paste),
        CanvasMenuEntry::Command(Command::DeleteSelected),
    ],
    &[
        CanvasMenuEntry::Command(Command::Group),
        CanvasMenuEntry::Command(Command::Ungroup),
    ],
    &[
        CanvasMenuEntry::Restack(LayerMove::Front),
        CanvasMenuEntry::Restack(LayerMove::Forward),
        CanvasMenuEntry::Restack(LayerMove::Backward),
        CanvasMenuEntry::Restack(LayerMove::Back),
    ],
    &[
        CanvasMenuEntry::Command(Command::AlignLeft),
        CanvasMenuEntry::Command(Command::AlignCenterX),
        CanvasMenuEntry::Command(Command::AlignRight),
        CanvasMenuEntry::Command(Command::AlignTop),
        CanvasMenuEntry::Command(Command::AlignCenterY),
        CanvasMenuEntry::Command(Command::AlignBottom),
    ],
    &[
        CanvasMenuEntry::Command(Command::DistributeX),
        CanvasMenuEntry::Command(Command::DistributeY),
    ],
];

impl EditorWindow {
    fn new(library: Library, host: Option<Shared>, cx: &mut Context<Self>) -> Self {
        let document = starter();
        let mut fonts = Fonts::new();
        let (list, issues) = yunari_render::build(&document, &mut fonts);
        let mut registry = Registry::new();
        registry.add(ClockProvider::new());
        registry.add(SystemProvider::new());
        let mut sampler = Sampler::new(registry);
        sampler.request("editor", yunari_render::reading::wanted_keys(&document));
        for issue in &issues {
            eprintln!("{:?} {:?}: {}", issue.code, issue.node_id, issue.detail);
        }
        let recents = load_recents(&library);
        Self {
            editor: Editor::new(document),
            library,
            fonts,
            history: yunari_render::History::new(),
            now_ms: 0,
            images: Rc::new(RefCell::new(Images::empty())),
            readings: Readings::default(),
            preferences: UnitPreferences::default(),
            sampler,
            list,
            issues,
            selection: Selection::new(),
            marquee: None,
            gesture: None,
            guides: Vec::new(),
            spacing_hold: None,
            internal_spacing: Vec::new(),
            nudge_epoch: 0,
            expanded: BTreeSet::new(),
            renaming: None,
            layer_focus: None,
            editing: None,
            overlay: None,
            themes: None,
            library_catalogue: Vec::new(),
            library_mine: Vec::new(),
            library_catalogue_loaded: false,
            library_error: None,
            preview_ratios: load_preview_ratios(),
            library_search: None,
            readings_live: false,
            recents,
            status: None,
            focus: cx.focus_handle(),
            canvas_bounds: Rc::new(Cell::new(None)),
            window_height: px(0.0),
            window_width: px(0.0),
            viewport: Viewport {
                scale: 1.0,
                offset: (0.0, 0.0),
            },
            camera_touched: false,
            panning: None,
            space_held: false,
            polling: false,
            host,
            mode: ShellMode::Editor,
            left_collapsed: false,
            right_collapsed: false,
            left_pane: LeftPane::Layers,
        }
    }

    /// Rebuilds the frame and asks for exactly the keys this document names.
    fn refresh(&mut self) {
        self.selection.retain_existing(self.editor.document());
        let (list, issues) = yunari_render::build_with_history(
            self.editor.document(),
            &mut self.fonts,
            &self.readings,
            &self.preferences,
            Some(&self.history),
            self.now_ms,
        );
        self.list = list;
        if issues != self.issues {
            for issue in &issues {
                if !self.issues.contains(issue) {
                    eprintln!("{:?} {:?}: {}", issue.code, issue.node_id, issue.detail);
                }
            }
            self.issues = issues;
        }
        self.sampler.request(
            "editor",
            yunari_render::reading::wanted_keys(self.editor.document()),
        );
    }

    /// Polls when the sampler says a poll is due. Returns whether the frame
    /// changed.
    fn poll(&mut self, now_ms: u64) -> bool {
        let Some(cycle) = self.sampler.tick(now_ms) else {
            return false;
        };
        self.readings = Readings::new(cycle.readings.into_values());
        self.now_ms = now_ms;
        self.history.record(now_ms, &self.readings);
        self.refresh();
        true
    }

    /// Pictures are read from beside the theme that names them, plus the bytes
    /// an import is holding that no save has written yet.
    fn point_images(&mut self) {
        let images = images_for(
            self.editor.source(),
            self.library.root(),
            self.editor.assets(),
        );
        self.images = Rc::new(RefCell::new(images));
        // The faces a theme ships are registered from the same two places, so
        // a reopened theme paints its own family before it is re-saved.
        let base = match self.editor.source() {
            Source::Library { id, .. } => Some(self.library.root().join(id)),
            Source::Unsaved => None,
        };
        let assets = self.editor.assets();
        let refusals = self.fonts.load_faces(self.editor.document(), |path| {
            assets.get(path).cloned().or_else(|| {
                base.as_ref()
                    .and_then(|base| std::fs::read(base.join(path)).ok())
            })
        });
        for refusal in refusals {
            eprintln!("{refusal}");
        }
    }

    fn apply(&mut self, command: Command, _window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::New => self.guard(Resume::New, cx),
            Command::NewFromTemplate => self.guard(Resume::Template, cx),
            Command::Open => self.guard(Resume::Library, cx),
            Command::Save => self.save(),
            Command::SaveAs => {
                self.overlay = Some(Overlay::SaveAs(Prompt::new(
                    "Save as",
                    "A theme id: lowercase letters, digits and dashes",
                    suggested_id(self.editor.document()),
                )));
            }
            Command::Export => self.export(),
            Command::ExportWidget => self.export_widget(),
            Command::Import => {
                self.overlay = Some(Overlay::Import(Prompt::new(
                    "Import package",
                    "Path to a .yunari-theme file",
                    "",
                )));
            }
            Command::Settings => {
                let monitors = settings_dialog::monitor_rows(self.host.as_ref(), cx);
                let displays = settings_dialog::display_rows(self.host.as_ref());
                let options = self
                    .host
                    .as_ref()
                    .map(|host| host.registry.devices())
                    .unwrap_or_default();
                let providers = self
                    .host
                    .as_ref()
                    .map(|host| {
                        host.registry
                            .status()
                            .into_iter()
                            .map(|(_, label, health)| {
                                (
                                    label.to_owned(),
                                    health.reason().unwrap_or("ready").to_owned(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut dialog = SettingsDialog::new(
                    self.settings_dir(),
                    monitors,
                    displays,
                    options,
                    providers,
                );
                if let Some(host) = self.host.as_ref() {
                    dialog =
                        dialog.with_bridge(host.signalrgb.is_some(), host.signalrgb_error.clone());
                }
                self.overlay = Some(Overlay::Settings(Box::new(dialog)));
            }
            Command::Widgets => {
                self.overlay = Some(Overlay::Widgets(WidgetDialog::new(&self.packs_directory())));
            }
            Command::AddPathPreset => {
                self.overlay = Some(Overlay::PathPresets(PathPresetDialog::new(
                    &self.settings_dir(),
                )));
            }
            Command::AssetFinder => {
                self.overlay = Some(Overlay::AssetFinder(AssetFinder::new()));
            }
            Command::TypographyPreset => {
                self.overlay = Some(Overlay::Typography(TypographyDialog::new()));
            }
            Command::Mcp => {
                let port = self.host.as_ref().map_or(0, |host| {
                    host.bound_port.load(std::sync::atomic::Ordering::Relaxed)
                });
                self.overlay = Some(Overlay::Mcp(McpDialog::new(port)));
            }
            // The source has no Store mode: the catalogue and the author's
            // own submissions live in the Library's navigation.
            Command::Store => {
                self.set_mode(ShellMode::Library, cx);
                if let Some(themes) = &mut self.themes {
                    themes.page = library_dialog::LibraryPage::Submissions;
                }
            }
            Command::ImportAsset => {
                self.overlay = Some(Overlay::ImportAsset(Prompt::new(
                    "Import asset",
                    "Path to a picture, video or Wallpaper Engine .mpkg on this PC",
                    "",
                )));
            }
            Command::Undo => {
                self.editor.undo();
                self.refresh();
            }
            Command::Redo => {
                self.editor.redo();
                self.refresh();
            }
            Command::AddRectangle => self.add(rectangle_node),
            Command::AddEllipse => self.add(ellipse_node),
            Command::AddPath => self.add(path_node),
            Command::AddLine => self.add(line_node),
            Command::AddArc => self.add(arc_node),
            Command::AddWedge => self.add(wedge_node),
            Command::AddText => self.add(text_node),
            Command::AddVerticalText => self.add(vertical_text_node),
            Command::AddImage => {
                let asset = assets::first_picture(self.editor.document()).map(|a| a.id.clone());
                match asset {
                    Some(asset_id) => self.add(|document| image_node(document, &asset_id)),
                    None => self.status = Some(Status::error(assets::NO_PICTURE_MESSAGE)),
                }
            }
            Command::AddGauge => self.add(|document| chart_node(document, ChartFamily::Gauge)),
            Command::AddLineChart => self.add(|document| chart_node(document, ChartFamily::Line)),
            Command::AddBarChart => self.add(|document| chart_node(document, ChartFamily::Bar)),
            Command::AddPieChart => self.add(|document| chart_node(document, ChartFamily::Pie)),
            Command::AddRadarChart => self.add(|document| chart_node(document, ChartFamily::Radar)),
            Command::AddPolarBar => {
                self.add(|document| chart_node(document, ChartFamily::PolarBar))
            }
            Command::DeleteSelected => {
                let ids = self.selection.ids().to_vec();
                if !ids.is_empty() {
                    self.editor.remove_nodes(&ids);
                    self.selection.clear();
                    self.refresh();
                }
            }
            Command::AlignLeft => self.arrange(arrange::Action::AlignLeft),
            Command::AlignCenterX => self.arrange(arrange::Action::AlignCenterX),
            Command::AlignRight => self.arrange(arrange::Action::AlignRight),
            Command::AlignTop => self.arrange(arrange::Action::AlignTop),
            Command::AlignCenterY => self.arrange(arrange::Action::AlignCenterY),
            Command::AlignBottom => self.arrange(arrange::Action::AlignBottom),
            Command::DistributeX => self.arrange(arrange::Action::DistributeX),
            Command::DistributeY => self.arrange(arrange::Action::DistributeY),
            Command::Group => {
                let ids = self.selection.ids().to_vec();
                match self.editor.group(&ids) {
                    Some(id) => {
                        self.selection.replace(id);
                        self.refresh();
                    }
                    None => {
                        self.status = Some(Status::error("Grouping needs two unlocked nodes"));
                    }
                }
            }
            Command::Ungroup => {
                let Some(id) = self.selection.first().map(str::to_owned) else {
                    self.status = Some(Status::error("Select a group to ungroup"));
                    return;
                };
                let freed = self.editor.ungroup(&id);
                if freed.is_empty() {
                    self.status = Some(Status::error("That selection is not a group"));
                } else {
                    self.selection.set(freed);
                    self.refresh();
                }
            }
            Command::Copy => {
                let ids = self.selection.ids().to_vec();
                self.status = Some(if self.editor.copy(&ids) {
                    Status::ok(format!("Copied {}", ids.len()))
                } else {
                    Status::error("Nothing selected to copy")
                });
            }
            Command::Cut => {
                let ids = self.selection.ids().to_vec();
                if self.editor.cut(&ids) {
                    self.selection.clear();
                    self.refresh();
                } else {
                    self.status = Some(Status::error("Nothing selected to cut"));
                }
            }
            Command::Paste => match self.editor.paste() {
                Some(pasted) => {
                    self.selection.set(pasted.ids.clone());
                    self.status = Some(if pasted.missing_assets.is_empty() {
                        Status::ok(format!("Pasted {}", pasted.ids.len()))
                    } else {
                        Status::error(format!(
                            "Pasted, but missing pictures: {}",
                            pasted.missing_assets.join(", ")
                        ))
                    });
                    self.refresh();
                }
                None => self.status = Some(Status::error("Nothing to paste")),
            },
            Command::Duplicate => {
                let ids = self.selection.ids().to_vec();
                match self.editor.duplicate(&ids) {
                    Some(pasted) => {
                        self.selection.set(pasted.ids.clone());
                        self.refresh();
                    }
                    None => self.status = Some(Status::error("Nothing selected to duplicate")),
                }
            }
            Command::OpenDisplay => self.open_display(cx),
            Command::Activate | Command::Publish => self.open_publish(cx),
        }
        cx.notify();
    }

    /// Opens the live window on a display, painting the document as it stands
    /// rather than the last save. The file is a hand-off: the player reads a
    /// theme from disk, so this writes one beside the library.
    fn open_display(&mut self, cx: &mut Context<Self>) {
        let path = self
            .library
            .root()
            .parent()
            .map(|dir| dir.join("live-theme.json"))
            .unwrap_or_else(|| std::env::temp_dir().join("yunari-live-theme.json"));
        let written = serde_json::to_vec(self.editor.document())
            .map_err(|error| error.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|error| error.to_string()));
        match written {
            Ok(()) => {
                yunari_player::open_fullscreen(Some(&path), None, cx);
                self.status = Some(Status::ok("Opened on this PC's display"));
            }
            Err(error) => self.status = Some(Status::error(format!("{path:?}: {error}"))),
        }
    }

    /// Aligns or distributes the selection, saying why when it cannot.
    fn arrange(&mut self, action: arrange::Action) {
        let ids = self.selection.ids().to_vec();
        if ids.len() < action.needed() {
            self.status = Some(Status::error(format!(
                "That needs {} selected nodes",
                action.needed()
            )));
            return;
        }
        let mut moved = false;
        self.editor
            .edit(|document| moved = arrange::apply(document, &ids, action));
        if moved {
            self.refresh();
        } else {
            self.status = Some(Status::error("Nothing moved: a selected node is locked"));
        }
    }

    fn add(&mut self, make: impl FnOnce(&ThemeDocument) -> Node) {
        let node = make(self.editor.document());
        self.selection.set(vec![node.id.clone()]);
        if self.editor.add_node(node) {
            self.refresh();
        }
    }

    fn save(&mut self) {
        match self.editor.save(&self.library, false) {
            Ok(saved) => {
                self.status = Some(Status::ok(format!("Saved {}", saved.entry.id)));
                self.point_images();
                self.refresh();
            }
            Err(LibraryError::Conflict { .. }) => {
                // A document that has never been saved has no id to write to.
                self.overlay = Some(Overlay::SaveAs(Prompt::new(
                    "Save as",
                    "A theme id: lowercase letters, digits and dashes",
                    suggested_id(self.editor.document()),
                )));
            }
            Err(LibraryError::Invalid(issues)) => {
                // The issues name what to fix, so the status line carries them
                // rather than a sentence that hides them.
                let issues = issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                self.status = Some(Status::error(format!("Not saved: {issues}")));
            }
            Err(error) => self.status = Some(Status::error(error.to_string())),
        }
    }

    fn save_as(&mut self, value: &str) {
        let id = value.trim();
        if id.is_empty() {
            self.status = Some(Status::error("A theme needs an id to be saved"));
            return;
        }
        match self.editor.save_as(&self.library, id) {
            Ok(saved) => {
                self.status = Some(Status::ok(format!("Saved {}", saved.entry.id)));
                self.point_images();
                self.refresh();
            }
            Err(error) => self.status = Some(Status::error(error.to_string())),
        }
    }

    /// Copies a stored theme under a fresh id, so a variant can be tried
    /// without touching the original.
    fn duplicate_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.library.duplicate(id) {
            Ok(saved) => self.status = Some(Status::ok(format!("Copied to {}", saved.entry.id))),
            Err(error) => self.status = Some(Status::error(error.to_string())),
        }
        self.refresh_library();
        cx.notify();
    }

    /// Deletes only after the author says so: the folder is recoverable, but
    /// a list that removes things on one click is not a list anyone trusts.
    fn confirm_delete_theme(&mut self, id: String, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Confirm {
            message: format!(
                "Delete the theme {id}? Its folder moves to the library's .trash, so it can be brought back by hand."
            ),
            resume: Resume::DeleteTheme(id),
        });
        cx.notify();
    }

    fn delete_theme(&mut self, id: &str) {
        match self.library.remove(id) {
            Ok(()) => self.status = Some(Status::ok(format!("Deleted {id}"))),
            Err(error) => self.status = Some(Status::error(error.to_string())),
        }
        // Back to the list, now one row shorter.
        self.overlay = Some(Overlay::Library(LibraryDialog::new(&self.library)));
    }

    /// Reloads the list Open is showing after a theme was added or removed.
    fn refresh_library(&mut self) {
        if let Some(Overlay::Library(dialog)) = &mut self.overlay {
            dialog.entries = self.library.list();
        }
        // The Themes mode reads its own copy; dropping it reloads on next draw.
        self.themes = None;
    }

    fn export(&mut self) {
        let path = export_path(self.library.root(), self.editor.document());
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match self
            .editor
            .export()
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|error| error.to_string()))
        {
            Ok(()) => self.status = Some(Status::ok(format!("Exported {}", path.display()))),
            Err(error) => self.status = Some(Status::error(error)),
        }
    }

    /// Writes the selected top-level nodes as a widget package beside the
    /// library, where the host's pack folder can take it.
    fn export_widget(&mut self) {
        let ids = self.selection.ids().to_vec();
        if ids.is_empty() {
            self.status = Some(Status::error("Select the nodes to export first"));
            return;
        }
        let path = widget_path(self.library.root(), self.editor.document());
        match crate::widgets::selection_package(&self.editor, &ids)
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|error| error.to_string()))
        {
            Ok(()) => {
                self.status = Some(Status::ok(format!("Exported {}", path.display())));
            }
            Err(error) => self.status = Some(Status::error(error)),
        }
    }

    /// The path typed into Import, read and stored under a free id.
    fn import(&mut self, value: &str, cx: &mut Context<Self>) {
        let path = value.trim();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = Some(Status::error(format!("{path}: {error}")));
                return;
            }
        };
        match import_package(&self.library, &bytes) {
            Ok(id) => self.open_entry(&id, cx),
            Err(error) => self.status = Some(Status::error(error)),
        }
    }

    /// Where the host keeps the widget packs this PC has installed.
    /// Turns one monitor into a display record, or takes the record away. The
    /// app's supervisor opens and closes the window; this only edits the state.
    fn toggle_monitor(&mut self, uuid: &str, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            if let Some(Overlay::Settings(dialog)) = &mut self.overlay {
                dialog.message = Some("This window is not attached to a host.".to_owned());
            }
            cx.notify();
            return;
        };
        let Ok(mut displays) = host.displays.lock() else {
            return;
        };
        let held = displays
            .list()
            .iter()
            .find(|record| {
                record.kind == yunari_host::displays::DisplayKind::Local
                    && record.monitor.as_deref() == Some(uuid)
            })
            .map(|record| record.id.clone());
        let message = match held {
            Some(id) => {
                displays.remove(&id);
                "That monitor no longer shows a theme.".to_owned()
            }
            None => {
                let label = self
                    .overlay_settings_monitor_label(uuid)
                    .unwrap_or_else(|| "Monitor".to_owned());
                match displays.adopt_local(&label, Some(uuid.to_owned()), None, None) {
                    Ok(record) => format!("That monitor shows theme {}.", record.label),
                    Err(refusal) => refusal.0,
                }
            }
        };
        drop(displays);
        let monitors = settings_dialog::monitor_rows(Some(&host), cx);
        let displays = settings_dialog::display_rows(Some(&host));
        if let Some(Overlay::Settings(dialog)) = &mut self.overlay {
            dialog.monitors = monitors;
            dialog.displays = displays;
            dialog.message = Some(message);
        }
        cx.notify();
    }

    /// Switches a local display record's shape, or turns it a step. The
    /// rotation stays one the shape accepts: a rectangle turns in quarter
    /// steps, a circle in fifteen-degree steps.
    fn change_presentation(&mut self, id: &str, shape: bool, cx: &mut Context<Self>) {
        use yunari_host::displays::DisplayShape;
        let Some(host) = self.host.clone() else {
            return;
        };
        let Ok(mut displays) = host.displays.lock() else {
            return;
        };
        let Some(record) = displays.get(id) else {
            return;
        };
        let current = record.shape.unwrap_or(DisplayShape::Rect);
        let rotation = record.rotation.unwrap_or(0);
        let (shape, rotation) = settings_dialog::next_presentation(current, rotation, shape);
        let message = match displays.patch(id, None, Some(shape), Some(rotation), None) {
            Ok(record) => format!(
                "{}: {}°, {}.",
                record.label,
                rotation,
                match shape {
                    DisplayShape::Rect => "rectangle",
                    DisplayShape::Circle => "circle",
                }
            ),
            Err(refusal) => refusal.0,
        };
        drop(displays);
        let monitors = settings_dialog::monitor_rows(Some(&host), cx);
        let displays = settings_dialog::display_rows(Some(&host));
        if let Some(Overlay::Settings(dialog)) = &mut self.overlay {
            dialog.monitors = monitors;
            dialog.displays = displays;
            dialog.message = Some(message);
        }
        cx.notify();
    }

    /// Switches a panel's frame layout to the next one it accepts. The store
    /// checks the choice against the panel's own family before storing it.
    fn change_render_mode(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let Ok(mut displays) = host.displays.lock() else {
            return;
        };
        let Some(record) = displays.get(&id) else {
            return;
        };
        let modes = record
            .device
            .as_ref()
            .and_then(|device| yunari_panels::for_device(device.vendor_id, device.product_id))
            .map(|family| family.render_modes)
            .unwrap_or(&[]);
        let Some(mode) = settings_dialog::next_render_mode(modes, record.render_mode) else {
            return;
        };
        let message = match displays.patch(&id, None, None, None, Some(mode)) {
            Ok(record) => format!("{}: {}.", record.label, settings_dialog::mode_text(mode)),
            Err(refusal) => refusal.0,
        };
        drop(displays);
        let monitors = settings_dialog::monitor_rows(Some(&host), cx);
        let displays = settings_dialog::display_rows(Some(&host));
        if let Some(Overlay::Settings(dialog)) = &mut self.overlay {
            dialog.monitors = monitors;
            dialog.displays = displays;
            dialog.message = Some(message);
        }
        cx.notify();
    }

    /// The label the settings row shows for a monitor uuid.
    fn overlay_settings_monitor_label(&self, uuid: &str) -> Option<String> {
        let Overlay::Settings(dialog) = self.overlay.as_ref()? else {
            return None;
        };
        dialog
            .monitors
            .iter()
            .find(|monitor| monitor.uuid == uuid)
            .map(|monitor| monitor.label.clone())
    }

    /// Where this PC's host keeps its settings, which the app shares.
    fn settings_dir(&self) -> PathBuf {
        yunari_host::config::HostConfig::resolve(None, None, false).settings_dir()
    }

    /// Flips one switch and keeps the dialog showing what the file now says.
    fn flip_setting(&mut self, id: &str, cx: &mut Context<Self>) {
        let settings = self.settings_dir();
        let Some(Overlay::Settings(dialog)) = &mut self.overlay else {
            return;
        };
        let message = match id {
            "settings-lan" => {
                dialog.lan = settings_dialog::set_lan(&settings, !dialog.lan);
                if dialog.lan {
                    "Phones can pair from this network within a second.".to_owned()
                } else {
                    "This PC only; a phone already paired loses access.".to_owned()
                }
            }
            "settings-autostart" => match settings_dialog::set_autostart(!dialog.autostart) {
                Ok(on) => {
                    dialog.autostart = on;
                    format!(
                        "Start with Windows is now {}.",
                        if on { "on" } else { "off" }
                    )
                }
                Err(error) => error,
            },
            _ => return,
        };
        dialog.message = Some(message);
        cx.notify();
    }

    /// Cycles the weather backend and keeps the key state beside it.
    fn cycle_weather_backend(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings_dir();
        let Some(Overlay::Settings(dialog)) = &mut self.overlay else {
            return;
        };
        let index = settings_dialog::WEATHER_BACKENDS
            .iter()
            .position(|id| *id == dialog.weather.provider)
            .unwrap_or(0);
        dialog.weather.provider = settings_dialog::WEATHER_BACKENDS
            [(index + 1) % settings_dialog::WEATHER_BACKENDS.len()]
        .to_owned();
        dialog.weather_key_set = settings_dialog::read_weather_key_set(&settings, &dialog.weather);
        dialog.message = Some(
            match settings_dialog::write_weather(&settings, &dialog.weather) {
                Ok(()) => format!("Weather backend is now {}.", dialog.weather.provider),
                Err(error) => error,
            },
        );
        cx.notify();
    }

    /// Turns the Windows location fallback on or off.
    fn toggle_weather_location(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings_dir();
        let Some(Overlay::Settings(dialog)) = &mut self.overlay else {
            return;
        };
        dialog.weather.use_windows_location = !dialog.weather.use_windows_location;
        dialog.message = Some(
            match settings_dialog::write_weather(&settings, &dialog.weather) {
                Ok(()) if dialog.weather.use_windows_location => {
                    "Windows location is on; it asks Windows itself for a fix.".to_owned()
                }
                Ok(()) => "Windows location is off; a place must be set.".to_owned(),
                Err(error) => error,
            },
        );
        cx.notify();
    }

    /// Asks for the SignalRGB receiver's UDP port.
    fn edit_bridge_port(&mut self) {
        let Some(Overlay::Settings(dialog)) = self.overlay.take() else {
            return;
        };
        self.overlay = Some(Overlay::BridgePort {
            prompt: Prompt::new(
                "SignalRGB port",
                "The UDP port the receiver binds, from 1 to 65535",
                dialog.bridge_port.to_string(),
            ),
            dialog,
        });
    }

    fn save_bridge_port(&mut self, mut dialog: SettingsDialog, value: &str) {
        let settings = self.settings_dir();
        dialog.message = Some(match settings_dialog::set_bridge_port(&settings, value) {
            Ok(port) => {
                dialog.bridge_port = port;
                format!("SignalRGB port {port} is saved; it applies on the next host start.")
            }
            Err(message) => message,
        });
        self.overlay = Some(Overlay::Settings(Box::new(dialog)));
    }

    /// Asks for a place as `label|latitude|longitude`.
    fn edit_weather_place(&mut self) {
        let Some(Overlay::Settings(dialog)) = self.overlay.take() else {
            return;
        };
        let value = format!(
            "{}|{}|{}",
            dialog.weather.label, dialog.weather.latitude, dialog.weather.longitude
        );
        self.overlay = Some(Overlay::WeatherPlace {
            dialog,
            prompt: Prompt::new(
                "Weather place",
                "Label|latitude|longitude, e.g. Berlin|52.52|13.405",
                value,
            ),
        });
    }

    fn save_weather_place(&mut self, mut dialog: SettingsDialog, value: &str) {
        let settings = self.settings_dir();
        let parts: Vec<&str> = value.split('|').map(str::trim).collect();
        let parsed = match parts.as_slice() {
            [label, latitude, longitude] if !label.is_empty() => {
                match (latitude.parse::<f64>(), longitude.parse::<f64>()) {
                    (Ok(latitude), Ok(longitude))
                        if (-90.0..=90.0).contains(&latitude)
                            && (-180.0..=180.0).contains(&longitude) =>
                    {
                        Some(((*label).to_owned(), latitude, longitude))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        dialog.message = Some(match parsed {
            Some((label, latitude, longitude)) => {
                dialog.weather.label = label;
                dialog.weather.latitude = latitude;
                dialog.weather.longitude = longitude;
                match settings_dialog::write_weather(&settings, &dialog.weather) {
                    Ok(()) => "The weather place is saved.".to_owned(),
                    Err(error) => error,
                }
            }
            None => "A place needs Label|latitude|longitude, with the numbers in range.".to_owned(),
        });
        self.overlay = Some(Overlay::Settings(Box::new(dialog)));
    }

    /// Cycles a disk slot through "not assigned" and each drive this PC
    /// reports, writing the choice where the providers read it.
    pub fn cycle_device_slot(&mut self, slot: &'static str, cx: &mut Context<Self>) {
        let Some(Overlay::Settings(mut dialog)) = self.overlay.take() else {
            return;
        };
        let settings = self.settings_dir();
        let current = match slot {
            "gpu" => dialog.devices.gpu.clone(),
            "system-disk" => dialog.devices.system_disk.clone(),
            _ => dialog.devices.data_disk.clone(),
        };
        let mut options: Vec<Option<String>> = vec![None];
        options.extend(
            dialog
                .options
                .iter()
                .filter(|device| device.group == slot)
                .map(|device| Some(device.id.clone())),
        );
        let next = match current.as_ref().and_then(|id| {
            options
                .iter()
                .position(|option| option.as_deref() == Some(id.as_str()))
        }) {
            Some(index) => options[(index + 1) % options.len()].clone(),
            None => options[1 % options.len()].clone(),
        };
        dialog.message = Some(
            match settings_dialog::set_device_slot(&settings, slot, next.as_deref()) {
                Ok(devices) => {
                    dialog.devices = devices;
                    "The device choice is saved.".to_owned()
                }
                Err(error) => error,
            },
        );
        self.overlay = Some(Overlay::Settings(dialog));
        cx.notify();
    }

    /// Asks for the selected backend's key; it is stored under the OS protector
    /// and never shown again.
    fn edit_weather_key(&mut self) {
        let Some(Overlay::Settings(dialog)) = self.overlay.take() else {
            return;
        };
        let Some(id) = settings_dialog::weather_key_id(&dialog.weather.provider) else {
            return;
        };
        self.overlay = Some(Overlay::Credential {
            dialog,
            id: id.to_owned(),
            prompt: Prompt::new("Weather key", "The key this backend issued to you", ""),
        });
    }

    /// Asks for a credential a custom source named in its `auth` block.
    pub fn edit_credential(&mut self, id: String) {
        let Some(Overlay::Settings(dialog)) = self.overlay.take() else {
            return;
        };
        self.overlay = Some(Overlay::Credential {
            dialog,
            prompt: Prompt::new(
                format!("Credential {id}"),
                "The value this source's auth expects",
                "",
            ),
            id,
        });
    }

    fn save_credential(&mut self, mut dialog: SettingsDialog, id: &str, value: &str) {
        let settings = self.settings_dir();
        dialog.message = Some(
            match settings_dialog::set_credential(&settings, id, value) {
                Ok(()) if value.trim().is_empty() => format!("The credential {id} is forgotten."),
                Ok(()) => format!("The credential {id} is stored for this Windows account."),
                Err(error) => error,
            },
        );
        dialog.weather_key_set = settings_dialog::read_weather_key_set(&settings, &dialog.weather);
        dialog.source_credentials = settings_dialog::read_source_credentials(&settings);
        self.overlay = Some(Overlay::Settings(Box::new(dialog)));
    }

    /// Asks for a stored display's new label.
    fn edit_display_name(&mut self, id: String) {
        let Some(Overlay::Settings(dialog)) = self.overlay.take() else {
            return;
        };
        let current = dialog
            .displays
            .iter()
            .find(|display| display.id == id)
            .map(|display| display.label.clone())
            .unwrap_or_default();
        self.overlay = Some(Overlay::DisplayName {
            dialog,
            prompt: Prompt::new("Display name", "What this display is called", &current),
            id,
        });
    }

    /// Stores a display's label. The store trims it and refuses an empty one,
    /// so the row shows the sentence the store answered rather than a guess.
    fn save_display_name(
        &mut self,
        mut dialog: SettingsDialog,
        id: &str,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        let message = match self.host.clone() {
            Some(host) => match host.displays.lock() {
                Ok(mut displays) => match displays.patch(id, Some(value), None, None, None) {
                    Ok(record) => format!("Renamed to {}.", record.label),
                    Err(refusal) => refusal.0,
                },
                Err(_) => "The display list is busy; try again.".to_owned(),
            },
            None => "No host is running, so the label was not saved.".to_owned(),
        };
        dialog.message = Some(message);
        dialog.monitors = settings_dialog::monitor_rows(self.host.as_ref(), cx);
        dialog.displays = settings_dialog::display_rows(self.host.as_ref());
        self.overlay = Some(Overlay::Settings(Box::new(dialog)));
    }

    fn packs_directory(&self) -> PathBuf {
        self.library
            .root()
            .parent()
            .map(|app_dir| yunari_packs::directory(&app_dir.join("settings")))
            .unwrap_or_else(|| yunari_packs::directory(Path::new("settings")))
    }

    /// Inserts one installed unit into the open theme.
    fn insert_widget(&mut self, unit: yunari_packs::Unit, cx: &mut Context<Self>) {
        self.overlay = None;
        let label = unit.label.clone();
        match crate::widgets::insert(&mut self.editor, &unit, crate::clipboard::PASTE_OFFSET) {
            Ok(id) => {
                self.selection.set(vec![id]);
                self.point_images();
                self.status = Some(Status::ok(format!("Inserted {label}")));
                self.refresh();
            }
            Err(message) => self.status = Some(Status::error(message)),
        }
        cx.notify();
    }

    /// Inserts one stored path preset with the geometry its data measures.
    fn insert_path_preset(
        &mut self,
        preset: yunari_host::path_presets::Preset,
        cx: &mut Context<Self>,
    ) {
        self.overlay = None;
        let Some((width, height, paint)) = yunari_host::path_presets::measure(&preset.data) else {
            self.status = Some(Status::error("That preset draws nothing."));
            cx.notify();
            return;
        };
        let label = preset.label.clone();
        self.add(move |document| {
            let mut node = path_with(
                document,
                "path",
                &preset.data,
                width.max(1.0),
                height.max(1.0),
                paint == yunari_host::path_presets::Paint::Fill,
            );
            node.name = Some(label);
            node
        });
        self.status = Some(Status::ok("Inserted a shape preset"));
        cx.notify();
    }

    fn apply_typography(&mut self, trio: crate::typography::Trio, cx: &mut Context<Self>) {
        self.overlay = None;
        self.editor.apply_typography(&trio);
        self.refresh();
        self.status = Some(Status::ok(format!("Applied {}", trio.name)));
        cx.notify();
    }

    /// Takes a file from this PC into the open theme. The bytes are held until
    /// the next save, so an imported picture draws before the theme is stored.
    /// A `.mpkg` is a mobile Wallpaper Engine export: its video is extracted and
    /// normalized, then becomes the theme's background.
    fn import_asset(&mut self, value: &str) {
        self.overlay = None;
        let path = value.trim();
        if path.to_ascii_lowercase().ends_with(".mpkg") {
            self.import_package(path);
            return;
        }
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = Some(Status::error(format!(
                    "{}{path}: {error}",
                    assets::UNREADABLE_MESSAGE
                )));
                return;
            }
        };
        let name = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path);
        match self.editor.import_asset(name, bytes) {
            Ok(id) => {
                self.point_images();
                self.status = Some(Status::ok(format!("Imported {id}")));
                self.refresh();
            }
            Err(message) => self.status = Some(Status::error(message)),
        }
    }

    /// One picked mobile package: the host's own unpacking and normalization,
    /// then the same import and background assignment a video makes.
    fn import_package(&mut self, path: &str) {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.status = Some(Status::error(format!(
                    "{}{path}: {error}",
                    assets::UNREADABLE_MESSAGE
                )));
                return;
            }
        };
        let Some(package) = yunari_host::wallpaper_engine::mobile_package(&bytes) else {
            self.status = Some(Status::error(
                "That file is not a Wallpaper Engine mobile package with a video.",
            ));
            return;
        };
        let normalized = match yunari_host::normalize_video::normalize(&package.video) {
            Ok(normalized) => normalized,
            Err(message) => {
                self.status = Some(Status::error(message));
                return;
            }
        };
        let title = package.title.unwrap_or_else(|| {
            Path::new(path)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("wallpaper")
                .to_owned()
        });
        let credit = crate::assets::ImportCredit {
            kind: "wallpaper".to_owned(),
            title,
            license: "unknown".to_owned(),
            source_url: package
                .workshop_id
                .map(|id| format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}")),
            author: None,
        };
        match self
            .editor
            .import_asset_with("wallpaper.mp4", normalized.bytes, Some(credit))
        {
            Ok(id) => {
                // A video import is a background: the same consequence every
                // video import the source makes has.
                let mut placed = None;
                self.editor.edit(|document| {
                    placed = inspector::apply_document(document, "background.media", &id).err();
                });
                match placed {
                    None => self.status = Some(Status::ok(format!("Imported {id}"))),
                    Some(message) => self.status = Some(Status::error(message)),
                }
                self.point_images();
                self.refresh();
            }
            Err(message) => self.status = Some(Status::error(message)),
        }
    }

    /// Places the picture an asset-finder row names, through the editor's own
    /// import so the same sniffing and SVG refusal apply.
    /// Asks the picture sources off the UI thread: a slow catalogue must not
    /// freeze the window, and the answer lands in the finder only if it is
    /// still open.
    fn search_stock_assets(&mut self, query: String, cx: &mut Context<Self>) {
        let mut tab = asset_finder::FinderTab::Icons;
        if let Some(Overlay::AssetFinder(dialog)) = &mut self.overlay {
            tab = dialog.tab;
            dialog.busy = true;
            dialog.note = None;
        }
        let credential = self
            .host
            .as_ref()
            .and_then(|host| host.credentials.get("unsplash"));
        cx.spawn(async move |this, cx| {
            let (rows, note) = cx
                .background_executor()
                .spawn(async move {
                    asset_finder::search_for(tab, &query, credential.as_deref())
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(Overlay::AssetFinder(dialog)) = &mut this.overlay {
                    dialog.busy = false;
                    dialog.searched = true;
                    dialog.remote = rows;
                    dialog.note = note;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn place_asset(&mut self, choice: &asset_finder::Choice, cx: &mut Context<Self>) {
        self.overlay = None;
        if let asset_finder::Pick::Project(id) = &choice.pick {
            self.status = Some(match yunari_host::wallpaper_engine::project(id) {
                answer if answer["ok"] == true => {
                    let name = answer["name"].as_str().unwrap_or(id);
                    if answer["reused"] == true {
                        Status::ok(format!("Already unpacked: {name}"))
                    } else {
                        Status::ok(format!("Unpacked {name}; open it in Wallpaper Engine."))
                    }
                }
                answer if answer["reason"] == "no-install" => {
                    Status::error("Wallpaper Engine is not installed on this PC.")
                }
                _ => Status::error("That wallpaper's scene could not be unpacked."),
            });
            cx.notify();
            return;
        }
        let credential = self
            .host
            .as_ref()
            .and_then(|host| host.credentials.get("unsplash"));
        let credit = match &choice.pick {
            asset_finder::Pick::Stock { credit, .. } => Some(credit.clone()),
            _ => None,
        };
        match asset_finder::bytes_of(&choice.pick, credential.as_deref()) {
            Ok((name, bytes)) => match self.editor.import_asset_with(&name, bytes, credit) {
                Ok(id) => {
                    self.point_images();
                    self.status = Some(Status::ok(format!("Imported {id}")));
                    self.refresh();
                }
                Err(message) => self.status = Some(Status::error(message)),
            },
            Err(message) => self.status = Some(Status::error(message)),
        }
        cx.notify();
    }

    /// The settings dialog, when one is open; the mode's first page reuses it.
    pub(super) fn settings_mode_dialog(&self) -> Option<&settings_dialog::SettingsDialog> {
        match &self.overlay {
            Some(Overlay::Settings(dialog)) => Some(dialog),
            _ => None,
        }
    }

    /// Opens the recent list at the control that asked for it.
    fn show_recent(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Recent { at });
        cx.notify();
    }

    /// Opens a stored theme, its pictures and all, replacing the open one.
    fn open_entry(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.editor.open_stored(&self.library, id) {
            Ok(()) => {
                self.selection.clear();
                self.overlay = None;
                self.camera_touched = false;
                self.status = Some(Status::ok(format!("Opened {id}")));
                let name = self
                    .editor
                    .document()
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| id.to_owned());
                note_recent(&mut self.recents, id, name);
                save_recents(&self.library, &self.recents);
                self.point_images();
                self.refresh();
            }
            Err(error) => {
                self.overlay = None;
                self.status = Some(Status::error(error.to_string()));
            }
        }
        cx.notify();
    }

    /// Runs an action that throws the open document away, or asks first.
    fn guard(&mut self, resume: Resume, cx: &mut Context<Self>) {
        if self.editor.is_dirty() {
            self.overlay = Some(Overlay::Confirm {
                message: "This theme has unsaved changes. Discard them?".to_owned(),
                resume,
            });
            cx.notify();
            return;
        }
        self.resume(resume, cx);
    }

    fn resume(&mut self, resume: Resume, cx: &mut Context<Self>) {
        match resume {
            Resume::New | Resume::Template => {
                let document = if resume == Resume::Template {
                    starter()
                } else {
                    yunari_model::presets::blank()
                };
                self.editor = Editor::new(document);
                self.selection.clear();
                self.overlay = None;
                self.camera_touched = false;
                self.status = None;
                self.point_images();
                self.refresh();
            }
            Resume::Library => {
                self.overlay = Some(Overlay::Library(LibraryDialog::new(&self.library)));
            }
            Resume::Import(path) => {
                self.overlay = None;
                self.import(&path, cx);
            }
            Resume::DeleteTheme(id) => {
                self.delete_theme(&id);
            }
            Resume::Quit => {}
        }
        cx.notify();
    }

    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
        cx.notify();
    }

    /// One box per selected node, in artboard units, for the outlines.
    fn selection_rects(&self) -> Vec<(f64, f64, f64, f64)> {
        let document = self.editor.document();
        self.selection
            .ids()
            .iter()
            .filter_map(|id| {
                let transform = selection::find(document, id)?.transform;
                Some((
                    transform.x,
                    transform.y,
                    transform.width * transform.scale_x,
                    transform.height * transform.scale_y,
                ))
            })
            .collect()
    }

    fn select_all(&mut self) {
        let ids = self
            .editor
            .document()
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        self.selection.set(ids);
    }

    fn artboard(&self) -> (f64, f64) {
        (
            self.editor.document().artboard.width,
            self.editor.document().artboard.height,
        )
    }

    /// The primary selection's box and rotation, for its control chrome.
    fn primary_rect(&self) -> Option<(String, Rect, f64)> {
        let id = self.selection.first()?.to_owned();
        let node = selection::find(self.editor.document(), &id)?;
        let transform = node.transform;
        Some((
            id,
            (
                transform.x,
                transform.y,
                transform.width * transform.scale_x,
                transform.height * transform.scale_y,
            ),
            transform.rotation,
        ))
    }

    /// Right-press over the canvas. The source selects the object under the
    /// pointer before opening the menu; an empty press leaves the selection
    /// alone and offers what can be inserted.
    fn canvas_context(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        let local = point(
            event.position.x - bounds.origin.x,
            event.position.y - bounds.origin.y,
        );
        let (x, y) = self.viewport.to_scene(local);
        let object = selection::hit(self.editor.document(), x, y);
        if let Some(id) = object.clone() {
            self.selection.replace(id);
        }
        self.overlay = Some(Overlay::ContextMenu {
            at: event.position,
            object,
        });
        cx.notify();
    }

    /// The canvas menu from the keyboard: where the platform's own menu key
    /// would put it, at the selection's bottom-left corner.
    fn context_from_keyboard(&mut self, cx: &mut Context<Self>) {
        let Some((id, rect, _)) = self.primary_rect() else {
            return;
        };
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        let at = point(
            bounds.origin.x + px(self.viewport.offset.0 + rect.0 as f32 * self.viewport.scale),
            bounds.origin.y + px(self.viewport.offset.1 + rect.3 as f32 * self.viewport.scale),
        );
        self.selection.replace(id.clone());
        self.overlay = Some(Overlay::ContextMenu {
            at,
            object: Some(id),
        });
        cx.notify();
    }

    /// The transforms a move or nudge may change: the unlocked selected nodes.
    fn start_transforms(&self) -> Vec<(String, Transform)> {
        let document = self.editor.document();
        self.selection
            .ids()
            .iter()
            .filter_map(|id| {
                let node = selection::find(document, id)?;
                (!node.locked).then(|| (id.clone(), node.transform))
            })
            .collect()
    }

    fn begin_handle(
        &mut self,
        id: String,
        rect: (f64, f64, f64, f64),
        handle: Handle,
        pointer: (f64, f64),
    ) {
        let centre = (rect.0 + rect.2 / 2.0, rect.1 + rect.3 / 2.0);
        let Some(node) = selection::find(self.editor.document(), &id).map(|node| node.transform)
        else {
            return;
        };
        self.editor.begin_burst();
        self.gesture = Some(match handle {
            Handle::Rotate => Gesture::Rotate {
                id,
                start: node,
                angle: transform::rotate(centre, pointer),
                centre,
            },
            handle => Gesture::Resize {
                id,
                handle,
                start: node,
                rect,
            },
        });
    }

    /// A press on the canvas: a handle starts a transform, a node joins or
    /// replaces the selection and may be dragged, and empty canvas starts a
    /// marquee.
    /// Records the gaps between the first two selected nodes. A two-object
    /// selection carries its authored spacing through a drag until the pointer
    /// pulls it outside the source's release window.
    fn capture_internal_spacing(&mut self) {
        self.internal_spacing.clear();
        let starts = self.start_transforms();
        let Some((first_id, first)) = starts.first() else {
            return;
        };
        let Some((second_id, second)) = starts.get(1) else {
            return;
        };
        let gap_x = second.x - first.x;
        let gap_y = second.y - first.y;
        if gap_x > 0.0 {
            self.internal_spacing.push((
                snap::Axis::X,
                first_id.clone(),
                second_id.clone(),
                gap_x,
            ));
        }
        if gap_y > 0.0 {
            self.internal_spacing.push((
                snap::Axis::Y,
                first_id.clone(),
                second_id.clone(),
                gap_y,
            ));
        }
    }

    fn canvas_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.spacing_hold = None;
        self.internal_spacing.clear();
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        let local = point(
            event.position.x - bounds.origin.x,
            event.position.y - bounds.origin.y,
        );
        let (x, y) = self.viewport.to_scene(local);
        let shift = event.modifiers.shift;
        if let Some((id, rect, rotation)) = self.primary_rect()
            && let Some(handle) = transform::handle_at(rect, rotation, x, y, self.viewport.scale)
            // Resizing a turned box would need the pointer in its own space;
            // rotation is affordable, so only the turn handle shows when turned.
            && (handle == Handle::Rotate || rotation.abs() < 1e-6)
        {
            self.marquee = None;
            self.begin_handle(id, rect, handle, (x, y));
            cx.notify();
            return;
        }
        match selection::hit(self.editor.document(), x, y) {
            Some(id) => {
                self.marquee = None;
                if shift {
                    self.selection.toggle(&id);
                } else if !self.selection.contains(&id) {
                    self.selection.replace(id.clone());
                }
                let locked =
                    selection::find(self.editor.document(), &id).is_some_and(|node| node.locked);
                if !shift && !locked {
                    let starts = self.start_transforms();
                    if let Some(anchor) = starts
                        .iter()
                        .find(|(held, _)| *held == id)
                        .map(|(_, transform)| (transform.x, transform.y))
                    {
                        self.capture_internal_spacing();
                        self.editor.begin_burst();
                        self.gesture = Some(Gesture::Move {
                            start: (x, y),
                            anchor,
                            starts,
                        });
                    }
                }
            }
            None => {
                if !shift {
                    self.selection.clear();
                }
                let anchor = (f32::from(local.x), f32::from(local.y));
                self.marquee = Some((anchor, anchor));
            }
        }
        cx.notify();
    }

    /// One step of the gesture in flight, through the editor so the whole drag
    /// stays one history entry.
    fn gesture_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        let local = point(
            event.position.x - bounds.origin.x,
            event.position.y - bounds.origin.y,
        );
        let scene = self.viewport.to_scene(local);
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        let mut guides = Vec::new();
        let changes: Vec<(String, Transform)> = match &gesture {
            Gesture::Move {
                start,
                anchor,
                starts,
            } => {
                let raw = (anchor.0 + scene.0 - start.0, anchor.1 + scene.1 - start.1);
                let enabled = !event.modifiers.control;
                let moving: Vec<String> = starts.iter().map(|(id, _)| id.clone()).collect();
                let (mut snapped_x, mut snapped_y, mut found) = snap::snap_move(
                    self.editor.document(),
                    &moving,
                    raw,
                    self.viewport.scale,
                    enabled,
                );
                // A spacing acquisition is held through the wider release
                // window (5px + 5px at this zoom). Beyond it the pointer owns
                // the object again and a fresh acquisition can land elsewhere.
                let release = 10.0 / self.viewport.scale.max(f32::MIN_POSITIVE);
                let mut held_guides = Vec::new();
                let mut still_held = Vec::new();
                if let Some(hold) = self.spacing_hold.take() {
                    let (axis, target) = hold;
                    let raw_axis = match axis { snap::Axis::X => raw.0, snap::Axis::Y => raw.1 };
                    if enabled && (raw_axis - target).abs() <= f64::from(release) {
                        match axis {
                            snap::Axis::X => snapped_x = target,
                            snap::Axis::Y => snapped_y = target,
                        }
                        let board = &self.editor.document().artboard;
                        held_guides.push(snap::Guide {
                            axis,
                            at: target,
                            from: 0.0,
                            to: match axis { snap::Axis::X => board.height, snap::Axis::Y => board.width },
                            spacing: true,
                        });
                        still_held.push(hold);
                    }
                }
                if still_held.is_empty() {
                    self.spacing_hold = found
                        .iter()
                        .filter(|guide| guide.spacing)
                        .map(|guide| (guide.axis, match guide.axis {
                            snap::Axis::X => snapped_x,
                            snap::Axis::Y => snapped_y,
                        }))
                        .next();
                } else {
                    self.spacing_hold = still_held.into_iter().next();
                    found.clear();
                }
                found.extend(held_guides);
                let (rounded_x, rounded_y) = transform::move_to(raw, raw, enabled);
                let guided = |axis| found.iter().any(|guide| guide.axis == axis);
                let x = if guided(snap::Axis::X) {
                    snapped_x
                } else {
                    rounded_x
                };
                let y = if guided(snap::Axis::Y) {
                    snapped_y
                } else {
                    rounded_y
                };
                guides = found;
                let (dx, dy) = (x - anchor.0, y - anchor.1);
                let mut moved: Vec<(String, Transform)> = starts
                    .iter()
                    .map(|(id, transform)| {
                        let mut next = *transform;
                        next.x += dx;
                        next.y += dy;
                        (id.clone(), next)
                    })
                    .collect();
                let release = f64::from(10.0 / self.viewport.scale.max(f32::MIN_POSITIVE));
                let mut kept = Vec::new();
                for hold in self.internal_spacing.drain(..) {
                    let (axis, first_id, second_id, gap) = hold;
                    let Some(first_pos) = moved
                        .iter()
                        .find(|(id, _)| *id == first_id)
                        .map(|(_, transform)| match axis {
                            snap::Axis::X => transform.x,
                            snap::Axis::Y => transform.y,
                        })
                    else {
                        continue;
                    };
                    let Some(second) = moved.iter_mut().find(|(id, _)| *id == second_id) else {
                        continue;
                    };
                    let current = match axis { snap::Axis::X => second.1.x, snap::Axis::Y => second.1.y };
                    let wanted = first_pos + gap;
                    let drift = current - wanted;
                    if enabled && drift.abs() <= release {
                        match axis {
                            snap::Axis::X => second.1.x = wanted,
                            snap::Axis::Y => second.1.y = wanted,
                        }
                        let board = &self.editor.document().artboard;
                        guides.push(snap::Guide {
                            axis,
                            at: (first_pos + wanted) / 2.0,
                            from: 0.0,
                            to: match axis { snap::Axis::X => board.height, snap::Axis::Y => board.width },
                            spacing: true,
                        });
                        kept.push((axis, first_id.clone(), second_id.clone(), gap));
                    }
                }
                self.internal_spacing = kept;
                moved
            }
            Gesture::Resize {
                id,
                handle,
                start,
                rect,
            } => {
                let uniform = !event.modifiers.shift;
                let from_centre = event.modifiers.alt;
                let next = transform::resize(*rect, *handle, scene, uniform, from_centre);
                let mut transform = *start;
                transform.x = next.0;
                transform.y = next.1;
                transform.width = (next.2 / start.scale_x.max(1e-6)).max(1.0);
                transform.height = (next.3 / start.scale_y.max(1e-6)).max(1.0);
                vec![(id.clone(), transform)]
            }
            Gesture::Rotate {
                id,
                start,
                angle,
                centre,
            } => {
                let now = transform::rotate(*centre, scene);
                let mut transform = *start;
                transform.rotation = (start.rotation + now - angle).rem_euclid(360.0);
                vec![(id.clone(), transform)]
            }
        };
        if self.editor.apply_transforms(&changes) {
            self.refresh();
        }
        self.guides = guides;
        self.gesture = Some(gesture);
        cx.notify();
    }

    /// A click on a layers row: Ctrl toggles, Shift extends, a second click on
    /// the only selected row renames it, and a plain click selects.
    fn layer_click(&mut self, id: &str, modifiers: Modifiers, cx: &mut Context<Self>) {
        self.layer_focus = Some(id.to_owned());
        if modifiers.control || modifiers.platform {
            self.selection.toggle(id);
        } else if modifiers.shift {
            if !self.selection.contains(id) {
                self.selection.toggle(id);
            }
        } else if self.selection.len() == 1 && self.selection.contains(id) {
            let name = selection::find(self.editor.document(), id)
                .and_then(|node| node.name.clone())
                .unwrap_or_else(|| {
                    selection::find(self.editor.document(), id)
                        .map_or_else(String::new, |node| layers::kind_name(node).to_owned())
                });
            self.renaming = Some(id.to_owned());
            self.overlay = Some(Overlay::Rename(Prompt::new(
                "Rename layer",
                "A name; empty clears it",
                name,
            )));
        } else {
            self.selection.replace(id);
        }
        cx.notify();
    }

    /// Keyboard path through the Layers pane. Arrow keys walk the flattened
    /// rows, a collapsed group opens on Right, an open one steps into it, and
    /// Left closes it or steps out to its parent. Enter/F2 renames the row.
    fn navigate_layers(&mut self, key: &str, shift: bool, cx: &mut Context<Self>) {
        let Some(focused) = self.layer_focus.clone() else {
            return;
        };
        let rows = layers::rows(self.editor.document(), &self.selection, &self.expanded);
        let Some(at) = rows.iter().position(|row| row.id == focused) else {
            self.layer_focus = None;
            return;
        };
        let target = match key {
            "arrowdown" => Some((at + 1).min(rows.len() - 1)),
            "arrowup" => Some(at.saturating_sub(1)),
            "home" => Some(0),
            "end" => Some(rows.len() - 1),
            _ => None,
        };
        if let Some(index) = target {
            let id = rows[index].id.clone();
            self.layer_focus = Some(id.clone());
            if shift {
                self.selection.toggle(&id);
            } else {
                self.selection.replace(id);
            }
            cx.notify();
            return;
        }
        let row = &rows[at];
        match key {
            "arrowright" => {
                if row.has_children && !self.expanded.contains(&row.id) {
                    self.toggle_expanded(&row.id, cx);
                } else if row.has_children && at + 1 < rows.len() {
                    self.layer_focus = Some(rows[at + 1].id.clone());
                    self.selection.replace(rows[at + 1].id.clone());
                    cx.notify();
                }
            }
            "arrowleft" => {
                if row.has_children && self.expanded.contains(&row.id) {
                    self.toggle_expanded(&row.id, cx);
                } else {
                    for parent in rows[..at].iter().rev() {
                        if parent.depth < row.depth {
                            self.layer_focus = Some(parent.id.clone());
                            self.selection.replace(parent.id.clone());
                            cx.notify();
                            break;
                        }
                    }
                }
            }
            "enter" | "f2" => {
                let id = row.id.clone();
                let name = selection::find(self.editor.document(), &id)
                    .and_then(|node| node.name.clone())
                    .unwrap_or_else(|| layers::kind_name(selection::find(self.editor.document(), &id).unwrap()).to_owned());
                self.renaming = Some(id.clone());
                self.selection.replace(id);
                self.overlay = Some(Overlay::Rename(Prompt::new(
                    "Rename layer",
                    "A name; empty clears it",
                    name,
                )));
                cx.notify();
            }
            _ => {}
        }
    }

    fn rename_node(&mut self, value: &str) {
        let Some(id) = self.renaming.take() else {
            return;
        };
        if self.editor.rename_node(&id, value) {
            self.refresh();
        } else {
            self.status = Some(Status::error(
                "Names are one line and at most 120 characters",
            ));
        }
    }

    fn write_preview_ratios(&mut self, cx: &mut Context<Self>) {
        let path = preview_ratios_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.preview_ratios) {
            if let Err(error) = std::fs::write(&path, bytes) {
                self.status = Some(Status::error(format!("Could not save ratios: {error}")));
            }
        }
        cx.notify();
    }

    fn add_preview_ratio(&mut self, value: &str, cx: &mut Context<Self>) {
        let Some(id) = preview_ratio_id(value) else {
            self.status = Some(Status::error(
                "A ratio needs width:height, or the word Circle.",
            ));
            cx.notify();
            return;
        };
        if self.preview_ratios.iter().any(|token| preview_ratio_id(token).as_deref() == Some(id.as_str())) {
            self.status = Some(Status::error("That shape is already in the list."));
            cx.notify();
            return;
        }
        self.preview_ratios.push(value.trim().to_owned());
        self.write_preview_ratios(cx);
    }

    fn remove_preview_ratio(&mut self, value: &str, cx: &mut Context<Self>) {
        self.preview_ratios.retain(|token| token != value);
        self.write_preview_ratios(cx);
    }

    fn begin_field(&mut self, key: &str, label: &str, value: &str, cx: &mut Context<Self>) {
        // A binding's key is chosen from what this host measures, so a typo
        // cannot become a permanently missing reading.
        if key.ends_with(".key")
            && let Some(rows) = self
                .host
                .as_ref()
                .map(|host| binding_keys::rows(Some(host), &self.readings))
            && !rows.is_empty()
        {
            self.overlay = Some(Overlay::BindingKey {
                field: key.to_owned(),
                rows,
            });
            cx.notify();
            return;
        }
        self.editing = Some(key.to_owned());
        self.overlay = Some(Overlay::Field(Prompt::new(
            format!("Edit {label}"),
            "Enter applies, Esc cancels",
            value,
        )));
        cx.notify();
    }

    /// Writes a measured key into the binding field the picker was opened for.
    fn choose_binding_key(&mut self, semantic_key: &str, cx: &mut Context<Self>) {
        let Some(Overlay::BindingKey { field, .. }) = self.overlay.take() else {
            return;
        };
        self.editing = Some(field);
        self.apply_field(semantic_key, cx);
        cx.notify();
    }

    fn apply_field(&mut self, value: &str, cx: &mut Context<Self>) {
        let Some(key) = self.editing.take() else {
            return;
        };
        let mut outcome = Ok(());
        if key == "preview-ratio.new" {
            self.add_preview_ratio(value, cx);
            return;
        }
        if let Some(rest) = key.strip_prefix("palette.") {
            outcome = if rest == "new" {
                let (name, colour) = value
                    .split_once('=')
                    .map_or((value, value), |(name, colour)| (name, colour));
                self.editor
                    .add_palette(name, colour)
                    .map(|_| ())
                    .ok_or_else(|| "A new colour needs name = #rrggbb".to_owned())
            } else if let Some(entry) = rest.strip_suffix(".name") {
                if self.editor.rename_palette(entry, value) {
                    Ok(())
                } else {
                    Err("A token name is one non-empty line".to_owned())
                }
            } else {
                self.editor.set_palette_colour(rest, value)
            };
        } else if let Some(rest) = key.strip_prefix("token.") {
            outcome = if rest == "new" {
                self.editor
                    .add_token(value)
                    .map(|_| ())
                    .ok_or_else(|| "A new type token needs a name".to_owned())
            } else if let Some(entry) = rest.strip_suffix(".family") {
                self.editor.set_token_family(entry, value)
            } else if let Some(entry) = rest.strip_suffix(".weight") {
                self.editor.set_token_weight(entry, value)
            } else {
                Err("No such type token field".to_owned())
            };
        } else if key.starts_with("theme.")
            || key.starts_with("artboard.")
            || key.starts_with("background.")
        {
            self.editor
                .edit(|document| outcome = inspector::apply_document(document, &key, value));
        } else if key == "bindings.new" {
            outcome = match self.selection.first().map(str::to_owned) {
                Some(id) => self.editor.add_binding(&id, value).map(|_| ()),
                None => Err("Select a node to bind a reading".to_owned()),
            };
        } else if let Some(id) = self.selection.first().map(str::to_owned) {
            self.editor
                .edit(|document| outcome = inspector::apply(document, &id, &key, value));
        } else {
            outcome = Err("Select a node to edit its fields".to_owned());
        }
        match outcome {
            Ok(()) => {
                self.status = Some(Status::ok(format!(
                    "Set {}",
                    key.rsplit('.').next().unwrap_or(&key)
                )));
                self.refresh();
            }
            Err(message) => self.status = Some(Status::error(message)),
        }
    }

    /// The selection inspector: the selected subject's sections and the binding
    /// affordance. With nothing selected it says so, as the source does.
    fn inspector_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut children: Vec<AnyElement> = Vec::new();
        if self.selection.first().is_some() {
            // The source keeps the eight arrange actions in the selection
            // inspector rather than the canvas dock.
            use gpui_kit::component::{
                Sizable,
                button::{Button, ButtonVariants},
                h_flex,
            };
            children.push(
                div()
                    .px_3()
                    .pt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Arrange")
                    .into_any_element(),
            );
            children.push(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .children(ARRANGE_ACTIONS.map(|command| {
                        Button::new(format!("arrange-{:?}", command))
                            .ghost()
                            .small()
                            .label(command.label())
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.apply(command, window, cx)
                            }))
                    }))
                    .into_any_element(),
            );
        }
        match self.selection.first() {
            Some(id) => children
                .extend(self.section_children(inspector::sections(self.editor.document(), id), cx)),
            None => children.push(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Select an object on the canvas to inspect it.")
                    .into_any_element(),
            ),
        }
        if self.selection.first().is_some() {
            children.push(
                div()
                    .id("binding-add")
                    .px_3()
                    .py_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .cursor_pointer()
                    .hover(|this| this.text_color(cx.theme().foreground))
                    .child("+ Add binding")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.begin_field("bindings.new", "semantic key", "", cx)
                    }))
                    .into_any_element(),
            );
        }
        self.scrolling_pane("inspector-body", None, children, cx)
    }

    /// The left pane's Theme settings slot: metadata, artboard and background.
    fn document_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sections = inspector::document_sections(self.editor.document());
        sections.retain(|section| section.title != "Palette");
        let children = self.section_children(sections, cx);
        self.scrolling_pane("document-body", Some("Theme settings"), children, cx)
    }

    /// The left pane's Design tokens slot: palette entries and type tokens.
    fn tokens_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (foreground, muted, input) = (
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().input,
        );
        let palette = self.editor.document().palette.clone();
        let mut children = vec![
            div()
                .px_3()
                .pt_2()
                .text_xs()
                .text_color(muted)
                .child("Palette")
                .into_any_element(),
        ];
        for (index, (id, entry)) in palette.into_iter().enumerate() {
            let colour_key = format!("palette.{id}");
            let name_key = format!("palette.{id}.name");
            let colour = palette::paint_text(&entry.value);
            let name = entry.name.clone();
            let (name_for_edit, colour_for_edit) = (name.clone(), colour.clone());
            children.push(
                div()
                    .id(("palette", index))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child(
                        div()
                            .id(("palette-name", index))
                            .flex_1()
                            .text_color(foreground)
                            .cursor_pointer()
                            .child(name)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.begin_field(&name_key, "token name", &name_for_edit, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(("palette-colour", index))
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .bg(input)
                            .text_color(foreground)
                            .cursor_pointer()
                            .child(colour)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.begin_field(&colour_key, "token colour", &colour_for_edit, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        children.push(
            div()
                .id("palette-add")
                .px_3()
                .py_1()
                .text_sm()
                .text_color(muted)
                .cursor_pointer()
                .child("+ Add colour")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.begin_field("palette.new", "new colour", "", cx)
                }))
                .into_any_element(),
        );
        children.push(
            div()
                .px_3()
                .pt_3()
                .text_xs()
                .text_color(muted)
                .child("Type")
                .into_any_element(),
        );
        for (index, (id, token)) in self
            .editor
            .document()
            .type_tokens
            .clone()
            .into_iter()
            .enumerate()
        {
            let family_key = format!("token.{id}.family");
            let weight_key = format!("token.{id}.weight");
            let family = token.family.clone();
            let weight = token
                .weight
                .as_ref()
                .map_or_else(String::new, tokens::weight_text);
            let id_for_family = id.clone();
            let (family_for_edit, weight_for_edit) = (family.clone(), weight.clone());
            children.push(
                div()
                    .id(("token", index))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child(div().text_color(muted).child(id_for_family))
                    .child(
                        div()
                            .id(("token-family", index))
                            .px_1()
                            .text_color(foreground)
                            .cursor_pointer()
                            .child(family)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.begin_field(&family_key, "font family", &family_for_edit, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(("token-weight", index))
                            .px_1()
                            .text_color(foreground)
                            .cursor_pointer()
                            .child(weight)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.begin_field(&weight_key, "weight", &weight_for_edit, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        children.push(
            div()
                .id("token-add")
                .px_3()
                .py_1()
                .text_sm()
                .text_color(muted)
                .cursor_pointer()
                .child("+ Add type token")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.begin_field("token.new", "new type token", "", cx)
                }))
                .into_any_element(),
        );
        self.scrolling_pane("tokens-body", Some("Design tokens"), children, cx)
    }

    /// The left pane's Add slot: everything the document can take, in the
    /// source's three families plus media.
    fn add_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            Sizable,
            button::{Button, ButtonVariants},
            h_flex,
        };

        const GROUPS: [(&str, &[Command]); 4] = INSERT_GROUPS;

        let (foreground, muted) = (cx.theme().foreground, cx.theme().muted_foreground);
        let mut children: Vec<AnyElement> = Vec::new();
        for (title, commands) in GROUPS {
            children.push(
                div()
                    .px_3()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(title)
                    .into_any_element(),
            );
            let mut rows: Vec<AnyElement> = Vec::new();
            for command in commands.iter().copied() {
                rows.push(
                    Button::new(format!("add-{command:?}"))
                        .ghost()
                        .small()
                        .label(command.label())
                        .text_color(foreground)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.apply(command, window, cx)
                        }))
                        .into_any_element(),
                );
            }
            children.push(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .children(rows)
                    .into_any_element(),
            );
        }
        self.scrolling_pane("add-body", Some("Add"), children, cx)
    }

    /// The left pane's Widgets slot: one row per unit of every installed pack.
    fn widgets_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (foreground, muted, input) = (
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().input,
        );
        let packs = yunari_packs::installed(&self.packs_directory());
        let mut children: Vec<AnyElement> = Vec::new();
        let mut rows: usize = 0;
        for pack in packs {
            for unit in pack.units {
                let unit = unit.clone();
                let label = format!(
                    "{}  ·  {}",
                    pack.name.clone().unwrap_or(pack.id.clone()),
                    unit.label
                );
                children.push(
                    div()
                        .id(("widget", rows))
                        .w_full()
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .text_sm()
                        .text_color(foreground)
                        .hover(|this| this.bg(input))
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.insert_widget(unit.clone(), cx)
                        }))
                        .into_any_element(),
                );
                rows += 1;
            }
        }
        if rows == 0 {
            children.push(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child(
                        "No widget packs are installed. Put a .yunari-widget-package in the host's settings/widget-packs folder.",
                    )
                    .into_any_element(),
            );
        }
        self.scrolling_pane("widgets-body", Some("Widgets"), children, cx)
    }

    /// The left pane's Assets slot: every declared file, with the two ways to
    /// add more.
    fn assets_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::IconName;
        use yunari_model::document::AssetKind;

        let (foreground, muted) = (cx.theme().foreground, cx.theme().muted_foreground);
        let assets = self.editor.document().assets.clone();
        let mut children: Vec<AnyElement> = Vec::new();
        if assets.is_empty() {
            children.push(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child("No assets in this theme yet.")
                    .into_any_element(),
            );
        }
        for (index, asset) in assets.into_iter().enumerate() {
            let (icon, kind) = match asset.kind {
                AssetKind::Image | AssetKind::Gif => (IconName::GalleryVerticalEnd, "Image"),
                AssetKind::Svg => (IconName::Frame, "SVG"),
                AssetKind::Video => (IconName::Play, "Video"),
                AssetKind::Font => (IconName::CaseSensitive, "Font"),
            };
            let name = asset
                .path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&asset.path)
                .to_owned();
            children.push(
                div()
                    .id(("asset", index))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .text_sm()
                    .child(div().text_xs().text_color(muted).child(icon))
                    .child(div().flex_1().text_color(foreground).child(name))
                    .child(div().text_xs().text_color(muted).child(kind))
                    .into_any_element(),
            );
        }
        children.push(
            div()
                .id("asset-import")
                .px_3()
                .py_1()
                .text_sm()
                .text_color(muted)
                .cursor_pointer()
                .child("+ Import asset")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.apply(Command::ImportAsset, window, cx)
                }))
                .into_any_element(),
        );
        children.push(
            div()
                .id("asset-find")
                .px_3()
                .py_1()
                .text_sm()
                .text_color(muted)
                .cursor_pointer()
                .child("Find assets")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.apply(Command::AssetFinder, window, cx)
                }))
                .into_any_element(),
        );
        self.scrolling_pane("assets-body", Some("Assets"), children, cx)
    }

    /// One section's title and its property rows, as click-to-edit fields.
    fn section_children(
        &self,
        sections: Vec<inspector::Section>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (foreground, muted, input) = (
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().input,
        );
        let mut children = Vec::new();
        for (section_index, section) in sections.into_iter().enumerate() {
            children.push(
                div()
                    .px_3()
                    .pt_2()
                    .text_xs()
                    .text_color(muted)
                    .child(section.title)
                    .into_any_element(),
            );
            for (row_index, field) in section.rows.into_iter().enumerate() {
                let key = field.key.clone();
                let label = field.label;
                let value = field.value.clone();
                children.push(
                    div()
                        .id(("field", section_index * 32 + row_index))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .cursor_pointer()
                        .text_sm()
                        .child(div().text_color(muted).child(label))
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(input)
                                .text_color(foreground)
                                .child(value.clone()),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.begin_field(&key, label, &value, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        children
    }

    /// A pane body: optional title strip and the rows below it, in the kit's
    /// own scroll area so the wheel and the scrollbar agree on one offset.
    fn scrolling_pane(
        &self,
        id: &'static str,
        title: Option<&'static str>,
        children: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Div {
        use gpui_kit::component::scroll::ScrollableElement;

        let theme = cx.theme();
        let (background, border, foreground) = (theme.background, theme.border, theme.foreground);
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(border)
            .bg(background);
        if let Some(title) = title {
            root = root.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .text_sm()
                    .text_color(foreground)
                    .child(title),
            );
        }
        root.child(
            div()
                .id(id)
                .flex_1()
                .w_full()
                .min_h(px(0.0))
                .overflow_y_scrollbar()
                .child(div().w_full().flex().flex_col().pb_2().children(children)),
        )
    }

    fn toggle_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_owned());
        }
        cx.notify();
    }

    /// Arrow keys move the selection; one burst of presses is one history entry.
    fn nudge(
        &mut self,
        key: &str,
        large: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let step = if large { NUDGE_STEP_LARGE } else { NUDGE_STEP };
        let (dx, dy) = match key {
            "left" => (-step, 0.0),
            "right" => (step, 0.0),
            "up" => (0.0, -step),
            "down" => (0.0, step),
            _ => return false,
        };
        let changes: Vec<(String, Transform)> = self
            .start_transforms()
            .into_iter()
            .map(|(id, mut transform)| {
                let (x, y) = transform::nudge(
                    (transform.x, transform.y, transform.width, transform.height),
                    dx,
                    dy,
                );
                transform.x = x;
                transform.y = y;
                (id, transform)
            })
            .collect();
        if changes.is_empty() {
            return false;
        }
        self.editor.begin_burst();
        self.editor.apply_transforms(&changes);
        self.refresh();
        self.nudge_epoch += 1;
        let epoch = self.nudge_epoch;
        cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(NUDGE_IDLE_MS))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this.nudge_epoch == epoch {
                    this.editor.end_burst();
                    this.refresh();
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
        true
    }

    fn canvas_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.gesture.is_some() {
            self.gesture_move(event, cx);
            return;
        }
        if self.panning.is_some() {
            self.pan_drag(event, cx);
            return;
        }
        let Some((anchor, _)) = self.marquee else {
            return;
        };
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        self.marquee = Some((
            anchor,
            (
                f32::from(event.position.x - bounds.origin.x),
                f32::from(event.position.y - bounds.origin.y),
            ),
        ));
        cx.notify();
    }

    /// Releases the gesture or the marquee, whichever was in flight.
    fn canvas_up(&mut self, shift: bool, cx: &mut Context<Self>) {
        self.panning = None;
        if self.gesture.take().is_some() {
            self.spacing_hold = None;
            self.internal_spacing.clear();
            self.guides.clear();
            self.editor.end_burst();
            self.refresh();
            cx.notify();
            return;
        }
        let Some((anchor, current)) = self.marquee.take() else {
            return;
        };
        let (x0, y0) = self.viewport.to_scene(point(px(anchor.0), px(anchor.1)));
        let (x1, y1) = self.viewport.to_scene(point(px(current.0), px(current.1)));
        if (x1 - x0).abs() < 1.0 && (y1 - y0).abs() < 1.0 {
            cx.notify();
            return;
        }
        let ids = selection::marquee(
            self.editor.document(),
            (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()),
        );
        if shift {
            for id in ids {
                if !self.selection.contains(&id) {
                    self.selection.toggle(&id);
                }
            }
        } else {
            self.selection.set(ids);
        }
        cx.notify();
    }

    /// Middle-drag or Space+drag pans the camera.
    fn pan_start(&mut self, position: Point<Pixels>) {
        self.panning = Some(position);
    }

    fn pan_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(last) = self.panning else {
            return;
        };
        self.panning = Some(event.position);
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        let dx = f32::from(event.position.x) - f32::from(last.x);
        let dy = f32::from(event.position.y) - f32::from(last.y);
        self.viewport.pan_by(dx, dy, bounds, self.artboard());
        self.camera_touched = true;
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        if self.overlay.is_some() {
            return;
        }
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        let (dx, dy) = scroll_delta(event);
        let cursor = point(
            event.position.x - bounds.origin.x,
            event.position.y - bounds.origin.y,
        );
        let board = self.artboard();
        if event.modifiers.control || event.modifiers.platform {
            self.viewport
                .zoom_at(wheel_factor(dy), cursor, bounds, board);
        } else if event.modifiers.shift {
            self.viewport.pan_by(-dx, 0.0, bounds, board);
        } else {
            self.viewport.pan_by(0.0, -dy, bounds, board);
        }
        self.camera_touched = true;
        cx.notify();
    }

    /// `+`/`-` walk the 1.1 ladder and `Shift+1` fits.
    fn zoom_key(&mut self, key: &str, shift: bool) -> bool {
        let Some(bounds) = self.canvas_bounds.get() else {
            return false;
        };
        let board = self.artboard();
        match key {
            "+" | "=" => self.viewport.zoom_by(ZOOM_STEP, bounds, board),
            "-" | "_" => self.viewport.zoom_by(1.0 / ZOOM_STEP, bounds, board),
            "0" => {
                self.viewport.fit(bounds, board);
                self.camera_touched = false;
                return true;
            }
            "1" | "!" if shift => {
                self.viewport.fit(bounds, board);
                self.camera_touched = false;
                return true;
            }
            _ => return false,
        }
        self.camera_touched = true;
        true
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let Some(overlay) = self.overlay.take() else {
            if keystroke.key == "space" {
                self.space_held = true;
                return;
            }
            if self.layer_focus.is_some()
                && matches!(
                    keystroke.key.as_str(),
                    "arrowdown"
                        | "arrowup"
                        | "arrowleft"
                        | "arrowright"
                        | "home"
                        | "end"
                        | "enter"
                        | "f2"
                )
            {
                self.navigate_layers(&keystroke.key, keystroke.modifiers.shift, cx);
                return;
            }
            if self.nudge(&keystroke.key, keystroke.modifiers.shift, window, cx) {
                return;
            }
            // Shift makes the browser report the shifted character, so both
            // spellings of the front/back chords land here.
            if keystroke.modifiers.control && matches!(keystroke.key.as_str(), "]" | "}") {
                if let Some(id) = self.selection.first().map(str::to_owned) {
                    self.editor.raise(&id, LayerMove::Front);
                    self.refresh();
                    cx.notify();
                }
                return;
            }
            if keystroke.modifiers.control && matches!(keystroke.key.as_str(), "[" | "{") {
                if let Some(id) = self.selection.first().map(str::to_owned) {
                    self.editor.raise(&id, LayerMove::Back);
                    self.refresh();
                    cx.notify();
                }
                return;
            }
            if keystroke.modifiers.control && keystroke.key == "a" {
                self.select_all();
                cx.notify();
                return;
            }
            if keystroke.key == "escape" && !self.selection.is_empty() {
                self.selection.clear();
                cx.notify();
                return;
            }
            if keystroke.key == "?" {
                self.overlay = Some(Overlay::Shortcuts);
                cx.notify();
                return;
            }
            // The platform's own menu key, so the canvas menu is not a
            // pointer-only surface.
            if keystroke.key == "contextmenu"
                || (keystroke.key == "f10" && keystroke.modifiers.shift)
            {
                self.context_from_keyboard(cx);
                return;
            }
            if self.zoom_key(&keystroke.key, keystroke.modifiers.shift) {
                cx.notify();
                return;
            }
            if let Some(command) = Command::from_key(
                &keystroke.key,
                keystroke.modifiers.control,
                keystroke.modifiers.shift,
                keystroke.modifiers.platform,
            ) {
                self.apply(command, window, cx);
            }
            return;
        };
        match overlay {
            Overlay::SaveAs(mut prompt) => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => self.overlay = Some(Overlay::SaveAs(prompt)),
                    Outcome::Cancel => {}
                    Outcome::Submit(value) => self.save_as(&value),
                }
            }
            Overlay::Import(mut prompt) => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => self.overlay = Some(Overlay::Import(prompt)),
                    Outcome::Cancel => {}
                    Outcome::Submit(value) => self.guard(Resume::Import(value), cx),
                }
            }
            Overlay::AssetFinder(mut dialog) => {
                if keystroke.key == "escape" && dialog.selected.is_some() {
                    dialog.selected = None;
                    self.overlay = Some(Overlay::AssetFinder(dialog));
                    cx.notify();
                    return;
                }
                match dialog
                    .query
                    .key(&keystroke.key, keystroke.key_char.as_deref())
                {
                    Outcome::Pending => self.overlay = Some(Overlay::AssetFinder(dialog)),
                    Outcome::Cancel => {}
                    Outcome::Submit(query) => {
                        self.overlay = Some(Overlay::AssetFinder(dialog));
                        self.search_stock_assets(query, cx);
                    }
                }
            }
            Overlay::Widgets(_)
            | Overlay::BindingKey { .. }
            | Overlay::PathPresets(_)
            | Overlay::Typography(_)
            | Overlay::Mcp(_)
            | Overlay::Settings(_)
            | Overlay::CollectionConsent { .. }
            | Overlay::Publish => {
                if keystroke.key == "escape" {
                    self.overlay = None;
                }
            }
            Overlay::ImportAsset(mut prompt) => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => self.overlay = Some(Overlay::ImportAsset(prompt)),
                    Outcome::Cancel => {}
                    Outcome::Submit(value) => self.import_asset(&value),
                }
            }
            Overlay::Confirm { message, resume } => {
                if keystroke.key == "escape" {
                    self.overlay = None;
                } else if keystroke.key == "enter" {
                    self.discard(resume, window, cx);
                } else {
                    self.overlay = Some(Overlay::Confirm { message, resume });
                }
            }
            Overlay::Shortcuts => {
                if keystroke.key != "escape" {
                    self.overlay = Some(Overlay::Shortcuts);
                }
            }
            // Any key but Escape leaves the canvas menu where it is; Escape is
            // its only keyboard exit until the rows take focus.
            menu @ (Overlay::ContextMenu { .. } | Overlay::Recent { .. }) => {
                if keystroke.key != "escape" {
                    self.overlay = Some(menu);
                }
            }
            Overlay::BridgePort { dialog, mut prompt } => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => {
                        self.overlay = Some(Overlay::BridgePort { dialog, prompt });
                    }
                    Outcome::Cancel => self.overlay = Some(Overlay::Settings(dialog)),
                    Outcome::Submit(value) => {
                        self.save_bridge_port(*dialog, &value);
                    }
                }
            }
            Overlay::WeatherPlace { dialog, mut prompt } => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => {
                        self.overlay = Some(Overlay::WeatherPlace { dialog, prompt });
                    }
                    Outcome::Cancel => self.overlay = Some(Overlay::Settings(dialog)),
                    Outcome::Submit(value) => {
                        self.save_weather_place(*dialog, &value);
                    }
                }
            }
            Overlay::Credential {
                dialog,
                id,
                mut prompt,
            } => match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                Outcome::Pending => {
                    self.overlay = Some(Overlay::Credential { dialog, id, prompt });
                }
                Outcome::Cancel => self.overlay = Some(Overlay::Settings(dialog)),
                Outcome::Submit(value) => {
                    self.save_credential(*dialog, &id, &value);
                }
            },
            Overlay::DisplayName {
                dialog,
                id,
                mut prompt,
            } => match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                Outcome::Pending => {
                    self.overlay = Some(Overlay::DisplayName { dialog, id, prompt });
                }
                Outcome::Cancel => self.overlay = Some(Overlay::Settings(dialog)),
                Outcome::Submit(value) => self.save_display_name(*dialog, &id, &value, cx),
            },
            Overlay::Field(mut prompt) => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => self.overlay = Some(Overlay::Field(prompt)),
                    Outcome::Cancel => self.editing = None,
                    Outcome::Submit(value) => self.apply_field(&value, cx),
                }
            }
            Overlay::Rename(mut prompt) => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => self.overlay = Some(Overlay::Rename(prompt)),
                    Outcome::Cancel => self.renaming = None,
                    Outcome::Submit(value) => self.rename_node(&value),
                }
            }
            Overlay::Library(dialog) => {
                if keystroke.key != "escape" {
                    self.overlay = Some(Overlay::Library(dialog));
                }
            }
        }
        cx.notify();
    }

    /// Throws the open document away and runs what the guard was holding.
    fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "space" && self.space_held {
            self.space_held = false;
            cx.notify();
        }
    }

    /// Keeps the artboard fitted until the author moves the camera.
    fn frame_camera(&mut self) {
        if self.camera_touched {
            return;
        }
        let Some(bounds) = self.canvas_bounds.get() else {
            return;
        };
        self.viewport = Viewport::contain(self.artboard(), bounds);
    }

    fn discard(&mut self, resume: Resume, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = None;
        if resume == Resume::Quit {
            window.remove_window();
            return;
        }
        self.resume(resume, cx);
    }

    fn layers_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{ActiveTheme, IconName};

        let theme = cx.theme();
        let (background, foreground, muted, border, selection) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.selection,
        );
        let count = layers::count(self.editor.document());
        let rows = layers::rows(self.editor.document(), &self.selection, &self.expanded);
        let items: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let id = row.id.clone();
                let caret_id = id.clone();
                let eye_id = id.clone();
                let lock_id = id.clone();
                let open = self.expanded.contains(&id);
                let ink = if row.visible { foreground } else { muted };
                div()
                    .id(("layer", index))
                    .on_drag(id.clone(), |name, _, _, cx| {
                        cx.new(|_| LayerDragPreview(name.clone()))
                    })
                    .on_drop({
                        let target = id.clone();
                        cx.listener(move |this, source: &String, _, _cx| {
                            if this.editor.move_node_to(source, &target) {
                                this.refresh();
                            }
                        })
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(8.0 + row.depth as f32 * 12.0))
                    .pr_2()
                    .py_1()
                    .bg(if row.selected { selection } else { background })
                    .cursor_pointer()
                    .child(
                        div()
                            .id(("caret", index))
                            .w(px(14.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(muted)
                            .child(if !row.has_children {
                                IconName::Minus
                            } else if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .when(!row.has_children, |this| this.opacity(0.0))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle_expanded(&caret_id, cx)
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(layer_icon(row.kind)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(ink)
                            .child(row.name.clone()),
                    )
                    .child(div().text_xs().text_color(muted).child(row.role))
                    .child(
                        div()
                            .id(("eye", index))
                            .px_1()
                            .text_color(muted)
                            .hover(|this| this.text_color(foreground))
                            .child(if row.visible {
                                IconName::Eye
                            } else {
                                IconName::EyeOff
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let shown = selection::find(this.editor.document(), &eye_id)
                                    .is_some_and(|node| node.visible);
                                if this.editor.set_visible(&eye_id, !shown) {
                                    this.refresh();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id(("lock", index))
                            .px_1()
                            .text_color(if row.locked { foreground } else { muted })
                            .hover(|this| this.text_color(foreground))
                            .child(IconName::Ban)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let locked = selection::find(this.editor.document(), &lock_id)
                                    .is_some_and(|node| node.locked);
                                if this.editor.set_locked(&lock_id, !locked) {
                                    this.refresh();
                                }
                                cx.notify();
                            })),
                    )
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.layer_click(&id, event.modifiers(), cx)
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(border)
            .bg(background)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .child(div().text_sm().text_color(foreground).child("Layers"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{count} objects")),
                    ),
            )
            .children(items)
    }

    fn canvas_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.clone();
        let images = self.images.clone();
        let bounds_cell = self.canvas_bounds.clone();
        let selections = self.selection_rects();
        let marquee = self.marquee;
        let guides = self.guides.clone();
        let handles: Vec<(Handle, (f64, f64))> = self
            .primary_rect()
            .map(|(_, rect, rotation)| {
                Handle::at(rect, rotation)
                    .into_iter()
                    .filter(|(handle, _)| *handle == Handle::Rotate || rotation.abs() < 1e-6)
                    .collect()
            })
            .unwrap_or_default();
        let view = self.viewport;
        div()
            .id("canvas")
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .bg(rgb(0x101014))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| this.on_scroll(event, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.layer_focus = None;
                    if this.space_held {
                        this.pan_start(event.position);
                    } else {
                        this.canvas_down(event, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, _, _| this.pan_start(event.position)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.layer_focus = None;
                    this.canvas_context(event, cx)
                }),
            )
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.canvas_drag(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.canvas_up(event.modifiers.shift, cx)
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.panning = None),
            )
            .child(
                div()
                    .relative()
                    .size_full()
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                bounds_cell.set(Some(bounds));
                                bounds
                            },
                            move |_, bounds, window, cx| {
                                let fit = yunari_render_gpui::Fit {
                                    scale: view.scale,
                                    origin: (
                                        f32::from(bounds.origin.x) + view.offset.0,
                                        f32::from(bounds.origin.y) + view.offset.1,
                                    ),
                                };
                                let mut store = images.borrow_mut();
                                yunari_render_gpui::paint_fitted(
                                    &list, fit, window, cx, &mut store,
                                );
                                for (x, y, width, height) in &selections {
                                    window.paint_quad(PaintQuad {
                                        bounds: Bounds {
                                            origin: fit.point(*x as f32, *y as f32),
                                            size: size(
                                                px(*width as f32 * fit.scale),
                                                px(*height as f32 * fit.scale),
                                            ),
                                        },
                                        corner_radii: Corners::default(),
                                        background: transparent_black().into(),
                                        border_widths: Edges::all(px(1.0)),
                                        border_color: rgb(ACCENT).into(),
                                        border_style: BorderStyle::Solid,
                                    });
                                }
                                for guide in &guides {
                                    paint_guide(guide, &view, bounds, window);
                                }
                                for (handle, (x, y)) in &handles {
                                    paint_handle(*handle, *x, *y, &view, bounds, window);
                                }
                                let Some((anchor, current)) = marquee else {
                                    return;
                                };
                                window.paint_quad(PaintQuad {
                                    bounds: Bounds {
                                        origin: point(
                                            bounds.origin.x + px(anchor.0.min(current.0)),
                                            bounds.origin.y + px(anchor.1.min(current.1)),
                                        ),
                                        size: size(
                                            px((current.0 - anchor.0).abs()),
                                            px((current.1 - anchor.1).abs()),
                                        ),
                                    },
                                    corner_radii: Corners::default(),
                                    background: transparent_black().into(),
                                    border_widths: Edges::all(px(1.0)),
                                    border_color: rgb(ACCENT).into(),
                                    border_style: BorderStyle::Solid,
                                });
                            },
                        )
                        .size_full(),
                    )
                    .child(self.stage_overlay(cx)),
            )
    }

    /// The stage's floating controls: zoom in/out, the present scale, fit,
    /// and the inspector toggle. Overlay chrome, not document content.
    fn stage_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            IconName, Sizable,
            button::{Button, ButtonVariants},
            h_flex,
        };

        let theme = cx.theme();
        let (panel, border, foreground) = (theme.popover, theme.border, theme.foreground);
        let present = format!("{:.0}%", self.viewport.scale * 100.0);
        let right = self.right_collapsed;
        let document = self.editor.document();
        let label = document
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| document.id.clone());
        let board = (
            self.viewport.offset.0,
            (self.viewport.offset.1 - 28.0).max(4.0),
        );
        let cluster = h_flex()
            .absolute()
            .top(px(12.0))
            .right(px(12.0))
            .gap_1()
            .p_1()
            .rounded_md()
            .bg(panel)
            .border_1()
            .border_color(border)
            .child(
                Button::new("stage-inspector")
                    .ghost()
                    .small()
                    .icon(IconName::PanelRight)
                    .tooltip(if right {
                        "Show inspector"
                    } else {
                        "Hide inspector"
                    })
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_right_pane(cx)),
                    ),
            )
            .child(
                Button::new("stage-zoom-out")
                    .ghost()
                    .small()
                    .icon(IconName::Minus)
                    .tooltip("Zoom out")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.zoom_stage(1.0 / ZOOM_STEP, cx)
                    })),
            )
            .child(
                div()
                    .id("stage-zoom-reset")
                    .w(px(46.0))
                    .text_xs()
                    .text_center()
                    .text_color(foreground)
                    .cursor_pointer()
                    .child(present)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reset_stage(cx))),
            )
            .child(
                Button::new("stage-zoom-in")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("Zoom in")
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.zoom_stage(ZOOM_STEP, cx)),
                    ),
            )
            .child(
                Button::new("stage-fit")
                    .ghost()
                    .small()
                    .icon(IconName::Maximize)
                    .tooltip("Fit artboard")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.fit_stage(cx))),
            );
        div()
            .absolute()
            .inset_0()
            .child(cluster)
            .child(
                div()
                    .id("stage-label")
                    .absolute()
                    .left(px(board.0))
                    .top(px(board.1))
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .bg(panel)
                    .border_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(foreground)
                    .cursor_pointer()
                    .child(label)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.fit_stage(cx))),
            )
            .child(
                div().absolute().bottom(px(14.0)).left(px(14.0)).child(
                    Button::new("stage-assets")
                        .ghost()
                        .small()
                        .icon(IconName::GalleryVerticalEnd)
                        .tooltip("Find assets")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.apply(Command::AssetFinder, window, cx)
                        })),
                ),
            )
            // The source's canvas dock: the object commands this selection can
            // run, floating over the stage. The eight arrange actions stay in
            // the inspector, as in the source.
            .when(!self.selection.is_empty(), |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom(px(14.0))
                        .left(px(0.0))
                        .right(px(0.0))
                        .flex()
                        .justify_center()
                        .child(
                            h_flex()
                                .gap_1()
                                .p_1()
                                .rounded_md()
                                .bg(panel)
                                .border_1()
                                .border_color(border)
                                .children(DOCK_ACTIONS.map(|entry| {
                                    Button::new(format!("dock-{}", entry.label()))
                                        .ghost()
                                        .small()
                                        .icon(dock_icon(entry))
                                        .tooltip(entry.label())
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                run_canvas_entry(this, entry, window, cx)
                                            },
                                        ))
                                        .into_any_element()
                                })),
                        ),
                )
            })
    }

    fn zoom_stage(&mut self, factor: f32, cx: &mut Context<Self>) {
        if let Some(bounds) = self.canvas_bounds.get() {
            let board = self.artboard();
            self.viewport.zoom_by(factor, bounds, board);
            self.camera_touched = true;
            cx.notify();
        }
    }

    fn reset_stage(&mut self, cx: &mut Context<Self>) {
        if let Some(bounds) = self.canvas_bounds.get() {
            let board = self.artboard();
            self.viewport = Viewport::reset(bounds, board);
            self.camera_touched = true;
            cx.notify();
        }
    }

    fn fit_stage(&mut self, cx: &mut Context<Self>) {
        if let Some(bounds) = self.canvas_bounds.get() {
            let board = self.artboard();
            self.viewport.fit(bounds, board);
            self.camera_touched = false;
            cx.notify();
        }
    }

    /// The canvas menu: a cursor-anchored list of the object's commands, or of
    /// everything insertable when the press missed every object.
    fn context_menu_layer(
        &self,
        at: Point<Pixels>,
        object: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::ActiveTheme;

        let (foreground, hover, border, muted) = (
            cx.theme().foreground,
            cx.theme().muted,
            cx.theme().border,
            cx.theme().muted_foreground,
        );
        let mut entries: Vec<AnyElement> = Vec::new();
        let row = |entry: CanvasMenuEntry, entries: &mut Vec<AnyElement>| {
            let label = entry.label();
            entries.push(
                div()
                    .id(format!("canvas-menu-{label}"))
                    .px_3()
                    .py_0p5()
                    .text_sm()
                    .text_color(foreground)
                    .cursor_pointer()
                    .hover(|this| this.bg(hover))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.overlay = None;
                        run_canvas_entry(this, entry, window, cx);
                    }))
                    .into_any_element(),
            );
        };
        let divider = |entries: &mut Vec<AnyElement>| {
            entries.push(div().h(px(1.0)).w_full().bg(border).into_any_element());
        };
        if object.is_some() {
            for (index, group) in OBJECT_MENU.iter().enumerate() {
                if index > 0 {
                    divider(&mut entries);
                }
                for entry in group.iter().copied() {
                    row(entry, &mut entries);
                }
            }
        } else {
            for (index, (title, commands)) in INSERT_GROUPS.iter().enumerate() {
                if index > 0 {
                    divider(&mut entries);
                }
                entries.push(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(muted)
                        .child(*title)
                        .into_any_element(),
                );
                for command in commands.iter().copied() {
                    row(CanvasMenuEntry::Command(command), &mut entries);
                }
            }
        }
        let height = self.window_height.min(px(520.0));
        self.menu_layer(at, px(200.0), height, entries, cx)
    }

    /// A menu: a transparent layer that dismisses it, then the panel anchored
    /// at the gesture. `height` bounds the panel so a short window scrolls it.
    fn menu_layer(
        &self,
        at: Point<Pixels>,
        width: Pixels,
        height: Pixels,
        entries: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use gpui_kit::component::ActiveTheme;
        use gpui_kit::component::scroll::ScrollableElement;

        let (border, panel) = (cx.theme().border, cx.theme().popover);
        let cap = height;
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.overlay = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.overlay = None;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                // `anchored` is gpui's own placement: it keeps the panel inside
                // the window, flipping it up when the press was near the foot.
                anchored()
                    .position(point(
                        at.x,
                        // Flip the panel above the press when it would run off
                        // the foot; `anchored` alone does not shrink it.
                        if at.y + cap > self.window_height - px(8.0) {
                            (at.y - height).max(px(8.0))
                        } else {
                            at.y
                        },
                    ))
                    .anchor(Anchor::TopLeft)
                    .child(
                        div()
                            .w(width)
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .bg(panel)
                            .child(
                                div()
                                    .h(height)
                                    .min_h(px(0.0))
                                    .overflow_y_scrollbar()
                                    .child(div().flex().flex_col().py_1().children(entries)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// **Open recent**: the themes this editor has opened, newest first, each
    /// by the name it had when it opened. Said rather than drawn empty when
    /// there are none, as the source does.
    fn recent_menu_layer(&self, at: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::ActiveTheme;

        let (foreground, hover, muted) = (
            cx.theme().foreground,
            cx.theme().muted,
            cx.theme().muted_foreground,
        );
        let mut entries: Vec<AnyElement> = Vec::new();
        if self.recents.is_empty() {
            entries.push(
                div()
                    .px_3()
                    .py_0p5()
                    .text_sm()
                    .text_color(muted)
                    .child("No recent themes yet.")
                    .into_any_element(),
            );
        }
        for recent in self.recents.iter().cloned() {
            let id = recent.id.clone();
            entries.push(
                div()
                    .id(format!("recent-{}", recent.id))
                    .px_3()
                    .py_0p5()
                    .text_sm()
                    .text_color(foreground)
                    .cursor_pointer()
                    .hover(|this| this.bg(hover))
                    .child(recent.name)
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_entry(&id, cx)),
                    )
                    .into_any_element(),
            );
        }
        // Sized to its rows rather than a fixed box: three opens are three
        // lines, and an empty menu is one.
        let rows = self.recents.len().max(1) as f32;
        self.menu_layer(at, px(200.0), px(rows * 26.0 + 16.0), entries, cx)
    }

    /// Opens a collection's consent sheet. The host reads the package under
    /// its configured origin; the editor only shows the includes it names.
    fn open_collection_consent(&mut self, row: &serde_json::Value, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            self.status = Some(Status::error(
                "The theme store needs the host running on this PC.",
            ));
            cx.notify();
            return;
        };
        let Some(url) = row.get("packageUrl").and_then(serde_json::Value::as_str) else {
            self.status = Some(Status::error(
                "That listing does not name a package to read.",
            ));
            cx.notify();
            return;
        };
        let includes = match host.store.collection_plan(url, None) {
            Ok(plan) => plan
                .get("members")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default(),
            Err(error) => {
                self.status = Some(Status::error(error));
                cx.notify();
                return;
            }
        };
        let chosen = includes
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_owned)
            .collect();
        self.overlay = Some(Overlay::CollectionConsent {
            row: row.clone(),
            includes,
            chosen,
        });
        cx.notify();
    }

    fn toggle_collection_include(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(Overlay::CollectionConsent { chosen, .. }) = &mut self.overlay {
            if !chosen.remove(id) {
                chosen.insert(id.to_owned());
            }
        }
        cx.notify();
    }

    /// Installs one catalogue row through the host's own install path. The
    /// editor never fetches bytes itself and never holds a session token; the
    /// row's own digest and size travel with the request.
    fn install_library_item(
        &mut self,
        row: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.host.clone() else {
            self.status = Some(Status::error(
                "The theme store needs the host running on this PC.",
            ));
            cx.notify();
            return;
        };
        let text = |key: &str| {
            row.get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let Some(request) = yunari_host::store_install::parse_request(&serde_json::json!({
            "packageUrl": text("packageUrl").unwrap_or_default(),
            "catalogueId": text("catalogueId").or_else(|| text("id")).unwrap_or_default(),
            "kind": text("kind").unwrap_or_default(),
            "version": text("version").unwrap_or_default(),
            "sha256": text("sha256"),
            "bytes": row.get("bytes").and_then(serde_json::Value::as_u64),
        })) else {
            self.status = Some(Status::error(
                "That listing does not record everything an install needs.",
            ));
            cx.notify();
            return;
        };
        self.status = Some(Status::ok("Installing…"));
        cx.notify();
        cx.spawn_in(window, async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let fetcher = std::sync::Arc::clone(&host);
                    let download: yunari_host::store_install::Download =
                        std::sync::Arc::new(move |url: &str| {
                            yunari_host::store_install::download(
                                fetcher.account.origin.as_deref(),
                                url,
                                &fetcher.account.transport,
                            )
                        });
                    yunari_host::store_install::install(&host, &request, &download)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.status = Some(match outcome {
                    Ok(installed) => Status::ok(format!("Installed as {}.", installed.id)),
                    Err(refusal) => Status::error(refusal.message),
                });
                this.refresh_library();
                cx.notify();
            });
        })
        .detach();
    }

    /// The catalogue answer names its rows `entries`, `items` or `themes`
    /// depending on which source answered. Reading all three keeps the page
    /// from showing an empty list for a valid answer.
    fn catalogue_entries(value: &serde_json::Value) -> Vec<serde_json::Value> {
        value
            .get("entries")
            .or_else(|| value.get("items"))
            .or_else(|| value.get("themes"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    fn reload_library_catalogue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.library_error = None;
        self.library_catalogue_loaded = false;
        self.load_library_catalogue(window, cx);
        cx.notify();
    }

    /// Reads the catalogue halves once, off the UI thread, for the Library's
    /// Collections and My submissions pages.
    fn load_library_catalogue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            self.library_catalogue_loaded = true;
            return;
        };
        self.library_catalogue_loaded = true;
        cx.spawn_in(window, async move |weak, cx| {
            let (catalogue, mine) = cx
                .background_executor()
                .spawn(async move {
                    let catalogue = host.store.themes(yunari_host::http::now_ms());
                    let mine = host.account.client().map(|client| client.mine());
                    (catalogue, mine)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                match catalogue {
                    Ok(value) => {
                        this.library_catalogue = Self::catalogue_entries(&value)
                    }
                    Err(message) => {
                        this.library_error = Some(message);
                        this.status = Some(Status::error("The store could not be read."));
                    }
                }
                match mine {
                    Some(Ok(mine)) => this.library_mine = Self::catalogue_entries(&mine),
                    Some(Err(error)) => {
                        this.library_error = Some(error.message);
                        this.status = Some(Status::error("Your submissions could not be read."));
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The header's chord opens the surface; pressing it moves nothing.
    fn open_publish(&mut self, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Publish);
        cx.notify();
    }

    /// Assigns the open theme to a display through the host's own store, so
    /// the player reads the same file the dialog wrote.
    fn assign_display(&mut self, id: &str, cx: &mut Context<Self>) {
        let theme = self.editor.document().id.clone();
        let mut assignment = yunari_host::assignment::Assignment::load(&self.settings_dir());
        assignment.set(id, Some(&theme));
        self.status = Some(Status::ok(format!("Assigned {theme} to {id}")));
        cx.notify();
    }

    /// Flips the host's LAN switch. The warning stays on the surface beside it.
    fn toggle_publish(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings_dir();
        let mut hosting = yunari_host::hosting::Hosting::load(&settings);
        hosting.lan = !hosting.lan;
        hosting.save(&settings);
        self.status = Some(Status::ok(if hosting.lan {
            "Publishing on the local network"
        } else {
            "Publishing stopped"
        }));
        cx.notify();
    }

    /// The collection sheet: required includes stay chosen, optional ones can
    /// be left out, and Install sends only the chosen members to the host.
    fn collection_consent_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::button::{Button, ButtonVariants};

        let Some(Overlay::CollectionConsent {
            includes, chosen, ..
        }) = &self.overlay
        else {
            return div().into_any_element();
        };
        let mut children: Vec<AnyElement> = Vec::new();
        children.push(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("A collection installs the listings it names. Optional members can be left out.")
                .into_any_element(),
        );
        for entry in includes {
            let id = entry
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if id.is_empty() {
                continue;
            }
            let optional = entry
                .get("optional")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let picked = chosen.contains(&id);
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(id.as_str())
                .to_owned();
            let toggle_id = id.clone();
            children.push(
                Button::new(format!("collection-include-{id}"))
                    .ghost()
                    .label(format!(
                        "{}  ·  {}{}",
                        name,
                        if optional { "optional" } else { "required" },
                        if picked { "  ·  included" } else { "  ·  skipped" }
                    ))
                    .with_variant(if picked {
                        gpui_kit::component::button::ButtonVariant::Primary
                    } else {
                        gpui_kit::component::button::ButtonVariant::Ghost
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_collection_include(&toggle_id, cx)
                    }))
                    .into_any_element(),
            );
        }
        let rows: Vec<serde_json::Value> = includes
            .iter()
            .filter(|entry| {
                entry
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| chosen.contains(id))
            })
            .cloned()
            .collect();
        children.push(
            Button::new("collection-install")
                .label("Install chosen")
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let Some(Overlay::CollectionConsent { row, .. }) = &this.overlay else {
                        return;
                    };
                    let row = row.clone();
                    let members = rows.clone();
                    this.overlay = None;
                    // A collection's members are installed through the same
                    // host path as a single listing; each carries its own
                    // package facts from the catalogue answer.
                    for member in members {
                        let found = member
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|id| {
                                this.library_catalogue
                                    .iter()
                                    .find(|candidate| {
                                        candidate.get("id").and_then(serde_json::Value::as_str)
                                            == Some(id)
                                    })
                                    .cloned()
                            });
                        if let Some(found) = found {
                            this.install_library_item(&found, window, cx);
                        }
                    }
                    let _ = row;
                    cx.notify();
                }))
                .into_any_element(),
        );
        children.push(
            div()
                .id("collection-cancel")
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .cursor_pointer()
                .child("Cancel  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx)))
                .into_any_element(),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .children(children)
            .into_any_element()
    }

    fn publish_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings_dir();
        let hosting = yunari_host::hosting::Hosting::load(&settings);
        let displays = yunari_host::displays::DisplayStore::load(&settings);
        let assignment = yunari_host::assignment::Assignment::load(&settings);
        let theme = self.editor.document().id.clone();
        let (ink, muted, border, accent) = (
            cx.theme().foreground,
            cx.theme().muted_foreground,
            rgb(MUTED),
            rgb(ACCENT),
        );
        let mut children: Vec<AnyElement> = Vec::new();
        children.push(
            div()
                .text_sm()
                .text_color(ink)
                .child("Assign to a display")
                .into_any_element(),
        );
        children.push(
            div()
                .text_xs()
                .text_color(muted)
                .child("Display")
                .into_any_element(),
        );
        for record in displays.list() {
            let id = record.id.clone();
            let active = assignment.get(&id) == Some(theme.as_str());
            children.push(
                div()
                    .id(format!("publish-display-{}", record.id))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(if active { accent } else { border })
                    .child(div().text_sm().child(record.label.clone()))
                    .child(if active { "Assigned" } else { "Assign" })
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.assign_display(&id, cx)
                        }),
                    )
                    .into_any_element(),
            );
        }
        if displays.list().is_empty() {
            children.push(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Pair a new remote display")
                    .into_any_element(),
            );
        }
        children.push(
            div()
                .text_xs()
                .text_color(muted)
                .child("Publishing")
                .into_any_element(),
        );
        children.push(
            div()
                .id("publish-toggle")
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(cx.theme().popover)
                .child(if hosting.lan {
                    "Stop publishing"
                } else {
                    "Publish to a phone"
                })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_publish(cx)))
                .into_any_element(),
        );
        children.push(
            div()
                .text_xs()
                .text_color(muted)
                .child(
                    "Phone access is HTTPS. Plain HTTP is the fallback on your own network — trusted networks only, never the internet.",
                )
                .into_any_element(),
        );
        if hosting.lan {
            if let Some(host) = &self.host {
                let port = host.bound_port.load(std::sync::atomic::Ordering::Relaxed);
                let name = host.identity.name.clone();
                let code = host
                    .pairing
                    .lock()
                    .ok()
                    .map(|mut pairing| pairing.current(yunari_host::http::now_ms()).code);
                children.push(
                    div()
                        .text_xs()
                        .text_color(ink)
                        .child("Phone")
                        .into_any_element(),
                );
                if let Some(address) = yunari_host::hosting::lan_address() {
                    children.push(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("Phone address: https://{address}:{port}"))
                            .into_any_element(),
                    );
                    children.push(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("Fallback http://{address}:{port}"))
                            .into_any_element(),
                    );
                } else {
                    children.push(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("The host could not find a network address.")
                            .into_any_element(),
                    );
                }
                children.push(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("Also https://{name}:{port}, where your device resolves it."))
                        .into_any_element(),
                );
                if let Some(code) = code {
                    children.push(
                        div()
                            .text_xs()
                            .text_color(ink)
                            .child(format!("Pairing code: {code}"))
                            .into_any_element(),
                    );
                    if let Some(address) = yunari_host::hosting::lan_address() {
                        let url = format!("https://{address}:{port}/pair?code={code}");
                        if let Ok(qr) = qrcode::QrCode::new(url.as_bytes()) {
                            let svg = qr.render::<qrcode::render::svg::Color>().build();
                            let image = std::sync::Arc::new(gpui_kit::Image::from_bytes(
                                gpui_kit::ImageFormat::Svg,
                                svg.into_bytes(),
                            ));
                            children.push(
                                div()
                                    .id("publish-qr")
                                    .size(px(120.0))
                                    .child(
                                        img(gpui_kit::ImageSource::Image(image)).size_full(),
                                    )
                                    .into_any_element(),
                            );
                        }
                    }
                }
            } else {
                children.push(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("The host is not running on this PC.")
                        .into_any_element(),
                );
            }
        }
        children.push(
            div()
                .id("publish-close")
                .mt_2()
                .text_xs()
                .text_color(muted)
                .cursor_pointer()
                .child("Close  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx)))
                .into_any_element(),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .children(children)
            .into_any_element()
    }

    /// The `?` reference: the source's own rows, one column per group, in the
    /// order the source's table declares them.
    fn shortcuts_panel(cx: &mut Context<Self>) -> AnyElement {
        let (foreground, muted, border) = (
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().border,
        );
        let mut columns: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        while index < SHORTCUT_REFERENCE.len() {
            let group = SHORTCUT_REFERENCE[index].group;
            let mut column = div()
                .w(px(200.0))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(group));
            while index < SHORTCUT_REFERENCE.len() && SHORTCUT_REFERENCE[index].group == group {
                let row = &SHORTCUT_REFERENCE[index];
                column = column.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .child(div().text_color(foreground).child(row.label))
                        .child(div().text_xs().text_color(muted).child(row.chord)),
                );
                index += 1;
            }
            columns.push(column.into_any_element());
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(border)
                    .pb_2()
                    .child(div().text_color(foreground).child("Keyboard shortcuts"))
                    .child(div().text_xs().text_color(muted).child("Esc closes")),
            )
            // A short window scrolls the sheet rather than letting it run past
            // the viewport.
            .child(
                div()
                    .id("shortcuts-grid")
                    .flex()
                    .flex_wrap()
                    .gap_x_6()
                    .gap_y_4()
                    .h(px(420.0))
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .children(columns),
            )
            .into_any_element()
    }

    fn prompt_panel(prompt: &Prompt, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .child(div().text_color(rgb(INK)).child(prompt.title.clone()))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(0x101014))
                    .text_color(rgb(INK))
                    .child(format!("{}▏", prompt.value)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(prompt.hint.clone()),
            )
            .child(
                div().flex().justify_end().child(
                    div()
                        .id("prompt-ok")
                        .px_3()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(ACCENT))
                        .text_color(rgb(0x101014))
                        .cursor_pointer()
                        .child("OK  ·  Enter")
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.submit_prompt(cx)),
                        ),
                ),
            )
            .into_any_element()
    }

    fn submit_prompt(&mut self, cx: &mut Context<Self>) {
        match self.overlay.take() {
            Some(Overlay::SaveAs(prompt)) => self.save_as(&prompt.value),
            Some(Overlay::Rename(prompt)) => self.rename_node(&prompt.value),
            Some(Overlay::Field(prompt)) => self.apply_field(&prompt.value, cx),
            Some(Overlay::Import(prompt)) => self.guard(Resume::Import(prompt.value), cx),
            Some(Overlay::ImportAsset(prompt)) => {
                self.import_asset(&prompt.value);
            }
            other => self.overlay = other,
        }
        cx.notify();
    }

    fn overlay_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = match &self.overlay {
            None => return None,
            Some(Overlay::SaveAs(prompt) | Overlay::Rename(prompt) | Overlay::Field(prompt)) => {
                Self::prompt_panel(prompt, cx)
            }
            Some(
                Overlay::WeatherPlace { prompt, .. }
                | Overlay::Credential { prompt, .. }
                | Overlay::DisplayName { prompt, .. }
                | Overlay::BridgePort { prompt, .. },
            ) => Self::prompt_panel(prompt, cx),
            Some(Overlay::Import(prompt) | Overlay::ImportAsset(prompt)) => {
                Self::prompt_panel(prompt, cx)
            }
            Some(Overlay::Library(dialog)) => library_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::Widgets(dialog)) => widget_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::AssetFinder(dialog)) => asset_finder::view(dialog, cx).into_any_element(),
            Some(Overlay::BindingKey { rows, .. }) => {
                binding_keys::view(rows, cx).into_any_element()
            }
            Some(Overlay::PathPresets(dialog)) => {
                path_preset_dialog::view(dialog, cx).into_any_element()
            }
            Some(Overlay::Typography(dialog)) => {
                typography_dialog::view(dialog, cx).into_any_element()
            }
            Some(Overlay::Mcp(dialog)) => mcp_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::Settings(dialog)) => settings_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::Shortcuts) => Self::shortcuts_panel(cx),
            Some(Overlay::Publish) => self.publish_panel(cx),
            Some(Overlay::CollectionConsent { .. }) => self.collection_consent_panel(cx),
            // A menu is anchored to the gesture, not to a dialog's centre.
            Some(Overlay::ContextMenu { at, object }) => {
                return Some(self.context_menu_layer(*at, object.clone(), cx));
            }
            Some(Overlay::Recent { at }) => {
                return Some(self.recent_menu_layer(*at, cx));
            }
            Some(Overlay::Confirm { message, resume }) => {
                let discard = resume.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .child(div().text_color(rgb(INK)).child(message.clone()))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("discard")
                                    .px_3()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(rgb(0xf38ba8))
                                    .text_color(rgb(0x101014))
                                    .cursor_pointer()
                                    .child("Discard")
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, window, cx| {
                                            this.discard(discard.clone(), window, cx)
                                        },
                                    )),
                            )
                            .child(
                                div()
                                    .id("keep")
                                    .px_3()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .text_color(rgb(MUTED))
                                    .child("Keep editing  ·  Esc")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.close_overlay(cx)
                                    })),
                            ),
                    )
                    .into_any_element()
            }
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000cc))
                .child(
                    div()
                        .w(px(460.0))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(0x2e2e38))
                        .bg(rgb(PANEL))
                        .child(content),
                )
                .into_any_element(),
        )
    }
}

impl Render for EditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.polling {
            self.polling = true;
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    let playing = this
                        .update(cx, |this, _| this.list.has_video())
                        .unwrap_or(false);
                    cx.background_executor()
                        .timer(if playing { FRAME } else { TICK })
                        .await;
                    let alive = this.update(cx, |this, cx| {
                        if this.poll(now_ms()) || this.list.has_video() {
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        // Keys are handled by this element, so it must hold the focus; nothing
        // else in the shell is focusable.
        if !self.focus.is_focused(window) {
            self.focus.focus(window, cx);
        }
        self.frame_camera();
        // Closing the window is the same decision as New or Open: unsaved work
        // is discarded only on purpose. A refused close keeps the window open
        // with the guard showing.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let mut close = true;
            this.update(cx, |this, cx| {
                if this.editor.is_dirty() {
                    close = false;
                    this.overlay = Some(Overlay::Confirm {
                        message: "This theme has unsaved changes. Quit anyway?".to_owned(),
                        resume: Resume::Quit,
                    });
                    cx.notify();
                }
            })
            .ok();
            close
        });
        self.window_height = window.bounds().size.height;
        self.window_width = window.bounds().size.width;
        if self.library_search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search listings"));
            cx.subscribe(&search, |this: &mut EditorWindow, search: Entity<InputState>, event: &InputEvent, cx: &mut Context<EditorWindow>| {
                if matches!(event, InputEvent::Change) {
                    if let Some(themes) = &mut this.themes {
                        themes.query = search.read(cx).value().to_string();
                        cx.notify();
                    }
                }
            })
            .detach();
            self.library_search = Some(search);
        }
        if self.mode == ShellMode::Library && !self.library_catalogue_loaded {
            self.load_library_catalogue(window, cx);
        }
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(Self::on_key_up))
            .child(self.header(cx))
            .child(self.body(cx))
            .child(self.status_bar(cx))
            .children(self.overlay_view(cx))
    }
}

/// Where a new node goes: inset from the edge, each add stepping further so a
/// second node is visible rather than hidden under the first.
fn place(document: &ThemeDocument, width: f64, height: f64) -> Transform {
    let count = document.nodes.len() as f64;
    let offset = INSET + STEP * (count % 8.0);
    Transform {
        x: offset,
        y: offset,
        width,
        height,
        ..Transform::default()
    }
}

/// A wheel event as pixels on both axes.
fn scroll_delta(event: &ScrollEvent) -> (f32, f32) {
    match event.delta {
        ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
        ScrollDelta::Lines(delta) => (delta.x * LINE_HEIGHT, delta.y * LINE_HEIGHT),
    }
}

/// The first id of this prefix the document does not already use.
fn free_id(document: &ThemeDocument, prefix: &str) -> String {
    let taken = |id: &str| find_node(&document.nodes, id).is_some();
    let mut index = 1;
    loop {
        let id = format!("{prefix}-{index}");
        if !taken(&id) {
            return id;
        }
        index += 1;
    }
}

fn style_with_fill(color: &str) -> StyleMap {
    let mut style = StyleMap::new();
    style.insert(
        "fill".to_owned(),
        StyleValue::Literal(StyleLiteral {
            value: serde_json::Value::String(color.to_owned()),
        }),
    );
    style
}

fn ellipse_node(document: &ThemeDocument) -> Node {
    Node {
        id: free_id(document, "ellipse"),
        name: None,
        kind: NodeKind::Ellipse,
        transform: place(document, 360.0, 360.0),
        visible: true,
        locked: false,
        style: surface_style(document),
        glass: None,
        children: Vec::new(),
    }
}

fn path_node(document: &ThemeDocument) -> Node {
    let data = "M 0 100 L 180 100 L 180 50 L 360 100 L 180 150 L 180 200 L 0 100 Z";
    path_with(document, "path", data, 360.0, 200.0, true)
}

fn line_node(document: &ThemeDocument) -> Node {
    path_with(document, "line", "M 0 100 L 360 100", 360.0, 200.0, false)
}

fn arc_node(document: &ThemeDocument) -> Node {
    path_with(
        document,
        "arc",
        "M 200 0 A 200 200 0 0 1 0 200",
        200.0,
        200.0,
        false,
    )
}

fn wedge_node(document: &ThemeDocument) -> Node {
    path_with(
        document,
        "wedge",
        "M 0 0 L 200 0 A 200 200 0 0 1 0 200 Z",
        200.0,
        200.0,
        true,
    )
}

/// An open path paints as ink on a stroke; a closed one takes the surface fill.
fn path_with(
    document: &ThemeDocument,
    prefix: &str,
    data: &str,
    width: f64,
    height: f64,
    closed: bool,
) -> Node {
    let style = if closed {
        surface_style(document)
    } else {
        let mut style = StyleMap::new();
        style.insert(
            "stroke".to_owned(),
            ref_or_literal(ink_ref(document), "#ecf5ff"),
        );
        style.insert(
            "strokeWidth".to_owned(),
            StyleValue::Literal(StyleLiteral {
                value: serde_json::json!(2),
            }),
        );
        style
    };
    Node {
        id: free_id(document, prefix),
        name: None,
        kind: NodeKind::Path {
            data: data.to_owned(),
            stroke_width: 2.0,
            closed,
        },
        transform: place(document, width, height),
        visible: true,
        locked: false,
        style,
        glass: None,
        children: Vec::new(),
    }
}

fn vertical_text_node(document: &ThemeDocument) -> Node {
    let mut node = text_node(document);
    if let NodeKind::Text { content } = &mut node.kind {
        content.vertical = true;
    }
    node
}

fn chart_node(document: &ThemeDocument, family: ChartFamily) -> Node {
    let mut settings = serde_json::Map::new();
    let surface = surface_ref(document);
    let ink = ink_ref(document);
    let paints: &[(&str, Option<&String>)] = match family {
        ChartFamily::Gauge => &[("track", surface.as_ref()), ("progress", ink.as_ref())],
        ChartFamily::Line => &[("stroke", ink.as_ref())],
        ChartFamily::Bar => &[("fill", ink.as_ref()), ("track", surface.as_ref())],
        ChartFamily::Pie => &[("remainderFill", surface.as_ref())],
        ChartFamily::Radar => &[("stroke", ink.as_ref())],
        ChartFamily::PolarBar => &[("fill", ink.as_ref()), ("track", surface.as_ref())],
    };
    for (key, reference) in paints {
        if let Some(reference) = reference {
            settings.insert(
                (*key).to_owned(),
                serde_json::json!({ "ref": (**reference).clone() }),
            );
        }
    }
    if family == ChartFamily::Pie
        && let Some(reference) = &ink
    {
        settings.insert("palette".to_owned(), serde_json::json!([reference.clone()]));
    }
    Node {
        id: free_id(document, "chart"),
        name: None,
        kind: NodeKind::Chart {
            content: ChartContent {
                family,
                settings: yunari_model::document::ChartSettings(settings),
            },
        },
        transform: place(document, 240.0, 160.0),
        visible: true,
        locked: false,
        style: StyleMap::new(),
        glass: None,
        children: Vec::new(),
    }
}

/// The card surface the source editor prefers, as a `palette.<id>` reference.
fn surface_ref(document: &ThemeDocument) -> Option<String> {
    palette_ref(
        document,
        &[
            "panel",
            "frost",
            "background",
            "scene",
            "surface",
            "track",
            "chartTrack",
            "charttrack",
        ],
    )
}

/// The ink the source editor prefers, as a `palette.<id>` reference.
fn ink_ref(document: &ThemeDocument) -> Option<String> {
    palette_ref(
        document,
        &["text", "ink", "foreground", "primary", "accent"],
    )
    .or_else(|| {
        let surface = surface_ref(document);
        document
            .palette
            .keys()
            .find(|id| Some(&format!("palette.{id}")) != surface.as_ref() && *id != "none")
            .map(|id| format!("palette.{id}"))
    })
}

fn palette_ref(document: &ThemeDocument, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|name| document.palette.contains_key(**name))
        .map(|name| format!("palette.{name}"))
}

fn ref_or_literal(reference: Option<String>, fallback: &str) -> StyleValue {
    match reference {
        Some(reference) => StyleValue::Ref(StyleRef { r#ref: reference }),
        None => StyleValue::Literal(StyleLiteral {
            value: serde_json::Value::String(fallback.to_owned()),
        }),
    }
}

fn surface_style(document: &ThemeDocument) -> StyleMap {
    let mut style = StyleMap::new();
    style.insert(
        "fill".to_owned(),
        ref_or_literal(surface_ref(document), "#1b2330"),
    );
    style
}

fn rectangle_node(document: &ThemeDocument) -> Node {
    Node {
        id: free_id(document, "rect"),
        name: None,
        kind: NodeKind::Rectangle { corner_radius: 8.0 },
        transform: place(document, 360.0, 200.0),
        visible: true,
        locked: false,
        style: style_with_fill("#4f9cf9"),
        glass: None,
        children: Vec::new(),
    }
}

fn image_node(document: &ThemeDocument, asset_id: &str) -> Node {
    Node {
        id: free_id(document, "image"),
        name: None,
        kind: NodeKind::Image {
            content: ImageContent {
                asset_id: asset_id.to_owned(),
                fit: ImageFit::Contain,
                monochrome: None,
            },
        },
        transform: place(document, 480.0, 320.0),
        visible: true,
        locked: false,
        style: StyleMap::default(),
        glass: None,
        children: Vec::new(),
    }
}

fn text_node(document: &ThemeDocument) -> Node {
    let mut style = style_with_fill("#e8e8ea");
    style.insert(
        "fontSize".to_owned(),
        StyleValue::Literal(StyleLiteral {
            value: serde_json::Value::from(32),
        }),
    );
    Node {
        id: free_id(document, "text"),
        name: None,
        kind: NodeKind::Text {
            content: TextContent {
                runs: vec![TextRun::Literal {
                    text: "New text".to_owned(),
                    type_token: "typeTokens.body".to_owned(),
                    style,
                }],
                box_: None,
                overflow: TextOverflow::Visible,
                align: TextAlign::Left,
                vertical_align: VerticalAlign::Top,
                vertical: false,
            },
        },
        transform: place(document, 240.0, 48.0),
        visible: true,
        locked: false,
        style: StyleMap::new(),
        glass: None,
        children: Vec::new(),
    }
}

/// A line of the snap, dashed 4 on and 4 off so it reads as a guide.
fn paint_guide(guide: &snap::Guide, view: &Viewport, bounds: Bounds<Pixels>, window: &mut Window) {
    const DASH: f32 = 4.0;
    let (from, to) = (guide.from as f32 * view.scale, guide.to as f32 * view.scale);
    let at = guide.at as f32 * view.scale;
    let mut walked = from;
    while walked < to {
        let run = DASH.min(to - walked);
        let quad = match guide.axis {
            snap::Axis::X => Bounds {
                origin: point(
                    bounds.origin.x + px(view.offset.0 + at),
                    bounds.origin.y + px(view.offset.1 + walked),
                ),
                size: size(px(1.0), px(run)),
            },
            snap::Axis::Y => Bounds {
                origin: point(
                    bounds.origin.x + px(view.offset.0 + walked),
                    bounds.origin.y + px(view.offset.1 + at),
                ),
                size: size(px(run), px(1.0)),
            },
        };
        window.paint_quad(PaintQuad {
            bounds: quad,
            corner_radii: Corners::default(),
            background: rgb(0x3d8bf4).into(),
            border_widths: Edges::default(),
            border_color: transparent_black(),
            border_style: BorderStyle::Solid,
        });
        walked += DASH * 2.0;
    }
}

/// A control handle: white with the accent stroke.
fn paint_handle(
    handle: Handle,
    x: f64,
    y: f64,
    view: &Viewport,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let (width, height) = match handle {
        Handle::TopLeft | Handle::TopRight | Handle::BottomRight | Handle::BottomLeft => {
            (12.0, 12.0)
        }
        Handle::Top | Handle::Bottom => (20.0, 8.0),
        Handle::Left | Handle::Right => (8.0, 20.0),
        Handle::Rotate => (32.0, 32.0),
    };
    let screen = view.to_screen(x, y);
    window.paint_quad(PaintQuad {
        bounds: Bounds {
            origin: point(
                bounds.origin.x + screen.x - px(width / 2.0),
                bounds.origin.y + screen.y - px(height / 2.0),
            ),
            size: size(px(width), px(height)),
        },
        corner_radii: Corners::default(),
        background: rgb(0xffffff).into(),
        border_widths: Edges::all(px(1.0)),
        border_color: rgb(ACCENT).into(),
        border_style: BorderStyle::Solid,
    });
}

fn find_node<'a>(nodes: &'a [Node], id: &str) -> Option<&'a Node> {
    for node in nodes {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_node(&node.children, id) {
            return Some(found);
        }
    }
    None
}

/// The pictures a frame can reach: those beside a stored theme, then the bytes
/// the editor is holding, so an import draws before it is saved.
fn images_for(source: &Source, root: &Path, assets: &BTreeMap<String, Vec<u8>>) -> Images {
    let mut images = match source {
        Source::Library { id, .. } => Images::at(root.join(id)),
        Source::Unsaved => Images::empty(),
    };
    for (path, bytes) in assets {
        images.put(path, bytes);
    }
    images
}

/// The layer row's kind mark, from the kit's icon set.
fn layer_icon(kind: &str) -> gpui_kit::component::IconName {
    use gpui_kit::component::IconName;
    match kind {
        "Group" => IconName::Frame,
        "Text" => IconName::ALargeSmall,
        "Chart" => IconName::ChartPie,
        "Image" | "Video" => IconName::GalleryVerticalEnd,
        _ => IconName::Square,
    }
}

/// The id Save As offers: the theme's name as a stable id.
fn suggested_id(document: &ThemeDocument) -> String {
    document
        .metadata
        .name
        .as_deref()
        .and_then(slug)
        .unwrap_or_else(|| document.id.clone())
}

/// A name as a theme id: lowercase, alphanumeric runs joined by one dash.
fn slug(name: &str) -> Option<String> {
    let mut id = String::new();
    let mut dash = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if dash && !id.is_empty() {
                id.push('-');
            }
            dash = false;
            id.push(character.to_ascii_lowercase());
        } else {
            dash = true;
        }
        if id.len() >= 64 {
            break;
        }
    }
    let id = id.trim_end_matches('-').to_owned();
    (!id.is_empty()).then_some(id)
}

/// A package lands beside the library root, named for the theme, so an export
/// never writes inside the library it is exporting from.
fn export_path(root: &Path, document: &ThemeDocument) -> PathBuf {
    let name = slug(&document.metadata.name.clone().unwrap_or_default())
        .unwrap_or_else(|| document.id.clone());
    let home = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    home.unwrap_or(root).join(format!("{name}.yunari-theme"))
}

/// Where an exported widget goes: beside the library, named after the theme.
fn widget_path(root: &Path, document: &ThemeDocument) -> PathBuf {
    let name = slug(&document.metadata.name.clone().unwrap_or_default())
        .unwrap_or_else(|| document.id.clone());
    let home = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    home.unwrap_or(root)
        .join(format!("{name}.yunari-widget-package"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Opens the editor window on this PC's theme library.
pub fn open(cx: &mut App) {
    let _ = open_with(None, cx);
}

/// The editor attached to a running host, so its settings and display records
/// are the ones the host serves. The handle is returned so the app can bring the
/// window back rather than open a second one.
pub fn open_with(host: Option<Shared>, cx: &mut App) -> AnyWindowHandle {
    window_for(host, None, cx)
}

/// Opens the editor with one command already applied, so a row outside the
/// editor (the tray's Store and Settings) lands on the surface it names.
pub fn open_command(host: Option<Shared>, command: Command, cx: &mut App) -> AnyWindowHandle {
    window_for(host, Some(command), cx)
}

fn window_for(host: Option<Shared>, command: Option<Command>, cx: &mut App) -> AnyWindowHandle {
    let library = Library::new(default_root());
    let mut options = TitleBar::window_options();
    options.titlebar = Some(TitlebarOptions {
        title: Some("Yunari — Editor".into()),
        ..TitleBar::title_bar_options()
    });
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        None,
        size(px(1440.0), px(900.0)),
        cx,
    )));
    options.window_min_size = Some(size(px(1024.0), px(700.0)));
    open_window(options, cx, move |window, cx| {
        let editor = cx.new(|cx| EditorWindow::new(library, host, cx));
        if let Some(command) = command {
            editor.update(cx, |editor, cx| editor.apply(command, window, cx));
        }
        editor
    })
    .expect("failed to open the editor window")
    .0
}

#[cfg(test)]
mod tests {
    // Named imports, not the module glob: GPUI exports its own `test`
    // attribute, and a glob would shadow the built-in one.
    use yunari_model::presets::blank;

    use super::{
        INSET, Node, NodeKind, Path, STEP, StyleMap, TextRun, ThemeDocument, Transform, chart_node,
        export_path, free_id, image_node, images_for, line_node, place, rectangle_node, slug,
        starter, suggested_id,
    };

    #[test]
    fn the_canvas_menu_offers_the_sources_object_commands_in_order() {
        use super::{CanvasMenuEntry, OBJECT_MENU};

        let labels: Vec<_> = OBJECT_MENU
            .iter()
            .flat_map(|group| group.iter())
            .map(|entry| entry.label())
            .collect();
        assert_eq!(
            labels,
            [
                "Duplicate",
                "Copy",
                "Cut",
                "Paste",
                "Delete",
                "Group",
                "Ungroup",
                "Bring to front",
                "Bring forward",
                "Send backward",
                "Send to back",
                "Align L",
                "Centre X",
                "Align R",
                "Align T",
                "Centre Y",
                "Align B",
                "Space X",
                "Space Y",
            ]
        );
        assert!(matches!(OBJECT_MENU[2][0], CanvasMenuEntry::Restack(_)));
    }

    #[test]
    fn recents_survive_a_round_trip_through_the_library_parent() {
        use super::{load_recents, note_recent, save_recents};

        let root = std::env::temp_dir().join(format!(
            "yunari-recents-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let library = yunari_library::Library::new(root.join("themes"));
        let mut recents = Vec::new();
        note_recent(&mut recents, "one", "One".to_owned());
        note_recent(&mut recents, "two", "Two".to_owned());
        save_recents(&library, &recents);

        let loaded = load_recents(&library);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "two");
        assert_eq!(loaded[0].name, "Two");
        assert_eq!(loaded[1].id, "one");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_broken_recents_file_reads_as_empty() {
        use super::load_recents;

        let root = std::env::temp_dir().join(format!(
            "yunari-recents-broken-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let library = yunari_library::Library::new(root.join("themes"));
        std::fs::create_dir_all(library.root().parent().unwrap()).unwrap();
        std::fs::write(
            library.root().parent().unwrap().join("recents.json"),
            b"not json",
        )
        .unwrap();
        assert!(load_recents(&library).is_empty());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_recent_list_keeps_the_last_five_opens_newest_first() {
        use super::{RecentTheme, note_recent};

        let mut recents: Vec<RecentTheme> = Vec::new();
        for index in 0..7 {
            note_recent(
                &mut recents,
                &format!("theme-{index}"),
                format!("Theme {index}"),
            );
        }
        assert_eq!(
            recents
                .iter()
                .map(|recent| recent.id.as_str())
                .collect::<Vec<_>>(),
            ["theme-6", "theme-5", "theme-4", "theme-3", "theme-2"]
        );
        // Reopening an old theme moves it to the front rather than repeating it.
        note_recent(&mut recents, "theme-4", "Theme 4 renamed".to_owned());
        assert_eq!(recents.len(), 5);
        assert_eq!(recents[0].id, "theme-4");
        assert_eq!(recents[0].name, "Theme 4 renamed");
        assert_eq!(
            recents
                .iter()
                .filter(|recent| recent.id == "theme-4")
                .count(),
            1
        );
    }

    #[test]
    fn the_inspector_keeps_the_eight_arrange_actions_the_dock_leaves_out() {
        use super::{ARRANGE_ACTIONS, CanvasMenuEntry, DOCK_ACTIONS};

        assert_eq!(
            ARRANGE_ACTIONS.map(|command| command.label()),
            [
                "Align L", "Centre X", "Align R", "Align T", "Centre Y", "Align B", "Space X",
                "Space Y",
            ]
        );
        // The dock must not repeat what the inspector owns.
        for command in ARRANGE_ACTIONS {
            assert!(
                !DOCK_ACTIONS.contains(&CanvasMenuEntry::Command(command)),
                "{command:?} is in both surfaces"
            );
        }
    }

    #[test]
    fn an_imported_picture_draws_before_the_theme_is_saved() {
        use std::collections::BTreeMap;

        use image::ImageEncoder;

        use crate::document::Source;

        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .expect("a picture");
        let assets = BTreeMap::from([("assets/red.png".to_owned(), png)]);
        let mut images = images_for(&Source::Unsaved, Path::new(""), &assets);
        let picture = images.load("assets/red.png").expect("the imported picture");
        assert_eq!((picture.width, picture.height), (1, 1));
        // A path the editor holds nothing for is still absent, not invented.
        assert!(images.load("assets/gone.png").is_none());
    }

    #[test]
    fn placing_an_image_names_the_themes_first_picture() {
        use yunari_model::document::{AssetKind, AssetReference};

        let mut document = blank();
        // Nothing to place yet.
        assert!(crate::assets::first_picture(&document).is_none());
        document.assets.push(AssetReference {
            id: "logo".to_owned(),
            kind: AssetKind::Image,
            path: "assets/logo.png".to_owned(),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        });
        let asset = crate::assets::first_picture(&document).expect("a picture");
        let node = image_node(&document, &asset.id);
        let NodeKind::Image { content } = &node.kind else {
            panic!("an image node");
        };
        assert_eq!(content.asset_id, "logo");
        // Its box is inside the blank artboard.
        assert!(node.transform.x + node.transform.width <= document.artboard.width);
    }

    #[test]
    fn the_blank_theme_holds_the_source_defaults() {
        let document = blank();
        assert_eq!(document.artboard.width, 1920.0);
        assert_eq!(document.artboard.height, 1080.0);
        assert_eq!(document.type_tokens.len(), 3);
        assert!(document.palette.contains_key("none"));
        assert_eq!(document.palette.len(), 8);
        assert!(yunari_model::validate::validate_document(&document).is_ok());
    }

    #[test]
    fn a_new_document_starts_with_a_title_and_a_live_clock() {
        let document = starter();
        assert_eq!(document.nodes.len(), 2);
        let clock = &document.nodes[1];
        let NodeKind::Text { content } = &clock.kind else {
            panic!("the second node is text");
        };
        assert!(matches!(&content.runs[0], TextRun::Value { .. }));
        let bindings = document.bindings.get(&clock.id).expect("a binding");
        assert_eq!(bindings[0].semantic_key, "time.now");
        // The clock is what the sampler must poll for this document.
        let wanted = yunari_render::reading::wanted_keys(&document);
        assert!(
            wanted.iter().any(|key| key == "time.now"),
            "the clock is polled"
        );
    }

    #[test]
    fn a_new_chart_names_the_blank_themes_paints() {
        use yunari_model::document::ChartFamily;

        let document = blank();
        let node = chart_node(&document, ChartFamily::Gauge);
        let NodeKind::Chart { content } = &node.kind else {
            panic!("a chart");
        };
        assert_eq!(content.family, ChartFamily::Gauge);
        assert_eq!(
            content.settings.0["track"],
            serde_json::json!({ "ref": "palette.panel" })
        );
        assert_eq!(
            content.settings.0["progress"],
            serde_json::json!({ "ref": "palette.text" })
        );
    }

    #[test]
    fn an_open_path_draws_its_ink_as_a_stroke() {
        let document = blank();
        let node = line_node(&document);
        let NodeKind::Path { closed, .. } = &node.kind else {
            panic!("a path");
        };
        assert!(!closed);
        assert!(node.style.contains_key("stroke"));
        assert!(!node.style.contains_key("fill"));
    }

    fn document_with(nodes: Vec<Node>) -> ThemeDocument {
        let mut document = blank();
        document.nodes = nodes;
        document
    }

    fn rect(id: &str, x: f64, y: f64, width: f64, height: f64) -> Node {
        Node {
            id: id.to_owned(),
            name: None,
            kind: NodeKind::Rectangle { corner_radius: 0.0 },
            transform: Transform {
                x,
                y,
                width,
                height,
                ..Transform::default()
            },
            visible: true,
            locked: false,
            style: StyleMap::new(),
            glass: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn the_shell_flow_authors_saves_reopens_and_reimports() {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};

        use yunari_library::Library;

        use crate::document::Editor;
        use crate::persistence::import_package;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "yunari-shell-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let library = Library::new(&root);

        let mut editor = Editor::new(blank());
        let node = rectangle_node(editor.document());
        assert_eq!(node.id, "rect-1");
        editor.add_node(node);
        editor.save_as(&library, "first").expect("a first save");
        // Undo empties the artboard; redo brings the rectangle back. The id a
        // save adopted is identity, not an edit, so undo never reverts it.
        assert!(editor.undo());
        assert!(editor.document().nodes.is_empty());
        assert_eq!(editor.document().id, "first");
        assert!(editor.redo());
        assert_eq!(editor.document().nodes.len(), 1);

        // Relaunch: a fresh editor opens the stored theme and finds it again.
        let mut reopened = Editor::new(blank());
        reopened
            .open_stored(&library, "first")
            .expect("a stored theme");
        assert_eq!(reopened.document().nodes.len(), 1);

        // Export then import: the copy gets a free id and the same artboard.
        let bytes = reopened.export().expect("a package");
        let id = import_package(&library, &bytes).expect("an imported copy");
        assert_eq!(id, "first-2");
        let mut imported = Editor::new(blank());
        imported
            .open_stored(&library, &id)
            .expect("an imported theme");
        assert_eq!(imported.document().nodes.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_new_node_gets_an_id_the_document_does_not_use() {
        let document = document_with(vec![rect("rect-1", 0.0, 0.0, 1.0, 1.0)]);
        assert_eq!(free_id(&document, "rect"), "rect-2");
    }

    #[test]
    fn each_add_steps_away_from_the_one_before() {
        let document = document_with(vec![rect("a", 0.0, 0.0, 1.0, 1.0)]);
        let placement = place(&document, 10.0, 10.0);
        assert_eq!(placement.x, INSET + STEP);
        assert_eq!(placement.width, 10.0);
    }

    #[test]
    fn a_theme_name_becomes_a_stable_id() {
        assert_eq!(slug("My Theme!"), Some("my-theme".to_owned()));
        assert_eq!(slug("  --  "), None);
        assert_eq!(suggested_id(&blank()), "untitled");
    }

    #[test]
    fn a_package_lands_beside_the_library_not_inside_it() {
        let root = Path::new("C:/Users/me/.yunari/themes");
        let mut document = blank();
        document.metadata.name = Some("Kitchen".to_owned());
        let path = export_path(root, &document);
        assert_eq!(path, Path::new("C:/Users/me/.yunari/kitchen.yunari-theme"));
    }
}
