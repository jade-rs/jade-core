//! Locks italic-with-no-italic-face measurement against Chromium's fake italic.
//!
//! `tests/fixtures/chromium_fake_italic.json` (regenerate with
//! `tests/playwright/chromium-fake-italic-capture.mjs`) records canvas
//! `measureText` for a family that registers ONLY a regular upright face:
//! Chromium then paints a synthetic oblique, and the recorded truth is that
//! this does NOT change advance widths — fake italic is a paint-time shear.
//! `ShapingMeasurer` falls back to the registered upright face for an italic
//! descriptor, which reproduces exactly these widths; altering advances to
//! "synthesize" italic would move every italic run off Chromium's line breaks.
//! The PDF renderer relies on the same invariant when it slants italic runs
//! with a text-matrix shear instead of a separate font program.
#![cfg(feature = "native-shaping")]

use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::layout_cache::TextMeasurer;
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use std::path::PathBuf;

const TOLERANCE_PX: f64 = 0.01;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chromium_fake_italic.json")
}

fn bundled_font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

/// The family under test registers ONLY the regular upright face, mirroring
/// the capture page's single `@font-face` rule.
fn upright_only_measurer() -> ShapingMeasurer {
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
    measurer
        .register_font_path(
            "Fake Italic Probe",
            400,
            RegisteredFontStyle::Normal,
            bundled_font_path("NotoSansJP-Regular.ttf"),
        )
        .expect("register regular face");
    measurer
}

struct Entry {
    text: String,
    size: f64,
    regular_width: f64,
    fake_italic_width: f64,
}

fn load_entries() -> Vec<Entry> {
    let raw = std::fs::read_to_string(fixture_path()).expect("read fake-italic fixture");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture");
    parsed["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|entry| Entry {
            text: entry[0].as_str().expect("text").to_string(),
            size: entry[1].as_f64().expect("size"),
            regular_width: entry[2].as_f64().expect("regular width"),
            fake_italic_width: entry[3].as_f64().expect("fake italic width"),
        })
        .collect()
}

#[test]
fn chromium_fake_italic_does_not_change_advances() {
    // The fixture itself must hold the empirical claim this test exists for.
    for entry in load_entries() {
        assert!(
            (entry.regular_width - entry.fake_italic_width).abs() < 1e-9,
            "Chromium recorded a fake-italic width delta for {:?} @{}px — \
             re-examine the synthetic oblique assumption before touching the \
             measurer or the PDF shear",
            entry.text,
            entry.size,
        );
    }
}

#[test]
fn italic_without_an_italic_face_measures_like_chromium_fake_italic() {
    let measurer = upright_only_measurer();
    let entries = load_entries();
    assert!(entries.len() >= 40, "fixture unexpectedly small");

    let mut failures = Vec::new();
    for entry in &entries {
        let italic_descriptor = format!("italic {:0.2}pt 'Fake Italic Probe'", entry.size);
        let actual = measurer
            .measure_text(&entry.text, &italic_descriptor)
            .expect("measure italic descriptor");
        let delta = (actual - entry.fake_italic_width).abs();
        if delta > TOLERANCE_PX {
            failures.push(format!(
                "{:?} @{}: chromium={:.5} shaping={:.5} delta={:.5}",
                entry.text, italic_descriptor, entry.fake_italic_width, actual, delta
            ));
        }

        // The upright descriptor agrees too, pinning that the italic fallback
        // is byte-for-byte the upright measurement.
        let upright_descriptor = format!("{:0.2}pt 'Fake Italic Probe'", entry.size);
        let upright_actual = measurer
            .measure_text(&entry.text, &upright_descriptor)
            .expect("measure upright descriptor");
        if (upright_actual - actual).abs() > 1e-9 {
            failures.push(format!(
                "{:?}: italic fallback diverged from the upright face ({} vs {})",
                entry.text, actual, upright_actual
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "italic-with-no-face must reproduce Chromium's fake italic:\n{}",
        failures.join("\n")
    );
}
