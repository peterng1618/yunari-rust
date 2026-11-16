//! The colours an authored style names.
//!
//! The format stores a paint as the CSS string an author typed, so this reads
//! the forms the editor writes and refuses the rest rather than painting a
//! guess: a colour it cannot read becomes an issue with a reason.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// The colour as a backend ready `[r, g, b, a]`.
    pub fn channels(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Why a colour string is not one this format reads, or `None` when it is not
/// a colour at all.
pub fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex);
    }
    let (head, rest) = text.split_once('(')?;
    let rest = rest.strip_suffix(')')?;
    match head.trim().to_ascii_lowercase().as_str() {
        "rgb" => parse_rgb(rest, false),
        "rgba" => parse_rgb(rest, true),
        _ => None,
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digits: Vec<u8> = hex
        .chars()
        .map(|c| c.to_digit(16).map(|digit| digit as u8))
        .collect::<Option<_>>()?;
    let nibbles = match digits[..] {
        [r, g, b] => [r, r, g, g, b, b, 15, 15],
        [r, g, b, a] => [r, r, g, g, b, b, a, a],
        [r, g, b, x, y, z] => [r, g, b, x, y, z, 15, 15],
        [r, g, b, x, y, z, a, o] => [r, g, b, x, y, z, a, o],
        _ => return None,
    };
    let byte = |index: usize| nibbles[index] * 16 + nibbles[index + 1];
    Some(Color {
        r: byte(0),
        g: byte(2),
        b: byte(4),
        a: byte(6),
    })
}

fn parse_rgb(body: &str, with_alpha: bool) -> Option<Color> {
    let parts: Vec<&str> = body.split([',', '/', ' ']).filter(|part| !part.is_empty()).collect();
    let expected = if with_alpha { 4 } else { 3 };
    if parts.len() != expected {
        return None;
    }
    let channel = |index: usize| -> Option<u8> {
        let part = parts[index];
        if let Some(percent) = part.strip_suffix('%') {
            let value: f32 = percent.trim().parse().ok()?;
            return Some(clamp_channel(value / 100.0 * 255.0));
        }
        let value: f32 = part.trim().parse().ok()?;
        Some(clamp_channel(value))
    };
    let alpha = if with_alpha {
        let part = parts[3];
        let value: f32 = part.trim().parse().ok()?;
        clamp_channel(value * 255.0)
    } else {
        255
    };
    Some(Color {
        r: channel(0)?,
        g: channel(1)?,
        b: channel(2)?,
        a: alpha,
    })
}

fn clamp_channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hex_in_every_length() {
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#0f08"), Some(Color::rgb(0, 255, 0).with_alpha(136)));
        assert_eq!(
            parse_color("#00ff0080"),
            Some(Color::rgb(0, 255, 0).with_alpha(128))
        );
    }

    #[test]
    fn reads_rgb_and_rgba() {
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(
            parse_color("rgba(1, 2, 3, 0.5)"),
            Some(Color::rgb(1, 2, 3).with_alpha(128))
        );
        assert_eq!(
            parse_color("rgb(100% 0% 0%)"),
            Some(Color::rgb(255, 0, 0))
        );
    }

    #[test]
    fn refuses_what_it_does_not_read() {
        assert_eq!(parse_color("rebeccapurple"), None);
        assert_eq!(parse_color("hsl(0 100% 50%)"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgb(1, 2)"), None);
    }

    #[test]
    fn a_hex_double_reads_its_digit() {
        assert_eq!(parse_color("#abc"), Some(Color::rgb(0xaa, 0xbb, 0xcc)));
    }
}
