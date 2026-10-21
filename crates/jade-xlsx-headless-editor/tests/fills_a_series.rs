//! What the fill handle writes: a run continued, not just copied.

use jade_xlsx_headless_api::{CellInput, CellKind};
use jade_xlsx_headless_editor::fill::{fill_axis, plan_fill, FillAxis, FillWrite};
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{
    Cell, CellRange, CellRef, CellValue, Row, Sheet, SheetVisibility, StyleTable,
};
use proptest::prelude::*;
use std::path::PathBuf;

const SHEET: &str = "売上サマリ";

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(&fixture("sample-workbook.xlsx")).expect("open")
}

fn set(workbook: &mut HeadlessWorkbook, values: &[(&str, CellInput)]) {
    let values = values
        .iter()
        .map(|(reference, value)| (reference.to_string(), value.clone()))
        .collect::<Vec<_>>();
    workbook.set_cells(SHEET, &values).expect("set");
}

fn number(workbook: &HeadlessWorkbook, reference: &str) -> f64 {
    let view = workbook.cell(SHEET, reference).expect("cell");
    assert_eq!(view.kind, CellKind::Number, "{reference}: {view:?}");
    view.number.expect("number")
}

fn text(workbook: &HeadlessWorkbook, reference: &str) -> String {
    let view = workbook.cell(SHEET, reference).expect("cell");
    assert_eq!(view.kind, CellKind::Text, "{reference}: {view:?}");
    view.text.expect("text")
}

// ── through the workbook ────────────────────────────────────────────────

#[test]
fn two_numbers_continue_by_their_difference() {
    let mut workbook = open();
    set(
        &mut workbook,
        &[
            ("F1", CellInput::Number(10.0)),
            ("F2", CellInput::Number(20.0)),
        ],
    );

    let summary = workbook.fill(SHEET, "F1:F2", "F1:F4").expect("fill");

    assert_eq!(summary.source, "F1:F2");
    assert_eq!(summary.target, "F1:F4");
    assert_eq!(summary.cells, vec!["F3", "F4"]);
    assert_eq!(number(&workbook, "F3"), 30.0);
    assert_eq!(number(&workbook, "F4"), 40.0);
    // The fill is one write, and so one undo step.
    assert!(workbook.undo().expect("undo"));
    assert_eq!(
        workbook.cell(SHEET, "F3").expect("cell").kind,
        CellKind::Empty
    );
}

#[test]
fn a_fill_upward_runs_the_series_backwards() {
    let mut workbook = open();
    set(
        &mut workbook,
        &[
            ("F5", CellInput::Number(30.0)),
            ("F6", CellInput::Number(40.0)),
        ],
    );

    let summary = workbook.fill(SHEET, "F5:F6", "F3:F6").expect("fill");

    assert_eq!(summary.cells, vec!["F3", "F4"]);
    assert_eq!(number(&workbook, "F3"), 10.0);
    assert_eq!(number(&workbook, "F4"), 20.0);
}

#[test]
fn more_than_two_numbers_follow_their_linear_trend() {
    // 1, 2, 4 is not arithmetic; Excel extends its least-squares line —
    // 0.8333 + 1.5 × i — rather than repeating the last step.
    let mut workbook = open();
    set(
        &mut workbook,
        &[
            ("F1", CellInput::Number(1.0)),
            ("F2", CellInput::Number(2.0)),
            ("F3", CellInput::Number(4.0)),
        ],
    );

    workbook.fill(SHEET, "F1:F3", "F1:F5").expect("fill");

    assert!((number(&workbook, "F4") - 5.333_333_333_333_33).abs() < 1e-12);
    assert!((number(&workbook, "F5") - 6.833_333_333_333_33).abs() < 1e-12);
}

#[test]
fn a_lone_number_copies_but_a_lone_date_steps_a_day() {
    let mut workbook = open();
    set(&mut workbook, &[("F1", CellInput::Number(7.0))]);
    workbook.fill(SHEET, "F1:F1", "F1:F3").expect("fill");
    assert_eq!(number(&workbook, "F2"), 7.0);
    assert_eq!(number(&workbook, "F3"), 7.0);

    // B9 shows 2023/03/15 through a `yyyy/mm/dd` format: the next cells are
    // the days after it.
    assert_eq!(
        workbook.cell(SHEET, "B9").expect("cell").display,
        "2023/03/15"
    );
    workbook.fill(SHEET, "B9:B9", "B9:B11").expect("fill");
    assert_eq!(number(&workbook, "B10"), 45_001.0);
    assert_eq!(number(&workbook, "B11"), 45_002.0);
    // The date format travels with the fill, so they read as dates too.
    assert_eq!(
        workbook.cell(SHEET, "B10").expect("cell").display,
        "2023/03/16"
    );
    assert_eq!(
        workbook.cell(SHEET, "B11").expect("cell").display,
        "2023/03/17"
    );
}

#[test]
fn text_ending_in_a_number_counts_on() {
    let mut workbook = open();
    set(
        &mut workbook,
        &[
            ("F1", CellInput::Text("Item 1".into())),
            ("G1", CellInput::Text("No.007".into())),
            ("H1", CellInput::Text("第3回".into())),
        ],
    );

    workbook.fill(SHEET, "F1:H1", "F1:H3").expect("fill");

    assert_eq!(text(&workbook, "F2"), "Item 2");
    assert_eq!(text(&workbook, "F3"), "Item 3");
    // Zero padding is kept.
    assert_eq!(text(&workbook, "G2"), "No.008");
    assert_eq!(text(&workbook, "H3"), "第5回");
}

#[test]
fn text_runs_with_different_words_repeat_instead() {
    let mut workbook = open();
    set(
        &mut workbook,
        &[
            ("F1", CellInput::Text("a1".into())),
            ("F2", CellInput::Text("b2".into())),
        ],
    );

    workbook.fill(SHEET, "F1:F2", "F1:F5").expect("fill");

    assert_eq!(text(&workbook, "F3"), "a1");
    assert_eq!(text(&workbook, "F4"), "b2");
    assert_eq!(text(&workbook, "F5"), "a1");
}

#[test]
fn formulas_repeat_with_their_references_moved() {
    let mut workbook = open();
    set(&mut workbook, &[("F4", CellInput::Formula("B4*2".into()))]);

    workbook.fill(SHEET, "F4:F4", "F4:F6").expect("fill");

    let f5 = workbook.cell(SHEET, "F5").expect("cell");
    assert_eq!(f5.formula.as_deref(), Some("B5*2"));
    assert_eq!(f5.display, "18840000");
    assert_eq!(
        workbook.cell(SHEET, "F6").expect("cell").formula.as_deref(),
        Some("B6*2")
    );
}

#[test]
fn a_target_that_is_not_the_source_grown_along_one_axis_is_refused() {
    let mut workbook = open();
    for (source, target) in [
        ("F1:F2", "G3:G4"),
        ("F1:F2", "F1:G4"),
        ("F1:F3", "F1:F2"),
        ("F1:F2", "F1:F2:F3"),
    ] {
        let error = workbook.fill(SHEET, source, target).expect_err(target);
        assert!(error.is_invalid_input(), "{source} → {target}: {error}");
    }
    // A target equal to the source has nothing to fill, and says so.
    let summary = workbook.fill(SHEET, "F1:F2", "F1:F2").expect("fill");
    assert!(summary.cells.is_empty());
    assert!(!workbook.can_undo());
}

// ── the plan itself ─────────────────────────────────────────────────────

fn sheet_with(cells: Vec<(CellRef, CellValue, Option<&str>)>) -> Sheet {
    let mut rows: Vec<Row> = Vec::new();
    for (reference, value, formula) in cells {
        let row = match rows.iter_mut().find(|row| row.index == reference.row) {
            Some(row) => row,
            None => {
                rows.push(Row {
                    index: reference.row,
                    height_points: None,
                    custom_height: false,
                    custom_format: false,
                    hidden: false,
                    style_index: None,
                    cells: Vec::new(),
                });
                rows.last_mut().expect("just pushed")
            }
        };
        row.cells.push(Cell {
            reference,
            value,
            style_index: None,
            formula: formula.map(str::to_string),
        });
    }
    rows.sort_by_key(|row| row.index);
    Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: SheetVisibility::Visible,
        columns: Vec::new(),
        rows,
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: None,
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: true,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        tab_color: None,
        sparkline_groups: Vec::new(),
        protection: None,
    }
}

fn range(start: CellRef, end: CellRef) -> CellRange {
    CellRange { start, end }
}

/// The values a plan writes, without the styles (none of these sheets has
/// any, so every write carries the default style 0).
fn values(writes: &[FillWrite]) -> Vec<(CellRef, CellInput)> {
    for write in writes {
        assert_eq!(write.style_index, Some(0), "{write:?}");
    }
    writes
        .iter()
        .map(|write| (write.at, write.value.clone()))
        .collect()
}

#[test]
fn the_axis_is_the_one_the_target_grows_along() {
    let source = range(CellRef::new(1, 1), CellRef::new(2, 2));
    assert_eq!(
        fill_axis(source, range(CellRef::new(1, 1), CellRef::new(2, 5))),
        Some(FillAxis::Rows)
    );
    assert_eq!(
        fill_axis(source, range(CellRef::new(1, 0), CellRef::new(2, 2))),
        Some(FillAxis::Rows)
    );
    assert_eq!(
        fill_axis(source, range(CellRef::new(0, 1), CellRef::new(4, 2))),
        Some(FillAxis::Columns)
    );
    // Grown on both, shrunk, or moved: not a fill.
    assert_eq!(
        fill_axis(source, range(CellRef::new(1, 1), CellRef::new(3, 3))),
        None
    );
    assert_eq!(
        fill_axis(source, range(CellRef::new(1, 1), CellRef::new(2, 1))),
        None
    );
    assert_eq!(
        fill_axis(source, range(CellRef::new(2, 2), CellRef::new(3, 5))),
        None
    );
}

#[test]
fn each_column_of_a_downward_fill_is_its_own_run() {
    // A: 1, 2 (a series). B: TRUE, FALSE (a pattern). C: "x1", "x1" (a
    // series with no step).
    let sheet = sheet_with(vec![
        (CellRef::new(0, 0), CellValue::Number(1.0), None),
        (CellRef::new(0, 1), CellValue::Number(2.0), None),
        (CellRef::new(1, 0), CellValue::Bool(true), None),
        (CellRef::new(1, 1), CellValue::Bool(false), None),
        (CellRef::new(2, 0), CellValue::Text("x1".into()), None),
        (CellRef::new(2, 1), CellValue::Text("x1".into()), None),
    ]);
    let writes = plan_fill(
        &sheet,
        &StyleTable::default(),
        range(CellRef::new(0, 0), CellRef::new(2, 1)),
        range(CellRef::new(0, 0), CellRef::new(2, 3)),
    );
    assert_eq!(
        values(&writes),
        vec![
            (CellRef::new(0, 2), CellInput::Number(3.0)),
            (CellRef::new(1, 2), CellInput::Bool(true)),
            (CellRef::new(2, 2), CellInput::Text("x1".into())),
            (CellRef::new(0, 3), CellInput::Number(4.0)),
            (CellRef::new(1, 3), CellInput::Bool(false)),
            (CellRef::new(2, 3), CellInput::Text("x1".into())),
        ]
    );
}

#[test]
fn a_run_with_a_gap_repeats_rather_than_guessing() {
    let sheet = sheet_with(vec![(CellRef::new(0, 0), CellValue::Number(1.0), None)]);
    // A1:A2 with A2 empty: the pattern is "1, blank".
    let writes = plan_fill(
        &sheet,
        &StyleTable::default(),
        range(CellRef::new(0, 0), CellRef::new(0, 1)),
        range(CellRef::new(0, 0), CellRef::new(0, 4)),
    );
    assert_eq!(
        values(&writes),
        vec![
            (CellRef::new(0, 2), CellInput::Number(1.0)),
            (CellRef::new(0, 3), CellInput::Blank),
            (CellRef::new(0, 4), CellInput::Number(1.0)),
        ]
    );
}

fn number_at(writes: &[FillWrite], reference: CellRef) -> f64 {
    match writes.iter().find(|write| write.at == reference) {
        Some(FillWrite {
            value: CellInput::Number(number),
            ..
        }) => *number,
        other => panic!("{reference:?}: {other:?}"),
    }
}

proptest! {
    /// An arithmetic progression keeps its step, forwards and backwards, and
    /// integer inputs stay exact — no float noise from the trend fit.
    #[test]
    fn an_arithmetic_progression_keeps_its_step(
        first in -1_000i64..1_000,
        step in -500i64..500,
        count in 2u32..7,
        below in 0u32..6,
        above in 1u32..8,
    ) {
        let cells = (0..count)
            .map(|index| {
                let value = first + step * i64::from(index);
                (CellRef::new(3, below + index), CellValue::Number(value as f64), None)
            })
            .collect();
        let sheet = sheet_with(cells);
        let source = range(CellRef::new(3, below), CellRef::new(3, below + count - 1));
        let target = range(CellRef::new(3, 0), CellRef::new(3, below + count - 1 + above));
        let writes = plan_fill(&sheet, &StyleTable::default(), source, target);

        prop_assert_eq!(writes.len() as u32, below + above);
        for write in &writes {
            let position = i64::from(write.at.row) - i64::from(below);
            let expected = (first + step * position) as f64;
            prop_assert_eq!(number_at(&writes, write.at), expected);
        }
    }

    /// Text that ends in a number counts along the same progression, with
    /// the words around the number untouched.
    #[test]
    fn counting_text_keeps_its_words(
        prefix in "[a-zA-Z ]{0,6}",
        suffix in "[a-zA-Z ]{0,4}",
        first in 0i64..1_000,
        step in 1i64..50,
        count in 1u32..5,
        above in 1u32..6,
    ) {
        let cells = (0..count)
            .map(|index| {
                let value = first + step * i64::from(index);
                (
                    CellRef::new(index, 0),
                    CellValue::Text(format!("{prefix}{value}{suffix}")),
                    None,
                )
            })
            .collect();
        let sheet = sheet_with(cells);
        let source = range(CellRef::new(0, 0), CellRef::new(count - 1, 0));
        let target = range(CellRef::new(0, 0), CellRef::new(count - 1 + above, 0));
        let writes = plan_fill(&sheet, &StyleTable::default(), source, target);

        // One cell continues by one; a run continues by its step.
        let step = if count == 1 { 1 } else { step };
        prop_assert_eq!(writes.len() as u32, above);
        for write in &writes {
            let value = first + step * i64::from(write.at.column);
            prop_assert_eq!(&write.value, &CellInput::Text(format!("{prefix}{value}{suffix}")));
        }
    }

    /// Anything that is not a series repeats with the source's period.
    #[test]
    fn a_pattern_repeats_with_the_source_period(
        flags in prop::collection::vec(any::<bool>(), 1..6),
        above in 1u32..12,
    ) {
        let count = flags.len() as u32;
        let cells = flags
            .iter()
            .enumerate()
            .map(|(index, flag)| (CellRef::new(0, index as u32), CellValue::Bool(*flag), None))
            .collect();
        let sheet = sheet_with(cells);
        let source = range(CellRef::new(0, 0), CellRef::new(0, count - 1));
        let target = range(CellRef::new(0, 0), CellRef::new(0, count - 1 + above));
        let writes = plan_fill(&sheet, &StyleTable::default(), source, target);

        prop_assert_eq!(writes.len() as u32, above);
        for write in &writes {
            let expected = flags[(write.at.row % count) as usize];
            prop_assert_eq!(&write.value, &CellInput::Bool(expected));
        }
    }
}

#[test]
fn a_fill_carries_the_source_formats_with_the_source_period() {
    let mut workbook = open();
    // F1:F2 have no style; F3 wears B4's yen format. Filling F1:F2 down
    // clears F3 — the source had no format — as Excel does.
    set(
        &mut workbook,
        &[
            ("F1", CellInput::Number(1.0)),
            ("F2", CellInput::Number(2.0)),
        ],
    );
    workbook
        .copy_format(SHEET, "B4:B4", "F3:F4")
        .expect("format");
    workbook.fill(SHEET, "F1:F2", "F1:F4").expect("fill");
    assert_eq!(workbook.cell(SHEET, "F3").expect("cell").display, "3");
    assert_eq!(workbook.cell(SHEET, "F4").expect("cell").display, "4");

    // G1 in yen over plain cells: the yen comes along.
    set(&mut workbook, &[("G1", CellInput::Number(100.0))]);
    workbook
        .copy_format(SHEET, "B4:B4", "G1:G1")
        .expect("format");
    workbook.fill(SHEET, "G1:G1", "G1:G3").expect("fill");
    assert_eq!(workbook.cell(SHEET, "G2").expect("cell").display, "¥100");
    assert_eq!(workbook.cell(SHEET, "G3").expect("cell").display, "¥100");

    // A two-cell source alternates its formats: yen, plain, yen, plain.
    set(
        &mut workbook,
        &[
            ("H1", CellInput::Number(1.0)),
            ("H2", CellInput::Number(2.0)),
        ],
    );
    workbook
        .copy_format(SHEET, "B4:B4", "H1:H1")
        .expect("format");
    workbook.fill(SHEET, "H1:H2", "H1:H5").expect("fill");
    assert_eq!(workbook.cell(SHEET, "H3").expect("cell").display, "¥3");
    assert_eq!(workbook.cell(SHEET, "H4").expect("cell").display, "4");
    assert_eq!(workbook.cell(SHEET, "H5").expect("cell").display, "¥5");
}

#[test]
fn the_format_painter_stamps_the_source_shape_from_a_single_cell() {
    let mut workbook = open();
    set(
        &mut workbook,
        &[
            ("F1", CellInput::Number(1.0)),
            ("F2", CellInput::Number(2.0)),
            ("F3", CellInput::Number(3.0)),
            ("F4", CellInput::Number(4.0)),
        ],
    );

    // B4:B5 are currency; F1 alone as the target means F1:F2.
    let summary = workbook.copy_format(SHEET, "B4:B5", "F1").expect("format");
    assert_eq!(summary.target, "F1:F2");
    assert_eq!(summary.cells, vec!["F1", "F2"]);
    assert_eq!(workbook.cell(SHEET, "F1").expect("cell").display, "¥1");
    assert_eq!(workbook.cell(SHEET, "F2").expect("cell").display, "¥2");
    assert_eq!(workbook.cell(SHEET, "F3").expect("cell").display, "3");

    // A larger target tiles the source; values stay.
    let summary = workbook
        .copy_format(SHEET, "B4:B4", "F3:F4")
        .expect("format");
    assert_eq!(summary.cells, vec!["F3", "F4"]);
    assert_eq!(workbook.cell(SHEET, "F4").expect("cell").display, "¥4");
    assert_eq!(workbook.cell(SHEET, "F4").expect("cell").number, Some(4.0));

    // One undo step takes the whole stamp back.
    assert!(workbook.undo().expect("undo"));
    assert_eq!(workbook.cell(SHEET, "F4").expect("cell").display, "4");
}
