//! Locks bold-with-no-bold-face measurement against Chromium's fake bold.
//!
//! `tests/fixtures/chromium_fake_bold.json` (regenerate with
//! `tests/playwright/chromium-fake-bold-capture.mjs`) records canvas
//! `measureText` for a family that registers ONLY a regular face: Chromium
//! then paints a synthetic ("fake") bold, and the recorded truth is that this
//! does NOT change advance widths — fake bold is a paint-time stroke
//! embolden. `ShapingMeasurer` falls back to the registered regular face for
//! a bold descriptor, which reproduces exactly these widths; widening
//! advances to "synthesize" bold would move every bold run off Chromium's
//! line breaks.
#![cfg(feature = "native-shaping")]

use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::layout_cache::TextMeasurer;
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use std::path::PathBuf;

const TOLERANCE_PX: f64 = 0.01;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chromium_fake_bold.json")
}

fn bundled_font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

/// The family under test registers ONLY the regular face, mirroring the
/// capture page's single `@font-face` rule.
fn regular_only_measurer() -> ShapingMeasurer {
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
    measurer
        .register_font_path(
            "Fake Bold Probe",
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
    fake_bold_width: f64,
}

fn load_entries() -> Vec<Entry> {
    let raw = std::fs::read_to_string(fixture_path()).expect("read fake-bold fixture");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture");
    parsed["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|entry| Entry {
            text: entry[0].as_str().expect("text").to_string(),
            size: entry[1].as_f64().expect("size"),
            regular_width: entry[2].as_f64().expect("regular width"),
            fake_bold_width: entry[3].as_f64().expect("fake bold width"),
        })
        .collect()
}

#[test]
fn chromium_fake_bold_does_not_change_advances() {
    // The fixture itself must hold the empirical claim this test exists for.
    for entry in load_entries() {
        assert!(
            (entry.regular_width - entry.fake_bold_width).abs() < 1e-9,
            "Chromium recorded a fake-bold width delta for {:?} @{}px — \
             re-examine the synthetic bold assumption before touching the \
             measurer",
            entry.text,
            entry.size,
        );
    }
}

#[test]
fn bold_without_a_bold_face_measures_like_chromium_fake_bold() {
    let measurer = regular_only_measurer();
    let entries = load_entries();
    assert!(entries.len() >= 40, "fixture unexpectedly small");

    let mut failures = Vec::new();
    for entry in &entries {
        let bold_descriptor = format!("bold {:0.2}pt 'Fake Bold Probe'", entry.size);
        let actual = measurer
            .measure_text(&entry.text, &bold_descriptor)
            .expect("measure bold descriptor");
        let delta = (actual - entry.fake_bold_width).abs();
        if delta > TOLERANCE_PX {
            failures.push(format!(
                "{:?} @{}: chromium={:.5} shaping={:.5} delta={:.5}",
                entry.text, bold_descriptor, entry.fake_bold_width, actual, delta
            ));
        }

        // The regular descriptor agrees too, pinning that the bold fallback
        // is byte-for-byte the regular measurement.
        let regular_descriptor = format!("{:0.2}pt 'Fake Bold Probe'", entry.size);
        let regular_actual = measurer
            .measure_text(&entry.text, &regular_descriptor)
            .expect("measure regular descriptor");
        if (regular_actual - actual).abs() > 1e-9 {
            failures.push(format!(
                "{:?}: bold fallback diverged from the regular face ({} vs {})",
                entry.text, actual, regular_actual
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "bold-with-no-face must reproduce Chromium's fake bold:\n{}",
        failures.join("\n")
    );
}
