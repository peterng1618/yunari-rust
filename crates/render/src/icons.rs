//! The shared icon set: Lucide's glyph geometry as path data.
//!
//! Lucide is drawn on a 24-unit square, so a glyph becomes path data scaled to
//! the box it is placed in, and the stroke scales with it. The geometry is
//! copied from `lucide-react` v1.61.0 (ISC) — see the repository's notice.
//!
//! A scene holds geometry, not components, so the node arrays are carried as
//! data and turned into the same path data an author would write.

use crate::display_list::PathCommand;
use crate::path;

type Nodes = &'static [(&'static str, &'static [(&'static str, &'static str)])];

/// One glyph: path data scaled to `size`, and the stroke width that belongs
/// with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub data: String,
    pub stroke_width: f64,
}

/// The glyph `name` at `size` artboard units, or `None` when the set has no
/// such name.
pub fn glyph(name: &str, size: f64) -> Option<Glyph> {
    let nodes = nodes_of(name)?;
    let authored = geometry(nodes);
    let commands = path::parse(&authored).ok()?;
    let scale = size / 24.0;
    Some(Glyph {
        data: scaled(&commands, scale),
        stroke_width: 2.0 * scale,
    })
}

/// Every name the set answers.
pub fn names() -> &'static [&'static str] {
    &[
        "cpu",
        "gpu",
        "ram",
        "vram",
        "trends",
        "storage",
        "network",
        "chevron",
        "thermometer",
        "fan",
        "gauge",
        "activity",
        "power",
        "clock",
        "zap",
        "arrow-up",
        "arrow-down",
    ]
}

fn nodes_of(name: &str) -> Option<Nodes> {
    Some(match name {
        "cpu" => CPU,
        "gpu" => CIRCUITBOARD,
        "ram" | "vram" => MEMORYSTICK,
        "trends" => CHARTCOLUMNS,
        "storage" => HARDDRIVE,
        "network" => WIFI,
        "chevron" => CHEVRONRIGHT,
        "thermometer" => THERMOMETER,
        "fan" => FAN,
        "gauge" => GAUGE,
        "activity" => ACTIVITY,
        "power" => POWER,
        "clock" => CLOCK,
        "zap" => ZAP,
        "arrow-up" => ARROWUP,
        "arrow-down" => ARROWDOWN,
        _ => return None,
    })
}

/// The glyph in Lucide's own 24-unit space, as one path-data string.
fn geometry(nodes: Nodes) -> String {
    let mut data = String::new();
    for (tag, attrs) in nodes {
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| value.parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        match *tag {
            "path" => {
                if let Some((_, d)) = attrs.iter().find(|(name, _)| *name == "d") {
                    data.push_str(d);
                    data.push(' ');
                }
            }
            "rect" => {
                let (x, y, width, height) = (attr("x"), attr("y"), attr("width"), attr("height"));
                let radius = attr("rx")
                    .max(attr("ry"))
                    .min(width / 2.0)
                    .min(height / 2.0);
                data.push_str(&format!(
                    "M{} {} h{} a{radius} {radius} 0 0 1 {radius} {radius} v{} a{radius} {radius} 0 0 1 -{radius} {radius} h-{} a{radius} {radius} 0 0 1 -{radius} -{radius} v-{} a{radius} {radius} 0 0 1 {radius} -{radius} z ",
                    x + radius,
                    y,
                    width - radius * 2.0,
                    height - radius * 2.0,
                    width - radius * 2.0,
                    height - radius * 2.0,
                ));
            }
            "circle" => {
                let (cx, cy, r) = (attr("cx"), attr("cy"), attr("r"));
                data.push_str(&format!(
                    "M{} {} a{r} {r} 0 1 0 {} 0 a{r} {r} 0 1 0 -{} 0 z ",
                    cx - r,
                    cy,
                    r * 2.0,
                    r * 2.0,
                ));
            }
            _ => {}
        }
    }
    data
}

/// Absolute path data, every coordinate and control point multiplied by
/// `scale`, so the glyph is drawn at the size the caller asked for.
fn scaled(commands: &[PathCommand], scale: f64) -> String {
    let mut data = String::new();
    let number = |value: f32| value as f64 * scale;
    for command in commands {
        match *command {
            PathCommand::MoveTo(x, y) => {
                data.push_str(&format!("M{:.3} {:.3} ", number(x), number(y)));
            }
            PathCommand::LineTo(x, y) => {
                data.push_str(&format!("L{:.3} {:.3} ", number(x), number(y)));
            }
            PathCommand::QuadTo(x1, y1, x, y) => {
                data.push_str(&format!(
                    "Q{:.3} {:.3} {:.3} {:.3} ",
                    number(x1),
                    number(y1),
                    number(x),
                    number(y)
                ));
            }
            PathCommand::CubicTo(x1, y1, x2, y2, x, y) => {
                data.push_str(&format!(
                    "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} ",
                    number(x1),
                    number(y1),
                    number(x2),
                    number(y2),
                    number(x),
                    number(y)
                ));
            }
            PathCommand::Close => data.push_str("Z "),
        }
    }
    data.trim_end().to_owned()
}

const CHARTCOLUMNS: Nodes = &[
    ("path", &[("d", "M5 21v-6")]),
    ("path", &[("d", "M12 21V3")]),
    ("path", &[("d", "M19 21V9")]),
];
const CHEVRONRIGHT: Nodes = &[("path", &[("d", "m9 18 6-6-6-6")])];
const CIRCUITBOARD: Nodes = &[
    (
        "rect",
        &[
            ("width", "18"),
            ("height", "18"),
            ("x", "3"),
            ("y", "3"),
            ("rx", "2"),
        ],
    ),
    ("path", &[("d", "M11 9h4a2 2 0 0 0 2-2V3")]),
    ("circle", &[("cx", "9"), ("cy", "9"), ("r", "2")]),
    ("path", &[("d", "M7 21v-4a2 2 0 0 1 2-2h4")]),
    ("circle", &[("cx", "15"), ("cy", "15"), ("r", "2")]),
];
const CPU: Nodes = &[
    ("path", &[("d", "M12 20v2")]),
    ("path", &[("d", "M12 2v2")]),
    ("path", &[("d", "M17 20v2")]),
    ("path", &[("d", "M17 2v2")]),
    ("path", &[("d", "M2 12h2")]),
    ("path", &[("d", "M2 17h2")]),
    ("path", &[("d", "M2 7h2")]),
    ("path", &[("d", "M20 12h2")]),
    ("path", &[("d", "M20 17h2")]),
    ("path", &[("d", "M20 7h2")]),
    ("path", &[("d", "M7 20v2")]),
    ("path", &[("d", "M7 2v2")]),
    (
        "rect",
        &[
            ("x", "4"),
            ("y", "4"),
            ("width", "16"),
            ("height", "16"),
            ("rx", "2"),
        ],
    ),
    (
        "rect",
        &[
            ("x", "8"),
            ("y", "8"),
            ("width", "8"),
            ("height", "8"),
            ("rx", "1"),
        ],
    ),
];
const HARDDRIVE: Nodes = &[
    ("path", &[("d", "M10 16h.01")]),
    (
        "path",
        &[(
            "d",
            "M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
        )],
    ),
    ("path", &[("d", "M21.946 12.013H2.054")]),
    ("path", &[("d", "M6 16h.01")]),
];
const MEMORYSTICK: Nodes = &[
    ("path", &[("d", "M12 12v-2")]),
    ("path", &[("d", "M12 18v-2")]),
    ("path", &[("d", "M16 12v-2")]),
    ("path", &[("d", "M16 18v-2")]),
    ("path", &[("d", "M2 11h1.5")]),
    ("path", &[("d", "M20 18v-2")]),
    ("path", &[("d", "M20.5 11H22")]),
    ("path", &[("d", "M4 18v-2")]),
    ("path", &[("d", "M8 12v-2")]),
    ("path", &[("d", "M8 18v-2")]),
    (
        "rect",
        &[
            ("x", "2"),
            ("y", "6"),
            ("width", "20"),
            ("height", "10"),
            ("rx", "2"),
        ],
    ),
];
const WIFI: Nodes = &[
    ("path", &[("d", "M12 20h.01")]),
    ("path", &[("d", "M2 8.82a15 15 0 0 1 20 0")]),
    ("path", &[("d", "M5 12.859a10 10 0 0 1 14 0")]),
    ("path", &[("d", "M8.5 16.429a5 5 0 0 1 7 0")]),
];
const THERMOMETER: Nodes = &[(
    "path",
    &[("d", "M14 4v10.54a4 4 0 1 1-4 0V4a2 2 0 0 1 4 0Z")],
)];
const FAN: Nodes = &[
    (
        "path",
        &[(
            "d",
            "M10.827 16.379a6.082 6.082 0 0 1-8.618-7.002l5.412 1.45a6.082 6.082 0 0 1 7.002-8.618l-1.45 5.412a6.082 6.082 0 0 1 8.618 7.002l-5.412-1.45a6.082 6.082 0 0 1-7.002 8.618l1.45-5.412Z",
        )],
    ),
    ("path", &[("d", "M12 12v.01")]),
];
const GAUGE: Nodes = &[
    ("path", &[("d", "m12 14 4-4")]),
    ("path", &[("d", "M3.34 19a10 10 0 1 1 17.32 0")]),
];
const ACTIVITY: Nodes = &[(
    "path",
    &[(
        "d",
        "M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2",
    )],
)];
const POWER: Nodes = &[
    ("path", &[("d", "M12 2v10")]),
    ("path", &[("d", "M18.4 6.6a9 9 0 1 1-12.77.04")]),
];
const CLOCK: Nodes = &[
    ("circle", &[("cx", "12"), ("cy", "12"), ("r", "10")]),
    ("path", &[("d", "M12 6v6l4 2")]),
];
const ZAP: Nodes = &[(
    "path",
    &[(
        "d",
        "M15.914 4a1.5 1.5 0 00-2.474-1.561l-9 9A1.5 1.5 0 005.5 14h4.002a.5.5 0 01.471.666L8.086 20a1.5 1.5 0 002.475 1.56l9-9A1.5 1.5 0 0018.5 10h-3.997a.5.5 0 01-.472-.667z",
    )],
)];
const ARROWUP: Nodes = &[
    ("path", &[("d", "m5 12 7-7 7 7")]),
    ("path", &[("d", "M12 19V5")]),
];
const ARROWDOWN: Nodes = &[
    ("path", &[("d", "M12 5v14")]),
    ("path", &[("d", "m19 12-7 7-7-7")]),
];
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_the_set_offers_draws() {
        for name in names() {
            let glyph = glyph(name, 24.0).unwrap_or_else(|| panic!("{name}"));
            assert!(glyph.data.starts_with('M'), "{name}: {}", glyph.data);
            assert!(glyph.stroke_width > 0.0, "{name}");
        }
    }

    #[test]
    fn a_glyph_scales_with_its_box() {
        let small = glyph("cpu", 24.0).expect("cpu");
        let large = glyph("cpu", 48.0).expect("cpu");
        assert!((large.stroke_width - small.stroke_width * 2.0).abs() < 1e-9);
        assert_ne!(large.data, small.data);
        assert!(glyph("no-such-icon", 24.0).is_none());
    }

    #[test]
    fn a_circle_and_a_round_rect_become_arcs_the_renderer_draws() {
        // `gpu` carries a rounded rect and two circles; both must parse.
        let glyph = glyph("gpu", 24.0).expect("gpu");
        assert!(
            glyph.data.contains("C"),
            "arcs become cubics: {}",
            glyph.data
        );
    }
}
