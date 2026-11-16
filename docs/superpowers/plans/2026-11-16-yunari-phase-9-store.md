# Store client and account — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A person can browse the configured theme store, sign in with a GitHub account, install a catalogue item (dashboard theme, widget pack, editor theme), submit a library theme, and see their own submissions — all from the native app, with the session token never reaching the browser.

**Architecture:** A new `yunari-store` crate owns the settings-dir session file, the Worker HTTP client and the version ladder. The host gains loopback-only routes under `/api/store/*` and `/api/themes/submit`, each guarded by the same loopback + origin checks pairing uses. The editor gains a Store mode that reads the catalogue proxy and posts only ids and catalogue facts, never bytes.

**Tech Stack:** Rust 2024, axum/rustls host, `ureq` for Worker calls, `sha2`, `zip`/`yunari-packs`, `yunari-library`, `yunari-render-raster` + `image` for thumbnails, GPUI editor.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md` (rows 12, 13, 30, 43).

## Global Constraints

- The browser or editor never holds the session token; it appears only in the `Authorization` header of Worker calls.
- Download refuses a non-http(s) address, a body over the cap, a size or SHA-256 that differs from the catalogue's declaration, and never follows a redirect.
- Nothing is written before every gate passes. A refused origin write removes a fresh theme copy.
- Every route that acts as the author's account is loopback + same-origin only; a foreign origin is 403.
- Path-preset packs and plugins are not installable in this build (their stores are dropped); the route answers the same "this host keeps no …" refusal the source does without those stores.
- `cargo fmt --all`, `clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` green per commit.
- Commits end with `Co-Authored-By: Claude Code <noreply@anthropic.com>`.

## Review Focus

- A Worker answer that is not JSON, empty, or a 401 must become one sentence and, for 401, forget the stored session.
- An archive that is not its declared kind must not partially write: no half theme folder, no stale origin file.
- A second install of the same catalogue item updates the copy in place rather than creating a sibling.
- The sign-in return URL must only redeem a state this host minted for that same loopback origin.
- A submission with no change to the envelope is `none`, not a forced patch.

---

### Task 1: `yunari-store` — session file and Worker client

**Files:**
- Create: `crates/store/Cargo.toml`, `crates/store/src/lib.rs`, `crates/store/src/session.rs`, `crates/store/src/client.rs`
- Modify: `Cargo.toml` (workspace members + dependency)

**Interfaces:**
- Produces:
  - `Session { token: String, email: String, user_id: String, obtained_at: String }`
  - `SessionStore::at(settings: &Path)`, `.read() -> Option<Session>`, `.write(&Session) -> Result<(), String>`, `.clear()`
  - `Request { method: Method, url: String, headers: Vec<(String,String)>, body: Option<Vec<u8>> }` with `Method::{Get, Post, Put, Delete}`
  - `Reply { status: u16, headers: Vec<(String,String)>, body: Vec<u8> }`
  - `Send = Arc<dyn Fn(&Request) -> Result<Reply, String> + Send + Sync>` (test seam) and `pub fn send(request: &Request) -> Result<Reply, String>` (ureq, 5 s, 32 MiB cap, no redirects)
  - `Client::new(origin, token, Send)`, methods `baseline(id) -> Result<Option<Baseline>, ClientError>`, `create_version(id, &CreateInput) -> Result<Plan, ClientError>`, `upload(url, &[u8])`, `finalize(id, version, thumbnail: &[u8], luminance: Option<f64>)`, `unpublish(id)`, `publish(id)`, `mine()`, `favorite(id, bool)`
  - `ClientError { status: u16, message: String }`

- [ ] Tests: session survives a write/read round trip, a corrupt file is `None`, clear removes it; a 401 body `{"message":"…"}` becomes that sentence, an empty 404 baseline is `None`, a non-JSON 2xx is an error.
- [ ] Implement both modules; commit.

### Task 2: Host session routes

**Files:**
- Create: `crates/host/src/store_routes.rs`
- Modify: `crates/host/src/server.rs` (state fields, router), `crates/host/src/lib.rs`

**Interfaces:**
- Consumes: `yunari_store::session::{Session, SessionStore}`; existing `crate::http::{is_loopback, host_of}`.
- Produces: `POST /api/store/auth/start` → `{url}`; `GET /api/store/auth/return?state&token` → HTML sentence; `GET /api/store/session` → `{signedIn,email?}`; `POST /api/store/auth/sign-out` → `{ok:true}`; `GET /api/store/mine`; `POST /api/store/items/:id/publish|unpublish`.
- `PendingStates` in memory: 10-minute TTL, 64 entries, one use each, bound to the loopback origin.

- [ ] Tests: a state minted for `127.0.0.1:5229` is refused for another origin and on replay; signed-out `session` and `mine` answer 401 with the source's sentence; a 401 from the Worker clears the stored token.
- [ ] Implement; commit.

### Task 3: Theme origins and install

**Files:**
- Create: `crates/host/src/origins.rs`, `crates/host/src/store_install.rs`
- Modify: `crates/host/src/server.rs`, `crates/host/src/lib.rs`

**Interfaces:**
- `Origins::at(settings)`; `read() -> BTreeMap<String, Origin>`; `write(theme_id, Origin)`; `remove(theme_id)`.
- `pub fn install(app: &App, request: &InstallRequest) -> Result<Installed, Refusal>` with `InstallRequest { package_url, catalogue_id, kind, version, sha256, bytes }`, `Refusal { stage, message }`, `stage ∈ address|download|integrity|package|store`.
- Kind handling: `dashboard-theme` → read with `yunari_library` package reader, save under a library id derived from the catalogue id, then write the origin (rollback the fresh copy on a failed origin write); `widget-pack` → `yunari_packs::install`; `editor-theme` → `<settings>/editor-themes/<id>.yunari-theme` after `yunari_library::package` accepts it; `path-preset-pack`/`plugin` → `stage: "store"` refusal.
- Route `POST /api/store/install` (loopback only) answering `{ok,id}` or `{ok:false,stage,message}` with 400/502 by stage.

- [ ] Tests: a good widget pack installs; a wrong digest writes nothing; an unknown kind is refused; reinstall of the same catalogue id reuses its library id; a foreign origin cannot reach the route.
- [ ] Implement; commit.

### Task 4: Submission

**Files:**
- Create: `crates/host/src/submit.rs`, `crates/host/src/thumbnail.rs`
- Modify: `crates/host/src/server.rs`

**Interfaces:**
- `pub fn version_plan(previous: Option<&str>, next: &str) -> VersionLevel` where levels are `initial|none|patch|minor|major`: no previous → `initial`; same envelope digest → `none`; otherwise compare semver parts.
- `pub fn thumbnail(document: &ThemeDocument, assets: &BTreeMap<String, Vec<u8>>) -> Result<(Vec<u8>, f64), String>` — render at 640 wide with `yunari-render-raster`, encode PNG with `image`, mean luminance 0–1.
- `POST /api/themes/submit` body `{id}`: read the library theme, build the package with the library's own writer, `baseline` → choose the level → `create_version` → `upload` → `finalize`; answer `{ok:true,status,version:{from,to,level}}`.

- [ ] Tests: ladder cases; an unchanged envelope is `none`; a missing id is 400; no session is 401; an upload 401 forgets the session; the thumbnail is a valid PNG of the artboard's aspect.
- [ ] Implement; commit.

### Task 5: Editor Store mode

**Files:**
- Create: `crates/editor/src/ui/store_mode.rs`
- Modify: `crates/editor/src/ui/menus.rs` (Store command), `crates/editor/src/ui.rs` (mode switch, overlay), `crates/editor/src/lib.rs`

**Interfaces:**
- Consumes: `/api/store/session`, the catalogue proxy `GET /api/store/themes`, `/api/store/install`, `/api/store/mine`, `/api/themes/submit`, `/api/store/auth/*`; the host's loopback origin from `host.lock`/bound port.
- Produces: a Store mode with the catalogue list (name, kind, version, publisher, install button), a Mine list (version, status, publish/unpublish, Submit on a library theme), sign-in state with email and a Sign out action; install and submit answer with the host's own sentence.

- [ ] Tests: catalogue rows render from a fixture listing; an install press posts the item's facts and no bytes; a signed-out Mine state offers Sign in; a submit answer shows the level and version the host reported.
- [ ] Implement; commit.

### Task 6: Spec rows

- [ ] Update rows 12, 13, 30, 43, 49 honestly; note the dropped kinds and the unported catalogue-index generator.
- [ ] Commit.
