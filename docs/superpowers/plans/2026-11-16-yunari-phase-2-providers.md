# Phase 2 — Providers, sampler, bindings and units

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A window shows real readings from this PC, and any key nothing can read renders as a gap with its reason.

**Architecture:** A `yunari-providers` crate owns the semantic-key vocabulary, the provider trait, the registry that turns many displays' asks into one poll, and the baseline providers. Readings stay as measured: conversions and formatting happen once, at presentation, in `yunari-render`, so a stored or transmitted value is never a derived one.

**Tech Stack:** Rust 2024, `sysinfo` for baseline system readings, `serde` for the document, the existing `yunari-render`/`yunari-raster`/`render-gpui` pipeline.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md` (provider registry, sampler, bindings, units; phase 2 row and its exit).

## Global Constraints

- Readings are never zeroed to hide an absence: a missing or failed reading is absent, with a reason, and the frame draws the em-dash placeholder the renderer already owns.
- Conversion happens at presentation, after the binding's authored `scale`/`offset`, never in the sample or on disk.
- A unit the key cannot reach is refused (plan issue) and the measured form is shown; the reading is never dropped because its unit was.
- Provider failures are isolated: one provider throwing does not stop a poll, and its message is redacted before it could reach a browser.
- Windows-first; the provider trait is the seam for other platforms and for tests.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` stay green.

## Review Focus

- A key nothing can read must render the gap with the unmapped reason, not an empty box and not a zero.
- Two displays asking for different keys must cost one poll of the union, and a provider answering fewer keys than asked must not make the rest look like zeros.
- A provider that throws mid-poll must leave the other providers' readings intact.
- `scale`/`offset` must apply once and only to numeric readings; a text instant must not be scaled.
- A refused unit must still show the measured value, with an issue naming the unit.

---

### Task 1: Vocabulary

**Files:**
- Create: `crates/providers/Cargo.toml`, `crates/providers/src/lib.rs`, `crates/providers/src/vocabulary.rs`
- Modify: `Cargo.toml` (member + workspace dependency)

**Interfaces:**
- Produces: `SemanticKeyDescriptor { key, family, label, unit, unit_kind, display_units, value_kind, media_kind, expected_tier, converts, instant }`, `pub const SEMANTIC_KEYS: [..]`, `pub fn describe(key: &str) -> Option<SemanticKeyDescriptor>`, `pub fn label_for(key: &str) -> &str`, `pub fn families() -> &'static [&'static str]`.

- [x] **Step 1:** Generate `vocabulary.rs` from the source vocabulary (`renderer-core/src/data/semantic-keys.ts`) with a one-off script; 101 built-in keys must survive with their `unit`, `unitKind`, `displayUnits`, `expectedTier`, `converts` and `instant` fields.
- [x] **Step 2:** Write tests: every key's `family` matches a built-in family; `describe("cpu.load").unit == Some("%")`; `describe("cpu.temp").converts == Some(Converts::Temperature)`; `describe("time.now").instant == Some(Instant { default_format: "HH:mm:ss" })`; an unknown key returns `None` and `label_for` returns it verbatim.
- [x] **Step 3:** Port the discovered-key shapes (`cpu.core.N.load|temp|clock`, `cooling.fan.<slug>.rpm|percent`, `cooling.pump.<slug>.rpm`, `motherboard.fan.<slug>.*`, `motherboard.voltage.<slug>`, `gpu.fan.N`, `disk.<slug>.<quantity>`, `network.<slug>.download|upload`, `psu.voltage.<slug>`, `custom.<src>.<field>`) as `describe` fallbacks, with one test per shape and one for a slug that is too long.
- [x] **Step 4:** `cargo test -p yunari-providers` and clippy green; commit.

### Task 2: Units and conversion

**Files:**
- Create: `crates/providers/src/units.rs`
- Test: same file (`#[cfg(test)]`)

**Interfaces:**
- Produces: `UnitPreferences { measurement, clock, data, data_scale, rate, rate_scale, wind_speed, pressure }`, `Default` matching the source defaults; `Overrides { unit, auto_scale }`; `Converted { value: f64, unit: Option<&'static str>, precision: Option<u8>, unsupported: bool }`; `pub fn convert(key: &str, value: f64, unit: Option<&str>, preferences: &UnitPreferences, overrides: Overrides) -> Converted`.

- [x] **Step 1:** Port `convertForDisplay`: temperature (°C→°F), clock (MHz→GHz with 2/1 decimals), data (binary/decimal, MB/GB/TB), data rate (bits/bytes, k/m/g), speed (km/h→mph, m/s), pressure (hPa→inHg). Other kinds pass through.
- [x] **Step 2:** Tests: 25 °C with imperial → 77 °F; 3600 MHz with `clock: ghz` → 3.6 GHz precision 2; 1200 MHz with `auto` → 1.2 GHz; 1 GB (2^30) with `data: decimal` → 1.073741824 GB; a binding unit of `"GiB"` pins binary; a unit the key cannot reach sets `unsupported` and keeps the measured value; scale/offset are *not* applied here (the caller does that).
- [x] **Step 3:** Commit.

### Task 3: Reading types, provider trait, registry

**Files:** create `crates/providers/src/sample.rs`, `provider.rs`, `registry.rs`.

**Interfaces:**
- Produces: `Reading { semantic_key, status, value, unit, text, instant, message }`, `Status { Ok, Missing, Error }`; trait `Provider { fn id(&self) -> &str; fn label(&self) -> &str; fn describe(&self) -> Vec<Descriptor>; fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String>; fn health(&self) -> Health }`; `Registry::{new, add, describe, sample(&[Ask]) -> Cycle}`, `Cycle { readings, failures, unmapped }`.

- [x] **Step 1:** Registry unions the asks' keys, sorts them, polls each provider once with that union, and marks every requested key no provider answered as unmapped.
- [x] **Step 2:** Tests: one poll for two asks with overlapping keys; a provider returning Err leaves other readings and reports a failure; a key no provider knows is unmapped exactly once; a provider that answers a key it was not asked for is ignored.
- [x] **Step 3:** Commit.

### Task 4: Clock provider

**Files:** create `crates/providers/src/clock.rs`.

- [x] **Step 1:** `time.now` and `date.today` as ISO 8601 instants, in the configured zone when one is set and the local zone otherwise; unit `-`-free text kind.
- [x] **Step 2:** Tests: a fixed instant formats as the expected ISO string; `date.today` is the same instant's date; an unknown zone falls back to local rather than failing the poll.
- [x] **Step 3:** Commit.

### Task 5: Baseline system provider

**Files:** create `crates/providers/src/system.rs`; add `sysinfo` dependency.

- [x] **Step 1:** `cpu.load`, `ram.used`, `ram.total`, `ram.percent`, one `disk.<slug>.*` set per mounted disk, and one `network.<slug>.download|upload` set per interface, each with the vocabulary's unit and the `baseline` tier.
- [x] **Step 2:** Tests with an injected snapshot (the sysinfo calls behind a small `Snapshot` trait) so expectations do not depend on the machine: two disks produce two key sets; an absent reading is `Missing`, not `0`.
- [x] **Step 3:** Commit.

### Task 6: Sampler

**Files:** create `crates/providers/src/sampler.rs`.

**Interfaces:**
- Produces: `Sampler::{new(registry), request(keys), tick(now_ms) -> Option<Cycle>}` where a tick polls only when a request is due by cadence, and coalesces overlapping ticks into one poll.

- [ ] **Step 1:** Tests: two ticks closer than the cadence produce one poll; a new key request makes the next tick due; a slow provider is not re-entered while it is still running.
- [ ] **Step 2:** Commit.

### Task 7: Bindings render readings

**Files:** modify `crates/render/src/plan.rs` (`text_shape`), add `crates/render/src/reading.rs`.

**Interfaces:**
- Consumes: `Reading`, `UnitPreferences`.
- Produces: `pub struct Readings(HashMap<String, Reading>)` and `pub fn build_with(document: &ThemeDocument, fonts: &mut Fonts, readings: &Readings, preferences: &UnitPreferences) -> (DisplayList, Vec<PlanIssue>)`; `build` stays as `build_with(.., &Readings::default(), &UnitPreferences::default())`.

- [ ] **Step 1:** A numeric reading applies `scale`/`offset`, converts, formats with the authored precision, then appends the unit.
- [ ] **Step 2:** Tests: a binding with `scale: 1024, unit: "KiB"` shows the converted figure; a missing reading draws the em-dash with a `MissingReading` issue naming the key; an unsupported unit keeps the measured value and reports the unit.
- [ ] **Step 3:** Commit.

### Task 8: Live window

**Files:** modify `crates/app/src/main.rs`.

- [ ] **Step 1:** Poll the sampler on a timer, re-plan the frame on each changed cycle, and repaint the window.
- [ ] **Step 2:** Exit check: run the app against a theme whose bindings name `time.now`, `cpu.load` and one unknown key; the first two tick and the third shows the em-dash with its reason in the log.
- [ ] **Step 3:** Commit.
