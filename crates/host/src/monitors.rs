//! The monitors this PC drives, named the way a local display names one.
//!
//! The uuid is the same uuid GPUI gives the monitor: a v5 UUID over the
//! device's UTF-16 name, so a monitor a phone adds through the HTTP API and a
//! monitor the Settings dialog names are the same record.

/// The monitor list is asked for on demand, so a stalled PowerShell must not
/// hold a request.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// `.NET` names each screen `\\.\DISPLAY1` and gives its physical rectangle.
const SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
@([System.Windows.Forms.Screen]::AllScreens | ForEach-Object {
  [pscustomobject]@{
    deviceName = $_.DeviceName
    primary = $_.Primary
    x = $_.Bounds.X
    y = $_.Bounds.Y
    width = $_.Bounds.Width
    height = $_.Bounds.Height
  }
}) | ConvertTo-Json -Compress
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub uuid: String,
    pub device_name: String,
    pub primary: bool,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Monitor {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "uuid": self.uuid,
            "deviceName": self.device_name,
            "primary": self.primary,
            "bounds": {
                "x": self.x,
                "y": self.y,
                "width": self.width,
                "height": self.height,
            },
        })
    }
}

/// Every monitor, in the order Windows reports them.
pub fn list() -> Result<Vec<Monitor>, String> {
    let answer = yunari_providers::winps::run_within(SCRIPT, TIMEOUT)?;
    parse(&answer)
}

/// The monitor list from the helper's answer. An entry without a device name is
/// not a monitor this host can drive, so it is dropped rather than guessed at.
fn parse(answer: &str) -> Result<Vec<Monitor>, String> {
    let value: serde_json::Value = serde_json::from_str(answer.trim())
        .map_err(|error| format!("The monitor list is not JSON: {error}"))?;
    let entries = match value {
        serde_json::Value::Array(entries) => entries,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.as_object()?;
            let device_name = entry.get("deviceName")?.as_str()?.trim();
            if device_name.is_empty() {
                return None;
            }
            Some(Monitor {
                uuid: device_uuid(device_name),
                device_name: device_name.to_owned(),
                primary: entry.get("primary").and_then(serde_json::Value::as_bool) == Some(true),
                x: entry
                    .get("x")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
                y: entry
                    .get("y")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
                width: entry
                    .get("width")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
                height: entry
                    .get("height")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
            })
        })
        .collect())
}

/// The uuid GPUI gives a monitor: a v5 UUID over the UTF-16 device name, padded
/// to the fixed 32-code-unit buffer Windows fills.
pub fn device_uuid(device_name: &str) -> String {
    let mut units = [0u16; 32];
    for (slot, unit) in units.iter_mut().zip(device_name.encode_utf16()) {
        *slot = unit;
    }
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_be_bytes()).collect();
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, &bytes).to_string()
}

/// Where a display record's monitor was last seen, in words a person reads.
pub fn label(monitor: &Monitor) -> String {
    format!("Monitor {}×{}", monitor.width, monitor.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_names_each_screen_and_its_rectangle() {
        let answer = r#"[{"deviceName":"\\\\.\\DISPLAY1","primary":true,"x":0,"y":0,"width":2560,"height":1440},{"deviceName":"","primary":false,"x":0,"y":0,"width":0,"height":0}]"#;
        let monitors = parse(answer).expect("an answer");
        // An entry with no device name cannot be driven, so it is dropped.
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].device_name, r"\\.\DISPLAY1");
        assert_eq!((monitors[0].width, monitors[0].height), (2560, 1440));
        assert!(monitors[0].primary);
        // A single object from an older helper is still one monitor.
        let single = r#"{"deviceName":"\\\\.\\DISPLAY2","width":1920,"height":1080}"#;
        assert_eq!(parse(single).unwrap()[0].device_name, r"\\.\DISPLAY2");
        assert!(parse("no monitors here").is_err());
    }

    #[test]
    fn the_uuid_is_the_one_gpui_gives_the_same_device_name() {
        // Uuid::new_v5(UUID_NAMESPACE_DNS, utf16be(device name padded to 32))
        assert_eq!(
            device_uuid(r"\\.\DISPLAY1"),
            "78692035-3a04-52f3-a2bf-1133155ae828"
        );
        assert_ne!(device_uuid(r"\\.\DISPLAY1"), device_uuid(r"\\.\DISPLAY2"));
    }
}
