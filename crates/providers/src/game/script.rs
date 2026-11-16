//! The embedded PowerShell helper that names the foreground window's process.
//!
//! `GetForegroundWindow` and `GetWindowThreadProcessId` need no elevation and
//! raise no prompt. `Process.MainModule.FileName` throws for a process this
//! user may not inspect, so a line carries whatever was readable and the caller
//! never treats a missing path as a name it can invent.

use serde_json::Value;

pub const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class YunariForegroundWindow {
  [StructLayout(LayoutKind.Sequential)]
  public struct Rect {
    public int Left;
    public int Top;
    public int Right;
    public int Bottom;
  }

  [StructLayout(LayoutKind.Sequential)]
  public struct MonitorInfo {
    public int cbSize;
    public Rect rcMonitor;
    public Rect rcWork;
    public uint dwFlags;
  }

  private const uint MonitorDefaultToNearest = 2;
  private const int D3DFullScreen = 3;

  [DllImport("user32.dll")]
  public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")]
  public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern int GetClassName(IntPtr hWnd, System.Text.StringBuilder lpClassName, int nMaxCount);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern IntPtr FindWindowEx(IntPtr hWndParent, IntPtr hWndChildAfter, string lpszClass, string lpszWindow);
  [DllImport("user32.dll")]
  public static extern bool GetWindowRect(IntPtr hWnd, out Rect lpRect);
  [DllImport("user32.dll")]
  public static extern IntPtr MonitorFromWindow(IntPtr hWnd, uint dwFlags);
  [DllImport("user32.dll")]
  public static extern bool GetMonitorInfo(IntPtr hMonitor, ref MonitorInfo lpmi);
  [DllImport("shell32.dll")]
  public static extern int SHQueryUserNotificationState(out int state);

  public static string ClassNameOf(IntPtr hWnd) {
    var name = new System.Text.StringBuilder(256);
    return GetClassName(hWnd, name, name.Capacity) > 0 ? name.ToString() : "";
  }

  public static bool IsFullscreen(IntPtr hWnd) {
    Rect window;
    if (!GetWindowRect(hWnd, out window)) return false;
    IntPtr monitor = MonitorFromWindow(hWnd, MonitorDefaultToNearest);
    if (monitor == IntPtr.Zero) return false;
    var info = new MonitorInfo();
    info.cbSize = Marshal.SizeOf(typeof(MonitorInfo));
    if (!GetMonitorInfo(monitor, ref info)) return false;
    return window.Left == info.rcMonitor.Left &&
      window.Top == info.rcMonitor.Top &&
      window.Right == info.rcMonitor.Right &&
      window.Bottom == info.rcMonitor.Bottom;
  }

  public static bool IsD3DFullscreen() {
    int state;
    return SHQueryUserNotificationState(out state) == 0 && state == D3DFullScreen;
  }
}
'@
while ($true) {
  $handle = [YunariForegroundWindow]::GetForegroundWindow()
  [uint32]$processId = 0
  $class = ''
  $fullscreen = $false
  $d3dFullscreen = $false
  if ($handle -ne [IntPtr]::Zero) {
    $class = [YunariForegroundWindow]::ClassNameOf($handle)
    if ($class -eq 'ApplicationFrameWindow') {
      $child = [YunariForegroundWindow]::FindWindowEx($handle, [IntPtr]::Zero, 'Windows.UI.Core.CoreWindow', [NullString]::Value)
      if ($child -ne [IntPtr]::Zero) {
        $handle = $child
        $class = [YunariForegroundWindow]::ClassNameOf($handle)
      }
    }
    [void][YunariForegroundWindow]::GetWindowThreadProcessId($handle, [ref]$processId)
    $fullscreen = [YunariForegroundWindow]::IsFullscreen($handle)
  }
  $d3dFullscreen = [YunariForegroundWindow]::IsD3DFullscreen()

  $line = [ordered]@{
    kind = 'foreground'
    pid = [int]$processId
    fullscreen = $fullscreen
    d3dFullscreen = $d3dFullscreen
  }
  if ($class -ne '') { $line['class'] = $class }

  if ($processId -ne 0) {
    $path = $null
    $name = $null
    try {
      $process = [System.Diagnostics.Process]::GetProcessById([int]$processId)
      $name = $process.ProcessName
      $path = $process.MainModule.FileName
    } catch {
      $path = $null
    }
    $line['path'] = $path
    $line['name'] = $name
  }
  [Console]::Out.WriteLine(($line | ConvertTo-Json -Compress))

  Start-Sleep -Milliseconds 1000
}
"#;

/// What the foreground window's process was, as the helper read it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Foreground {
    /// Zero means no foreground window.
    pub pid: u32,
    pub path: Option<String>,
    /// The process name, when the executable path could not be read.
    pub name: Option<String>,
    pub class: Option<String>,
    pub fullscreen: bool,
    /// `SHQueryUserNotificationState` reported a running D3D fullscreen app.
    pub d3d_fullscreen: bool,
}

impl Foreground {
    /// The executable's file name, from the path when there is one.
    pub fn executable(&self) -> Option<&str> {
        self.path
            .as_deref()
            .and_then(|path| path.rsplit(['\\', '/']).next())
            .filter(|name| !name.is_empty())
            .or(self.name.as_deref())
    }
}

/// One helper line, or nothing when the line is not a foreground reading.
pub fn parse(line: &str) -> Option<Foreground> {
    let payload: Value = serde_json::from_str(line).ok()?;
    if payload.get("kind").and_then(Value::as_str) != Some("foreground") {
        return None;
    }
    let pid = payload.get("pid")?.as_f64()?;
    if !pid.is_finite() || pid < 0.0 || pid.fract() != 0.0 || pid > u32::MAX as f64 {
        return None;
    }
    let text = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let flag = |field: &str| payload.get(field).and_then(Value::as_bool).unwrap_or(false);
    Some(Foreground {
        pid: pid as u32,
        path: text("path"),
        name: text("name"),
        class: text("class"),
        fullscreen: flag("fullscreen"),
        d3d_fullscreen: flag("d3dFullscreen"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_foreground_line_is_the_process_and_its_window() {
        let line = r#"{"kind":"foreground","pid":4242,"class":"UnityWndClass","fullscreen":true,"d3dFullscreen":true,"path":"D:\\Games\\game.exe","name":"game"}"#;
        let foreground = parse(line).expect("a foreground line");
        assert_eq!(foreground.pid, 4242);
        assert_eq!(foreground.executable(), Some("game.exe"));
        assert!(foreground.fullscreen);
        assert!(foreground.d3d_fullscreen);
    }

    #[test]
    fn a_line_without_a_readable_path_still_names_the_process() {
        let line = r#"{"kind":"foreground","pid":7,"name":"explorer"}"#;
        let foreground = parse(line).expect("a foreground line");
        assert_eq!(foreground.executable(), Some("explorer"));
        // No foreground window is not a name.
        let idle = parse(r#"{"kind":"foreground","pid":0}"#).expect("a line");
        assert_eq!(idle.executable(), None);
        assert_eq!(parse(r#"{"kind":"odd","pid":1}"#), None);
        assert_eq!(parse(r#"{"kind":"foreground","pid":1.5}"#), None);
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn the_script_reads_the_foreground_window_without_elevation() {
        assert!(SCRIPT.contains("GetForegroundWindow"));
        assert!(SCRIPT.contains("SHQueryUserNotificationState"));
        // The window class decides a packaged app, not its frame host.
        assert!(SCRIPT.contains("Windows.UI.Core.CoreWindow"));
    }
}
