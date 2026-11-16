#![forbid(unsafe_code)]
//! USB and serial panels: the families this host drives, one driver each, and
//! the runner that feeds them frames.

pub mod family;
pub mod kraken;
pub mod manager;
pub mod runner;
pub mod signalrgb;
pub mod tur_usb;
pub mod turzx;

pub use family::{
    CommandKind, Geometry, PanelCommand, PanelDevice, PanelFamily, PanelHint, PanelShape,
    RenderMode, StillFormat, families, for_device,
};

/// The panels of this family attached right now.
pub fn attached(family: &PanelFamily) -> Vec<PanelHint> {
    match family.id {
        "lianli-9-2" | "lianli-8-8" => tur_usb::attached(family),
        "kraken-z3" => kraken::attached(family),
        "turzx-5" => turzx::attached(family),
        _ => Vec::new(),
    }
}

/// Open the attached device of this family.
///
/// Refused with a sentence rather than a bare failure when none is attached,
/// because "not plugged in" and "another program holds it" are the two failures
/// an owner meets and they need different answers.
pub fn open_device(family: &PanelFamily) -> Result<Box<dyn PanelDevice>, String> {
    match family.id {
        "lianli-9-2" | "lianli-8-8" => {
            let Some(bulk) = tur_usb::device::UsbBulk::find(family.vendor_id, family.product_id)
            else {
                return Err(format!(
                    "{} is not attached. Check the panel's USB cable.",
                    family.label
                ));
            };
            let mut panel = tur_usb::device::TurUsbPanel::new(bulk, family.label, family.geometry);
            panel.open().map_err(|error| {
                let _ = panel.close();
                format!(
                    "{} could not be opened: {error}. Another program holding it — L-Connect 3 is the usual one — has to be closed first.",
                    family.label
                )
            })?;
            Ok(Box::new(panel))
        }
        "kraken-z3" => {
            let mut panel = kraken::transport::open(family)?;
            panel.open().map_err(|error| {
                let _ = panel.close();
                format!(
                    "{} could not be opened: {error}. NZXT CAM holding the LCD is the usual reason.",
                    family.label
                )
            })?;
            Ok(Box::new(panel))
        }
        "turzx-5" => Ok(Box::new(turzx::open(family)?)),
        _ => Err(format!(
            "No driver on this host claims {}.",
            family.usb_id()
        )),
    }
}
