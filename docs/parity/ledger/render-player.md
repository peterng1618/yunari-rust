# Render + player parity ledger

Source: `D:\git-repos\yunari` @ `1241866e53df252307e22ff3c51c842f89e458c8`.
Port: `D:\git-repos\yunari-rust` at `port/native-app` (no port source modified).

Evidence runs used for the rows below:

- `cargo test -p yunari-render -p yunari-render-raster -p yunari-player -p yunari-model` — 252 passed, 1 ignored (11 suites).
- `cargo test -p yunari-host --lib stream` — 7 passed.
- Live port app on `http://127.0.0.1:5227` with a one-theme app dir; live source host on `http://127.0.0.1:4175`.
- Screenshots at 1920x1080 under `test-results/parity-shots/` (native captures are 1936x1119 outer, 1920x1080 client).

| Row | Status | Evidence |
|---|---|---|
| R1 | matched | `cargo test -p yunari-render --lib background` (14 tests: cover, scale, centre letterbox, pan, edge bands, fade, ramp, masks) passes; `crates/render/src/background.rs` mirrors `background-composition.ts` decision for decision. |
| R2 | matched | `cargo test -p yunari-model --lib validate` (`accepts_square_circle_artboard`, `refuses_circle_artboard_that_is_not_square`) and `cargo test -p yunari-render-raster --lib` (`a_square_panel_takes_any_angle_and_clips_its_disc`, `a_landscape_theme_is_contained_in_a_portrait_panel`) pass. |
| R3 | not checked — no glass golden and no driven glass window; `crates/raster/src/lib.rs` reads as blur-only (no saturation/grain) but without a fixture the visual result was not compared. |
| R4 | differs #6, #7 | Line settings/paint and gauge threshold paint reproduced; gauge, line, bar, pie, radar and polar-bar geometry were read but only these two differences were filed. |
| R5 | differs #7 | Threshold bands with an unresolved palette ref paint the survivors; source refuses the whole fill. |
| R6 | not checked — number/date formatting was exercised by tests, but wrap points, ellipsis, vertical text and alignment were not compared against `measurement.ts` on a rendered theme. |
| R7 | matched | `crates/render/src/plan.rs:642` default size ladder; `plan::tests::a_run_authored_size_wins_over_the_role_default` and token/run validation tests pass in the full suite. |
| R8 | not checked — no driven import; only `crates/model/src/validate.rs` asset/face validation was read, not exercised against source media sniffing. |
| R9 | not checked — no video/animation theme was run; clip start/loop/stop and wall-clock stills were not compared. |
| R10 | matched | `MISSING_VALUE_TEXT` "—" with a reason; `history::tests::a_missing_reading_is_a_gap_not_a_zero` and `plan::tests::a_value_run_with_no_reading_reads_the_placeholder_and_says_which_key` pass. |
| R11 | differs #1, #2, #3, #8 | Hosted stream dead (#1), chart-only sensors never requested (#2), no display controls (#3), no service worker/install (#8). |
| R12 | differs #1, #5 | Live stream delivered no `samples` event; the batch its own test asserts has numeric `sentAt` and flat samples instead of `SampleEntry`. |
| R13 | differs #4 | A failed published fetch leaves only `Connecting…`; source replaces the display with the failure page and both recovery actions. Availability notice and host refind were not reproduced, and are not covered by a filed issue. |
| R14 | not checked — no panel hardware and the panel protocol test does not cover fitting; `crates/raster/src/lib.rs::fit_into` was read only. |
| R15 | not checked — `format::tests::*` and `instant::tests::*` pass, but localized names and the source's `Date.UTC` day rollover were not compared end to end; a static difference in `crates/render/src/instant.rs` was not filed under the 8-issue cap. |

Issues filed from this pass: #1, #2, #3, #4, #5, #6, #7, #8.

Rows with no contract evidence (R3, R6, R8, R9, R14, R15) are recorded as not
checked rather than matched; they remain coverage gaps for the next pass.
