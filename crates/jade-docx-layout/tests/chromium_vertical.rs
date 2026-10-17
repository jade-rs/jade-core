//! Locks `ShapingMeasurer`'s vertical metrics against Chromium.
//!
//! The browser renderer derives line heights from canvas `TextMetrics` on the
//! sample string `"Hgあ"`: `measure_line_box_height` sums
//! `fontBoundingBoxAscent + fontBoundingBoxDescent`, and
//! `measure_font_vertical_metrics` prefers `actualBoundingBoxAscent/Descent`.
//! `tests/fixtures/chromium_vertical_metrics.json` records what Chromium
//! returned for both, so the native backend can be held to the same numbers.
//!
//! Line height drives page breaks, so a drift here moves pagination even when
//! every advance width still agrees.
#![cfg(feature = "native-shaping")]

use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use std::path::PathBuf;

/// Chromium reports the font bounding box in whole pixels, so anything beyond
/// float noise is a real disagreement.
const TOLERANCE_PX: f64 = 0.001;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chromium_vertical_metrics.json")
}

fn bundled_font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

struct Entry {
    descriptor: String,
    font_box: (f64, f64),
    ink_box: (f64, f64),
}

fn load_entries(family_marker: &str) -> Vec<Entry> {
    let raw = std::fs::read_to_string(fixture_path()).expect("read vertical metrics fixture");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture");
    parsed["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|entry| Entry {
            descriptor: entry[0].as_str().expect("descriptor").to_string(),
            font_box: (
                entry[1].as_f64().expect("fbAscent"),
                entry[2].as_f64().expect("fbDescent"),
            ),
            ink_box: (
                entry[3].as_f64().expect("abAscent"),
                entry[4].as_f64().expect("abDescent"),
            ),
        })
        .filter(|entry| entry.descriptor.contains(family_marker))
        .collect()
}

fn noto_measurer() -> ShapingMeasurer {
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
    for (weight, file) in [
        (400, "NotoSansJP-Regular.ttf"),
        (700, "NotoSansJP-Bold.ttf"),
    ] {
        measurer
            .register_font_path(
                "Noto Sans JP",
                weight,
                RegisteredFontStyle::Normal,
                bundled_font_path(file),
            )
            .expect("register face");
    }
    measurer
}

#[test]
fn line_box_height_matches_chromium_font_bounding_box() {
    let measurer = noto_measurer();
    let entries = load_entries("Noto Sans JP");
    assert!(!entries.is_empty(), "no Noto rows in fixture");

    for entry in &entries {
        let expected = entry.font_box.0 + entry.font_box.1;
        let actual = measurer
            .measure_line_box_height(&entry.descriptor)
            .expect("measure line box height");
        assert!(
            (actual - expected).abs() < TOLERANCE_PX,
            "{}: chromium={expected} native={actual}",
            entry.descriptor
        );
    }
}

#[test]
fn vertical_metrics_match_chromium_actual_bounding_box() {
    let measurer = noto_measurer();
    let entries = load_entries("Noto Sans JP");

    for entry in &entries {
        let (ascent, descent) = measurer
            .measure_font_vertical_metrics(&entry.descriptor)
            .expect("measure vertical metrics");
        assert!(
            (ascent - entry.ink_box.0).abs() < TOLERANCE_PX,
            "{} ascent: chromium={} native={ascent}",
            entry.descriptor,
            entry.ink_box.0
        );
        assert!(
            (descent - entry.ink_box.1).abs() < TOLERANCE_PX,
            "{} descent: chromium={} native={descent}",
            entry.descriptor,
            entry.ink_box.1
        );
    }
}

#[test]
fn line_box_height_rounds_each_half_independently() {
    let measurer = noto_measurer();

    // Noto's hhea gives 18.56px ascent and 4.608px descent at 12pt. Rounding the
    // sum would yield 23; Chromium rounds each half, giving 19 + 5.
    let height = measurer
        .measure_line_box_height("12.00pt 'Noto Sans JP'")
        .expect("measure");

    assert!((height - 24.0).abs() < TOLERANCE_PX, "got {height}");
}

/// The font bounding box comes from the *primary* face only, so it is
/// reproducible even when the sample string needs fallback. The ink box is not:
/// Chromium substitutes a system Japanese font for `あ` and synthesises bold,
/// neither of which a registered-faces measurer can reproduce. Ignored rather
/// than skipped so it cannot pass silently without the system font.
#[test]
#[ignore = "requires macOS /System/Library/Fonts/Supplemental/Arial.ttf"]
fn font_bounding_box_matches_chromium_for_a_face_lacking_the_sample_kana() {
    let arial = PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf");
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
    measurer
        .register_font_path("TestArial", 400, RegisteredFontStyle::Normal, &arial)
        .expect("register Arial");

    for entry in load_entries("TestArial") {
        let expected = entry.font_box.0 + entry.font_box.1;
        let actual = measurer
            .measure_line_box_height(&entry.descriptor)
            .expect("measure line box height");
        assert!(
            (actual - expected).abs() < TOLERANCE_PX,
            "{}: chromium={expected} native={actual}",
            entry.descriptor
        );
    }
}
