//! Cross-backend layout parity.
//!
//! `ShapingMeasurer` exists so a native shell can lay out documents without a
//! browser. `tests/chromium_parity.rs` proves it reproduces Chromium's advance
//! widths; this test closes the loop by proving those widths drive jade-docx-layout
//! to the same wrapped lines and the same vertical positions as the established
//! `fontdue` backend on real Word fixtures.
//!
//! Asserting backend-against-backend rather than against a copied golden means
//! the expectation cannot drift out of sync with `noto_jp_layout_golden.rs`.
#![cfg(feature = "native-shaping")]

use futures::executor::block_on;
use jade_docx_layout::font_measurer::{
    FontdueLineWrapContext, FontdueMeasurer, RegisteredFontStyle,
};
use jade_docx_layout::shaping_measurer::{ShapingLineWrapContext, ShapingMeasurer, ShapingOptions};
use jade_docx_layout::text_extract::layout_docx_lines_from_bytes;
use std::path::PathBuf;

/// The faces the Word fixtures name, all resolving to the bundled Noto files.
const FACES: &[(&str, u16, &str)] = &[
    ("Noto Sans JP", 400, "NotoSansJP-Regular.ttf"),
    ("Noto Sans JP", 700, "NotoSansJP-Bold.ttf"),
    ("MS Gothic", 400, "NotoSansJP-Regular.ttf"),
    ("ＭＳ ゴシック", 400, "NotoSansJP-Regular.ttf"),
    ("MS Mincho", 400, "NotoSansJP-Regular.ttf"),
    ("ＭＳ 明朝", 400, "NotoSansJP-Regular.ttf"),
];

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name)
}

fn bundled_font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

/// `(paragraph, line index, text, rounded y)` for every laid-out line.
type LineShape = (usize, usize, String, f64);

fn shape_lines<C>(bytes: &[u8], context: &C) -> Vec<LineShape>
where
    C: jade_docx_layout::line_wrap::LineWrapContext,
    C::Error: std::fmt::Debug,
{
    block_on(layout_docx_lines_from_bytes(bytes, context))
        .expect("layout fixture")
        .get_lines()
        .map(|line| {
            (
                line.paragraph_index,
                line.line_index_in_paragraph,
                line.text.clone(),
                (line.y * 1000.0).round() / 1000.0,
            )
        })
        .collect()
}

fn fontdue_lines(bytes: &[u8]) -> Vec<LineShape> {
    let mut measurer = FontdueMeasurer::new(12.0);
    for (family, weight, file) in FACES {
        measurer
            .register_font_path(
                *family,
                *weight,
                RegisteredFontStyle::Normal,
                bundled_font_path(file),
            )
            .expect("register fontdue face");
    }
    shape_lines(bytes, &FontdueLineWrapContext::new(measurer, None))
}

fn shaping_lines(bytes: &[u8]) -> Vec<LineShape> {
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
    for (family, weight, file) in FACES {
        measurer
            .register_font_path(
                *family,
                *weight,
                RegisteredFontStyle::Normal,
                bundled_font_path(file),
            )
            .expect("register shaping face");
    }
    shape_lines(bytes, &ShapingLineWrapContext::new(measurer, None))
}

fn assert_backends_agree(fixture: &str) {
    let bytes = std::fs::read(fixture_path(fixture)).expect("read fixture");
    let expected = fontdue_lines(&bytes);
    let actual = shaping_lines(&bytes);

    assert!(!expected.is_empty(), "{fixture} produced no lines");
    assert_eq!(
        expected.len(),
        actual.len(),
        "{fixture}: line count differs between backends"
    );

    let divergent: Vec<String> = expected
        .iter()
        .zip(actual.iter())
        .filter(|(left, right)| left != right)
        .map(|(left, right)| format!("  fontdue={left:?}\n  shaping={right:?}"))
        .collect();
    assert!(
        divergent.is_empty(),
        "{fixture}: {} of {} lines differ between backends:\n{}",
        divergent.len(),
        expected.len(),
        divergent
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn shaping_backend_matches_fontdue_layout_for_yokohama_page3_only() {
    assert_backends_agree("yokohama_page3_only.docx");
}

/// Full 222-line sweep over the largest Japanese fixture. Ignored by default:
/// the shaping side finishes in tens of milliseconds, but the `fontdue`
/// reference side needs about a minute in a debug build. Run it on demand with
/// `cargo test -p jade-docx-layout --features native-shaping -- --ignored`.
#[test]
#[ignore = "slow: fontdue reference layout dominates the runtime"]
fn shaping_backend_matches_fontdue_layout_for_yokohama_page6() {
    assert_backends_agree("noto-jp-yokohama_page6.docx");
}
