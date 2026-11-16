//! The embedded PowerShell helper that reads Windows Now Playing through the
//! SMTC session manager.
//!
//! Two things in it are not guessable and are pinned by tests in the source
//! this port came from: the WinRT awaits go through the *generic* `AsTask`
//! overload (the non-generic `IAsyncAction` one has the same arity and makes
//! `MakeGenericMethod` throw), and the output encoding is set before the first
//! write (without it a Vietnamese or CJK title arrives as mojibake through the
//! pipe). The artwork half additionally needs `DataReader`'s reflected
//! constructor, because it has no public constructor PowerShell can bind.

/// The largest raw image whose base64 JSON line still fits the line buffer:
/// 4/3 expansion plus room for the envelope, derived rather than guessed.
pub const ARTWORK_RAW_BYTES: usize = (crate::winps::LINE_CAP - 512) * 3 / 4;

const HEADER: &str = r##"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager,Windows.Media.Control,ContentType=WindowsRuntime]
$null = [Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties,Windows.Media.Control,ContentType=WindowsRuntime]

$AsTaskMethod = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
  $_.Name -eq 'AsTask' -and $_.IsGenericMethodDefinition -and $_.GetParameters().Count -eq 1 -and $_.GetGenericArguments().Count -eq 1
} | Select-Object -First 1

function Wait-WinRt($operation, [Type]$resultType) {
  if ($null -eq $AsTaskMethod) { throw 'no generic AsTask overload' }
  $task = $AsTaskMethod.MakeGenericMethod($resultType).Invoke($null, @($operation))
  $task.GetAwaiter().GetResult()
}

function Convert-PlaybackState($playbackStatus) {
  $name = [string]$playbackStatus
  if ($name -eq 'Playing') { return 'playing' }
  if ($name -eq 'Paused') { return 'paused' }
  return 'stopped'
}

$AppNames = @{}
function Get-AppName($aumid) {
  if ($AppNames.ContainsKey($aumid)) { return $AppNames[$aumid] }
  $name = $aumid
  try {
    $key = "HKCU:\\Software\\Classes\\AppUserModelId\\$aumid"
    if (Test-Path $key) {
      $display = (Get-ItemProperty $key -ErrorAction Stop).DisplayName
      if (-not [string]::IsNullOrWhiteSpace($display)) { $name = $display }
    }
  } catch { }
  $AppNames[$aumid] = $name
  return $name
}

function Get-SessionId($a,$t,$r,$b){$a+':'+([string]$t+[string]$r+[string]$b).GetHashCode()}

function Get-SessionLine($session, $isCurrent = $false) {
  $properties = Wait-WinRt ($session.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
  $playback = $session.GetPlaybackInfo()
  $state = Convert-PlaybackState $playback.PlaybackStatus
  $timeline = $session.GetTimelineProperties()
  $position = $timeline.Position.TotalSeconds
  if ($state -eq 'playing') {
    $position = $position + ([DateTimeOffset]::UtcNow - $timeline.LastUpdatedTime.ToUniversalTime()).TotalSeconds
  }
  if ($position -lt 0) { $position = 0 }
  [ordered]@{
    kind = 'session'
    app = Get-AppName $session.SourceAppUserModelId
    sessionId = Get-SessionId $session.SourceAppUserModelId $properties.Title $properties.Artist $properties.AlbumTitle
    title = $properties.Title
    artist = $properties.Artist
    album = $properties.AlbumTitle
    state = $state
    current = $isCurrent
    positionS = [math]::Round($position, 3)
    durationS = [math]::Round($timeline.EndTime.TotalSeconds, 3)
    hasArtwork = ($null -ne $properties.Thumbnail)
  }
}
"##;

/// Endpoint volume and mute through CoreAudio. No endpoint means neither key,
/// never zero and unmuted. Compact because the whole script travels as one
/// `-EncodedCommand`, which Windows caps near 32k characters.
const AUDIO: &str = r##"
$AudioInterop = @'
using System;using System.Runtime.InteropServices;
public static class YunariAudio{
[ComImport,Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]class E{}
[ComImport,Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]interface N{[PreserveSig]int X0();[PreserveSig]int GetDefaultAudioEndpoint(int f,int r,out D d);}
[ComImport,Guid("D666063F-1587-4E43-81F1-B948E807363F"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]interface D{[PreserveSig]int Activate(ref Guid i,int c,IntPtr p,out V v);}
[ComImport,Guid("5CDF2C82-841E-4546-9722-0CF74078229A"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]interface V{[PreserveSig]int X0();[PreserveSig]int X1();[PreserveSig]int X2();[PreserveSig]int X3();[PreserveSig]int X4();[PreserveSig]int X5();[PreserveSig]int GetMasterVolumeLevelScalar(out float l);[PreserveSig]int X7();[PreserveSig]int X8();[PreserveSig]int X9();[PreserveSig]int X10();[PreserveSig]int SetMute([MarshalAs(UnmanagedType.Bool)]bool m,IntPtr c);[PreserveSig]int GetMute([MarshalAs(UnmanagedType.Bool)]out bool m);}
static readonly Guid I=new Guid("5CDF2C82-841E-4546-9722-0CF74078229A");const int C=23;
public static bool TryRead(out int v,out bool m){v=0;m=false;try{var n=(N)new E();D d;if(n.GetDefaultAudioEndpoint(0,1,out d)!=0||d==null)return false;Guid i=I;V a;if(d.Activate(ref i,C,IntPtr.Zero,out a)!=0||a==null)return false;float l;if(a.GetMasterVolumeLevelScalar(out l)!=0)return false;bool b;if(a.GetMute(out b)!=0)return false;v=(int)Math.Round(l*100.0);m=b;return true;}catch{return false;}}}
'@
$AudioAvailable = $false
try {
  Add-Type -TypeDefinition $AudioInterop -Language CSharp
  $AudioAvailable = $true
} catch {
  $AudioAvailable = $false
}

function Get-AudioLine {
  if (-not $AudioAvailable) { return [ordered]@{ kind = 'audio' } }
  $volume = 0
  $muted = $false
  if (-not [YunariAudio]::TryRead([ref]$volume, [ref]$muted)) {
    return [ordered]@{ kind = 'audio' }
  }
  [ordered]@{
    kind = 'audio'
    volume = [int]$volume
    muted = [bool]$muted
  }
}
"##;

const ARTWORK: &str = r##"
$null = [Windows.Storage.Streams.DataReader,Windows.Storage.Streams,ContentType=WindowsRuntime]
$null = [Windows.Storage.Streams.IInputStream,Windows.Storage.Streams,ContentType=WindowsRuntime]
$null = [Windows.Storage.Streams.IRandomAccessStreamWithContentType,Windows.Storage.Streams,ContentType=WindowsRuntime]

$ArtworkRetryLimit = 5
$artworkAttempts = @{}

function Wait-Operation($operation) {
  $deadline = [DateTime]::UtcNow.AddSeconds(10)
  while ([string]$operation.Status -eq 'Started') {
    if ([DateTime]::UtcNow -gt $deadline) { throw 'async operation did not finish' }
    Start-Sleep -Milliseconds 5
  }
  if ([string]$operation.Status -ne 'Completed') { throw "async operation ended: $($operation.Status)" }
  return $operation.GetResults()
}

function Test-CompleteImage($buffer, $ext) {
  if ($ext -eq 'png') {
    if ($buffer.Length -lt 8) { return $false }
    return ([Text.Encoding]::ASCII.GetString($buffer, $buffer.Length - 8, 4) -eq 'IEND') -and
      ($buffer[$buffer.Length - 4] -eq 0xae -and $buffer[$buffer.Length - 3] -eq 0x42 -and
       $buffer[$buffer.Length - 2] -eq 0x60 -and $buffer[$buffer.Length - 1] -eq 0x82)
  }
  if ($ext -eq 'jpg') {
    return ($buffer.Length -ge 2) -and ($buffer[$buffer.Length - 2] -eq 0xff) -and ($buffer[$buffer.Length - 1] -eq 0xd9)
  }
  if ($ext -eq 'webp') {
    if ($buffer.Length -lt 12) { return $false }
    return ([int64][BitConverter]::ToUInt32($buffer, 4) + 8) -eq $buffer.Length
  }
  return $false
}

function Write-ArtworkMiss($id, $why) {
  [Console]::Out.WriteLine(([ordered]@{kind='artwork-miss';sessionId=$id;reason=$why} | ConvertTo-Json -Compress))
}

function Get-ArtworkResult($session, $sessionId) {
  $properties = Wait-WinRt ($session.TryGetMediaPropertiesAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionMediaProperties])
  $thumbnail = $properties.Thumbnail
  if ($null -eq $thumbnail) { return @{ status = 'retry' } }
  $stream = Wait-WinRt ($thumbnail.OpenReadAsync()) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])

  $constructor = [Windows.Storage.Streams.DataReader].GetConstructor(@([Windows.Storage.Streams.IInputStream]))
  if ($null -eq $constructor) { return @{ status = 'retry' } }
  $reader = $constructor.Invoke(@($stream))
  $chunks = [System.Collections.Generic.List[byte]]::new()
  try {
    while ($true) {
      $loaded = [int](Wait-Operation ($reader.LoadAsync(65536)))
      if ($loaded -le 0) { break }
      $chunk = [byte[]]::new($loaded)
      $reader.ReadBytes($chunk)
      $chunks.AddRange($chunk)
      if ($chunks.Count -gt 786048) { return @{ status = 'over' } }
    }
  } finally {
    $reader.Dispose()
  }

  $buffer = $chunks.ToArray()
  if ($buffer.Length -lt 12) { return @{ status = 'retry' } }
  $ext = $null
  if ($buffer[0] -eq 0x89 -and $buffer[1] -eq 0x50 -and $buffer[2] -eq 0x4e -and $buffer[3] -eq 0x47) { $ext = 'png' }
  elseif ($buffer[0] -eq 0xff -and $buffer[1] -eq 0xd8 -and $buffer[2] -eq 0xff) { $ext = 'jpg' }
  elseif ([Text.Encoding]::ASCII.GetString($buffer, 0, 4) -eq 'RIFF' -and [Text.Encoding]::ASCII.GetString($buffer, 8, 4) -eq 'WEBP') { $ext = 'webp' }
  if ($null -eq $ext) { return @{ status = 'retry' } }
  if (-not (Test-CompleteImage $buffer $ext)) { return @{ status = 'retry' } }

  $hash = [System.Security.Cryptography.SHA256]::Create().ComputeHash($buffer)
  $hex = -join ($hash | ForEach-Object { $_.ToString('x2') })
  @{ status = 'artwork'; line = [ordered]@{
    kind = 'artwork'
    sessionId = $sessionId
    sha256 = $hex
    ext = $ext
    base64 = [Convert]::ToBase64String($buffer)
  } }
}

function Try-EmitArtwork($session, $line) {
  $sessionId = $line['sessionId']
  $key = "$sessionId|$($line['title'])|$($line['artist'])|$($line['album'])"
  if ($lastArtwork.ContainsKey($sessionId) -and $lastArtwork[$sessionId] -eq $key) { return }
  try {
    $result = Get-ArtworkResult $session $sessionId
  } catch {
    return
  }
  if ($result['status'] -eq 'artwork') {
    [Console]::Out.WriteLine(($result['line'] | ConvertTo-Json -Compress))
    $lastArtwork[$sessionId] = $key
    $artworkAttempts.Remove($sessionId)
    return
  }
  if ($result['status'] -eq 'over') {
    $lastArtwork[$sessionId] = $key
    $artworkAttempts.Remove($sessionId)
    Write-ArtworkMiss $sessionId 'over'
    return
  }
  $attempt = @{ key = $key; count = 0 }
  if ($artworkAttempts.ContainsKey($sessionId)) { $attempt = $artworkAttempts[$sessionId] }
  if ($attempt['key'] -ne $key) { $attempt = @{ key = $key; count = 0 } }
  $attempt['count'] = $attempt['count'] + 1
  $artworkAttempts[$sessionId] = $attempt
  if ($attempt['count'] -ge $ArtworkRetryLimit) {
    $lastArtwork[$sessionId] = $key
    $artworkAttempts.Remove($sessionId)
    Write-ArtworkMiss $sessionId 'exhausted'
  }
}
"##;

const POLL: &str = r##"
$manager = $null
while ($true) {
  try {
    [Console]::Out.WriteLine((Get-AudioLine | ConvertTo-Json -Compress))
  } catch {
    [Console]::Out.WriteLine('{"kind":"audio"}')
  }

  if ($null -eq $manager) {
    try {
      $manager = Wait-WinRt ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager]::RequestAsync()) ([Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager])
    } catch {
      $manager = $null
    }
  }
  if ($null -eq $manager) {
    [Console]::Out.WriteLine('{"kind":"error"}')
    Start-Sleep -Milliseconds 1000
    continue
  }

  $sessions = $null
  try {
    $raw = $manager.GetSessions()
    if ($null -eq $raw) { throw 'GetSessions returned null' }
    $sessions = @($raw | ForEach-Object { $_ })
  } catch {
    $sessions = $null
    [Console]::Out.WriteLine('{"kind":"error"}')
  }
  if ($null -ne $sessions) {
    if ($sessions.Count -eq 0) {
      [Console]::Out.WriteLine('{"kind":"none"}')
    } else {
      $currentId = $null
      try {
        $current = $manager.GetCurrentSession()
        if ($null -ne $current) { $currentId = $current.SourceAppUserModelId }
      } catch {
        $currentId = $null
      }
      foreach ($session in $sessions) {
        try {
          $isCurrent = ($null -ne $currentId) -and ($session.SourceAppUserModelId -eq $currentId)
          $line = Get-SessionLine $session $isCurrent
          [Console]::Out.WriteLine(($line | ConvertTo-Json -Compress))
          if ($WantArtwork) { Try-EmitArtwork $session $line }
        } catch {
          [Console]::Out.WriteLine('{"kind":"error"}')
        }
      }
      if($WantArtwork-and$lastArtwork.Count-gt64){$lastArtwork.Clear();$artworkAttempts.Clear()}
    }
  }
  Start-Sleep -Milliseconds 1000
}
"##;

/// The poll script. Artwork is compiled out unless the caller asked for it:
/// the script chosen at spawn is the request.
pub fn script(artwork: bool) -> String {
    if artwork {
        format!("{HEADER}{AUDIO}{ARTWORK}\n$WantArtwork = $true\n$lastArtwork = @{{}}\n{POLL}")
    } else {
        format!("{HEADER}{AUDIO}{POLL}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_carries_the_two_pinned_winrt_details_and_the_poll() {
        let plain = script(false);
        assert!(plain.contains("[Console]::OutputEncoding = [Text.Encoding]::UTF8"));
        assert!(plain.contains("$_.IsGenericMethodDefinition"));
        assert!(plain.contains("while ($true)"));
        // No artwork read unless a display binds `media.artwork`.
        assert!(!plain.contains("$WantArtwork = $true"));
        assert!(!plain.contains("[Windows.Storage.Streams.DataReader"));

        let with_artwork = script(true);
        assert!(with_artwork.contains("$WantArtwork = $true"));
        assert!(with_artwork.contains("GetConstructor(@([Windows.Storage.Streams.IInputStream]))"));
        assert!(with_artwork.contains("786048"));
    }
}
