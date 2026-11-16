//! LibreHardwareMonitor's `data.json` tree.
//!
//! LHM is an optional external program, never a linked library, so this reads
//! its published JSON and translates nothing it does not have.

use serde_json::Value;

/// One sensor LHM publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct LhmSensor {
    /// LHM's own identifier, e.g. `/intelcpu/0/temperature/2`.
    pub sensor_id: String,
    /// The hardware node this sensor sits under, e.g. `/intelcpu/0`.
    pub hardware_id: String,
    /// The hardware node's own name, e.g. `GpuNvidia`, `Ryzen 9 5950X`.
    pub hardware_name: String,
    pub text: String,
    /// LHM's sensor kind: `Temperature`, `Load`, `Throughput`, `Data`, …
    pub kind: String,
    /// The unformatted value. LHM writes `"NaN"` for a sensor it cannot read,
    /// which is a gap here rather than a zero.
    pub value: Option<f64>,
}

/// Flattens the tree to its readable sensors.
///
/// A sensor node is the one carrying both `SensorId` and `Type`; every other
/// node is a container whose hardware it remembers for its children.
pub fn flatten(payload: &Value) -> Vec<LhmSensor> {
    let mut sensors = Vec::new();
    let Some(children) = payload.get("Children").and_then(Value::as_array) else {
        return sensors;
    };
    for child in children {
        walk(child, "", "", &mut sensors);
    }
    sensors
}

fn walk(node: &Value, hardware_id: &str, hardware_name: &str, sensors: &mut Vec<LhmSensor>) {
    let own_id = node
        .get("HardwareId")
        .and_then(Value::as_str)
        .unwrap_or(hardware_id);
    // A hardware node's name is its own `Text` ("GpuNvidia", "Storage", …);
    // a sensor's `Text` names the sensor instead.
    let own_name = if node.get("HardwareId").is_some() {
        node.get("Text")
            .and_then(Value::as_str)
            .unwrap_or(hardware_name)
    } else {
        hardware_name
    };

    if let (Some(sensor_id), Some(kind)) = (
        node.get("SensorId").and_then(Value::as_str),
        node.get("Type").and_then(Value::as_str),
    ) {
        sensors.push(LhmSensor {
            sensor_id: sensor_id.to_owned(),
            hardware_id: own_id.to_owned(),
            hardware_name: own_name.to_owned(),
            text: node
                .get("Text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            kind: kind.to_owned(),
            value: numeric(node.get("RawValue")),
        });
    }

    if let Some(children) = node.get("Children").and_then(Value::as_array) {
        for child in children {
            walk(child, own_id, own_name, sensors);
        }
    }
}

/// LHM serialises `RawValue` as a string carrying its unit (`"3.0 %"`,
/// `"1.2 MB/s"`, `"45.5 W"`), despite typing it float. Values it could not read
/// arrive as the literal `"NaN"`.
fn numeric(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64().filter(|number| number.is_finite()),
        Value::String(text) => {
            let text = text.trim_start();
            // The number, then whatever unit follows it.
            let end = text
                .char_indices()
                .find(|(_, character)| {
                    !(character.is_ascii_digit()
                        || matches!(character, '-' | '+' | '.' | 'e' | 'E'))
                })
                .map_or(text.len(), |(at, _)| at);
            text[..end]
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_carrying_string_is_its_leading_number() {
        let payload = serde_json::json!({
            "Children": [{
                "Text": "NVIDIA GeForce RTX 4090",
                "HardwareId": "/gpu-nvidia/0",
                "Children": [{
                    "Text": "GPU Core",
                    "SensorId": "/gpu-nvidia/0/temperature/0",
                    "Type": "Temperature",
                    "RawValue": "45.5 °C",
                    "Children": []
                }, {
                    "Text": "GPU Memory",
                    "SensorId": "/gpu-nvidia/0/smalldata/1",
                    "Type": "SmallData",
                    "RawValue": "1024.0 MB",
                    "Children": []
                }, {
                    "Text": "GPU Fan",
                    "SensorId": "/gpu-nvidia/0/fan/0",
                    "Type": "Fan",
                    "RawValue": "NaN",
                    "Children": []
                }]
            }]
        });
        let sensors = flatten(&payload);
        assert_eq!(sensors.len(), 3);
        assert_eq!(sensors[0].value, Some(45.5));
        assert_eq!(sensors[0].hardware_id, "/gpu-nvidia/0");
        assert_eq!(sensors[0].hardware_name, "NVIDIA GeForce RTX 4090");
        assert_eq!(sensors[1].value, Some(1024.0));
        // The fan LHM could not read is a gap, not a zero.
        assert_eq!(sensors[2].value, None);
    }

    #[test]
    fn a_payload_that_is_not_a_tree_has_no_sensors() {
        assert!(flatten(&serde_json::json!({"Text": "Libre Hardware Monitor"})).is_empty());
        assert!(flatten(&serde_json::json!(null)).is_empty());
    }
}
