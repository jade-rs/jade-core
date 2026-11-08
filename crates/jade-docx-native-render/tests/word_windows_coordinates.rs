//! Unaligned Windows Word PDF origins, retaining each captured product version.
//! Full XY tests use installed fonts; portable CI baseline checks use stand-ins.
mod common;
use common::repo_path;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
#[cfg(target_os = "windows")]
use jade_docx_native_render::system_fonts;
use jade_docx_native_render::{FaceSpec, NativeDocument};
use serde_json::json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// Keep the original capture and every explicitly registered Word version.
fn reference_sets() -> Vec<(PathBuf, Value)> {
    let legacy = repo_path("tests/word-oracle/windows-probes-20261101");
    let mut reference: Value =
        serde_json::from_slice(&std::fs::read(legacy.join("coordinates.json")).unwrap()).unwrap();
    reference["_fixture_count"] = Value::from(7);
    reference["_glyph_count"] = Value::from(5106);
    reference["_baseline_line_count"] = Value::from(144);
    let mut sets = vec![(legacy, reference)];
    let root = repo_path("tests/word-oracle/windows");
    let index: Value =
        serde_json::from_slice(&std::fs::read(root.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["schema"], 1);
    assert!(!index["references"].as_array().unwrap().is_empty());
    for entry in index["references"].as_array().unwrap() {
        let directory = root.join(entry["directory"].as_str().unwrap());
        let mut reference: Value = serde_json::from_slice(
            &std::fs::read(directory.join(entry["reference"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        assert_eq!(
            reference["word_product_version"],
            entry["word_product_version"]
        );
        let projection = reference["table_glyph_projection"]
            .as_str()
            .unwrap_or("visible");
        assert!(matches!(projection, "visible" | "unclipped"));
        if projection == "unclipped" {
            let file = entry["visible_reference"]
                .as_str()
                .expect("unclipped origins require visible proof");
            assert!(!file.contains(['/', '\\', ':']));
            let visible: Value =
                serde_json::from_slice(&std::fs::read(directory.join(file)).unwrap()).unwrap();
            for key in [
                "word_product_version",
                "word",
                "capture_id",
                "captured_at_utc",
                "tolerance_pt",
            ] {
                assert_eq!(
                    visible[key], reference[key],
                    "visible Word provenance: {key}"
                );
            }
            assert_eq!(visible["table_glyph_projection"], "visible");
            assert_eq!(visible["pdf_text_flags"], 199);
            let names = |value: &Value| {
                value["fixtures"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|fixture| fixture["name"].as_str().unwrap().to_owned())
                    .collect::<std::collections::BTreeSet<_>>()
            };
            assert_eq!(names(&visible), names(&reference), "visible Word coverage");
            assert_eq!(
                visible["fixtures"].as_array().unwrap().len(),
                reference["fixtures"].as_array().unwrap().len()
            );
            assert_eq!(
                visible["fixtures"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|fixture| fixture["glyphs"].as_array().unwrap().len() as u64)
                    .sum::<u64>(),
                entry["visible_glyph_count"].as_u64().unwrap()
            );
            reference["_visible_reference"] = visible;
        }
        reference["_fixture_count"] = entry["fixture_count"].clone();
        reference["_glyph_count"] = entry["glyph_count"].clone();
        reference["_baseline_line_count"] = entry["baseline_line_count"].clone();
        for fixture in reference["fixtures"].as_array().unwrap() {
            if let Some(pages) = fixture["metafiles"].as_array() {
                assert_eq!(pages.len() as u64, fixture["page_count"].as_u64().unwrap());
                for (index, page) in pages.iter().enumerate() {
                    assert_eq!(page["page"], index + 1);
                    let file = page["file"].as_str().unwrap();
                    assert!(!file.contains(['/', '\\']));
                    let raw = std::fs::read(directory.join(file)).unwrap();
                    assert_eq!(page["sha256"], format!("{:x}", Sha256::digest(raw)));
                }
            }
        }
        sets.push((directory, reference));
    }
    sets
}

fn spatial_order(glyphs: &mut [Value]) {
    glyphs.sort_by(|a, b| {
        a[0].as_u64()
            .cmp(&b[0].as_u64())
            .then_with(|| a[3].as_f64().unwrap().total_cmp(&b[3].as_f64().unwrap()))
    });
    let mut start = 0;
    while start < glyphs.len() {
        let mut end = start + 1;
        while end < glyphs.len()
            && glyphs[end][0] == glyphs[start][0]
            && (glyphs[end][3].as_f64().unwrap() - glyphs[start][3].as_f64().unwrap()).abs() <= 0.25
        {
            end += 1;
        }
        glyphs[start..end]
            .sort_by(|a, b| a[2].as_f64().unwrap().total_cmp(&b[2].as_f64().unwrap()));
        start = end;
    }
}

fn all_glyphs(doc: &mut NativeDocument) -> Vec<Value> {
    all_glyphs_with_table_projection(doc, false)
}

fn all_glyphs_with_table_projection(doc: &mut NativeDocument, unclipped: bool) -> Vec<Value> {
    let mut glyphs = Vec::new();
    let snapshot = if unclipped {
        doc.capture_full_layout_debug_unclipped_tables().unwrap()
    } else {
        doc.capture_full_layout_debug().unwrap()
    };
    assert!(snapshot.complete);
    let mut lines = snapshot.body_lines;
    lines.extend(snapshot.auxiliary_lines.into_iter().map(|entry| entry.line));
    for line in lines {
        for fragment in line.fragments {
            for glyph in fragment.glyphs {
                if glyph.text.is_empty()
                    || glyph
                        .text
                        .chars()
                        .all(|ch| ch.is_whitespace() || ch.is_control())
                {
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
    glyphs
}

// New complete-paint references additionally retain the old visible check.
// Neither projection can silently drop a character, page or absolute origin.
fn glyphs_for_reference(
    doc: &mut NativeDocument,
    reference: &Value,
    fixture: &Value,
) -> Vec<Value> {
    let unclipped = reference["table_glyph_projection"].as_str() == Some("unclipped");
    if unclipped {
        let visible = reference["_visible_reference"]["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|visible| visible["name"] == fixture["name"])
            .expect("visible fixture");
        for key in ["docx_sha256", "pdf_sha256", "pdf_file", "page_count"] {
            assert_eq!(visible[key], fixture[key], "visible source: {key}");
        }
        let mut actual = all_glyphs(doc);
        let mut expected = visible["glyphs"].as_array().unwrap().clone();
        spatial_order(&mut actual);
        spatial_order(&mut expected);
        assert_eq!(visible["page_count"], doc.page_count());
        assert_eq!(
            actual.len(),
            expected.len(),
            "{}: visible missing/extra glyph",
            fixture["name"]
        );
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_eq!(actual[0], expected[0], "visible page");
            assert_eq!(actual[1], expected[1], "visible text/order");
            for axis in [2, 3] {
                assert!(
                    (actual[axis].as_f64().unwrap() - expected[axis].as_f64().unwrap()).abs()
                        <= 0.375,
                    "{}: visible axis {axis}, actual {actual}, Word {expected}",
                    fixture["name"]
                );
            }
        }
    }
    all_glyphs_with_table_projection(doc, unclipped)
}

#[test]
fn unclipped_table_capture_preserves_visible_pixels_and_origins_on_both_platforms() {
    let font = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let faces = ["Arial", "MS Mincho", "Noto Sans JP"].map(|family| FaceSpec {
        family,
        weight: 400,
        path: &font,
    });
    let previous = jade_docx_layout::line_metrics::word_layout_platform();
    for fixture in [
        "table-negative-spacing-unequal-4-compat14",
        "table-positive-spacing-bidi-border-widths-compat14",
        "table-positive-spacing-bidi-border-widths-compat15",
    ] {
        let bytes = std::fs::read(repo_path(&format!(
            "public/fixtures/word-windows-edge-{fixture}.docx"
        )))
        .unwrap();
        for platform in [WordLayoutPlatform::Mac, WordLayoutPlatform::Windows] {
            set_word_layout_platform(platform);
            let mut doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
            let before = all_glyphs(&mut doc);
            let pixels = doc.page_pixels(0).unwrap().0.to_vec();
            let raw = all_glyphs_with_table_projection(&mut doc, true);
            assert!(raw.len() >= before.len());
            if platform == WordLayoutPlatform::Windows {
                assert!(
                    raw.len() > before.len(),
                    "fixture must exercise genuinely clipped glyphs"
                );
            }
            assert!(
                pixels == doc.page_pixels(0).unwrap().0,
                "diagnostic changed visible pixels: {platform:?}"
            );
            assert_eq!(
                before,
                all_glyphs(&mut doc),
                "diagnostic changed visible origins: {platform:?}"
            );
        }
    }
    set_word_layout_platform(previous);
}

/// Opening rules and gaps must survive cache invalidation and DOCX export.
/// Exercise both policies with portable fonts; measured Word origins are
/// checked separately by the installed-font oracle below.
#[test]
fn spaced_tables_preserve_origins_after_edit_export_and_undo() {
    let font = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
    let faces = ["Arial", "MS Mincho", "Noto Sans JP"].map(|family| FaceSpec {
        family,
        weight: 400,
        path: &font,
    });
    for platform in [WordLayoutPlatform::Mac, WordLayoutPlatform::Windows] {
        set_word_layout_platform(platform);
        for (fixture, paragraph, expected_page, expected_text) in [
            ("table-spacing-continuation-compat14", 13, 1, "R3C1 alpha"),
            ("table-spacing-auto-compat14", 13, 0, "R3C1 alpha"),
            (
                "table-spacing-at-least-continuation-compat14",
                13,
                1,
                "R3C1 alpha",
            ),
            ("table-spacing-mixed-borders-compat14", 13, 0, "R3C1 alpha"),
            (
                "table-collapsed-implicit-mixed-borders-compat14",
                53,
                0,
                "R3C1 alpha",
            ),
            (
                "table-spacing-nil-mixed-borders-compat14",
                53,
                0,
                "R3C1 alpha",
            ),
            (
                "table-collapsed-mixed-borders-compat14",
                53,
                0,
                "R3C1 alpha",
            ),
            (
                "table-spacing-missing-width-mixed-borders-compat14",
                53,
                0,
                "R3C1 alpha",
            ),
            (
                "table-spacing-negative-mixed-borders-compat14",
                53,
                0,
                "R3C1 alpha",
            ),
            ("table-negative-spacing-columns-3-compat14", 29, 0, "R1C1"),
            (
                "table-negative-spacing-autofit-bidi-alignment-compat15",
                15,
                0,
                "R1C1",
            ),
            (
                "table-positive-spacing-sizing-autofit-bidi-compat14",
                15,
                0,
                "R1C1",
            ),
            (
                "table-zero-spacing-bidi-border-widths-compat15",
                15,
                0,
                "R1C1",
            ),
            (
                "table-positive-spacing-bidi-border-widths-compat14",
                15,
                0,
                "R1C1",
            ),
            ("table-negative-spacing-unequal-3-compat14", 29, 0, "R1C1"),
            ("table-negative-spacing-row-scopes-compat14", 15, 0, "R1C1"),
            (
                "table-negative-spacing-autofit-unequal-3-compat14",
                29,
                0,
                "R1C1",
            ),
            (
                "table-negative-spacing-bidi-unequal-3-compat14",
                29,
                0,
                "R1C1",
            ),
        ] {
            let bytes = std::fs::read(repo_path(&format!(
                "public/fixtures/word-windows-edge-{fixture}.docx"
            )))
            .unwrap();
            let mut doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
            let mut before = all_glyphs(&mut doc);
            spatial_order(&mut before);
            assert_eq!(
                doc.paragraph_text(paragraph).as_deref(),
                Some(expected_text)
            );
            let caret = doc.place_caret(paragraph, 0).unwrap();
            let page = doc.page_and_offset(caret.document_y).0;
            assert_eq!(page, expected_page, "{platform:?}/{fixture}: third row");
            doc.insert_text(paragraph, 0, "Edited ", page + 1).unwrap();
            let mut edited = all_glyphs(&mut doc);
            spatial_order(&mut edited);
            assert_ne!(edited, before);
            let exported = doc.export_docx().unwrap();
            let mut reopened = NativeDocument::open(&exported, 816.0, 1056.0, &faces).unwrap();
            let mut fresh = all_glyphs(&mut reopened);
            spatial_order(&mut fresh);
            let context = format!("{platform:?}/{fixture}");
            assert_equal_origins(&edited, &fresh, &format!("{context}: export"));
            doc.undo().unwrap();
            let mut restored = all_glyphs(&mut doc);
            spatial_order(&mut restored);
            assert_equal_origins(&restored, &before, &format!("{context}: undo"));
        }
    }
}

fn assert_equal_origins(actual: &[Value], expected: &[Value], context: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context}: missing or extra glyphs"
    );
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(actual, expected, "{context}: glyph {index}");
    }
}

/// Arial is installed on Windows hosted runners. These Word captures need
/// no Office installation or redistribution of Microsoft font files in CI.
#[test]
#[cfg(target_os = "windows")]
fn windows_word_arial_edge_origins() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    assert!(system_fonts::FontLibrary::system().has("Arial"));
    let mut names = std::collections::BTreeSet::new();
    let mut checked = 0;
    for (_, reference) in reference_sets() {
        assert_eq!(reference["tolerance_pt"], 0.375);
        for fixture in reference["fixtures"].as_array().unwrap() {
            let name = fixture["name"].as_str().unwrap();
            if !name.starts_with("word-windows-edge-") {
                continue;
            }
            let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
            assert_eq!(
                fixture["docx_sha256"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            let faces = system_fonts::word_document_faces(&bytes, &[]);
            let mut doc =
                NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces).unwrap();
            assert_eq!(fixture["page_count"], doc.page_count(), "{name}: pages");
            let mut actual = glyphs_for_reference(&mut doc, &reference, fixture);
            let mut expected = fixture["glyphs"].as_array().unwrap().clone();
            spatial_order(&mut actual);
            spatial_order(&mut expected);
            assert_eq!(actual.len(), expected.len(), "{name}: missing/extra glyphs");
            for (actual, expected) in actual.iter().zip(&expected) {
                assert_eq!(actual[0], expected[0], "{name}: page");
                assert_eq!(actual[1], expected[1], "{name}: text/order");
                for axis in [2, 3] {
                    assert!(
                        (actual[axis].as_f64().unwrap() - expected[axis].as_f64().unwrap()).abs()
                            <= 0.375,
                        "{name}: axis {axis}, actual {actual}, Word {expected}"
                    );
                }
            }
            names.insert(name.to_owned());
            checked += expected.len();
        }
    }
    assert_eq!(
        names,
        std::collections::BTreeSet::from_iter(
            [
                "word-windows-edge-indents",
                "word-windows-edge-tabs",
                "word-windows-edge-spacing",
                "word-windows-edge-run-width",
                "word-windows-edge-run-position",
                "word-windows-edge-arial-script-sweep",
                "word-windows-edge-arial-script-sweep-bold",
                "word-windows-edge-arial-script-sweep-italic",
                "word-windows-edge-arial-script-sweep-bold-italic",
                "word-windows-edge-decimal-tracking",
                "word-windows-edge-decimal-suffix",
                "word-windows-edge-decimal-separators",
                "word-windows-edge-table-alignment",
                "word-windows-edge-table-borders",
                "word-windows-edge-table-alignment-compat12",
                "word-windows-edge-table-alignment-compat14",
                "word-windows-edge-table-borders-compat12",
                "word-windows-edge-table-borders-compat14",
                "word-windows-edge-table-indents-compat12",
                "word-windows-edge-table-indents-compat14",
                "word-windows-edge-table-bidi-compat12",
                "word-windows-edge-table-bidi-compat14",
                "word-windows-edge-table-bidi-compat15",
                "word-windows-edge-table-spacing-compat12",
                "word-windows-edge-table-spacing-compat14",
                "word-windows-edge-table-spacing-compat15",
                "word-windows-edge-table-spacing-borders-compat12",
                "word-windows-edge-table-spacing-borders-compat14",
                "word-windows-edge-table-spacing-borders-compat15",
                "word-windows-edge-table-spacing-continuation-compat12",
                "word-windows-edge-table-spacing-continuation-compat14",
                "word-windows-edge-table-spacing-continuation-compat15",
                "word-windows-edge-table-zero-horizontal-borders-compat12",
                "word-windows-edge-table-zero-horizontal-borders-compat14",
                "word-windows-edge-table-zero-padding-compat12",
                "word-windows-edge-table-zero-padding-compat14",
                "word-windows-edge-table-spacing-auto-compat12",
                "word-windows-edge-table-spacing-auto-compat14",
                "word-windows-edge-table-spacing-auto-compat15",
                "word-windows-edge-table-spacing-auto-borders-compat12",
                "word-windows-edge-table-spacing-auto-borders-compat14",
                "word-windows-edge-table-spacing-auto-borders-compat15",
                "word-windows-edge-table-spacing-at-least-compat12",
                "word-windows-edge-table-spacing-at-least-compat14",
                "word-windows-edge-table-spacing-at-least-compat15",
                "word-windows-edge-table-spacing-at-least-borders-compat12",
                "word-windows-edge-table-spacing-at-least-borders-compat14",
                "word-windows-edge-table-spacing-at-least-borders-compat15",
                "word-windows-edge-table-spacing-at-least-continuation-compat12",
                "word-windows-edge-table-spacing-at-least-continuation-compat14",
                "word-windows-edge-table-spacing-at-least-continuation-compat15",
                "word-windows-edge-table-spacing-mixed-borders-compat12",
                "word-windows-edge-table-spacing-mixed-borders-compat14",
                "word-windows-edge-table-spacing-mixed-borders-compat15",
                "word-windows-edge-table-collapsed-hairline-borders-compat12",
                "word-windows-edge-table-collapsed-hairline-borders-compat14",
                "word-windows-edge-table-collapsed-hairline-borders-compat15",
                "word-windows-edge-table-spacing-hairline-borders-compat12",
                "word-windows-edge-table-spacing-hairline-borders-compat14",
                "word-windows-edge-table-spacing-hairline-borders-compat15",
                "word-windows-edge-table-collapsed-implicit-mixed-borders-compat12",
                "word-windows-edge-table-collapsed-implicit-mixed-borders-compat14",
                "word-windows-edge-table-collapsed-implicit-mixed-borders-compat15",
                "word-windows-edge-table-spacing-nil-mixed-borders-compat12",
                "word-windows-edge-table-spacing-nil-mixed-borders-compat14",
                "word-windows-edge-table-spacing-nil-mixed-borders-compat15",
                "word-windows-edge-table-spacing-auto-mixed-borders-compat12",
                "word-windows-edge-table-spacing-auto-mixed-borders-compat14",
                "word-windows-edge-table-spacing-auto-mixed-borders-compat15",
                "word-windows-edge-table-spacing-percent-mixed-borders-compat12",
                "word-windows-edge-table-spacing-percent-mixed-borders-compat14",
                "word-windows-edge-table-spacing-percent-mixed-borders-compat15",
                "word-windows-edge-table-collapsed-mixed-borders-compat12",
                "word-windows-edge-table-collapsed-mixed-borders-compat14",
                "word-windows-edge-table-collapsed-mixed-borders-compat15",
                "word-windows-edge-table-spacing-missing-width-mixed-borders-compat12",
                "word-windows-edge-table-spacing-missing-width-mixed-borders-compat14",
                "word-windows-edge-table-spacing-missing-width-mixed-borders-compat15",
                "word-windows-edge-table-spacing-missing-type-mixed-borders-compat12",
                "word-windows-edge-table-spacing-missing-type-mixed-borders-compat14",
                "word-windows-edge-table-spacing-missing-type-mixed-borders-compat15",
                "word-windows-edge-table-spacing-negative-mixed-borders-compat12",
                "word-windows-edge-table-spacing-negative-mixed-borders-compat14",
                "word-windows-edge-table-spacing-negative-mixed-borders-compat15",
                "word-windows-edge-table-negative-spacing-columns-2-compat14",
                "word-windows-edge-table-negative-spacing-columns-3-compat14",
                "word-windows-edge-table-negative-spacing-columns-4-compat14",
                "word-windows-edge-table-negative-spacing-columns-5-compat14",
                "word-windows-edge-table-negative-spacing-unequal-2-compat14",
                "word-windows-edge-table-negative-spacing-unequal-3-compat14",
                "word-windows-edge-table-negative-spacing-unequal-4-compat14",
                "word-windows-edge-table-negative-spacing-row-scopes-compat14",
                "word-windows-edge-table-negative-spacing-autofit-unequal-3-compat14",
                "word-windows-edge-table-negative-spacing-bidi-unequal-3-compat14",
                "word-windows-edge-table-negative-spacing-autofit-bidi-alignment-compat12",
                "word-windows-edge-table-negative-spacing-autofit-bidi-alignment-compat14",
                "word-windows-edge-table-negative-spacing-autofit-bidi-alignment-compat15",
                "word-windows-edge-table-negative-spacing-autofit-bidi-unequal-3-compat12",
                "word-windows-edge-table-negative-spacing-autofit-bidi-unequal-3-compat14",
                "word-windows-edge-table-negative-spacing-autofit-bidi-unequal-3-compat15",
                "word-windows-edge-table-negative-spacing-autofit-unequal-3-compat12",
                "word-windows-edge-table-negative-spacing-autofit-unequal-3-compat15",
                "word-windows-edge-table-negative-spacing-bidi-unequal-3-compat12",
                "word-windows-edge-table-negative-spacing-bidi-unequal-3-compat15",
                "word-windows-edge-table-positive-spacing-bidi-border-widths-compat14",
                "word-windows-edge-table-positive-spacing-bidi-border-widths-compat15",
                "word-windows-edge-table-positive-spacing-sizing-autofit-bidi-compat14",
                "word-windows-edge-table-positive-spacing-sizing-autofit-bidi-compat15",
                "word-windows-edge-table-positive-spacing-sizing-autofit-bidi-short-compat14",
                "word-windows-edge-table-positive-spacing-sizing-autofit-ltr-compat14",
                "word-windows-edge-table-positive-spacing-sizing-autofit-ltr-short-compat14",
                "word-windows-edge-table-positive-spacing-sizing-fixed-bidi-compat14",
                "word-windows-edge-table-positive-spacing-sizing-fixed-bidi-short-compat14",
                "word-windows-edge-table-zero-spacing-bidi-border-widths-compat14",
                "word-windows-edge-table-zero-spacing-bidi-border-widths-compat15",
                "word-windows-edge-table-zero-spacing-ltr-border-widths-compat14",
                "word-windows-edge-table-zero-spacing-ltr-border-widths-compat15",
            ]
            .map(str::to_owned)
        )
    );
    assert!(
        checked >= 68025,
        "do not silently remove edge-case coverage"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn windows_word_font_boxes_and_grid_origins() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    for (evidence, reference) in reference_sets() {
        let library = system_fonts::FontLibrary::system();
        for family in ["Arial", "Times New Roman", "MS Mincho", "Yu Mincho"] {
            assert!(
                library.has(family),
                "install Windows font {family} to run this oracle"
            );
        }
        let tolerance = 0.375;
        assert_eq!(reference["tolerance_pt"], tolerance);
        assert_eq!(
            reference["fixtures"].as_array().unwrap().len() as u64,
            reference["_fixture_count"].as_u64().unwrap()
        );
        let mut checked = 0;
        for fixture in reference["fixtures"].as_array().unwrap() {
            let name = fixture["name"].as_str().unwrap();
            let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
            assert_eq!(
                fixture["docx_sha256"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            let pdf = std::fs::read(evidence.join(format!("{name}.pdf"))).unwrap();
            assert_eq!(fixture["pdf_sha256"], format!("{:x}", Sha256::digest(&pdf)));
            let faces = system_fonts::word_document_faces(&bytes, &[]);
            let mut doc =
                NativeDocument::open_with_font_faces(&bytes, 816.0, 1056.0, &faces).unwrap();
            assert_eq!(fixture["page_count"], doc.page_count());
            let mut lines = doc.renderer().layout_debug_lines();
            let snapshot = doc.capture_full_layout_debug().unwrap();
            assert!(snapshot.complete);
            lines.extend(snapshot.auxiliary_lines.into_iter().map(|entry| entry.line));
            let mut glyphs = Vec::new();
            for line in lines {
                for fragment in line.fragments {
                    for glyph in fragment.glyphs {
                        if glyph.text.is_empty()
                            || glyph
                                .text
                                .chars()
                                .all(|c| c.is_whitespace() || c.is_control())
                        {
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
            let mut expected = fixture["glyphs"].as_array().unwrap().clone();
            spatial_order(&mut glyphs);
            spatial_order(&mut expected);
            assert!(!expected.is_empty());
            checked += expected.len();
            assert_eq!(glyphs.len(), expected.len(), "{name}: missing/extra glyphs");
            for (i, (actual, expected)) in glyphs.iter().zip(&expected).enumerate() {
                assert_eq!(actual[0], expected[0], "{name}/{i}: page");
                assert_eq!(actual[1], expected[1], "{name}/{i}: text/order");
                for axis in [2, 3] {
                    let delta =
                        (actual[axis].as_f64().unwrap() - expected[axis].as_f64().unwrap()).abs();
                    assert!(
                    delta <= tolerance,
                    "{name}/{i}: axis {axis}, delta {delta}pt, actual {actual}, Word {expected}"
                );
                }
            }
        }
        assert_eq!(
            checked as u64,
            reference["_glyph_count"].as_u64().unwrap(),
            "review coverage when changing Windows probes"
        );
    }
}

/// Also runs on macOS CI without redistributing Microsoft's Windows fonts.
/// Requested-face vertical metrics must remain independent of the stand-in.
#[test]
fn windows_word_baselines_with_bundled_standins() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    for (_evidence, reference) in reference_sets() {
        let regular = repo_path("fonts/Noto_Sans_JP/static/NotoSansJP-Regular.ttf");
        let faces: Vec<_> = [
            "Noto Sans JP",
            "Arial",
            "Times New Roman",
            "MS Mincho",
            "MS Gothic",
            "Yu Mincho",
            "Yu Gothic",
            "Yu Gothic Light",
            "Yu Gothic Medium",
        ]
        .into_iter()
        .map(|family| FaceSpec {
            family,
            weight: 400,
            path: &regular,
        })
        .collect();
        let mut checked = 0;
        for fixture in reference["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|fixture| {
                fixture["name"] == "word-windows-yu-gothic-lines"
                    || fixture["name"]
                        .as_str()
                        .unwrap()
                        .starts_with("word-windows-tnr-")
                    || (reference["coordinate_source"] == "Word Page.EnhMetaFileBits / MS-EMF"
                        && matches!(
                            fixture["name"].as_str(),
                            Some("yokohama_page1" | "yokohama_page6")
                        ))
            })
        {
            let name = fixture["name"].as_str().unwrap();
            let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
            assert_eq!(
                fixture["docx_sha256"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            let mut doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
            assert_eq!(fixture["page_count"], doc.page_count());
            let (page_count, glyph_count) = match name {
                "word-windows-yu-gothic-lines" => (24, 2160),
                "yokohama_page1" => (1, 853),
                "yokohama_page6" => (5, 5425),
                "word-windows-tnr-balanced-sweep" | "word-windows-tnr-unbalanced-sweep" => {
                    (20, 480)
                }
                _ => unreachable!(),
            };
            assert_eq!(doc.page_count(), page_count);
            let mut expected = fixture["glyphs"].as_array().unwrap().clone();
            let mut actual = glyphs_for_reference(&mut doc, &reference, fixture);
            assert_eq!(expected.len(), glyph_count);
            assert_eq!(actual.len(), expected.len());
            spatial_order(&mut expected);
            spatial_order(&mut actual);
            for (actual, expected) in actual.iter().zip(&expected) {
                assert_eq!(actual[0], expected[0]);
                assert_eq!(actual[1], expected[1]);
                for axis in [2, 3] {
                    assert!(
                        (actual[axis].as_f64().unwrap() - expected[axis].as_f64().unwrap()).abs()
                            <= 0.375,
                        "{name} stand-ins: {actual} vs Word {expected}"
                    );
                }
            }
        }
        for fixture in reference["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|fixture| fixture["name"] == "numbering-restart")
        {
            let bytes = std::fs::read(repo_path("public/fixtures/numbering-restart.docx")).unwrap();
            assert_eq!(
                fixture["docx_sha256"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            let doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
            assert_eq!(fixture["page_count"], doc.page_count());
            let expected = fixture["glyphs"].as_array().unwrap();
            let numeric_rows: Vec<_> = expected
                .iter()
                .filter(|g| g[1] == "括")
                .map(|g| {
                    (
                        g[0].as_u64().unwrap(),
                        g[3].as_f64().unwrap(),
                        g[2].as_f64().unwrap(),
                    )
                })
                .collect();
            assert_eq!(numeric_rows.len(), 5);
            let selected = |g: &&Value| {
                matches!(g[1].as_str(), Some("B" | "C" | "D" | "E"))
                    || (matches!(g[1].as_str(), Some("1" | "2" | "3" | "4" | "5"))
                        && numeric_rows.iter().any(|(page, y, x)| {
                            g[0] == *page
                                && g[2].as_f64().unwrap() > *x
                                && (g[3].as_f64().unwrap() - y).abs() <= 0.375
                        }))
            };
            let mut expected: Vec<_> = expected.iter().filter(selected).cloned().collect();
            let mut actual = Vec::new();
            for line in doc.renderer().layout_debug_lines() {
                for fragment in line.fragments {
                    for glyph in fragment.glyphs {
                        actual.push(json!([
                            line.page + 1,
                            glyph.text,
                            (fragment.x + glyph.x - 2.0 * line.report_x_offset) * 0.75,
                            fragment.baseline * 0.75
                        ]));
                    }
                }
            }
            actual = actual.iter().filter(selected).cloned().collect();
            assert_eq!(
                expected.len(),
                21,
                "fixed cells and all five numeric boundaries"
            );
            assert_eq!(actual.len(), expected.len());
            spatial_order(&mut expected);
            spatial_order(&mut actual);
            for (actual, expected) in actual.iter().zip(&expected) {
                assert_eq!(actual[0], expected[0]);
                assert_eq!(actual[1], expected[1]);
                for axis in [2, 3] {
                    assert!(
                        (actual[axis].as_f64().unwrap() - expected[axis].as_f64().unwrap()).abs()
                            <= 0.375,
                        "numbering stand-ins: {actual} vs Word {expected}"
                    );
                }
            }
        }
        for fixture in reference["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| {
                f["name"]
                    .as_str()
                    .unwrap()
                    .starts_with("word-windows-controlled-")
            })
        {
            let name = fixture["name"].as_str().unwrap();
            let bytes = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
            assert_eq!(
                fixture["docx_sha256"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            let doc = NativeDocument::open(&bytes, 816.0, 1056.0, &faces).unwrap();
            assert_eq!(doc.page_count(), 12);
            let mut expected: Vec<_> = fixture["glyphs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|g| (g[0].as_u64().unwrap(), g[3].as_f64().unwrap()))
                .collect();
            expected.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-6);
            let actual: Vec<_> = doc
                .renderer()
                .layout_debug_lines()
                .into_iter()
                .map(|line| ((line.page + 1) as u64, line.fragments[0].baseline * 0.75))
                .collect();
            assert_eq!(actual.len(), 36, "{name}");
            assert_eq!(expected.len(), actual.len());
            for (i, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
                assert_eq!(actual.0, expected.0, "{name}/{i}: page");
                assert!(
                    (actual.1 - expected.1).abs() <= 0.375,
                    "{name}/{i}: {actual:?} vs Word {expected:?}"
                );
                checked += 1;
            }
        }
        assert_eq!(
            checked as u64,
            reference["_baseline_line_count"].as_u64().unwrap()
        );
    }
}
