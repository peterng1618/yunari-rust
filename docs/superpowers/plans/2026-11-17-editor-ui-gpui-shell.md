# Editor UI P1 Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the editor's wrapping command-row shell with the source's one-row header, a fixed three-region workspace and a kit status bar, themed by `ActiveTheme`.

**Architecture:** `crates/editor/src/ui/shell.rs` owns the shell vocabulary and the `EditorWindow` chrome methods. `crates/editor/src/ui.rs` keeps `EditorWindow` state and the existing pane bodies. Every visible control is a kit component; the existing pane internals stay behaviour-identical until P2.

**Tech Stack:** Rust, `gpui-kit` 0.9.1 (`gpui-component`), GPUI.

**Spec:** `docs/superpowers/specs/editor-ui-gpui.md`

## Global Constraints

- `gpui-kit = "0.9.1"`; add no dependency.
- New shell code uses `ActiveTheme` tokens (`cx.theme()`), not `rgb(0x...)` literals. Pane internals keep their literals until P2.
- The source has **no menu bar**. Do not add `AppMenuBar`; `shell-menu-bar.tsx` is the header toolbar.
- Header is one row: brand, mode switch, editor toolbar, pane toggles. The
  toolbar shows in Editor mode only, as in the source.
- `Command::ALL` stays the single command registry. The toolbar is a subset; the command palette lands P3.
- No `DockArea` and no `h_resizable`. Use `h_flex` with the source's fixed
  46px rail + 246px / 1fr / 276px columns.
- Window opens at 1440x900, minimum 1024x700; acceptance capture is 1920x1080.
- Keep the open document across mode changes. The body is hidden, not unmounted.
- Keep all existing editor behaviour and tests passing.

## Review Focus

- **Custom title-bar interaction.** `TitleBar` owns a drag region; the brand, mode toggles and toolbar buttons must still receive clicks. Check one mode toggle and one toolbar button per capture pass.
- **Fixed pane sizing.** The old left and right panes hard-code `w(px(220.0))` and `w(px(260.0))` and the canvas uses `flex_1()`. The shell owns 246px / 1fr / 276px columns; the pane bodies fill their column and the stage keeps `min_w(px(0.0))` so it shrinks.
- **Keyboard focus.** The root `focus` handle currently owns every shortcut. The header's buttons and toggles must not leave the root unfocused after a click; type an arrow key after clicking a mode toggle to confirm nudging still works.
- **Mode persistence.** Switching Themes/Editor/Settings must not clear `editor`, `selection`, `expanded` or `overlay`. The editor body is hidden with the state intact.
- **Long status text.** A save error plus document name plus zoom plus selection must not clip or push the right region off at 1024px.

---

### Task 1: Shell vocabulary and toolbar groups

**Files:**
- Create: `crates/editor/src/ui/shell.rs`
- Modify: `crates/editor/src/ui.rs` (module declaration only)
- Test: `crates/editor/src/ui/shell.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::ui::menus::Command`.
- Produces:
  - `pub enum ShellMode { Themes, Editor, Settings }`
  - `pub enum LeftPane { Layers, Add, Widgets, Assets, Tokens, Document }`
  - `impl ShellMode { pub const ALL: [Self; 3]; pub fn label(self) -> &'static str; pub fn index(self) -> usize; }`
  - `pub const TOOLBAR_GROUPS: [&'static [Command]; 3]`

- [ ] **Step 1: Add the module declaration**

In `crates/editor/src/ui.rs`, next to `pub mod menus;`, add:

```rust
pub mod shell;
```

- [ ] **Step 2: Write the failing test**

Create `crates/editor/src/ui/shell.rs` with only the test module:

```rust
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_modes_are_themes_editor_settings_in_order() {
        assert_eq!(
            ShellMode::ALL.map(ShellMode::label),
            ["Themes", "Editor", "Settings"]
        );
        assert_eq!(
            ShellMode::ALL.map(ShellMode::index),
            [0, 1, 2]
        );
    }

    #[test]
    fn toolbar_groups_match_the_source_header() {
        assert_eq!(
            TOOLBAR_GROUPS,
            [
                &[Command::New, Command::Open, Command::Import][..],
                &[
                    Command::Save,
                    Command::SaveAs,
                    Command::Export,
                    Command::ExportWidget,
                    Command::OpenDisplay,
                ][..],
                &[Command::AssetFinder][..],
            ]
        );
    }

    #[test]
    fn toolbar_groups_do_not_repeat_a_command() {
        let mut seen = std::collections::BTreeSet::new();
        for command in TOOLBAR_GROUPS.into_iter().flatten() {
            assert!(seen.insert(*command), "{command:?} appears twice");
        }
    }
}
```

`Command` is `Ord`? It derives only `Debug, Clone, Copy, PartialEq, Eq`. `BTreeSet` needs `Ord`. Change the test to use a `Vec`:

```rust
let mut seen = Vec::new();
for command in TOOLBAR_GROUPS.into_iter().flatten() {
    assert!(!seen.contains(command), "{command:?} appears twice");
    seen.push(*command);
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p yunari-editor ui::shell::tests`
Expected: FAIL — `ShellMode` and `TOOLBAR_GROUPS` are not defined.

- [ ] **Step 4: Implement the vocabulary**

At the top of `crates/editor/src/ui/shell.rs`, above the test module:

```rust
//! The editor's chrome: mode vocabulary, header toolbar groups and state.

use crate::ui::menus::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    Themes,
    Editor,
    Settings,
}

impl ShellMode {
    pub const ALL: [Self; 3] = [Self::Themes, Self::Editor, Self::Settings];

    pub fn label(self) -> &'static str {
        match self {
            Self::Themes => "Themes",
            Self::Editor => "Editor",
            Self::Settings => "Settings",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Themes => 0,
            Self::Editor => 1,
            Self::Settings => 2,
        }
    }
}

/// The source header's toolbar, in source order:
/// open actions, document actions, then the asset finder.
pub const TOOLBAR_GROUPS: [&[Command]; 3] = [
    &[Command::New, Command::Open, Command::Import],
    &[
        Command::Save,
        Command::SaveAs,
        Command::Export,
        Command::ExportWidget,
        Command::OpenDisplay,
    ],
    &[Command::AssetFinder],
];
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p yunari-editor ui::shell::tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/editor/src/ui.rs crates/editor/src/ui/shell.rs
git commit -m "feat(editor): add shell modes and source toolbar groups"
```

---

### Task 2: Add shell view state to `EditorWindow`

**Files:**
- Modify: `crates/editor/src/ui.rs:197-246` (`EditorWindow` fields)
- Modify: `crates/editor/src/ui.rs:248-310` (`EditorWindow::new`)
- Modify: `crates/editor/src/ui/shell.rs` (`impl EditorWindow`)

**Interfaces:**
- Consumes: `ShellMode`.
- Produces:
  - `EditorWindow.mode: ShellMode`
  - `EditorWindow.left_collapsed: bool`
  - `EditorWindow.right_collapsed: bool`
  - `fn set_mode(&mut self, mode: ShellMode, cx: &mut Context<Self>)`
  - `fn toggle_left_pane(&mut self, cx: &mut Context<Self>)`
  - `fn toggle_right_pane(&mut self, cx: &mut Context<Self>)`

- [ ] **Step 1: Add the fields**

In `pub struct EditorWindow`, after `host: Option<Shared>,`, add:

```rust
    /// Which top-level surface the header is showing. The editor body stays
    /// mounted while another mode shows, so the open document survives.
    mode: ShellMode,
    /// The left and right panes' collapsed state. View state, never document
    /// state.
    left_collapsed: bool,
    right_collapsed: bool,
```

In `EditorWindow::new`, after `host,`, add:

```rust
            mode: ShellMode::Editor,
            left_collapsed: false,
            right_collapsed: false,
```

Import `ShellMode` in `ui.rs` beside the other UI imports:

```rust
use self::shell::ShellMode;
```

- [ ] **Step 2: Add the state methods**

In `crates/editor/src/ui/shell.rs`, add:

```rust
use gpui_kit::*;
use crate::ui::EditorWindow;

impl EditorWindow {
    pub(super) fn set_mode(&mut self, mode: ShellMode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    pub(super) fn toggle_left_pane(&mut self, cx: &mut Context<Self>) {
        self.left_collapsed = !self.left_collapsed;
        cx.notify();
    }

    pub(super) fn toggle_right_pane(&mut self, cx: &mut Context<Self>) {
        self.right_collapsed = !self.right_collapsed;
        cx.notify();
    }
}
```

`EditorWindow`'s fields are private to `ui`, and `ui::shell` is a child module, so this access compiles.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p yunari-editor`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/editor/src/ui.rs crates/editor/src/ui/shell.rs
git commit -m "feat(editor): add shell mode and pane collapse state"
```

---

### Task 3: Header — `TitleBar`, mode switch and toolbar

**Files:**
- Modify: `crates/editor/src/ui/shell.rs`
- Modify: `crates/editor/src/ui.rs` (`Render` root later in Task 6)

**Interfaces:**
- Consumes: `ShellMode`, `TOOLBAR_GROUPS`, `EditorWindow::apply`.
- Produces: `fn header(&self, cx: &mut Context<Self>) -> impl IntoElement`

- [ ] **Step 1: Write the header method**

Add to the `impl EditorWindow` in `crates/editor/src/ui/shell.rs`:

```rust
    pub(super) fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            ActiveTheme, Button, ButtonVariants, IconName, Sizable, Toggle, ToggleGroup,
            Toolbar, TitleBar, h_flex,
        };

        let mode = self.mode;
        let left = self.left_collapsed;
        let right = self.right_collapsed;

        let modes = ShellMode::ALL
            .into_iter()
            .map(|candidate| {
                Toggle::new(format!("shell-mode-{}", candidate.index()))
                    .label(candidate.label())
                    .checked(candidate == mode)
                    .on_click(cx.listener(
                        move |this, _: &bool, _, cx| this.set_mode(candidate, cx),
                    )) // `&bool` is Toggle's click value.            })
            .collect::<Vec<_>>();

        let mut toolbar = Toolbar::new("theme-toolbar");
        for (group_index, group) in TOOLBAR_GROUPS.iter().enumerate() {
            if group_index > 0 {
                toolbar = toolbar.content(
                    div()
                        .w(px(1.0))
                        .h(px(16.0))
                        .bg(cx.theme().border),
                );
            }
            for command in group.iter().copied() {
                toolbar = toolbar.child(
                    Button::new(format!("toolbar-{:?}", command))
                        .ghost()
                        .icon(toolbar_icon(command))
                        .tooltip(command.label())
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.apply(command, window, cx)
                        })),
                );
            }
        }

        TitleBar::new().child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child("Yunari"),
                )
                .child(
                    ToggleGroup::new("shell-modes")
                        .small()
                        .single()
                        .segmented()
                        .children(modes),
                )
                .child(div().flex_1())
                .child(toolbar)
                .child(
                    Button::new("toggle-left-pane")
                        .ghost()
                        .small()
                        .icon(IconName::PanelLeft)
                        .tooltip(if left { "Show left pane" } else { "Hide left pane" })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_left_pane(cx)
                        })),
                )
                .child(
                    Button::new("toggle-right-pane")
                        .ghost()
                        .small()
                        .icon(IconName::PanelRight)
                        .tooltip(if right { "Show inspector" } else { "Hide inspector" })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_right_pane(cx)
                        })),
                ),
        )
    }
```

Add the icon mapping at module scope in `shell.rs`:

```rust
use gpui_kit::component::IconName;

fn toolbar_icon(command: Command) -> IconName {
    match command {
        Command::New => IconName::Plus,
        Command::Open => IconName::FolderOpen,
        Command::Import => IconName::File,
        Command::Save => IconName::Check,
        Command::SaveAs => IconName::Copy,
        Command::Export => IconName::ExternalLink,
        Command::ExportWidget => IconName::Frame,
        Command::OpenDisplay => IconName::Play,
        Command::AssetFinder => IconName::Search,
        _ => IconName::Ellipsis,
    }
}
```

`IconName::PanelLeft`, `PanelRight`, `FolderOpen`, `ExternalLink`, `Frame` exist in `gpui-kit-assets` (default icon list). If one name does not resolve, use `IconName::Square` for that command; do not add an asset.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p yunari-editor`
Expected: PASS.

If `cx.listener` cannot infer the `Toggle` click type, replace the per-toggle callback with a group callback:

```rust
.on_click(cx.listener(|this, states: &Vec<bool>, _, cx| {
    if let Some(index) = states.iter().position(|checked| *checked) {
        this.set_mode(ShellMode::ALL[index], cx);
    }
}))
```

and drop the per-`Toggle` `.on_click`.

- [ ] **Step 3: Commit**

```bash
git add crates/editor/src/ui/shell.rs
git commit -m "feat(editor): render source header with title bar, modes and toolbar"
```

---

### Task 4: Body — fixed three-region workspace

**Files:**
- Modify: `crates/editor/src/ui/shell.rs`
- Modify: `crates/editor/src/ui.rs:2815-2924` (`layers_view` root)
- Modify: `crates/editor/src/ui.rs:2925-3037` (`canvas_view` root)
- Modify: `crates/editor/src/ui.rs:2134-2415` (`inspector_view` root)

**Interfaces:**
- Consumes: `EditorWindow::layers_view`, `canvas_view`, `inspector_view`.
- Produces: `fn body(&self, cx: &mut Context<Self>) -> impl IntoElement`

- [ ] **Step 1: Remove the hard-coded pane widths**

In `layers_view`, replace:

```rust
        div()
            .w(px(220.0))
            .flex_shrink_0()
            .h_full()
```

with:

```rust
        div()
            .size_full()
```

In `inspector_view`, replace:

```rust
        div()
            .w(px(260.0))
            .flex_shrink_0()
            .h_full()
```

with:

```rust
        div()
            .size_full()
```

In `canvas_view`, replace:

```rust
        div()
            .id("canvas")
            .flex_1()
```

with:

```rust
        div()
            .id("canvas")
            .size_full()
```

- [ ] **Step 2: Write the body method**

Add to `impl EditorWindow` in `shell.rs`:

```rust
    pub(super) fn body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::{
            ActiveTheme,
            empty::{Empty, EmptyDescription, EmptyHeader, EmptyTitle},
            h_flex,
        };

        let mut editor = h_flex().size_full().min_h(px(0.0));
        editor = editor.child(
            div()
                .w(px(46.0))
                .flex_shrink_0()
                .h_full()
                .child(self.rail(cx)),
        );
        if !self.left_collapsed {
            let pane = match self.left_pane {
                LeftPane::Layers => self.layers_view(cx).into_any_element(),
                other => pane_placeholder(other, cx).into_any_element(),
            };
            editor = editor.child(div().w(px(246.0)).flex_shrink_0().h_full().child(pane));
        }
        editor = editor.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .child(self.canvas_view(cx)),
        );
        if !self.right_collapsed {
            editor = editor.child(
                div()
                    .w(px(276.0))
                    .flex_shrink_0()
                    .h_full()
                    .child(self.inspector_view(cx)),
            );
        }

        div()
            .flex_1()
            .w_full()
            .min_h(px(0.0))
            .relative()
            .overflow_hidden()
            .child(
                div()
                    .size_full()
                    .when(self.mode != ShellMode::Editor, |this| this.invisible())
                    .child(editor),
            )
            .when(self.mode != ShellMode::Editor, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().background)
                        .child(
                            Empty::new().header(
                                EmptyHeader::new()
                                    .title(EmptyTitle::new().child(self.mode.label()))
                                    .description(EmptyDescription::new().child(
                                        "This surface lands in P4; the open theme is kept.",
                                    )),
                            ),
                        ),
                )
            })
    }
```

The editor body keeps its layout while hidden, so switching back restores the camera; the placeholder paints over it.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p yunari-editor`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/editor/src/ui.rs crates/editor/src/ui/shell.rs
git commit -m "feat(editor): lay out the editor body in source columns"
```

---

### Task 5: Status bar

**Files:**
- Modify: `crates/editor/src/ui/shell.rs`
- Modify: `crates/editor/src/ui.rs:3038-3075` (remove `status_view` after replacing)

**Interfaces:**
- Consumes: `EditorWindow.editor`, `viewport`, `selection`, `status`.
- Produces: `fn status_bar(&self, cx: &mut App) -> impl IntoElement`

- [ ] **Step 1: Write the status bar method**

Add to `impl EditorWindow` in `shell.rs`:

```rust
    pub(super) fn status_bar(&self, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::{ActiveTheme, StatusBar};

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

        let mut left = h_flex()
            .gap_2()
            .child(format!("{name} · {state}"));
        if !self.selection.is_empty() {
            left = left.child(format!("{} selected", self.selection.len()));
        }

        let mut right = h_flex().gap_2().child(format!(
            "{:.0}%",
            self.viewport.scale * 100.0
        ));
        if let Some(status) = &self.status {
            right = right.child(
                div()
                    .text_color(if status.error {
                        cx.theme().danger
                    } else {
                        cx.theme().success
                    })
                    .child(status.text.clone()),
            );
        }

        StatusBar::new().left(left).right(right)
    }
```

`StatusBar` accepts `AnyElement`; `h_flex` is a `Div` and converts.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p yunari-editor`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/editor/src/ui/shell.rs
git commit -m "feat(editor): render the status line with the kit status bar"
```

---

### Task 6: Root render and window options

**Files:**
- Modify: `crates/editor/src/ui.rs:3237-3300` (`impl Render for EditorWindow`)
- Modify: `crates/editor/src/ui.rs:3802-3835` (`window_for`)

**Interfaces:**
- Consumes: `EditorWindow::header`, `body`, `status_bar`.
- Produces: the P1 shell as the window root.

- [ ] **Step 1: Replace the root rows**

In `impl Render for EditorWindow`, replace the block from `let mut rows: Vec<AnyElement> = Vec::new();` through the closing `div()` expression with:

```rust
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(Self::on_key_up))
            .child(self.header(cx))
            .child(self.body(cx))
            .child(self.status_bar(cx))
            .children(self.overlay_view(cx))
```

`cx.theme()` needs `use gpui_kit::component::ActiveTheme;` in `ui.rs`.

- [ ] **Step 2: Remove the old bar renderers**

Delete `fn toolbar` (around line 2788) and `fn status_view` (around line 3038). They have no remaining callers.

- [ ] **Step 3: Use the kit title-bar window options**

In `window_for`, replace the `open_window(WindowOptions { ... }` argument with:

```rust
    let mut options = TitleBar::window_options();
    options.titlebar = Some(TitlebarOptions {
        title: Some("Yunari — Editor".into()),
        ..TitleBar::title_bar_options()
    });
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        None,
        size(px(1440.0), px(900.0)),
        cx,
    )));
    options.window_min_size = Some(size(px(1024.0), px(700.0)));

    open_window(options, cx, move |window, cx| {
```

Keep the existing closure body unchanged. Import `gpui_kit::component::TitleBar` in `ui.rs` if the `use gpui_kit::*` glob does not expose it.

- [ ] **Step 4: Verify the build**

Run: `cargo build -p yunari-app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/editor/src/ui.rs
git commit -m "feat(editor): mount the P1 shell in the editor window"
```

---

### Task 7: Acceptance evidence

**Files:**
- Create: `docs/parity/ledger/editor.md`
- Evidence: `test-results/parity-shots/editor-port-1080p.png`
- Evidence: `test-results/parity-shots/editor-source-1080p.png`

- [ ] **Step 1: Run the full editor suite**

Run: `cargo test -p yunari-editor`
Expected: PASS, including the existing behaviour tests and the three shell tests.

- [ ] **Step 2: Capture the port at 1920x1080**

Run:

```powershell
cargo build -p yunari-app
powershell -File scripts/capture-window.ps1 -Exe target/debug/yunari-app.exe -Width 1920 -Height 1080 -Out test-results/parity-shots/editor-port-1080p.png -DelaySeconds 8
```

- [ ] **Step 3: Capture the source at 1920x1080**

Run the source host and capture the editor page at 1920x1080:

```powershell
cd D:\git-repos\yunari\src\web
node packages/host/bin/yunari.js --no-browser
```

Open the printed editor URL in a 1920x1080 viewport, then save the capture to `test-results/parity-shots/editor-source-1080p.png`. Do not use a scaled or cropped screenshot.

- [ ] **Step 4: Judge the pair**

Check the port capture against the source capture:

- header is one row; brand, three modes, toolbar and pane toggles are visible;
- no command row wraps or overflows at 1920x1080 or 1024x700;
- left pane, canvas and inspector all have width, with visible dividers;
- status bar shows document state left and zoom right;
- clicking `Settings` shows the P4 placeholder without losing the document; clicking `Editor` restores it;
- clicking a mode toggle leaves arrow-key nudging working.

Record any failure as a task, not as a note. Do not mark P1 accepted with a clipped control.

- [ ] **Step 5: Write the ledger entry**

Create `docs/parity/ledger/editor.md`:

```markdown
# Editor ledger

## P1 shell — 2026-11-17

- Spec: `docs/superpowers/specs/editor-ui-gpui.md`
- Port capture: `test-results/parity-shots/editor-port-1080p.png`
- Source capture: `test-results/parity-shots/editor-source-1080p.png`
- Result: accepted / rejected (replace with the judged result)
- Evidence: header one row; no clipped control row at 1920x1080 or 1024x700;
  three fixed source columns; status bar; mode switch keeps the document.
- Deferred: Themes and Settings surfaces (P4); command palette (P3); pane
  internals still use P2-era controls.
```

- [ ] **Step 6: Commit**

```bash
git add docs/parity/ledger/editor.md
git commit -m "docs: record P1 shell acceptance evidence"
```

---

## Self-Review

- **Spec coverage:** P1 shell (header, modes, toolbar, fixed regions, status bar, theme, window options) is covered by Tasks 1–6. P1 screenshot gate is Task 7. P2–P5 are out of scope.
- **Step scan:** Every code step names the exact file, signature and values. The only fallback is the documented `ToggleGroup` callback shape and icon-name fallback, both bounded.
- **Type consistency:** `ShellMode`, `TOOLBAR_GROUPS`, `header`, `body`, `status_bar`, `set_mode`, `toggle_left_pane`, `toggle_right_pane` are named once and reused with the same signatures.
- **Review Focus tests:** Mode vocabulary and toolbar order are unit-tested. Title-bar clicks, pane sizing, focus retention, mode persistence and long status text are screenshot/interaction checks in Task 7 because GPUI UI-test support is not a current dev dependency.
- **Proportion:** The plan implements one shell layer and does not redefine editor behaviour.
