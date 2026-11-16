//! The two real transports a Kraken is reached through: HIDAPI for control and
//! libusb for the pixel stream.

use std::time::Duration;

use super::device::{BulkStream, Hid, KrakenZ3, REPORT_BYTES};
use crate::family::{PanelFamily, StillFormat};

/// The bulk endpoint the glass's pixels go to, on interface 0.
const ENDPOINT: u8 = 0x02;

pub struct HidReport {
    device: hidapi::HidDevice,
}

impl Hid for HidReport {
    fn write(&mut self, report: &[u8; REPORT_BYTES]) -> Result<(), String> {
        self.device
            .write(report)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn read(&mut self, timeout_ms: u32) -> Result<Option<Vec<u8>>, String> {
        // A read that drew nothing is the normal answer to a command the device
        // does not reply to, not an error worth raising.
        let mut report = [0_u8; REPORT_BYTES];
        match self.device.read_timeout(&mut report, timeout_ms as i32) {
            Ok(read) if read > 0 => Ok(Some(report[..read.min(REPORT_BYTES)].to_vec())),
            Ok(_) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn close(&mut self) {}
}

pub struct KrakenBulk {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
}

impl BulkStream for KrakenBulk {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.handle
            .write_bulk(ENDPOINT, bytes, Duration::from_secs(5))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn close(&mut self) {
        self.handle.release_interface(0).ok();
    }
}

/// Open the attached Kraken of this family.
pub fn open(family: &PanelFamily) -> Result<KrakenZ3<HidReport, KrakenBulk>, String> {
    let api = hidapi::HidApi::new()
        .map_err(|error| format!("The NZXT Kraken is not attached. ({error})"))?;
    let info = api
        .device_list()
        .find(|device| {
            device.vendor_id() == family.vendor_id && device.product_id() == family.product_id
        })
        .ok_or_else(|| "The NZXT Kraken is not attached.".to_owned())?;
    let control = api
        .open_path(info.path())
        .map_err(|error| format!("The NZXT Kraken is not attached. ({error})"))?;

    let mut bulk = None;
    if let Ok(devices) = rusb::devices() {
        for device in devices.iter() {
            let Ok(descriptor) = device.device_descriptor() else {
                continue;
            };
            if descriptor.vendor_id() != family.vendor_id
                || descriptor.product_id() != family.product_id
            {
                continue;
            }
            let Ok(handle) = device.open() else {
                continue;
            };
            handle.set_active_configuration(1).ok();
            // The HID interface belongs to the class driver; only interface 0
            // is claimed here.
            if handle.claim_interface(0).is_ok() {
                bulk = Some(KrakenBulk { handle });
                break;
            }
        }
    }
    let Some(bulk) = bulk else {
        let _ = family;
        return Err(
            "The NZXT Kraken is not attached. Its pixel interface could not be claimed.".to_owned(),
        );
    };
    Ok(KrakenZ3::new(HidReport { device: control }, bulk))
}

/// The still format the family's frames are encoded in.
pub const fn still_format() -> StillFormat {
    StillFormat::Png
}
