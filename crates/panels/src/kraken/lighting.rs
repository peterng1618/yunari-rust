//! The Kraken Z's lighting and fan reports, taken from SignalRGB's shipped
//! `Nzxt_Kraken_Z3_AIO.js` (firmware 5.11.0).
//!
//! Interface 1 takes these as 64-byte HID reports, the same control channel the
//! LCD commands use, so every writer here goes through the panel's one handle.
//! The LED payload is GRB: the plugin calls `getColors("Inline", "GRB")`, so the
//! wire wants green before red even though a colour frame carries RGB triples.

use crate::family::{Lighting, LightingChannel, LightingLayout, Telemetry};

/// One channel of 40 LEDs.
pub const KRAKEN_LED_COUNT: u32 = 40;

/// The pump's firmware floor: a lower requested value is raised to it.
const PUMP_MINIMUM_PERCENT: u8 = 25;
/// One report carries 60 colour bytes: 20 LEDs.
const LEDS_PER_REPORT: usize = 20;
/// The bytes one 40-LED frame occupies: 120 RGB components.
pub const FRAME_COMPONENTS: usize = KRAKEN_LED_COUNT as usize * 3;

const CHANNELS: [LightingChannel; 1] = [LightingChannel {
    name: "Kraken Z",
    leds: KRAKEN_LED_COUNT,
}];

/// The Kraken's LEDs: one channel of 40, laid out as three rings of eight.
pub fn lighting() -> Lighting {
    Lighting {
        led_count: KRAKEN_LED_COUNT,
        channels: &CHANNELS,
        // Three AER RGB fans are daisy-chained to the pump's 40-LED channel,
        // each a ring of 8 LEDs; the remaining slots stay empty and black.
        layout: LightingLayout {
            shape: "rings",
            width: 3,
            height: 3,
            count: 3,
        },
    }
}

/// A whole percent from 0 to 100. Refuses a value no byte can represent rather
/// than writing a fraction the caller did not ask for.
pub fn whole_percent(value: f64) -> Result<u8, String> {
    if !value.is_finite() {
        return Err("A fan or pump percent must be a finite number.".to_owned());
    }
    Ok(value.round().clamp(0.0, 100.0) as u8)
}

/// The pump's report level: the requested whole percent, but never below the
/// firmware's 25% floor.
pub fn pump_percent(value: f64) -> Result<u8, String> {
    Ok(whole_percent(value)?.max(PUMP_MINIMUM_PERCENT))
}

/// The fan report level; fans may stop (0%).
pub fn fan_percent(value: f64) -> Result<u8, String> {
    whole_percent(value)
}

/// The two `22 10` / `22 11` data reports for one frame of RGB triples.
pub fn lighting_reports(rgb: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if rgb.len() != FRAME_COMPONENTS {
        return Err(format!(
            "A Kraken lighting frame is {FRAME_COMPONENTS} RGB bytes; received {}.",
            rgb.len()
        ));
    }
    let mut reports = Vec::with_capacity(2);
    for packet in 0..2_u8 {
        let mut report = vec![0x22, 0x10 | packet, 0x01, 0x00];
        let first = usize::from(packet) * LEDS_PER_REPORT;
        for led in first..first + LEDS_PER_REPORT {
            let at = led * 3;
            // GRB: the device's channel order, not the frame's.
            report.extend_from_slice(&[rgb[at + 1], rgb[at], rgb[at + 2]]);
        }
        reports.push(report);
    }
    Ok(reports)
}

/// The `22 A0` commit that shows the data written by the two reports.
pub fn lighting_commit() -> Vec<u8> {
    vec![
        0x22, 0xa0, 0x01, 0x00, 0x01, 0x00, 0x00, 0x28, 0x00, 0x00, 0x80, 0x00, 0x32, 0x00, 0x00,
        0x01,
    ]
}

/// `72 01`: the pump's 40 fan-slot percentages.
pub fn pump_report(percent: f64) -> Result<Vec<u8>, String> {
    fan_report_of(0x01, pump_percent(percent)?)
}

/// `72 02`: the fan's 40 percentages.
pub fn fan_report(percent: f64) -> Result<Vec<u8>, String> {
    fan_report_of(0x02, fan_percent(percent)?)
}

fn fan_report_of(command: u8, level: u8) -> Result<Vec<u8>, String> {
    let mut report = vec![0x72, command, 0x00, 0x00];
    report.extend(std::iter::repeat_n(level, 40));
    Ok(report)
}

/// The fields of a `75 01` status reply: liquid temperature at 15 (whole °C)
/// and 16 (tenths), pump RPM 18:17, pump percent 19, fan RPM 24:23, fan
/// percent 25.
pub fn parse_status(report: &[u8]) -> Result<Telemetry, String> {
    if report.len() < 26 {
        return Err(format!(
            "A Kraken status reply is at least 26 bytes; received {}.",
            report.len()
        ));
    }
    Ok(Telemetry {
        liquid_temp_c: f64::from(report[15]) + f64::from(report[16]) / 10.0,
        pump_rpm: (u32::from(report[18]) << 8) | u32::from(report[17]),
        pump_percent: report[19],
        fan_rpm: (u32::from(report[24]) << 8) | u32::from(report[23]),
        fan_percent: report[25],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_two_grb_reports_and_a_commit() {
        let mut rgb = vec![0_u8; FRAME_COMPONENTS];
        // LED 0 is red, LED 20 is blue.
        rgb[0] = 255;
        rgb[20 * 3 + 2] = 255;
        let reports = lighting_reports(&rgb).expect("two reports");
        assert_eq!(reports.len(), 2);
        assert_eq!(&reports[0][..4], &[0x22, 0x10, 0x01, 0x00]);
        assert_eq!(&reports[1][..4], &[0x22, 0x11, 0x01, 0x00]);
        // GRB: red first LED becomes G=0, R=255, B=0.
        assert_eq!(&reports[0][4..7], &[0, 255, 0]);
        // The second report starts at LED 20: blue becomes G=0, R=0, B=255.
        assert_eq!(&reports[1][4..7], &[0, 0, 255]);
        assert_eq!(lighting_commit()[..2], [0x22, 0xa0]);
        assert!(lighting_reports(&[0; 6]).unwrap_err().contains("120"));
    }

    #[test]
    fn the_pump_keeps_its_firmware_floor_and_fans_may_stop() {
        assert_eq!(pump_percent(10.0).unwrap(), 25);
        assert_eq!(pump_percent(250.0).unwrap(), 100);
        assert_eq!(fan_percent(0.0).unwrap(), 0);
        assert!(pump_percent(f64::NAN).is_err());
        let pump = pump_report(10.0).unwrap();
        assert_eq!(&pump[..2], &[0x72, 0x01]);
        assert_eq!(pump.len(), 44);
        assert!(pump[4..].iter().all(|slot| *slot == 25));
    }

    #[test]
    fn a_status_reply_is_read_at_the_shipped_offsets() {
        let mut reply = vec![0_u8; 26];
        reply[0] = 0x75;
        reply[1] = 0x01;
        reply[15] = 30;
        reply[16] = 5;
        reply[17] = 0x10;
        reply[18] = 0x27;
        reply[19] = 60;
        reply[23] = 0x40;
        reply[24] = 0x1f;
        reply[25] = 45;
        let status = parse_status(&reply).expect("a status");
        assert!((status.liquid_temp_c - 30.5).abs() < f64::EPSILON);
        assert_eq!(status.pump_rpm, 10_000);
        assert_eq!(status.pump_percent, 60);
        assert_eq!(status.fan_rpm, 8_000);
        assert_eq!(status.fan_percent, 45);
        assert!(parse_status(&reply[..20]).is_err());
    }
}
