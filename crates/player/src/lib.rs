#![forbid(unsafe_code)]
//! The live display: a window painting one theme with this PC's readings.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use yunari_model::document::ThemeDocument;
use yunari_providers::clock::ClockProvider;
use yunari_providers::registry::Registry;
use yunari_providers::sampler::Sampler;
use yunari_providers::system::SystemProvider;
use yunari_providers::units::UnitPreferences;
use yunari_render::plan::PlanIssue;
use yunari_render::{DisplayList, Fonts, Readings};
use yunari_render_raster::Images;

/// How often the window looks at the sampler. The sampler itself decides when a
/// poll is actually due, so this only bounds how late a new reading can be.
const TICK: Duration = Duration::from_millis(250);

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
    preferences: UnitPreferences,
    issues: Vec<PlanIssue>,
    polling: bool,
}

impl Stage {
    /// The theme at `path`, or the fixture.
    fn new(path: Option<&Path>) -> Self {
        let json = path
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| FIXTURE.to_owned());
        let images = match path.and_then(Path::parent) {
            Some(base) => Images::at(base),
            None => Images::empty(),
        };
        let mut fonts = Fonts::new();
        let document = match yunari_model::document::parse_document(&json) {
            Ok(document) => document,
            Err(error) => {
                eprintln!("theme refused: {error}");
                std::process::exit(2);
            }
        };
        let mut registry = Registry::new();
        registry.add(ClockProvider::new());
        registry.add(SystemProvider::new());
        let mut sampler = Sampler::new(registry);
        // Ask once for exactly the keys this theme's value runs name.
        sampler.request("window", yunari_render::reading::wanted_keys(&document));
        let preferences = UnitPreferences::default();
        let (list, issues) = yunari_render::build_with(
            &document,
            &mut fonts,
            &Readings::default(),
            &preferences,
        );
        report(&issues, &[]);
        Self {
            document,
            list,
            images: Rc::new(RefCell::new(images)),
            fonts,
            sampler,
            readings: Readings::default(),
            preferences,
            issues,
            polling: false,
        }
    }

    /// Polls when the sampler says a poll is due, and rebuilds the frame.
    /// Returns whether anything changed.
    fn poll(&mut self, now_ms: u64) -> bool {
        let Some(cycle) = self.sampler.tick(now_ms) else {
            return false;
        };
        self.readings = Readings::new(cycle.readings.into_values());
        let (list, issues) = yunari_render::build_with(
            &self.document,
            &mut self.fonts,
            &self.readings,
            &self.preferences,
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
                    cx.background_executor().timer(TICK).await;
                    let changed = stage.update(cx, |stage, cx| {
                        if stage.poll(now_ms()) {
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
    let path: Option<PathBuf> = path.map(Path::to_path_buf);
    open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Yunari".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(640.0), px(380.0)),
                cx,
            ))),
            ..Default::default()
        },
        cx,
        move |_, cx| cx.new(|_| Stage::new(path.as_deref())),
    )
    .expect("failed to open the Yunari window");
}
