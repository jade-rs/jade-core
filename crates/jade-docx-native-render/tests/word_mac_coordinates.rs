//! Unadjusted Mac layout coordinates using bundled faces, plus opt-in Office-font probes.
mod common;
use common::repo_path;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_native_render::{FaceSpec, NativeDocument};
use serde_json::{json, Value};

fn open_with_bundled_faces(bytes: &[u8]) -> NativeDocument {
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
    NativeDocument::open(bytes, 816.0, 1056.0, &faces).unwrap()
}

fn capture(bytes: &[u8]) -> Value {
    capture_coordinates(&open_with_bundled_faces(bytes))
}

fn capture_coordinates(doc: &NativeDocument) -> Value {
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

#[test]
fn mac_missing_first_footer_stays_blank() {
    let bytes = std::fs::read(repo_path("public/fixtures/header-footer-sections.docx")).unwrap();
    let reference: Value = serde_json::from_slice(
        &std::fs::read(repo_path(
            "tests/word-oracle/mac-glyph-corpus-20261019/header-footer-sections.json",
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(reference["docx_sha256"], sha256(&bytes));
    let word_pages = reference["pages"].as_array().unwrap();
    let word_has_footer = |page: usize| {
        word_pages[page]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["text"] == "SHAREDFOOTER")
    };
    assert!(!word_has_footer(0));
    assert!(word_has_footer(1));

    let mut doc = open_with_bundled_faces(&bytes);
    let snapshot = doc.capture_full_layout_debug().unwrap();
    assert!(snapshot.complete);
    assert_eq!(snapshot.page_count, 2);
    assert!(snapshot.source_surface_indices_complete);
    assert!(snapshot.text_box_source_inventory_complete);
    assert!(snapshot.known_uncaptured_stories.is_empty());
    let story_text = |page: usize, story: &str| {
        snapshot
            .auxiliary_lines
            .iter()
            .filter(|entry| entry.page == page && entry.story == story)
            .flat_map(|entry| &entry.line.fragments)
            .flat_map(|fragment| &fragment.glyphs)
            .map(|glyph| glyph.text.as_str())
            .collect::<String>()
    };
    assert_eq!(
        story_text(0, "footer"),
        "",
        "titlePg must not use the default footer"
    );
    assert_eq!(
        story_text(1, "footer"),
        "SHARED FOOTER",
        "default footer still inherits into section two"
    );
    assert_eq!(story_text(0, "header"), "TITLE PAGE HEADER");
    assert_eq!(story_text(1, "header"), "SECTION TWO HEADER");
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
                    character.get("baseline_pt").unwrap_or(&row["baseline_pt"])
                ]));
            }
        }
    }
    spatial_order(&json!({"page_count": reference["page_count"], "glyphs": glyphs}))
}

#[test]
fn mac_probe_coordinates() {
    let mut paths = Vec::new();
    for directory in [
        "mac-probes-16114",
        "mac-probes-20261019",
        "mac-floating-horizontal-20261030",
    ] {
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
        34,
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
fn word_reference_preserves_glyph_baselines_and_legacy_row_fallback() {
    let reference = json!({
        "page_count": 1,
        "pages": [{"page": 1, "rows": [{
            "baseline_pt": 80.0,
            "characters": [
                {"text": "A", "x_pt": 72.0, "baseline_pt": 80.0},
                {"text": "B", "x_pt": 80.0, "baseline_pt": 80.6},
                {"text": "C", "x_pt": 88.0}
            ]
        }]}]
    });
    let expected = json!({"page_count": 1, "glyphs": [
        [1, "A", 72.0, 80.0],
        [1, "B", 80.0, 80.6],
        [1, "C", 88.0, 80.0]
    ]});
    let actual = word_layout(&reference);
    assert_eq!(actual, expected);

    let mut flattened = actual.clone();
    flattened["glyphs"][1][3] = json!(80.0);
    assert!(compare(&actual, &flattened, WORD_TOLERANCE_PT).is_err());
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

/// Local opt-in parity check; CI cannot redistribute the Office Arial face.
#[test]
#[ignore = "requires JADE_OFFICE_ARIAL pointing to the captured Office font"]
fn floating_tables_match_word_with_office_arial() {
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let font = std::env::var("JADE_OFFICE_ARIAL")
        .expect("set JADE_OFFICE_ARIAL to Office Arial arial.ttf");
    assert_eq!(
        sha256(&std::fs::read(&font).unwrap()),
        "15663c7e8495594c6136eb51b11983eb8f5bbcdd07bb2c327e0d1e83415b0f06",
        "font differs from the Word capture; remeasure rather than fitting coordinates"
    );
    let faces = [FaceSpec {
        family: "Arial",
        weight: 400,
        path: std::path::Path::new(&font),
    }];
    for (directory, name) in [
        ("mac-probes-20261019", "word-mac-probe-floating-order"),
        ("mac-probes-20261019", "word-mac-probe-float-lookahead"),
        (
            "mac-floating-horizontal-20261030",
            "word-mac-probe-floating-horizontal-14",
        ),
        (
            "mac-floating-horizontal-20261030",
            "word-mac-probe-floating-horizontal-15",
        ),
    ] {
        let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
        let reference: Value = serde_json::from_slice(
            &std::fs::read(repo_path(&format!(
                "tests/word-oracle/{directory}/{name}.json"
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            reference["docx_sha256"],
            sha256(&bytes),
            "changed source {name}"
        );
        let doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
        compare(
            &word_layout(&reference),
            &spatial_order(&capture_coordinates(&doc)),
            WORD_TOLERANCE_PT,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

/// Local opt-in parity check for the spaces beside inline pictures, in the
/// Yu Mincho Word for Mac ships (`Contents/Resources/DFonts/yumin.ttf`).
#[test]
#[ignore = "requires JADE_OFFICE_YU_MINCHO pointing to the captured Office font"]
fn object_spaces_match_word_with_office_yu_mincho() {
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let font = std::env::var("JADE_OFFICE_YU_MINCHO")
        .expect("set JADE_OFFICE_YU_MINCHO to Office Yu Mincho yumin.ttf");
    assert_eq!(
        sha256(&std::fs::read(&font).unwrap()),
        "926c1f98c3c603656222e0122c6bdabe17817d1c284a69abd4c337276dde9233",
        "font differs from the Word capture; remeasure rather than fitting coordinates"
    );
    let path = std::path::Path::new(&font);
    let faces = ["Yu Mincho", "游明朝"].map(|family| FaceSpec {
        family,
        weight: 400,
        path,
    });
    for (directory, name) in [
        ("mac-probes-20261019", "word-mac-probe-object-spaces"),
        (
            "mac-probes-20261030",
            "word-mac-probe-object-space-runs-balanced",
        ),
        (
            "mac-probes-20261030",
            "word-mac-probe-object-space-runs-unbalanced",
        ),
    ] {
        let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
        let reference: Value = serde_json::from_slice(
            &std::fs::read(repo_path(&format!(
                "tests/word-oracle/{directory}/{name}.json"
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            reference["docx_sha256"],
            sha256(&bytes),
            "changed source {name}"
        );
        let doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
        compare(
            &word_layout(&reference),
            &spatial_order(&capture_coordinates(&doc)),
            WORD_TOLERANCE_PT,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
#[ignore = "requires JADE_OFFICE_YU_MINCHO pointing to the captured Office font"]
fn nested_autofit_word_matches_word_with_office_yu_mincho() {
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let font = std::env::var("JADE_OFFICE_YU_MINCHO").expect("set JADE_OFFICE_YU_MINCHO");
    assert_eq!(
        sha256(&std::fs::read(&font).unwrap()),
        "926c1f98c3c603656222e0122c6bdabe17817d1c284a69abd4c337276dde9233",
        "font differs from the frozen Word capture"
    );
    let path = std::path::Path::new(&font);
    let faces = ["Yu Mincho", "游明朝"].map(|family| FaceSpec {
        family,
        weight: 400,
        path,
    });
    let bytes = std::fs::read(repo_path("public/fixtures/table-nested.docx")).unwrap();
    let reference: Value = serde_json::from_slice(
        &std::fs::read(repo_path(
            "tests/word-oracle/mac-glyph-corpus-20261019/table-nested.json",
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(reference["docx_sha256"], sha256(&bytes));
    let doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
    compare(
        &word_layout(&reference),
        &spatial_order(&capture_coordinates(&doc)),
        WORD_TOLERANCE_PT,
    )
    .unwrap();
}

/// Freeze the painted stories that the legacy body-selection capture omits.
/// These bundled-font coordinates are regression evidence, not Word agreement.
#[test]
fn mac_numbering_and_ruby_coordinates() {
    const SNAPSHOT: &str = "tests/word-oracle/mac-native-auxiliary-coordinates.json";
    const PROBES: &[(&str, &str, usize)] = &[
        ("word-mac-probe-numbering-label", "numbering", 9),
        ("word-mac-probe-numbering-label-faces", "numbering", 5),
        ("word-mac-probe-ruby-grid", "ruby", 15),
        ("word-mac-probe-ruby-nogrid", "ruby", 15),
        ("word-mac-probe-ruby-wide-grid", "ruby", 20),
        ("word-mac-probe-ruby-wide-nogrid", "ruby", 20),
    ];
    let mut fixtures = serde_json::Map::new();
    for &(name, story, count) in PROBES {
        let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
        let reference_bytes = std::fs::read(repo_path(&format!(
            "tests/word-oracle/mac-probes-16114/{name}.json"
        )))
        .unwrap();
        let reference: Value = serde_json::from_slice(&reference_bytes).unwrap();
        assert_eq!(
            reference["docx_sha256"],
            sha256(&bytes),
            "{name}: source/reference mismatch"
        );
        let mut doc = open_with_bundled_faces(&bytes);
        let before = capture_coordinates(&doc);
        let snapshot = doc.capture_full_layout_debug().unwrap();
        assert!(snapshot.complete, "{name}: incomplete layout");
        assert!(
            snapshot.known_uncaptured_stories.is_empty(),
            "{name}: uncaptured stories {:?}",
            snapshot.known_uncaptured_stories
        );
        assert!(
            snapshot.source_surface_indices_complete,
            "{name}: incomplete source indices"
        );
        assert!(
            snapshot.text_box_source_inventory_complete,
            "{name}: incomplete text-box inventory"
        );
        assert_eq!(
            snapshot.auxiliary_lines.len(),
            count,
            "{name}: painted label/guide count"
        );
        let mut glyphs = Vec::new();
        for entry in &snapshot.auxiliary_lines {
            assert_eq!(entry.story, story, "{name}: unexpected story");
            let line = &entry.line;
            assert_eq!(entry.page, line.page, "{name}: inconsistent page");
            assert!(!line.text.is_empty(), "{name}: empty label/guide");
            for fragment in &line.fragments {
                for glyph in &fragment.glyphs {
                    if glyph.text.is_empty() || glyph.text.chars().all(char::is_control) {
                        continue;
                    }
                    let x = (fragment.x + glyph.x - 2.0 * line.report_x_offset) * 0.75;
                    let y = fragment.baseline * 0.75;
                    assert!(
                        x.is_finite() && y.is_finite(),
                        "{name}: non-finite coordinate"
                    );
                    glyphs.push(json!([entry.page + 1, glyph.text, x, y]));
                }
            }
        }
        assert!(!glyphs.is_empty(), "{name}: empty painted capture");
        // Diagnostics must preserve the editable body's coordinates and repeat
        // every guide/label even after warming the renderer's paragraph cache.
        assert!(
            compare(&before, &capture_coordinates(&doc), SNAPSHOT_TOLERANCE_PT).is_ok(),
            "{name}: diagnostic capture changed body coordinates"
        );
        let repeated = doc.capture_full_layout_debug().unwrap();
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            serde_json::to_value(&repeated).unwrap(),
            "{name}: repeated full capture differs"
        );
        fixtures.insert(name.to_owned(), json!({
            "docx_sha256": sha256(&bytes), "word_reference_sha256": sha256(&reference_bytes), "story": story,
            "layout": {"page_count": snapshot.page_count, "glyphs": glyphs}
        }));
    }
    let mut fonts = serde_json::Map::new();
    for weight in ["Regular", "Bold"] {
        let path = format!("fonts/Noto_Sans_JP/static/NotoSansJP-{weight}.ttf");
        fonts.insert(
            path.clone(),
            json!(sha256(&std::fs::read(repo_path(&path)).unwrap())),
        );
    }
    let actual = json!({"schema": 1, "fonts": fonts, "fixtures": fixtures});
    if std::env::var("JADE_UPDATE_WORD_MAC_AUXILIARY_SNAPSHOT").as_deref() == Ok("1") {
        assert!(
            std::env::var_os("CI").is_none(),
            "snapshot updates forbidden in CI"
        );
        std::fs::write(
            repo_path(SNAPSHOT),
            serde_json::to_string(&actual).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let expected: Value =
        serde_json::from_slice(&std::fs::read(repo_path(SNAPSHOT)).unwrap()).unwrap();
    assert_eq!(expected["schema"], actual["schema"]);
    assert_eq!(expected["fonts"], actual["fonts"], "font identity changed");
    let expected_fixtures = expected["fixtures"].as_object().unwrap();
    let actual_fixtures = actual["fixtures"].as_object().unwrap();
    assert_eq!(
        expected_fixtures.keys().collect::<Vec<_>>(),
        actual_fixtures.keys().collect::<Vec<_>>()
    );
    for (name, actual) in actual_fixtures {
        let expected = &expected_fixtures[name];
        for key in ["docx_sha256", "word_reference_sha256", "story"] {
            assert_eq!(expected[key], actual[key], "{name}: {key} changed");
        }
        compare(
            &expected["layout"],
            &actual["layout"],
            SNAPSHOT_TOLERANCE_PT,
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}
