#![forbid(unsafe_code)]

use std::path::PathBuf;

use gpui_kit::*;

/// `yunari-app [theme.json]` plays a theme; with no theme it opens the editor.
fn main() {
    let path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            match path.as_deref() {
                Some(path) => yunari_player::open(Some(path), cx),
                None => yunari_editor::open(cx),
            }
        });
}
