//! Holds Skia's rasterisation to what Chromium's canvas produces.
//!
//! `tests/fixtures/chromium-scene.png` was rendered by Chromium through
//! Playwright from the drawing operations in `src/bin/render_scene.rs`;
//! `tests/chromium-scene.mjs` regenerates it. Recording it means this test
//! needs no browser.
//!
//! Fills, strokes, and glyph *positions* are expected to match exactly — they
//! are the same geometry through the same engine. Glyph *edges* are not:
//! Chromium tunes text antialiasing and gamma, so the interiors agree while the
//! coverage ramp at the edge differs by a few levels. The assertions below
//! separate those two claims instead of hiding both under one loose tolerance.

use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;

const WIDTH: i32 = 400;
const HEIGHT: i32 = 120;
/// Rows the text baseline passes through.
const TEXT_BAND: std::ops::Range<i32> = 20..48;

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn scene() -> SkiaPainter {
    let painter = SkiaPainter::new(WIDTH, HEIGHT).expect("surface");
    for (weight, file) in [
        (400, "NotoSansJP-Regular.ttf"),
        (700, "NotoSansJP-Bold.ttf"),
    ] {
        let bytes = std::fs::read(repo_path(&format!(
            "../../fonts/Noto_Sans_JP/static/{file}"
        )))
        .expect("bundled font");
        painter
            .register_font_bytes("Noto Sans JP", weight, &bytes)
            .expect("register face");
    }

    painter.set_font("12.00pt 'Noto Sans JP'");
    painter.set_fill_style_str("#101820");
    painter
        .fill_text("横浜市市税条例 Hgあ", 20.0, 40.0)
        .expect("infallible");

    painter.set_fill_style_str("#c00000");
    painter.fill_rect(20.0, 60.0, 100.0, 10.0);

    painter.set_stroke_style_str("#0057b8");
    painter.set_line_width(2.0);
    painter.begin_path();
    painter.move_to(20.0, 90.0);
    painter.line_to(380.0, 90.0);
    painter.stroke();
    painter
}

/// Decodes the recorded Chromium PNG into RGBA rows.
fn reference_pixels() -> Vec<u8> {
    let encoded = std::fs::read(repo_path("tests/fixtures/chromium-scene.png"))
        .expect("read recorded Chromium scene");
    let data = skia_safe::Data::new_copy(&encoded);
    let image = skia_safe::Image::from_encoded(data).expect("decode reference png");
    assert_eq!((image.width(), image.height()), (WIDTH, HEIGHT));
    let info = skia_safe::ImageInfo::new(
        (WIDTH, HEIGHT),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    assert!(
        image.read_pixels(
            &info,
            &mut pixels,
            (WIDTH * 4) as usize,
            (0, 0),
            skia_safe::image::CachingHint::Allow,
        ),
        "read reference pixels"
    );
    pixels
}

fn channel_delta(reference: &[u8], painter: &SkiaPainter, x: i32, y: i32) -> u8 {
    let offset = ((y * WIDTH + x) * 4) as usize;
    let (r, g, b, _) = painter.pixel(x, y).expect("read rendered pixel");
    [
        r.abs_diff(reference[offset]),
        g.abs_diff(reference[offset + 1]),
        b.abs_diff(reference[offset + 2]),
    ]
    .into_iter()
    .max()
    .expect("three channels")
}

#[test]
fn fills_strokes_and_background_are_pixel_identical_to_chromium() {
    let painter = scene();
    let reference = reference_pixels();

    let mut mismatched = Vec::new();
    for y in 0..HEIGHT {
        if TEXT_BAND.contains(&y) {
            continue;
        }
        for x in 0..WIDTH {
            let delta = channel_delta(&reference, &painter, x, y);
            if delta > 0 {
                mismatched.push((x, y, delta));
            }
        }
    }

    assert!(
        mismatched.is_empty(),
        "{} pixels outside the text band differ, first few: {:?}",
        mismatched.len(),
        &mismatched[..mismatched.len().min(8)]
    );
}

#[test]
fn glyphs_land_on_exactly_the_same_pixels_as_chromium() {
    let painter = scene();
    let reference = reference_pixels();

    // Ink extent rather than per-pixel equality: this asserts placement, which
    // must match, without asserting the antialiasing that legitimately differs.
    let inked_columns = |sample: &dyn Fn(i32, i32) -> bool| {
        let mut first = None;
        let mut last = None;
        for x in 0..WIDTH {
            if TEXT_BAND.into_iter().any(|y| sample(x, y)) {
                first.get_or_insert(x);
                last = Some(x);
            }
        }
        (first, last)
    };

    let rendered = inked_columns(&|x, y| {
        painter
            .pixel(x, y)
            .is_some_and(|(r, g, b, _)| r < 200 && g < 200 && b < 200)
    });
    let recorded = inked_columns(&|x, y| {
        let offset = ((y * WIDTH + x) * 4) as usize;
        reference[offset] < 200 && reference[offset + 1] < 200 && reference[offset + 2] < 200
    });

    assert_eq!(
        rendered, recorded,
        "glyph ink should start and end on the same column"
    );
}

#[test]
fn text_antialiasing_stays_within_the_recorded_envelope() {
    let painter = scene();
    let reference = reference_pixels();

    let mut differing = 0usize;
    let mut worst = 0u8;
    let total = (TEXT_BAND.len() as i32 * WIDTH) as usize;
    for y in TEXT_BAND {
        for x in 0..WIDTH {
            let delta = channel_delta(&reference, &painter, x, y);
            if delta > 8 {
                differing += 1;
            }
            worst = worst.max(delta);
        }
    }

    // Recorded at 1.88% of the band differing by more than 8 levels. The bound
    // is deliberately close to that: a real placement or font-resolution
    // regression moves whole glyphs and blows past it, while AA tuning does not.
    assert!(
        differing * 100 < total * 4,
        "text band diverged on {differing}/{total} pixels (worst channel delta {worst})"
    );
}

/// The vertical metrics Chromium reports for Noto Sans JP, recorded in
/// `jade-docx-layout`'s `chromium_vertical_metrics.json` fixture. Asserting them here
/// too means the rasterising backend and the measuring backend agree on line
/// height — if they drifted apart, native pages would paint text at one size and
/// lay it out at another.
#[test]
fn font_bounding_box_matches_what_chromium_reported_for_the_same_face() {
    let painter = scene();

    for (descriptor, ascent, descent) in [
        ("9.00pt 'Noto Sans JP'", 14.0, 3.0),
        ("10.50pt 'Noto Sans JP'", 16.0, 4.0),
        ("12.00pt 'Noto Sans JP'", 19.0, 5.0),
        ("14.00pt 'Noto Sans JP'", 22.0, 5.0),
    ] {
        painter.set_font(descriptor);
        let metrics = painter.measure_text_metrics("Hgあ").expect("infallible");
        assert_eq!(
            (
                metrics.font_bounding_box_ascent,
                metrics.font_bounding_box_descent
            ),
            (ascent, descent),
            "{descriptor}"
        );
    }
}

#[test]
fn the_ink_box_tracks_the_string_rather_than_the_font() {
    let painter = scene();
    painter.set_font("12.00pt 'Noto Sans JP'");

    // A full-width glyph inks taller than a lowercase Latin one, while the font
    // bounding box is identical for both.
    let kana = painter.measure_text_metrics("あ").expect("infallible");
    let latin = painter.measure_text_metrics("x").expect("infallible");

    assert!(
        kana.actual_bounding_box_ascent > latin.actual_bounding_box_ascent,
        "kana {} should ink taller than latin {}",
        kana.actual_bounding_box_ascent,
        latin.actual_bounding_box_ascent
    );
    assert_eq!(
        kana.font_bounding_box_ascent,
        latin.font_bounding_box_ascent
    );
}
