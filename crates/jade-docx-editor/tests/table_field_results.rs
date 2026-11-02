//! Table sorting and formulas use displayed field results, including after reload.
mod common;

use common::{load, Session};
use jade_docx_editor::commands::CommandContext;
use jade_docx_editor::headless_api::apply_headless_commands_with_executor_to_document;
use jade_docx_editor::word_document::WordDocument;
use jade_docx_headless_api::{
    HeadlessCommand, HeadlessEditRequest, HeadlessTableSortKey, HeadlessTableSortKind,
};
use proptest::prelude::*;

fn hidden(instruction: &str, cache: i32, complex: bool) -> String {
    if complex {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>{instruction}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{cache}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    } else {
        format!(
            r#"<w:fldSimple w:instr="{instruction}"><w:r><w:t>{cache}</w:t></w:r></w:fldSimple>"#
        )
    }
}

fn body(values: &[(i32, i32)], instruction: &str, complex: bool) -> String {
    let rows: String = values
        .iter()
        .map(|(value, cache)| {
            format!(
                r#"<w:tr><w:tc><w:tcPr/><w:p>{}<w:r><w:t>{value}</w:t></w:r></w:p></w:tc></w:tr>"#,
                hidden(instruction, *cache, complex)
            )
        })
        .collect();
    format!(
        r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid>{rows}</w:tbl><w:p/>"#
    )
}

fn sort(session: &mut Session, descending: bool) {
    session.selection = apply_headless_commands_with_executor_to_document(
        &mut session.document,
        &mut session.executor,
        session.selection,
        &HeadlessEditRequest {
            commands: vec![HeadlessCommand::SortTable {
                table_order_index: 0,
                keys: vec![HeadlessTableSortKey {
                    column: 0,
                    kind: HeadlessTableSortKind::Number,
                    descending,
                }],
                header_rows: 0,
            }],
        },
    )
    .expect("sort");
}

fn values(document: &WordDocument) -> Vec<i32> {
    (0..document.table_by_order(0).unwrap().rows.len())
        .map(|index| {
            document
                .paragraph(index)
                .unwrap()
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .parse()
                .unwrap()
        })
        .collect()
}

fn assert_sum(document: &WordDocument, expected: &[i32]) {
    assert_eq!(
        document
            .evaluate_table_formula(expected.len() - 1, "SUM(ABOVE)", None)
            .unwrap(),
        expected[..expected.len() - 1]
            .iter()
            .sum::<i32>()
            .to_string(),
    );
}

fn capture(document: &WordDocument) {
    let Some(directory) = std::env::var_os("JADE_FIELD_PBT_EXPORT_DIR") else {
        return;
    };
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).unwrap();
    let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::fs::write(
        directory.join(format!("table-field-{next}.docx")),
        document.export_docx().unwrap(),
    )
    .unwrap();
}

#[test]
fn numeric_sort_ignores_hidden_assignment_caches_and_preserves_them_on_save() {
    for complex in [false, true] {
        for instruction in ["SET Counter 10", "ASK Counter Prompt"] {
            let mut session = Session::open(&body(&[(3, 900), (5, 1)], instruction, complex), &[]);
            let before = session.document.export_docx().unwrap();
            sort(&mut session, false);
            assert_eq!(values(&session.document), vec![3, 5]);
            let after = session.document.export_docx().unwrap();
            assert!(common::part(&after, "word/document.xml").contains(">900<"));
            assert_eq!(values(&load(&after)), vec![3, 5]);
            capture(&load(&after));
            session.undo();
            assert_eq!(session.document.export_docx().unwrap(), before);
            session.redo();
            assert_eq!(session.document.export_docx().unwrap(), after);
        }
    }
}

#[test]
fn formulas_ignore_hidden_assignment_caches_before_and_after_reload() {
    for complex in [false, true] {
        for instruction in ["SET Counter 10", "ASK Counter Prompt"] {
            let session = Session::open(
                &body(&[(3, 900), (5, 1), (2, 700)], instruction, complex),
                &[],
            );
            assert_sum(&session.document, &[3, 5, 2]);
            assert_sum(&session.reopened(), &[3, 5, 2]);
            capture(&session.document);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    #[test]
    fn table_operations_match_visible_numbers(
        input in prop::collection::vec((1i32..100, 100i32..1000), 2..9),
        operations in prop::collection::vec(0u8..5, 1..20),
        complex in any::<bool>(),
        ask in any::<bool>(),
    ) {
        let instruction = if ask { "ASK Counter Prompt" } else { "SET Counter 10" };
        let mut session = Session::open(&body(&input, instruction, complex), &[]);
        let mut model: Vec<i32> = input.iter().map(|(value, _)| *value).collect();
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        for operation in operations {
            match operation {
                0 | 1 => {
                    undo.push((model.clone(), session.document.export_docx().unwrap()));
                    redo.clear();
                    sort(&mut session, operation == 1);
                    model.sort();
                    if operation == 1 { model.reverse(); }
                }
                2 if !undo.is_empty() => {
                    let (expected, bytes) = undo.pop().unwrap();
                    redo.push((model.clone(), session.document.export_docx().unwrap()));
                    prop_assert!(session.executor.undo(&mut CommandContext::new(&mut session.document, &mut session.selection)).unwrap());
                    model = expected;
                    prop_assert_eq!(session.document.export_docx().unwrap(), bytes);
                }
                3 if !redo.is_empty() => {
                    let (expected, bytes) = redo.pop().unwrap();
                    undo.push((model.clone(), session.document.export_docx().unwrap()));
                    prop_assert!(session.executor.redo(&mut CommandContext::new(&mut session.document, &mut session.selection)).unwrap());
                    model = expected;
                    prop_assert_eq!(session.document.export_docx().unwrap(), bytes);
                }
                4 => {
                    session = Session::new(session.reopened());
                    undo.clear();
                    redo.clear();
                }
                _ => {}
            }
            prop_assert_eq!(values(&session.document), model.clone());
            assert_sum(&session.document, &model);
            let reopened = session.reopened();
            prop_assert_eq!(values(&reopened), model.clone());
            assert_sum(&reopened, &model);
            capture(&reopened);
        }
    }
}

#[test]
fn bibliography_refresh_does_not_turn_hidden_heading_fields_into_visible_text() {
    use jade_docx_editor::commands::insert_citation::AddCitationSourceCommand;
    use jade_docx_editor::fields::{FieldCommand, FieldEnvironment, FieldScope};
    use jade_docx_editor::word::references::citations::set_style_command;
    use jade_docx_editor::word_document::CitationSourceInfo;

    for complex in [false, true] {
        for instruction in ["SET Counter 10", "ASK Counter Prompt"] {
            for f9 in [false, true] {
                let body = format!(
                    r#"<w:p/><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>BIBLIOGRAPHY</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{}<w:r><w:t>References</w:t></w:r></w:p><w:p><w:r><w:t>Old bibliography</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
                    hidden(instruction, 900, complex),
                );
                let mut session = Session::open(&body, &[]);
                session.run(Box::new(AddCitationSourceCommand::new(
                    CitationSourceInfo {
                        tag: "One".into(),
                        source_type: "Book".into(),
                        author: "Smith, John".into(),
                        title: "A source".into(),
                        year: "2020".into(),
                        publisher: None,
                        url: None,
                    },
                )));
                session
                    .document
                    .set_bibliography_style("IEEE", "\\IEEE2006OfficeOnline.xsl")
                    .unwrap();
                assert_eq!(
                    session.document.body_paragraph_text(1).as_deref(),
                    Some("References")
                );
                let before = session.document.export_docx().unwrap();
                if f9 {
                    session.run(Box::new(FieldCommand::update(
                        FieldScope::Document,
                        FieldEnvironment::default(),
                    )));
                } else {
                    session.run(Box::new(set_style_command(
                        "IEEE".into(),
                        "\\IEEE2006OfficeOnline.xsl".into(),
                    )));
                }
                assert_eq!(
                    session.document.body_paragraph_text(1).as_deref(),
                    Some("References"),
                    "{instruction}, complex={complex}, f9={f9}"
                );
                assert_eq!(
                    session.document.body_paragraph_text(2).as_deref(),
                    Some("[1] J. Smith, A source, 2020.")
                );
                let after = session.document.export_docx().unwrap();
                let reopened = session.reopened();
                assert_eq!(
                    reopened.body_paragraph_text(1).as_deref(),
                    Some("References")
                );
                capture(&reopened);
                session.undo();
                assert_eq!(session.document.export_docx().unwrap(), before);
                session.redo();
                assert_eq!(session.document.export_docx().unwrap(), after);
            }
        }
    }
}
