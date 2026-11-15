use proptest::prelude::*;
use yunari_model::document::{Artboard, ThemeDocument, parse_document, write_document};
use yunari_model::validate::validate_document;

fn base() -> ThemeDocument {
    parse_document(
        r#"{"schemaVersion":"1.0.0","id":"theme_1",
            "artboard":{"width":800,"height":480},"nodes":[]}"#,
    )
    .expect("base document")
}

proptest! {
    #[test]
    fn validation_never_panics(width in any::<f64>(), height in any::<f64>()) {
        let mut document = base();
        document.artboard = Artboard { width, height, ..document.artboard };
        let _ = validate_document(&document);
    }

    #[test]
    fn writing_a_valid_document_round_trips(name in "[a-zA-Z0-9 ]{0,120}") {
        let mut document = base();
        document.metadata.name = Some(name);
        if let Ok(json) = write_document(&document) {
            let reparsed = parse_document(&json).expect("written documents parse");
            prop_assert_eq!(reparsed, document);
        }
    }
}
