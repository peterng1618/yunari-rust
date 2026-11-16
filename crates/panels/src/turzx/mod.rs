//! TURZX 5" panels: a serial port at 115200 baud with 250-byte frames.

pub mod device;
pub mod overlay;
pub mod protocol;

use std::time::Duration;

use crate::family::{PanelDevice, PanelFamily, PanelHint};
use device::{LABEL, Serial, TurzxPanel};

const BAUD: u32 = 115_200;
const READ_TIMEOUT_MS: u64 = 100;

/// The TURZX panels attached right now. A port is claimed only after it answers
/// HELLO, so a port that cannot be opened lists nothing.
pub fn attached(family: &PanelFamily) -> Vec<PanelHint> {
    panel_ports(family)
        .into_iter()
        .map(|(name, serial)| PanelHint {
            vendor_id: family.vendor_id,
            product_id: family.product_id,
            serial,
            label: family.label.to_owned(),
            device_id: Some(name),
        })
        .collect()
}

/// Every open port carrying this family's ids, with its serial when it reports
/// one.
fn panel_ports(family: &PanelFamily) -> Vec<(String, String)> {
    let Ok(ports) = serialport::available_ports() else {
        return Vec::new();
    };
    ports
        .into_iter()
        .filter_map(|port| {
            let serialport::SerialPortType::UsbPort(usb) = &port.port_type else {
                return None;
            };
            (usb.vid == family.vendor_id && usb.pid == family.product_id).then(|| {
                (
                    port.port_name,
                    usb.serial_number.clone().unwrap_or_default(),
                )
            })
        })
        .collect()
}

/// An open serial port, as the panel's driver needs it.
pub struct ComPort {
    port: Box<dyn serialport::SerialPort>,
}

impl Serial for ComPort {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.port
            .write_all(bytes)
            .and_then(|()| self.port.flush())
            .map_err(|error| error.to_string())
    }

    fn read(&mut self, timeout_ms: u64) -> Result<Vec<u8>, String> {
        self.port
            .set_timeout(Duration::from_millis(timeout_ms.min(READ_TIMEOUT_MS)))
            .map_err(|error| error.to_string())?;
        let mut buffer = vec![0_u8; 4_096];
        // One `read` returns as soon as any byte lands, so a reply split across
        // transfers is gathered until the port falls quiet.
        let mut reply = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        while std::time::Instant::now() < deadline {
            match self.port.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => reply.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(reply)
    }

    fn drain(&mut self) {
        if let Ok(queued) = self.port.bytes_to_read() {
            let mut buffer = vec![0_u8; queued as usize];
            if let Ok(read) = self.port.read(&mut buffer) {
                // The bytes are stale by definition; only the amount matters.
                buffer.truncate(read);
            }
        }
    }

    fn close(&mut self) -> Result<(), String> {
        self.port
            .clear(serialport::ClearBuffer::All)
            .map_err(|error| error.to_string())
    }
}

/// Open the attached panel of this family.
pub fn open(family: &PanelFamily) -> Result<TurzxPanel<ComPort>, String> {
    for (name, _) in panel_ports(family) {
        let port = match serialport::new(&name, BAUD)
            .timeout(Duration::from_millis(READ_TIMEOUT_MS))
            .open()
        {
            Ok(port) => port,
            Err(error) => {
                let message = error.to_string();
                if is_held(&message) {
                    return Err(format!(
                        "{LABEL} could not be opened: {message}. Another program holding it —                          the TURZX vendor app is the usual one — has to be closed first."
                    ));
                }
                continue;
            }
        };
        let mut panel = TurzxPanel::new(ComPort { port }, family.geometry);
        match panel.open() {
            Ok(()) => return Ok(panel),
            // Silence is news about the panel, not about the next port: a device
            // that opened and stopped taking data needs its USB node restarted.
            Err(error) => return Err(error),
        }
    }
    Err(format!(
        "{LABEL} is not attached. Check the panel's USB cable."
    ))
}

/// The codes a held COM port answers with on Windows and Linux.
fn is_held(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("access denied")
        || lowered.contains("access is denied")
        || lowered.contains("permission denied")
        || lowered.contains("busy")
        || lowered.contains("being used")
}
