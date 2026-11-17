# Parity loop — pass summary

One row per pass. Numbers are verified state, not filed work: an issue counts
only after an independent verifier confirmed it.

| Pass | Date | Source | Audited rows | Issues filed | Triage precision | Fixes verified | Escape rate | Test independence | Probed rows | Tokens / wall clock |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 (render + player pilot) | 2026-11-17 | `1241866e` | R1–R15 first look; R3, R6, R8, R9, R14, R15 unchecked | 8 (#1–#8) | 8/8 = 100% | 2 (#6, #7) | 0/2 reopened | 2/2 new tests fail on parent | 12/15 (R3, R9, R13 unprobed) | ~1.1M subagent tokens, ~2 h |

## Status at the end of pass 1

- Landed on `main`: threshold-fill refusal (#7), line-chart area/markers/axis
  (#6), probe registry + sweep + differential fixtures, coverage-gap ledger.
- In flight: stream sample delivery (#1, #5), chart-only binding polling (#2),
  text layout batch (#16–#18), asset decoding batch (#19–#20).
- Queue: glass finish/sampling (#14, #15), panel `contentFit` (#21), player
  display controls (#3), player load-failure state (#4), service worker (#8).
- Unprobed rows are the next audit's first target: R3, R9, R13. R3 and R9 now
  have audit evidence (ledger `coverage-gaps.md`) but no registered probe yet.

## What the pass changed in the loop

- Coverage is now probe-first: `scripts/parity-sweep.ps1` runs 12 registered
  probes with no agents; audits spend their tokens on failures, unprobed rows
  and a 10% risk-weighted resample.
- Differential fixtures (`scripts/parity-fixtures.mjs`) make the source an
  executable oracle. Their first run found two real rounding divergences,
  both fixed before this pass closed.
