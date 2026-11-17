# Parity review & fix loop

A repeatable loop that finds where the Rust port differs from the Node app's
behaviour, files each difference as a GitHub issue with reproducible evidence,
fixes them one at a time behind tests, and verifies the fix independently.

The loop exists because "the spec says Met" is a claim, not evidence. Every
stage below is built so a claim without an artifact cannot pass.

## Roles

Four roles, each a fresh agent with a narrow brief. No agent does two roles on
the same finding.

| Role | Does | Does not |
|---|---|---|
| Auditor | Derives expected behaviour from the source app, inspects the port, files issues with a repro | Edit port code, edit the parity sweep, close issues |
| Triager | Reproduces, dedupes, classifies, labels `state:ready`; closes invalid | Fix code |
| Fixer | Writes a failing test first, fixes the smallest cause, opens a PR | Triage their own issue, mark it verified |
| Verifier | Reproduces on the parent commit, checks the fix and the test, runs the surface, closes or reopens | Trust the fixer's description or evidence |

The verifier sees only the issue and the PR diff — not the fixer's reasoning.
That is deliberate: a wrong explanation should not survive by being persuasive.

## Stage 0 — one-time setup

1. Pin the source commit under review (`git -C D:\git-repos\yunari rev-parse
   HEAD`). Every issue names it.
2. Build the capability matrix, `docs/parity/matrix.md`, one row per
   capability from the source's `docs/product.md`. Each row carries:
   - source entry points (file/symbol),
   - port entry points,
   - the behaviours a user can observe,
   - how each is checked (test, driven window, screenshot, HTTP call, pixel
     diff),
   - the evidence path that satisfies it.
   The matrix is the audit's coverage ledger. An auditor may not leave a row
   unchecked; "no finding" still needs a ledger line.
3. Labels: `parity`, `surface:<editor|render|player|host|providers|panels|mcp|store|packs|tray>`,
   `class:<regression|missing|wrong-default|interaction|copy|layout|state|perf|a11y>`,
   `severity:<S1|S2|S3|S4>`, `state:<needs-repro|ready|in-fix|fixed|verified|wontfix>`.
4. Issue template `.github/ISSUE_TEMPLATE/parity.yml` with the fields below as
   required.
5. Probe helpers:
   - source: the existing Playwright setup in `src/web` (the editor serves at
     `/editor`; `playwright.parity.config.ts` shows the pattern),
   - port: a headless raster probe plus window capture; keyboard-only driving
     through the app's own test seam. Never take over the user's pointer or
     keyboard.
6. A filing script (`scripts/parity-file.ps1`) that takes one JSON file and
   calls `gh issue create` — no free-text shell quoting.

## Stage 1 — audit (per surface)

One auditor per surface, run in parallel, each blind to the port's spec claims.
The brief is fixed (see "Auditor brief").

Order of derivation matters:

1. Read the source implementation and its tests. Write down the expected
   behaviour *before* opening the port file for that capability.
2. Inspect the port at the cited path.
3. Run the port. A static read alone may only produce a `suspect` finding.
4. Fill the ledger row: matched, differs (issue number), or no port surface
   (issue).
5. File issues; write the ledger to `docs/parity/ledger/<surface>.md`.

A finding is only filed when it has both sides:

```markdown
## Source behaviour
`src/web/packages/.../x.ts:123` — what it does, with the snippet or test name.

## Port behaviour
`crates/.../x.rs:456` — what it does instead, or what is missing.

## Expected vs observed
One sentence each. Observable by a user, not an implementation opinion.

## Reproduction
Exact commands/keys/theme JSON from a clean state. Include the run that shows
the port differing.

## Evidence
Command output, screenshot path, test name + result, or pixel diff path.

## Classification
surface · class · severity · confidence (confirmed|suspect) · source commit
```

Severity: S1 data loss / security / wrong reading shown as real; S2 a feature
that cannot be used as documented; S3 visibly wrong behaviour or copy; S4
polish.

Classes of regression the audit must look for explicitly:

- **Behaviour changed while the feature exists** — defaults, empty states,
  ordering, rounding, validation messages, debounce, clamping, unit conversion.
- **Wrong state persistence** — what a save writes, what a reopen restores,
  undo/redo boundaries, selection and camera leaking into documents.
- **Interaction gaps** — keyboard paths, focus order, confirmations, refusal
  messages, disabled states, error recovery.
- **Missing controls** — a source control that has no port equivalent, or a
  port control wired to nothing.
- **Copy drift** — product vocabulary, refusal sentences, labels.
- **Aesthetic/functional UI drift** — a control gone, unreachable, mis-wired,
  unlabelled, clipped, too small, or a state that cannot be told apart. The
  gpui-kit adaptation may differ in chrome, spacing and platform font; it may
  not lose a function or read worse than the source.
- **Silent fallbacks** — a missing reading, asset or face drawn as something
  plausible instead of an absence with a reason.

## UI screenshots

Every UI or layout finding is judged at fullscreen in a 16:9 1920x1080 client
area. A smaller viewport is not a substitute; if the host panel cannot show
that size, the shot is taken offscreen/through the raster path and the issue
says so.

- **Source:** start the source through its own host (its Playwright webServer
  boots the real host and serves `/editor` and `/player`; a bare
  `vite preview` has no API and only shows the read-failure state), then drive
  Chromium with viewport `{width: 1920, height: 1080}` and screenshot the full
  viewport.
- **Port:** `scripts/capture-window.ps1 -Exe <Yunari.exe> -Width 1920 -Height
  1080 -Out <png>` resizes the window's client area and captures it with
  `PrintWindow`/`PW_RENDERFULLCONTENT`, so the window is never foregrounded and
  the pointer and keyboard stay the user's. The raster path can render the same
  theme at 1920x1080 for a pixel-level check.
- Same theme document, same artboard, same zoom/fit, same language, same
  scroll position on both sides.

The port is not a pixel copy: its UI is adapted to gpui-kit. Pixel positions,
widget chrome, platform fonts and exact spacing are allowed to differ. What must
survive is **function and aesthetic intent**.

Judge, in this order:

1. **Function**: every source control and state exists and does what it did;
   validation, refusals, defaults and persistence behave the same.
2. **Information architecture**: the same things are grouped, discoverable and
   reachable; no action moves behind a path a user would not find.
3. **State communication**: selected, hover, focus, disabled, empty, loading,
   error are all visible and distinguishable.
4. **Aesthetic craft**: alignment, rhythm, spacing scale, contrast, hierarchy,
   truncation, hit targets, small-window behaviour. An adapted layout passes if
   it reads at least as clearly as the source; it fails if it looks unfinished,
   crowds controls, clips labels or buries the primary action.

A valid UI finding names the observable loss: a control that is gone,
unreachable, mis-wired, unlabelled, clipped, too small to hit, or a state that
cannot be told apart — with a 1920x1080 shot of both sides and the concrete
region. "Spacing differs", "control looks different", "font is system" are not
findings by themselves; a deliberate, coherent adaptation is the point of the
port. A pixel diff is supporting evidence, not the verdict.

Shots live under `test-results/parity-shots/` (not committed); the issue names
the exact file paths and what to look at, and the verifier regenerates them.

## Stage 2 — triage

The triager runs every repro on a clean checkout and marks:

- `confirmed` → `state:ready`,
- not reproducible → comment with what was run, close as invalid,
- duplicate → close, link the survivor,
- suspect only → keep `state:needs-repro` with the missing evidence named.

Two auditors independently sample 10% of the other's closed-as-matched ledger
rows. Disagreement becomes a new audit task, not a debate in the issue.

## Stage 3 — fix

One fixer per issue, or one per group of issues touching the same file.

1. Write a test that fails for the source-expected behaviour. Paste the failing
   output into the PR.
2. Fix the smallest cause. No unrelated refactors, no weakened assertions, no
   `#[ignore]`, no rewritten snapshots without a sentence saying why.
3. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D
   warnings`, `cargo test --workspace`.
4. Commit on `parity/<issue>-<slug>`; PR body: source citation, port citation,
   failing-before output, passing-after output, `Closes #<issue>`.
5. Never edit `docs/superpowers/specs/...` parity rows in the same PR unless
   the issue itself is about that claim, and then only with the test linked.

## Stage 4 — verify

The verifier, on a clean worktree:

1. Checks out the PR's parent, applies the new test alone, and confirms it
   fails for the stated reason.
2. Applies the fix, confirms the test passes and the behaviour matches the
   source citation.
3. Runs the affected surface's checks (HTTP calls, driven window, screenshot)
   independently of the fixer's artifacts.
4. Reads the diff for scope and for weakened tests.
5. `verified` + close, or reopen with evidence.

A fix that passes its own test but changes unrelated behaviour is a reopen.

## Stage 5 — lock it in

Every verified issue leaves one regression case behind: a test in the owning
crate, a golden fixture, or a matrix row whose evidence path now points at
that test. The next audit starts from the matrix, so fixed regressions can only
return as new failures.

## Backlog drain

- Queue: `gh issue list --label state:ready`, ordered `severity` then oldest.
- One or two fixers in parallel, only on disjoint crates; otherwise sequential
  to keep diffs and test runs sane.
- Batch cap per pass (for example 10 fixes) and a re-audit after every pass.
  Bigger batches make verification shallow.
- `wontfix` requires a one-paragraph product reason recorded in the issue; the
  matching matrix row says "dropped" with that reason, not "met".

## Metrics that stop self-deception

- **Precision:** valid issues / filed issues, from triage. Below ~70% means the
  auditor brief or evidence bar is too loose.
- **Escape rate:** issues the verifier reopens / fixes attempted. Non-zero
  means the fix stage is claiming too early.
- **Coverage:** ledger rows with evidence / total matrix rows.
- **Test independence:** fixes whose new test fails on the parent / all fixes.
  Anything else is a tautological test.

These are reported per pass in `docs/parity/ledger/summary.md`.

## Rollout

1. Pilot one surface, `render` + `player`, to calibrate the template and
   precision.
2. Then `editor` (highest UI-regression risk), `host` + `mcp`, `providers`,
   `panels`, `store` + `packs`, `tray`.
3. After the first full pass, re-run with the fixed matrix and the regression
   suite as the seed.

## Agent briefs

**Auditor**

> You are auditing the Rust port in `D:\git-repos\yunari-rust` against the Node
> app in `D:\git-repos\yunari` at commit `<sha>`, surface `<surface>`.
> Do not read or trust the port's parity-sweep document; it records claims.
> 1. Read the source for every capability in `docs/parity/matrix.md` rows under
>    your surface. State expected behaviour before opening the port file.
> 2. Inspect the port at the matrix's port entry points and run it.
> 3. For every difference, file one issue with the template fields (source
>    citation, port citation, expected vs observed, reproduction, evidence,
>    classification). No finding without both citations and a repro.
> 4. Fill every ledger row, including matches. Write
>    `docs/parity/ledger/<surface>.md`.
> 5. UI/layout findings need 1920x1080 fullscreen shots for both sides
>    (source through its own host, port through `scripts/capture-window.ps1`),
>    and judge function and aesthetic intent, not pixel identity: gpui-kit
>    adaptation is expected; a lost function, hidden action, unreadable state or
>    cramped/clipped control is the finding.
> You may not edit port code. If you cannot reproduce, file it as `suspect`
> with the missing evidence named.

**Fixer**

> Pick issue `#<n>` (`state:ready`). Reproduce it first from the issue alone.
> Add the failing test, paste the failing output, fix the smallest cause, run
> the three gates, open a PR that closes the issue. Do not touch other
> behaviour, the parity sweep, or the test's assertions after it passes.
> If the issue is wrong, say why with evidence and stop.

**Verifier**

> Issue `#<n>`, PR `<url>`. Do not read the fixer's explanation first. On a
> clean worktree: apply only the new test to the PR's parent and confirm it
> fails for the stated reason; then apply the PR and confirm the behaviour
> against the source citation. Run the surface's checks yourself. Report
> `verified` or reopen with the command output that makes the decision.

## Risks

- **Source app drift** — always name the source commit; a moving target turns
  the matrix into fiction.
- **Screenshot noise** — compare tokens, geometry and copy before pixels; a
  pixel diff is evidence, not a verdict.
- **Issue flooding** — the batch cap and triage gate keep the backlog
  drainable; a 200-issue dump with 40% precision costs more than it fixes.
- **Hardware paths** — panels and phone pairing need a device. The loop files
  them as `state:needs-repro` with the manual checklist in `docs/install.md`
  and never marks them verified from tests alone.
