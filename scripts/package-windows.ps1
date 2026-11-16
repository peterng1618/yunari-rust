# Builds the release binaries and stages this PC's copy as a zip.
#
# There is no installer service and no npm package: the app is one executable
# plus the MCP bridge beside it. Everything the app serves (pages, player,
# weather icons) is compiled into the binary; the settings folder is created on
# first run, so the zip is the whole product.
param(
    [string]$Version = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"

if ($Version -eq "") {
    $manifest = Get-Content (Join-Path $root "Cargo.toml") -Raw
    if ($manifest -notmatch '(?m)^\[workspace\.package\][\s\S]*?^version = "([^"]+)"') {
        throw "The workspace version could not be read from Cargo.toml."
    }
    $Version = $Matches[1]
}

if (-not $SkipBuild) {
    # GPUI compiles its shaders only when debug assertions are off, so a release
    # build needs the Windows SDK's shader compiler before cargo starts.
    $fxc = if ($env:GPUI_FXC_PATH) { $env:GPUI_FXC_PATH } else { (Get-Command fxc.exe -ErrorAction SilentlyContinue).Source }
    if (-not $fxc -or -not (Test-Path $fxc)) {
        throw "A release build needs the Windows SDK's fxc.exe on PATH or in GPUI_FXC_PATH. Debug builds show the same UI; only this packaging step needs the SDK."
    }
    & $cargo build --release --manifest-path (Join-Path $root "Cargo.toml") -p yunari-app -p yunari-host
    if ($LASTEXITCODE -ne 0) { throw "The release build failed." }
}

$release = Join-Path $root "target\release"
$app = Join-Path $release "yunari-app.exe"
$mcp = Join-Path $release "yunari-mcp.exe"
foreach ($binary in @($app, $mcp)) {
    if (-not (Test-Path $binary)) { throw "Missing release binary: $binary" }
}

$dist = Join-Path $root "dist"
$stage = Join-Path $dist "yunari-$Version-windows-x64"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item $app (Join-Path $stage "Yunari.exe")
Copy-Item $mcp (Join-Path $stage "yunari-mcp.exe")
# `yunari-mcp install` copies this skill beside the binaries, so the zip carries
# it rather than reading the checkout it was built from.
Copy-Item -Recurse (Join-Path $root "skills") (Join-Path $stage "skills")

$readme = @"
Yunari $Version (Windows x64)

Yunari.exe        the host, the local displays and the theme editor.
yunari-mcp.exe    the stdio bridge agents use to reach the host's MCP surface.
skills\           the theme-designer skill an agent reads.

Run Yunari.exe. On first run it creates its settings folder under
%USERPROFILE%\.yunari and opens the editor. The host serves this PC on a loopback
port; pairing a phone is done from the host's pair page.

Agent tools: run `yunari-mcp.exe install` to register the bridge with Claude
Code and copy the skill; `yunari-mcp.exe status` reports both, and
`yunari-mcp.exe uninstall` removes them again.

Uninstall: quit Yunari from its window, delete this folder, and delete
%USERPROFILE%\.yunari if you also want to discard the themes and settings.

MIT licensed.
"@
Set-Content -Path (Join-Path $stage "README.txt") -Value $readme -Encoding utf8

$zip = Join-Path $dist "yunari-$Version-windows-x64.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
$hash = (Get-FileHash -Path $zip -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Output "Staged: $stage"
Write-Output "Archive: $zip"
Write-Output "SHA-256: $hash"
