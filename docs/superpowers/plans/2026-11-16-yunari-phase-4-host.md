# Phase 4 — Host, pairing and the browser display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** This PC serves themes to paired phones over TLS, streams readings as SSE, and drives local monitors fullscreen.

**Architecture:** `yunari-host` is a library the app starts on a worker thread: an axum router over one port that sniffs each connection's first byte to choose TLS, backed by JSON files in `%USERPROFILE%\.yunari\settings` and themes in the existing `yunari-library`. A phone loads a small embedded page that pairs with a code, opens an SSE stream, and paints the host's own `DisplayList` (serialized as JSON) on a canvas — one renderer, no second implementation.

**Tech Stack:** axum 0.8, hyper 1 + hyper-util (per-connection serving), tokio-rustls + rcgen (local CA), tokio, serde_json, yunari-library, yunari-providers, yunari-render.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- Host binds `127.0.0.1:5227` by default; ports attempt `5227..=5236`; LAN opt-in serves `0.0.0.0`.
- Every refusal string, limit and file name below is copied from the source survey; keep them verbatim.
- Session token: 32 random bytes base64url; stored only as SHA-256 base64url. Cookie `yunari_session`, `Max-Age=34560000; HttpOnly; SameSite=Strict; Path=/` plus `; Secure` when TLS.
- Pairing code: 6 chars from `ABCDEFGHJKMNPQRSTUVWXYZ23456789`, TTL 600 s, one live code, per-address miss penalty `min(250 * 2^(missed-1), 4000)` ms.
- `/api/*` except `/api/host` refuses a loopback request whose `Origin` is foreign.
- `/yunari-ca.crt` is served before any auth gate.
- Theme rendering reuses `yunari_render::plan::build_with`; the browser never interprets the theme document.
- Rust workspace gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt`.

## Review Focus

- A phone that stops mid-stream must not leak a connection slot; the 64/8 caps count live streams only.
- A rotating or deleted display must stop streaming immediately (401), not linger until the phone reconnects.
- Loopback admin must stay unreachable from a phone: every `/api/displays*`, theme write and pairing-code route checks loopback, not just auth.
- Frame JSON must stay bounded: 64 max keys, one frame per tick, no unbounded history per connection.
- A missing sensor prints the product's missing message with the `host:<key>` sensor id, never a zero.

---

### Task 1: Config, host identity and the lock

**Files:**
- Create: `crates/host/src/config.rs`, `crates/host/src/identity.rs`, `crates/host/src/lock.rs`
- Modify: `crates/host/src/lib.rs`, `crates/host/Cargo.toml`

**Interfaces:**
- Produces: `HostConfig { app_dir: PathBuf, port: u16, lan: bool }`, `HostConfig::resolve(app_dir: Option<PathBuf>, port: Option<u16>, lan: bool) -> HostConfig` with `settings_dir()`, `themes_dir()`; `identity::load_or_mint(settings_dir) -> HostIdentity { id: String, name: String }` (16 hex chars, name ≤64 from `COMPUTERNAME`); `lock::acquire(settings_dir, port) -> Result<HostLock, HeldError>` where `HeldError { pid: u32, port: u16 }`; `HostLock::release(&self)`.
- `host::serve(config) -> Result<(), HostError>` placeholder returning once Task 4 lands.

- [ ] Write tests: app dir defaults to `%USERPROFILE%\.yunari`, settings/themes subdirs; identity file round-trips and a second load returns the same id; a lock file with a live pid (this process) refuses with that pid and port; a stale pid is replaced.
- [ ] Implement file IO through a `write_json_atomic` helper (temp file + rename in the same directory).
- [ ] Commit.

### Task 2: Display store

**Files:**
- Create: `crates/host/src/displays.rs`

**Interfaces:**
- Produces: `DisplayRecord { id, label, kind: DisplayKind, created_at, last_seen_at, token_hash: Option<String>, shape: Option<Shape>, rotation: Option<u16> }`; `DisplayKind { Paired, Local, Panel }`; `DisplayStore::load(dir) -> DisplayStore`, `save()`, `list()`, `get(id)`, `mint(label) -> (DisplayRecord, String token)`, `adopt(id,label,shape,rotation) -> (DisplayRecord, String)` (paired, token minted), `authenticate(token) -> Option<String>` (SHA-256 base64url lookup, touching `last_seen_at` at most every 5 s), `rotate(id) -> Option<String>`, `remove(id)`, `default_record()`.
- `DEFAULT_DISPLAY = "default"`, id regex `^[a-z0-9][a-z0-9-]{0,63}$`, label ≤64, labels cleaned with the source's rules (id from label, collision suffix 2 random hex bytes).

- [ ] Tests: a minted token authenticates and is not stored in clear; rotating invalidates the old token; `default` cannot be removed; ids stay within 64 chars and reject `default`/`this-pc`; a JSON round-trip preserves kind and shape.
- [ ] Commit.

### Task 3: TLS authority

**Files:**
- Create: `crates/host/src/tls.rs`

**Interfaces:**
- Produces: `Authority::load(settings_dir) -> Result<Authority, TlsError>`, `Authority::server_config(&self) -> Arc<rustls::ServerConfig>`, `Authority::certificate_pem(&self) -> String`, `fingerprint_hex(&self) -> String`, `local_names() -> Vec<String>`.
- CA: ECDSA P-256 (source used RSA-2048; note the deviation), CN `Yunari Local CA`, O `Yunari`, 3650 days, persisted as `tls/ca.crt` + `tls/ca-key.bin`; leaf 397 days, SANs from `local_names()`, generated in memory each start.
- `local_names`: `localhost`, hostname, `<hostname>.local`, every non-internal interface address, `127.0.0.1`, `::1`.

- [ ] Tests: two loads reuse the CA certificate bytes; leaf SANs include `localhost` and `127.0.0.1`; fingerprint is 64 lowercase hex chars; a corrupt `ca.crt` returns an error rather than panicking.
- [ ] Commit.

### Task 4: Same-port listener and router skeleton

**Files:**
- Create: `crates/host/src/http.rs`, `crates/host/src/server.rs`
- Modify: `crates/host/src/lib.rs`, `crates/host/Cargo.toml` (add `hyper`, `hyper-util`, `http-body-util`, `tokio-rustls`, `bytes`)

**Interfaces:**
- Produces: `server::start(config) -> Result<Server, HostError>` where `Server { addr: SocketAddr, local_url: String, certificate: Option<String>, shutdown: tokio::sync::oneshot::Sender<()> }`; `http::router(state) -> axum::Router`; `http::AppState { settings_dir, config, displays: Mutex<DisplayStore>, identity, authority, library, pairing, sampler, ... }`.
- Accept loop: `TcpListener`, 5 s read timeout for one byte, `0x16` → `TlsAcceptor` then `hyper_util::server::conn::auto::Builder::serve_connection`, else plain stream; cap 256 live connections; remove `Server: axum` header noise only if trivial.
- `GET /api/health` → `{"ok":true}`; `GET /api/host` → `{"id","name","port"}` with `Access-Control-Allow-Origin: *`, token only when authenticated.

- [ ] Tests: health answers 200; `/api/host` carries the identity and never a token unauthenticated; router refuses `/api/displays` with 403 from a non-loopback `ConnectInfo` and foreign `Origin`.
- [ ] Integration check: start on an ephemeral port, `curl` plain HTTP health, then `curl -k` HTTPS health through TLS on the same port.
- [ ] Commit.

### Task 5: Pairing

**Files:**
- Create: `crates/host/src/pairing.rs`, `crates/host/src/pairing_routes.rs`
- Modify: `crates/host/src/http.rs`

**Interfaces:**
- Produces: `Pairing::mint() -> (String code, SystemTime expires_at)`, `Pairing::current()`, `Pairing::claim(code, remote) -> Result<(DisplayRecord, String), ClaimError>` applying the penalty asynchronously before re-reading; `session_cookie(token, secure) -> String`; `display_id_from_request(headers) -> Option<String>`.
- Routes: `GET /pair` (embedded page), `POST /api/session` `{token}` → 302 `/` + cookie, `POST /api/pairing/claim` `{code,label,shape?,rotation?}` → `{displayId, token}`, `GET|POST /api/pairing/code` → `{code, expiresAt}`, `GET|POST /api/pairing/sessions` → `{displays}`, rotation via `POST /api/pairing/sessions?displayId=`.
- Refusals verbatim: `"That pairing code is not the one this host is showing."`, `"Displays are available on this PC only."`.

- [ ] Tests: a claim with the live code yields a token that authenticates; an expired code refuses; wrong codes accrue penalty per remote and do not reset the live code; a code is spent after one successful claim; shape/rotation validation (`rect` 0/90/180/270, `circle` 0–359).
- [ ] `curl` check: minted code from loopback, claim it, use the returned cookie on `/api/host`.
- [ ] Commit.

### Task 6: Sensors and the SSE stream

**Files:**
- Create: `crates/host/src/stream.rs`, `crates/host/src/frame.rs`
- Modify: `crates/host/src/http.rs`

**Interfaces:**
- Produces: `stream::registry() -> Registry` with `ClockProvider` + `SystemProvider`; `GET /api/sensors` (loopback-only measured flag) → catalogue; `GET /ws?keys=&lang=` → `text/event-stream`, `": connected\n\n"`, one `event: samples` frame per second with `{"version":1,"sentAt":<ms>,"samples":[{semanticKey,sample}],"frame":<DisplayList JSON|null>}`.
- `frame::render(document, readings, now) -> DisplayList` via `build_with`; serialization behind a `serde` feature on `yunari-render` or a `serde_json::Value` conversion in `frame.rs` (prefer adding `#[derive(Serialize)]` under a `serde` feature to `DisplayList`, `Op`, `Shape`, `Paint`, `Stroke`, `PathCommand`, `Affine`, `Color`, `TextLayout`, `TextSegment`, `TextStyle`, `GradientStop`, `Backdrop`, `Geometry`).
- Limits: 64 keys, 128 bytes per key, 64 streams, 8 per display, queue 120 dropping oldest; refusals verbatim from the survey.

- [ ] Tests: 65 keys → 400 with the exact message; a 129-byte key → 400; the 9th stream for one display → 429; a missing key renders `status:"missing"` with `sensorId "host:<key>"`; a document with one value run produces `frame` containing that run's formatted text; a second tick drops the oldest frame when the queue is full (unit test the queue).
- [ ] Commit.

### Task 7: Theme and assignment API

**Files:**
- Create: `crates/host/src/themes_routes.rs`, `crates/host/src/assignment.rs`
- Modify: `crates/host/src/http.rs`

**Interfaces:**
- Produces: `Assignment::load(settings_dir)`, `get(display) -> Option<String>`, `set(display, id)`, `resolve(display, requested) -> Option<String>` (explicit → assignment → sole library theme); fixes the legacy `{ "id": ... }` shape and the retired `this-pc` key.
- Routes: `GET /api/themes` (paired: `{themes:[{id,name}]}`; loopback adds entries + `origin`), `GET /api/themes/:id/document` (envelope JSON, 404 `"Theme not found."`), `GET /api/themes/:id/assets/<path>` (nosniff, CSP `sandbox; default-src 'none'`, no-store), `GET|PUT /api/themes/active` (loopback), `GET /api/published` (paired or loopback) → `{id,revision,display:{theme,shape,rotation}}`, `GET /` unpaired navigation → 302, `GET /?theme=` → player page.
- Display guard: remote, unauthenticated, bundle-asset allowlist (`/assets/`, `/favicon.svg`, `/manifest.webmanifest`, `/sw.js`, `/pair`, `/yunari-ca.crt`) → navigation 302 `/pair?reason=unpaired`, else 403 `"This display is not paired with the host."`.

- [ ] Tests: assignment round-trips and migrates `{"id":...}`; an unknown theme id resolves to `None`; `/api/published` refuses unauthenticated remote; bundled asset path traversal (`../`) is refused.
- [ ] Commit.

### Task 8: The phone page

**Files:**
- Create: `crates/host/assets/pair.html`, `crates/host/assets/player.html`, `crates/host/assets/player.js`, `crates/host/assets/rotation.js`
- Modify: `crates/host/src/pairing_routes.rs`, `crates/host/src/themes_routes.rs` (serve them with `include_str!`)

**Interfaces:**
- Pair page: code, name (≤64), shape select, rotation input; POST claim, POST session, store nothing, `location.replace("/")`.
- Player: cookie auth, `new EventSource("/ws?keys=<bound keys>&lang=<lang>")`, canvas sized to `frame.width/height`, paints every op (rect, ellipse, line, path, text, image via `/api/themes/<id>/assets/<path>`, glass approximated by the wrapped shape, image fit), reconnect status text, wake lock, fullscreen on tap; refetch the document when `/api/published` reports a new revision (poll every 2 s).
- The player names the version mismatch message verbatim.

- [ ] Tests: page strings are embedded and served with the right content types; the JS paint function is exercised in a headless browser (Playwright MCP) against a running host with the fixture theme.
- [ ] Commit.

### Task 9: Local displays fullscreen

**Files:**
- Modify: `crates/player/src/lib.rs`, `crates/editor/src/ui.rs` (Host pane: pairing code, displays, "Open on monitor")
- Create: `crates/host/src/local.rs`

**Interfaces:**
- Produces: `player::open_on_monitor(document: ThemeDocument, monitor_uuid: Option<Uuid>, cx: &mut App)` opening a `WindowBounds::Fullscreen` window sized to the chosen `PlatformDisplay`'s bounds; `local::monitors(cx) -> Vec<MonitorInfo { uuid, label, bounds }>`.
- A `local` display record stores the monitor uuid; the Host pane lists records with kind, label, shape/rotation, last seen, and per row: assign theme, open, remove.

- [ ] Tests: the Host pane lists a fixture store and the code refreshes on command (unit test state, not pixels).
- [ ] Manual check: open the fixture fullscreen on the primary monitor with `Escape` or `Ctrl+Q` closing it.
- [ ] Commit.

### Task 10: Lifecycle, autostart and the exit check

**Files:**
- Create: `crates/host/src/autostart.rs`
- Modify: `crates/app/src/main.rs`, `crates/host/src/lib.rs`

**Interfaces:**
- Produces: `Host::start_in_process(app_dir)` from the app: tokio runtime thread, lock refusal printed and the app continues in editor-only mode; `Host::stop()` closes streams and releases the lock; autostart writes `<APPDATA>\Microsoft\Windows\Start Menu\Programs\Startup\Yunari.vbs` (UTF-16LE with BOM) and reports the source's two messages; `GET|POST /api/displays/autostart` → `{enabled}`.
- LAN toggle `GET|PUT /api/hosting` → `{"lan":bool}`, restarting the listener when it changes.

- [ ] Tests: autostart enable/disable round-trips against a temp `APPDATA`; the lock is released on drop; the hosting file round-trips.
- [ ] Exit check: pair from a second browser profile over HTTPS, see the fixture render and readings update; open a local display fullscreen; confirm a phone request to `/api/displays` is 403 and `/api/host` works; `cargo test --workspace` and clippy clean.
- [ ] Commit.
