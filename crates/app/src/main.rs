#![forbid(unsafe_code)]

use gpui_kit::*;
use yunari_theme::SPACING_STEPS_PX;

struct AppShell;

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap(px(SPACING_STEPS_PX[2] as f32))
            .p(px(SPACING_STEPS_PX[5] as f32))
            .child("Yunari")
            .child("native shell — phase 0")
    }
}

fn main() {
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            init(cx);
            open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| AppShell))
                .expect("failed to open Yunari window");
        });
}
