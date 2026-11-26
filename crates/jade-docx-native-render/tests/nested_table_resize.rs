//! Compare a saved column resize with Windows Word, then undo and redo it.
mod common;

use common::{open_bytes, repo_path};
use jade_docx_editor::document::TableLayoutMode;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use zip::ZipArchive;

#[derive(Debug, PartialEq)]
struct Widths {
    preferred_width: Option<(f32, bool)>,
    fixed_layout: bool,
    grid: Vec<Option<f32>>,
    cells: Vec<Vec<Option<(f32, bool)>>>,
}

fn width(value: Option<&String>) -> Option<(f32, bool)> {
    let value = value?.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return Some((percent.parse().unwrap(), true));
    }
    value
        .strip_suffix("pt")
        .map(|points| (points.parse().unwrap(), false))
}

fn widths(document: &WordDocument) -> Widths {
    let table = document.table_by_order(1).expect("nested table");
    Widths {
        preferred_width: width(table.style.get("width")),
        fixed_layout: table.table_layout == Some(TableLayoutMode::Fixed),
        grid: table.columns.iter().map(|column| column.width).collect(),
        cells: table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| width(cell.style.get("width")))
                    .collect()
            })
            .collect(),
    }
}

#[test]
fn nested_right_edge_resize_matches_word_and_round_trips() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    verify_resizes("nested-table-resize", 18);
    verify_resizes("table-cell-border-content", 39);
    verify_resizes("nested-table-percent-resize", 12);
    verify_resizes("spaced-table-border-content", 24);
}

fn verify_resizes(capture: &str, expected_cases: usize) {
    let archive = std::fs::read(repo_path(&format!(
        "tests/word-oracle/windows/{capture}/word-16.0.19127.20936.zip",
    )))
    .unwrap();
    let mut archive = ZipArchive::new(Cursor::new(archive)).unwrap();
    let manifest = archive.by_name("word/capture.json").unwrap();
    let manifest: serde_json::Value = serde_json::from_reader(manifest).unwrap();
    assert_eq!(manifest["product_version"], "16.0.19127.20936");
    let cases = manifest["documents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case.get("before_fixture").is_some())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), expected_cases);

    for case in cases {
        let before = case["before_fixture"].as_str().unwrap();
        let after = case["name"].as_str().unwrap();
        let source = std::fs::read(repo_path(&format!("public/fixtures/{before}.docx"))).unwrap();
        let word = std::fs::read(repo_path(&format!("public/fixtures/{after}.docx"))).unwrap();
        assert_eq!(
            case["before_input_sha256"],
            format!("{:x}", Sha256::digest(&source))
        );
        assert_eq!(case["input_sha256"], format!("{:x}", Sha256::digest(&word)));
        let expected = widths(open_bytes(&word).word_document());
        let mut jade = open_bytes(&source);
        let original = widths(jade.word_document());
        jade.apply_command(
            HeadlessCommand::ResizeTableColumnRightEdge {
                table_order_index: 1,
                column_index: 1,
                new_width: case["new_width_pt"].as_f64().unwrap_or(65.0) as f32,
            },
            false,
        )
        .unwrap_or_else(|error| panic!("{after}: {error}"));
        assert_eq!(
            widths(jade.word_document()),
            expected,
            "{after}: edited model"
        );
        let saved = jade.export_docx().unwrap();
        assert_eq!(
            widths(open_bytes(&saved).word_document()),
            expected,
            "{after}: saved model"
        );
        assert_eq!(
            jade.page_count(),
            case["page_count"].as_u64().unwrap() as usize,
            "{after}"
        );
        jade.undo().unwrap();
        let saved = jade.export_docx().unwrap();
        assert_eq!(
            widths(open_bytes(&saved).word_document()),
            original,
            "{after}: undo"
        );
        jade.redo().unwrap();
        let saved = jade.export_docx().unwrap();
        assert_eq!(
            widths(open_bytes(&saved).word_document()),
            expected,
            "{after}: redo"
        );
    }
}
