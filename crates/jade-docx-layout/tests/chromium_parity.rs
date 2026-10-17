//! Locks `ShapingMeasurer` against advance widths recorded from Chromium.
//!
//! `tests/fixtures/chromium_metrics.json` was produced by driving Chromium
//! through Playwright and calling canvas `measureText` on the bundled Noto Sans
//! JP faces — the exact measurement the browser renderer performs today. The
//! fixture therefore pins the native backend to the browser's line-breaking
//! inputs; if these widths drift, wrapped lines drift with them.
#![cfg(feature = "native-shaping")]

use jade_docx_layout::font_measurer::RegisteredFontStyle;
use jade_docx_layout::layout_cache::TextMeasurer;
use jade_docx_layout::shaping_measurer::{ShapingMeasurer, ShapingOptions};
use std::path::PathBuf;

/// Widths that agree to well under a hundredth of a pixel are treated as equal;
/// the residual is float noise in the recorded canvas values.
const TOLERANCE_PX: f64 = 0.01;

/// Two shaping differences between Chromium and HarfBuzz remain unexplained and
/// are deliberately not special-cased, because a per-character exception is
/// exactly the kind of rule the layout guide tells us to avoid:
///
/// - a pair opening with `ー` (U+30FC), where Chromium applies a kana kern pair
///   that it applies nowhere else;
/// - a pair involving `…`, where Chromium kerns against full-width Latin.
///
/// Both stay under 1px at document text sizes.
fn is_known_residual(text: &str) -> bool {
    text.starts_with('ー') || text.contains('…')
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/chromium_metrics.json")
}

fn bundled_font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

fn chromium_parity_measurer() -> ShapingMeasurer {
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
    measurer
        .register_font_path(
            "Noto Sans JP",
            400,
            RegisteredFontStyle::Normal,
            bundled_font_path("NotoSansJP-Regular.ttf"),
        )
        .expect("register regular face");
    measurer
        .register_font_path(
            "Noto Sans JP",
            700,
            RegisteredFontStyle::Normal,
            bundled_font_path("NotoSansJP-Bold.ttf"),
        )
        .expect("register bold face");
    measurer
}

struct Entry {
    text: String,
    descriptor: String,
    expected: f64,
}

fn load_entries() -> Vec<Entry> {
    let raw = std::fs::read_to_string(fixture_path()).expect("read chromium metrics fixture");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture");
    parsed["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|entry| {
            let text = entry[0].as_str().expect("text").to_string();
            let size_pt = entry[1].as_f64().expect("size");
            let weight = entry[2].as_u64().expect("weight");
            let expected = entry[3].as_f64().expect("width");
            let prefix = if weight >= 700 { "bold " } else { "" };
            Entry {
                descriptor: format!("{prefix}{size_pt:0.2}pt 'Noto Sans JP'"),
                text,
                expected,
            }
        })
        .collect()
}

#[test]
fn shaping_measurer_reproduces_chromium_advance_widths() {
    let measurer = chromium_parity_measurer();
    let entries = load_entries();
    assert!(entries.len() > 3_000, "fixture unexpectedly small");

    let mut unexpected = Vec::new();
    let mut known = 0_usize;
    let mut worst_known = 0.0_f64;

    for entry in &entries {
        let actual = measurer
            .measure_text(&entry.text, &entry.descriptor)
            .expect("measure");
        let delta = (actual - entry.expected).abs();
        if delta <= TOLERANCE_PX {
            continue;
        }
        if is_known_residual(&entry.text) {
            known += 1;
            worst_known = worst_known.max(delta);
            continue;
        }
        unexpected.push(format!(
            "{:?} @{}: chromium={:.5} shaping={:.5} delta={:.5}",
            entry.text, entry.descriptor, entry.expected, actual, delta
        ));
    }

    assert!(
        unexpected.is_empty(),
        "{} of {} measurements diverge from Chromium beyond {TOLERANCE_PX}px:\n{}",
        unexpected.len(),
        entries.len(),
        unexpected
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        worst_known < 1.0,
        "known residual grew to {worst_known:.5}px, above the 1px bound"
    );
    // Guards against the residual set silently widening.
    assert!(
        known * 200 < entries.len(),
        "known residuals ({known}) exceed 0.5% of {} measurements",
        entries.len()
    );
}

#[test]
fn plain_opentype_shaping_does_not_match_chromium() {
    // Without the browser corrections the backend is measurably different, so a
    // passing parity test above cannot be an artefact of a loose tolerance.
    let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::PLAIN_OPENTYPE);
    measurer
        .register_font_path(
            "Noto Sans JP",
            400,
            RegisteredFontStyle::Normal,
            bundled_font_path("NotoSansJP-Regular.ttf"),
        )
        .expect("register regular face");

    let divergent = load_entries()
        .iter()
        .filter(|entry| !entry.descriptor.starts_with("bold"))
        .filter(|entry| {
            let actual = measurer
                .measure_text(&entry.text, &entry.descriptor)
                .expect("measure");
            (actual - entry.expected).abs() > TOLERANCE_PX
        })
        .count();

    assert!(
        divergent > 500,
        "expected plain shaping to diverge widely, saw only {divergent}"
    );
}
