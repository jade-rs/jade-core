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

#[test]
fn fractional_cell_clip_retains_searchable_text_without_changing_pixels() {
    let bytes = std::fs::read(
        std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("Fonts/verdana.ttf"),
    )
    .unwrap();
    let draw = |preserve, width| {
        let painter = SkiaPainter::new_recording(96.0, 96.0).unwrap();
        let raster = SkiaPainter::new(96, 96).unwrap();
        for painter in [&painter, &raster] {
            painter.set_preserve_clipped_pdf_text(preserve);
            painter.set_text_output_dpi(Some(600.0));
            painter.register_font_bytes("Verdana", 400, &bytes).unwrap();
            painter.set_font("42.72px Verdana");
            painter.set_text_baseline("alphabetic");
            painter.begin_path();
            painter.rect(10.8, 0.0, width, 96.0);
            painter.clip();
            painter.fill_text("B", 10.8, 50.0).unwrap();
        }
        let png = raster.encode_png().unwrap();
        let pdf = SkiaPainter::encode_pdf(&[painter]).unwrap();
        let text = lopdf::Document::load_mem(&pdf)
            .unwrap()
            .extract_text(&[1])
            .unwrap_or_default();
        (png, text)
    };
    // Original Windows Word's 3pt cell at32.04pt: Verdana B's ink
    // lies beyond the exact edge, but inside its rounded device clip.
    let (ordinary_pixels, _) = draw(false, 4.0);
    let (preserved_pixels, retained_text) = draw(true, 4.0);
    assert_eq!(ordinary_pixels, preserved_pixels);
    assert_eq!(retained_text.matches('B').count(), 1);
    // An actually intersecting glyph must keep one copy, with identical ink.
    let (ordinary_pixels, _) = draw(false, 16.0);
    let (preserved_pixels, retained_text) = draw(true, 16.0);
    assert_eq!(ordinary_pixels, preserved_pixels);
    assert_eq!(retained_text.matches('B').count(), 1);
}
