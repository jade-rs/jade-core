//! Relative ruby baselines measured from an actual Word PDF, without origin fitting.
mod common;
use common::repo_path;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_native_render::{FaceSpec, NativeDocument};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
#[ignore = "known Arial subscript gap; requires JADE_OFFICE_ARIAL pointing to Office arial.ttf"]
fn ruby_guide_positions_match_word_relative_baselines() {
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let font = std::env::var("JADE_OFFICE_ARIAL").expect("set JADE_OFFICE_ARIAL");
    assert_eq!(
        sha256(&std::fs::read(&font).unwrap()),
        "15663c7e8495594c6136eb51b11983eb8f5bbcdd07bb2c327e0d1e83415b0f06",
        "font changed: recapture Word evidence before comparing"
    );
    let root = repo_path("tests/word-oracle/mac-probes-20261108/ruby-position");
    let source = std::fs::read(root.join("ruby-position.docx")).unwrap();
    let oracle: Value =
        serde_json::from_slice(&std::fs::read(root.join("word-glyphs.json")).unwrap()).unwrap();
    assert_eq!(oracle["source_sha256"], sha256(&source));
    assert_eq!(
        oracle["pdf_sha256"],
        sha256(&std::fs::read(root.join("ruby-position-word.pdf")).unwrap())
    );
    let spans = oracle["pages"][0]["spans"].as_array().unwrap();
    let word_baselines = |text: &str| -> Vec<f64> {
        spans
            .iter()
            .filter(|span| span["text"] == text)
            .map(|span| span["glyphs"][0]["origin"][1].as_f64().unwrap())
            .collect()
    };
    let word_guides = word_baselines("ABC");
    let word_bases = word_baselines("BASE");
    assert_eq!(word_guides.len(), 7);
    assert_eq!(word_bases.len(), 7);
    let faces = [FaceSpec {
        family: "Arial",
        weight: 400,
        path: std::path::Path::new(&font),
    }];
    let mut doc = NativeDocument::open(&source, 816.0, 1056.0, &faces).unwrap();
    let snapshot = doc.capture_full_layout_debug().unwrap();
    assert!(snapshot.complete);
    let guides: Vec<_> = snapshot
        .auxiliary_lines
        .iter()
        .filter(|entry| entry.story == "ruby" && entry.line.text == "ABC")
        .map(|entry| entry.line.fragments[0].baseline * 0.75)
        .collect();
    let bases: Vec<_> = snapshot
        .body_lines
        .iter()
        .filter_map(|line| {
            let text: String = line
                .fragments
                .iter()
                .flat_map(|f| &f.glyphs)
                .map(|g| g.text.as_str())
                .collect();
            let start = text.find("BASE")?;
            let mut offset = 0;
            for fragment in &line.fragments {
                for glyph in &fragment.glyphs {
                    if offset == start {
                        return Some(fragment.baseline * 0.75);
                    }
                    offset += glyph.text.len();
                }
            }
            panic!("base does not start at a captured glyph");
        })
        .collect();
    assert_eq!(guides.len(), 7, "all guides must be captured");
    assert_eq!(bases.len(), 7, "all bases must be captured");
    let mut differences = Vec::new();
    for (index, label) in [
        "normal",
        "raise2",
        "lower3",
        "super",
        "sub",
        "super_raise2",
        "lower30",
    ]
    .iter()
    .enumerate()
    {
        let expected = word_bases[index] - word_guides[index];
        let actual = bases[index] - guides[index];
        let measurement = format!("{label}: Word base-guide={expected:.4}pt, native={actual:.4}pt");
        eprintln!("{measurement}");
        if (actual - expected).abs() > 0.375 {
            differences.push(measurement);
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
}
