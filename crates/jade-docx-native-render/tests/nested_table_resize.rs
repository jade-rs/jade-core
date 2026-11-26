//! Compare a saved column resize with Windows Word, then undo and redo it.
mod common;

use common::{open_bytes, repo_path};
use jade_docx_editor::document::TableLayoutMode;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::HeadlessCommand;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_docx_native_render::NativeDocument;
use sha2::{Digest, Sha256};
use std::io::Cursor;
#[cfg(windows)]
use std::io::Read;
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
    widths_at(document, 1)
}

fn widths_at(document: &WordDocument, order: usize) -> Widths {
    let table = document
        .table_by_order(order)
        .expect("table in document order");
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
    verify_resizes("nested-table-resize", 18, open_bytes);
    verify_resizes("table-cell-border-content", 39, open_bytes);
    verify_resizes("nested-table-percent-resize", 12, open_bytes);
    verify_resizes("spaced-table-border-content", 24, open_bytes);
}

#[cfg(windows)]
#[test]
fn autofit_nested_resize_matches_word_font_metrics_and_round_trips() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    verify_resizes("nested-autofit-advances", 36, open_with_windows_fonts);
    verify_resizes("nested-autofit-resize", 72, open_with_windows_fonts);
}

#[cfg(windows)]
fn open_with_windows_fonts(bytes: &[u8]) -> NativeDocument {
    let faces = jade_docx_native_render::system_fonts::word_document_faces(bytes, &[]);
    NativeDocument::open_with_font_faces(
        bytes,
        common::PAGE_WIDTH_PX,
        common::PAGE_HEIGHT_PX,
        &faces,
    )
    .expect("open the document with Windows fonts")
}

#[cfg(windows)]
#[test]
fn autofit_nested_grids_match_windows_word_at_twip_boundaries() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let (mut archive, manifest) = load_word_capture("nested-autofit-twip-rounding");
    let cases = manifest["documents"].as_array().unwrap();
    assert_eq!(cases.len(), 42);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let (source, saved) = read_word_save(&mut archive, case);
        let word = open_with_windows_fonts(&saved);
        let jade = open_with_windows_fonts(&source);
        let reopened = open_with_windows_fonts(&jade.export_docx().unwrap());
        let layout = jade.renderer().table_resize_metadata();
        assert_eq!(layout.tables.len(), 2, "{name}");
        for info in layout.tables {
            let expected = word.word_document().table_by_order(info.order).unwrap();
            let expected_grid = expected
                .columns
                .iter()
                .map(|column| (f64::from(column.width.unwrap()) * 20.0).round() as i32)
                .collect::<Vec<_>>();
            let rendered_grid = info
                .column_widths
                .iter()
                .map(|width| (width * 15.0).round() as i32)
                .collect::<Vec<_>>();
            assert_eq!(rendered_grid, expected_grid, "{name}: table {}", info.order);
            assert_eq!(
                widths_at(reopened.word_document(), info.order),
                widths_at(word.word_document(), info.order),
                "{name}: saved table {}",
                info.order
            );
        }
    }
}

#[cfg(windows)]
fn read_word_save(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    case: &serde_json::Value,
) -> (Vec<u8>, Vec<u8>) {
    let name = case["name"].as_str().unwrap();
    let source = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
    assert_eq!(
        case["input_sha256"],
        format!("{:x}", Sha256::digest(&source))
    );
    let mut saved = Vec::new();
    archive
        .by_name(&format!("word/saved/{name}.docx"))
        .unwrap()
        .read_to_end(&mut saved)
        .unwrap();
    assert_eq!(
        case["word_save_sha256"],
        format!("{:x}", Sha256::digest(&saved))
    );
    (source, saved)
}

#[cfg(windows)]
#[test]
fn docx_export_matches_word_autofit_grids_on_both_sides_of_page_break() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let (mut archive, manifest) = load_word_capture("nested-autofit-parent-save");
    let cases = manifest["documents"].as_array().unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let (source, saved) = read_word_save(&mut archive, case);
        let word = open_with_windows_fonts(&saved);
        let faces = jade_docx_native_render::system_fonts::word_document_faces(&source, &[]);
        let mut jade = NativeDocument::open_with_font_faces_and_paint_window(
            &source,
            common::PAGE_WIDTH_PX,
            common::PAGE_HEIGHT_PX,
            &faces,
            0..1,
        )
        .unwrap();

        let check_saved = |bytes: Vec<u8>| {
            let reopened = open_with_windows_fonts(&bytes);
            assert_eq!(reopened.page_count(), 2, "{name}");
            for order in 0..4 {
                assert_eq!(
                    widths_at(reopened.word_document(), order),
                    widths_at(word.word_document(), order),
                    "{name}: saved table {order}"
                );
            }
            if name.contains("grid-change") {
                let mut package = ZipArchive::new(Cursor::new(bytes)).unwrap();
                let mut xml = String::new();
                package
                    .by_name("word/document.xml")
                    .unwrap()
                    .read_to_string(&mut xml)
                    .unwrap();
                let previous = xml
                    .split_once("<w:tblGridChange")
                    .unwrap()
                    .1
                    .split_once("</w:tblGridChange>")
                    .unwrap()
                    .0;
                assert!(previous.contains("<w:gridCol w:w=\"3550\""));
                assert!(previous.contains("<w:gridCol w:w=\"4730\""));
            }
        };
        let original_parent = widths_at(jade.word_document(), 2);
        check_saved(jade.export_docx().unwrap());
        assert_eq!(
            widths_at(jade.word_document(), 2),
            original_parent,
            "{name}: live grid"
        );
        assert!(!jade.can_undo());

        let marker = (0..jade.word_document().paragraph_count())
            .find(|&i| {
                jade.word_document()
                    .rendered_paragraph_text(i)
                    .contains("FIRSTPAGE")
            })
            .expect("edit paragraph on the first page");
        jade.insert_text(marker, 0, "Edit ", 1).unwrap();
        let selection = jade.selection_range();
        check_saved(jade.export_docx().unwrap());
        assert_eq!(jade.selection_range(), selection, "{name}: selection");
        assert!(jade.can_undo());
        jade.undo().unwrap();
        check_saved(jade.export_docx().unwrap());
    }
}

#[cfg(windows)]
#[test]
fn batched_autofit_resize_remeasures_each_edit_and_matches_word() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let (_, manifest) = load_word_capture("nested-autofit-batch-resize");
    let cases = manifest["documents"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let before = case["before_fixture"].as_str().unwrap();
        let source = std::fs::read(repo_path(&format!("public/fixtures/{before}.docx"))).unwrap();
        let saved = std::fs::read(repo_path(&format!("public/fixtures/{name}.docx"))).unwrap();
        assert_eq!(
            case["before_input_sha256"],
            format!("{:x}", Sha256::digest(&source))
        );
        assert_eq!(
            case["input_sha256"],
            format!("{:x}", Sha256::digest(&saved))
        );
        let word = open_with_windows_fonts(&saved);
        let mut jade = open_with_windows_fonts(&source);
        let actions = case["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        let commands: Vec<_> = actions
            .iter()
            .map(|action| {
                if let Some(width) = action["resize_pt"].as_f64() {
                    HeadlessCommand::ResizeTableColumnRightEdge {
                        table_order_index: 1,
                        column_index: 1,
                        new_width: width as f32,
                    }
                } else {
                    let length = action["parent_word_length"].as_u64().unwrap() as usize;
                    assert_eq!(action["delete_text"], "W");
                    let original = "W".repeat(length);
                    let paragraph = (0..jade.word_document().paragraph_count())
                        .find(|&index| {
                            jade.word_document().rendered_paragraph_text(index) == original
                        })
                        .unwrap();
                    HeadlessCommand::ReplaceRange {
                        start_paragraph: paragraph,
                        start_offset: 0,
                        end_paragraph: paragraph,
                        end_offset: 1,
                        text: String::new(),
                    }
                }
            })
            .collect();
        let original = jade.export_docx().unwrap();
        let check_saved = |document: &NativeDocument, expected: &NativeDocument| {
            let reopened = open_with_windows_fonts(&document.export_docx().unwrap());
            for order in 0..2 {
                assert_eq!(
                    widths_at(reopened.word_document(), order),
                    widths_at(expected.word_document(), order),
                    "{name}: saved table {order}"
                );
            }
            assert_eq!(reopened.page_count(), expected.page_count(), "{name}");
            if let HeadlessCommand::ReplaceRange {
                start_paragraph, ..
            } = commands[0]
            {
                assert_eq!(
                    reopened
                        .word_document()
                        .rendered_paragraph_text(start_paragraph),
                    expected
                        .word_document()
                        .rendered_paragraph_text(start_paragraph),
                    "{name}: parent text"
                );
            }
        };
        jade.apply_commands(commands.clone(), false).unwrap();
        check_saved(&jade, &word);
        jade.undo().unwrap();
        assert!(
            !jade.can_undo(),
            "{name}: undo the entire batch in one step"
        );
        let original = open_with_windows_fonts(&original);
        check_saved(&jade, &original);
        jade.redo().unwrap();
        check_saved(&jade, &word);

        if name.ends_with("shorten-n24-rightedge-compat15") {
            jade.undo().unwrap();
            let mut invalid = commands.clone();
            invalid.push(HeadlessCommand::ResizeTableColumnRightEdge {
                table_order_index: 1,
                column_index: 99,
                new_width: 65.0,
            });
            let error = jade.apply_commands(invalid, false).unwrap_err();
            assert!(error.contains("index: 2"), "{error}");
            assert!(
                !jade.can_undo(),
                "a failed batch must not enter the history"
            );
            check_saved(&jade, &original);
        }
    }
}

#[cfg(windows)]
#[test]
fn autofit_nested_grid_saves_within_the_windows_word_text_frame() {
    set_word_layout_platform(WordLayoutPlatform::Windows);
    let twips = |width: Option<f32>| (f64::from(width.unwrap()) * 20.0).round() as i32;
    for (cohort, count) in [
        ("nested-autofit-text-content", 54),
        ("nested-autofit-text-frame", 18),
        ("nested-autofit-text-margins", 18),
        ("nested-autofit-text-indents", 6),
        ("nested-autofit-text-resize", 18),
        ("nested-autofit-text-overflow", 18),
    ] {
        let (mut archive, manifest) = load_word_capture(cohort);
        let cases = manifest["documents"].as_array().unwrap();
        assert_eq!(cases.len(), count);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let (input, expected) = read_word_save(&mut archive, case);
            let word = open_with_windows_fonts(&expected);
            let jade = open_with_windows_fonts(&input);
            let saved = open_with_windows_fonts(&jade.export_docx().unwrap());
            for order in 0..2 {
                let authored = widths_at(jade.word_document(), order);
                let exported = widths_at(saved.word_document(), order);
                assert_eq!(exported.preferred_width, authored.preferred_width, "{name}");
                assert_eq!(exported.cells, authored.cells, "{name}");
            }
            let parent = widths_at(saved.word_document(), 0);
            let expected_parent = widths_at(word.word_document(), 0);
            let child = widths_at(saved.word_document(), 1);
            let expected_child = widths_at(word.word_document(), 1);
            assert_eq!(
                parent.grid.iter().copied().map(twips).sum::<i32>(),
                expected_parent.grid.iter().copied().map(twips).sum::<i32>(),
                "{name}: table extent"
            );
            let child_width = child.grid.iter().copied().map(twips).sum::<i32>();
            let word_child_width = expected_child.grid.iter().copied().map(twips).sum::<i32>();
            if word_child_width > twips(expected_parent.grid[1]) {
                assert_eq!(
                    child_width, word_child_width,
                    "{name}: absolute nested width"
                );
            } else {
                assert!(
                    child_width <= twips(parent.grid[1]),
                    "{name}: nested columns fit inside the containing cell"
                );
            }
        }
    }
}

fn load_word_capture(capture: &str) -> (ZipArchive<Cursor<Vec<u8>>>, serde_json::Value) {
    let archive = std::fs::read(repo_path(&format!(
        "tests/word-oracle/windows/{capture}/word-16.0.19127.20936.zip",
    )))
    .unwrap();
    let mut archive = ZipArchive::new(Cursor::new(archive)).unwrap();
    let manifest = archive.by_name("word/capture.json").unwrap();
    let manifest: serde_json::Value = serde_json::from_reader(manifest).unwrap();
    assert_eq!(manifest["product_version"], "16.0.19127.20936");
    (archive, manifest)
}

fn verify_resizes(capture: &str, expected_cases: usize, open: fn(&[u8]) -> NativeDocument) {
    let (_, manifest) = load_word_capture(capture);
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
        let word = open(&word);
        let expected = widths(word.word_document());
        let compare_parent = capture.starts_with("nested-autofit-");
        let mut jade = open(&source);
        let original = widths(jade.word_document());
        let original_saved = compare_parent.then(|| {
            let saved = jade.export_docx().unwrap();
            widths(open(&saved).word_document())
        });
        let resize = HeadlessCommand::ResizeTableColumnRightEdge {
            table_order_index: 1,
            column_index: 1,
            new_width: case["new_width_pt"].as_f64().unwrap_or(65.0) as f32,
        };
        let result = if compare_parent {
            jade.apply_commands(vec![resize], false)
        } else {
            jade.apply_command(resize, false)
        };
        result.unwrap_or_else(|error| panic!("{after}: {error}"));
        assert_eq!(
            widths(jade.word_document()),
            expected,
            "{after}: edited model"
        );
        let saved = jade.export_docx().unwrap();
        let reopened = open(&saved);
        assert_eq!(
            widths(reopened.word_document()),
            expected,
            "{after}: saved model"
        );
        if compare_parent {
            assert_eq!(
                widths_at(reopened.word_document(), 0),
                widths_at(word.word_document(), 0),
                "{after}: saved parent table"
            );
        }
        assert_eq!(
            jade.page_count(),
            case["page_count"].as_u64().unwrap() as usize,
            "{after}"
        );
        jade.undo().unwrap();
        assert_eq!(widths(jade.word_document()), original, "{after}: live undo");
        let saved = jade.export_docx().unwrap();
        assert_eq!(
            widths(open(&saved).word_document()),
            original_saved.unwrap_or(original),
            "{after}: undo"
        );
        jade.redo().unwrap();
        let saved = jade.export_docx().unwrap();
        assert_eq!(
            widths(open(&saved).word_document()),
            expected,
            "{after}: redo"
        );
    }
}
