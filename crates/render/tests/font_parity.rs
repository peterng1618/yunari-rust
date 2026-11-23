//! Font parity: a packaged face declared by a theme must resolve for the
//! family and weight its type tokens name, and a run must keep its authored
//! size/weight. Uses the installed themes' real woff2 bytes when present;
//! says so when absent, so no fabricated face stands in for them.

use std::path::PathBuf;

use yunari_render::text::{FontFamily, ShapeBuffer, TextMetrics, TextShaping, TextStyle, attrs};
use yunari_render::{Fonts, Shape};

fn measure(fonts: &mut Fonts, text: &str, style: &TextStyle) -> f32 {
    let metrics = TextMetrics::new(style.size, style.size * 1.2);
    let mut buffer = ShapeBuffer::new(fonts.system_mut(), metrics);
    let width = {
        let mut borrowed = buffer.borrow_with(fonts.system_mut());
        borrowed.set_text(text, &attrs(style, 1.2), TextShaping::Advanced, None);
        borrowed
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
    };
    buffer.shape_until_scroll(fonts.system_mut(), false);
    width
}

fn theme_dir(id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    let dir = home.join(".yunari").join("themes").join(id);
    dir.join("theme.json").is_file().then_some(dir)
}

/// The overlay theme declares Manrope 600 and Lora 700. Their woff2 name
/// tables say "Manrope ExtraLight" and "Lora"; the former disagrees with the
/// declared family, which is why the alias owner has to exist.
#[test]
fn a_declared_package_face_resolves_at_its_declared_weight() {
    let Some(dir) = theme_dir("a1098a60-7463-43aa-9699-6198580cd7c7") else {
        eprintln!("installed overlay theme absent; skipping real-face assertion");
        return;
    };
    let json = std::fs::read_to_string(dir.join("theme.json")).expect("theme readable");
    let document = yunari_model::document::parse_document(&json).expect("theme parses");
    let mut fonts = Fonts::new();
    let refusals = fonts.load_faces(&document, |path| std::fs::read(dir.join(path)).ok());
    assert!(refusals.is_empty(), "{refusals:?}");

    for (family, weight) in [("Manrope", 600u16), ("Lora", 700)] {
        let style = TextStyle {
            family: Some(FontFamily::Named(family.to_owned())),
            weight,
            ..TextStyle::default()
        };
        let fallback = TextStyle {
            family: Some(FontFamily::Named("__no_such_face__".to_owned())),
            weight,
            ..TextStyle::default()
        };
        // 600/700 packaged faces have advances unlike the system fallback;
        // at weight 400 the CSS matcher legitimately picks a different face.
        let width = measure(&mut fonts, "MMMMMMMMWWWW", &style);
        let other = measure(&mut fonts, "MMMMMMMMWWWW", &fallback);
        assert!(
            (width - other).abs() > 0.5,
            "{family} {weight} resolved to the fallback: {width} vs {other}"
        );
    }
}

/// The declared family is the alias owner even when it is not the font's own
/// name. A synthetic theme declares the installed Lora face as "Panel Face",
/// a name no system font uses, then shapes at a weight nothing else has.
#[test]
fn a_declared_family_alias_reaches_shaping_even_when_it_is_not_the_fonts_name() {
    let Some(dir) = theme_dir("a1098a60-7463-43aa-9699-6198580cd7c7") else {
        eprintln!("installed overlay theme absent; skipping alias assertion");
        return;
    };
    let bytes = std::fs::read(dir.join("assets/lora-700-c05fd528.woff2")).expect("face readable");
    let document = yunari_model::document::parse_document(
        r##"{"schemaVersion":"3.0.0","id":"alias","artboard":{"width":100,"height":100},
            "nodes":[],
            "assets":[{"id":"f","kind":"font","path":"assets/face.woff2","family":"Panel Face",
                "weight":700,"style":"normal","format":"woff2"}]}"##,
    )
    .expect("fixture parses");
    let mut fonts = Fonts::new();
    let refusals = fonts.load_faces(&document, |path| {
        (path == "assets/face.woff2").then(|| bytes.clone())
    });
    assert!(refusals.is_empty(), "{refusals:?}");

    let aliased = measure(
        &mut fonts,
        "MMMMMMMMWWWW",
        &TextStyle {
            family: Some(FontFamily::Named("Panel Face".to_owned())),
            weight: 700,
            ..TextStyle::default()
        },
    );
    let fallback = measure(
        &mut fonts,
        "MMMMMMMMWWWW",
        &TextStyle {
            family: Some(FontFamily::Named("__no_such_face__".to_owned())),
            weight: 700,
            ..TextStyle::default()
        },
    );
    assert!(
        (aliased - fallback).abs() > 0.5,
        "the declared alias did not reach shaping: {aliased} vs {fallback}"
    );
}

/// The Kraken theme declares DM Sans 600 and Bebas Neue 400. Both packaged
/// faces must resolve at those weights, or their clock reads in a fallback.
#[test]
fn kraken_theme_packaged_faces_resolve_at_their_declared_weights() {
    let Some(dir) = theme_dir("1cf543ae-cf7f-4dc2-a6cd-1c7d7430932d") else {
        eprintln!("installed Kraken theme absent; skipping real-face assertion");
        return;
    };
    let json = std::fs::read_to_string(dir.join("theme.json")).expect("theme readable");
    let document = yunari_model::document::parse_document(&json).expect("theme parses");
    let mut fonts = Fonts::new();
    let refusals = fonts.load_faces(&document, |path| std::fs::read(dir.join(path)).ok());
    assert!(refusals.is_empty(), "{refusals:?}");

    for (family, weight) in [("DM Sans", 600u16), ("Bebas Neue", 400)] {
        let style = TextStyle {
            family: Some(FontFamily::Named(family.to_owned())),
            weight,
            ..TextStyle::default()
        };
        let fallback = TextStyle {
            family: Some(FontFamily::Named("__no_such_face__".to_owned())),
            weight,
            ..TextStyle::default()
        };
        let width = measure(&mut fonts, "MMMMMMMMWWWW", &style);
        let other = measure(&mut fonts, "MMMMMMMMWWWW", &fallback);
        assert!(
            (width - other).abs() > 0.5,
            "{family} {weight} resolved to the fallback: {width} vs {other}"
        );
    }
}

#[test]
fn a_declared_face_reaches_the_text_op_with_its_authored_family_size_and_weight() {
    let Some(dir) = theme_dir("a1098a60-7463-43aa-9699-6198580cd7c7") else {
        eprintln!("installed overlay theme absent; skipping real-theme assertion");
        return;
    };
    let json = std::fs::read_to_string(dir.join("theme.json")).expect("theme readable");
    let document = yunari_model::document::parse_document(&json).expect("theme parses");
    let mut fonts = Fonts::new();
    assert!(
        fonts
            .load_faces(&document, |path| std::fs::read(dir.join(path)).ok())
            .is_empty()
    );
    let list = yunari_render::fabric::build(
        &document,
        &mut fonts,
        &yunari_render::Readings::default(),
        &yunari_providers::units::UnitPreferences::default(),
    )
    .0;
    let texts: Vec<_> = list
        .ops
        .iter()
        .filter_map(|op| match &op.shape {
            Shape::Text(layout) => Some(layout),
            _ => None,
        })
        .collect();
    assert!(!texts.is_empty(), "the theme has no text ops");
    let heading = texts
        .iter()
        .find(|layout| {
            layout
                .segments
                .iter()
                .any(|segment| segment.style.size == 144.0)
        })
        .expect("the 144px clock run");
    let run = heading
        .segments
        .iter()
        .find(|segment| segment.style.size == 144.0)
        .unwrap();
    assert_eq!(
        run.style.family.as_ref().and_then(|f| f.name()),
        Some("Lora")
    );
    assert_eq!(run.style.weight, 700);
    assert_eq!(run.style.size, 144.0);
}
