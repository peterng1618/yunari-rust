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

### Reading a theme stored by the source editor (2026-11-16)

The source editor stores `{schemaVersion, fabricVersion, globals, scene}` with
flat Fabric scene objects. `yunari_model::source` lifts one of those envelopes
into this model on read: palette, type tokens, assets, bindings and chart
settings carry across unchanged, each object becomes a node (`type` → `kind`,
`left/top/width/height` → `transform`, `yunariPaint` → `style`, `yunariGlass` →
`glass`, `objects` → `children`, Fabric path commands → one SVG path string), and
a kind this port does not draw is dropped rather than failing the theme.

Two consequences, both recorded here because they are product decisions:

- The document's semantic-key grammar accepts its segments in any case, since
  stored themes bind `weather.feelsLike` and `weather.windDirection.compass`.
  A provider's catalogue, not the document, decides which keys exist.
- A theme saved again is written in this model's shape; its original bytes stay
  beside it as `theme.fabric.json` so the other editor's copy is never lost.

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
- Modes: `Themes` local library plus a `Store…` dialog that lists the catalogue
  and this account's items; `Settings` This PC (machine, displays, panels,
  sensor providers, store account) and Editor (preferences, shortcuts).
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
- A byte-compatible Node/Fabric document schema. A source theme is lifted into
  this model on read (see "Reading a theme stored by the source editor") and
  written back in the native shape; the original bytes are kept beside it.
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

**Phase 3B — met (2026-11-16).** Plan:
`docs/superpowers/plans/2026-11-16-yunari-phase-3b-stage.md`.

- The artboard is a stage: `Viewport` carries the source's constants (0.02–64
  scale, 1.1 ladder, wheel zoom divided by 1000, 48 px overscroll, 16 px line
  height); wheel pans, Ctrl+wheel zooms at the cursor, Space or middle-drag pans,
  `+`/`-`/`0`/`Shift+1` change the camera, and the status line shows the zoom.
  Driven: fit 84 %, `+` `+` 101 %, `-` 92 %, `0` back to fit.
- Selection is a list of ids: click replaces, Shift-click toggles, a marquee takes
  fully contained visible nodes, `Ctrl+A` takes the roots, `Escape` clears. Hits
  invert the node's drawn transform, so a rotated node is taken where it is
  painted. Every refresh drops ids the document no longer has, and Delete removes
  the whole list in one edit. Driven: two rectangles, `Ctrl+A` shows "2 selected"
  with two outlines, `Delete`, then one `Ctrl+Z` restores both and neither is
  selected.
- Snapping draws lines from the artboard's edges and centre and from every
  visible node outside the drag; the nearest line inside 5 screen pixels wins,
  a boundary beats an edge and an edge a centre, Ctrl turns it off, and guides
  paint dashed in `#3D8BF4` across the artboard.
- Move, resize, rotate and nudge go through `Editor::begin_burst`/`end_burst`, so
  any one gesture is a single undo entry; the pure geometry lives in
  `transform.rs` (uniform corners, Shift frees, Alt resizes about the centre,
  whole-degree rotation, 1-unit minimum, one line of arrow-nudge bursts after a
  300 ms idle).
- The layers panel is a 220 px right column: rows read topmost-first with depth,
  a child of a hidden group reads hidden, groups fold with a caret, and a row
  click selects (Ctrl toggles, Shift extends). `Ctrl+]`/`Ctrl+[` raise and lower;
  hide, lock and rename are `Editor` edits. Driven: reordering a text behind a
  rectangle changed both the panel order and the paint, and `Ctrl+Z` restored it.
- Arrange aligns 2+ nodes on the first selected node and distributes equal gaps
  for 3+; any locked member refuses the whole op. Grouping rebases the selection
  into a union-box group and ungrouping bakes the offset back. The clipboard
  copies bindings and asset references, pastes at +10 with fresh node and binding
  ids and re-pointed runs, and reports the pictures the document cannot show.
  Driven: `Ctrl+G` made a selected group of two rectangles, `Ctrl+Shift+G` put
  both back, `Ctrl+D` added two copies at +10, `Ctrl+C`/`Ctrl+V` added two more
  and the status line read "Pasted 2", each undone by one `Ctrl+Z`.
- Deliberate simplifications, each with its upgrade path recorded in the code:
  handles paint axis-aligned and a turned node offers only the turn handle
  (resize would need the pointer in node space); snapping has no spacing/gap
  mode; the panel has no drag-reorder; nothing skews; marquee and canvas hit test
  top-level nodes, so a child is selected through the panel; a second click on
  the only selected row renames it because gpui's click event carries no click
  count.
- Not exercised: every pointer path — marquee, drag with guides, resize, rotate
  and the panel's row and toolbar clicks. The driver only delivers synthetic
  keyboard input while the window keeps focus, and pointer input needs the
  foreground, which would take over the user's screen. Their maths and the
  `Editor` edits behind them are covered by 70 editor tests; the handle chrome,
  selection outlines, guides and panel rows were confirmed painted in
  screenshots.
- Deferred to 3C: the inspector (per-kind fields), add/create beyond rectangle
  and text, palette and type-token editing, asset import, chart binding sources,
  presets.

**Phase 3C — met (2026-11-16).** The inspector is the second column of the
editor window and every field the Rust model declares for a node or a document
has a row unless listed below.

- With no selection the panel shows Theme (name, author, description, version,
  language, tags), Artboard (width, height, content fit, shape) and Background
  (mode, colour, media). Artboard edges are whole numbers 1–16384, refused
  rather than clamped. The blank theme now matches the source: 1920 x 1080, the
  eight seeded paint tokens including the reserved `none`, the three role type
  tokens, and a `palette.chartTrack` background.
- A selected node shows Transform, Appearance, then its kind's section: corner
  radius; image asset and fit; video asset, loop and muted; text runs (text,
  token, size, colour), align/overflow/vertical align/vertical and line height;
  chart family and the scalar, flag and paint settings each renderer reads; and
  a bindings section (key, unit, precision, unit display, format).
- `palette.rs`, `tokens.rs` and `bindings.rs` own their invariants: free ids
  minted from names, the reserved `none` token and the role tokens refused as
  targets of rename/recolour/removal, references scanned before a token or
  binding is removed, semantic keys validated with the model's own predicate,
  and every write recorded as one history entry.
- Background media takes a declared image/SVG/video asset, bounds, scale,
  offsets, both flips, a video fallback frame, and advanced anchor/size/fade
  with the model's ranges; the colour slot moves to the media when media is
  present, and background colours are palette references only, as in the source.
- The add menu now creates text, vertical text, rectangle, ellipse, the arrow
  path, the horizontal-line/arc/wedge path presets and all six chart families,
  at the source's default sizes and with palette-derived paints when the tokens
  exist. Open paths paint ink as a 2 px stroke; closed ones take the surface
  fill.
- Driven: the app launched on the blank theme with the correct artboard, `E`
  added a 360 x 360 ellipse whose inspector read X 40, Y 40, W 360, H 360 and
  `palette.panel`, `T` added "New text" which rendered once the camera zoomed to
  44 %, and the status line tracked "1 selected", "unsaved changes" and zoom.
  The first run exposed a layout bug — the canvas used `size_full()` inside an
  unbounded row, so the artboard sat below the window and the status line was
  clipped; the shell row is now bounded and the artboard is fitted (25 % of
  1920 x 1080). The inspector scrolls by keeping its own offset and clipping,
  because gpui-kit 0.9.1 exposes neither a scroll container nor
  `overflow_y_scroll`.
- Deliberate deviations: sections are Transform/Appearance/kind rather than the
  source's Content/Position/Layer/Paint; style paints accept a hex literal or a
  `palette.<id>` reference (only the background is reference-only); semantic
  keys are typed into a prompt instead of chosen from the ~140-entry vocabulary;
  chart paint arrays and most advanced settings are not offered.
- Not exercised: pointer input — clicking a field, a toolbar button or a layer
  row, and mouse-wheel scrolling of the inspector — because the driver refuses
  pointer input unless the window is foreground, which would take over the
  user's screen. All 108 editor tests and the workspace suite pass, clippy is
  clean with `-D warnings`, and every write path above has a unit test.
- Deferred: gradient token editing and delete-with-reassign; font faces and
  family/weight pickers; asset import (limits, image downscale, video trim, font
  metadata, SVG sanitising); the eight-card bundled widget pack (needs its
  packaged icons); shadow, border-width and frosted-glass shape fields; polygon
  and triangle kinds, which the Rust model does not carry yet.

## Phase 4 — host, pairing and the browser display — met (2026-11-16)

- `yunari-host` is a library the app starts on a worker thread: one port serves
  plain HTTP and TLS, chosen by the first byte (`0x16` takes the TLS handshake),
  through `hyper-util`'s auto server. The app keeps its editor when the port is
  taken (5227–5236 are tried), when the folder is held by another host (pid and
  port are named in the refusal), and when the TLS root cannot be written.
- A local certificate authority (`tls/ca.crt`, `tls/ca-key.bin`) is minted on
  first use and reused; a 397-day leaf with SANs for `localhost`, the machine
  name, `<name>.local` and every interface address is minted each start.
  `GET /yunari-ca.crt` serves the root before any gate with the source's
  content type, filename and `x-yunari-ca-sha256` fingerprint.
- Pairing: six characters from `ABCDEFGHJKMNPQRSTUVWXYZ23456789`, ten minutes,
  one live code, per-address penalty `min(250·2^(missed−1), 4000)` ms, the
  source's single refusal sentence, a 32-byte base64url token stored only as a
  SHA-256 hash, and the source's cookie (`Max-Age=34560000; HttpOnly;
  SameSite=Strict; Path=/`, `Secure` over TLS). The `/pair` page and the
  `POST /api/session` exchange complete the flow.
- The sample stream is SSE at `/ws` with the source's limits (64 keys, 128-byte
  keys, 64 streams, 8 per display, 120-frame queue) and its connected comment,
  event name and payload version. Each one-second tick samples the registry
  once for every display, builds that display's missing readings with the
  product's message, and publishes the batch together with the display's frame
  as the planner's own `DisplayList` JSON.
- The phone page is embedded (`pair.html`, `player.html`, `player.js`) and paints
  that list on a canvas: solid and linear paints, rects with radii, ellipses,
  paths, lines, text runs with family/weight/size/colour, pictures from
  `/api/themes/:id/assets/<path>`, wake lock and tap-to-fullscreen. It polls
  `/api/published` for the assignment and reconnects when the theme changes.
- Loopback-only surfaces are checked per request (the source's rule): displays,
  pairing codes and sessions, theme writes, autostart, hosting, and any `/api/*`
  request from a loopback peer with a foreign `Origin` are refused. A remote
  unpaired page is redirected to `/pair?reason=unpaired`; remote asset requests
  are refused with the source's sentence.
- Persistence matches the source's files: `host-identity.json`,
  `display-clients.json` (`{displays:[…]}`, kinds, token hashes, 5 s touch),
  `active-theme.json` (including the legacy `{id}` shape and the retired
  `this-pc` key), `hosting.json`, `host.lock` (pid + port), `tls/`. Autostart
  writes the shell's UTF-16LE startup script and reports the source's two
  sentences.
- Driven: the app's host answered `/api/health` on plain HTTP and, through the
  CA, on HTTPS on the same port; `/api/host` returned the minted identity;
  a theme copied into the library was listed and served; a code minted over
  loopback was claimed by the pairing page in a headless browser, which then
  landed on `/` and painted the theme live (screenshot); `Ctrl+Shift+F` in the
  editor wrote `live-theme.json` and opened the player fullscreen on this
  display (2560×1440).
- Deliberate deviations: the root is ECDSA P-256 rather than RSA-2048; the
  frame rides in the SSE batch (a field the source's payload lacks) so the page
  never re-implements the renderer; a full frame queue drops the newest frame
  instead of the oldest (either way the next frame is a whole snapshot); the CA
  key is a plain file, since Windows' credential protector has no Rust crate
  here yet.
- Not exercised: a real phone installing the root certificate (the browser
  refused the untrusted root, and installing one is the user's decision), LAN
  reachability, and the display peer API from another machine. 44 host tests and
  the workspace suite pass; clippy is clean with `-D warnings`.
- Deferred: the tray icon and its menu; local-display launching from the host
  process (the editor opens it today, because gpui windows must be created on
  the UI thread); monitor enumeration over `/api/displays/monitors`; applying a
  LAN toggle without a restart; the admin SPA (`/admin/`) and the store/MCP
  routes (phase 6); panels and video (phase 5).

## Phase 5 — USB panels — met (2026-11-16)

Plan: `docs/superpowers/plans/2026-11-16-yunari-phase-5-panels.md`.

- Four families in `crates/panels/src/family.rs`: Lian Li 9.2", Lian Li
  Universal Screen 8.8", TURZX 5" and NZXT Kraken Z53/Z63/Z73, each with its own
  geometry, still format, command list and measured-firmware support row (an
  empty list is written empty, never guessed).
- Drivers: Lian Li over HID, Kraken over HID for control plus libusb for its
  pixel stream, TURZX over its serial protocol. Each keeps the source's packet
  shape and its refusals (`produced the wrong number of bytes`, HELLO/status
  sentences).
- Frames come from the same `render-raster` path the goldens use: the display's
  assigned theme is resolved, planned with that display's keys, fitted into the
  family geometry with its rotation, and encoded as the device's still format.
  A byte-identical frame is not sent; a full frame queue drops rather than grows.
- One runner per display, newest-frame coalescing, three consecutive failures
  mark it lost and the manager retries; a command to a stopped display is the
  source's 409 sentence. Host routes are loopback-only and a phone gets 403.
- Panel video (Lian Li blocks, TURZX MP4 upload and cache, overlays, Kraken
  paletted mode B, the Chromium page rasteriser), SignalRGB and device telemetry
  stay deferred — recorded in the plan's header note and below.
- Not exercised: real hardware. Driver contracts are covered by protocol tests
  and the serial/sink seams; plugging each family in is a manual check.

## Phase 6 — providers and integrations — met (2026-11-16)

Plan: `docs/superpowers/plans/2026-11-16-yunari-phase-6-providers.md`.

- Every remaining key family has a provider behind the existing `Provider`
  trait: LibreHardwareMonitor (caption table, gap reasons), frame timing (RTSS
  shared memory, PresentMon ETW fallback), weather (Open-Meteo), media (the
  source's SMTC PowerShell helper, content-addressed artwork cache), custom HTTP
  sources (`sources.json`, RFC 6901 pointers) and the running game (foreground
  window plus the installed-game catalogue — Steam, Epic, GOG, itch and Xbox —
  with Steam's cached box art, and a Steam store search by executable name for
  a game no catalogue claims, whose portrait is cached after a JPEG/PNG magic
  check). A key nothing can
  read is `missing` with an owner-actionable sentence, never a zero.
- Media and game helpers share one PowerShell runner (`yunari_providers::winps`),
  so the line cap, the encoded command and the process kill exist once. Helper
  output is treated as data: capped lines, typed parsing, base64 decoding, image
  sniffing and allow-listed routes for artwork.
- Integrations on the loopback API: `GET /api/store/themes` proxies one
  configured origin and caches it for five minutes; `POST /api/mcp` answers
  JSON-RPC 2.0 (`initialize`, `tools/list`, `tools/call`) over the theme library,
  one theme document and the sensor catalogue. Both refuse a phone.
- Deferred with reasons in the plan: the store install path and account flows,
  the stdio MCP launcher and skill installer, and the SignalRGB keys sink (there
  is no `lighting.*` vocabulary to bind, and the source bridge writes panel LEDs,
  not readings).

## Phase 7 — assets and release notes — met (2026-11-16)

Picture import sniffs the bytes rather than trusting the extension, caps size by
kind, and refuses an SVG whose text names an active construct; the accepted
bytes are declared as an asset with their SHA-256 and seeded into the raster
cache so the picture draws before the theme is saved. `docs/install.md` records
what a release build needs.

The shipped release artifact is blocked, not faked: GPUI compiles HLSL with
`fxc.exe` only when `debug_assertions` is off, and this machine has no Windows
SDK. Measurements were taken with `--config 'profile.release.debug-assertions=true'`
and are reported above.

## Phase 8 — widget packs — met (2026-11-16)

`yunari-packs` reads the source package format through the same gates the source
reader applied: 128 entries, 95 MiB archive, 80 MiB entry, 180 MiB expanded, no
climbing or duplicate names, a SHA-256 per asset, and units from the manifest's
list or the widget's own provenance. Each unit lifts to a scene this editor
draws, with its assets' bytes.

The host owns the installed folder: `GET/POST /api/widget-packs` list and install
for this PC only, and a fresh settings folder receives the committed "Starter
cards" pack. The editor's picker (`Ctrl+Shift+W`) inserts a unit, merging only the
tokens and assets the document lacks, remapping asset ids, and keeping every id
fresh so two insertions stay distinct and undo removes the second.

Not ported: writing a package back out (row 8), the path-preset store and
`user:`/`pack:` refs (row 9).

## Monitor windows (2026-11-16)

The host records a local display per monitor (`display-clients.json`); the app
keeps one fullscreen player window per record, fullscreen on the monitor uuid
the record names, painting the theme its assignment resolves to. The window is
replaced when the assignment or the theme file's stamp changes, closed when the
record goes, and reads through the host's provider registry rather than its own
clock-and-system pair. Settings lists what GPUI sees and toggles a monitor's
record, so the loop is reachable without editing files.

The shared registry meant widening the seam: `Sampler::new` takes anything that
converts into `Arc<Registry>`, and the player's `Stage` gained a registry-taking
constructor with a fallible load, so a supervised window with an unreadable
theme opens nothing instead of exiting the app.

Not ported: the tray menu, and an HTTP endpoint that lists monitors (the
enumeration lives in the editor shell, where GPUI owns it).

## Parity sweep against `product.md` (2026-11-16)

Every capability `docs/product.md` names, with what this port does today. "Met"
means the behaviour exists and is covered by tests or a driven check; "partial"
names the missing part; "deferred" names the phase or reason; "dropped" is a
recorded product decision.

| # | `product.md` capability | Status | Where / why |
|---|---|---|---|
| 1 | Authoring: shapes, text, images/SVG, groups, layers, undo, snapping | Met | `crates/editor` (3A–3C); video nodes are carried but report `UnsupportedContent` until a decoder lands |
| 2 | Card widgets from installed widget packs | Met | `yunari-packs` reads the source `.yunari-widget-package` format; the host owns the installed folder and the editor inserts any unit (`Ctrl+Shift+W`) |
| 3 | Six chart families | Met | `yunari-render` plans gauge, line, bar, pie, radar and polar bar as native geometry |
| 4 | Rich text and typography presets | Partial | Runs, tokens, size/colour, alignment, overflow, vertical and line height are authored; the only presets are the three role tokens |
| 5 | Paints, type tokens, frosted glass | Met | Gradient tokens editable after 3C; glass is a display-list op resolved by the raster backend |
| 6 | Rectangular or circular artboard, disc cropping | Met | Model validation refuses a non-square circle artboard; raster and GPUI crop to the disc |
| 7 | New theme blank or from a bundled starter template | Met | A new document starts from the built-in starter: the blank theme's tokens, a title and a clock bound to `time.now` |
| 8 | Export a selection as `yunari-widget-package` | Met | `Ctrl+Shift+E` writes the selected top-level nodes as a package beside the library: one unit, assets with their digests, and the writer runs its own reader before handing the bytes on. The document inside is this build's native shape, so another editor reads it only once it reads that shape; a source-editor package still imports here |
| 9 | Host-owned widget packs and path-preset list, `user:`/`pack:` refs, one-time browser migration | Partial | Packs are host-owned and installed per PC; the path-preset store, `user:`/`pack:` refs and the browser migration are not ported |
| 10 | Starter theme and default widget pack as committed packages | Met | `yunari_packs::STARTER` is a committed package the host installs on an empty folder; the starter theme is built in Rust beside `blank()` |
| 11 | Local-first theme library | Met | `yunari-library` writes `themes/<id>/theme.json` + `assets/`; source themes are lifted on read |
| 12 | Browse the store catalogue | Partial | `GET /api/store/themes` proxies and caches the configured origin; the editor's `Store…` dialog lists name, kind, version and publisher. Listing thumbnails are not drawn, and the browser-side catalogue pages are not ported |
| 13 | Install a package, sign in, submit a theme, see own submissions | Done | Sign-in is the Worker's social flow through a one-use loopback state, with the session sealed by DPAPI and never shown to the browser. The `Store…` dialog installs a listing after the host checks its address, size and SHA-256, submits a stored theme through the version ladder, and lists this account's items with publish and unpublish |
| 14 | CPU, memory, disks, network, clock | Met | `system` + `clock` providers via `sysinfo`/`jiff` |
| 15 | Temperatures, fans, power, per-GPU detail through LHM | Met | Caption table with a generated binding list; absent LHM is one sentence per key |
| 16 | Frame metrics from RTSS or PresentMon | Met | RTSS shared memory first, PresentMon ETW fallback, rolling 60 s window |
| 17 | GPU counters and vendor APIs | Deferred | `gpu.*` is read from LHM only; no vendor SDK path |
| 18 | Missing readings stay missing | Met | `Reading::missing` with a reason; the dash is never a zero |
| 19 | Several monitors each showing a chosen theme | Met | A supervisor opens one fullscreen window per local display record on its monitor, replacing it when its assignment or the theme's bytes change; windows read the host's own provider registry |
| 20 | Choose one GPU and one disk for this PC; a theme may answer a slot itself | Partial | `devices.json` holds the assignment and the names beside it, read on every poll. LHM's `gpu.*`, `vram.*` and `disk.*` keys scope to the assigned card or drive, and an assigned device the PC no longer reports is a gap rather than another device's figures; the baseline provider answers `disk.used`/`disk.total`/`disk.used.percent`/`disk.name` from the system drive (first reported when unassigned) and `disk.data.*` only once a data drive is assigned. Settings cycles the GPU, system disk and data drive from the devices the providers report. Gap: the two providers mint ids from the name each reports, so a drive's id can differ between LHM and the baseline |
| 21 | USB panels: Lian Li 9.2"/8.8", TURZX 5", Kraken Z53/Z63/Z73 | Met | Still frames and controls (brightness, screen, restart, status, reset-device), one runner per display (phase 5) |
| 22 | Panel video, overlays, TURZX MP4 cache | Deferred | Needs the video decoder first (phase 1/5 notes) |
| 23 | Kraken paletted mode B | Deferred | Comes with video |
| 24 | Wallpaper Engine Workshop library and previews | Deferred | No provider; the asset finder that would show them is also deferred |
| 25 | Steam presence and metadata | Met | The foreground game is named from Steam's local library plus Epic manifests, GOG registry keys, itch app folders and Xbox packages. A game no catalogue claims is looked up by executable name on the Steam store (app-only results, edition-insensitive comparison, `game-metadata.json` cache with a ten-minute failure backoff and 500 entries); its portrait is downloaded once and served from the settings folder. `game.artwork` prefers Steam's cached box art, then the store cover |
| 26 | Weather backends and places | Met | Open-Meteo, WeatherAPI.com and OpenWeatherMap with faithful condition-code tables, the place from `weather.json`, Windows location as the fallback (a 24 h fix, asked through the embedded WinRT helper), and per-backend attribution. Backend keys live in the protected store and Settings names them. A stream's `?lang=` selects the German, Vietnamese or English condition and Windows-location words; a place the author chose is never translated |
| 27 | Now-playing media session | Met | SMTC helper, artwork cache and its route; audio endpoint volume/mute |
| 28 | Custom HTTP JSON sources; credentials in the host folder | Partial | GET sources with pointers, plain-text `headers` and `query` (both validated, a query value percent-encoded so it cannot add a parameter), and a per-source `timeoutMs` (500 ms–30 s). Paged sources fetch every page in order and merge them, failing whole when one page fails. `auth` kinds `none`/`bearer`/`basic`/`header`/`query` name a `credentialId` in the protected store; the secret is resolved at fetch time and a missing one is a reason, never a request without it. A user-authored address is resolved and refused when it lands on link-local/metadata or on this PC's own network (private, loopback, CGNAT; IPv4 inside mapped, NAT64, 6to4 and Teredo forms decoded). Redirects are not followed, so no credential ever rides a hop. Gap: the name is resolved then dialled by name, so a name that changes its answer in between is not caught |
| 29 | Extension platform | Dropped | Out of scope for v1 (this spec) |
| 30 | Store: five kinds, version ladder, verified download, staged refusals, kill list, publisher rules, consent | Partial | The ladder, verified download (https, same origin as the configured store, size and SHA-256) and staged refusals are ported, and installing a dashboard theme, widget pack or editor theme is wired into the `Store…` dialog. Path-preset packs and plugins are refused with a sentence. The kill list, publisher rules and the catalogue-index build command stay on the deployed Worker and are not reproduced host-side; install consent is the Install press itself |
| 31 | Readings are truthful | Met | Non-negotiable in `sample::{Reading, Status}` and every provider test |
| 32 | The theme is portable | Partial | Native JSON envelope plus `.yunari-theme` import/export with validation before write; background framing is authored document state, not yet split into `background-media.json` |
| 33 | The previous major is the compatibility floor | Deviation | The native schema is `1.0.0`; a source `3.0.0` theme is lifted on read and re-saved in the native shape, with the original beside it as `theme.fabric.json`. No 2.x migration, and any other major is refused |
| 34 | Data packs are host state | Partial | Installed widget packs live in `<settings>/widget-packs`; there is no other data-pack kind |
| 35 | Local by default | Met | Loopback bind; `--host` opts into LAN |
| 36 | Secure origin mandatory for install and wake lock | Met | Local CA and leaf, HTTPS beside HTTP on one port, `/yunari-ca.crt`; the root is ECDSA P-256 where the source used RSA-2048. Installing the root on a real phone was not exercised |
| 37 | Authoring state is what persists | Met | View state (zoom, lens, grouping view) lives in the editor shell, never in the document |
| 38 | No usage telemetry | Met | No analytics route or client code exists |
| 39 | Host surface (`@yunari/host`) | Met | `yunari-host`: one port, plain + TLS, SSE, pairing, themes, displays, panels, autostart |
| 40 | Player at `/`, display-only, installable | Met | The browser player paints the host's DisplayList over SSE; the planned WASM bundle was not needed, so the page stays small. Installing it is the browser's own prompt |
| 41 | Editor at `/editor`, `/settings` forwarded to it | Dropped | The native editor replaces both served surfaces (this spec) |
| 42 | `/pair` page | Met | Six-character code, ten-minute life, penalty and durable credential |
| 43 | Editor Themes/Settings modes, Publish, asset finder/import | Partial | Themes mode lists, opens, duplicates and deletes stored themes; the Settings dialog covers phones, autostart, monitors, the weather backend and place, the Windows-location switch, backend keys and the credentials a custom source names; the Store… dialog signs in, installs, submits a theme and publishes or withdraws this account's items. The sensor-provider and display settings panels and the asset finder are not ported |
| 44 | Tray menu, live LAN toggle, monitor enumeration API | Partial | The phone toggle is live: the host rebinds about a second after the setting changes. Settings lists this PC's monitors and turns each into a display record, and `GET /api/monitors` answers the same list over loopback with the uuid a local display record holds. Autostart is there too. There is no tray menu |
| 45 | MCP server over the host API | Partial | `POST /api/mcp` answers `initialize`, `tools/list` and `tools/call` for themes and sensors; the `yunari-mcp` binary bridges stdio to it through the port in `host.lock`. A skill installer is deferred |
| 46 | SignalRGB bridge | Deferred | Panels own their LEDs here and the vocabulary has no `lighting.*` keys, so there is nothing to bind yet |
| 47 | Windows is the first-class platform | Met | Windows reads sit behind seams (PowerShell helpers, HID/serial, registry); the workspace still builds and tests off Windows |
| 48 | npm, winget and packaged executable | Deferred | No installer yet; the app runs from a built binary |
| 49 | Deployed store Worker, D1 and R2 | External | Unchanged; this host only proxies whatever origin is configured |

What this sweep changes: nothing in the port's behaviour. It records the gaps
most likely to surprise a user coming from the source — no widget packs or store
installs (2, 9, 13), no panel video (22), no Wallpaper Engine (24) and a smaller
editor chrome (43) — and the deliberate deviations (native schema 1.0.0 in 33,
the WASM player not needed in 40).

## Performance (2026-11-16)

Measured on this development machine: Intel Core i9-10850K (10 cores, 20 logical
processors), 32 GB RAM, Windows 11 Pro 22631. Binary: `cargo build --release
--config 'profile.release.debug-assertions=true' -p yunari-app` — the optimized
profile, because the plain release build needs the Windows SDK's `fxc.exe` to
compile GPUI's HLSL shaders and this machine does not have that SDK (see
"Release build" below). Debug assertions do not change codegen or allocations
in any material way.

- **Idle cost, editor open and the host serving in the same process.** After a
  60 s warm-up, ten 30 s samples: working set 81.0–82.0 MB, private bytes
  92.4–93.4 MB, CPU 0.09–0.15 % of the machine — about 1.8–3.0 % of one core.
  `/api/health` answered `{"ok":true}` throughout. Method:
  `Start-Process target/release/yunari-app.exe`, then `TotalProcessorTime`
  deltas over 30 s windows, `WorkingSet64` and `PrivateMemorySize64`.
- **One frame, plan and raster.** `cargo test --release -p
  yunari-render-raster --test frame_cost -- --ignored --nocapture` paints a
  representative 1920×1080 theme (gradient background, three plates, a gauge, a
  line chart, two text nodes) through `yunari_render::build` and
  `yunari_render_raster::render`: mean 31.5 ms, p95 40.2 ms, min 21.5 ms, max
  43.0 ms over 60 frames. This is the software raster path the panels and the
  goldens use; the GPUI window shares the same plan and paints through the GPU,
  so its frame cost is not this number and was not isolated — the editor
  publishes no frame-time figure.
- **Not measured:** the Node baseline (the source stack was not run in this
  environment, so no side-by-side is claimed), GPUI window frame timings, and
  panel cadence (no panel was attached during this sweep).

## Release build

`cargo build --release` fails on `gpui-pre-windows`: its build script compiles
the renderer's HLSL with `fxc.exe` only when `debug_assertions` is off, and
`fxc.exe` ships in the Windows SDK. Running the app does not need it. Building
the shipped artifact on a machine without that SDK needs either the SDK
installed or a GPUI release that ships prebuilt shader bytes; until then the
installable artifact stays deferred (open follow-up: installer format and
update channel). The app runs today from a debug build or the optimized
debug-assertions build above.
