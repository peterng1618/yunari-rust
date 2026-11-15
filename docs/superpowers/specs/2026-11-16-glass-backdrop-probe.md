# Glass backdrop capture — probe result

**Date:** 2026-11-16
**Spec:** `2026-11-16-yunari-rust-port-design.md` (Risk 1, phase 1 exit)
**Question:** can the GPUI backend capture and blur the backdrop under a glass
region, so glass is a paint-time effect rather than a pre-composited image?

## What the dependencies offer

Checked against the pinned tree: `gpui-pre` 0.3.12, `gpui-pre-platform` 0.3.12,
`gpui-pre-windows` 0.3.12.

- No backdrop/filter primitive exists. `gpui-pre` has no layer effect, no
  `backdrop-filter`, and no mid-frame scene readback. The only hit for
  "backdrop" is the Mica/Mica-Alt window material in
  `gpui-pre/src/platform.rs` — a window-level background, not a per-region
  effect.
- Headless window capture is macOS and Linux only. `current_headless_renderer()`
  in `gpui-pre-platform/src/gpui_platform.rs` returns `Ok(None)` for every other
  target (`#[cfg(not(any(target_os = "macos", target_os = "linux")))]`), and
  `PlatformHeadlessRenderer` is implemented only for Metal
  (`gpui-pre-apple`) and wgpu (`gpui-pre-wgpu`). The Windows DirectX renderer
  has an internal `render_to_image`, but no trait implementation to reach it.
- Consequence: on Windows `TestPlatform`'s window bails with
  `render_to_image not available: no HeadlessRenderer configured`, so GPUI-side
  pixel goldens are not available either.

`Window::rendered_scene()` exists under `bench-support` and exposes painted
quads with bounds and content masks. That is scene inspection, not pixels, and
it is not enabled by `gpui-kit`'s default features.

## Decision

**Adopt the spec's fallback.** Glass is resolved before paint, not during it:

1. `yunari-render` emits a `Backdrop` op carrying the glass region, corner
   radius, blur radius, tint/material and `sampling` mode. It is ordinary
   DisplayList geometry, so every backend sees the same thing.
2. The raster backend resolves it: paint the ops beneath the glass into a
   scratch pixmap, Gaussian-blur the region, clip to the glass shape, composite
   the material over it. It is the reference for goldens.
3. The GPUI backend resolves it the same way — the backdrop comes from the
   raster backend's output for that region and is painted as an image. Desktop
   glass therefore costs one offscreen raster pass per changed region, not a
   per-frame readback.
4. `sampling: static` caches that image; `sampling: dynamic` regenerates it when
   the scene beneath it changes. A backdrop that nothing paints beneath (no
   background, no media) resolves to the artboard background colour, which is
   what the player shows for a transparent region.

No GPU blur is attempted. If measurements later show the offscreen pass
dominating a frame, the upgrade is a blur shader inside the GPUI renderer — a
localised change behind the same `Backdrop` op.

## Consequences

- Goldens for glass live on `render-raster`, which is the path that already owns
  pixel proof. There is no GPUI screenshot test on Windows, so window-level
  checks assert on scene quads where feasible and otherwise on the DisplayList.
- The phone player blurs with canvas `filter: blur()`, which is the same
  geometry resolved by the platform's own rasteriser.
- Risk 1 is closed: no part of the glass feature depends on a GPUI API that does
  not exist on Windows.
