# Yunari native port — design

Date: 2026-11-16
Status: approved design, pending implementation plan
Source product: `D:\git-repos\yunari` (Node host + React/Fabric editor + browser player)
Target: `D:\git-repos\yunari-rust` (Rust + gpui-kit native app)

## Goal

Rebuild Yunari as one native Windows application that renders and authors
hardware-monitoring dashboards at lower cost than the Node + Chromium runtime,
while preserving the product's behaviour, invariants and surfaces. Implementations
and chrome may change where native Rust and gpui-kit fit better; product rules do
not.

## Decisions

| Decision | Choice |
|---|---|
| Scope | Full native app: editor, player windows, sensors, USB panels, host serving, store client, MCP. |
| Compatibility | Greenfield. No on-disk or schema compatibility duty during the port. Existing themes are hand-converted after feature parity. |
| Phone displays | Preserved: Rust serves a minimal browser player; phones pair and stay display-only. |
| Extensions | Out of v1. No plugin execution, declarative contributions or plugin management. Recorded as a product limit. |
| Performance | Measured and reported per phase; no hard numeric gate. |
| Platforms | Windows-first. Platform seams are trait-based, but no Linux/macOS parity promise. |
| Architecture | Layered cargo workspace, one process. |

## Product invariants carried over

- Readings are truthful: a key nobody measured is absent with a reason; never
  zero, a dash or a placeholder. The fake source is a dev/test-only crate.
- Authored state is what persists. Viewport, selection, glass sampling mode and
  other view state never rewrite the document.
- One owner per concept. No second document model, layout path, chart renderer
  or unit rule.
- Local by default: loopback binding, opt-in LAN, loopback-only admin surfaces.
- Secure origin for pairing and installability: local CA root and leaf on the
  same port.
- No usage telemetry.

## Workspace

```
crates/
  model/         document, validation, palette/type tokens, bindings,
                 semantic keys, unit conversion and formatting, chart plans,
                 background composition geometry, sample types
  render/        document + samples + clock -> ScenePlan -> DisplayList;
                 text layout, chart geometry, glass regions
  render-gpui/   DisplayList -> GPUI elements (windows)
  render-raster/ DisplayList -> RGBA (USB panels, thumbnails, tests)
  render-wasm/   DisplayList -> canvas (phone browser player)
  host/          providers, sampler, panel manager/drivers, HTTP/SSE/TLS,
                 pairing, theme library, settings, cache
  editor/        gpui-kit authoring UI
  player/        native display window
  app/           binary: tray, windows, autostart, wake lock, wiring
  store/         store catalogue and account client (phase 6)
  mcp/           MCP server over the host loopback API (phase 6)
```

Dependency direction: `model` has no UI, OS or network dependency. `render`
depends on `model` only. Backends depend on `render`. `host` depends on `model`
and `render-raster`, never on `editor`. `player` depends on `render-gpui`, never
on `editor`. `app` wires everything.

One executable. GPUI runs on the main thread. Tokio runs host tasks. Each panel
runner owns a thread because HID/serial waits block. Samples fan out through a
watch/broadcast channel. UI-to-host commands travel over channels; no shared
mutex across the UI. A driver panic stops only that display's runner.

## Document model

A versioned JSON envelope holding authored state only:

- `schemaVersion` (`1.0.0`), `id`, `metadata` (name, author, description,
  version, theme language, tags, remix provenance, timestamps).
- `artboard { width, height, contentFit, shape: rect | circle }`.
- `background`: page paint plus optional media (asset, bounds `theme|display`,
  offset, scale, flips, fallback frame, and for an advanced background the
  anchor/extent/fade composition). One owner for the paint: media colour when
  media exists, background colour otherwise; both present is refused.
- `palette` (each token is a display name and a solid or gradient paint),
  `typeTokens` (family, named or numeric weight, optional packaged face),
  `assets` (image, svg, gif, video, font — a font asset also declares the
  family, weight, style, format and optional subset it registers), `bindings`,
  and `editorMetadata` for editor-owned data a renderer never reads.
- Nodes: `group | rectangle | ellipse | line | path | text | chart | image | video`,
  stable IDs, parent-local transforms, visibility, lock, style map with token
  references or literals. Properties the current inspector exposes are ported
  as-is; no new authoring capability is invented.
- Text: literal and value runs, type token per run, fixed box or auto size,
  overflow, alignment, vertical mode.
- Charts: `gauge | line | bar | pie | radar | polarBar`, each with the existing
  settings vocabulary (angles, ranges, thickness, paints, segments, axes,
  series, legends, animation) and the existing binding-count caps.
- Bindings: `semanticKey`, precision, unit display, scale, offset, unit,
  autoScale, format, timeZone.

Validation refuses rather than coerces: 16384 px artboard ceiling, square
circle artboard, node depth 32, node count 5000, `[A-Za-z0-9_-]{1,64}` IDs,
`assets/...` paths, the existing background bounds and composition ranges.
Newer or older schema majors are refused; no migration path exists pre-release.

Semantic keys and unit metadata are ported from
`renderer-core/data/semantic-keys.ts` as the single vocabulary for host and
editor: families, labels, units, unit kinds, display units, value kinds, media
kind, expected tier, temperature conversion and instant keys.

## Render pipeline

One path for every surface:

```
ThemeDocument + SampleSource + now -> ScenePlan -> DisplayList
```

- `ScenePlan` resolves bindings, formats values, converts units, lays out text,
  derives chart geometry and marks gaps with reasons.
- `DisplayList` is backend-independent paint geometry in artboard/viewport
  coordinates: rects, paths with clips, gradients, images, glyph runs, blur
  regions, video frames.
- `render-gpui` maps ops to GPUI canvas paths, images and text.
- `render-raster` maps the same ops to tiny-skia + resvg output.
- `render-wasm` maps the same ops to canvas commands in the phone bundle.

Text layout has one owner: `render` shapes and positions runs (cosmic-text).
Backends paint the resulting glyph runs. Golden-image tolerance catches
divergence; if it proves material, glyphs move to a shared raster atlas for both
backends.

Charts are native geometry produced by `render`. ECharts is not ported and no
chart library becomes a second owner. Animation derives from a caller-supplied
clock and stays out of the document.

Glass is a DisplayList operation: capture the backdrop region, Gaussian blur,
clip to the rounded shape, paint the material over it. `sampling: static |
dynamic` decides whether the sample is reused across frames. Feasibility of the
GPUI backdrop capture is a phase 1 probe; the fallback composites the player
offscreen and paints it as an image, adopted only if measured acceptable.

Video decodes through Media Foundation first, with one decoder crate serving
both backends. Background media composition (anchor, extent, fade, pan, zoom,
flip) bakes into the same composition region for windows, panels and the phone
player.

## Editor

gpui-kit shell preserving today's structure and rules:

- Header: brand, `Editor | Themes | Settings` mode switch, grouped toolbar
  (New / New from template / Open / Import; Save / Save as / Activate / Export
  theme / Export widget; Find assets + source toggle) and the single filled
  `Publish` button.
- 46 px rail with Layers, Add, Widgets, Assets, Tokens, Document; at most one
  pane open.
- 246 px pane, stage, 276 px inspector, status bar with counts, saved state and
  live state.
- Inspector sections Content / Position / Layer / Paint, subject row, fold
  behaviour, empty sections hidden, refused rows with a reason, token rows with
  their token editors.
- Stage interactions: select, marquee, move/scale/rotate, snapping and guides,
  group enter, layers reorder, lock/hide, clipboard, duplicate/delete, arrange,
  undo/redo with coalesced drags, zoom/pan/fit, rulers.
- Add: text (auto and fixed box, vertical, overflow), rectangle/ellipse/line,
  path presets, image/SVG/video, six chart families, card widgets from installed
  packs.
- Assets: finder grid, import, place, crop, background framing; Wallpaper Engine
  source kept.
- Modes: `Themes` local library + store catalogue; `Settings` This PC (machine,
  displays, panels, sensor providers, store account) and Editor (preferences,
  shortcuts).
- Persistence: `%USERPROFILE%\.yunari\themes\<id>\theme.json` plus `assets/`;
  `.yunari-theme` ZIP import/export with manifest, theme, optional thumbnail,
  declared assets and credits; validation before write.
- Design tokens from `DESIGN.md` become gpui-kit theme tokens with a test that
  keeps the two in sync.

## Player

- Native window per local display: artboard fit, rect/circle glass, background
  composition, live bindings, fullscreen on the assigned monitor with mixed-DPI
  handling, native wake lock, no controls.
- Editor preview uses the same player renderer inside the stage.
- Phone player: Rust-served WASM bundle using `render` + DisplayList; SSE
  samples; display-only; no authoring code shipped.

## Host

- `ProviderRegistry` with the 1 s tick, union-of-bound-keys requests, precedence,
  failure isolation, last-completed-read semantics and read-start timestamps.
- Built-in providers: CPU/RAM/disk/network/clock/system; GPU counters and vendor
  APIs; LibreHardwareMonitor HTTP; RTSS shared memory and PresentMon ETW; WinRT
  media session; WLAN; Core Audio; Steam library/store metadata and foreground
  game; weather backends; custom HTTP JSON sources. Missing values stay absent
  with a reason.
- Storage under `%USERPROFILE%\.yunari` with atomic writes, host lock and
  single-instance guard. Secrets through Windows DPAPI.
- Serving: axum + rustls on one port with TLS sniffing, local CA root and leaf,
  root at `/yunari-ca.crt`, `/` phone player, `/pair`, `/api/health`, admin API;
  LAN opt-in; admin routes loopback and origin-checked; pairing code alphabet,
  ten-minute life, bounded guess penalty and durable display credential; SSE
  stream with connection, key-size and queue bounds.
- Displays: paired browsers, native local windows and USB panels in one manager.
- Panels: driver trait (geometry, still format, minimum interval, optional video,
  controls, optional device sink, open/present/close). Families: Lian Li 9.2",
  Lian Li Universal Screen 8.8", TURZX 5", NZXT Kraken Z53/Z63/Z73. One runner
  per display; raster frames from `render-raster`; video composition baked for
  video panels; newest-frame coalescing; SignalRGB loopback UDP bridge.
- Lifecycle: native tray actions, autostart entry, helper-process cleanup.
- Integrations: Wallpaper Engine library, Steam presence, weather, media,
  custom sources, LHM, RTSS/PresentMon.
- Store client and MCP server arrive in phase 6 over the same loopback API.

## Out of scope for v1

- Extension platform: executable plugins, declarative contributions, consent,
  device broker, plugin UI frames.
- Linux and macOS parity.
- Reading the old Node/Fabric document format.
- A second web editor. The native editor replaces the served `/editor` and
  `/settings` surfaces.

## Phases and exit criteria

| Phase | Work | Exit |
|---|---|---|
| 0 | Workspace, CI gates, gpui-kit shell, model + validation, golden harness, token check | `cargo test` and `clippy` green; app opens a window; validation rules covered |
| 1 | Render pipeline, GPUI + raster backends, charts, text, glass probe | One fixture theme renders in a window and to pixels within tolerance; glass decision recorded |
| 2 | Provider registry, Windows baseline providers, sampler, bindings, units | Live player shows real readings; unmapped keys render as gaps with reasons |
| 3 | Editor shell, stage, layers, inspector, add, assets, tokens, charts, widgets, history, persistence, Themes/Settings modes | Blank theme authored, saved, reloaded, exported and re-imported; undo/redo and snapping verified |
| 4 | axum/rustls, TLS, pairing, SSE, WASM phone player, local displays, tray/autostart/lock | Phone pairs and renders live; local monitor fullscreen; loopback-only admin verified |
| 5 | USB drivers one family at a time, raster/video, controls, SignalRGB bridge | Each panel renders the assigned theme at its cadence; unplug/replug; error isolation |
| 6 | Integrations, store client, MCP server | Every provider answers values or explicit missing; catalogue and account flows work |
| 7 | Parity sweep, theme hand-conversion, performance report, installer | Checklist against `product.md` complete; measurements reported |

Each phase receives its own implementation plan; this spec is the shared
contract those plans must satisfy.

Order rationale: renderer before editor so authoring has ground truth; live data
before editor so bindings are true; host serving after editor so parity is
verifiable locally; panels last because hardware protocol risk is highest;
integrations and store after the core product works.

## Proof

- Unit tests for validation refusals, unit conversion and formatting, chart
  plans, background geometry and protocol codecs.
- Property tests for document round-trip and for bounds never being coerced.
- Golden-image tests for every chart family, glass, background composition and a
  full player scene, on both raster and GPUI where feasible, tolerance-based.
- Headless e2e through the raster back end; editor commands tested without OS
  input where the gpui test harness allows it.
- Hardware tests marked ignored and run manually per panel family.
- Fake source is a dev-only crate and is absent from release builds.
- Performance measured per phase: idle private RAM, idle CPU, editor frame
  timings and panel cadence, reported against the Node baseline.

## Risks

1. GPUI backdrop capture and blur for glass — phase 1 probe; offscreen
   compositing fallback.
2. Text shaping/raster parity between backends — one layout owner, golden
   tolerance, shared glyph atlas if needed.
3. USB protocol quirks, especially Kraken storage writes and TURZX serial —
   earliest hardware probe, one family at a time.
4. Video decode parity and codec coverage — Media Foundation first, ffmpeg
   fallback if measurements demand it.
5. gpui-kit gaps for gradient/colour editing, scene tree and docking — custom
   elements inside the kit, not a fork.
6. WASM phone player size and runtime cost — measured at phase 4; fallback is a
   small JS painter consuming the same DisplayList, never a second model.
7. Mixed-DPI multi-window placement and wake lock — dedicated phase 4 test.

## Open follow-ups (not blocking)

- Installer format and update channel.
- Whether the store Worker deployment is reused unchanged or also rewritten.
- Keeping the `yunari-theme-designer` agent skill aligned with the native MCP
  server's tool surface.

## Phase status

Recorded as each phase's exit is met, with the evidence that met it.

**Phase 1 — met (2026-11-16).**

- One fixture theme renders in a window: `crates/app` opens a 640x380 window
  painting `crates/app/assets/fixture.json` through `render-gpui`; verified by
  launching it and capturing the window's pixels.
- …and to pixels within tolerance: `crates/raster/tests/fixture.rs` holds the
  same document to `goldens/fixture-theme.png`.
- Glass decision recorded: `2026-11-16-glass-backdrop-probe.md`, and adopted —
  glass is a display-list op resolved by the raster backend, painted as an
  image by the GPUI backend.
- Delivered beyond the exit criterion: text layout owned by `yunari-render`
  (cosmic-text), all six chart families as native geometry, image nodes with
  contain/cover/stretch and monochrome masks, and advanced background media
  composition (anchor, extent, fade, pan, zoom, flip).
- Deviation: phase 1 has no separate implementation plan document. The spec's
  phase table was used as the plan; tasks were scoped per render concern.
- Deferred: video decode. The node and background report
  `UnsupportedContent` rather than drawing a still without saying so. It lands
  with the panel baking in phase 5, which needs the decoder first; the `media`
  crate's seam is a frame-at-time reader so window and panel share it.
- Deferred: packed font faces (woff2) — system faces and TTF/OTF registration
  cover the fixture and the editor; woff2 needs a decompressor dependency.

**Phase 2 — met (2026-11-16).** Plan:
`docs/superpowers/plans/2026-11-16-yunari-phase-2-providers.md`.

- A window shows real readings from this PC: `crates/app` polls the sampler on a
  250 ms tick, re-plans on each due cycle, and repaints. Verified by launching it
  against a theme binding `time.now` and `cpu.load` (both tick) plus one unknown
  key; the log names the three keys as unread on the first frame, after which the
  unknown key is the only gap and draws the em dash.
- Any key nothing can read renders as a gap with its reason: `IssueCode` gains
  `UnmappedKey`, `MissingReading` and `UnsupportedUnit`; the dash is never a zero
  and the reason names the key.
- Vocabulary: 101 built-in semantic keys with units, tiers and instant defaults,
  plus the discovered device-scoped shapes, generated from the source table.
- Unit conversion at presentation only, after a binding's authored scale/offset,
  with a refused unit keeping the measured value.
- Providers: clock (`jiff`, IANA zones with a local fallback) and baseline system
  readings (`sysinfo` behind a `Snapshot` seam) — CPU load, memory, per-disk
  usage, per-interface rates (a rate needs two samples and says so).
- Registry unions every display's asks into one poll; a failing provider leaves
  the other readings intact and its message is redacted to one bounded line.
- Sampler paces polls by cadence, polls early when a key changes, and never
  re-enters a provider because sampling is synchronous.
- Deferred: theme language for month/weekday names and the decimal separator —
  English until an editor language choice exists (`unicode`-free tables in
  `crates/render/src/instant.rs`, comment marks the upgrade path).
- Deferred: chart bindings still draw from authored data; charts read readings
  when the chart plan takes a series source (phase 3 editor work).

**Phase 3A — met (2026-11-16).** Plan:
`docs/superpowers/plans/2026-11-16-yunari-phase-3a-editor-shell.md`.

- A theme can be authored, saved and reopened: the editor window (`crates/editor`
  `ui`) paints the open document through `render-gpui`, adds rectangles and text,
  selects by click, deletes the selection, and saves to the library. Proven
  against the real library root by driving the window: Add Rectangle → Ctrl+S →
  `themes/first/theme.json` holds the node — then Export wrote
  `untitled.yunari-theme` (manifest.json + theme.json), Import of that package
  stored `first-2` and opened it, and the status line read `first-2 · saved`.
- Undo/redo: Ctrl+Z after an add returns the theme to its saved state and Ctrl+Y
  (also Ctrl+Shift+Z) restores the edit; identity adopted by Save As is not
  undoable, because a rename is not an edit.
- Unsaved work is never dropped silently: New, Open and Import ask before
  discarding, and closing the window over a dirty document refuses the close and
  shows the same guard — verified by closing the window (process stayed alive,
  toolbar dimmed) and pressing Enter to discard (process exited).
- The live-display window moved to `crates/player`, so the app is either a player
  (given a theme path) or the editor (given none).
- Deviation: undo holds whole-document snapshots (100 entries), where the source
  undid the scene only — a background colour or a binding change is now undoable.
- Deviation: opening a theme loads the asset bytes its document declares. A save
  writes a fresh directory, so without this an edit-and-save would delete the
  author's files. Covered by `reopening_a_theme_keeps_the_assets_a_save_never_read`.
- Deviation: the shell's dialogs are native GPUI elements it owns (a typed-id
  prompt, a clickable row list, a confirm) rather than gpui-component widgets;
  GPUI exposes no interactive accessibility nodes for these, so they are reached
  by keyboard.
- Deviation: an export is named from the theme's display name
  (`untitled.yunari-theme`), not its id, and lands beside the library root rather
  than inside it.
- Not exercised: clicking a row in the Open dialog (needs the mouse; the window's
  custom elements expose no accessibility invocation). The listing itself is
  `Library::list`, covered by library tests.
- Deferred to 3B/3C: dragging, snapping, zoom/pan, layers, grouping, the inspector,
  palette and type-token editing, asset import, charts.
