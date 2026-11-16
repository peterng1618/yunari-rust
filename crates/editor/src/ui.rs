//! The editor window: a command bar, the artboard, and the dialogs a shell
//! needs to open, save, export and import a theme.
//!
//! The window owns the authoring state; every mutation goes through [`Editor`],
//! so undo, the dirty dot and what a save writes cannot disagree.

pub mod library_dialog;
pub mod menus;
pub mod prompt;
pub mod settings_dialog;
pub mod store_dialog;
pub mod widget_dialog;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use yunari_host::http::Shared;
use yunari_library::{Library, LibraryError, default_root};
use yunari_model::document::{
    Artboard, ArtboardShape, Background, Binding, ChartContent, ChartFamily, ContentFit,
    ImageContent, ImageFit, Metadata, Node, NodeKind, Paint, PaletteEntry, StyleLiteral, StyleMap,
    StyleRef, StyleValue, TextAlign, TextContent, TextOverflow, TextRun, ThemeDocument, Transform,
    TypeToken, TypeWeight, VerticalAlign,
};
use yunari_providers::clock::ClockProvider;
use yunari_providers::registry::Registry;
use yunari_providers::sampler::Sampler;
use yunari_providers::system::SystemProvider;
use yunari_providers::units::UnitPreferences;
use yunari_render::plan::PlanIssue;
use yunari_render::{DisplayList, Fonts, Readings};
use yunari_render_raster::Images;

use self::library_dialog::LibraryDialog;
use self::menus::Command;
use self::prompt::{Outcome, Prompt};
use self::settings_dialog::SettingsDialog;
use self::store_dialog::StoreDialog;
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
    Settings(Box<SettingsDialog>),
    Store(StoreDialog),
    Rename(Prompt),
    Field(Prompt),
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
    /// An action that would throw unsaved work away, waiting for an answer.
    Confirm {
        message: String,
        resume: Resume,
    },
}

/// The action a guard is holding.
#[derive(Clone, PartialEq, Eq)]
enum Resume {
    New,
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
    /// Bumped per arrow press so a stale idle timer ends nothing.
    nudge_epoch: u64,
    /// Group rows the layers panel is showing the children of.
    expanded: BTreeSet<String>,
    /// The row a rename prompt is editing.
    renaming: Option<String>,
    /// The inspector key a field prompt is editing.
    editing: Option<String>,
    overlay: Option<Overlay>,
    status: Option<Status>,
    focus: FocusHandle,
    /// Where the canvas was last painted, so a click can be turned into an
    /// artboard point.
    canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
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
    /// How far the inspector's rows are pushed up, since gpui-kit has no
    /// scroll container of its own.
    inspector_scroll: f32,
    /// The running host, when this editor is attached to one.
    host: Option<Shared>,
}

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
        Self {
            editor: Editor::new(document),
            library,
            fonts,
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
            nudge_epoch: 0,
            expanded: BTreeSet::new(),
            renaming: None,
            editing: None,
            overlay: None,
            status: None,
            focus: cx.focus_handle(),
            canvas_bounds: Rc::new(Cell::new(None)),
            viewport: Viewport {
                scale: 1.0,
                offset: (0.0, 0.0),
            },
            camera_touched: false,
            panning: None,
            space_held: false,
            polling: false,
            inspector_scroll: 0.0,
            host,
        }
    }

    /// Rebuilds the frame and asks for exactly the keys this document names.
    fn refresh(&mut self) {
        self.selection.retain_existing(self.editor.document());
        let (list, issues) = yunari_render::build_with(
            self.editor.document(),
            &mut self.fonts,
            &self.readings,
            &self.preferences,
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
    }

    fn apply(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            Command::New => self.guard(Resume::New, cx),
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
                let options = self
                    .host
                    .as_ref()
                    .map(|host| host.registry.devices())
                    .unwrap_or_default();
                self.overlay = Some(Overlay::Settings(Box::new(SettingsDialog::new(
                    self.settings_dir(),
                    monitors,
                    options,
                ))));
            }
            Command::Widgets => {
                self.overlay = Some(Overlay::Widgets(WidgetDialog::new(&self.packs_directory())));
            }
            Command::Store => self.open_store(window, cx),
            Command::ImportAsset => {
                self.overlay = Some(Overlay::ImportAsset(Prompt::new(
                    "Import asset",
                    "Path to a picture on this PC",
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
        if let Some(Overlay::Settings(dialog)) = &mut self.overlay {
            dialog.monitors = monitors;
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

    /// Takes a file from this PC into the open theme. The bytes are held until
    /// the next save, so an imported picture draws before the theme is stored.
    fn import_asset(&mut self, value: &str) {
        self.overlay = None;
        let path = value.trim();
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

    /// Opens a stored theme, its pictures and all, replacing the open one.
    fn open_entry(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.editor.open_stored(&self.library, id) {
            Ok(()) => {
                self.selection.clear();
                self.overlay = None;
                self.camera_touched = false;
                self.status = Some(Status::ok(format!("Opened {id}")));
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
            Resume::New => {
                self.editor = Editor::new(starter());
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

    /// Opens the store's catalogue and whatever this account has published,
    /// reading both off the UI thread.
    fn open_store(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            self.status = Some(Status::error(
                "The theme store needs the host running on this PC.",
            ));
            cx.notify();
            return;
        };
        let email = host.account.sessions.read().map(|session| session.email);
        self.overlay = Some(Overlay::Store(StoreDialog::new(email)));
        cx.notify();
        self.load_store(&host, window, cx);
    }

    fn refresh_store(&mut self, cx: &mut Context<Self>, window: &mut Window) {
        let Some(host) = self.host.clone() else {
            return;
        };
        if let Some(Overlay::Store(dialog)) = &mut self.overlay {
            dialog.loading = true;
            dialog.message = None;
        }
        cx.notify();
        self.load_store(&host, window, cx);
    }

    fn load_store(&mut self, host: &Shared, window: &mut Window, cx: &mut Context<Self>) {
        let host = std::sync::Arc::clone(host);
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
                let Some(Overlay::Store(dialog)) = &mut this.overlay else {
                    return;
                };
                dialog.loading = false;
                match catalogue {
                    Ok(catalogue) => dialog.rows = store_dialog::rows_of(&catalogue),
                    Err(message) => dialog.say(message),
                }
                match mine {
                    Some(Ok(mine)) => dialog.mine = store_dialog::rows_of(&mine),
                    Some(Err(error)) => dialog.say(error.message),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Installs one listing through the host's install path, which checks the
    /// package against the size and digest the listing recorded.
    fn install_store_item(&mut self, id: String, cx: &mut Context<Self>, window: &mut Window) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some(row) = (match &self.overlay {
            Some(Overlay::Store(dialog)) => dialog.row(&id).cloned(),
            _ => None,
        }) else {
            return;
        };
        let Some(request) = yunari_host::store_install::parse_request(&serde_json::json!({
            "packageUrl": row.package_url,
            "catalogueId": row.id,
            "kind": row.kind,
            "version": row.version,
            "sha256": row.sha256,
            "bytes": row.bytes,
        })) else {
            if let Some(Overlay::Store(dialog)) = &mut self.overlay {
                dialog.say("That listing does not record everything an install needs.");
            }
            cx.notify();
            return;
        };
        if let Some(Overlay::Store(dialog)) = &mut self.overlay {
            dialog.say("Installing…");
        }
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
                let message = match outcome {
                    Ok(installed) => format!("Installed as {}.", installed.id),
                    Err(refusal) => refusal.message,
                };
                this.refresh_library();
                if let Some(Overlay::Store(dialog)) = &mut this.overlay {
                    dialog.say(message);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Publishes or withdraws one of this account's items.
    fn store_transition(
        &mut self,
        id: String,
        publish: bool,
        cx: &mut Context<Self>,
        window: &mut Window,
    ) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let Some(client) = host.account.client() else {
            if let Some(Overlay::Store(dialog)) = &mut self.overlay {
                dialog.say("Sign in to the Yunari store first.");
            }
            cx.notify();
            return;
        };
        let target = id.clone();
        cx.spawn_in(window, async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    if publish {
                        client.publish(&target)
                    } else {
                        client.unpublish(&target)
                    }
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                let message = match outcome {
                    Ok(transitioned) => {
                        if let Some(Overlay::Store(dialog)) = &mut this.overlay
                            && let Some(row) = dialog.mine.iter_mut().find(|row| row.id == id)
                        {
                            row.status = Some(transitioned.status.clone());
                        }
                        format!("That item is now {}.", transitioned.status)
                    }
                    Err(error) => error.message,
                };
                if let Some(Overlay::Store(dialog)) = &mut this.overlay {
                    dialog.say(message);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Submits the stored theme this window has open, through the version
    /// ladder the host works out against the item's stored baseline.
    fn submit_current_theme(&mut self, cx: &mut Context<Self>, window: &mut Window) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let message = if self.editor.is_dirty() {
            Some("Save this theme before submitting it.".to_owned())
        } else {
            match self.editor.source() {
                Source::Library { id, .. } => {
                    self.submit_theme(host, id.clone(), cx, window);
                    return;
                }
                Source::Unsaved => Some("Save this theme to the library first.".to_owned()),
            }
        };
        if let Some(message) = message
            && let Some(Overlay::Store(dialog)) = &mut self.overlay
        {
            dialog.say(message);
        }
        cx.notify();
    }

    fn submit_theme(
        &mut self,
        host: Shared,
        id: String,
        cx: &mut Context<Self>,
        window: &mut Window,
    ) {
        if let Some(Overlay::Store(dialog)) = &mut self.overlay {
            dialog.say("Submitting…");
        }
        cx.notify();
        cx.spawn_in(window, async move |weak, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { yunari_host::submit::submit(&host, &id) })
                .await;
            let _ = weak.update(cx, |this, cx| {
                let message = match outcome {
                    Ok(submitted) => match submitted.from {
                        Some(from) => format!(
                            "Submitted {from} → {} as {}.",
                            submitted.to, submitted.status
                        ),
                        None => format!("Submitted {} as {}.", submitted.to, submitted.status),
                    },
                    Err(refusal) => refusal.message,
                };
                if let Some(Overlay::Store(dialog)) = &mut this.overlay {
                    dialog.say(message);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn store_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let port = host.bound_port.load(std::sync::atomic::Ordering::Relaxed);
        let message = if port == 0 {
            "The host is not listening on this PC yet.".to_owned()
        } else {
            match yunari_host::store_routes::start_url(&host, &format!("http://127.0.0.1:{port}")) {
                Ok(url) => {
                    // The address is built by this host, never typed here.
                    let _ = std::process::Command::new("explorer").arg(&url).spawn();
                    "Finish signing in in your browser, then press Refresh.".to_owned()
                }
                Err(message) => message,
            }
        };
        if let Some(Overlay::Store(dialog)) = &mut self.overlay {
            dialog.say(message);
        }
        cx.notify();
    }

    fn store_sign_out(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let message = match host.account.sessions.clear() {
            Ok(()) => "Signed out.".to_owned(),
            Err(error) => error,
        };
        if let Some(Overlay::Store(dialog)) = &mut self.overlay {
            dialog.email = None;
            dialog.mine.clear();
            dialog.say(message);
        }
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
    fn canvas_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
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
                let (snapped_x, snapped_y, found) = snap::snap_move(
                    self.editor.document(),
                    &moving,
                    raw,
                    self.viewport.scale,
                    enabled,
                );
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
                starts
                    .iter()
                    .map(|(id, transform)| {
                        let mut next = *transform;
                        next.x += dx;
                        next.y += dy;
                        (id.clone(), next)
                    })
                    .collect()
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

    fn begin_field(&mut self, key: &str, label: &str, value: &str, cx: &mut Context<Self>) {
        self.editing = Some(key.to_owned());
        self.overlay = Some(Overlay::Field(Prompt::new(
            format!("Edit {label}"),
            "Enter applies, Esc cancels",
            value,
        )));
        cx.notify();
    }

    fn apply_field(&mut self, value: &str) {
        let Some(key) = self.editing.take() else {
            return;
        };
        let mut outcome = Ok(());
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

    fn inspector_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sections = match self.selection.first() {
            Some(id) => inspector::sections(self.editor.document(), id),
            None => inspector::document_sections(self.editor.document()),
        };
        let mut children: Vec<AnyElement> = Vec::new();
        if sections.is_empty() {
            children.push(
                div()
                    .p_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Select a node to edit it")
                    .into_any_element(),
            );
        }
        let palette = self.editor.document().palette.clone();
        for (section_index, section) in sections.into_iter().enumerate() {
            children.push(
                div()
                    .px_2()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
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
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .text_sm()
                        .child(div().text_color(rgb(MUTED)).child(label))
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_sm()
                                .bg(rgb(0x101014))
                                .text_color(rgb(INK))
                                .child(value.clone()),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.begin_field(&key, label, &value, cx)
                        }))
                        .into_any_element(),
                );
            }
        }
        if self.selection.first().is_some() {
            children.push(
                div()
                    .id("binding-add")
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .cursor_pointer()
                    .child("+ Add binding")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.begin_field("bindings.new", "semantic key", "", cx)
                    }))
                    .into_any_element(),
            );
        }
        children.push(
            div()
                .px_2()
                .pt_3()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Palette")
                .into_any_element(),
        );
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
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(
                        div()
                            .id(("palette-name", index))
                            .flex_1()
                            .text_color(rgb(INK))
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
                            .bg(rgb(0x101014))
                            .text_color(rgb(INK))
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
                .px_2()
                .py_1()
                .text_sm()
                .text_color(rgb(MUTED))
                .cursor_pointer()
                .child("+ Add colour")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.begin_field("palette.new", "new colour", "", cx)
                }))
                .into_any_element(),
        );
        children.push(
            div()
                .px_2()
                .pt_3()
                .text_xs()
                .text_color(rgb(MUTED))
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
                    .px_2()
                    .py_1()
                    .text_sm()
                    .child(div().text_color(rgb(MUTED)).child(id_for_family))
                    .child(
                        div()
                            .id(("token-family", index))
                            .px_1()
                            .text_color(rgb(INK))
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
                            .text_color(rgb(INK))
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
                .px_2()
                .py_1()
                .text_sm()
                .text_color(rgb(MUTED))
                .cursor_pointer()
                .child("+ Add type token")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.begin_field("token.new", "new type token", "", cx)
                }))
                .into_any_element(),
        );
        // The rows are one column pushed up by the wheel: kit's div has no
        // scroll container, so the panel keeps the offset and clips.
        let max_scroll = (children.len() as f32 * 30.0 - 500.0).max(0.0);
        div()
            .w(px(260.0))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(0x2e2e38))
            .bg(rgb(PANEL))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Inspector"),
            )
            .child(
                div()
                    .id("inspector-body")
                    .flex_1()
                    .w_full()
                    .overflow_hidden()
                    .on_scroll(cx.listener(move |this, event: &ScrollEvent, _, cx| {
                        let (_, dy) = scroll_delta(event);
                        this.inspector_scroll = (this.inspector_scroll - dy).clamp(0.0, max_scroll);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .mt(px(-self.inspector_scroll))
                            .children(children),
                    ),
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
            if self.nudge(&keystroke.key, keystroke.modifiers.shift, window, cx) {
                return;
            }
            if keystroke.modifiers.control && keystroke.key == "]" {
                if let Some(id) = self.selection.first().map(str::to_owned) {
                    self.editor.raise(&id, LayerMove::Front);
                    self.refresh();
                    cx.notify();
                }
                return;
            }
            if keystroke.modifiers.control && keystroke.key == "[" {
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
            Overlay::Widgets(_) | Overlay::Settings(_) | Overlay::Store(_) => {
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
            Overlay::Field(mut prompt) => {
                match prompt.key(&keystroke.key, keystroke.key_char.as_deref()) {
                    Outcome::Pending => self.overlay = Some(Overlay::Field(prompt)),
                    Outcome::Cancel => self.editing = None,
                    Outcome::Submit(value) => self.apply_field(&value),
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

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons: Vec<_> = Command::ALL
            .into_iter()
            .enumerate()
            .map(|(index, command)| {
                div()
                    .id(("command", index))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(INK))
                    .child(command.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.apply(command, window, cx)
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .p_2()
            .bg(rgb(PANEL))
            .children(buttons)
    }

    fn layers_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = layers::rows(self.editor.document(), &self.selection, &self.expanded);
        let items: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let id = row.id.clone();
                let caret_id = id.clone();
                let eye_id = id.clone();
                let lock_id = id.clone();
                let group = row.kind == "Group";
                let caret = if !group {
                    " "
                } else if self.expanded.contains(&id) {
                    "v"
                } else {
                    ">"
                };
                let eye = if row.visible { "V" } else { "-" };
                let lock = if row.locked { "L" } else { "." };
                div()
                    .id(("layer", index))
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(6.0 + row.depth as f32 * 12.0))
                    .pr_2()
                    .py_1()
                    .bg(if row.selected {
                        rgb(0x2b2b36)
                    } else {
                        rgb(PANEL)
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .id(("caret", index))
                            .w(px(12.0))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(caret)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.toggle_expanded(&caret_id, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(if row.visible { rgb(INK) } else { rgb(MUTED) })
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .id(("eye", index))
                            .px_1()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(eye)
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
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(lock)
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
            .w(px(220.0))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(0x2e2e38))
            .bg(rgb(PANEL))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child("Layers"),
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
            .flex_1()
            .bg(rgb(0x101014))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| this.on_scroll(event, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
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
                        yunari_render_gpui::paint_fitted(&list, fit, window, cx, &mut store);
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
    }

    fn status_view(&self) -> impl IntoElement {
        let document = self.editor.document();
        let name = document
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| document.id.clone());
        let state = if self.editor.is_dirty() {
            "unsaved changes"
        } else {
            "saved"
        };
        let mut line = div()
            .h(px(26.0))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(rgb(PANEL))
            .text_sm()
            .text_color(rgb(MUTED))
            .child(format!("{name} · {state}"))
            .child(match self.editor.source() {
                Source::Library { id, .. } => format!("· {id}"),
                Source::Unsaved => "· not in the library".to_owned(),
            });
        let zoom = format!("{:.0}%", self.viewport.scale * 100.0);
        line = line.child(zoom);
        if !self.selection.is_empty() {
            line = line.child(format!("{} selected", self.selection.len()));
        }
        if let Some(status) = &self.status {
            line = line.child(
                div()
                    .text_color(if status.error {
                        rgb(0xf38ba8)
                    } else {
                        rgb(0xa6e3a1)
                    })
                    .child(status.text.clone()),
            );
        }
        line
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
            Some(Overlay::Field(prompt)) => self.apply_field(&prompt.value),
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
            Some(Overlay::WeatherPlace { prompt, .. } | Overlay::Credential { prompt, .. }) => {
                Self::prompt_panel(prompt, cx)
            }
            Some(Overlay::Import(prompt) | Overlay::ImportAsset(prompt)) => {
                Self::prompt_panel(prompt, cx)
            }
            Some(Overlay::Library(dialog)) => library_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::Widgets(dialog)) => widget_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::Settings(dialog)) => settings_dialog::view(dialog, cx).into_any_element(),
            Some(Overlay::Store(dialog)) => store_dialog::view(dialog, cx).into_any_element(),
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
                    cx.background_executor().timer(TICK).await;
                    let alive = this.update(cx, |this, cx| {
                        if this.poll(now_ms()) {
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
        let mut rows: Vec<AnyElement> = Vec::new();
        rows.push(self.toolbar(cx).into_any_element());
        rows.push(
            div()
                .flex()
                .flex_1()
                .w_full()
                .min_h(px(0.0))
                .overflow_hidden()
                .child(self.layers_view(cx))
                .child(self.canvas_view(cx))
                .child(self.inspector_view(cx))
                .into_any_element(),
        );
        rows.push(self.status_view().into_any_element());
        if let Some(overlay) = self.overlay_view(cx) {
            rows.push(overlay);
        }
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(0x15151a))
            .text_color(rgb(INK))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(Self::on_key_up))
            .children(rows)
    }
}

/// The theme a new document starts from: a title and a clock reading live, so
/// the first frame shows a working value rather than an empty board. The tokens
/// are the source editor's blank theme.
fn starter() -> ThemeDocument {
    let mut document = blank();
    let mut title = text_node(&document);
    title.id = free_id(&document, "title");
    title.transform = Transform {
        x: 96.0,
        y: 88.0,
        width: 720.0,
        height: 80.0,
        ..Transform::default()
    };
    if let NodeKind::Text { content } = &mut title.kind {
        content.runs = vec![TextRun::Literal {
            text: "Untitled".to_owned(),
            type_token: "typeTokens.heading".to_owned(),
            style: style_with_fill("#e8e8ea"),
        }];
    }
    document.nodes.push(title);

    let mut clock = text_node(&document);
    clock.id = free_id(&document, "clock");
    clock.transform = Transform {
        x: 96.0,
        y: 176.0,
        width: 360.0,
        height: 96.0,
        ..Transform::default()
    };
    if let NodeKind::Text { content } = &mut clock.kind {
        content.runs = vec![TextRun::Value {
            binding_id: "clock-value".to_owned(),
            precision: None,
            unit_display: None,
            type_token: "typeTokens.heading".to_owned(),
            style: style_with_fill("#ecf5ff"),
        }];
    }
    let clock_id = clock.id.clone();
    document.nodes.push(clock);
    document.bindings.insert(
        clock_id,
        vec![Binding {
            id: "clock-value".to_owned(),
            semantic_key: "time.now".to_owned(),
            precision: None,
            unit_display: None,
            scale: None,
            offset: None,
            unit: None,
            auto_scale: None,
            format: None,
            time_zone: None,
        }],
    );
    document
}

/// The tokens and artboard a new document shares, matching the source editor's
/// blank theme: the three role tokens and the eight paint tokens every card
/// expects, on a 1920 x 1080 artboard.
fn blank() -> ThemeDocument {
    let mut type_tokens = BTreeMap::new();
    for (id, family, weight) in [
        ("heading", "Segoe UI, sans-serif", 600.0),
        ("body", "Segoe UI, sans-serif", 400.0),
        ("mono", "Consolas, monospace", 400.0),
    ] {
        type_tokens.insert(
            id.to_owned(),
            TypeToken {
                family: family.to_owned(),
                weight: Some(TypeWeight::Number(weight)),
                face: None,
            },
        );
    }
    let mut palette = BTreeMap::new();
    for (id, name, colour) in [
        ("none", "None", "#00000000"),
        ("text", "Text", "#ecf5ff"),
        ("dim", "Muted text", "#a8bed0"),
        ("panel", "Panel", "#081523d9"),
        ("frost", "Frosted panel", "#0815234d"),
        ("panelStroke", "Panel outline", "#9fc7e52b"),
        ("rule", "Rule", "#7dbde0"),
        ("chartTrack", "Chart track", "#223047"),
    ] {
        palette.insert(
            id.to_owned(),
            PaletteEntry {
                name: name.to_owned(),
                value: Paint::Solid {
                    color: colour.to_owned(),
                },
            },
        );
    }
    ThemeDocument {
        schema_version: "1.0.0".to_owned(),
        id: "untitled".to_owned(),
        metadata: Metadata {
            name: Some("Untitled".to_owned()),
            ..Metadata::default()
        },
        artboard: Artboard {
            width: 1920.0,
            height: 1080.0,
            content_fit: ContentFit::Contain,
            shape: ArtboardShape::Rect,
        },
        background: Background {
            mode: Default::default(),
            color: Some(StyleValue::Ref(StyleRef {
                r#ref: "palette.chartTrack".to_owned(),
            })),
            media: None,
        },
        palette,
        type_tokens,
        assets: Vec::new(),
        nodes: Vec::new(),
        bindings: BTreeMap::new(),
        editor_metadata: BTreeMap::new(),
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
    open_with(None, cx);
}

/// The editor attached to a running host, so its settings and display records
/// are the ones the host serves.
pub fn open_with(host: Option<Shared>, cx: &mut App) {
    let library = Library::new(default_root());
    open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Yunari — Editor".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(960.0), px(640.0)),
                cx,
            ))),
            ..Default::default()
        },
        cx,
        move |_, cx| cx.new(|cx| EditorWindow::new(library, host, cx)),
    )
    .expect("failed to open the editor window");
}

#[cfg(test)]
mod tests {
    // Named imports, not the module glob: GPUI exports its own `test`
    // attribute, and a glob would shadow the built-in one.
    use super::{
        INSET, Node, NodeKind, Path, STEP, StyleMap, TextRun, ThemeDocument, Transform, blank,
        chart_node, export_path, free_id, image_node, images_for, line_node, place, rectangle_node,
        slug, starter, suggested_id,
    };

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
