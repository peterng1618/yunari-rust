//! One Windows location fix, asked for only when no manual place is set.
//!
//! The WinRT `Geolocator` is reached through PowerShell because it has no Rust
//! binding in this workspace, and the script is an embedded constant: nothing
//! user-supplied can reach it. A denial, a timeout and an absent PowerShell are
//! each their own sentence; none is ever turned into a guessed place.

use std::time::Duration;

use serde_json::Value;

pub const DENIED_REASON: &str = "Windows location was denied";
pub const TIMEOUT_REASON: &str = "Windows location did not answer in time";
pub const UNAVAILABLE_REASON: &str =
    "PowerShell was not found, so Windows location is unavailable.";
/// The script ran but Windows refused or could not produce a fix.
pub const SCRIPT_FAILED_REASON: &str = "Windows location is unavailable on this PC.";
pub const FAILED_REASON: &str = "Windows location could not be read.";

/// Windows may show a permission prompt, so this has to outlast a person.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// A fix, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
}

pub const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
try {
  [void][Windows.Devices.Geolocation.Geolocator,Windows.Devices.Geolocation,ContentType=WindowsRuntime]
  $accessOperation = [Windows.Devices.Geolocation.Geolocator]::RequestAccessAsync()
  $access = [System.WindowsRuntimeSystemExtensions]::AsTask($accessOperation).GetAwaiter().GetResult()
  if ($access.ToString() -ne 'Allowed') {
    [Console]::Out.WriteLine('{"error":"denied"}')
    exit 0
  }
  $locator = [Windows.Devices.Geolocation.Geolocator]::new()
  $positionOperation = $locator.GetGeopositionAsync()
  $fix = [System.WindowsRuntimeSystemExtensions]::AsTask($positionOperation).GetAwaiter().GetResult()
  $point = $fix.Coordinate.Point.Position
  $json = ConvertTo-Json -Compress -InputObject ([ordered]@{
    latitude = $point.Latitude
    longitude = $point.Longitude
  })
  [Console]::Out.WriteLine($json)
} catch {
  [Console]::Out.WriteLine('{"error":"unavailable"}')
}
"#;

/// How one fix is produced. A seam so a test never starts PowerShell.
pub type Reader = std::sync::Arc<dyn Fn() -> Result<Position, String> + Send + Sync>;

/// The real reader: the helper script, held to [`TIMEOUT`].
pub fn reader() -> Reader {
    std::sync::Arc::new(read)
}

pub fn read() -> Result<Position, String> {
    from_output(crate::winps::run_within(SCRIPT, TIMEOUT))
}

/// One helper result, read into a fix or the sentence that stopped it.
pub fn from_output(output: Result<String, String>) -> Result<Position, String> {
    let output = output.map_err(|error| {
        if error.contains("did not finish") {
            TIMEOUT_REASON.to_owned()
        } else {
            UNAVAILABLE_REASON.to_owned()
        }
    })?;
    let Some(line) = output.lines().find(|line| !line.trim().is_empty()) else {
        return Err(FAILED_REASON.to_owned());
    };
    match parse_line(line) {
        Some(Fix::Position(position)) => Ok(position),
        Some(Fix::Denied) => Err(DENIED_REASON.to_owned()),
        Some(Fix::Unavailable) => Err(SCRIPT_FAILED_REASON.to_owned()),
        None => Err(FAILED_REASON.to_owned()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Fix {
    Position(Position),
    Denied,
    Unavailable,
}

/// One line from the helper; malformed JSON is not a position and not a place.
fn parse_line(line: &str) -> Option<Fix> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let record = value.as_object()?;
    match record.get("error").and_then(Value::as_str) {
        Some("denied") => return Some(Fix::Denied),
        Some("unavailable") => return Some(Fix::Unavailable),
        _ => {}
    }
    let latitude = record.get("latitude")?.as_f64()?;
    let longitude = record.get("longitude")?.as_f64()?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }
    Some(Fix::Position(Position {
        latitude,
        longitude,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fix_is_read_and_a_denial_is_its_own_sentence() {
        assert_eq!(
            from_output(Ok("{\"latitude\":52.52,\"longitude\":13.405}\n".to_owned())),
            Ok(Position {
                latitude: 52.52,
                longitude: 13.405
            })
        );
        assert_eq!(
            from_output(Ok("{\"error\":\"denied\"}\n".to_owned())).unwrap_err(),
            DENIED_REASON
        );
        assert_eq!(
            from_output(Ok("{\"error\":\"unavailable\"}\n".to_owned())).unwrap_err(),
            SCRIPT_FAILED_REASON
        );
    }

    #[test]
    fn a_garbled_line_or_out_of_range_fix_is_not_a_place() {
        assert_eq!(
            from_output(Ok("not json\n".to_owned())).unwrap_err(),
            FAILED_REASON
        );
        assert_eq!(from_output(Ok(String::new())).unwrap_err(), FAILED_REASON);
        assert_eq!(
            from_output(Ok("{\"latitude\":120,\"longitude\":0}\n".to_owned())).unwrap_err(),
            FAILED_REASON
        );
    }

    #[test]
    fn a_timeout_and_an_absent_powershell_are_different_sentences() {
        assert_eq!(
            from_output(Err("it did not finish within 30 seconds.".to_owned())).unwrap_err(),
            TIMEOUT_REASON
        );
        assert_eq!(
            from_output(Err("The system cannot find the file specified.".to_owned())).unwrap_err(),
            UNAVAILABLE_REASON
        );
    }

    #[test]
    fn the_script_is_the_embedded_winrt_read() {
        assert!(SCRIPT.contains("Windows.Devices.Geolocation.Geolocator"));
        assert!(SCRIPT.contains("RequestAccessAsync"));
        assert!(SCRIPT.contains("GetGeopositionAsync"));
    }
}
