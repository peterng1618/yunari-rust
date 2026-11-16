# Phase 5 — USB and serial panels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A panel on the desk draws its assigned theme, at its own cadence, and survives being unplugged.

**Architecture:** A new `yunari-panels` crate holds one `PanelDevice` per family behind a transport seam: Lian Li (bulk USB + DES packets), NZXT Kraken Z3 (HID control + bulk data), TURZX (serial). The host's existing `frame`/`raster` path renders the display's assigned theme into panel geometry, the runner coalesces frames per device, and the manager reconciles panel display records every 15 s so a replug reopens on its own.

**Tech Stack:** rusb 0.9 (bulk), hidapi 2 (Kraken control), serialport 4 (TURZX), des + cbc (Lian Li packet crypto), image (JPEG/PNG encode), tiny-skia via `yunari-raster` (pixels), tokio (host side).

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- Every refusal string, opcode, packet layout, timing constant and limit below is copied from the source survey; keep them verbatim. Source files to copy from: `D:\git-repos\yunari\src\web\packages\host\src\panels\` (`panel-protocol.ts`, `tur-usb-panel.ts`, `turzx-protocol.ts`, `turzx-panel.ts`, `kraken-z3-panel.ts`, `panel-runner.ts`, `panel-manager.ts`, `panel-families.ts`).
- Families and geometry: `lianli-9-2` `0x1cbe:0xa092` 464×1920; `lianli-8-8` `0x1cbe:0xa088` 480×1920; TURZX `0x1d6b:0x0106` 800×480; Kraken `0x1e71:0x3008` 320×320.
- Cadence: runner default 1000 ms; Kraken 140 ms (RGBA mode). Three consecutive capture/send failures mark the device lost.
- Panel stills come from the same `DisplayList` the phone gets: `plan::build_with` → `yunari-raster` → fit and turn into panel geometry → encode.
- No hardware-in-the-loop test is available: every driver test runs against a fake transport that records bytes, mirroring the source's fake devices.
- Only stills in this phase. Panel video (H.264 blocks, MP4 upload, overlays) and the SignalRGB bridge are deferred and recorded as such.
- Rust workspace gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt`.

## Review Focus

- A device unplugged mid-frame must not wedge the manager: three failures detach it, the record stays, and the reconcile pass reopens it when it returns.
- A panel that rejects a write must not be marked connected: an error is reported once per loss, not once per tick.
- A frame that has not changed must not be re-sent byte for byte, so an idle panel costs no USB traffic.
- The fit and turn must never crop a portrait artboard into a landscape panel silently: contain by default, and quarter-turn validation per family shape.
- A panel display must never be reachable as a paired display: the command and preview routes stay loopback-only.

---

### Task 1: Family registry and panel geometry

**Files:**
- Create: `crates/panels/src/family.rs`, `crates/panels/src/device.rs`
- Modify: `crates/panels/src/lib.rs`, `crates/panels/Cargo.toml`

**Interfaces:**
- Produces: `Geometry { width: u32, height: u32 }`; `PanelSupport { maker, hardware_revision, supported_firmware, updated_at }`; `PanelCommand::{Brightness(u8), Screen(bool), PanelSettings{start_mode,flip,sleep_delay}, Restart, Status, Hello}`; `PanelFamily { id, label, vendor_id, product_id, support, commands, geometry, shape }`; `families() -> &'static [PanelFamily]`; `for_device(vendor_id, product_id) -> Option<&'static PanelFamily>`; `attached() -> Vec<PanelHint { vendor_id, product_id, serial, label, model, device_id }>`; `PanelDevice` trait with `geometry()`, `open()`, `present(&[u8])`, `still_format()`, `controls()`, `close()`.

- [ ] Tests: ids are unique; each family's geometry and ids match the table above; `for_device` returns the first match; `attached()` for a family whose enumeration throws is skipped without emptying the list.
- [ ] Commit.

### Task 2: Fitting and turning artboard pixels into panel pixels

**Files:**
- Modify: `crates/raster/src/lib.rs`
- Test: same file's test module

**Interfaces:**
- Produces: `pub fn fit_into(list:&DisplayList, width:u32, height:u32, rotation:u16, round:bool, fonts: Option<&mut Fonts>, images: Option<&mut Images>) -> Raster` — renders the list, contains it in a width×height box of transparent pixels, then turns by `rotation` degrees; a rectangular box accepts only 0/90/180/270 and a round one any integer 0..=359.

- [ ] Tests: a 480×120 artboard in a 320×320 box is centred with margins top and bottom, not cropped; a 90° turn swaps the occupied edges; a rectangular box refuses 45°; a round box accepts 45°.
- [ ] Commit.

### Task 3: Lian Li packet assembly

**Files:**
- Create: `crates/panels/src/tur_usb/protocol.rs`, `crates/panels/src/tur_usb/mod.rs`
- Test: `protocol.rs` tests

**Interfaces:**
- Produces: `COMMAND_SIZE=500`, `PACKET_SIZE=512`, `PACKET_KEY=b"slv3tuzx"`, `MAGIC=[0x1a,0x6d]`, `TRAILER=[0xa1,0x1a]`; `opcode` module (`VERSION=10`, `BRIGHTNESS=14`, `FRAME_RATE=15`, `ROTATE=13`, `PUSH_JPEG=101`, `PUSH_PNG=102`, `PLAY_BLOCK=121`, `OVERLAY_LAYER=105`, `STOP_PLAY=123`, `BUFFER_LEVEL=122`); `command(opcode:u8, payload:Option<&[u8]>, parameter:Option<u8>) -> Vec<u8>` producing the encrypted 512-byte frame with the payload appended; `parameter_command(opcode, byte)`.

- [ ] Tests: byte 0 is the opcode, bytes 2–3 the magic, bytes 4–7 the little-endian timestamp, bytes 8–11 the big-endian payload length for a payload command and byte 8 the parameter otherwise; the frame is exactly 512 bytes ending in the trailer; the payload follows the frame unencrypted and in the same vector; the same call twice differs only in the timestamp.
- [ ] Commit.

### Task 4: Lian Li device

**Files:**
- Create: `crates/panels/src/tur_usb/device.rs`
- Test: same file, fake `Bulk` transport

**Interfaces:**
- Produces: `trait Bulk { fn write(&mut self, bytes:&[u8]) -> Result<(),String>; fn read(&mut self, len:usize, timeout_ms:u64) -> Result<Vec<u8>,String>; fn clear_halt(&mut self) -> Result<(),String> }`; `TurUsbPanel<B:Bulk>` implementing `PanelDevice`; constants `MAX_PAYLOAD=512_000`, `READ_TIMEOUT_MS=2_000`, `WRITE_TIMEOUT_MS=5_000`, `BLOCK_WRITE_TIMEOUT_MS=30_000`, `ROTATE_MODE=2`, `BLOCK_PAUSE_MS=30`, `DRAIN_DEPTH=2`, `ACK_DEPTH=3`, `DRAIN_POLL_MS=50`, `DRAIN_WAIT_MS=30_000`.

- [ ] Tests: `open` sends version, frame-rate 30, rotate 2 and the transparent overlay in that order; a still sends stop-play before every JPEG; a frame over 512000 bytes is refused with `A {n}-byte frame exceeds the panel's 512000-byte limit.`; a reply depth over 3 polls buffer-level until 2; a failed write retries once after clearing both halts; brightness sends the percent and screen-off sends 0 while screen-on restores the last level; status returns the firmware string and `The panel did not answer its firmware query.` when it does not.
- [ ] Commit.

### Task 5: Kraken control and buckets

**Files:**
- Create: `crates/panels/src/kraken/mod.rs`, `crates/panels/src/kraken/control.rs`
- Test: same files, fake `Hid` and `Bulk`

**Interfaces:**
- Produces: `trait Hid { fn write(&mut self, report:&[u8]) -> Result<(),String>; fn read(&mut self, timeout_ms:u64) -> Result<Vec<u8>,String> }`; `KrakenZ3<H:Hid,B:Bulk>` with `SLOT_BYTES=1024`, `BUCKET_COUNT=16`, `RGBA_MODE=0x02`, `PALETTED_MODE=0x03`, `HEADER_MAGIC`, `KRAKEN_FRAME_INTERVAL_MS=140`; reports exactly 64 bytes.

- [ ] Tests: `10 01` answered `11 01` reads the firmware and `30 01` the brightness and orientation; a report of 65 bytes is refused; a frame deletes, creates, begins, writes header then payload as two bulk writes, ends and switches; a refused delete never writes the bucket; status 9 retries into the next bucket and a second failure rebuilds the table; status 2 ends the open transfer; brightness carries the orientation read from `31 01`; status answers `Firmware {a}.{b}.{c}, LCD brightness {n}%, orientation {o*90}°`.
- [ ] Commit.

### Task 6: TURZX serial

**Files:**
- Create: `crates/panels/src/turzx/mod.rs`, `crates/panels/src/turzx/protocol.rs`
- Test: same files, fake serial transport

**Interfaces:**
- Produces: `trait Serial { fn write(&mut self, bytes:&[u8]) -> Result<(),String>; fn read(&mut self, timeout_ms:u64) -> Result<Vec<u8>,String> }`; `TurzxPanel<S:Serial>`; `FRAME_SIZE=250`, `PAYLOAD_CHUNK=249`; `parse_hello(text) -> Option<{model, device}>` matching `^chs_([a-z0-9_]+)\.(dev\d+)_rom([0-9.]+)$`; the command table from `turzx-protocol.ts`.

- [ ] Tests: every write is a 250-byte multiple; a bitmap prefixed `PRE_UPDATE`, `START_DISPLAY_BITMAP`, `DISPLAY_BITMAP_5INCH`; a full 800×480 frame produces 1,542,250 bytes; BGRA swaps red and blue; HELLO parses `chs_5inch.dev1_rom1.88` and refuses anything else; brightness sends `7b ef 69 …` with `round(level/100*255)`; status returns the reply carrying `needReSend` or `renderCnt`; a panel that answers HELLO but not the status is reported `is attached but did not answer: … Unplug and replug it, or restart its USB device.`.
- [ ] Commit.

### Task 7: Runner

**Files:**
- Create: `crates/panels/src/runner.rs`

**Interfaces:**
- Produces: `Runner<D: PanelDevice>` with `capture: Box<dyn Fn(u32,u32) -> Result<Vec<u8>,String>>`, `interval_ms`, `run()` doing one tick, `stop()`; `FAILURES_BEFORE_LOST=3`; a tick skips while one is in flight and runs one follow-up after it; a byte-identical frame is not sent.

- [ ] Tests: an unchanged capture sends one frame and a changed one sends a second; three capture errors mark the runner not running and report lost once; a tick during an in-flight tick queues exactly one follow-up; a stop after a failure still closes the device.
- [ ] Commit.

### Task 8: Manager and panel display records

**Files:**
- Create: `crates/panels/src/manager.rs`
- Modify: `crates/host/src/displays.rs` (add `device: Option<PanelDeviceId>` to `DisplayRecord` and `DisplayStore::adopt_panel`)

**Interfaces:**
- Produces: `PanelDeviceId { vendor_id: u16, product_id: u16, serial: String, device_id: Option<String> }`; `Manager` with `start(display, device, frame_source)`, `stop(display)`, `command(display, PanelCommand)`, `status(display) -> PanelConnectionStatus`, `reconcile(displays)`, `disconnect(display)`, `reconnect(display)`, `stop_all()`; `PanelManagerError::{NotRunning(String), NoCommands(String), NoDriver(u16,u16), NoDevice}` with the source's sentences.

- [ ] Tests: `reconcile` starts every panel record with no runner and stops a runner whose record vanished; a lost runner is detached and reported `disconnected; retrying.`; a command on a stopped display refuses with `Panel display {id} is not running; start it before sending a command.`; a device without controls refuses with `… does not accept commands.`; `restart` always detaches and reports `restarting; reconnecting when the panel returns.`; two records naming one device refuse with `A panel display already holds that device.`
- [ ] Commit.

### Task 9: Host routes for panels

**Files:**
- Modify: `crates/host/src/http.rs`, `crates/host/src/server.rs`, `crates/host/src/lib.rs`
- Create: `crates/host/src/panels_routes.rs`

**Interfaces:**
- Produces: `GET /api/displays/panels` → `{families:[{id,label,vendorId,productId,width,height,commands,support}], attached:[…]}`; `POST /api/displays` with `{kind:"panel", label, device:{vendorId,productId,serial}}` creating a panel record; `POST /api/displays/:id/panel/command` `{kind, …}` → `200 {"ok":true,"reply":…}`, 409 on a port failure, 500 otherwise; `POST /api/displays/:id/panel/reset-device` (Windows only, `pnputil /restart-device` with the parent revalidated against `^USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}(?:&MI_[0-9A-F]{2})?\\[A-Za-z0-9_&+.-]{1,64}$`); `GET /api/themes/:id/panel-preview?display=` → PNG, `nosniff`, `no-store`.

- [ ] Tests: a panel POST with an unknown vendor/product refuses `No driver on this host claims …`; a phone request to any panel route is 403; preview refuses a display that is not a panel with `That display is not a panel.`; a command to a stopped display is 409 with the runner sentence.
- [ ] Commit.

### Task 10: Frames from the assigned theme, and lifecycle

**Files:**
- Modify: `crates/host/src/server.rs`, `crates/host/src/stream.rs` (share the resolved-document cache), `crates/app/src/main.rs`

**Interfaces:**
- Produces: a `frame_source` closure in the host that resolves the display's theme (`Assignment::resolve`), reads the document, builds `DisplayList` with the display's keys, calls `fit_into` at the family geometry with the display's rotation, and encodes for the device (`still_format`); the manager starts after the listener binds and `stop_all` runs on shutdown.

- [ ] Unit test: the closure for a display assigned the fixture theme with a 320×320 geometry returns PNG bytes whose decoded size is 320×320; an unassigned display returns the empty-document plate rather than an error.
- [ ] Commit.

## Header note: deferred in this phase

- Panel video: Lian Li H.264 blocks, TURZX MP4 upload and file cache, overlays and clip-mode pages.
- The Chromium page rasteriser is not ported: frames come from the same raster path the golden tests use, so a panel shows the theme, not a browser's idea of it.
- SignalRGB bridge, device telemetry (`liquidTempC`, `pumpRpm`, `fanRpm`) and lighting channels move to phase 6 with the provider work.
- Kraken paletted mode B, Lian Li `play-block` pacing and TURZX overlay diffs are recorded as unimplemented until video lands.
