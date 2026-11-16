//! The editor window: a command bar, the artboard, and the dialogs a shell
//! needs to open, save, export and import a theme.
//!
//! The window owns the authoring state; every mutation goes through [`Editor`],
//! so undo, the dirty dot and what a save writes cannot disagree.

pub mod library_dialog;
pub mod menus;
pub mod prompt;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use yunari_library::{Library, LibraryError, default_root};
use yunari_model::document::{
    Artboard, ArtboardShape, Background, ContentFit, Metadata, Node, NodeKind, StyleLiteral,
    StyleMap, StyleValue, TextAlign, TextContent, TextOverflow, TextRun, ThemeDocument, Transform,
    TypeToken, VerticalAlign,
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
use crate::document::{Editor, Source};
use crate::persistence::import_package;
use crate::viewport::{LINE_HEIGHT, Viewport, ZOOM_STEP, wheel_factor};

/// How often the window looks at the sampler; the sampler decides when a poll
/// is actually due, so this only bounds how late a new reading can be.
const TICK: Duration = Duration::from_millis(250);

const ACCENT: u32 = 0x4f9cf9;
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
    selection: Option<String>,
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
            selection: None,
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
        }
    }

    /// Rebuilds the frame and asks for exactly the keys this document names.
    fn refresh(&mut self) {
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
            Command::AddText => self.add(text_node),
            Command::DeleteSelected => {
                if let Some(id) = self.selection.take() {
                    self.editor.remove_nodes(&[id]);
                    self.refresh();
                }
            }
        }
        cx.notify();
    }

    fn add(&mut self, make: fn(&ThemeDocument) -> Node) {
        let node = make(self.editor.document());
        self.selection = Some(node.id.clone());
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
                self.selection = None;
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
                self.selection = None;
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

    /// The selection's box in artboard units, for the outline.
    fn selection_rect(&self) -> Option<(f64, f64, f64, f64)> {
        let id = self.selection.as_deref()?;
        let node = find_node(&self.editor.document().nodes, id)?;
        let transform = node.transform;
        Some((
            transform.x,
            transform.y,
            transform.width * transform.scale_x,
            transform.height * transform.scale_y,
        ))
    }

    fn artboard(&self) -> (f64, f64) {
        (
            self.editor.document().artboard.width,
            self.editor.document().artboard.height,
        )
    }

    /// A window pixel as an artboard point, if the canvas has been painted.
    fn scene_at(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.canvas_bounds.get()?;
        let local = point(position.x - bounds.origin.x, position.y - bounds.origin.y);
        Some(self.viewport.to_scene(local))
    }

    fn select_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((x, y)) = self.scene_at(position) else {
            return;
        };
        let hit = node_at(self.editor.document(), x, y);
        if hit != self.selection {
            self.selection = hit;
            cx.notify();
        }
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
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            ScrollDelta::Lines(delta) => (delta.x * LINE_HEIGHT, delta.y * LINE_HEIGHT),
        };
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

    fn canvas_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.clone();
        let images = self.images.clone();
        let bounds_cell = self.canvas_bounds.clone();
        let selection = self.selection_rect();
        let view = self.viewport;
        div()
            .id("canvas")
            .flex_1()
            .w_full()
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
                        this.select_at(event.position, cx);
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
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.pan_drag(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.panning = None),
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
                        let Some((x, y, width, height)) = selection else {
                            return;
                        };
                        window.paint_quad(PaintQuad {
                            bounds: Bounds {
                                origin: fit.point(x as f32, y as f32),
                                size: size(
                                    px(width as f32 * fit.scale),
                                    px(height as f32 * fit.scale),
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
            Some(Overlay::Import(prompt)) => self.guard(Resume::Import(prompt.value), cx),
            other => self.overlay = other,
        }
        cx.notify();
    }

    fn overlay_view(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content = match &self.overlay {
            None => return None,
            Some(Overlay::SaveAs(prompt)) => Self::prompt_panel(prompt, cx),
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
        rows.push(self.canvas_view(cx).into_any_element());
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

/// The theme an author starts from: a valid document with one body token, so
/// text added to it paints at a readable size.
fn blank() -> ThemeDocument {
    let mut type_tokens = BTreeMap::new();
    type_tokens.insert(
        "body".to_owned(),
        TypeToken {
            family: "Segoe UI".to_owned(),
            weight: None,
            face: None,
        },
    );
    ThemeDocument {
        schema_version: "1.0.0".to_owned(),
        id: "untitled".to_owned(),
        metadata: Metadata {
            name: Some("Untitled".to_owned()),
            ..Metadata::default()
        },
        artboard: Artboard {
            width: 450.0,
            height: 675.0,
            content_fit: ContentFit::Contain,
            shape: ArtboardShape::Rect,
        },
        background: Background::default(),
        palette: BTreeMap::new(),
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
                    text: "Text".to_owned(),
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

/// The topmost visible, unlocked node under an artboard point. Turned nodes are
/// hit by their box, which is what the outline shows.
fn node_at(document: &ThemeDocument, x: f64, y: f64) -> Option<String> {
    document
        .nodes
        .iter()
        .rev()
        .find(|node| {
            let transform = node.transform;
            let width = transform.width * transform.scale_x;
            let height = transform.height * transform.scale_y;
            node.visible
                && !node.locked
                && x >= transform.x
                && x <= transform.x + width
                && y >= transform.y
                && y <= transform.y + height
        })
        .map(|node| node.id.clone())
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
        INSET, STEP, Node, NodeKind, Path, StyleMap, ThemeDocument, Transform, blank, export_path,
        free_id, node_at, place, rectangle_node, slug, suggested_id,
    };

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
    fn a_click_selects_the_topmost_node_under_it() {
        let document = document_with(vec![
            rect("under", 0.0, 0.0, 100.0, 100.0),
            rect("over", 50.0, 50.0, 100.0, 100.0),
        ]);
        assert_eq!(node_at(&document, 60.0, 60.0).as_deref(), Some("over"));
        assert_eq!(node_at(&document, 10.0, 10.0).as_deref(), Some("under"));
        assert_eq!(node_at(&document, 500.0, 500.0), None);
    }

    #[test]
    fn a_locked_or_hidden_node_is_never_selected() {
        let mut locked = rect("locked", 0.0, 0.0, 100.0, 100.0);
        locked.locked = true;
        let mut hidden = rect("hidden", 0.0, 0.0, 100.0, 100.0);
        hidden.visible = false;
        let document = document_with(vec![locked, hidden]);
        assert_eq!(node_at(&document, 10.0, 10.0), None);
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
