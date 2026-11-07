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
    let mut glyphs = Vec::new();
    let mut lines = doc.renderer().layout_debug_lines();
    let snapshot = doc.capture_full_layout_debug().unwrap();
    assert!(snapshot.complete);
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
            let mut actual = all_glyphs(&mut doc);
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
            ]
            .map(str::to_owned)
        )
    );
    assert!(
        checked >= 13199,
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
            let mut actual = all_glyphs(&mut doc);
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
