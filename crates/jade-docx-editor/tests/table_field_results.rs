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
                .map(|run| run.cached_display_text())
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
fn addin_citations_keep_visible_text_formatting_and_external_code_on_f9() {
    for complex in [false, true] {
        for locked in [false, true] {
            let code = "ADDIN ZOTERO_ITEM CSL_CITATION {&quot;id&quot;:&quot;sample&quot;}";
            let result = "<w:r><w:rPr><w:i/><w:color w:val=\"123456\"/></w:rPr><w:t>(山田, 2020)</w:t></w:r>";
            let field = if complex {
                format!(
                    r#"<w:r><w:fldChar w:fldCharType="begin" w:fldLock="{locked}"/></w:r><w:r><w:instrText>{code}</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>{result}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
                )
            } else {
                format!(
                    r#"<w:fldSimple w:instr="{code}" w:fldLock="{locked}">{result}</w:fldSimple>"#
                )
            };
            let mut session = Session::open(&format!("<w:p>{field}</w:p>"), &[]);
            let before = session.document.export_docx().unwrap();
            assert_eq!(formula_result(&session.document, 0), "(山田, 2020)");
            for selection_only in [false, true] {
                update_formula(&mut session, 0, selection_only);
                assert_eq!(formula_result(&session.document, 0), "(山田, 2020)");
                // The owner, rather than Jade's F9 evaluator, controls both code and result.
                assert_eq!(session.document.export_docx().unwrap(), before);
                let reopened = session.reopened();
                assert_eq!(formula_result(&reopened, 0), "(山田, 2020)");
                capture(&reopened);
            }
            let xml = common::part(&before, "word/document.xml");
            assert!(xml.contains(code));
            assert!(xml.contains("<w:i/>"));
            assert!(xml.contains("123456"));
        }
    }
}

#[test]
fn addin_results_contribute_to_table_sorting_and_f9_through_reload() {
    for complex in [false, true] {
        let mut session = Session::open(
            &formula_body(&[(3, 900), (5, 1)], "ADDIN vendor-payload", complex),
            &[],
        );
        assert_eq!(formula_result(&session.document, 0), "9003");
        assert_eq!(formula_result(&session.document, 1), "15");
        for selection_only in [false, true] {
            update_formula(&mut session, 2, selection_only);
            assert_eq!(formula_result(&session.document, 2), "9018");
            let bytes = session.document.export_docx().unwrap();
            assert!(common::part(&bytes, "word/document.xml").contains("ADDIN vendor-payload"));
            session = Session::new(session.reopened());
            assert_eq!(formula_result(&session.document, 0), "9003");
            assert_eq!(formula_result(&session.document, 1), "15");
            assert_eq!(formula_result(&session.document, 2), "9018");
            capture(&session.document);
        }
        let mut sortable = Session::open(
            &body(&[(3, 900), (5, 1)], "ADDIN vendor-payload", complex),
            &[],
        );
        sort(&mut sortable, false);
        assert_eq!(values(&sortable.document), [15, 9003]);
        assert_eq!(values(&sortable.reopened()), [15, 9003]);
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
        addin in any::<bool>(),
    ) {
        let instruction = if addin { "ADDIN vendor-payload" } else if ask { "ASK Counter Prompt" } else { "SET Counter 10" };
        let mut session = Session::open(&body(&input, instruction, complex), &[]);
        // ADDIN's owner supplies the visible result; assignment caches are hidden.
        let mut model: Vec<i32> = input.iter().map(|(value, cache)| {
            if addin { cache * 10i32.pow(value.ilog10() + 1) + value } else { *value }
        }).collect();
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
        for instruction in [
            "SET Counter 10",
            "ASK Counter Prompt",
            "ADDIN vendor-payload",
        ] {
            let heading = if instruction.starts_with("ADDIN ") {
                "900References"
            } else {
                "References"
            };
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
                    Some(heading)
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
                    Some(heading),
                    "{instruction}, complex={complex}, f9={f9}"
                );
                assert_eq!(
                    session.document.body_paragraph_text(2).as_deref(),
                    Some("[1] J. Smith, A source, 2020.")
                );
                let after = session.document.export_docx().unwrap();
                let reopened = session.reopened();
                assert_eq!(reopened.body_paragraph_text(1).as_deref(), Some(heading));
                capture(&reopened);
                session.undo();
                assert_eq!(session.document.export_docx().unwrap(), before);
                session.redo();
                assert_eq!(session.document.export_docx().unwrap(), after);
            }
        }
    }
}

/// The dialog queries consume the last paint's cell registrations, as the shells do.
struct TableDialogSession {
    document: WordDocument,
    cells: Vec<jade_docx_editor::word::CellLayout>,
}

impl jade_docx_editor::word::WordSession for TableDialogSession {
    fn word_document(&self) -> &WordDocument {
        &self.document
    }
    fn selection_range(&self) -> Option<jade_selection::SelectionRange> {
        Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(3, 0),
        ))
    }
    fn set_selection_range(&mut self, _: Option<jade_selection::SelectionRange>) {
        panic!("read only")
    }
    fn apply_command(&mut self, _: HeadlessCommand, _: bool) -> Result<(), String> {
        panic!("read only")
    }
    fn apply_commands(&mut self, _: Vec<HeadlessCommand>, _: bool) -> Result<(), String> {
        panic!("read only")
    }
    fn execute_command(
        &mut self,
        _: Box<dyn jade_docx_editor::commands::Command>,
        _: bool,
    ) -> Result<(), String> {
        panic!("read only")
    }
    fn cell_id_for_paragraph(&self, paragraph: usize) -> Option<usize> {
        self.cells
            .iter()
            .position(|cell| cell.paragraphs.contains(&paragraph))
    }
    fn cell_layout(&self, id: usize) -> Option<&jade_docx_editor::word::CellLayout> {
        self.cells.get(id)
    }
    fn cell_layouts(&self) -> &[jade_docx_editor::word::CellLayout] {
        &self.cells
    }
    fn table_order_for(&self, id: usize) -> Option<usize> {
        (id == 0).then_some(0)
    }
    fn hyperlink_regions(&self) -> Vec<jade_docx_editor::canvas_renderer::HyperlinkRegion> {
        vec![]
    }
    fn select_cells(&mut self, _: usize, _: usize) -> Result<(), String> {
        panic!("read only")
    }
}

fn dialog_session(above: &str) -> TableDialogSession {
    let document = load(&common::package(
        &format!(
            r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:tcPr/><w:p>{above}</w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>3</w:t></w:r></w:p></w:tc><w:tc><w:tcPr/><w:p/></w:tc></w:tr></w:tbl><w:p/>"#
        ),
        &[],
    ));
    TableDialogSession {
        document,
        cells: (0..4)
            .map(|index| jade_docx_editor::word::CellLayout {
                paragraphs: vec![index],
                row: index / 2,
                column: index % 2,
                column_span: 1,
                ..Default::default()
            })
            .collect(),
    }
}

#[test]
fn table_dialogs_ignore_hidden_field_caches_before_and_after_reload() {
    use jade_docx_editor::word::table::WordTables;
    for complex in [false, true] {
        for instruction in ["SET Counter 10", "ASK Counter Prompt"] {
            let mut session = dialog_session(&format!(
                "{}<w:r><w:t>Amount</w:t></w:r>",
                hidden(instruction, 900, complex)
            ));
            let before = session.document.export_docx().unwrap();
            for _ in 0..2 {
                assert_eq!(session.sort_columns(true), ["Item", "Amount"]);
                assert_eq!(session.suggested_formula(), "=SUM(LEFT)");
                assert_eq!(
                    session.document.export_docx().unwrap(),
                    before,
                    "dialog queries preserve field instructions and caches"
                );
                capture(&session.document);
                session.document = load(&before);
            }
        }
    }
}

#[test]
fn table_sort_dialog_includes_computed_field_results_before_and_after_reload() {
    use jade_docx_editor::word::table::WordTables;
    for complex in [false, true] {
        let mut session = dialog_session(&hidden("DOCVARIABLE Total", 12, complex));
        for _ in 0..2 {
            assert_eq!(session.sort_columns(true), ["Item", "12"]);
            capture(&session.document);
            session.document = load(&session.document.export_docx().unwrap());
        }
    }
}

#[test]
fn table_formula_dialog_includes_computed_field_results_before_and_after_reload() {
    use jade_docx_editor::word::table::WordTables;
    for complex in [false, true] {
        let mut session = dialog_session(&hidden("DOCVARIABLE Total", 12, complex));
        for _ in 0..2 {
            assert_eq!(session.suggested_formula(), "=SUM(ABOVE)");
            capture(&session.document);
            session.document = load(&session.document.export_docx().unwrap());
        }
    }
}

fn formula_body(input: &[(i32, i32)], instruction: &str, complex: bool) -> String {
    body(input, instruction, complex).replace(
        "</w:tbl>",
        &format!(
            "<w:tr><w:tc><w:tcPr/><w:p>{}</w:p></w:tc></w:tr></w:tbl>",
            hidden("=SUM(ABOVE)", 0, !complex)
        ),
    )
}

fn formula_result(document: &WordDocument, paragraph: usize) -> String {
    document
        .paragraph(paragraph)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.cached_display_text())
        .collect()
}

fn update_formula(session: &mut Session, paragraph: usize, selection_only: bool) {
    use jade_docx_editor::fields::{FieldCommand, FieldEnvironment, FieldScope};
    use jade_selection::{SelectionRange, TextPosition};
    let scope = if selection_only {
        FieldScope::Selection(SelectionRange::collapsed(TextPosition::new(paragraph, 0)))
    } else {
        FieldScope::Document
    };
    session.run(Box::new(FieldCommand::update(
        scope,
        FieldEnvironment::default(),
    )));
}

#[test]
fn f9_table_formulas_ignore_hidden_caches_through_history_and_reload() {
    for complex in [false, true] {
        for instruction in [
            "SET Counter 10",
            "ASK Counter Prompt",
            "PRIVATE",
            "XE Index",
            "TA Authority",
            "TC Entry",
            "RD chapter.docx",
        ] {
            for selection_only in [false, true] {
                let xml = formula_body(&[(3, 900), (5, 1)], instruction, complex);
                let mut session = Session::open(&xml, &[]);
                let before = session.document.export_docx().unwrap();
                update_formula(&mut session, 2, selection_only);
                assert_eq!(
                    formula_result(&session.document, 2),
                    "8",
                    "complex={complex}, instruction={instruction}, selection={selection_only}"
                );
                let after = session.document.export_docx().unwrap();
                session.undo();
                assert_eq!(session.document.export_docx().unwrap(), before);
                session.redo();
                assert_eq!(session.document.export_docx().unwrap(), after);
                session = Session::new(session.reopened());
                update_formula(&mut session, 2, selection_only);
                assert_eq!(formula_result(&session.document, 2), "8");
                capture(&session.document);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    #[test]
    fn f9_formula_history_matches_visible_sum(
        input in prop::collection::vec((1i32..100, 100i32..1000), 2..9),
        operations in prop::collection::vec(0u8..5, 1..15),
        complex in any::<bool>(),
        instruction in prop::sample::select(vec!["SET Counter 10", "ASK Counter Prompt", "PRIVATE", "ADDIN vendor-payload", "XE Index", "TA Authority", "TC Entry", "RD chapter.docx"]),
    ) {
        let mut session = Session::open(&formula_body(&input, instruction, complex), &[]);
        let sum = input.iter().map(|(visible, cache)| {
            if instruction.starts_with("ADDIN ") {
                cache * 10i32.pow(visible.ilog10() + 1) + visible
            } else {
                *visible
            }
        }).sum::<i32>().to_string();
        let mut model = "0".to_string();
        let mut undo = Vec::new();
        let mut redo = Vec::new();
        for operation in operations {
            match operation {
                0 | 1 => {
                    undo.push((model.clone(), session.document.export_docx().unwrap()));
                    redo.clear();
                    update_formula(&mut session, input.len(), operation == 1);
                    model = sum.clone();
                }
                2 if !undo.is_empty() => {
                    let (expected, bytes) = undo.pop().unwrap();
                    redo.push((model.clone(), session.document.export_docx().unwrap()));
                    session.undo();
                    model = expected;
                    prop_assert_eq!(session.document.export_docx().unwrap(), bytes);
                }
                3 if !redo.is_empty() => {
                    let (expected, bytes) = redo.pop().unwrap();
                    undo.push((model.clone(), session.document.export_docx().unwrap()));
                    session.redo();
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
            prop_assert_eq!(formula_result(&session.document, input.len()), model.as_str());
            let reopened = session.reopened();
            prop_assert_eq!(formula_result(&reopened, input.len()), model.as_str());
            capture(&reopened);
        }
    }
}
