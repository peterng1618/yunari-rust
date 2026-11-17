# Parity capability matrix

Source: `D:\git-repos\yunari` at commit `1241866e53df252307e22ff3c51c842f89e458c8`
(2026-11-15). Port: this repository. Rows are the audit's unit of coverage: an
auditor may not leave one unchecked, and a row marked matched needs the evidence
it names, not a sentence.

| # | Capability | Source entry points | Port entry points | Observable behaviour | Evidence |
|---|---|---|---|---|---|
| R1 | Background modes and media composition (colour, cover, contain, anchor, extent, fade, pan/zoom/flip) | `src/web/packages/renderer-core/src/theme/background-modes.ts`, `background-composition.ts` | `crates/render/src/background.rs`, `crates/raster/src/lib.rs`, `crates/render-gpui/src` | A theme's background paints in the same mode, placement and crop at the same artboard size | Port test/ledger + rendered comparison |
| R2 | Rectangular and circular artboards, disc cropping | `renderer-core/src/artboard.ts`, `artboard.test.ts` | `crates/model/src/validate.rs`, `crates/render/src/plan.rs`, `crates/raster/src/lib.rs`, `crates/render-gpui/src` | Circle artboard is square, corners cropped, rect theme on a round panel inscribed | Validation test + raster check |
| R3 | Frosted glass on shapes | `renderer-core/src/theme/glass.ts`, `glass.test.ts` | `crates/render/src/plan.rs` (glass op), `crates/raster/src`, `crates/render-gpui/src` | Glass blur radius and sampling match; corners of a disc crop | Golden/fixture + driven window |
| R4 | Six chart families (gauge, line, bar, pie, radar, polar bar) | `renderer-core/src/charts/{gauge,line,bar,pie,radar,polar-bar}.ts` and their tests | `crates/render/src/chart/*` | Same geometry, defaults, axis furniture, categories and empty states | Chart tests + golden + ledger |
| R5 | Chart paints, thresholds, gradients, palette resolution | `renderer-core/src/charts/fill.ts`, `chart-paint.ts` | `crates/render/src/chart/mod.rs`, `crates/render/src/plan.rs` | A paint resolves to the same colour or gradient; unresolvable paint is a reason, not a substituted colour | Tests + ledger |
| R6 | Text layout: measurement, wrapping, alignment, overflow, vertical, line height, instant/number formatting | `renderer-core/src/scene/measurement.ts`, `fonts.ts`, `datetime/format.ts`, `datetime/names.ts` | `crates/render/src/text.rs`, `instant.rs`, `reading.rs`, `format.rs`, `crates/locale` | Same wrap points, ellipsis, alignment and formatted words/numbers for the theme's language | Tests + rendered comparison |
| R7 | Type tokens, type size ladder, rich-text runs | `renderer-core/src/theme/type-size.ts`, `palette-tokens.ts` | `crates/model/src/document.rs` (tokens/runs), `crates/render/src/plan.rs` | Default sizes per role, token family/weight, run overrides | Tests |
| R8 | Assets: image/SVG/GIF/video/font paths, media types, declared faces | `renderer-core/src/theme/assets.ts`, `asset-media-type.ts`, `scene/fonts.ts` | `crates/model/src/validate.rs`, `crates/render/src/image.rs`, `video.rs`, `text.rs`, `crates/raster/src/images.rs` | Correct asset chosen, media type sniffed, face registered or refused with a reason | Tests + driven import |
| R9 | Theme video clips and animation | `renderer-core/src/scene/animation.ts`, `animation.test.ts`, host clip plumbing | `crates/render/src/video.rs`, `crates/raster/src/images.rs`, `crates/render-gpui/src` | A clip starts, loops/stops as the source does, and a still frame matches the wall clock | Test + driven window |
| R10 | Readings, absence and gap reasons | `renderer-core/src/scene/plan.ts` (value runs, issues), `src/web/packages/host/src/...` sampling | `crates/render/src/reading.rs`, `crates/providers` | A missing key is an absence with a reason, never a zero or plausible placeholder; units convert at presentation | Tests + ledger |
| R11 | Player surface: display-only, fullscreen/monitor, wake lock, install, no authoring controls | `src/web/packages/player/src/{main.ts,chrome.ts,display-controls.ts}`, host page routes | `crates/player/src`, `crates/host/src` (player page), `crates/app/src` | What the player shows, hides and refuses; wake lock and install prompts | Driven window + HTTP + ledger |
| R12 | Live frame stream (SSE) contract: version, samples, frame DisplayList, connected event, limits | `src/web/packages/player/src/hosted-plan.ts`, host stream code | `crates/host/src/stream.rs`, embedded `player.html`/`player.js` | A phone receives the same event shape and paints the same frame | HTTP test + captured stream |
| R13 | Player states: loading, load failure, availability notice, host refind | `player/src/{load-failure.ts,availability-notice.ts,host-refind.ts,load-failure-reason.ts}` | `crates/player/src`, `crates/host/src` player page | Each state appears with the source's copy and recovery action | Driven states + ledger |
| R14 | Panel still fitting (rotation, circle crop, scale) | source panel render path | `crates/host/src/panels.rs`, `crates/render-raster` | A theme fits the panel's glass exactly as the source does | Protocol test + manual checklist |
| R15 | Formatting and localization of units, numbers, dates | `renderer-core/src/scene/datetime/format.ts`, `names.ts`, unit tables | `crates/render/src/instant.rs`, `format.rs`, `crates/providers/src/units.rs`, `crates/locale` | Same separators, words and unit spellings for the same language and preferences | Tests + ledger |

Ledger files: `docs/parity/ledger/<surface>.md`.

An auditor records per row: `matched` (with evidence), `differs #n`, `missing
#n`, or `not checked — <why>`. "Not checked" is a finding of its own if the
matrix says the capability ships.
