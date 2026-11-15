#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::*;
use yunari_render::{DisplayList, Fonts};
use yunari_render_raster::Images;

/// The theme a window shows until the library exists: the same fixture the
/// golden tests hold, so what a window paints and what a golden compares are
/// the same document.
const FIXTURE: &str = include_str!("../assets/fixture.json");

struct Stage {
    list: DisplayList,
    /// Pictures are read from beside the theme that names them.
    images: Rc<RefCell<Images>>,
}

impl Stage {
    /// The theme named on the command line, or the fixture.
    fn new() -> Self {
        let file = std::env::args().nth(1);
        let json = file
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| FIXTURE.to_owned());
        let images = match file.as_ref().and_then(|path| {
            std::path::Path::new(path)
                .parent()
                .map(std::path::Path::to_path_buf)
        }) {
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
        let (list, issues) = yunari_render::build(&document, &mut fonts);
        // A frame's diagnostics are the author's to see; nothing is invented to
        // hide one.
        for issue in &issues {
            eprintln!("{:?} {:?}: {}", issue.code, issue.node_id, issue.detail);
        }
        Self {
            list,
            images: Rc::new(RefCell::new(images)),
        }
    }
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
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

fn main() {
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            init(cx);
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
                |_, cx| cx.new(|_| Stage::new()),
            )
            .expect("failed to open the Yunari window");
        });
}
