#![forbid(unsafe_code)]
//! The live display: a window painting one theme with this PC's readings.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use yunari_model::document::ThemeDocument;
use yunari_providers::clock::ClockProvider;
use yunari_providers::registry::Registry;
use yunari_providers::sampler::Sampler;
use yunari_providers::system::SystemProvider;
use yunari_providers::units::UnitPreferences;
use yunari_render::plan::PlanIssue;
use yunari_render::{DisplayList, Fonts, History, Readings};
use yunari_render_raster::Images;

/// How often the window looks at the sampler. The sampler itself decides when a
/// poll is actually due, so this only bounds how late a new reading can be.
const TICK: Duration = Duration::from_millis(250);
/// A playing clip repaints at its own cadence between sampler polls.
const FRAME: Duration = Duration::from_millis(33);

/// The theme a window shows until the library exists: the same fixture the
/// golden tests hold, so what a window paints and what a golden compares are
/// the same document.
const FIXTURE: &str = include_str!("../assets/fixture.json");

struct Stage {
    document: ThemeDocument,
    list: DisplayList,
    /// Pictures are read from beside the theme that names them.
    images: Rc<RefCell<Images>>,
    fonts: Fonts,
    sampler: Sampler,
    readings: Readings,
    /// What each key measured in recent polls, so a line chart draws a window.
    history: History,
    preferences: UnitPreferences,
    issues: Vec<PlanIssue>,
    polling: bool,
}

/// The providers a window reads when nobody hands it a richer registry.
fn base_registry() -> Arc<Registry> {
    let mut registry = Registry::new();
    registry.add(ClockProvider::new());
    registry.add(SystemProvider::new());
    Arc::new(registry)
}

impl Stage {
    /// The theme at `path`, or the fixture. A refused theme ends the process:
    /// a window with nothing to paint is not what a launched player asked for.
    fn new(path: Option<&Path>) -> Self {
        match Self::try_new(path, base_registry()) {
            Ok(stage) => stage,
            Err(error) => {
                eprintln!("theme refused: {error}");
                std::process::exit(2);
            }
        }
    }

    /// The theme at `path` read through `registry`, so a monitor shows the same
    /// readings the host serves rather than clock and system alone.
    fn try_new(path: Option<&Path>, registry: Arc<Registry>) -> Result<Self, String> {
        let json = path
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| FIXTURE.to_owned());
        let images = match path.and_then(Path::parent) {
            Some(base) => Images::at(base),
            None => Images::empty(),
        };
        let mut fonts = Fonts::new();
        let document =
            yunari_model::document::parse_document(&json).map_err(|error| error.to_string())?;
        // The faces the theme ships are read from beside it, as its pictures
        // are; a face that will not load is reported and the shaper falls back.
        if let Some(base) = path.and_then(Path::parent) {
            for refusal in fonts.load_faces(&document, |asset| std::fs::read(base.join(asset)).ok())
            {
                eprintln!("{refusal}");
            }
        }
        let mut sampler = Sampler::new(registry);
        // Ask once for exactly the keys this theme's bindings name.
        sampler.request("window", yunari_render::reading::wanted_keys(&document));
        let preferences = UnitPreferences::default();
        let (list, issues) =
            yunari_render::build_with(&document, &mut fonts, &Readings::default(), &preferences);
        report(&issues, &[]);
        Ok(Self {
            document,
            list,
            images: Rc::new(RefCell::new(images)),
            fonts,
            sampler,
            readings: Readings::default(),
            history: History::new(),
            preferences,
            issues,
            polling: false,
        })
    }

    /// Polls when the sampler says a poll is due, and rebuilds the frame.
    /// Returns whether anything changed.
    fn poll(&mut self, now_ms: u64) -> bool {
        let Some(cycle) = self.sampler.tick(now_ms) else {
            return false;
        };
        self.readings = Readings::new(cycle.readings.into_values());
        self.history.record(now_ms, &self.readings);
        let (list, issues) = yunari_render::build_with_history(
            &self.document,
            &mut self.fonts,
            &self.readings,
            &self.preferences,
            Some(&self.history),
            now_ms,
        );
        self.list = list;
        if issues != self.issues {
            report(&issues, &self.issues);
            self.issues = issues;
        }
        true
    }
}

/// A frame's diagnostics are the author's to see; nothing is invented to hide
/// one. Only diagnostics that changed are printed, so a gap does not shout once
/// per poll.
fn report(issues: &[PlanIssue], previous: &[PlanIssue]) {
    for issue in issues {
        if !previous.contains(issue) {
            eprintln!("{:?} {:?}: {}", issue.code, issue.node_id, issue.detail);
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

impl Render for Stage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One poll loop per window: a frame can render for other reasons too,
        // so the flag keeps a burst of notifies from starting one per frame.
        if !self.polling {
            self.polling = true;
            cx.spawn_in(window, async move |stage, cx| {
                loop {
                    let playing = stage
                        .update(cx, |stage, _| stage.list.has_video())
                        .unwrap_or(false);
                    cx.background_executor()
                        .timer(if playing { FRAME } else { TICK })
                        .await;
                    let changed = stage.update(cx, |stage, cx| {
                        if stage.poll(now_ms()) || stage.list.has_video() {
                            cx.notify();
                        }
                    });
                    if changed.is_err() {
                        // The window is gone; nothing left to poll for.
                        break;
                    }
                }
            })
            .detach();
        }
        let list = self.list.clone();
        let images = self.images.clone();
        div().size_full().bg(rgb(0x101014)).child(
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds, window, cx| {
                    yunari_render_gpui::paint(&list, bounds, window, cx, &mut images.borrow_mut());
                },
            )
            .size_full(),
        )
    }
}

/// Opens the live window for a theme file, or for the fixture when no path is
/// given.
pub fn open(path: Option<&Path>, cx: &mut App) {
    open_at(path, None, None, cx).unwrap_or_else(|| panic!("failed to open the Yunari window"));
}

/// Opens a live window fullscreen on `monitor` (a platform display uuid), or
/// on the primary display when no monitor is named.
pub fn open_fullscreen(path: Option<&Path>, monitor: Option<&str>, cx: &mut App) {
    open_at(path, Some(monitor), None, cx)
        .unwrap_or_else(|| panic!("failed to open the Yunari window"));
}

/// A monitor window a host drives: its theme and the readings it names come
/// from the host, and the handle is returned so a supervisor can close it when
/// the display changes.
pub fn open_supervised(
    registry: Arc<Registry>,
    path: Option<&Path>,
    monitor: Option<&str>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    open_at(path, Some(monitor), Some(registry), cx)
}

/// Places the window on a monitor, or centres a small one when none is named.
fn open_at(
    path: Option<&Path>,
    monitor: Option<Option<&str>>,
    registry: Option<Arc<Registry>>,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    // A monitored window that cannot read its theme says so and opens nothing:
    // a fixture on a wall would be a lie the author cannot see.
    if registry.is_some()
        && let Some(path) = path
        && let Err(error) = std::fs::read_to_string(path)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                yunari_model::document::parse_document(&json)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
    {
        eprintln!("{}: {error}", path.display());
        return None;
    }
    let path: Option<PathBuf> = path.map(Path::to_path_buf);
    let bounds = match monitor {
        None => WindowBounds::Windowed(Bounds::centered(None, size(px(640.0), px(380.0)), cx)),
        Some(uuid) => {
            let display = cx
                .displays()
                .into_iter()
                .find(|display| match (display.uuid(), uuid) {
                    (Ok(display_uuid), Some(wanted)) => display_uuid.to_string() == wanted,
                    (Ok(_), None) => true,
                    (Err(_), Some(_)) => false,
                    (Err(_), None) => false,
                });
            match display {
                Some(display) => WindowBounds::Fullscreen(display.bounds()),
                None => {
                    WindowBounds::Windowed(Bounds::centered(None, size(px(640.0), px(380.0)), cx))
                }
            }
        }
    };
    open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Yunari".into()),
                ..Default::default()
            }),
            window_bounds: Some(bounds),
            ..Default::default()
        },
        cx,
        move |_, cx| {
            cx.new(|_| match registry {
                Some(registry) => {
                    Stage::try_new(path.as_deref(), registry).unwrap_or_else(|_| Stage::new(None))
                }
                None => Stage::new(path.as_deref()),
            })
        },
    )
    .ok()
    .map(|(handle, _)| handle)
}
