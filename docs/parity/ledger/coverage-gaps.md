# Coverage-gaps audit ledger

Source: `D:\git-repos\yunari` @ `1241866e53df252307e22ff3c51c842f89e458c8`
Port: this worktree, `2026-11-17`. Scope: rows R3, R6, R8, R9, R14, R15.

Every observation below comes from a headless raster probe built from the
port's public crates: `docs/parity/probes/coverage-gaps`. Build and run it from
the repository root:

```powershell
$env:CARGO_TARGET_DIR = "target"
cargo build --manifest-path docs/parity/probes/coverage-gaps/Cargo.toml
target/debug/coverage-gaps-probe.exe <mode>
```

`glass-sampling` and `video` need ffmpeg; set `YUNARI_FFMPEG` to an executable
or put `ffmpeg.exe` on `PATH`. The probe prints `skipped: no ffmpeg` rather
than drawing a false conclusion.

| Row | Area | Status | Evidence | Issue |
| --- | --- | --- | --- | --- |
| R3 | Frosted glass on shapes | **differs** | `glass-grain`: interior red `min 0 max 0, distinct 1`, panel pixel `[0,0,255,255]`; source also applies `saturate(1.6)` and 1.5 % grain (`scene-fabric/src/glass.ts:558-584`). `glass-sampling`: a `sampling:"static"` panel changes with live video, `1493/1600` pixels differ in 500 ms; source resolves sampling at `glass.ts:496-502,692-700`. | #14, #15 |
| R6 | Text layout, measurement, wrapping, overflow, vertical, formatting | **differs** | `short-box`: `20-unit box: 0 segments`; `overflow`: `layout clip None, 2 lines kept of 6`; source clamps to one line and removes the clip for `visible` (`authored-box.ts:310-327`). `vertical`: 11 segments for a ZWJ family emoji + `A\r\nB`; source stacks graphemes and drops CR/LF (`text-runs.ts:55-65`). `letter-spacing`: display lists equal with and without `letterSpacing`; source emits `charSpacing` (`paint.ts:85-88`). | #16, #17, #18 |
| R8 | Assets: image/SVG/GIF/video/font paths and media types | **differs** | `svg`: `center [0,0,0,0]`, `max alpha 0`; source rasterises SVG through `<img>` (`fabric-image.ts:52-63`). `avif`: document refused with `An asset kind must match its file extension.`; source accepts `avif` as an image (`asset-media-type.ts:18,35`). | #19, #20 |
| R9 | Theme video clips and animation | **matched** | `target/debug/coverage-gaps-probe.exe video` — a 1 s clip keeps painting past its duration and repeats: `300 ms center [0,16,49,255]`, `1500 ms center [0,16,49,255]`, `600 ms center [3,51,63,255]`, `1800 ms center [3,51,63,255]`, every sample differing from the previous by 1216–1492 of 1600 pixels. Source starts the backdrop video muted and looping (`scene-fabric/src/background-media.ts:412-415`). | — |
| R14 | Panel still fitting (rotation, circle crop, scale) | **differs** | `content-fit`: a 100×50 artboard authored `"contentFit":"cover"` in a 100×100 panel gives `top [0,0,0,0] middle [255,0,0,255] bottom [0,0,0,0]` — the contain rule's bars. `crates/raster/src/lib.rs:804` always uses `min`; source selects `min`/`max` from `contentFit` (`renderer-core/src/artboard.ts:119-120,165-171`). Rotation and circle crop were not the failing part in this probe. | #21 |
| R15 | Formatting and localization of units, numbers, dates | **matched** | `cargo test -p yunari-locale --quiet` 7 passed; `cargo test -p yunari-render --quiet instant::tests` 6 passed; `cargo test -p yunari-render --quiet format::tests` 9 passed; `cargo test -p yunari-providers --quiet units` 10 passed. Source is Intl/CLDR (`renderer-core/src/scene/datetime/format.ts`, `names.ts`, `scene/measurement.ts`); the port's ICU4X paths match on the sampled languages and units covered by these suites. | — |

## Notes

- R3's expected statement in the matrix names blur radius and sampling. Blur
  radius matches; sampling does not, and the finish is absent. Both differences
  are filed.
- R6 covers several independent behaviours. The probe confirms four; the
  line-placement/ellipsis cases not probed here are not claimed as matched.
- R9 was checked through the headless raster path, not the GPUI window. The
  clip decodes against the wall clock, advances, loops, and stays muted by
  construction (the port passes `-an`); no driven-window comparison was made
  for animation easing.
- R15 was checked through the existing test suites, not through a driven
  window; only sampled languages/units are covered.
