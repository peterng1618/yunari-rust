//! TURZX 5" panels: a serial port at 115200 baud with 250-byte frames.

use crate::family::{PanelFamily, PanelHint};

/// The TURZX panels attached right now. A port is claimed only after it answers
/// HELLO, so a port that cannot be opened lists nothing.
pub fn attached(family: &PanelFamily) -> Vec<PanelHint> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .filter_map(|port| {
            let serialport::SerialPortType::UsbPort(usb) = &port.port_type else {
                return None;
            };
            (usb.vid == family.vendor_id && usb.pid == family.product_id).then(|| PanelHint {
                vendor_id: family.vendor_id,
                product_id: family.product_id,
                serial: usb.serial_number.clone().unwrap_or_default(),
                label: family.label.to_owned(),
                device_id: Some(port.port_name),
            })
        })
        .collect()
}
