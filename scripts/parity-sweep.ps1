# Runs every matrix probe and writes a machine-readable report.
#
# A probe is one command that exits non-zero when a capability regresses. The
# orchestrator reads the report instead of asking an agent to re-audit rows that
# already have an executable check.
#
#   scripts/parity-sweep.ps1 [-Out target/parity/report.json]
#
# Exit code is 0 when every probe passes, 1 when any fails (the report is
# written either way).
param(
    [string]$Out = "target/parity/report.json"
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$registry = Join-Path $repo "docs/parity/probes.json"
$cargo = Join-Path $env:USERPROFILE ".cargo/bin/cargo.exe"
if (-not (Test-Path $cargo)) { throw "cargo not found at $cargo" }

$rows = Get-Content $registry -Raw | ConvertFrom-Json
$commit = (& git -C $repo rev-parse --short HEAD).Trim()
$probes = @()

foreach ($row in $rows) {
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    # Cargo writes progress to stderr; keep it as captured text, not a
    # terminating PowerShell error.
    $ErrorActionPreference = "Continue"
    $output = & $cargo @($row.args) 2>&1 | Out-String
    $code = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    $watch.Stop()
    $tail = ($output -split "`r?`n" | Where-Object { $_ -ne "" } | Select-Object -Last 8) -join "`n"
    $probes += [ordered]@{
        row      = $row.row
        name     = $row.name
        command  = "cargo " + ($row.args -join " ")
        passed   = ($code -eq 0)
        exitCode = $code
        duration = [math]::Round($watch.Elapsed.TotalSeconds, 1)
        tail     = $tail
    }
    $status = if ($code -eq 0) { "PASS" } else { "FAIL" }
    Write-Output ("{0,-4} {1,-40} {2} ({3:n1}s)" -f $row.row, $row.name, $status, $watch.Elapsed.TotalSeconds)
}

$passed = @($probes | Where-Object { $_.passed }).Count
$report = [ordered]@{
    generatedAt = (Get-Date).ToUniversalTime().ToString("o")
    commit      = $commit
    totalRows   = 15
    summary     = [ordered]@{
        probes = $probes.Count
        passed = $passed
        failed = $probes.Count - $passed
        probeCoverage = [math]::Round($probes.Count / 15, 2)
    }
    probes      = $probes
}

$directory = Split-Path -Parent $Out
if ($directory -and -not (Test-Path $directory)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
$report | ConvertTo-Json -Depth 6 | Set-Content -Path $Out -Encoding utf8
Write-Output ""
Write-Output ("{0}/{1} probes passed, coverage {2} rows of 15 -> {3}" -f $passed, $probes.Count, $probes.Count, $Out)
if ($passed -ne $probes.Count) { exit 1 }
