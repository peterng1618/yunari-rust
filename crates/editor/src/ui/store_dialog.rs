//! The store catalogue, the author's own items and the actions each row offers.
//!
//! Reading is one call to the host's catalogue proxy; installing and submitting
//! go through the host's own install and submit paths, so the editor never
//! downloads bytes and never holds the session token.

use gpui_kit::*;
use serde_json::Value;

use crate::ui::EditorWindow;

/// One catalogue row, as the listing names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub version: String,
    pub publisher: String,
    pub package_url: String,
    pub sha256: Option<String>,
    pub bytes: Option<u64>,
    pub status: Option<String>,
}

/// The rows a catalogue answer carries, in the store's own order. An entry
/// missing a fact this build needs to install it is still listed; the Install
/// press says what is missing.
pub fn rows_of(catalogue: &Value) -> Vec<Row> {
    entries(catalogue)
        .into_iter()
        .filter_map(|entry| {
            let text = |key: &str| {
                entry
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            };
            Some(Row {
                id: text("id")?,
                name: text("name")?,
                kind: text("kind").unwrap_or_else(|| "dashboard-theme".to_owned()),
                version: text("version").unwrap_or_else(|| "1.0.0".to_owned()),
                publisher: text("publisher").unwrap_or_default(),
                package_url: text("packageUrl").unwrap_or_default(),
                sha256: text("sha256"),
                bytes: entry.get("bytes").and_then(Value::as_u64),
                status: text("status"),
            })
        })
        .collect()
}

fn entries(value: &Value) -> Vec<&Value> {
    value
        .get("entries")
        .or_else(|| value.get("items"))
        .or_else(|| value.get("themes"))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().collect())
        .unwrap_or_default()
}

pub struct StoreDialog {
    pub rows: Vec<Row>,
    pub mine: Vec<Row>,
    pub email: Option<String>,
    pub message: Option<String>,
    pub loading: bool,
}

impl StoreDialog {
    /// A dialog that is still reading; `email` is the held account, when there
    /// is one.
    pub fn new(email: Option<String>) -> Self {
        Self {
            rows: Vec::new(),
            mine: Vec::new(),
            email,
            message: None,
            loading: true,
        }
    }

    /// The catalogue row with this id, so an action uses the facts the listing
    /// showed rather than what a row looked like when it was drawn.
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|row| row.id == id)
    }

    pub fn say(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
        self.loading = false;
    }
}

/// A small clickable word in a row's action strip.
fn action(
    id: (&'static str, usize),
    label: &str,
    on_click: impl Fn(&mut EditorWindow, &ClickEvent, &mut Window, &mut Context<EditorWindow>) + 'static,
    cx: &mut Context<EditorWindow>,
) -> AnyElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .text_color(rgb(0x8a8a94))
        .child(label.to_owned())
        .on_click(cx.listener(on_click))
        .into_any_element()
}

fn row_line(row: &Row) -> String {
    let publisher = if row.publisher.is_empty() {
        String::new()
    } else {
        format!("  ·  {}", row.publisher)
    };
    let status = row
        .status
        .as_ref()
        .map(|status| format!("  ·  {status}"))
        .unwrap_or_default();
    format!(
        "{}  ·  {} {}{publisher}{status}",
        row.name, row.kind, row.version
    )
}

pub fn view(dialog: &StoreDialog, cx: &mut Context<EditorWindow>) -> impl IntoElement {
    let rows: Vec<_> = dialog
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let id = row.id.clone();
            let installable = row.sha256.is_some() && row.bytes.is_some();
            let mut line = div().flex_1().child(row_line(row));
            if !installable {
                line = line.child(div().text_color(rgb(0x8a8a94)).child("  ·  no digest"));
            }
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .py_2()
                .rounded_sm()
                .bg(rgb(0x22222a))
                .child(line)
                .child(action(
                    ("store-install", index),
                    "Install",
                    move |this, _, window, cx| this.install_store_item(id.clone(), cx, window),
                    cx,
                ))
        })
        .collect();

    let mine: Vec<_> = dialog
        .mine
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let id = row.id.clone();
            let published = row.status.as_deref() == Some("published");
            let (label, publish) = if published {
                ("Unpublish", false)
            } else {
                ("Publish", true)
            };
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .px_3()
                .py_2()
                .rounded_sm()
                .bg(rgb(0x22222a))
                .child(div().flex_1().child(row_line(row)))
                .child(action(
                    ("store-transition", index),
                    label,
                    move |this, _, window, cx| {
                        this.store_transition(id.clone(), publish, cx, window)
                    },
                    cx,
                ))
        })
        .collect();

    let account = dialog
        .email
        .clone()
        .unwrap_or_else(|| "Not signed in".to_owned());
    let message = dialog.message.clone().unwrap_or_else(|| {
        if dialog.loading {
            "Reading the store…".to_owned()
        } else {
            String::new()
        }
    });

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .pt_3()
                .child(div().flex_1().text_color(rgb(0xe8e8ea)).child("Store"))
                .child(div().text_color(rgb(0x8a8a94)).child(account))
                .child(
                    div()
                        .id("store-refresh")
                        .px_2()
                        .cursor_pointer()
                        .text_color(rgb(0x8a8a94))
                        .child("Refresh")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.refresh_store(cx, window)
                        })),
                )
                .child(if dialog.email.is_some() {
                    div()
                        .id("store-sign-out")
                        .px_2()
                        .cursor_pointer()
                        .text_color(rgb(0x8a8a94))
                        .child("Sign out")
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.store_sign_out(cx)),
                        )
                } else {
                    div()
                        .id("store-sign-in")
                        .px_2()
                        .cursor_pointer()
                        .text_color(rgb(0x8a8a94))
                        .child("Sign in")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.store_sign_in(cx)))
                }),
        )
        .child(div().flex().flex_col().gap_1().px_2().children(rows))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .text_color(rgb(0x8a8a94))
                .child(div().flex_1().child("Your items"))
                .child(
                    div()
                        .id("store-submit")
                        .px_2()
                        .cursor_pointer()
                        .child("Submit this theme")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.submit_current_theme(cx, window)
                        })),
                ),
        )
        .child(div().flex().flex_col().gap_1().px_2().children(mine))
        .child(div().px_3().py_2().text_color(rgb(0x8a8a94)).child(message))
        .child(
            div()
                .id("store-close")
                .px_3()
                .py_2()
                .cursor_pointer()
                .text_color(rgb(0x8a8a94))
                .child("Close  ·  Esc")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx))),
        )
}

#[cfg(test)]
mod tests {
    // Named imports, not a glob: GPUI exports its own `test` attribute.
    use super::{StoreDialog, rows_of};

    #[test]
    fn a_catalogue_answer_becomes_rows_and_a_bad_entry_is_skipped() {
        let listing = serde_json::json!({
            "generatedAt": "2026-11-16T00:00:00Z",
            "entries": [
                {
                    "id": "0123abcd-4567-89ab-cdef-0123456789ab",
                    "kind": "dashboard-theme",
                    "name": "Aurora",
                    "version": "1.2.0",
                    "publisher": "huy",
                    "packageUrl": "https://store.example/api/upload/aurora",
                    "sha256": "a".repeat(64),
                    "bytes": 1234
                },
                {"name": "No id"}
            ]
        });
        let rows = rows_of(&listing);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Aurora");
        assert_eq!(rows[0].kind, "dashboard-theme");
        assert_eq!(rows[0].bytes, Some(1234));
        assert_eq!(rows[0].publisher, "huy");
        assert_eq!(rows_of(&serde_json::json!(null)), Vec::new());
    }

    #[test]
    fn the_mine_answer_is_read_by_the_same_reader() {
        let mine = serde_json::json!({
            "entries": [{
                "id": "0123abcd-4567-89ab-cdef-0123456789ab",
                "name": "Aurora",
                "kind": "dashboard-theme",
                "version": "1.3.0",
                "status": "pending"
            }]
        });
        let rows = rows_of(&mine);
        assert_eq!(rows[0].status.as_deref(), Some("pending"));
    }

    #[test]
    fn a_row_is_found_by_its_catalogue_id() {
        let mut dialog = StoreDialog::new(None);
        dialog.rows = rows_of(&serde_json::json!({
            "entries": [{"id": "abc", "name": "Aurora"}]
        }));
        assert_eq!(
            dialog.row("abc").map(|row| row.name.as_str()),
            Some("Aurora")
        );
        assert!(dialog.row("missing").is_none());
    }
}
