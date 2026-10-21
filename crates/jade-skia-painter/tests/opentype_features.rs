//! Runs whose font descriptor carries OpenType features (Word's ligatures,
//! figure styles) are drawn shaped: the font's ligature glyph, not its
//! letters side by side.
use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn japanese() -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf"),
    )
    .unwrap()
}

fn ink(draw: impl Fn(&SkiaPainter)) -> Vec<(u8, u8, u8, u8)> {
    let painter = SkiaPainter::new(120, 60).unwrap();
    painter
        .register_font_bytes("Noto Sans JP", 400, &japanese())
        .unwrap();
    painter.set_fill_style_str("#000000");
    draw(&painter);
    (0..60)
        .flat_map(|y| (0..120).map(move |x| (x, y)))
        .map(|(x, y)| painter.pixel(x, y).unwrap())
        .collect()
}

#[test]
fn a_plain_font_declines_the_shaped_path() {
    let painter = SkiaPainter::new(10, 10).unwrap();
    painter
        .register_font_bytes("Noto Sans JP", 400, &japanese())
        .unwrap();
    painter.set_font("40px 'Noto Sans JP'");
    assert!(!painter
        .try_fill_shaped_text("ff", &[(0, 0.0), (1, 10.0)], 5.0)
        .unwrap());
}

#[test]
fn ligatures_draw_the_ligature_glyph() {
    let font = "40px 'Noto Sans JP'";
    let f = {
        let painter = SkiaPainter::new(1, 1).unwrap();
        painter
            .register_font_bytes("Noto Sans JP", 400, &japanese())
            .unwrap();
        painter.set_font(font);
        painter.measure_text_width("f").unwrap()
    };
    let clusters = [(0, 10.0), (1, 10.0 + f)];
    let letters = ink(|painter| {
        painter.set_font(font);
        painter.fill_text("f", 10.0, 45.0).unwrap();
        painter.fill_text("f", 10.0 + f, 45.0).unwrap();
    });
    let unjoined = ink(|painter| {
        painter.set_font(&format!("ot:-liga {font}"));
        assert!(painter.try_fill_shaped_text("ff", &clusters, 45.0).unwrap());
    });
    let joined = ink(|painter| {
        painter.set_font(&format!("ot:liga {font}"));
        assert!(painter.try_fill_shaped_text("ff", &clusters, 45.0).unwrap());
    });
    assert_eq!(letters, unjoined, "without liga the letters stay apart");
    assert_ne!(letters, joined, "with liga the ff ligature is drawn");
    assert!(joined.iter().any(|pixel| pixel.0 < 128));
}
