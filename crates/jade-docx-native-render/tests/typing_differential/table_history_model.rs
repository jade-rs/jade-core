//! Structural table edits renumber paragraphs in every following table.

use super::{assert_page_pixels, open_bytes, package, Doc, COORD_EPSILON};
use jade_docx_headless_api::HeadlessCommand;
use proptest::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    body: [String; 3],
    tables: [Vec<Vec<String>>; 2],
}

impl Default for State {
    fn default() -> Self {
        Self {
            body: [
                "before 日本語".into(),
                "between 😀".into(),
                "after e\u{301}".into(),
            ],
            tables: [
                vec![
                    vec!["A 日".into(), "B 😀".into()],
                    vec!["C".into(), "D".into()],
                ],
                vec![vec!["one".into(), "two 日本語".into(), "three".into()]],
            ],
        }
    }
}

impl State {
    fn body_paragraph(&self, body: usize) -> usize {
        body + self.tables[..body.min(2)]
            .iter()
            .flatten()
            .map(Vec::len)
            .sum::<usize>()
    }

    fn cell_paragraph(&self, table: usize, row: usize, column: usize) -> usize {
        self.body_paragraph(table) + 1 + row * self.tables[table][0].len() + column
    }

    fn paragraphs(&self) -> Vec<&str> {
        let mut paragraphs = Vec::new();
        for index in 0..3 {
            paragraphs.push(self.body[index].as_str());
            if index < 2 {
                paragraphs.extend(self.tables[index].iter().flatten().map(String::as_str));
            }
        }
        paragraphs
    }

    fn package(&self) -> Vec<u8> {
        fn paragraph(text: &str) -> String {
            let text = text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            format!("<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>")
        }
        let mut body = String::new();
        for index in 0..3 {
            body.push_str(&paragraph(&self.body[index]));
            if index == 2 {
                break;
            }
            let columns = self.tables[index][0].len();
            let width = 5760 / columns;
            body.push_str(r#"<w:tbl><w:tblPr><w:tblW w:w="5760" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblBorders>"#);
            for edge in ["top", "left", "bottom", "right", "insideH", "insideV"] {
                body.push_str(&format!(
                    "<w:{edge} w:val=\"single\" w:sz=\"8\" w:color=\"000000\"/>"
                ));
            }
            body.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
            for _ in 0..columns {
                body.push_str(&format!("<w:gridCol w:w=\"{width}\"/>"));
            }
            body.push_str("</w:tblGrid>");
            for row in &self.tables[index] {
                body.push_str("<w:tr>");
                for text in row {
                    body.push_str(&format!(
                        "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/></w:tcPr>{}</w:tc>",
                        paragraph(text)
                    ));
                }
                body.push_str("</w:tr>");
            }
            body.push_str("</w:tbl>");
        }
        package(&body, "")
    }

    fn check_state(&self, document: &Doc, context: &str) {
        let paragraphs = self.paragraphs();
        assert_eq!(
            document.paragraph_count(),
            paragraphs.len(),
            "{context}: paragraph count"
        );
        for (index, text) in paragraphs.iter().enumerate() {
            assert_eq!(
                document.paragraph_text(index).as_deref(),
                Some(*text),
                "{context}: paragraph {index}"
            );
        }
        for (table, rows) in self.tables.iter().enumerate() {
            let snapshot = document.table_snapshot(table).expect("table snapshot");
            assert_eq!(snapshot.table_order_index, table, "{context}");
            assert_eq!(
                snapshot.rows.len(),
                rows.len(),
                "{context}: table {table} rows"
            );
            assert_eq!(
                snapshot.columns.len(),
                rows[0].len(),
                "{context}: table {table} columns"
            );
            for (row, cells) in rows.iter().enumerate() {
                assert_eq!(snapshot.rows[row].cells.len(), cells.len(), "{context}");
                for (column, text) in cells.iter().enumerate() {
                    let cell = &snapshot.rows[row].cells[column];
                    assert_eq!((cell.row, cell.column), (row, column), "{context}");
                    assert_eq!(
                        &cell.text, text,
                        "{context}: table {table}, row {row}, column {column}"
                    );
                    let paragraph = self.cell_paragraph(table, row, column);
                    let renderer = document.renderer();
                    let cell_id = renderer.cell_id_for_paragraph(paragraph).expect("cell ID");
                    let layout = renderer.cell_layout(cell_id).expect("cell layout");
                    assert_eq!(
                        (
                            renderer.table_order_for(layout.table_id),
                            layout.row,
                            layout.column,
                        ),
                        (Some(table), row, column),
                        "{context}: cell mapping {paragraph}"
                    );
                }
            }
        }
        assert!(
            document.table_snapshot(2).is_none(),
            "{context}: no extra table"
        );
        for body in 0..3 {
            assert!(
                document
                    .renderer()
                    .cell_id_for_paragraph(self.body_paragraph(body))
                    .is_none(),
                "{context}: body is outside tables"
            );
        }
    }

    fn check_render(&self, actual: &mut Doc, context: &str) {
        // Rebuild only from plain strings and grid dimensions, without any
        // mutation, renderer state, or table AST copied from the actual document.
        let mut expected = open_bytes(&self.package());
        expected.set_selection_range(actual.selection_range());
        expected.repaint_all().unwrap();
        assert_eq!(
            actual.page_count(),
            expected.page_count(),
            "{context}: page count"
        );
        for page in 0..expected.page_count() {
            assert_page_pixels(actual, &expected, page, context);
        }
        for (paragraph, text) in self.paragraphs().iter().enumerate() {
            for offset in [0, text.len()] {
                let have = actual
                    .position_rect(paragraph, offset)
                    .expect("actual caret");
                let want = expected
                    .position_rect(paragraph, offset)
                    .expect("expected caret");
                for (have, want) in [
                    (have.x, want.x),
                    (have.document_y, want.document_y),
                    (have.height, want.height),
                ] {
                    assert!(
                        (have - want).abs() < COORD_EPSILON,
                        "{context}: caret {paragraph}:{offset}: {have} != {want}"
                    );
                }
            }
        }
        assert_eq!(
            actual.renderer().selection_range(),
            actual.selection_range(),
            "{context}: geometry queries preserve selection"
        );
    }
}

#[derive(Default)]
struct Model {
    state: State,
    undo: Vec<State>,
    redo: Vec<State>,
}

impl Model {
    fn change(&mut self, next: State) {
        self.undo.push(std::mem::replace(&mut self.state, next));
        self.redo.clear();
    }

    fn check(&self, document: &mut Doc, context: &str) {
        self.state.check_state(document, context);
        assert_eq!(
            document.history_len(),
            self.undo.len(),
            "{context}: history"
        );
        assert_eq!(
            document.can_undo(),
            !self.undo.is_empty(),
            "{context}: undo"
        );
        assert_eq!(
            document.can_redo(),
            !self.redo.is_empty(),
            "{context}: redo"
        );
        assert_eq!(
            document.renderer().selection_range(),
            document.selection_range(),
            "{context}: renderer selection"
        );
        self.state.check_render(document, context);
    }
}

#[derive(Clone, Debug)]
enum Operation {
    Row(u8, u8, bool),
    CellText(u8, u8, u8, String),
    BodyText(u8, String),
    Outside(bool),
    FailedBatch(u8),
    Merge(u8),
    Undo,
    Redo,
}

fn step(model: &mut Model, document: &mut Doc, operation: &Operation, index: usize) {
    let context = format!("step {index}: {operation:?}");
    match operation {
        Operation::Row(table, row, insert) => {
            let table = usize::from(*table) % 2;
            let row = usize::from(*row) % model.state.tables[table].len();
            let paragraph = model.state.cell_paragraph(table, row, 0);
            // Paint the new selection before the edit: rejected commands and
            // empty insertions intentionally leave the previous paint alone.
            document
                .select((paragraph, 0), (paragraph, 0), usize::MAX)
                .unwrap();
            let before = document.selection_range();
            let result = document.edit_table_row(*insert);
            if !insert && model.state.tables[table].len() == 1 {
                assert!(result.is_err(), "{context}: deleting the only row");
                assert_eq!(
                    document.selection_range(),
                    before,
                    "{context}: rejected selection"
                );
            } else {
                result.unwrap();
                let mut next = model.state.clone();
                if *insert {
                    next.tables[table]
                        .insert(row + 1, vec![String::new(); next.tables[table][0].len()]);
                } else {
                    next.tables[table].remove(row);
                }
                model.change(next);
            }
        }
        Operation::CellText(table, row, column, text) => {
            let table = usize::from(*table) % 2;
            let row = usize::from(*row) % model.state.tables[table].len();
            let column = usize::from(*column) % model.state.tables[table][row].len();
            let paragraph = model.state.cell_paragraph(table, row, column);
            let offset = model.state.tables[table][row][column].len();
            document
                .select((paragraph, offset), (paragraph, offset), usize::MAX)
                .unwrap();
            document
                .insert_text(paragraph, offset, text, usize::MAX)
                .unwrap();
            if !text.is_empty() {
                let mut next = model.state.clone();
                next.tables[table][row][column].push_str(text);
                model.change(next);
            }
        }
        Operation::BodyText(body, text) => {
            let body = usize::from(*body) % 3;
            let paragraph = model.state.body_paragraph(body);
            let offset = model.state.body[body].len();
            document
                .select((paragraph, offset), (paragraph, offset), usize::MAX)
                .unwrap();
            document
                .insert_text(paragraph, offset, text, usize::MAX)
                .unwrap();
            if !text.is_empty() {
                let mut next = model.state.clone();
                next.body[body].push_str(text);
                model.change(next);
            }
        }
        Operation::Outside(insert) => {
            let paragraph = model.state.body_paragraph(1);
            document
                .select((paragraph, 0), (paragraph, "between".len()), usize::MAX)
                .unwrap();
            let before = document.selection_range();
            assert!(
                document.edit_table_row(*insert).is_err(),
                "{context}: outside a table"
            );
            assert_eq!(
                document.selection_range(),
                before,
                "{context}: rejected selection"
            );
        }
        Operation::FailedBatch(table) => {
            let table = usize::from(*table) % 2;
            let before = document.selection_range();
            assert!(
                document
                    .apply_commands(
                        vec![
                            HeadlessCommand::InsertTableRow {
                                table_order_index: table,
                                row_index: 0,
                                template_row: Some(0)
                            },
                            HeadlessCommand::SetTableCellText {
                                table_order_index: table,
                                row: 0,
                                column: 0,
                                text: "discarded 日本語".into()
                            },
                            HeadlessCommand::DeleteTableRow {
                                table_order_index: 2,
                                row_index: 0
                            },
                        ],
                        false
                    )
                    .is_err(),
                "{context}: failed structural batch"
            );
            assert_eq!(
                document.selection_range(),
                before,
                "{context}: failed batch selection"
            );
        }
        Operation::Merge(mark) => {
            let mark = usize::from(*mark);
            document.merge_history_since(mark);
            if mark < model.undo.len() {
                model.undo.truncate(mark + 1);
            }
        }
        Operation::Undo => {
            document.undo().unwrap();
            if let Some(previous) = model.undo.pop() {
                model
                    .redo
                    .push(std::mem::replace(&mut model.state, previous));
            }
        }
        Operation::Redo => {
            document.redo().unwrap();
            if let Some(next) = model.redo.pop() {
                model.undo.push(std::mem::replace(&mut model.state, next));
            }
        }
    }
    model.check(document, &context);
}

fn check_sequence(operations: &[Operation]) {
    let mut model = Model::default();
    let mut document = open_bytes(&model.state.package());
    model.check(&mut document, "initial");
    for (index, operation) in operations.iter().enumerate() {
        step(&mut model, &mut document, operation, index);
    }
    let mut reopened = open_bytes(&document.export_docx().unwrap());
    model.state.check_state(&reopened, "export/reopen");
    model.state.check_render(&mut reopened, "export/reopen");
    assert!(!reopened.can_undo());
    assert!(!reopened.can_redo());
    let tip = model
        .redo
        .first()
        .cloned()
        .unwrap_or_else(|| model.state.clone());
    let mut index = operations.len();
    while !model.undo.is_empty() {
        step(&mut model, &mut document, &Operation::Undo, index);
        index += 1;
    }
    assert_eq!(model.state, State::default(), "full undo");
    while !model.redo.is_empty() {
        step(&mut model, &mut document, &Operation::Redo, index);
        index += 1;
    }
    assert_eq!(model.state, tip, "full redo");
}

#[test]
fn rejected_row_edit_preserves_the_last_paint_after_moving_the_caret() {
    let state = State::default();
    let mut document = open_bytes(&state.package());
    let paragraph = state.body_paragraph(1);
    document
        .select((paragraph, 0), (paragraph, 7), usize::MAX)
        .unwrap();
    let pixels: Vec<_> = (0..document.page_count())
        .map(|page| document.page_pixels(page).unwrap())
        .collect();
    document
        .place_caret(state.cell_paragraph(1, 0, 0), 0)
        .unwrap();
    let selection = document.selection_range();
    assert!(document.edit_table_row(false).is_err());
    assert_eq!(document.selection_range(), selection);
    assert_eq!(document.renderer().selection_range(), selection);
    assert_eq!(document.history_len(), 0);
    assert!(!document.can_redo());
    state.check_state(&document, "rejected row edit");
    assert_eq!(document.page_count(), pixels.len());
    for (page, before) in pixels.into_iter().enumerate() {
        assert!(
            document.page_pixels(page).unwrap() == before,
            "rejected edit must preserve the last paint on page {page}"
        );
    }
}

#[test]
fn querying_geometry_after_a_structural_redo_preserves_the_selection() {
    let state = State::default();
    let mut document = open_bytes(&state.package());
    document
        .place_caret(state.cell_paragraph(0, 0, 0), 0)
        .unwrap();
    document.edit_table_row(true).unwrap();
    document.undo().unwrap();
    let paragraph = state.body_paragraph(1);
    document
        .select((paragraph, 0), (paragraph, 7), usize::MAX)
        .unwrap();
    assert!(document.edit_table_row(false).is_err());
    document.redo().unwrap();
    let selection = document.selection_range();
    assert_eq!(
        document.renderer().selection_range(),
        selection,
        "before geometry query"
    );
    document.position_rect(0, 0).unwrap();
    assert_eq!(
        document.selection_range(),
        selection,
        "shell selection after geometry query"
    );
    assert_eq!(
        document.renderer().selection_range(),
        selection,
        "renderer selection after geometry query"
    );
}

#[test]
fn rows_and_cell_typing_keep_both_table_addresses_through_grouped_history() {
    use Operation::*;
    check_sequence(&[
        Row(0, 0, true),
        CellText(0, 1, 1, "new 日本語 😀".into()),
        Row(1, 0, true),
        CellText(1, 1, 2, "折り返し 日本語 ab ".repeat(12)),
        BodyText(2, " end".into()),
        Merge(1),
        Undo,
        Redo,
        Row(0, 0, false),
        Undo,
        CellText(1, 0, 1, " fork".into()),
        Redo,
    ]);
}

#[test]
fn rejected_row_edits_and_empty_cell_typing_preserve_the_redo_branch() {
    use Operation::*;
    check_sequence(&[
        Row(0, 0, false),
        Row(1, 0, true),
        Undo,
        Row(0, 0, false),
        Row(1, 0, false),
        Outside(true),
        Outside(false),
        CellText(0, 0, 0, String::new()),
        Redo,
        BodyText(1, " & <body>".into()),
    ]);
}

#[test]
fn failed_table_batches_restore_rows_text_selection_and_redo() {
    use Operation::*;
    check_sequence(&[
        Row(0, 1, true),
        CellText(0, 2, 0, "pending".into()),
        Undo,
        Outside(false),
        FailedBatch(0),
        FailedBatch(1),
        Redo,
        Row(1, 0, true),
        Row(1, 1, false),
    ]);
}

fn operation() -> impl Strategy<Value = Operation> {
    let text = prop::sample::select(vec![
        String::new(),
        "日".into(),
        "😀e\u{301}".into(),
        " & <xml> ".into(),
        "折り返し 日本語 ab ".repeat(8),
    ]);
    prop_oneof![
        5 => (0u8..2, any::<u8>(), any::<bool>()).prop_map(|(table, row, insert)| Operation::Row(table, row, insert)),
        4 => (0u8..2, any::<u8>(), any::<u8>(), text.clone()).prop_map(|(table, row, column, text)| Operation::CellText(table, row, column, text)),
        1 => (0u8..3, text).prop_map(|(body, text)| Operation::BodyText(body, text)),
        1 => any::<bool>().prop_map(Operation::Outside),
        1 => (0u8..2).prop_map(Operation::FailedBatch),
        1 => (0u8..8).prop_map(Operation::Merge),
        3 => Just(Operation::Undo),
        3 => Just(Operation::Redo),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 512,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/typing_differential/table_history_model.seeds")
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn structural_table_edits_follow_an_independent_history_model(
        operations in prop::collection::vec(operation(), 1..=16)
    ) {
        check_sequence(&operations);
    }
}
