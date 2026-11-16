# Phase 6: Providers and Integrations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every semantic key the product promises has a provider on Windows, and the three integrations the desktop app owns (theme store, MCP server, SignalRGB bridge) answer on the same loopback API.

**Architecture:** Each source module becomes a provider implementing the existing `Provider` trait; the registry polls them together, so an absent backend is a `missing` reading with a reason rather than an error. Table-driven mapping (LHM captions, steam catalogue) stays data plus one matcher, never a branch per caption.

**Tech Stack:** Rust 2024, reqwest-free HTTP via `std::net`+`ureq`? No — the workspace already uses `ureq` nowhere; HTTP providers use `reqwest` blocking or `ureq`. `ureq` (blocking, small) is the choice; it is a new workspace dependency only for providers.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- Windows-first; every OS-specific read sits behind a trait seam so its tests run on any host.
- A provider never panics and never blocks the registry past its own timeout (2 s for LHM, 3 s for weather, 2 s for custom HTTP).
- A sensor that cannot be read is `missing` with an owner-actionable sentence, never a zero.
- Clipboard of source behaviour: exact key names from `crates/providers/src/vocabulary.rs` (already generated); exact refusals from the source module named in each task.
- Workspace gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.

## Review Focus

- A machine where LibreHardwareMonitor is not installed: every LHM key is `missing` with the "install or start LHM" reason, and the host still starts.
- LHM publishing `"NaN"` or `"3.0 %"`: the first is a gap, the second is 3.0 — never NaN in the JSON.
- Two GPUs (iGPU + dGPU): `gpu.*` reads the discrete one, and the dynamic per-instance keys still address both.
- A weather fetch that times out: the reading is `missing` with the timeout reason, not the previous hour's weather silently.
- Prices/forecast from the store or weather endpoints are untrusted data: rendered as text, never interpreted as a route or a file path.

---

### Task 1: LHM tree and the static bindings

**Files:**
- Create: `crates/providers/src/lhm/mod.rs`, `crates/providers/src/lhm/tree.rs`, `crates/providers/src/lhm/bindings.rs`
- Test: same files

**Interfaces:**
- Consumes: `Provider`, `Descriptor`, `Reading`, `sample::{Reading, Status}`.
- Produces: `LhmSensor { sensor_id, hardware_id, hardware_type, text, kind, value: Option<f64> }`; `flatten(payload: &serde_json::Value) -> Vec<LhmSensor>`; `Binding { key, group, kind, any_text: &'static [&'static str], combine }`; `group_of(name, id) -> Option<Group>`; `readings_for(sensors, key) -> Option<(f64, unit)>`.

- [x] Test: `"3.0 %"` parses to 3.0; `"NaN"` is `None`; a nested child sensor keeps its parent's hardware id and type.
- [x] Test: `group_of` picks network before storage and gpu before cpu; `/psu/0` is psu whatever its text says.
- [x] Test: `cpu.temp` takes `CPU Package` before `Core Max`; a cooling node's `Liquid` also answers `cpu.temp`.
- [x] Commit.

### Task 2: LHM provider, descriptors and gap reasons

**Files:**
- Create: `crates/providers/src/lhm/provider.rs`
- Modify: `crates/providers/src/lib.rs`

**Interfaces:**
- Produces: `LhmProvider::new(base_url)`; `LHM_KEYS` list; descriptors tagged `extended`; `sample` reads `data.json` once per poll for every asked key.

- [x] Test: a server answering the fixture payload gives `cpu.temp` a number and `psu.power` a number.
- [x] Test: an unreachable server answers every key with `Status::Missing` and the "not reachable" sentence.
- [x] Test: only the union of asked keys is read; a key the mapping has that nobody asked is absent.
- [x] Commit.

### Task 3: Frame timing (RTSS shared memory, PresentMon fallback)

**Files:**
- Create: `crates/providers/src/frame/{mod.rs,rtss.rs,presentmon.rs,rolling.rs}`, `crates/rtss/{Cargo.toml,src/lib.rs,src/windows.rs}`
- Test: same files

**Interfaces:**
- Produces: `FrameProvider` answering `fps.current`, `frametime.current`, `fps.low1`, `frame.app`; `FrameWindow` over the last 60 s; `FrameSource`/`FrameEvent` in `crates/providers/src/frame/mod.rs`; `yunari-rtss::{decode, frames_since, newest_index}` and the one `unsafe` Win32 mapping read.

- [x] Test: the rolling window reports the 99th-percentile frame time from a scripted series.
- [x] Test: no RTSS mapping (RTSS not running) is `missing` with the reason, not zero.
- [x] Commit.

### Task 4: Weather

**Files:**
- Create: `crates/providers/src/weather/{mod.rs,open_meteo.rs,conditions.rs}`, `crates/host/assets/weather-icons/*.svg`
- Modify: `crates/host/src/{pages.rs,http.rs,server.rs}`

**Interfaces:**
- Produces: every key in `weather::KEYS`; `WeatherSettings::from_json` (host's `weather.json`, field-by-field defaults); `OpenMeteo::fetch(&Location, &Fetch) -> Result<Weather, String>`; `GET /api/weather/icon/{file}` serving only vendored slugs.
- Deferred: WeatherAPI and OpenWeatherMap backends (named in the reading's reason), Windows location fallback (named in the reason), German/Vietnamese condition tables (English only until the host has a language setting).

- [x] Test: the fixture answer maps `weather_code` 3 to "Overcast" and 61 to "Slight rain".
- [x] Test: a fetch that times out is `missing` with the timeout sentence.
- [x] Commit.

### Task 5: Media (SMTC via the source's own helper)

**Files:**
- Create: `crates/providers/src/media/{mod.rs,helper.rs,script.rs,artwork.rs}`
- Modify: `crates/host/src/{server.rs,pages.rs,http.rs}`
- Test: same files

**Interfaces:**
- Produces: keys `media.title`, `media.artist`, `media.album`, `media.artwork`, `media.state`, `media.position`, `media.duration`, `media.app`, `audio.volume`, `audio.muted`; the helper is the source's PowerShell script, run as one `-EncodedCommand` process; `media::artwork::Artwork` (content-addressed `put`/`read`, 64-file cache) and `GET /api/media/artwork/{file}` for a paired display or this PC.

- [x] Test: the helper's JSON maps to the six readings; a missing Windows session is `missing`.
- [x] Commit.

### Task 6: Custom HTTP sources

**Files:**
- Create: `crates/providers/src/custom/{mod.rs,pointer.rs,sources.rs}`
- Test: same files

**Interfaces:**
- Produces: `custom.<source>.<field>` keys from `sources.json` (`{id,name,url,intervalMs,fields:[{id,label,pointer,type,unit,scale,offset}]}`), validated field-by-field; `pointer::resolve` (RFC 6901, array indexes and `~0`/`~1` escapes); a 2 s timeout through `http::get`; the URL revalidated to `http`/`https` before any request.
- Deferred: credentials (`auth`), paging, per-source `headers`/`query` and `timeoutMs`; there is no credential store or settings UI yet, and a file written for the desktop app still loads (unknown keys are ignored).

- [x] Test: a pointer `/a/b/0` reads the first array element; a missing pointer is `missing` with the pointer's name.
- [x] Test: a `file://` URL is refused before any request.
- [x] Commit.

### Task 7: Game catalogue (Steam)

**Files:**
- Create: `crates/providers/src/game/{mod.rs,steam.rs,catalogue.rs}`
- Test: same files

**Interfaces:**
- Produces: `game.name`, `game.appid`, `game.started` from the foreground executable matched against `libraryfolders.vdf` + app manifests; box art is a library asset path, never a network fetch.

- [ ] Test: the manifest parse finds `appid` and `name` from a fixture `appmanifest_*.acf`.
- [ ] Test: an unknown foreground executable answers `missing`, not the last game.
- [ ] Commit.

### Task 8: Store client, MCP and SignalRGB bridge

**Files:**
- Create: `crates/host/src/store.rs`, `crates/host/src/mcp.rs`, `crates/host/src/signalrgb.rs`
- Modify: `crates/host/src/http.rs`, `crates/host/src/server.rs`

**Interfaces:**
- Produces: `GET /api/store/themes` (proxied, cached, loopback-only), `POST /api/mcp` (JSON-RPC, loopback-only, the same document surface the editor uses), and a `POST /api/signalrgb/keys` sink that maps SignalRGB channels onto `lighting.*` readings.

- [ ] Test: store responses are cached and a repeated request does not hit the network.
- [ ] Test: an MCP request from a phone is 403; from loopback it lists the same themes the library has.
- [ ] Commit.

### Task 9: Parity sweep against `product.md`

**Files:**
- Modify: `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

- [ ] Walk every `product.md` capability; record implemented / deferred / dropped with the reason.
- [ ] Commit.
