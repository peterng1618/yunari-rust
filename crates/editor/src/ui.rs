//! The editor window: a command bar, the artboard, and the dialogs a shell
//! needs to open, save, export and import a theme.
//!
//! The window owns the authoring state; every mutation goes through [`Editor`],
//! so undo, the dirty dot and what a save writes cannot disagree.

pub mod library_dialog;
pub mod menus;
pub mod prompt;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use yunari_library::{Library, LibraryError, default_root};
use yunari_model::document::{
    Artboard, ArtboardShape, Background, ChartContent, ChartFamily, ContentFit, Metadata, Node,
    NodeKind, Paint, PaletteEntry, StyleLiteral, StyleMap, StyleRef, StyleValue, TextAlign,
    TextContent, TextOverflow, TextRun, ThemeDocument, Transform, TypeToken, TypeWeight,
    VerticalAlign,
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
use crate::arrange;
use crate::document::{Editor, Source};
use crate::inspector;
use crate::palette;
use crate::layers::{self, Move as LayerMove};
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
    Library(LibraryDialog),
    Rename(Prompt),
    Field(Prompt),
    /// An action that would throw unsaved work away, waiting for an answer.
    Confirm { message: String, resume: Resume },
}

/// The action a guard is holding.
#[derive(Clone, PartialEq, Eq)]
enum Resume {
    New,
    Library,
    Import(String),
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
}

impl EditorWindow {
    fn new(library: Library, cx: &mut Context<Self>) -> Self {
        let document = blank();
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

    /// Pictures are read from beside the theme that names them.
    fn point_images(&mut self) {
        let images = match self.editor.source() {
            Source::Library { id, .. } => Images::at(self.library.root().join(id)),
            Source::Unsaved => Images::empty(),
        };
        self.images = Rc::new(RefCell::new(images));
    }

    fn apply(&mut self, command: Command, cx: &mut Context<Self>) {
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
            Command::Import => {
                self.overlay = Some(Overlay::Import(Prompt::new(
                    "Import package",
                    "Path to a .yunari-theme file",
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
                        self.status =
                            Some(Status::error("Grouping needs two unlocked nodes"));
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
        }
        cx.notify();
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
        self.editor.edit(|document| moved = arrange::apply(document, &ids, action));
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
                self.editor = Editor::new(blank());
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
        let local = point(event.position.x - bounds.origin.x, event.position.y - bounds.origin.y);
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
                let locked = selection::find(self.editor.document(), &id)
                    .is_some_and(|node| node.locked);
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
        let local = point(event.position.x - bounds.origin.x, event.position.y - bounds.origin.y);
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
                let raw = (
                    anchor.0 + scene.0 - start.0,
                    anchor.1 + scene.1 - start.1,
                );
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
                let x = if guided(snap::Axis::X) { snapped_x } else { rounded_x };
                let y = if guided(snap::Axis::Y) { snapped_y } else { rounded_y };
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
            self.status = Some(Status::error("Names are one line and at most 120 characters"));
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
                self.editor.add_token(value).map(|_| ()).ok_or_else(|| {
                    "A new type token needs a name".to_owned()
                })
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
            self.editor.edit(|document| outcome = inspector::apply_document(document, &key, value));
        } else if key == "bindings.new" {
            outcome = match self.selection.first().map(str::to_owned) {
                Some(id) => self.editor.add_binding(&id, value).map(|_| ()),
                None => Err("Select a node to bind a reading".to_owned()),
            };
        } else if let Some(id) = self.selection.first().map(str::to_owned) {
            self.editor.edit(|document| outcome = inspector::apply(document, &id, &key, value));
        } else {
            outcome = Err("Select a node to edit its fields".to_owned());
        }
        match outcome {
            Ok(()) => {
                self.status = Some(Status::ok(format!("Set {}", key.rsplit('.').next().unwrap_or(&key))));
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
        for (index, (id, token)) in self.editor.document().type_tokens.clone().into_iter().enumerate()
        {
            let family_key = format!("token.{id}.family");
            let weight_key = format!("token.{id}.weight");
            let family = token.family.clone();
            let weight = token.weight.as_ref().map_or_else(String::new, tokens::weight_text);
            let id_for_family = id.clone();
            let (family_for_edit, weight_for_edit) = (family.clone(), weight.clone());            children.push(
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
                        this.inspector_scroll =
                            (this.inspector_scroll - dy).clamp(0.0, max_scroll);
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
                    (
                        transform.x,
                        transform.y,
                        transform.width,
                        transform.height,
                    ),
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
        let (x0, y0) = self
            .viewport
            .to_scene(point(px(anchor.0), px(anchor.1)));
        let (x1, y1) = self
            .viewport
            .to_scene(point(px(current.0), px(current.1)));
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
                self.apply(command, cx);
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
            Overlay::Confirm { message, resume } => {
                if keystroke.key == "escape" {
                    self.overlay = None;
                } else if keystroke.key == "enter" {
                    self.discard(resume, window, cx);
                } else {
                    self.overlay = Some(Overlay::Confirm { message, resume });
                }
            }
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
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.apply(command, cx)),
                    )
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
                    .bg(if row.selected { rgb(0x2b2b36) } else { rgb(PANEL) })
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
                    .filter(|(handle, _)| {
                        *handle == Handle::Rotate || rotation.abs() < 1e-6
                    })
                    .collect()
            })
            .unwrap_or_default();
        let view = self.viewport;
        div()
            .id("canvas")
            .flex_1()
            .bg(rgb(0x101014))
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                this.on_scroll(event, cx)
            }))
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
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.pan_start(event.position)
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
                    .text_color(if status.error { rgb(0xf38ba8) } else { rgb(0xa6e3a1) })
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
            .child(div().text_sm().text_color(rgb(MUTED)).child(prompt.hint.clone()))
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
            Some(Overlay::Import(prompt)) => Self::prompt_panel(prompt, cx),
            Some(Overlay::Library(dialog)) => library_dialog::view(dialog, cx).into_any_element(),
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
                                    .on_click(
                                        cx.listener(move |this, _: &ClickEvent, window, cx| {
                                            this.discard(discard.clone(), window, cx)
                                        }),
                                    ),
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
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, _, cx| this.close_overlay(cx),
                                    )),
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

/// The theme an author starts from, matching the source editor's blank theme:
/// the three role tokens and the eight paint tokens every card expects, on a
/// 1920 x 1080 artboard.
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
    path_with(document, "arc", "M 200 0 A 200 200 0 0 1 0 200", 200.0, 200.0, false)
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
    palette_ref(document, &["text", "ink", "foreground", "primary", "accent"]).or_else(|| {
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
        kind: NodeKind::Rectangle {
            corner_radius: 8.0,
        },
        transform: place(document, 360.0, 200.0),
        visible: true,
        locked: false,
        style: style_with_fill("#4f9cf9"),
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
                origin: point(bounds.origin.x + px(view.offset.0 + at), bounds.origin.y + px(view.offset.1 + walked)),
                size: size(px(1.0), px(run)),
            },
            snap::Axis::Y => Bounds {
                origin: point(bounds.origin.x + px(view.offset.0 + walked), bounds.origin.y + px(view.offset.1 + at)),
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
        Handle::TopLeft | Handle::TopRight | Handle::BottomRight | Handle::BottomLeft => (12.0, 12.0),
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
    let home = root.parent().filter(|parent| !parent.as_os_str().is_empty());
    home.unwrap_or(root)
        .join(format!("{name}.yunari-theme"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Opens the editor window on this PC's theme library.
pub fn open(cx: &mut App) {
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
        move |_, cx| cx.new(|cx| EditorWindow::new(library, cx)),
    )
    .expect("failed to open the editor window");
}

#[cfg(test)]
mod tests {
    // Named imports, not the module glob: GPUI exports its own `test`
    // attribute, and a glob would shadow the built-in one.
    use super::{
        INSET, STEP, Node, NodeKind, Path, StyleMap, ThemeDocument, Transform, blank, chart_node,
        export_path, free_id, line_node, place, rectangle_node, slug, suggested_id,
    };

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
            kind: NodeKind::Rectangle {
                corner_radius: 0.0,
            },
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
