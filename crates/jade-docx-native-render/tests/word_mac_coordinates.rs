//! Unadjusted Mac layout coordinates, using only redistributable bundled faces.
mod common;
use common::repo_path;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_native_render::{FaceSpec, NativeDocument};
use serde_json::{json, Value};

fn capture(bytes: &[u8]) -> Value {
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let regular = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let bold = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Bold.ttf");
    let mut faces = Vec::new();
    for family in [
        "Noto Sans JP",
        "Times New Roman",
        "Yu Mincho",
        "MS Mincho",
        "MS Gothic",
        "MS PGothic",
        "Century",
        "Calibri",
        "Arial",
        "serif",
    ] {
        faces.push(FaceSpec {
            family,
            weight: 400,
            path: &regular,
        });
        faces.push(FaceSpec {
            family,
            weight: 700,
            path: &bold,
        });
    }
    let doc = NativeDocument::open(bytes, 816.0, 1056.0, &faces).unwrap();
    let mut glyphs = Vec::new();
    for line in doc.renderer().layout_debug_lines() {
        for fragment in line.fragments {
            for glyph in fragment.glyphs {
                if glyph.text.is_empty() || glyph.text.chars().all(char::is_control) {
                    continue;
                }
                glyphs.push(json!([
                    line.page + 1,
                    glyph.text,
                    (fragment.x + glyph.x - 2.0 * line.report_x_offset) * 0.75,
                    fragment.baseline * 0.75
                ]));
            }
        }
    }
    json!({"page_count": doc.page_count(), "glyphs": glyphs})
}

// Whole probes already within half a CSS pixel of the Word for Mac PDF.
const STRICT_WORD_FIXTURES: &[&str] = &[
    "word-mac-probe-pairs-kerned",
    "word-mac-probe-squeeze-comma",
    "word-mac-probe-squeeze-conditions-compress",
    "word-mac-probe-squeeze-conditions-nocompress",
    "word-mac-probe-squeeze-k40-b3",
    "word-mac-probe-squeeze-k5-b2",
    "word-mac-probe-squeeze-plain",
    "word-mac-probe-tab-shrink-comma",
    "word-mac-probe-tab-shrink-kanji",
];

const SNAPSHOT: &str = "tests/word-oracle/mac-native-coordinates.json";
const SNAPSHOT_TOLERANCE_PT: f64 = 0.01;
const WORD_TOLERANCE_PT: f64 = 0.375; // Half a CSS pixel, without origin alignment.

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn compare(expected: &Value, actual: &Value, tolerance: f64) -> Result<(), String> {
    if expected["page_count"] != actual["page_count"] {
        return Err(format!(
            "page count: {} -> {}",
            expected["page_count"], actual["page_count"]
        ));
    }
    let expected = expected["glyphs"].as_array().expect("expected glyphs");
    let actual = actual["glyphs"].as_array().expect("captured glyphs");
    if expected.is_empty() || actual.is_empty() || expected.len() != actual.len() {
        return Err(format!(
            "glyph count: {} -> {} (must be nonzero)",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        if expected[0] != actual[0] || expected[1] != actual[1] {
            return Err(format!("glyph {index}: page/text {expected} -> {actual}"));
        }
        for axis in [2, 3] {
            let old = expected[axis].as_f64().expect("reference coordinate");
            let new = actual[axis].as_f64().expect("captured coordinate");
            if !old.is_finite() || !new.is_finite() || (new - old).abs() > tolerance {
                return Err(format!(
                    "glyph {index} {}: {} delta {:.6}pt exceeds {tolerance}pt ({old} -> {new})",
                    actual[1],
                    if axis == 2 { "x" } else { "baseline" },
                    new - old
                ));
            }
        }
    }
    Ok(())
}

/// PDF extraction omits whitespace and object placeholders. Compare every other
/// glyph, sorting by page/text/position only to remove paint-order differences.
/// No coordinate is translated, rescaled or replaced by an oracle value.
fn spatial_order(layout: &Value) -> Value {
    let mut glyphs: Vec<_> = layout["glyphs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|glyph| {
            glyph[1]
                .as_str()
                .unwrap()
                .chars()
                .any(|c| !c.is_whitespace() && c != '\u{fffc}')
        })
        .cloned()
        .collect();
    glyphs.sort_by(|a, b| {
        a[0].as_u64()
            .cmp(&b[0].as_u64())
            .then_with(|| a[1].as_str().cmp(&b[1].as_str()))
            .then_with(|| a[3].as_f64().unwrap().total_cmp(&b[3].as_f64().unwrap()))
            .then_with(|| a[2].as_f64().unwrap().total_cmp(&b[2].as_f64().unwrap()))
    });
    json!({"page_count": layout["page_count"], "glyphs": glyphs})
}

fn word_layout(reference: &Value) -> Value {
    let mut glyphs = Vec::new();
    for page in reference["pages"].as_array().unwrap() {
        for row in page["rows"].as_array().unwrap() {
            for character in row["characters"].as_array().unwrap() {
                glyphs.push(json!([
                    page["page"],
                    character["text"],
                    character["x_pt"],
                    row["baseline_pt"]
                ]));
            }
        }
    }
    spatial_order(&json!({"page_count": reference["page_count"], "glyphs": glyphs}))
}

#[test]
fn mac_probe_coordinates() {
    let mut paths = Vec::new();
    for directory in ["mac-probes-16114", "mac-probes-20261019"] {
        for entry in
            std::fs::read_dir(repo_path(&format!("tests/word-oracle/{directory}"))).unwrap()
        {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "json") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    assert_eq!(
        paths.len(),
        32,
        "review coverage when adding/removing Mac probes"
    );
    let mut fonts = serde_json::Map::new();
    for weight in ["Regular", "Bold"] {
        let path = format!("fonts/Noto_Sans_JP/static/NotoSansJP-{weight}.ttf");
        fonts.insert(
            path.clone(),
            json!(sha256(
                &std::fs::read(repo_path(&path)).expect("bundled font must exist")
            )),
        );
    }
    let mut fixtures = serde_json::Map::new();
    let mut direct_word_failures = Vec::new();
    for path in paths {
        let name = path.file_stem().unwrap().to_str().unwrap();
        let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
        let reference_bytes = std::fs::read(&path).unwrap();
        let reference: Value = serde_json::from_slice(&reference_bytes).unwrap();
        assert_eq!(
            reference["docx_sha256"],
            sha256(&bytes),
            "{name}: recapture Word reference after changing fixture"
        );
        let layout = capture(&bytes);
        assert!(
            !layout["glyphs"].as_array().unwrap().is_empty(),
            "{name}: empty capture"
        );
        // STRICT_WORD_FIXTURES is deliberately reviewed, not discovered at test time.
        if STRICT_WORD_FIXTURES.contains(&name) {
            if let Err(error) = compare(
                &word_layout(&reference),
                &spatial_order(&layout),
                WORD_TOLERANCE_PT,
            ) {
                direct_word_failures.push(format!("{name}: {error}"));
            }
        }
        assert!(fixtures.insert(name.to_owned(), json!({
            "docx_sha256": sha256(&bytes), "word_reference_sha256": sha256(&reference_bytes), "layout": layout
        })).is_none(), "duplicate fixture {name}");
    }
    for name in STRICT_WORD_FIXTURES {
        assert!(
            fixtures.contains_key(*name),
            "missing strict Word probe {name}"
        );
    }
    let actual = json!({"schema": 1, "fonts": fonts, "fixtures": fixtures});
    // Explicit local maintenance only. CI can never bless its own regressions.
    if std::env::var("JADE_UPDATE_WORD_MAC_SNAPSHOT").as_deref() == Ok("1") {
        assert!(
            std::env::var_os("CI").is_none(),
            "snapshot updates forbidden in CI"
        );
        assert!(
            direct_word_failures.is_empty(),
            "{}",
            direct_word_failures.join("\n")
        );
        std::fs::write(
            repo_path(SNAPSHOT),
            serde_json::to_string(&actual).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let expected: Value = serde_json::from_slice(
        &std::fs::read(repo_path(SNAPSHOT)).expect("committed snapshot must exist"),
    )
    .unwrap();
    let mut failures = direct_word_failures;
    assert_eq!(expected["schema"], actual["schema"]);
    assert_eq!(
        expected["fonts"], actual["fonts"],
        "bundled font changed: review and remeasure"
    );
    let expected_fixtures = expected["fixtures"].as_object().unwrap();
    let actual_fixtures = actual["fixtures"].as_object().unwrap();
    assert_eq!(
        expected_fixtures.keys().collect::<Vec<_>>(),
        actual_fixtures.keys().collect::<Vec<_>>()
    );
    for (name, actual) in actual_fixtures {
        let expected = &expected_fixtures[name];
        for key in ["docx_sha256", "word_reference_sha256"] {
            assert_eq!(
                expected[key], actual[key],
                "{name}: {key} changed: review and remeasure"
            );
        }
        if let Err(error) = compare(
            &expected["layout"],
            &actual["layout"],
            SNAPSHOT_TOLERANCE_PT,
        ) {
            failures.push(format!("{name}: {error}"));
        }
    }
    if !failures.is_empty() {
        let directory = repo_path("crates/jade-docx-native-render/target/word-mac-coordinates");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("actual.json"),
            serde_json::to_string(&actual).unwrap(),
        )
        .unwrap();
        std::fs::write(directory.join("differences.txt"), failures.join("\n")).unwrap();
        panic!(
            "{} coordinate regression(s):\n{}\nFull capture: {}",
            failures.len(),
            failures
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
            directory.display()
        );
    }
}

#[test]
fn comparator_rejects_shifted_missing_reordered_or_repaginated_glyphs() {
    let expected =
        json!({"page_count": 1, "glyphs": [[1, "あ", 72.0, 81.6], [1, "い", 82.5, 81.6]]});
    assert!(compare(&expected, &expected, SNAPSHOT_TOLERANCE_PT).is_ok());
    for (field, value) in [
        (0, json!(2)),
        (1, json!("う")),
        (2, json!(72.02)),
        (3, json!(81.62)),
    ] {
        let mut shifted = expected.clone();
        shifted["glyphs"][0][field] = value;
        assert!(compare(&expected, &shifted, SNAPSHOT_TOLERANCE_PT).is_err());
    }
    let mut reordered = expected.clone();
    reordered["glyphs"].as_array_mut().unwrap().reverse();
    assert!(compare(&expected, &reordered, SNAPSHOT_TOLERANCE_PT).is_err());
    let mut missing = expected.clone();
    missing["glyphs"].as_array_mut().unwrap().pop();
    assert!(compare(&expected, &missing, SNAPSHOT_TOLERANCE_PT).is_err());
    let mut extra_page = expected.clone();
    extra_page["page_count"] = json!(2);
    assert!(compare(&expected, &extra_page, SNAPSHOT_TOLERANCE_PT).is_err());
    assert!(compare(
        &json!({"page_count": 1, "glyphs": []}),
        &json!({"page_count": 1, "glyphs": []}),
        SNAPSHOT_TOLERANCE_PT
    )
    .is_err());
    let mut shifted = expected.clone();
    shifted["glyphs"][0][2] = json!(72.5);
    assert!(compare(&expected, &shifted, WORD_TOLERANCE_PT).is_err());
}
