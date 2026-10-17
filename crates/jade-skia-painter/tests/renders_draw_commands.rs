//! End-to-end proof that a document's draw commands rasterise without a browser.
//!
//! The same `DrawCommand` stream the browser replays for cached paragraphs is
//! executed here against Skia. Nothing in `jade-docx-editor`'s executor knows which
//! backend it is talking to, so a pass here means the `Painter` trait carries
//! enough information to paint, not just to be recorded.

use jade_docx_editor::draw_command::executor::execute_commands;
use jade_docx_editor::draw_command::{DrawCommand, GlyphRunStep, TextStyle};
use jade_skia_painter::SkiaPainter;
use std::path::PathBuf;
use std::rc::Rc;

const FONT: &str = "12.00pt 'Noto Sans JP'";

fn noto_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name);
    std::fs::read(path).expect("read bundled Noto Sans JP")
}

fn painter(width: i32, height: i32) -> SkiaPainter {
    let painter = SkiaPainter::new(width, height).expect("create raster surface");
    painter
        .register_font_bytes("Noto Sans JP", 400, &noto_bytes("NotoSansJP-Regular.ttf"))
        .expect("register regular face");
    painter
        .register_font_bytes("Noto Sans JP", 700, &noto_bytes("NotoSansJP-Bold.ttf"))
        .expect("register bold face");
    painter
}

fn style() -> TextStyle {
    TextStyle {
        font_size_pt: 12.0,
        font_descriptor_east_asia: Rc::from(FONT),
        font_descriptor_latin_ascii: Rc::from(FONT),
        fill_color: Rc::from("#101820"),
        ..TextStyle::default()
    }
}

/// Writes the rendered page next to the test binary so a human can look at it.
fn save_png(painter: &SkiaPainter, name: &str) -> PathBuf {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(name);
    std::fs::create_dir_all(out.parent().expect("parent")).ok();
    std::fs::write(&out, painter.encode_png().expect("encode png")).expect("write png");
    out
}

#[test]
fn fill_rect_paints_the_requested_colour() {
    let painter = painter(80, 40);

    execute_commands(
        &painter,
        &[DrawCommand::FillRect {
            x: 10.0,
            y: 10.0,
            width: 40.0,
            height: 20.0,
            color: Rc::from("#ff0000"),
        }],
    )
    .expect("skia backend is infallible");

    assert_eq!(
        painter.pixel(30, 20),
        Some((255, 0, 0, 255)),
        "inside the rect"
    );
    assert_eq!(
        painter.pixel(5, 20),
        Some((255, 255, 255, 255)),
        "outside it"
    );
}

#[test]
fn text_commands_put_ink_on_the_surface() {
    let painter = painter(360, 80);

    execute_commands(
        &painter,
        &[DrawCommand::Text {
            text: Rc::from("横浜市市税条例 Hgあ"),
            x: 10.0,
            y: 40.0,
            style: style(),
        }],
    )
    .expect("infallible");

    let path = save_png(&painter, "text-command.png");
    // Glyphs are antialiased, so assert on "not the white background" over the
    // band the baseline runs through rather than on an exact colour.
    let inked = (10..350)
        .filter(|x| {
            (20..45).any(|y| {
                painter
                    .pixel(*x, y)
                    .is_some_and(|(r, g, b, _)| r < 200 && g < 200 && b < 200)
            })
        })
        .count();
    assert!(
        inked > 50,
        "expected glyph ink across the line, got {inked} columns ({path:?})"
    );
}

#[test]
fn a_glyph_run_lands_at_its_declared_offsets() {
    let painter = painter(360, 80);
    let text = "横浜市市税条例";
    let advance = 16.0; // 12pt full-width cell
    let glyphs: Vec<GlyphRunStep> = text
        .char_indices()
        .enumerate()
        .map(|(index, (offset, ch))| GlyphRunStep {
            text_start: offset as u32,
            text_end: (offset + ch.len_utf8()) as u32,
            font_index: 0,
            x_offset: index as f64 * advance,
        })
        .collect();

    execute_commands(
        &painter,
        &[DrawCommand::TextRun {
            text: Rc::from(text),
            x: 20.0,
            y: 40.0,
            style: style(),
            fonts: vec![Rc::from(FONT)],
            glyphs,
        }],
    )
    .expect("infallible");

    let path = save_png(&painter, "glyph-run.png");
    let ink_in = |from: i32, to: i32| {
        (from..to).any(|x| {
            (20..45).any(|y| {
                painter
                    .pixel(x, y)
                    .is_some_and(|(r, g, b, _)| r < 200 && g < 200 && b < 200)
            })
        })
    };
    // Ink where the run was placed, and none before it.
    assert!(
        ink_in(20, 20 + 7 * advance as i32),
        "run should be inked ({path:?})"
    );
    assert!(
        !ink_in(0, 18),
        "nothing should be painted left of the run ({path:?})"
    );
}

#[test]
fn decorations_stroke_in_the_text_colour() {
    let painter = painter(120, 40);

    execute_commands(
        &painter,
        &[DrawCommand::Underline {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            font_size_pt: 12.0,
            color: Rc::from("#c00000"),
            kind: None,
        }],
    )
    .expect("infallible");

    let path = save_png(&painter, "underline.png");
    let reddish = (10..90)
        .filter(|x| {
            (20..30).any(|y| {
                painter
                    .pixel(*x, y)
                    .is_some_and(|(r, g, b, _)| r > 120 && g < 120 && b < 120)
            })
        })
        .count();
    assert!(
        reddish > 60,
        "underline should span the width in its colour, got {reddish} ({path:?})"
    );
}

/// Paints one underline of `kind` and returns, for each pixel row, how many
/// dark pixels and how many dark→light transitions ("runs") it holds. Word's
/// underline kinds are distinguishable from these two numbers alone: solid
/// rules are one long run, dotted/dashed rules are many runs, double rules ink
/// two separated row bands, waves spread ink across several rows.
fn underline_row_profile(kind: &str) -> Vec<(i32, usize, usize)> {
    let painter = painter(140, 48);
    execute_commands(
        &painter,
        &[DrawCommand::Underline {
            x: 10.0,
            y: 20.0,
            width: 120.0,
            font_size_pt: 12.0,
            color: Rc::from("#101820"),
            kind: Some(Rc::from(kind)),
        }],
    )
    .expect("infallible");
    save_png(&painter, &format!("underline-{kind}.png"));

    let mut rows = Vec::new();
    for y in 20..34 {
        let mut ink = 0usize;
        let mut runs = 0usize;
        let mut in_run = false;
        for x in 5..135 {
            // Count antialiased edge pixels as ink too: a 2px rule's outer
            // rows blend toward the paper and would vanish under a strict
            // darkness cut, making thick indistinguishable from single.
            let dark = painter
                .pixel(x, y)
                .is_some_and(|(r, g, b, _)| (r as u32 + g as u32 + b as u32) < 600);
            if dark {
                ink += 1;
                if !in_run {
                    runs += 1;
                }
            }
            in_run = dark;
        }
        if ink > 0 {
            rows.push((y, ink, runs));
        }
    }
    rows
}

/// `w:u w:val="double"` (§17.3.2.40): two rules separated by a visible gap.
#[test]
fn double_underline_paints_two_separated_rules() {
    let rows = underline_row_profile("double");
    assert!(
        rows.iter().all(|(_, _, runs)| *runs == 1),
        "double rules are solid: {rows:?}"
    );
    // Two bands: at least one empty pixel row strictly between inked rows.
    let ys: Vec<i32> = rows.iter().map(|(y, _, _)| *y).collect();
    let has_gap = ys.windows(2).any(|pair| pair[1] - pair[0] > 1);
    assert!(
        ys.len() >= 2 && has_gap,
        "expected two rule bands with a gap between them: {rows:?}"
    );
}

/// `w:val="dotted"`: the rule is broken into many short marks.
#[test]
fn dotted_underline_paints_many_marks() {
    let rows = underline_row_profile("dotted");
    let max_runs = rows.iter().map(|(_, _, runs)| *runs).max().unwrap_or(0);
    assert!(
        max_runs >= 10,
        "dotted must break into many marks, got {rows:?}"
    );
}

/// `w:val="dash"`: fewer, longer marks than dotted, still not a solid rule.
#[test]
fn dashed_underline_paints_dashes() {
    let rows = underline_row_profile("dash");
    let max_runs = rows.iter().map(|(_, _, runs)| *runs).max().unwrap_or(0);
    assert!(
        (3..40).contains(&max_runs),
        "dash must break the rule into dashes, got {rows:?}"
    );
}

/// `w:val="wave"`: the stroke oscillates across several pixel rows instead of
/// staying inside the one-or-two-row band a straight rule occupies.
#[test]
fn wavy_underline_spreads_across_rows() {
    let rows = underline_row_profile("wave");
    assert!(
        rows.len() >= 4,
        "a wave should ink several adjacent rows, got {rows:?}"
    );
}

/// `w:val="thick"`: a heavier rule inks more rows than the single rule.
#[test]
fn thick_underline_is_heavier_than_single() {
    let single_rows = underline_row_profile("single").len();
    let thick_rows = underline_row_profile("thick").len();
    assert!(
        thick_rows > single_rows,
        "thick ({thick_rows} rows) must outweigh single ({single_rows} rows)"
    );
}
