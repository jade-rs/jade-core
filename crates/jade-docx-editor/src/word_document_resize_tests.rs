// Stateful resize coverage at the command/document boundary. The reference
// model only stores dimensions and history; it never calls production resize
// helpers to calculate the expected outcome.
use super::*;
use crate::commands::resize_table_row::ResizeTableRowCommand;
use crate::commands::Command;

#[derive(Clone, Debug, PartialEq)]
struct Dimensions {
    widths: Vec<f32>,
    heights: Vec<f32>,
    fixed: bool,
}

#[derive(Clone, Debug)]
enum Step {
    Column(usize, usize, i16, bool),
    Columns(usize, Vec<i16>, bool),
    Row(usize, usize, i16, bool),
    NonFinite(usize, bool, bool),
    Undo,
    Redo,
    Reopen,
}

fn steps() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0usize..5, 0usize..6, -20i16..160, any::<bool>())
            .prop_map(|(t, i, v, pair)| Step::Column(t, i, v, pair)),
        3 => (0usize..5, prop_oneof![3 => prop::collection::vec(0i16..160, 3), 1 => prop::collection::vec(-20i16..160, 0..5)], any::<bool>())
            .prop_map(|(t, v, rendered)| Step::Columns(t, v, rendered)),
        4 => (0usize..5, 0usize..6, -20i16..160, any::<bool>())
            .prop_map(|(t, i, v, pair)| Step::Row(t, i, v, pair)),
        1 => (0usize..3, any::<bool>(), any::<bool>())
            .prop_map(|(t, row, nan)| Step::NonFinite(t, row, nan)),
        3 => Just(Step::Undo),
        3 => Just(Step::Redo),
        2 => Just(Step::Reopen),
    ]
}

fn expected_error(step: &Step) -> Option<String> {
    let message = match step {
        Step::Columns(_, widths, _) if widths.is_empty() => {
            "set_table_column_widths requires at least one width".into()
        }
        Step::Columns(_, widths, _) if widths.iter().any(|v| *v < 0) => {
            "widths must be finite non-negative numbers".into()
        }
        Step::Columns(t, _, _) | Step::Column(t, _, _, _) | Step::Row(t, _, _, _) if *t >= 3 => {
            "target table not found".into()
        }
        Step::Columns(t, widths, _) if widths.len() != 3 => format!(
            "table {t} has 3 columns but {} widths were given",
            widths.len()
        ),
        Step::Column(_, i, _, _) if *i >= 3 => {
            format!("column {i} is out of range; the table has 3 columns")
        }
        Step::Column(_, 2, _, true) => "column 3 is out of range; the table has 3 columns".into(),
        Step::Row(_, i, _, _) if *i >= 4 => "target row not found".into(),
        Step::Row(_, 3, _, true) => "adjacent row missing".into(),
        Step::NonFinite(_, true, _) => "table row height must be finite".into(),
        Step::NonFinite(_, false, _) => "table column width must be finite".into(),
        _ => return None,
    };
    Some(message)
}

fn nested_xml(depth: usize) -> String {
    let nested = if depth < 2 {
        nested_xml(depth + 1)
    } else {
        String::new()
    };
    let cell = |properties: &str, text: &str| {
        format!("<w:tc><w:tcPr>{properties}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>")
    };
    let row = |properties: &str, cells: String| {
        format!(
        "<w:tr><w:trPr><w:trHeight w:val=\"400\" w:hRule=\"exact\"/>{properties}</w:trPr>{cells}</w:tr>"
    )
    };
    // A spanning cell, a vertical merge, an omitted leading/trailing grid and
    // a nested table coexist at every depth. Explicit bottom margins differ
    // by depth so row resize must use the target table's own margin.
    let host = format!("<w:tc><w:tcPr><w:tcW w:w=\"2000\" w:type=\"dxa\"/></w:tcPr><w:p/><w:p><w:r><w:t>host{depth}</w:t></w:r></w:p>{nested}<w:p/></w:tc>");
    let rows = [
        row("", cell("<w:tcW w:w=\"2800\" w:type=\"dxa\"/><w:gridSpan w:val=\"2\"/>", &format!("merged{depth}")) + &host),
        row("", cell("<w:tcW w:w=\"1200\" w:type=\"dxa\"/><w:vMerge w:val=\"restart\"/>", "vertical") + &cell("<w:tcW w:w=\"1600\" w:type=\"dxa\"/>", "middle") + &cell("<w:tcW w:w=\"2000\" w:type=\"dxa\"/>", "right")),
        row("", cell("<w:tcW w:w=\"1200\" w:type=\"dxa\"/><w:vMerge/>", "") + &cell("<w:tcW w:w=\"1600\" w:type=\"dxa\"/>", "bottom") + &cell("<w:tcW w:w=\"2000\" w:type=\"dxa\"/>", "edge")),
        row("<w:gridBefore w:val=\"1\"/><w:gridAfter w:val=\"1\"/><w:wBefore w:w=\"1200\" w:type=\"dxa\"/><w:wAfter w:w=\"2000\" w:type=\"dxa\"/>", cell("<w:tcW w:w=\"1600\" w:type=\"dxa\"/>", "omitted")),
    ].join("");
    format!("<w:tbl><w:tblPr><w:tblLayout w:type=\"autofit\"/><w:tblCellMar><w:bottom w:w=\"{}\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w=\"1200\"/><w:gridCol w:w=\"1600\"/><w:gridCol w:w=\"2000\"/></w:tblGrid>{rows}</w:tbl>", (depth + 1) * 20)
}

fn assert_dimensions(doc: &WordDocument, expected: &[Dimensions]) {
    for (order, state) in expected.iter().enumerate() {
        let table = doc.table_by_order(order).expect("nested table survives");
        assert_eq!(
            table
                .columns
                .iter()
                .map(|c| c.width.unwrap())
                .collect::<Vec<_>>(),
            state.widths,
            "table {order} grid"
        );
        assert_eq!(
            table.table_layout == Some(TableLayoutMode::Fixed),
            state.fixed,
            "table {order} layout"
        );
        assert_eq!(table.rows.len(), 4);
        for (index, row) in table.rows.iter().enumerate() {
            assert_eq!(
                row.height,
                Some(state.heights[index]),
                "table {order} row {index}"
            );
            assert_eq!(row.height_rule, Some(TableRowHeightRule::Exact));
            let mut column = row.grid_before.unwrap_or(0) as usize;
            for cell in &row.cells {
                let end = column + cell.span.unwrap_or(1) as usize;
                assert_eq!(
                    cell.style.get("width"),
                    Some(&format!(
                        "{:.2}pt",
                        state.widths[column..end].iter().sum::<f32>()
                    )),
                    "table {order} cell width"
                );
                column = end;
            }
            assert_eq!(column + row.grid_after.unwrap_or(0) as usize, 3);
        }
        let omitted = &table.rows[3];
        assert_eq!(
            omitted.width_before,
            Some(format!("{:.2}pt", state.widths[0]))
        );
        assert_eq!(
            omitted.width_after,
            Some(format!("{:.2}pt", state.widths[2]))
        );
        assert_eq!(table.rows[0].cells[0].span, Some(2));
        assert_eq!(
            table.rows[1].cells[0].vertical_merge.as_deref(),
            Some("restart")
        );
        assert_eq!(
            table.rows[2].cells[0].vertical_merge.as_deref(),
            Some("continue")
        );
    }
    assert!(doc.table_by_order(expected.len()).is_none());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn property_mixed_resize_history_matches_model(sequence in prop::collection::vec(steps(), 1..65)) {
        run_sequence(sequence)?;
    }
}

fn run_sequence(sequence: Vec<Step>) -> Result<(), proptest::test_runner::TestCaseError> {
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{}</w:body></w:document>"#,
        nested_xml(0)
    );
    let bytes = build_document_docx(&xml);
    let mut doc = executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
    let mut expected = vec![
        Dimensions {
            widths: vec![60.0, 80.0, 100.0],
            heights: vec![20.0; 4],
            fixed: false
        };
        3
    ];
    let mut undo = Vec::new();
    let mut redo = Vec::new();
    let mut commands = CommandExecutor::new();
    let mut selection = None;
    assert_dimensions(&doc, &expected);
    for (index, step) in sequence.iter().enumerate() {
        let mut next = expected.clone();
        let (command, valid): (Option<Box<dyn Command>>, bool) = match step {
            Step::Column(t, i, v, pair) => {
                let valid = *t < 3 && *i < if *pair { 2 } else { 3 };
                if valid {
                    next[*t].widths[*i] = (*v as f32).max(9.0);
                    if *pair {
                        next[*t].widths[*i + 1] = (160.0 - *v as f32).max(9.0);
                    }
                    next[*t].fixed = true;
                }
                let command = if *pair {
                    ResizeTableColumnCommand::between_columns(*t, *i, *v as f32, 160.0 - *v as f32)
                } else {
                    ResizeTableColumnCommand::table_right_edge(*t, *i, *v as f32)
                };
                (Some(Box::new(command)), valid)
            }
            Step::Columns(t, widths, rendered) => {
                let valid = *t < 3 && widths.len() == 3 && widths.iter().all(|v| *v >= 0);
                if valid {
                    next[*t].widths = widths
                        .iter()
                        .map(|v| {
                            if *rendered {
                                *v as f32
                            } else {
                                (*v as f32).max(9.0)
                            }
                        })
                        .collect();
                    next[*t].fixed = true;
                }
                let values = widths.iter().map(|v| *v as f32).collect();
                let command = if *rendered {
                    ResizeTableColumnCommand::rendered_columns(*t, values)
                } else {
                    ResizeTableColumnCommand::all_columns(*t, values)
                };
                (Some(Box::new(command)), valid)
            }
            Step::Row(t, i, v, pair) => {
                let valid = *t < 3 && *i < if *pair { 3 } else { 4 };
                if valid {
                    let margin = (*t + 1) as f32;
                    next[*t].heights[*i] = ((*v as f32).max(9.0) - margin).max(0.05);
                    if *pair {
                        next[*t].heights[*i + 1] =
                            ((160.0 - *v as f32).max(9.0) - margin).max(0.05);
                    }
                }
                let command = if *pair {
                    ResizeTableRowCommand::between_rows(*t, *i, *v as f32, 160.0 - *v as f32)
                } else {
                    ResizeTableRowCommand::table_bottom_edge(*t, *i, *v as f32)
                };
                (Some(Box::new(command)), valid)
            }
            Step::NonFinite(t, row, nan) => {
                let value = if *nan { f32::NAN } else { f32::INFINITY };
                let command: Box<dyn Command> = if *row {
                    Box::new(ResizeTableRowCommand::between_rows(*t, 0, 20.0, value))
                } else {
                    Box::new(ResizeTableColumnCommand::between_columns(
                        *t, 0, 60.0, value,
                    ))
                };
                (Some(command), false)
            }
            Step::Undo => {
                let previous = undo.pop();
                let actual = commands
                    .undo(&mut CommandContext::new(&mut doc, &mut selection))
                    .unwrap();
                prop_assert_eq!(actual, previous.is_some(), "step {} {:?}", index, step);
                if let Some(previous) = previous {
                    redo.push(expected.clone());
                    expected = previous;
                }
                (None, true)
            }
            Step::Redo => {
                let following = redo.pop();
                let actual = commands
                    .redo(&mut CommandContext::new(&mut doc, &mut selection))
                    .unwrap();
                prop_assert_eq!(actual, following.is_some(), "step {} {:?}", index, step);
                if let Some(following) = following {
                    undo.push(expected.clone());
                    expected = following;
                }
                (None, true)
            }
            Step::Reopen => {
                let bytes = doc.export_docx().unwrap();
                doc = executor::block_on(WordDocument::load(&bytes, &Options::default())).unwrap();
                // Reopening starts a new editing session/history.
                commands = CommandExecutor::new();
                undo.clear();
                redo.clear();
                selection = None;
                (None, true)
            }
        };
        if let Some(command) = command {
            let result =
                commands.execute(command, &mut CommandContext::new(&mut doc, &mut selection));
            prop_assert_eq!(
                result.is_ok(),
                valid,
                "step {} {:?}: {:?}",
                index,
                step,
                result
            );
            let actual_error = result.err().map(|error| match error {
                crate::commands::CommandError::Custom(message) => message,
                other => panic!("unexpected error at step {index} {step:?}: {other}"),
            });
            prop_assert_eq!(
                actual_error,
                expected_error(step),
                "step {} {:?}",
                index,
                step
            );
            if valid {
                undo.push(expected.clone());
                redo.clear();
                expected = next;
            }
        }
        prop_assert_eq!(
            commands.undo_depth(),
            undo.len(),
            "step {} {:?}",
            index,
            step
        );
        prop_assert_eq!(
            commands.can_redo(),
            !redo.is_empty(),
            "step {} {:?}",
            index,
            step
        );
        assert_dimensions(&doc, &expected);
    }
    let reopened = executor::block_on(WordDocument::load(
        &doc.export_docx().unwrap(),
        &Options::default(),
    ))
    .unwrap();
    assert_dimensions(&reopened, &expected);
    Ok(())
}

#[test]
fn rendered_resize_keeps_subminimum_columns_and_history() {
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{}</w:body></w:document>"#,
        nested_xml(0)
    );
    let mut doc = executor::block_on(WordDocument::load(
        &build_document_docx(&xml),
        &Options::default(),
    ))
    .unwrap();
    let mut commands = CommandExecutor::new();
    let mut selection = None;
    let widths = vec![0.55, 4.0, 5.0];
    commands
        .execute(
            Box::new(ResizeTableColumnCommand::rendered_columns(
                2,
                widths.clone(),
            )),
            &mut CommandContext::new(&mut doc, &mut selection),
        )
        .unwrap();
    assert_eq!(
        doc.table_by_order(2)
            .unwrap()
            .columns
            .iter()
            .map(|c| c.width.unwrap())
            .collect::<Vec<_>>(),
        widths
    );
    assert!(commands
        .undo(&mut CommandContext::new(&mut doc, &mut selection))
        .unwrap());
    assert_eq!(doc.table_by_order(2).unwrap().columns[0].width, Some(60.0));
    assert!(commands
        .redo(&mut CommandContext::new(&mut doc, &mut selection))
        .unwrap());
    let reopened = executor::block_on(WordDocument::load(
        &doc.export_docx().unwrap(),
        &Options::default(),
    ))
    .unwrap();
    assert_eq!(
        reopened
            .table_by_order(2)
            .unwrap()
            .columns
            .iter()
            .map(|c| c.width.unwrap())
            .collect::<Vec<_>>(),
        widths
    );
}

#[test]
fn dragged_widths_and_heights_are_the_whole_twips_a_save_keeps() {
    // A drag converts pixels to points, which are rarely whole twips, and
    // `w:gridCol` / `w:trHeight` hold nothing finer. The model laid the table
    // out unrounded and the file wrote it rounded, so the columns and rows
    // moved a fraction of a point on reload.
    let xml = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{}</w:body></w:document>"#,
        nested_xml(0)
    );
    let mut doc = executor::block_on(WordDocument::load(
        &build_document_docx(&xml),
        &Options::default(),
    ))
    .unwrap();
    let mut commands = CommandExecutor::new();
    let mut selection = None;
    for command in [
        Box::new(ResizeTableColumnCommand::rendered_columns(
            2,
            vec![61.237, 47.418, 5.0],
        )) as Box<dyn Command>,
        Box::new(ResizeTableRowCommand::between_rows(2, 0, 20.013, 31.37)),
    ] {
        commands
            .execute(command, &mut CommandContext::new(&mut doc, &mut selection))
            .unwrap();
    }
    let dimensions = |doc: &WordDocument| {
        let table = doc.table_by_order(2).unwrap();
        (
            table.columns.iter().map(|c| c.width).collect::<Vec<_>>(),
            table.rows.iter().map(|r| r.height).collect::<Vec<_>>(),
        )
    };
    let reopened = executor::block_on(WordDocument::load(
        &doc.export_docx().unwrap(),
        &Options::default(),
    ))
    .unwrap();
    assert_eq!(dimensions(&reopened), dimensions(&doc));
}
