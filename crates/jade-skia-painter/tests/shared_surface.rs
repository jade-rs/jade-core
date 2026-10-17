//! The painter is handed out by value, so cloning has to be cheap *and* has to
//! keep every clone painting onto the same pixels.
//!
//! `CanvasRenderHost::painter` returns the backend's painter by value, many
//! times per page. A backend whose clone got its own surface would compile and
//! then silently drop most of the page.

use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;

fn white_canvas(width: i32, height: i32) -> SkiaPainter {
    let painter = SkiaPainter::new(width, height).expect("create raster surface");
    painter.set_fill_style_str("#ffffff");
    painter.fill_rect(0.0, 0.0, width as f64, height as f64);
    painter
}

#[test]
fn clones_paint_onto_the_same_surface() {
    let first = white_canvas(40, 20);
    let second = first.clone();

    first.set_fill_style_str("#000000");
    first.fill_rect(0.0, 0.0, 10.0, 20.0);
    second.set_fill_style_str("#000000");
    second.fill_rect(20.0, 0.0, 10.0, 20.0);

    // Both rectangles are visible from either handle.
    for painter in [&first, &second] {
        assert!(painter.pixel(5, 10).expect("read").0 < 40, "left rect");
        assert!(painter.pixel(25, 10).expect("read").0 < 40, "right rect");
    }
}

/// Style is part of what a canvas context carries, and clones of a canvas
/// context share it. A clone whose style stack was its own would silently drop
/// a `restore` issued through a different handle.
#[test]
fn clones_share_the_style_stack() {
    let first = white_canvas(20, 20);
    let second = first.clone();

    first.set_fill_style_str("#000000");
    first.save();
    second.set_fill_style_str("#ff0000");
    second.restore();

    // The restore ran on the state the other handle saved.
    first.fill_rect(5.0, 5.0, 10.0, 10.0);
    let (r, g, b, _) = first.pixel(10, 10).expect("read");
    assert!(
        r < 40 && g < 40 && b < 40,
        "expected the saved black, got {r},{g},{b}"
    );
}

#[test]
fn a_face_registered_through_one_handle_is_visible_from_another() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let bytes = std::fs::read(path).expect("read bundled Noto Sans JP");

    let first = white_canvas(120, 40);
    let second = first.clone();
    second
        .register_font_bytes("Noto Sans JP", 400, &bytes)
        .expect("register face");

    // Registration is setup, but it happens through whichever handle is at
    // hand; measuring from the other one has to see the same faces.
    first.set_font("12.00pt 'Noto Sans JP'");
    let width = first.measure_text_width("Hg").expect("measure");
    assert!(width > 0.0, "registered face should measure, got {width}");
}

/// A GPU compositor on Apple platforms wants BGRA with premultiplied alpha,
/// and gpui's `RenderImage` documents itself as holding exactly that. Reading
/// in RGBA order and handing it over would compile and then show the page with
/// red and blue swapped, which is easy to miss on mostly-black text.
#[test]
fn pixels_read_back_in_bgra_order() {
    let painter = white_canvas(4, 4);
    // Pure red: the channel that moves is the one that proves the order.
    painter.set_fill_style_str("#ff0000");
    painter.fill_rect(0.0, 0.0, 4.0, 4.0);

    let (pixels, width, height) = painter
        .read_bgra_premultiplied()
        .expect("read back the surface");

    assert_eq!((width, height), (4, 4));
    assert_eq!(pixels.len(), 4 * 4 * 4);
    let (b, g, r, a) = (pixels[0], pixels[1], pixels[2], pixels[3]);
    assert_eq!(
        (b, g, r, a),
        (0, 0, 255, 255),
        "expected red in the third byte, got b={b} g={g} r={r} a={a}"
    );
}

/// A glyph the styled face lacks has to come from one that has it.
///
/// Text is drawn a grapheme at a time, so this is per-character fallback, the
/// same as the measurer and the browser do. Without it, Word's line- and
/// page-break marks render as tofu against a Japanese text face, which carries
/// the pilcrow but not the arrows.
#[test]
fn a_glyph_missing_from_the_styled_face_falls_back() {
    let painter = white_canvas(60, 60);

    let noto = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    painter
        .register_font_bytes("Primary", 400, &std::fs::read(&noto).expect("read Noto"))
        .expect("register the primary face");

    let ink = |painter: &SkiaPainter| {
        (0..60)
            .flat_map(|y| (0..60).map(move |x| (x, y)))
            .filter(|(x, y)| painter.pixel(*x, *y).is_some_and(|(r, _, _, _)| r < 200))
            .count()
    };

    // U+21B5 is the paragraph-end mark, and Noto Sans JP does not have it.
    painter.set_font("24.00pt 'Primary'");
    painter.set_fill_style_str("#000000");
    painter
        .fill_text("\u{21B5}", 8.0, 44.0)
        .expect("recording backend is infallible");
    let with_only_primary = ink(&painter);

    let symbols = std::path::Path::new("/System/Library/Fonts/Apple Symbols.ttf");
    if !symbols.exists() {
        // The fallback face is a system font; without it there is nothing to
        // fall back *to* and the rest of the test would prove nothing.
        return;
    }
    painter
        .register_font_bytes(
            "Fallback",
            400,
            &std::fs::read(symbols).expect("read symbols"),
        )
        .expect("register the fallback face");

    let after = white_canvas(60, 60);
    after
        .register_font_bytes("Primary", 400, &std::fs::read(&noto).expect("read Noto"))
        .expect("primary");
    after
        .register_font_bytes(
            "Fallback",
            400,
            &std::fs::read(symbols).expect("read symbols"),
        )
        .expect("fallback");
    after.set_font("24.00pt 'Primary'");
    after.set_fill_style_str("#000000");
    after.fill_text("\u{21B5}", 8.0, 44.0).expect("infallible");

    // Deliberately not "more ink": a tofu box is a filled rectangle and inks
    // far more than the thin arrow it stands in for, so the fallback drawing
    // *less* is the expected direction. What matters is that something was
    // drawn and that it is not the same mark.
    let fallen_back = ink(&after);
    assert!(fallen_back > 0, "the fallback drew nothing at all");
    assert!(
        fallen_back < with_only_primary,
        "the glyph inks as much as the tofu box it replaced ({fallen_back} vs \
         {with_only_primary}) — the fallback probably did not happen"
    );
}
