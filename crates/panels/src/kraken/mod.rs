//! NZXT Kraken Z53/Z63/Z73: HID control reports and a bulk pixel stream.

pub mod device;
pub mod transport;

use crate::family::{PanelFamily, PanelHint};

/// The Krakens attached right now.
pub fn attached(family: &PanelFamily) -> Vec<PanelHint> {
    let Ok(api) = hidapi::HidApi::new() else {
        return Vec::new();
    };
    api.device_list()
        .filter(|device| {
            device.vendor_id() == family.vendor_id && device.product_id() == family.product_id
        })
        .map(|device| PanelHint {
            vendor_id: family.vendor_id,
            product_id: family.product_id,
            // A blank serial is not a serial; the record keeps it empty rather
            // than storing a placeholder that would never match again.
            serial: device.serial_number().unwrap_or_default().trim().to_owned(),
            label: family.label.to_owned(),
            device_id: None,
        })
        .collect()
}
