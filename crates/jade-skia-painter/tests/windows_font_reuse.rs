#![cfg(target_os = "windows")]

use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn font(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static")
            .join(name),
    )
    .unwrap()
}

#[test]
fn reopened_documents_and_aliases_reuse_identical_font_content() {
    let bytes = font("NotoSansJP-Regular.ttf");
    let mut first_id = None;
    let mut first_width = None;
    for document in 0..40 {
        let painter = SkiaPainter::new(1, 1).unwrap();
        let family = format!("Document {document}");
        // Separate allocations must still share the underlying DirectWrite face.
        painter
            .register_font_bytes(&family, 400, &bytes.clone())
            .unwrap();
        let (_, _, face) = painter.registered_faces().pop().unwrap();
        assert_eq!(*first_id.get_or_insert(face.unique_id()), face.unique_id());
        painter.set_font(&format!("16px '{family}'"));
        let width = painter.measure_text_width("Word 日本語").unwrap();
        assert_eq!(*first_width.get_or_insert(width), width);
    }
}

#[test]
fn content_and_collection_index_are_not_confused() {
    let painter = SkiaPainter::new(1, 1).unwrap();
    let regular = font("NotoSansJP-Regular.ttf");
    painter.register_font_bytes("Face", 400, &regular).unwrap();
    painter
        .register_font_bytes("Face", 700, &font("NotoSansJP-Bold.ttf"))
        .unwrap();
    let faces = painter.registered_faces();
    assert_ne!(faces[0].2.unique_id(), faces[1].2.unique_id());
    // Index 1 is not a valid face of this standalone TTF; a cached index 0
    // must not turn this invalid registration into a successful one.
    assert!(painter
        .register_font_bytes_styled("Invalid", 400, false, &regular, 1)
        .is_none());
    painter.set_font("16px 'Face'");
    let normal = painter.measure_text_width("Hello world").unwrap();
    painter.set_font("bold 16px 'Face'");
    assert!((normal - painter.measure_text_width("Hello world").unwrap()).abs() > 0.5);
}
