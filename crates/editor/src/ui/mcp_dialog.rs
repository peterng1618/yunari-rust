//! The agent-tools page: where this host's MCP server answers, and the bridge
//! an agent launches so it never has to learn the port.

use gpui_kit::*;

use crate::ui::EditorWindow;

pub struct McpDialog {
    pub endpoint: String,
    pub bridge: String,
}

impl McpDialog {
    /// The endpoint names the port this host actually took; the bridge ships
    /// beside the app and finds the port itself.
    pub fn new(port: u16) -> Self {
        let bridge = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("yunari-mcp.exe")))
            .unwrap_or_else(|| std::path::PathBuf::from("yunari-mcp.exe"));
        Self {
            endpoint: format!("http://127.0.0.1:{port}/api/mcp"),
            bridge: bridge.display().to_string(),
        }
    }
}

/// A labelled pair of rows: the HTTP endpoint, then the stdio bridge.
pub fn view(dialog: &McpDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let row = |label: &'static str, value: &str| {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_sm()
            .bg(rgb(0x22222a))
            .child(div().text_color(rgb(0x8a8a94)).child(label))
            .child(div().text_color(rgb(0xe8e8ea)).child(value.to_owned()))
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_3()
                .pt_3()
                .text_color(rgb(0xe8e8ea))
                .child("Agent tools (MCP)"),
        )
        .child(row("JSON-RPC over HTTP", &dialog.endpoint))
        .child(row("stdio bridge", &dialog.bridge))
        .child(row(
            "register (copy the skill too)",
            &format!("\"{}\" install", dialog.bridge),
        ))
        .child(row(
            "check or remove",
            &format!("\"{}\" status | uninstall", dialog.bridge),
        ))
        .child(
            div()
                .px_3()
                .text_color(rgb(0x8a8a94))
                .child("Point an MCP client at the bridge; it finds this host's port."),
        )
        .child(
            div()
                .id("mcp-close")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Close  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}
