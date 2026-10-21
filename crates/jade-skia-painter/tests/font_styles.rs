//! A descriptor's italic and bold pick the family's italic and bold faces,
//! and a family without one has it synthesised — slanted or emboldened — as
//! Word and browsers do, rather than drawn upright and regular.
use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

fn font(path: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_JP/static")
            .join(path),
    )
    .unwrap()
}

/// The leftmost inked column in rows `rows` of a painter.
fn left_edge(painter: &SkiaPainter, rows: std::ops::Range<i32>, width: i32) -> Option<i32> {
    rows.flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let (r, _, _, _) = painter.pixel(x, y).unwrap();
            r < 128
        })
        .map(|(x, _)| x)
        .min()
}

fn draw_bar(css: &str) -> SkiaPainter {
    let painter = SkiaPainter::new(60, 60).unwrap();
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(0.0, 0.0, 60.0, 60.0);
    painter
        .register_font_bytes("Noto Sans JP", 400, &font("NotoSansJP-Regular.ttf"))
        .unwrap();
    painter.set_fill_style_str("#000000");
    painter.set_font(css);
    painter.fill_text("l", 20.0, 50.0).unwrap();
    painter
}

#[test]
fn italic_without_an_italic_face_is_drawn_slanted() {
    let upright = draw_bar("40px 'Noto Sans JP'");
    let italic = draw_bar("italic 40px 'Noto Sans JP'");
    let lean = |painter: &SkiaPainter| {
        let top = left_edge(painter, 15..20, 60).unwrap();
        let bottom = left_edge(painter, 44..49, 60).unwrap();
        top - bottom
    };
    assert_eq!(lean(&upright), 0, "an upright l stands straight");
    assert!(
        lean(&italic) >= 4,
        "an italic l leans right: {}",
        lean(&italic)
    );
}

#[test]
fn italic_and_bold_pick_the_family_faces_of_that_style() {
    let painter = SkiaPainter::new(1, 1).unwrap();
    painter
        .register_font_bytes_styled("Face", 400, false, &font("NotoSansJP-Regular.ttf"), 0)
        .unwrap();
    // The bold file stands in for an italic face: its widths tell them apart.
    painter
        .register_font_bytes_styled("Face", 400, true, &font("NotoSansJP-Bold.ttf"), 0)
        .unwrap();
    let width = |css: &str| {
        painter.set_font(css);
        painter.measure_text_width("Hello world").unwrap()
    };
    let regular = width("16px 'Face'");
    let italic = width("italic 16px 'Face'");
    assert!(
        (italic - regular).abs() > 0.5,
        "italic {italic} measured with the italic face, regular {regular}"
    );
    assert_eq!(
        width("italic 16px 'Face'"),
        width("italic bold 16px 'Face'")
    );
}

/// CoreText's font manager reads only a collection's first face; the others
/// are cut out of it, so ＭＳ Ｐゴシック (the third face of msgothic.ttc) is a
/// face that can be drawn with.
#[test]
fn a_later_face_of_a_collection_registers() {
    let Some(path) = [
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
    ]
    .into_iter()
    .find(|path| std::path::Path::new(path).exists()) else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let painter = SkiaPainter::new(1, 1).unwrap();
    painter
        .register_font_bytes_styled("Second", 700, false, &bytes, 1)
        .expect("the second face of a collection registers");
    painter.set_font("16px 'Second'");
    assert!(painter.measure_text_width("Hello").unwrap() > 10.0);
}

fn ink(painter: &SkiaPainter, size: i32) -> usize {
    (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .filter(|&(x, y)| painter.pixel(x, y).unwrap().0 < 128)
        .count()
}

/// A Japanese character in a run whose first family is a Latin face is
/// drawn in the list's East Asian face, at the weight asked — not in
/// whichever face happened to be registered first.
#[test]
fn a_character_the_first_family_lacks_comes_from_the_next_at_its_weight() {
    let thai = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fonts/Noto_Sans_Thai/NotoSansThai-VariableFont_wdth,wght.ttf"),
    )
    .unwrap();
    let draw = |css: &str| {
        let painter = SkiaPainter::new(60, 60).unwrap();
        painter.set_fill_style_str("#ffffff");
        painter.fill_rect(0.0, 0.0, 60.0, 60.0);
        painter
            .register_font_bytes("Stand-in", 400, &font("NotoSansJP-Regular.ttf"))
            .unwrap();
        painter.register_font_bytes("Latin", 700, &thai).unwrap();
        painter
            .register_font_bytes("Japanese", 400, &font("NotoSansJP-Regular.ttf"))
            .unwrap();
        painter
            .register_font_bytes("Japanese", 700, &font("NotoSansJP-Bold.ttf"))
            .unwrap();
        painter.set_fill_style_str("#000000");
        painter.set_font(css);
        painter.fill_text("対", 5.0, 50.0).unwrap();
        ink(&painter, 60)
    };
    let regular = draw("40px 'Japanese'");
    let bold = draw("40px 'Japanese'".replace("40px", "bold 40px").as_str());
    let through_latin = draw("bold 40px 'Latin', 'Japanese'");
    assert!(
        bold > regular + regular / 10,
        "bold {bold} vs regular {regular}"
    );
    assert_eq!(through_latin, bold);
}
