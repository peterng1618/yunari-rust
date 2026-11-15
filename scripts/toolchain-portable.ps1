# Provisions the portable Windows toolchain used when MSVC is absent:
# cargo-xwin for the Windows SDK's headers and import libraries, LLVM for
# clang-cl, llvm-lib, llvm-rc and lld-link, and zstd to unpack LLVM.
#
# Everything lands under %LOCALAPPDATA%, so no administrator rights are needed
# and nothing on the machine is registered. Run through scripts/dev-setup.ps1 -Provision.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$tool = Join-Path $env:LOCALAPPDATA 'yunari-toolchain'
$llvmVersion = '23.1.5'
$llvmArchive = "clang+llvm-$llvmVersion-x86_64-pc-windows-msvc"
$llvmRoot = Join-Path $tool "llvm-$llvmVersion"

New-Item -ItemType Directory -Force $tool | Out-Null

function Get-File([string]$url, [string]$path) {
    if (Test-Path $path) {
        Write-Output "have $(Split-Path -Leaf $path)"
        return
    }
    Write-Output "download $url"
    Invoke-WebRequest $url -OutFile $path -UseBasicParsing
}

# 1. The Windows SDK headers and import libraries, from Microsoft's own CDN.
$cargoXwin = Join-Path $tool 'xwin\cargo-xwin.exe'
if (-not (Test-Path $cargoXwin)) {
    $zip = Join-Path $tool 'cargo-xwin.zip'
    Get-File 'https://github.com/rust-cross/cargo-xwin/releases/download/v0.23.1/cargo-xwin-v0.23.1.windows-x64.zip' $zip
    Expand-Archive $zip -DestinationPath (Join-Path $tool 'xwin') -Force
}
$sdkRoot = Join-Path $env:LOCALAPPDATA 'cargo-xwin\xwin'
if (-not (Test-Path (Join-Path $sdkRoot 'sdk\lib\um\x86_64'))) {
    Write-Output 'splat the Windows SDK and CRT'
    & $cargoXwin env --target x86_64-pc-windows-msvc | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-xwin could not download the Windows SDK."
    }
}

# 2. LLVM, which supplies every tool the MSVC build scripts expect on Windows.
$clangCl = Join-Path $llvmRoot "$llvmArchive\bin\clang-cl.exe"
if (-not (Test-Path $clangCl)) {
    $zstd = Get-ChildItem $tool -Recurse -Filter 'zstd.exe' -File -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
    if (-not $zstd) {
        $zip = Join-Path $tool 'zstd.zip'
        Get-File 'https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-v1.5.7-win64.zip' $zip
        Expand-Archive $zip -DestinationPath (Join-Path $tool 'zstd') -Force
        $zstd = Get-ChildItem $tool -Recurse -Filter 'zstd.exe' -File | Select-Object -First 1 -ExpandProperty FullName
    }
    $archive = Join-Path $tool "llvm-$llvmVersion.tar.zst"
    Get-File "https://github.com/llvm/llvm-project/releases/download/llvmorg-$llvmVersion/$llvmArchive.tar.zst" $archive

    $tar = Join-Path $tool "llvm-$llvmVersion.tar"
    Write-Output 'decompress LLVM (a large archive with a wide window)'
    & $zstd -d -f --long=31 $archive -o $tar | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "zstd could not decompress $archive."
    }
    & tar -xf $tar -C $llvmRoot
    if ($LASTEXITCODE -ne 0) {
        throw "tar could not extract $tar."
    }
    Remove-Item $tar -Force
}

Write-Output "portable toolchain ready under $tool"
