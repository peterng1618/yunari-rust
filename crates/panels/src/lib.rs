#![forbid(unsafe_code)]
//! USB and serial panels: the families this host drives, one driver each, and
//! the runner that feeds them frames.

pub mod family;
pub mod kraken;
pub mod tur_usb;
pub mod turzx;

pub use family::{
    CommandKind, Geometry, PanelCommand, PanelDevice, PanelFamily, PanelHint, PanelShape,
    StillFormat, families, for_device,
};
