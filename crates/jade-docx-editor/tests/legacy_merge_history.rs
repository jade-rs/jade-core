//! Legacy horizontal merges must keep their logical content through editing and saving.
mod common;

use common::{load, Session};
use jade_docx_editor::commands::merge_table_cells::{MergeTableCellsCommand, TableCellSelection};
use jade_docx_editor::commands::{CommandContext, CommandExecutor};
use jade_docx_editor::headless_api::apply_snapshot_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;

fn paragraph(text: &str) -> String {
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

fn body(start: &str, continuation: &str, separators: bool) -> String {
    let separator = if separators {
        "\n <!-- between cells -->\n "
    } else {
        ""
    };
    format!(
        "<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w=\"1000\"/><w:gridCol w:w=\"1000\"/><w:gridCol w:w=\"1000\"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:hMerge w:val=\"restart\"/></w:tcPr>{}</w:tc>{separator}<w:tc><w:tcPr><w:hMerge/></w:tcPr>{}</w:tc>{separator}<w:tc><w:tcPr/>{}</w:tc></w:tr></w:tbl>",
        paragraph(start), paragraph(continuation), paragraph("TAIL")
    )
}

fn merge() -> Box<MergeTableCellsCommand> {
    Box::new(MergeTableCellsCommand::new(
        0,
        vec![
            TableCellSelection {
                row: 0,
                column: 0,
                column_span: 2,
            },
            TableCellSelection {
                row: 0,
                column: 2,
                column_span: 1,
            },
        ],
    ))
}

fn texts(document: &WordDocument) -> Vec<String> {
    (0..document.paragraph_count())
        .map(|index| {
            document
                .paragraph(index)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect()
        })
        .collect()
}

fn spans(document: &WordDocument) -> Vec<u32> {
    document.table_by_order(0).unwrap().rows[0]
        .cells
        .iter()
        .map(|cell| cell.span.unwrap_or(1))
        .collect()
}

#[test]
fn legacy_merge_keeps_nonempty_continuation_on_save() {
    let mut session = Session::open(&body("START", "KEEP 日本語 & 😀", false), &[]);
    let before = texts(&session.document);
    session.run(merge());
    assert_eq!(texts(&session.document), before);
    assert_eq!(texts(&session.reopened()), before);
    assert_eq!(spans(&session.reopened()), vec![3]);
}

#[test]
fn legacy_merge_skips_xml_separators_when_removing_continuations() {
    let mut session = Session::open(&body("START", "", true), &[]);
    let before = texts(&session.document);
    session.run(merge());
    assert_eq!(spans(&session.reopened()), vec![3]);
    assert_eq!(texts(&session.reopened()), before);
}

#[test]
fn editing_the_cell_after_a_legacy_merge_updates_that_cell() {
    let mut session = Session::open(&body("START", "KEEP", false), &[]);
    let tail = texts(&session.document)
        .iter()
        .position(|text| text == "TAIL")
        .unwrap();
    session.selection = apply_snapshot_headless_commands_with_executor_to_document(
        &mut session.document,
        &mut session.executor,
        session.selection,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: tail,
                offset: 4,
                text: "!".into(),
            }],
        },
    )
    .unwrap();
    assert_eq!(texts(&session.reopened()), vec!["START", "KEEP", "TAIL!"]);
}

#[derive(Clone, Debug)]
enum Operation {
    Merge,
    Undo,
    Redo,
    Reload,
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"))), ..ProptestConfig::default() })]
    #[test]
    fn legacy_merge_history_preserves_content_and_geometry(
        start in prop::sample::select(vec!["", "START", "開始 😀"]),
        continuation in prop::sample::select(vec!["", "KEEP", "継続 & <文字>"]),
        separators in any::<bool>(),
        orphan in any::<bool>(),
        operations in prop::collection::vec(prop_oneof![
            4 => Just(Operation::Merge), 2 => Just(Operation::Undo),
            2 => Just(Operation::Redo), 1 => Just(Operation::Reload),
        ], 1..17),
    ) {
        let mut xml = body(start, continuation, separators);
        if orphan {
            xml = xml.replace("</w:tblGrid>", "<w:gridCol w:w=\"1000\"/></w:tblGrid>")
                .replace("</w:tr>", &format!("<w:tc><w:tcPr><w:hMerge/></w:tcPr>{}</w:tc></w:tr>", paragraph("ORPHAN")));
        }
        let mut session = Session::open(&xml, &[]);
        // Independent text contract: an empty mandatory restart is replaced by
        // meaningful continuation content; an empty continuation adds no paragraph.
        let mut expected = if !continuation.is_empty() {
            if start.is_empty() { vec![continuation.to_owned()] }
            else { vec![start.to_owned(), continuation.to_owned()] }
        } else { vec![start.to_owned()] };
        expected.push("TAIL".into());
        if orphan { expected.push("ORPHAN".into()); }
        let mut merged = false;
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        prop_assert_eq!(&texts(&session.document), &expected);
        for (step, operation) in operations.iter().enumerate() {
            match operation {
                Operation::Merge => {
                    let result = session.try_run(merge());
                    prop_assert_eq!(result.is_ok(), !merged, "step {}: {:?}", step, operation);
                    if !merged { undo.push(false); redo.clear(); merged = true; }
                }
                Operation::Undo | Operation::Redo => {
                    let (from, to) = if matches!(operation, Operation::Undo) { (&mut undo, &mut redo) } else { (&mut redo, &mut undo) };
                    let previous = from.pop();
                    let mut context = CommandContext::new(&mut session.document, &mut session.selection);
                    let changed = if matches!(operation, Operation::Undo) { session.executor.undo(&mut context) } else { session.executor.redo(&mut context) }.unwrap();
                    prop_assert_eq!(changed, previous.is_some(), "step {}: {:?}", step, operation);
                    if let Some(previous) = previous { to.push(merged); merged = previous; }
                }
                Operation::Reload => {
                    session.document = load(&session.document.export_docx().unwrap());
                    session.executor = CommandExecutor::new();
                    session.selection = None;
                    undo.clear(); redo.clear();
                }
            }
            prop_assert_eq!(&texts(&session.document), &expected, "live step {}: {:?}", step, operation);
            let saved = load(&session.document.export_docx().unwrap());
            prop_assert_eq!(&texts(&saved), &expected, "saved step {}: {:?}", step, operation);
            let mut expected_spans = if merged { vec![3] } else { vec![2, 1] };
            if orphan { expected_spans.push(1); }
            prop_assert_eq!(&spans(&session.document), &expected_spans, "live step {}: {:?}", step, operation);
            prop_assert_eq!(spans(&saved), expected_spans, "saved step {}: {:?}", step, operation);
        }
    }
}

#[test]
fn merging_normal_cells_does_not_remove_an_orphan_continuation() {
    let cells = format!(
        "<w:tc><w:tcPr/>{}</w:tc><w:tc><w:tcPr/>{}</w:tc><w:tc><w:tcPr><w:hMerge/></w:tcPr>{}</w:tc>",
        paragraph("A"), paragraph("B"), paragraph("ORPHAN"),
    );
    let xml = format!("<w:tbl><w:tblGrid><w:gridCol w:w=\"1000\"/><w:gridCol w:w=\"1000\"/><w:gridCol w:w=\"1000\"/></w:tblGrid><w:tr>{cells}</w:tr></w:tbl>");
    let mut session = Session::open(&xml, &[]);
    session.run(Box::new(MergeTableCellsCommand::new(
        0,
        vec![
            TableCellSelection {
                row: 0,
                column: 0,
                column_span: 1,
            },
            TableCellSelection {
                row: 0,
                column: 1,
                column_span: 1,
            },
        ],
    )));
    assert_eq!(texts(&session.document), vec!["A", "B", "ORPHAN"]);
    assert_eq!(texts(&session.reopened()), vec!["A", "B", "ORPHAN"]);
    assert_eq!(spans(&session.reopened()), vec![2, 1]);
}

#[test]
fn legacy_merge_retains_explicit_empty_paragraphs_and_field_markup() {
    for continuation in [
        "<w:p/><w:p/>",
        "<w:p><w:fldSimple w:instr=\"DATE\"><w:r><w:t/></w:r></w:fldSimple></w:p>",
        "<w:tbl><w:tblGrid><w:gridCol w:w=\"1000\"/></w:tblGrid><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>NESTED</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>",
    ] {
        let xml = body("", "PLACEHOLDER", true).replace(&paragraph("PLACEHOLDER"), continuation);
        let mut session = Session::open(&xml, &[]);
        let before = texts(&session.document);
        session.run(merge());
        assert_eq!(texts(&session.document), before);
        assert_eq!(texts(&session.reopened()), before, "{continuation}");
        assert_eq!(spans(&session.reopened()), vec![3]);
        if continuation.contains("DATE") {
            assert!(common::document_xml(&session.document).contains("DATE"));
            assert!(common::document_xml(&session.reopened()).contains("DATE"));
        }
    }
}

#[test]
fn editing_a_retained_continuation_preserves_the_following_cell() {
    let mut session = Session::open(&body("START", "KEEP", true), &[]);
    session.selection = apply_snapshot_headless_commands_with_executor_to_document(
        &mut session.document,
        &mut session.executor,
        session.selection,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::InsertText {
                paragraph: 1,
                offset: 4,
                text: "!".into(),
            }],
        },
    )
    .unwrap();
    assert_eq!(texts(&session.reopened()), vec!["START", "KEEP!", "TAIL"]);
    session.run(merge());
    assert_eq!(texts(&session.reopened()), vec!["START", "KEEP!", "TAIL"]);
}

#[test]
fn wrapped_legacy_cells_fail_without_changing_content_or_history() {
    let original = body("START", "KEEP", false);
    for content in [paragraph("START"), paragraph("KEEP"), paragraph("TAIL")] {
        let start = original[..original.find(&content).unwrap()]
            .rfind("<w:tc>")
            .unwrap();
        let end = original[start..].find("</w:tc>").unwrap() + start + "</w:tc>".len();
        let mut xml = original.clone();
        xml.insert_str(end, "</w:sdtContent></w:sdt>");
        xml.insert_str(start, "<w:sdt><w:sdtContent>");
        let mut session = Session::open(&xml, &[]);
        let before = texts(&session.document);
        let before_xml = session.xml();
        let selection = session.selection;
        assert!(session.try_run(merge()).is_err(), "wrapped {content}");
        assert_eq!(session.selection, selection);
        assert_eq!(texts(&session.document), before);
        assert_eq!(session.xml(), before_xml);
        assert_eq!(texts(&session.reopened()), before);
        assert_eq!(spans(&session.reopened()), vec![2, 1]);
        let mut context = CommandContext::new(&mut session.document, &mut session.selection);
        assert!(!session.executor.undo(&mut context).unwrap());
    }
}

#[test]
fn removing_a_normal_cell_does_not_attach_its_orphan_to_a_selected_legacy_group() {
    let xml = body("START", "KEEP", true)
        .replace("</w:tblGrid>", "<w:gridCol w:w=\"1000\"/></w:tblGrid>")
        .replace(
            "</w:tr>",
            &format!(
                "<w:tc><w:tcPr><w:hMerge/></w:tcPr>{}</w:tc></w:tr>",
                paragraph("ORPHAN")
            ),
        );
    let mut session = Session::open(&xml, &[]);
    let expected = vec!["START", "KEEP", "TAIL", "ORPHAN"];
    assert_eq!(texts(&session.document), expected);
    session.run(merge());
    assert_eq!(texts(&session.document), expected);
    assert_eq!(texts(&session.reopened()), expected);
    assert_eq!(spans(&session.reopened()), vec![3, 1]);
}

#[test]
fn mixed_case_legacy_merge_markers_keep_content_and_mapping() {
    let xml = body("START", "KEEP", true)
        .replace("w:val=\"restart\"", "w:val=\"ReStArT\"")
        .replace("<w:hMerge/>", "<w:hMerge w:val=\"CoNtInUe\"/>");
    let mut session = Session::open(&xml, &[]);
    session.run(merge());
    assert_eq!(texts(&session.reopened()), vec!["START", "KEEP", "TAIL"]);
    assert_eq!(spans(&session.reopened()), vec![3]);
}

#[test]
fn unknown_legacy_merge_value_does_not_continue_a_group() {
    let xml =
        body("START", "KEEP", true).replace("<w:hMerge/>", "<w:hMerge w:val=\"future-value\"/>");
    let mut session = Session::open(&xml, &[]);
    assert_eq!(spans(&session.document), vec![1, 1, 1]);
    session.run(Box::new(MergeTableCellsCommand::new(
        0,
        vec![
            TableCellSelection {
                row: 0,
                column: 0,
                column_span: 1,
            },
            TableCellSelection {
                row: 0,
                column: 1,
                column_span: 1,
            },
        ],
    )));
    assert_eq!(texts(&session.reopened()), vec!["START", "KEEP", "TAIL"]);
    assert_eq!(spans(&session.reopened()), vec![2, 1]);
}
