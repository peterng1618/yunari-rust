# Child-workspace setup for Orca worktrees.
# Idempotent: verifies the Windows Rust toolchain and native build prerequisites,
# writes the linker configuration this machine needs, then primes the cargo
# cache. It never edits source.
param(
    # Downloads the portable toolchain when MSVC is absent. A one-time cost
    # per machine, so a plain run only verifies what provisioning left behind.
    [switch]$Provision
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$cargo = Join-Path $cargoBin 'cargo.exe'
$rustc = Join-Path $cargoBin 'rustc.exe'
$rustup = Join-Path $cargoBin 'rustup.exe'

if (-not (Test-Path $cargo)) {
    throw "Rust is not installed. Install it once with: winget install Rustlang.Rustup, then run: rustup default stable-msvc"
}
$env:PATH = "$cargoBin;$env:PATH"

$rustVersion = (& $rustc --version) -replace '^rustc ', ''
$rustVersion = ($rustVersion -split ' ')[0]
$parts = $rustVersion.Split('.')
if ([int]$parts[0] -lt 1 -or ([int]$parts[0] -eq 1 -and [int]$parts[1] -lt 92)) {
    throw "Rust $rustVersion is too old; the workspace requires 1.92 or later. Run: rustup update stable-msvc"
}

$active = & $rustup show active-toolchain
if ($active -notmatch 'msvc') {
    throw "The active Rust toolchain is not MSVC: $active. Run: rustup default stable-msvc"
}

& $rustup component add rustfmt clippy | Out-Null

$cmake = Get-Command cmake -ErrorAction SilentlyContinue
if (-not $cmake -and (Test-Path 'C:\Program Files\CMake\bin\cmake.exe')) {
    $env:PATH = "C:\Program Files\CMake\bin;$env:PATH"
    $cmake = Get-Command cmake -ErrorAction SilentlyContinue
}
if (-not $cmake) {
    throw "CMake is required by gpui-kit. Install it once with: winget install Kitware.CMake"
}

# MSVC is the supported native toolchain. Where it is absent the workspace can
# still build with clang and the Windows SDK's own import libraries, which need
# no administrator rights to install; that setup is written into a local cargo
# config rather than into source.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsPath = $null
if (Test-Path $vswhere) {
    $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
}

$configPath = Join-Path $repoRoot '.cargo\config.toml'
$configDir = Split-Path -Parent $configPath

if ($vsPath) {
    if (Test-Path $configPath) {
        Remove-Item $configPath -Force
    }
    $toolchain = 'MSVC'
}
else {
    $tool = Join-Path $env:LOCALAPPDATA 'yunari-toolchain'
    if ($Provision) {
        & (Join-Path $PSScriptRoot 'toolchain-portable.ps1')
        if ($LASTEXITCODE -ne 0) {
            throw "The portable toolchain could not be provisioned."
        }
    }
    # Anchored on clang-cl, which only the LLVM distribution provides, so a
    # stray lld-link elsewhere cannot win the search.
    $llvmBin = Get-ChildItem $tool -Recurse -Filter 'clang-cl.exe' -File -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty DirectoryName
    $sdkRoot = Join-Path $env:LOCALAPPDATA 'cargo-xwin\xwin'
    if (-not $llvmBin -or -not (Test-Path (Join-Path $sdkRoot 'sdk\lib\um\x86_64'))) {
        throw "Neither MSVC nor the portable toolchain is present. Run once: scripts/dev-setup.ps1 -Provision"
    }

    $slashes = { param($path) $path -replace '\\', '/' }
    $include = @(
        'crt/include', 'sdk/include/ucrt', 'sdk/include/um',
        'sdk/include/shared', 'sdk/include/winrt'
    ) | ForEach-Object { & $slashes (Join-Path $sdkRoot $_) }
    $config = @"
# Generated for this machine by scripts/dev-setup.ps1; not committed.
# MSVC is absent, so the linker, the compiler and the Windows headers and
# libraries come from the portable toolchain under %LOCALAPPDATA%.
[target.x86_64-pc-windows-msvc]
linker = "$(& $slashes (Join-Path $llvmBin 'lld-link.exe'))"
rustflags = [
  "-C", "linker-flavor=lld-link",
  "-L", "native=$(& $slashes (Join-Path $sdkRoot 'crt/lib/x86_64'))",
  "-L", "native=$(& $slashes (Join-Path $sdkRoot 'sdk/lib/um/x86_64'))",
  "-L", "native=$(& $slashes (Join-Path $sdkRoot 'sdk/lib/ucrt/x86_64'))",
]

[env]
CC = "$(& $slashes (Join-Path $llvmBin 'clang-cl.exe'))"
CXX = "$(& $slashes (Join-Path $llvmBin 'clang-cl.exe'))"
AR = "$(& $slashes (Join-Path $llvmBin 'llvm-lib.exe'))"
RC = "$(& $slashes (Join-Path $llvmBin 'llvm-rc.exe'))"
INCLUDE = "$($include -join ';')"
"@
    New-Item -ItemType Directory -Force $configDir | Out-Null
    Set-Content -Path $configPath -Value $config -Encoding utf8
    $toolchain = 'portable clang + Windows SDK libraries'
}

Push-Location $repoRoot
try {
    & $cargo fetch
    if ($LASTEXITCODE -ne 0) {
        throw "cargo fetch exited with $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

Write-Output "yunari-rust workspace ready: rust $rustVersion (msvc), cmake, $toolchain, crates fetched."
