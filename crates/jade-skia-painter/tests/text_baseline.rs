//! Where `fill_text` puts the glyphs, under each canvas text baseline.

use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// A painter with one face registered, so the text has glyphs to draw.
fn painter() -> SkiaPainter {
    let painter = SkiaPainter::new(200, 200).expect("a surface");
    let bytes = std::fs::read(repo_path(
        "fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf",
    ))
    .expect("read the bundled face");
    painter
        .register_font_bytes("Noto Sans JP", 400, &bytes)
        .expect("register the face");
    painter.set_font("32px 'Noto Sans JP'");
    painter
}

/// The rows of the surface that carry any ink.
fn ink_rows(painter: &SkiaPainter) -> Option<(i32, i32)> {
    let (bgra, width, height) = painter.read_bgra_premultiplied()?;
    let mut top = height;
    let mut bottom = -1;
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            if bgra[index] < 200 {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    (bottom >= 0).then_some((top, bottom))
}

/// `alphabetic` is the default: the `y` is the baseline, so the glyphs sit
/// above it.
#[test]
fn alphabetic_draws_above_the_given_y() {
    let painter = painter();
    painter.fill_text("Mg", 10.0, 100.0).expect("draw");

    let (top, bottom) = ink_rows(&painter).expect("ink");
    assert!(top < 100, "nothing above the baseline: {top}..{bottom}");
    // A descender goes below it, but the bulk is above.
    assert!(bottom < 120, "too far below the baseline: {top}..{bottom}");
}

/// `top` means the `y` is the top of the text, so everything is below it.
///
/// The renderer draws table cells this way. Ignoring it drew every cell's text
/// an ascent too high, where the row's top border cut it in half.
#[test]
fn top_draws_below_the_given_y() {
    let painter = painter();
    painter.set_text_baseline("top");
    painter.fill_text("Mg", 10.0, 100.0).expect("draw");

    let (top, bottom) = ink_rows(&painter).expect("ink");
    assert!(
        top >= 100,
        "with textBaseline=top the text still starts above the y: {top}..{bottom}",
    );
}

/// `bottom` puts the em box's bottom on the `y`.
#[test]
fn bottom_draws_above_the_given_y() {
    let painter = painter();
    painter.set_text_baseline("bottom");
    painter.fill_text("Mg", 10.0, 100.0).expect("draw");

    let (top, bottom) = ink_rows(&painter).expect("ink");
    assert!(
        bottom <= 100,
        "with textBaseline=bottom the text goes below the y: {top}..{bottom}",
    );
}

/// `middle` sits between the two.
#[test]
fn middle_straddles_the_given_y() {
    let painter = painter();
    painter.set_text_baseline("middle");
    painter.fill_text("Mg", 10.0, 100.0).expect("draw");

    let (top, bottom) = ink_rows(&painter).expect("ink");
    assert!(
        top < 100 && bottom > 100,
        "with textBaseline=middle the text does not straddle the y: {top}..{bottom}",
    );
}

/// The mode is part of the saved state, like every other style.
#[test]
fn the_baseline_is_restored_with_the_rest_of_the_state() {
    let painter = painter();
    painter.save();
    painter.set_text_baseline("top");
    painter.restore();
    painter.fill_text("Mg", 10.0, 100.0).expect("draw");

    let (top, _) = ink_rows(&painter).expect("ink");
    assert!(
        top < 100,
        "the baseline set inside save/restore outlived the restore",
    );
}

/// A canvas shadow reaches the surface.
///
/// Word's shadow, emboss and imprint character effects are drawn this way, so a
/// backend that ignores the shadow setters draws those runs plain and nobody
/// finds out until they compare with Word.
#[test]
fn a_shadow_puts_ink_where_the_glyph_is_not() {
    let plain = painter();
    plain.fill_text("M", 40.0, 100.0).expect("draw");
    let (plain_top, plain_bottom) = ink_rows(&plain).expect("ink");

    let shadowed = painter();
    shadowed.set_shadow_color("rgba(0, 0, 0, 0.6)");
    shadowed.set_shadow_offset_x(6.0);
    shadowed.set_shadow_offset_y(6.0);
    shadowed.set_shadow_blur(0.0);
    shadowed.fill_text("M", 40.0, 100.0).expect("draw");
    let (shadow_top, shadow_bottom) = ink_rows(&shadowed).expect("ink");

    assert_eq!(shadow_top, plain_top, "the glyph itself moved");
    assert!(
        shadow_bottom > plain_bottom,
        "the shadow left no ink below the glyph: {plain_top}..{plain_bottom} \
         became {shadow_top}..{shadow_bottom}",
    );
}

/// A transparent shadow colour is no shadow, as on a canvas.
#[test]
fn a_transparent_shadow_draws_nothing_extra() {
    let plain = painter();
    plain.fill_text("M", 40.0, 100.0).expect("draw");
    let plain_rows = ink_rows(&plain).expect("ink");

    let shadowed = painter();
    shadowed.set_shadow_color("rgba(0, 0, 0, 0)");
    shadowed.set_shadow_offset_x(6.0);
    shadowed.set_shadow_offset_y(6.0);
    shadowed.fill_text("M", 40.0, 100.0).expect("draw");

    assert_eq!(ink_rows(&shadowed).expect("ink"), plain_rows);
}
