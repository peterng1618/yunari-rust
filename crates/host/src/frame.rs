//! Turning a theme and this PC's readings into the list a phone paints.

use yunari_model::document::ThemeDocument;
use yunari_providers::registry::Cycle;
use yunari_providers::sample::Reading;
use yunari_render::{DisplayList, Fonts, Readings};

/// What a display shows when no provider on this PC can read a key.
pub const MISSING_MESSAGE: &str = "no provider on this PC reports that sensor; check the device assignment or that its source is running";

/// The readings a display needs: every key it asked for, with a gap that says
/// why when nothing on this PC reads it.
pub fn readings(cycle: &Cycle, keys: &[String]) -> Vec<Reading> {
    let mut readings = Vec::with_capacity(keys.len());
    for key in keys {
        match cycle.reading(key) {
            Some(reading) => readings.push(reading.clone()),
            None => readings.push(Reading::missing(key.clone(), MISSING_MESSAGE)),
        }
    }
    readings
}

pub fn readings_for(cycle: &Cycle, keys: &[String]) -> Readings {
    Readings::new(readings(cycle, keys))
}

/// The geometry a phone draws, built by the same planner the window and the
/// panels use.
pub fn frame(document: &ThemeDocument, readings: &Readings, fonts: &mut Fonts) -> DisplayList {
    let (list, _issues) = yunari_render::plan::build_with(
        document,
        fonts,
        readings,
        &yunari_providers::units::UnitPreferences::default(),
    );
    list
}

/// The sample a batch carries: the reading's own fields, and the host's
/// sensor id so a display can say which sensor it asked about.
pub fn sample_json(reading: &Reading) -> serde_json::Value {
    let status = match reading.status {
        yunari_providers::sample::Status::Ok => "ok",
        yunari_providers::sample::Status::Missing => "missing",
        yunari_providers::sample::Status::Error => "error",
    };
    serde_json::json!({
        "sensorId": format!("host:{}", reading.semantic_key),
        "semanticKey": reading.semantic_key,
        "timestamp": jiff::Timestamp::now().to_string(),
        "status": status,
        "value": reading.value,
        "unit": reading.unit,
        "text": reading.text,
        "instant": reading.instant,
        "message": reading.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_providers::sample::Reading;

    #[test]
    fn a_key_nothing_reads_is_a_gap_with_a_reason() {
        let cycle = Cycle::default();
        let readings = readings_for(&cycle, &["cpu.total".to_owned()]);
        let reading = readings.get("cpu.total").expect("a gap");
        assert_eq!(reading.message.as_deref(), Some(MISSING_MESSAGE));
        let json = sample_json(reading);
        assert_eq!(json["sensorId"], "host:cpu.total");
        assert_eq!(json["status"], "missing");
        assert!(json["value"].is_null());
    }

    #[test]
    fn a_read_key_keeps_its_figure() {
        let mut cycle = Cycle::default();
        cycle.readings.insert(
            "cpu.usage".to_owned(),
            Reading::number("cpu.usage", 12.5, "%"),
        );
        let readings = readings_for(&cycle, &["cpu.usage".to_owned()]);
        let json = sample_json(readings.get("cpu.usage").unwrap());
        assert_eq!(json["status"], "ok");
        assert_eq!(json["value"], 12.5);
        assert_eq!(json["unit"], "%");
    }
}
