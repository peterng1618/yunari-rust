# Running and uninstalling Yunari

Yunari is a Windows desktop app: an editor, a display host and a USB-panel
driver in one process. There is no setup program — it runs from its own
executable, and `scripts/package-windows.ps1` stages that executable, the MCP
bridge and a README as one zip.

## Package a release

```powershell
pwsh -File scripts/package-windows.ps1
```

It builds `Yunari.exe` and `yunari-mcp.exe`, stages them under
`dist/yunari-<version>-windows-x64/`, writes the zip beside it and prints the
archive's SHA-256. Copy that folder anywhere; nothing is installed system-wide
and nothing is written outside `%USERPROFILE%\.yunari`.

## Run it

1. Build it:

   ```sh
   cargo build -p yunari-app          # debug
   cargo build --release -p yunari-app # optimized
   ```

   The optimized build needs the Windows SDK's `fxc.exe` on `PATH` (or
   `GPUI_FXC_PATH`), because GPUI compiles its HLSL shaders only in a release
   build. Running the app does not need the SDK.

2. Start `target/debug/yunari-app.exe` (or `target/release/…`). The editor
   opens; the host starts in the same process on port 5227, trying 5227–5236
   when a port is taken.

3. Open a theme as a player window: `yunari-app.exe path\to\theme.json`.

## What it needs

- Windows 10 or 11, 64-bit. No WebView, no Node, no Electron.
- Options it reads when present:
  - LibreHardwareMonitor's web server (default `http://localhost:8085`) for
    temperatures, fans, power and per-GPU detail.
  - RTSS, or the PresentMon path in the configuration, for frame metrics.
  - Steam, for the foreground game's name and cached box art.
  - A theme store origin in `%USERPROFILE%\.yunari\store.json`
    (`{"origin":"https://…"}`) for the catalogue proxy.

## Where it keeps state

`%USERPROFILE%\.yunari`

- `themes\<id>\theme.json` and `assets\` — the library.
- `display-clients.json`, `active-theme.json`, `hosting.json`, `host.lock` —
  displays, assignment, LAN setting and the single-instance lock.
- `tls\` — the local certificate authority, minted on first run. Its root is
  at `/yunari-ca.crt` for a phone to install.
- `weather.json`, `sources.json`, `weather-icons\`, `media-artwork\` — provider
  settings and their caches.
- A startup entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
  when autostart is switched on.

## Uninstall

1. Remove the startup entry if it was enabled:
   `reg delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v Yunari /f`
2. Delete `%USERPROFILE%\.yunari` (this deletes themes — export first if you
   want them).
3. Delete the executable.
4. On a phone, remove the Yunari root certificate from its trusted store.

## Talking to an agent

`yunari-mcp`, built beside the app, bridges JSON-RPC on stdin to the running
host's `POST /api/mcp`: it finds the port in `%USERPROFILE%\.yunari\settings\host.lock`,
so point an MCP client at that binary with no arguments while the app is running.

## Not shipped yet

There is no MSI/MSIX, no winget manifest and no signed binary: signing and a
published update channel are open follow-ups. Until they exist, the honest
distribution is a built binary, not an unsigned installer.
