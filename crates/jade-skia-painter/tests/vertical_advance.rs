use jade_painter::{capture::RoutedPainter, Painter};
use jade_skia_painter::SkiaPainter;

// Change actual font table data so an em-sized substitute cannot pass this test.
fn font_with_advance(fraction: f64) -> Vec<u8> {
    let mut bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"
    ))
    .unwrap();
    let face = rustybuzz::ttf_parser::Face::parse(&bytes, 0).unwrap();
    let advance = (f64::from(face.units_per_em()) * fraction) as u16;
    let table_offset = |tag: &[u8]| {
        let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
        (0..count)
            .find_map(|i| {
                let record = 12 + i * 16;
                (&bytes[record..record + 4] == tag).then(|| {
                    u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize
                })
            })
            .unwrap()
    };
    let vhea = table_offset(b"vhea");
    let vmtx = table_offset(b"vmtx");
    let count = u16::from_be_bytes(bytes[vhea + 34..vhea + 36].try_into().unwrap()) as usize;
    for i in 0..count {
        bytes[vmtx + 4 * i..vmtx + 4 * i + 2].copy_from_slice(&advance.to_be_bytes());
    }
    bytes
}

#[test]
fn vertical_advance_uses_font_tables_and_survives_capture_routing() {
    let painter = SkiaPainter::new(200, 200).unwrap();
    painter
        .register_font_bytes("Vertical Metric Test", 400, &font_with_advance(0.75))
        .unwrap();
    let routed = RoutedPainter::new(painter, None);
    for (font, expected) in [
        ("32px 'Vertical Metric Test'", 24.0),
        ("16px 'Vertical Metric Test'", 12.0),
    ] {
        routed.set_font(font);
        for text in ["i", "あ"] {
            assert_eq!(
                routed.measure_upright_vertical_advance(text).unwrap(),
                Some(expected)
            );
        }
        for text in ["", "ab", "a\u{20dd}", "\u{10ffff}"] {
            assert_eq!(routed.measure_upright_vertical_advance(text).unwrap(), None);
        }
    }
}

#[test]
fn fonts_without_vertical_metrics_leave_layout_fallback_available() {
    let mut bytes = font_with_advance(0.75);
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    for i in 0..count {
        let record = 12 + i * 16;
        if &bytes[record..record + 4] == b"vmtx" {
            bytes[record..record + 4].copy_from_slice(b"TEST");
        }
    }
    let painter = SkiaPainter::new(200, 200).unwrap();
    painter
        .register_font_bytes("No Vertical Metrics", 400, &bytes)
        .unwrap();
    painter.set_font("32px 'No Vertical Metrics'");
    assert!(painter.measure_text_width("i").unwrap() > 0.0);
    assert_eq!(painter.measure_upright_vertical_advance("i").unwrap(), None);
}

#[test]
fn upright_brackets_use_vertical_alternates_and_capture_replays_them() {
    use jade_painter::capture::{new_sink, replay};
    let bytes = font_with_advance(1.0);
    let render = |text: &str, upright: bool, capture: bool| {
        let painter = SkiaPainter::new(80, 80).unwrap();
        painter
            .register_font_bytes("Vertical Test", 400, &bytes)
            .unwrap();
        let sink = new_sink();
        let routed = RoutedPainter::new(painter, capture.then(|| sink.clone()));
        routed.set_font("40px 'Vertical Test'");
        routed.set_text_baseline("top");
        routed.set_fill_style_str("#000000");
        if upright {
            routed.fill_upright_text(text, 10.0, 10.0).unwrap();
        } else {
            routed.fill_text(text, 10.0, 10.0).unwrap();
        }
        if capture {
            assert_eq!(routed.inner().fill_text_calls(), 0);
            replay(&sink.borrow(), routed.inner(), |_| {}).unwrap();
        }
        assert_eq!(routed.inner().fill_text_calls(), 1);
        routed.inner().encode_png().unwrap()
    };
    // Unicode's presentation form supplies an independent glyph reference.
    let horizontal = render("（", false, false);
    let reference = render("︵", false, false);
    assert_ne!(horizontal, reference);
    assert!(render("（", true, false) == reference);
    assert!(render("（", true, true) == reference);
    assert!(render("あ", true, true) == render("あ", true, false));
    for text in ["i", "ab", "a\u{20dd}", ""] {
        assert!(
            render(text, true, false) == render(text, false, false),
            "{text:?}"
        );
    }
}

#[test]
fn upright_advance_comes_from_the_substituted_glyph() {
    let mut bytes = font_with_advance(1.0);
    let face = rustybuzz::ttf_parser::Face::parse(&bytes, 0).unwrap();
    let glyph = face.glyph_index('︵').unwrap().0 as usize;
    assert_ne!(face.glyph_index('（').unwrap().0 as usize, glyph);
    let advance = face.units_per_em() * 3 / 4;
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let table = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&record| &bytes[record..record + 4] == b"vmtx")
        .unwrap();
    let offset = u32::from_be_bytes(bytes[table + 8..table + 12].try_into().unwrap()) as usize;
    bytes[offset + glyph * 4..offset + glyph * 4 + 2].copy_from_slice(&advance.to_be_bytes());
    let painter = SkiaPainter::new(80, 80).unwrap();
    painter
        .register_font_bytes("Alternate Metrics", 400, &bytes)
        .unwrap();
    painter.set_font("32px 'Alternate Metrics'");
    assert_eq!(
        painter.measure_upright_vertical_advance("（").unwrap(),
        Some(24.0)
    );
    assert_eq!(
        painter.measure_upright_vertical_advance("あ").unwrap(),
        Some(32.0)
    );
}

#[test]
fn vertical_alternates_keep_original_unicode_in_pdf() {
    for editor_recording in [false, true] {
        let painter = if editor_recording {
            SkiaPainter::new_editor_recording(100.0, 100.0, 1.0).unwrap()
        } else {
            SkiaPainter::new_recording(100.0, 100.0).unwrap()
        };
        painter
            .register_font_bytes("Vertical PDF", 400, &font_with_advance(1.0))
            .unwrap();
        painter.set_font("32px 'Vertical PDF'");
        painter.fill_upright_text("（", 10.0, 50.0).unwrap();
        let bytes = SkiaPainter::encode_pdf(&[painter]).unwrap();
        let document = lopdf::Document::load_mem(&bytes).unwrap();
        let text = document.extract_text(&[1]).unwrap();
        assert!(text.contains('（'), "original Unicode missing: {text:?}");
        assert!(
            !text.contains('︵'),
            "presentation Unicode leaked: {text:?}"
        );
    }
}

#[test]
fn composed_vertical_graphemes_preserve_pixels_advances_and_pdf_unicode() {
    let bytes = font_with_advance(0.75);
    let render = |text: &str| {
        let painter = SkiaPainter::new(80, 80).unwrap();
        painter
            .register_font_bytes("Grapheme", 400, &bytes)
            .unwrap();
        painter.set_font("32px Grapheme");
        painter.set_fill_style_str("#204080");
        painter.fill_upright_text(text, 10.0, 50.0).unwrap();
        assert_eq!(
            painter.measure_upright_vertical_advance(text).unwrap(),
            Some(24.0)
        );
        painter.encode_png().unwrap()
    };
    for (cluster, composed) in [("か\u{3099}", "が"), ("a\u{301}", "á")] {
        assert_eq!(render(cluster), render(composed));
        for editor in [false, true] {
            let painter = if editor {
                SkiaPainter::new_editor_recording(100.0, 100.0, 1.0).unwrap()
            } else {
                SkiaPainter::new_recording(100.0, 100.0).unwrap()
            };
            painter
                .register_font_bytes("Grapheme", 400, &bytes)
                .unwrap();
            painter.set_font("32px Grapheme");
            painter.fill_upright_text(cluster, 10.0, 50.0).unwrap();
            let pdf = SkiaPainter::encode_pdf(&[painter]).unwrap();
            let doc = lopdf::Document::load_mem(&pdf).unwrap();
            let text = doc.extract_text(&[1]).unwrap();
            assert!(
                text.contains(cluster),
                "original grapheme missing: {text:?}"
            );
        }
    }
}
