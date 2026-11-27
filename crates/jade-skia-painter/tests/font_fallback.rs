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

#[test]
fn arabic_fallback_uses_contextual_advances_and_positioned_glyphs() {
    let arabic = font("Noto_Sans_Arabic/NotoSansArabic-Regular.ttf");
    let painter = SkiaPainter::new(200, 100).unwrap();
    painter
        .register_font_bytes("Japanese", 400, &font(JAPANESE))
        .unwrap();
    painter.register_font_bytes("Arabic", 400, &arabic).unwrap();
    painter.set_font("40px Japanese");
    let text = "سلام";
    let face = rustybuzz::Face::from_slice(&arabic, 0).unwrap();
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(&face, &[], buffer);
    let mut expected = 0.0;
    let mut positions = Vec::new();
    for (glyph, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        positions.push((glyph.cluster as usize, 10.0 + expected));
        expected += f64::from(position.x_advance) * 40.0 / f64::from(face.units_per_em());
    }
    positions.sort_by_key(|&(byte, _)| byte);
    let measured = painter.measure_text_width(text).unwrap();
    assert!(
        (measured - expected).abs() < 0.01,
        "{measured} vs {expected}"
    );
    assert!(painter
        .try_fill_shaped_text(text, &positions, 60.0)
        .unwrap());

    let direct = SkiaPainter::new(200, 100).unwrap();
    direct.register_font_bytes("Arabic", 400, &arabic).unwrap();
    direct.set_font("40px Arabic");
    direct.fill_text(text, 10.0, 60.0).unwrap();
    assert!(painter.encode_png().unwrap() == direct.encode_png().unwrap());

    let whole_word = SkiaPainter::new(200, 100).unwrap();
    whole_word
        .register_font_bytes("Arabic", 400, &arabic)
        .unwrap();
    whole_word.set_font("40px Arabic");
    assert!(whole_word
        .try_fill_shaped_text(text, &[(0, 10.0)], 60.0)
        .unwrap());
    assert!(whole_word.encode_png().unwrap() == direct.encode_png().unwrap());
}

#[test]
fn mixed_arabic_uses_visual_order_without_reversing_the_digits() {
    let make = || {
        let painter = SkiaPainter::new(300, 100).unwrap();
        painter
            .register_font_bytes("Japanese", 400, &font(JAPANESE))
            .unwrap();
        painter
            .register_font_bytes(
                "Arabic",
                400,
                &font("Noto_Sans_Arabic/NotoSansArabic-Regular.ttf"),
            )
            .unwrap();
        painter.set_font("30px Japanese");
        painter
    };
    let mixed = make();
    mixed.fill_text("abc عربي 123", 5.0, 60.0).unwrap();
    let reference = make();
    let prefix = "abc 123 ";
    reference.fill_text(prefix, 5.0, 60.0).unwrap();
    let x = 5.0 + reference.measure_text_width(prefix).unwrap();
    reference.fill_text("عربي", x, 60.0).unwrap();
    assert!(mixed.encode_png().unwrap() == reference.encode_png().unwrap());
}

#[test]
fn positioned_arabic_keeps_its_logical_text_in_pdf() {
    let bytes = font("Noto_Sans_Arabic/NotoSansArabic-Regular.ttf");
    let text = "سلام";
    let painter = SkiaPainter::new_recording(200.0, 100.0).unwrap();
    painter.register_font_bytes("Arabic", 400, &bytes).unwrap();
    painter.set_font("40px Arabic");
    let positions = [(0, 63.0), (2, 54.0), (4, 39.0), (6, 20.0)];
    assert!(painter
        .try_fill_shaped_text(text, &positions, 60.0)
        .unwrap());
    let pdf = SkiaPainter::encode_pdf(&[painter]).unwrap();
    let doc = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(doc.extract_text(&[1]).unwrap().trim(), text);
}
