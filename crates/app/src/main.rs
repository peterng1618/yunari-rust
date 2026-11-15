#![forbid(unsafe_code)]

use std::path::PathBuf;

use gpui_kit::*;

/// `yunari-app [theme.json]` plays a theme; the editor arrives with its shell.
fn main() {
    let path: Option<PathBuf> = std::env::args().nth(1).map(PathBuf::from);
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            yunari_player::open(path.as_deref(), cx);
        });
}
