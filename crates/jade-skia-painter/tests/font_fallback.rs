//! Each character falls back to a registered face that has it, as a
//! browser's text does — when measured as when drawn. Thai after Latin in
//! one run is measured and drawn in the Thai face, not as the styled face's
//! missing-glyph boxes.
use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn font(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts")
            .join(path),
    )
    .unwrap()
}

const JAPANESE: &str = "Noto_Sans_JP/static/NotoSansJP-Regular.ttf";
const THAI: &str = "Noto_Sans_Thai/NotoSansThai-VariableFont_wdth,wght.ttf";

#[test]
fn thai_tone_marks_stack_above_upper_vowels() {
    let bytes = font(THAI);
    let render = |text| {
        let painter = SkiaPainter::new(180, 100).unwrap();
        painter.register_font_bytes("Thai", 400, &bytes).unwrap();
        painter.set_font("60px Thai");
        painter.set_fill_style_str("#000");
        painter.fill_text(text, 5., 80.).unwrap();
        let (pixels, width, _) = painter.read_bgra_premultiplied().unwrap();
        let top = pixels
            .chunks_exact(width as usize * 4)
            .position(|row| row.chunks_exact(4).any(|pixel| pixel[0] < 128))
            .unwrap();
        let metrics = painter.measure_text_metrics(text).unwrap();
        assert!((metrics.width - painter.measure_text_width(text).unwrap()).abs() < 0.01);
        (top, metrics)
    };
    for (bare, marked) in [("ยี", "ยี่"), ("หนึง", "หนึ่ง")] {
        let (bare_top, bare_metrics) = render(bare);
        let (marked_top, marked_metrics) = render(marked);
        assert!(
            marked_top + 4 < bare_top,
            "{marked}: {marked_top} vs {bare_top}"
        );
        assert!(
            marked_metrics.actual_bounding_box_ascent
                > bare_metrics.actual_bounding_box_ascent + 4.
        );
        assert!((marked_metrics.width - bare_metrics.width).abs() < 0.01);
    }
}

#[test]
fn shaped_thai_keeps_its_original_unicode_in_pdf() {
    let bytes = font(THAI);
    let text = "ยี่ หนึ่ง น้ำ";
    let painter = SkiaPainter::new_recording(250., 100.).unwrap();
    painter.register_font_bytes("Thai", 400, &bytes).unwrap();
    painter.set_font("40px Thai");
    painter.fill_text(text, 10., 60.).unwrap();
    let pdf = SkiaPainter::encode_pdf(&[painter]).unwrap();
    let doc = lopdf::Document::load_mem(&pdf).unwrap();
    let extracted = doc.extract_text(&[1]).unwrap();
    assert_eq!(
        extracted.split_whitespace().collect::<String>(),
        text.split_whitespace().collect::<String>(),
        "{extracted:?}"
    );
}

fn width(faces: &[(&str, &str)], css: &str, text: &str) -> f64 {
    let painter = SkiaPainter::new(1, 1).unwrap();
    for (family, path) in faces {
        painter
            .register_font_bytes(*family, 400, &font(path))
            .unwrap();
    }
    painter.set_font(css);
    painter.measure_text_width(text).unwrap()
}

#[test]
fn thai_after_latin_measures_in_the_thai_face() {
    let both = [("Noto Sans JP", JAPANESE), ("Noto Sans Thai", THAI)];
    let latin = width(&both, "16px 'Noto Sans JP'", "Total ");
    let baht = width(&[("Noto Sans Thai", THAI)], "16px 'Noto Sans Thai'", "บาท");
    let mixed = width(&both, "16px 'Noto Sans JP'", "Total บาท");
    assert!(
        (mixed - (latin + baht)).abs() < 0.01,
        "{mixed} vs {latin} + {baht}"
    );
    let metrics = {
        let painter = SkiaPainter::new(1, 1).unwrap();
        for (family, path) in both {
            painter
                .register_font_bytes(family, 400, &font(path))
                .unwrap();
        }
        painter.set_font("16px 'Noto Sans JP'");
        painter.measure_text_metrics("Total บาท").unwrap()
    };
    assert!((metrics.width - mixed).abs() < 0.01);
}

#[test]
fn thai_after_latin_draws_in_the_thai_face() {
    let draw = |text: &str| {
        let painter = SkiaPainter::new(140, 24).unwrap();
        for (family, path) in [("Noto Sans JP", JAPANESE), ("Noto Sans Thai", THAI)] {
            painter
                .register_font_bytes(family, 400, &font(path))
                .unwrap();
        }
        painter.set_font("16px 'Noto Sans JP'");
        painter.set_fill_style_str("#000000");
        painter.fill_text(text, 0.0, 18.0).unwrap();
        painter.encode_png().unwrap()
    };
    // The Thai after "Total " is drawn as "Total " then the Thai alone in
    // the Thai face would be, at the same pen position.
    let together = draw("Total บาท");
    let painter = SkiaPainter::new(140, 24).unwrap();
    for (family, path) in [("Noto Sans JP", JAPANESE), ("Noto Sans Thai", THAI)] {
        painter
            .register_font_bytes(family, 400, &font(path))
            .unwrap();
    }
    painter.set_font("16px 'Noto Sans JP'");
    painter.set_fill_style_str("#000000");
    painter.fill_text("Total ", 0.0, 18.0).unwrap();
    let latin = painter.measure_text_width("Total ").unwrap();
    painter.fill_text("บาท", latin, 18.0).unwrap();
    assert!(
        together == painter.encode_png().unwrap(),
        "the Thai is not drawn as the Thai face draws it"
    );
}
