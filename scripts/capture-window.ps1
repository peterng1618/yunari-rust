# Captures a Yunari window at an exact client size for parity evidence.
#
# PrintWindow is used (PW_RENDERFULLCONTENT) so the window does not have to be
# foreground and the user's screen/pointer is never taken over. The window is
# our own process, started here and stopped at the end.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Theme = "",
    [int]$Width = 1920,
    [int]$Height = 1080,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$DelaySeconds = 5
)

$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class WinCap {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool AdjustWindowRectEx(ref RECT rect, uint style, bool menu, uint exStyle);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hWnd, int index);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int cmd);
}
"@

$arguments = @()
if ($Theme -ne "") { $arguments += $Theme }
$process = Start-Process -FilePath $Exe -ArgumentList $arguments -PassThru

try {
    $handle = [IntPtr]::Zero
    for ($i = 0; $i -lt 100 -and $handle -eq [IntPtr]::Zero; $i++) {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
        $handle = $process.MainWindowHandle
    }
    if ($handle -eq [IntPtr]::Zero) { throw "The window never appeared." }

    # Restore first so the frame is not collapsed, then size the client area.
    [WinCap]::ShowWindow($handle, 9) | Out-Null   # SW_RESTORE
    $style = [WinCap]::GetWindowLong($handle, -16)
    $exStyle = [WinCap]::GetWindowLong($handle, -20)
    $rect = New-Object WinCap+RECT
    $rect.Left = 0; $rect.Top = 0; $rect.Right = $Width; $rect.Bottom = $Height
    [WinCap]::AdjustWindowRectEx([ref]$rect, [uint32]$style, $false, [uint32]$exStyle) | Out-Null
    $outerWidth = $rect.Right - $rect.Left
    $outerHeight = $rect.Bottom - $rect.Top
    [WinCap]::SetWindowPos($handle, [IntPtr]::Zero, 0, 0, $outerWidth, $outerHeight, 0x0004) | Out-Null  # SWP_NOZORDER

    Start-Sleep -Seconds $DelaySeconds
    $process.Refresh()

    $bitmap = New-Object System.Drawing.Bitmap($outerWidth, $outerHeight)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    if (-not [WinCap]::PrintWindow($handle, $hdc, 2)) { throw "PrintWindow refused the capture." }
    $graphics.ReleaseHdc($hdc)
    $graphics.Dispose()

    $directory = Split-Path -Parent $Out
    if ($directory -and -not (Test-Path $directory)) { New-Item -ItemType Directory -Path $directory -Force | Out-Null }
    $bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    Write-Output "Captured: $Out ($outerWidth x $outerHeight)"
}
finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
}
