//! The families this host can drive, and what a driver must look like.

use std::time::Duration;

/// A panel's wire geometry, in pixels, as the device expects them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
}

/// Who made a panel and which firmware has been seen answering on it. An empty
/// firmware list means nobody has measured one on that model yet; it is written
/// as empty rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelSupport {
    pub maker: &'static str,
    pub hardware_revision: &'static str,
    pub supported_firmware: &'static [&'static str],
    pub updated_at: &'static str,
}

/// One control a panel accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Brightness,
    Screen,
    PanelSettings,
    Restart,
    Status,
    Hello,
}

impl CommandKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CommandKind::Brightness => "brightness",
            CommandKind::Screen => "screen",
            CommandKind::PanelSettings => "panel-settings",
            CommandKind::Restart => "restart",
            CommandKind::Status => "status",
            CommandKind::Hello => "hello",
        }
    }
}

/// A control, with its payload already validated by the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelCommand {
    Brightness(u8),
    Screen(bool),
    PanelSettings {
        start_mode: u8,
        flip: bool,
        sleep_delay: u8,
    },
    Restart,
    Status,
    Hello,
}

impl PanelCommand {
    pub fn kind(self) -> CommandKind {
        match self {
            PanelCommand::Brightness(_) => CommandKind::Brightness,
            PanelCommand::Screen(_) => CommandKind::Screen,
            PanelCommand::PanelSettings { .. } => CommandKind::PanelSettings,
            PanelCommand::Restart => CommandKind::Restart,
            PanelCommand::Status => CommandKind::Status,
            PanelCommand::Hello => CommandKind::Hello,
        }
    }
}

/// A panel whose picture is a rectangle accepts quarter turns only; the round
/// Kraken's glass takes any angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelShape {
    Rect,
    Circle,
}

#[derive(Debug, Clone, Copy)]
pub struct PanelFamily {
    pub id: &'static str,
    pub label: &'static str,
    pub vendor_id: u16,
    pub product_id: u16,
    pub support: PanelSupport,
    pub commands: &'static [CommandKind],
    pub geometry: Geometry,
    pub shape: PanelShape,
}

impl PanelFamily {
    pub fn usb_id(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

/// One panel attached right now, as a record can hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelHint {
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial: String,
    pub label: String,
    pub device_id: Option<String>,
}

/// Every family this host can drive.
pub fn families() -> &'static [PanelFamily] {
    &FAMILIES
}

pub fn for_device(vendor_id: u16, product_id: u16) -> Option<&'static PanelFamily> {
    families()
        .iter()
        .find(|family| family.vendor_id == vendor_id && family.product_id == product_id)
}

static FAMILIES: [PanelFamily; 4] = [
    PanelFamily {
        id: "lianli-9-2",
        label: "Lian Li 9.2\" LCD",
        vendor_id: 0x1cbe,
        product_id: 0xa092,
        support: PanelSupport {
            maker: "Lian Li",
            hardware_revision: "9.2\"",
            supported_firmware: &["lianli92_0001_2026041401"],
            updated_at: "2026-10-31",
        },
        commands: &[
            CommandKind::Brightness,
            CommandKind::Screen,
            CommandKind::Status,
        ],
        geometry: Geometry {
            width: 464,
            height: 1920,
        },
        shape: PanelShape::Rect,
    },
    PanelFamily {
        id: "lianli-8-8",
        label: "Lian Li Universal Screen 8.8\"",
        vendor_id: 0x1cbe,
        product_id: 0xa088,
        support: PanelSupport {
            maker: "Lian Li",
            hardware_revision: "8.8\"",
            supported_firmware: &[],
            updated_at: "2026-10-28",
        },
        commands: &[
            CommandKind::Brightness,
            CommandKind::Screen,
            CommandKind::Status,
        ],
        geometry: Geometry {
            width: 480,
            height: 1920,
        },
        shape: PanelShape::Rect,
    },
    PanelFamily {
        id: "turzx-5",
        label: "TURZX 5\" Panel",
        vendor_id: 0x1d6b,
        product_id: 0x0106,
        support: PanelSupport {
            maker: "TURZX",
            hardware_revision: "Rev C",
            supported_firmware: &["1.88"],
            updated_at: "2026-10-29",
        },
        commands: &[
            CommandKind::Brightness,
            CommandKind::Screen,
            CommandKind::PanelSettings,
            CommandKind::Restart,
            CommandKind::Status,
            CommandKind::Hello,
        ],
        geometry: Geometry {
            width: 800,
            height: 480,
        },
        shape: PanelShape::Rect,
    },
    PanelFamily {
        id: "kraken-z3",
        label: "NZXT Kraken Z (Z53/Z63/Z73)",
        vendor_id: 0x1e71,
        product_id: 0x3008,
        support: PanelSupport {
            maker: "NZXT",
            hardware_revision: "Z53/Z63/Z73",
            supported_firmware: &["5.11.0"],
            updated_at: "2026-11-06",
        },
        commands: &[
            CommandKind::Brightness,
            CommandKind::Screen,
            CommandKind::Status,
        ],
        geometry: Geometry {
            width: 320,
            height: 320,
        },
        shape: PanelShape::Circle,
    },
];

/// What a driver writes a still as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillFormat {
    Jpeg,
    Png,
    /// Straight RGBA of the panel's own geometry, for a device that reorders
    /// pixels itself.
    Rgba,
}

/// One channel of LEDs a device drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LightingChannel {
    pub name: &'static str,
    pub leds: u32,
}

/// How the LEDs are laid out, for a sender that wants to paint them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LightingLayout {
    pub shape: &'static str,
    pub width: u32,
    pub height: u32,
    pub count: u32,
}

/// The LEDs a device drives: one frame is `led_count` RGB triples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lighting {
    pub led_count: u32,
    pub channels: &'static [LightingChannel],
    pub layout: LightingLayout,
}

/// Which controls a device accepts besides its own picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Controls {
    pub pump: bool,
    pub fans: bool,
    pub telemetry: bool,
}

/// What a device reports about itself, in the units its own reply carries.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub liquid_temp_c: f64,
    pub pump_rpm: u32,
    pub pump_percent: u8,
    pub fan_rpm: u32,
    pub fan_percent: u8,
}

/// One panel, open or not. Every driver is generic over its transport so a test
/// can hand it a fake and read back exactly what it wrote.
pub trait PanelDevice: Send {
    fn geometry(&self) -> Geometry;

    fn open(&mut self) -> Result<(), String>;

    fn present(&mut self, still: &[u8]) -> Result<(), String>;

    fn still_format(&self) -> StillFormat {
        StillFormat::Jpeg
    }

    fn frame_interval_ms(&self) -> u32 {
        1_000
    }

    /// Whether this device accepts this control at all.
    fn accepts(&self, kind: CommandKind) -> bool {
        let _ = kind;
        false
    }

    fn run(&mut self, command: PanelCommand) -> Result<String, String>;

    fn close(&mut self) -> Result<(), String>;

    /// The LEDs this device drives, when it drives any.
    fn lighting(&self) -> Option<Lighting> {
        None
    }

    /// Writes one frame of RGB triples, `led_count` of them.
    fn write_colors(&mut self, rgb: &[u8]) -> Result<(), String> {
        let _ = rgb;
        Err("That device has no lighting.".to_owned())
    }

    /// The controls beyond the picture this device accepts.
    fn controls(&self) -> Option<Controls> {
        None
    }

    /// Sets the pump and fan levels, each one already a whole percent.
    fn set_controls(&mut self, pump: Option<f64>, fan: Option<f64>) -> Result<(), String> {
        let _ = (pump, fan);
        Err("That device accepts no controls.".to_owned())
    }

    /// Asks the device what it is doing right now.
    fn telemetry(&mut self) -> Result<Telemetry, String> {
        Err("That device reports no telemetry.".to_owned())
    }
}

/// A boxed driver is a driver: the manager keeps one runner per panel and only
/// the trait object's methods are ever called.
impl PanelDevice for Box<dyn PanelDevice> {
    fn geometry(&self) -> Geometry {
        (**self).geometry()
    }

    fn open(&mut self) -> Result<(), String> {
        (**self).open()
    }

    fn present(&mut self, still: &[u8]) -> Result<(), String> {
        (**self).present(still)
    }

    fn still_format(&self) -> StillFormat {
        (**self).still_format()
    }

    fn frame_interval_ms(&self) -> u32 {
        (**self).frame_interval_ms()
    }

    fn accepts(&self, kind: CommandKind) -> bool {
        (**self).accepts(kind)
    }

    fn run(&mut self, command: PanelCommand) -> Result<String, String> {
        (**self).run(command)
    }

    fn close(&mut self) -> Result<(), String> {
        (**self).close()
    }

    fn lighting(&self) -> Option<Lighting> {
        (**self).lighting()
    }

    fn write_colors(&mut self, rgb: &[u8]) -> Result<(), String> {
        (**self).write_colors(rgb)
    }

    fn controls(&self) -> Option<Controls> {
        (**self).controls()
    }

    fn set_controls(&mut self, pump: Option<f64>, fan: Option<f64>) -> Result<(), String> {
        (**self).set_controls(pump, fan)
    }

    fn telemetry(&mut self) -> Result<Telemetry, String> {
        (**self).telemetry()
    }
}

/// One attached device, as its family enumerates it.
pub fn attached() -> Vec<PanelHint> {
    let mut hints = Vec::new();
    for family in families() {
        // A family whose enumeration fails is skipped: one busy bus must not
        // hide the panels on every other family.
        let Ok(mut found) = enumerate(family) else {
            continue;
        };
        hints.append(&mut found);
    }
    dedupe(hints)
}

/// A family's devices by the ids it claims. Enumeration lives beside the family
/// because each bus answers differently.
fn enumerate(family: &PanelFamily) -> Result<Vec<PanelHint>, String> {
    let hints = match family.id {
        "lianli-9-2" | "lianli-8-8" => crate::tur_usb::attached(family),
        "turzx-5" => crate::turzx::attached(family),
        "kraken-z3" => crate::kraken::attached(family),
        _ => Vec::new(),
    };
    let _ = family.usb_id();
    Ok(hints)
}

fn dedupe(hints: Vec<PanelHint>) -> Vec<PanelHint> {
    let mut seen = std::collections::HashSet::new();
    hints
        .into_iter()
        .filter(|hint| seen.insert((hint.vendor_id, hint.product_id, hint.serial.clone())))
        .collect()
}

/// The read deadline a command waits with. Kept here so every driver agrees.
pub const COMMAND_READ_TIMEOUT: Duration = Duration::from_millis(2_000);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_has_its_own_id_and_device() {
        let mut ids: Vec<&str> = families().iter().map(|family| family.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), families().len());
        let mut devices: Vec<(u16, u16)> = families()
            .iter()
            .map(|family| (family.vendor_id, family.product_id))
            .collect();
        devices.sort_unstable();
        devices.dedup();
        assert_eq!(devices.len(), families().len());
    }

    #[test]
    fn the_declared_geometry_and_controls_match_the_hardware() {
        let kraken = for_device(0x1e71, 0x3008).expect("the Kraken");
        assert_eq!(
            kraken.geometry,
            Geometry {
                width: 320,
                height: 320
            }
        );
        assert_eq!(kraken.shape, PanelShape::Circle);
        assert_eq!(kraken.support.maker, "NZXT");
        let lianli = for_device(0x1cbe, 0xa092).expect("the 9.2 inch");
        assert_eq!(
            lianli.geometry,
            Geometry {
                width: 464,
                height: 1920
            }
        );
        assert_eq!(
            lianli.support.supported_firmware,
            &["lianli92_0001_2026041401"]
        );
        let turzx = for_device(0x1d6b, 0x0106).expect("the TURZX");
        assert_eq!(
            turzx.geometry,
            Geometry {
                width: 800,
                height: 480
            }
        );
        assert!(turzx.commands.contains(&CommandKind::Hello));
        assert_eq!(
            for_device(0x1cbe, 0xa088)
                .expect("the 8.8 inch")
                .support
                .supported_firmware,
            &[] as &[&str]
        );
    }

    #[test]
    fn an_unknown_device_names_no_family() {
        assert!(for_device(0x1234, 0x5678).is_none());
        assert!(for_device(0x1cbe, 0xa093).is_none());
    }

    #[test]
    fn a_command_knows_its_control() {
        assert_eq!(PanelCommand::Brightness(40).kind(), CommandKind::Brightness);
        assert_eq!(PanelCommand::Screen(false).kind(), CommandKind::Screen);
        assert_eq!(PanelCommand::Restart.kind(), CommandKind::Restart);
        assert_eq!(CommandKind::PanelSettings.as_str(), "panel-settings");
    }
}
