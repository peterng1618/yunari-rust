//! Lian Li / TUR USB panels: one vendor interface, 512-byte encrypted command
//! frames, one JPEG per still.

use rusb::UsbContext;

pub mod device;
pub mod protocol;

use crate::family::{PanelFamily, PanelHint};

/// The panels of this family attached right now.
pub fn attached(family: &PanelFamily) -> Vec<PanelHint> {
    let Ok(devices) = rusb::devices() else {
        return Vec::new();
    };
    devices
        .iter()
        .filter_map(|device| {
            let descriptor = device.device_descriptor().ok()?;
            if descriptor.vendor_id() != family.vendor_id
                || descriptor.product_id() != family.product_id
            {
                return None;
            }
            Some(PanelHint {
                vendor_id: family.vendor_id,
                product_id: family.product_id,
                serial: device
                    .open()
                    .ok()
                    .and_then(|handle| handle.read_serial_number_string_ascii(&descriptor).ok())
                    .unwrap_or_default(),
                label: family.label.to_owned(),
                device_id: None,
            })
        })
        .collect()
}
