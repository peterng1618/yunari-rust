# NSH UI migration

**Status:** proposed 2026-11-17. Requires a passing spike before any gpui UI
code is retired. Supersedes the UI framework choice in
`2026-11-17-editor-ui-rebuild.md`; its reference map, acceptance gate and
phases still apply.

## Why

The roadmap includes a native player app for Android and iOS. GPUI is
desktop-only: a mobile player on the current stack means a second UI
framework (SwiftUI/Compose), a second design system, a second test harness,
and eventually an editor rewrite to converge.

Native Shell (NSH) runs the same Rust UI on Windows, macOS, Linux, iOS,
Android and the web. One framework, one component library, one headless test
driver, one packaging path. That roadmap fit outweighs the desktop-only
convenience of `gpui-component`.

The editor UI failure was not caused by GPUI. It was never built to the
source reference. NSH does not fix that by itself: `nsh/base` is unstyled, so
the reference map and the screenshot gate remain the work.

## Scope

Keep, unchanged:

- `crates/model`, `crates/render`, `crates/raster`, `crates/player`,
  `crates/library`, `crates/store`, `crates/packs`, `crates/theme`,
  `crates/locale`, `crates/panels`, `crates/golden`;
- editor behaviour modules: `document`, `selection`, `transform`, `arrange`,
  `grouping`, `history`, `layers`, `inspector`, `palette`, `tokens`,
  `bindings`, `persistence`, `clipboard`, `snap`, `assets`, `fixtures`;
- `crates/host`, `crates/providers`, `crates/nvml`, `crates/rtss` on desktop,
  with Windows-only code behind `cfg(windows)` so the portable core compiles
  for mobile.

Replace:

- `crates/app` shell and `crates/editor/src/ui*` with NSH Rust components;
- `crates/render-gpui` with `crates/render-nsh` (raster image first, Canvas
  operations only where live drawing earns it);
- the build/CI path: `nsh.json`, `nsh` CLI, `nsh package`, headless
  `nsh_testing` tests.

Do not migrate the desktop player surface in this pass.

## Spike gate

A timeboxed lane proves the stack on this machine before the switch:

1. Windows NSH Rust app starts from the workspace and uses `yunari-model` /
   `yunari-render` types.
2. Three-pane editor shell (`ResizablePanelGroup` or `DockArea`), artboard
   rendered from a `DisplayList` via the raster image path, one inspector
   section with kit controls.
3. Application menu, context menu and command palette from one command list.
4. 1920x1080 capture beside the source reference; no clipped controls.
5. Frame timing while dragging a pane and redrawing the artboard.
6. `nsh package --target windows` produces a runnable directory.
7. iOS and Android "hello" builds compile from the same crate layout, with
   `crates/model` + `crates/render` linked.

Pass: switch. Fail on 1, 2, 4 or 7: stay on GPUI and build the mobile player
as a separate NSH app later, sharing the portable crates.

## Phases

- **S0 — spike.** Above gate; record the evidence in
  `docs/parity/ledger/editor.md`.
- **S1 — shell.** Modes, editor frame, panes, top bar, status bar, theme.
- **S2 — panels.** Layers tree, inspector sections, palette, assets,
  typography.
- **S3 — commands and canvas.** Command surface, context menus, shortcuts,
  selection/transform overlays, zoom/fit.
- **S4 — dialogs and modes.** Settings, store, asset finder, pickers,
  library, MCP, save/error states.
- **S5 — parity resume and mobile groundwork.** Probes run on the new surface;
  extract the portable player crate boundary for the Android/iOS app.

## Mobile player boundary

NSH covers a normal player app: video, canvas, gestures, lists, mobile tab
bar, haptics, packaging for `.ipa`/`.apk`. It does not provide Android's
`WallpaperService` (live wallpaper) or iOS wallpaper/background integration;
those need a native host embedding NSH, or platform-specific code. Sensors
and battery/network data come through `host` native modules or a small
platform crate, not `crates/providers` as it stands.

## Risks

- **NSH maturity.** v0.5.1, first release 2026-10-14. Pin the version, vendor
  the skills, and keep the gpui implementation until the spike passes.
- **Unstyled base.** Every look and state is ours; budget design work the
  gpui-component path would have given for free.
- **Toolchain.** `nsh` CLI, runtime downloads and packaging enter CI; the
  crate name on crates.io is unrelated, so install from the verified release
  archive.
- **Performance.** NSH trails GPUI Fast under sustained dense redraws; measure
  the editor canvas before committing.
- **Platform gaps.** Live wallpaper and background playback are native work,
  not NSH elements.

## Decision

Approve the switch if the spike passes. The reference map, component mapping
and per-screen acceptance checklists carry over; only the widget layer and
renderer backend change.
