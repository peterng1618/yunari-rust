//! A file from this PC, checked before it becomes a theme asset.
//!
//! Import is a trust boundary: the name and the extension are the user's, the
//! bytes are not. A file is sniffed by its own content, capped, and sanitised
//! when it is SVG, so what the renderer later decodes is what this module saw.

use yunari_model::document::{AssetKind, AssetReference, ThemeDocument};

/// What a finder row knows about the asset it will place, so the import lands
/// with its attribution rather than as an anonymous file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCredit {
    pub kind: String,
    pub title: String,
    pub license: String,
    pub source_url: Option<String>,
    pub author: Option<String>,
}

/// A picture the raster backend decodes at frame time, and its own ceiling.
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// An SVG is text the raster backend parses; it is capped far lower.
pub const MAX_SVG_BYTES: u64 = 4 * 1024 * 1024;

pub const NOT_A_FILE_MESSAGE: &str = "That file is not a picture this editor can show.";
pub const UNSAFE_SVG_MESSAGE: &str =
    "That SVG carries a script or an outside reference, so it was not imported.";
pub const NO_PICTURE_MESSAGE: &str = "Import a picture first.";
pub const UNREADABLE_MESSAGE: &str = "That file could not be read: ";

/// How large a file of this kind may be, in bytes.
pub fn limit(kind: AssetKind) -> u64 {
    match kind {
        AssetKind::Svg => MAX_SVG_BYTES,
        _ => MAX_IMAGE_BYTES,
    }
}

/// The kind a file's own bytes say it is, with its name only for SVG, which
/// has no magic number of its own.
pub fn sniff(name: &str, bytes: &[u8]) -> Result<AssetKind, String> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Ok(AssetKind::Image);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Ok(AssetKind::Image);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok(AssetKind::Gif);
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Ok(AssetKind::Image);
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".svg")
        && let Ok(text) = std::str::from_utf8(bytes)
        && text.contains("<svg")
    {
        return Ok(AssetKind::Svg);
    }
    Err(NOT_A_FILE_MESSAGE.to_owned())
}

/// Refuses an SVG that could run code or reach outside the theme.
pub fn sanitize_svg(text: &str) -> Result<(), String> {
    let lower = text.to_ascii_lowercase();
    // ponytail: a textual refusal, not a full SVG parser. It stops the script
    // and outside-reference forms this renderer could act on; if the raster
    // backend ever resolves more of SVG, this moves into its parser.
    const REFUSED: [&str; 10] = [
        "<script",
        "<foreignobject",
        "javascript:",
        "onload=",
        "onerror=",
        "onclick=",
        "onmouseover=",
        "onbegin=",
        "<!entity",
        "href=\"http",
    ];
    if REFUSED.iter().any(|needle| lower.contains(needle)) || lower.contains("url(http") {
        return Err(UNSAFE_SVG_MESSAGE.to_owned());
    }
    Ok(())
}

/// The extension a stored copy takes, from the kind its bytes named.
pub fn extension(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "png",
        AssetKind::Gif => "gif",
        AssetKind::Svg => "svg",
        AssetKind::Video => "mp4",
        AssetKind::Font => "ttf",
    }
}

/// The picture a placed image node should name first: the theme's own first
/// asset a renderer can draw.
pub fn first_picture(document: &ThemeDocument) -> Option<&AssetReference> {
    document.assets.iter().find(|asset| {
        matches!(
            asset.kind,
            AssetKind::Image | AssetKind::Svg | AssetKind::Gif
        )
    })
}

/// A file name reduced to what a theme path may hold.
pub fn stem(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    let cleaned: String = stem
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || char == '-' || char == '_' {
                char
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-').to_owned();
    if cleaned.is_empty() {
        "picture".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

    #[test]
    fn a_kind_comes_from_the_bytes_not_the_name() {
        assert_eq!(sniff("a.png", PNG), Ok(AssetKind::Image));
        assert_eq!(
            sniff("a.jpg", &[0xff, 0xd8, 0xff, 0xe0]),
            Ok(AssetKind::Image)
        );
        assert_eq!(sniff("a.gif", b"GIF89a...."), Ok(AssetKind::Gif));
        assert_eq!(
            sniff("a.webp", b"RIFF\x10\0\0\0WEBPVP8 "),
            Ok(AssetKind::Image)
        );
        assert_eq!(
            sniff(
                "logo.svg",
                br#"<?xml version="1.0"?><svg viewBox="0 0 1 1"/>"#
            ),
            Ok(AssetKind::Svg)
        );
        // A ZIP named as a picture, and plain text, are not pictures.
        assert_eq!(
            sniff("a.png", b"PK\x03\x04"),
            Err(NOT_A_FILE_MESSAGE.to_owned())
        );
        assert_eq!(
            sniff("notes.txt", b"hello"),
            Err(NOT_A_FILE_MESSAGE.to_owned())
        );
        // The SVG name alone is not enough either.
        assert_eq!(
            sniff("a.svg", b"<html>"),
            Err(NOT_A_FILE_MESSAGE.to_owned())
        );
    }

    #[test]
    fn an_svg_with_a_script_or_an_outside_reference_is_refused() {
        assert!(sanitize_svg(r#"<svg><rect width="1"/></svg>"#).is_ok());
        for unsafe_svg in [
            r#"<svg><script>alert(1)</script></svg>"#,
            r#"<svg onload="x()"/>"#,
            r#"<svg><a href="javascript:x()"/></svg>"#,
            r#"<svg><foreignObject/></svg>"#,
            r#"<svg><image href="http://evil.example/x.png"/></svg>"#,
        ] {
            assert_eq!(
                sanitize_svg(unsafe_svg),
                Err(UNSAFE_SVG_MESSAGE.to_owned()),
                "{unsafe_svg}"
            );
        }
    }

    #[test]
    fn a_name_becomes_a_path_segment() {
        assert_eq!(stem("C:\\Users\\me\\My Photo.PNG"), "my-photo");
        assert_eq!(stem("../../etc/passwd"), "passwd");
        assert_eq!(stem("###.png"), "picture");
        assert_eq!(extension(AssetKind::Svg), "svg");
    }
}
