//! Merging and unmerging cells from a caller with no window.
//!
//! Excel's Merge Cells keeps what the top-left cell holds and discards the
//! value or formula of every other cell in the range ("Merging cells only
//! keeps the upper-left value and discards other values"), and the merged
//! cell wears the top-left cell's format — which is why, after Unmerge,
//! every former member still wears it. Plain Merge Cells does not centre:
//! that is Merge & Center, a separate command.

use jade_xlsx_headless_api::{CellInput, CellKind};
use jade_xlsx_headless_editor::{effective_style, HeadlessError, HeadlessWorkbook};
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef};
use proptest::prelude::*;
use std::path::PathBuf;

const SHEET: &str = "売上サマリ";

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(&fixture("sample-workbook.xlsx")).expect("open")
}

fn merges(workbook: &HeadlessWorkbook, sheet: &str) -> Vec<String> {
    let summary = workbook.summary();
    let mut merges = summary
        .sheets
        .iter()
        .find(|candidate| candidate.name == sheet)
        .expect("sheet")
        .merges
        .clone();
    merges.sort();
    merges
}

fn reopened(workbook: &HeadlessWorkbook) -> HeadlessWorkbook {
    HeadlessWorkbook::open(&workbook.export_bytes().expect("export")).expect("reopen")
}

fn style(workbook: &HeadlessWorkbook, reference: &str) -> u32 {
    let sheet = &workbook.workbook().sheets[workbook.sheet_index(SHEET).unwrap()];
    effective_style(sheet, CellRef::parse(reference).unwrap())
}

#[test]
fn merging_keeps_the_top_left_value_and_clears_the_rest() {
    let mut workbook = open();
    let anchor_style = style(&workbook, "A3");
    let summary = workbook.merge_cells(SHEET, "A3:B4").expect("merge");
    assert_eq!(summary.sheet, SHEET);
    assert_eq!(summary.range, "A3:B4");
    assert_eq!(summary.cleared, ["B3", "A4", "B4"]);

    let anchor = workbook.cell(SHEET, "A3").expect("cell");
    assert_eq!(anchor.kind, CellKind::Text);
    for reference in ["B3", "A4", "B4"] {
        let cell = workbook.cell(SHEET, reference).expect("cell");
        assert_eq!(cell.kind, CellKind::Empty, "{reference}");
        assert_eq!(cell.formula, None, "{reference}");
        // Every member wears the merged cell's format.
        assert_eq!(style(&workbook, reference), anchor_style, "{reference}");
    }
    assert_eq!(merges(&workbook, SHEET), ["A1:D1", "A3:B4"]);
    // What pointed at the cleared cells recalculates.
    assert_eq!(workbook.cell(SHEET, "D4").unwrap().display, "¥15,230,000");
    assert_eq!(workbook.cell(SHEET, "B7").unwrap().display, "¥15,550,000");

    // Through export and back.
    let again = reopened(&workbook);
    assert_eq!(merges(&again, SHEET), ["A1:D1", "A3:B4"]);
    assert_eq!(again.cell(SHEET, "B4").unwrap().kind, CellKind::Empty);
}

#[test]
fn merging_is_one_undo_step() {
    let mut workbook = open();
    let before = workbook.bytes().to_vec();
    workbook.merge_cells(SHEET, "B4:C5").expect("merge");
    assert!(workbook.undo().expect("undo"));
    assert!(!workbook.can_undo());
    assert_eq!(workbook.bytes(), before.as_slice());
    assert_eq!(merges(&workbook, SHEET), ["A1:D1"]);
    assert_eq!(workbook.cell(SHEET, "C5").unwrap().display, "¥8,990,000");

    assert!(workbook.redo().expect("redo"));
    assert_eq!(merges(&workbook, SHEET), ["A1:D1", "B4:C5"]);
    assert_eq!(workbook.cell(SHEET, "C5").unwrap().kind, CellKind::Empty);
}

#[test]
fn a_range_written_back_to_front_is_the_same_range() {
    let mut workbook = open();
    let summary = workbook.merge_cells(SHEET, "C5:B4").expect("merge");
    assert_eq!(summary.range, "B4:C5");
    assert_eq!(workbook.cell(SHEET, "B4").unwrap().display, "¥12,850,000");
    assert_eq!(merges(&workbook, SHEET), ["A1:D1", "B4:C5"]);
}

#[test]
fn a_formula_in_the_top_left_survives_and_the_others_go() {
    let mut workbook = open();
    let summary = workbook.merge_cells(SHEET, "D4:D5").expect("merge");
    assert_eq!(summary.cleared, ["D5"]);
    assert_eq!(
        workbook.cell(SHEET, "D4").unwrap().formula.as_deref(),
        Some("SUM(B4:C4)")
    );
    assert_eq!(workbook.cell(SHEET, "D5").unwrap().formula, None);
    // D7 sums D4:D6 and lost D5's contribution.
    assert_eq!(workbook.cell(SHEET, "D7").unwrap().display, "¥41,615,000");
}

#[test]
fn merging_empty_cells_clears_nothing() {
    let mut workbook = open();
    let summary = workbook.merge_cells(SHEET, "F10:H12").expect("merge");
    assert!(summary.cleared.is_empty());
    assert_eq!(merges(&workbook, SHEET), ["A1:D1", "F10:H12"]);
}

#[test]
fn overlaps_single_cells_and_tables_are_refused_without_writing() {
    let mut workbook = open();
    let before = workbook.bytes().to_vec();
    for (range, check) in [
        (
            "C1:E2",
            (|error: &HeadlessError| matches!(error, HeadlessError::MergeOverlaps { existing, .. } if existing == "A1:D1"))
                as fn(&HeadlessError) -> bool,
        ),
        ("A1:D1", |error| {
            matches!(error, HeadlessError::MergeOverlaps { .. })
        }),
        ("B4", |error| {
            matches!(error, HeadlessError::MergeTooSmall(_))
        }),
        ("B4:B4", |error| {
            matches!(error, HeadlessError::MergeTooSmall(_))
        }),
        ("B4:", |error| {
            matches!(error, HeadlessError::BadReference(_))
        }),
    ] {
        let error = workbook.merge_cells(SHEET, range).expect_err(range);
        assert!(check(&error), "{range}: {error}");
        assert!(error.is_invalid_input(), "{range}: {error}");
    }
    assert!(workbook
        .merge_cells("Nope", "A1:B2")
        .expect_err("no sheet")
        .is_invalid_input());
    assert!(!workbook.can_undo());
    assert_eq!(workbook.bytes(), before.as_slice());

    // A table's cells cannot be merged, inside it or across its edge.
    let mut tables = HeadlessWorkbook::open(&fixture("grid-operations-workbook.xlsx")).unwrap();
    let before = tables.bytes().to_vec();
    for range in ["B2:C3", "C3:D5", "A2:D10"] {
        let error = tables.merge_cells("Operations", range).expect_err(range);
        assert!(
            matches!(&error, HeadlessError::MergeInTable { table, .. } if table == "OperationsTable"),
            "{range}: {error}"
        );
        assert!(error.is_invalid_input());
    }
    assert_eq!(tables.bytes(), before.as_slice());
    // Beside the table is fine.
    tables.merge_cells("Operations", "D2:F3").expect("merge");
}

#[test]
fn unmerging_removes_every_merge_the_range_touches() {
    let mut workbook = open();
    workbook.merge_cells(SHEET, "B9:C10").expect("merge");
    // Values stay where they are: the top-left kept its own, the others
    // were cleared by the merge.
    let removed = workbook.unmerge_cells(SHEET, "B1:B10").expect("unmerge");
    assert_eq!(removed, ["A1:D1", "B9:C10"]);
    assert!(merges(&workbook, SHEET).is_empty());
    assert_eq!(workbook.cell(SHEET, "A1").unwrap().kind, CellKind::Text);
    let xml = sheet_xml(
        &workbook.export_bytes().unwrap(),
        "xl/worksheets/sheet1.xml",
    );
    assert!(!xml.contains("mergeCell"), "{xml}");
    assert!(reopened(&workbook).summary().sheets[0].merges.is_empty());

    // One step each.
    assert!(workbook.undo().unwrap());
    assert_eq!(merges(&workbook, SHEET), ["A1:D1", "B9:C10"]);
}

#[test]
fn unmerging_the_exact_range_leaves_the_others() {
    let mut workbook = open();
    workbook.merge_cells(SHEET, "B9:C10").expect("merge");
    assert_eq!(
        workbook.unmerge_cells(SHEET, "B9:C10").expect("unmerge"),
        ["B9:C10"]
    );
    assert_eq!(merges(&workbook, SHEET), ["A1:D1"]);
    assert_eq!(merges(&reopened(&workbook), SHEET), ["A1:D1"]);
}

#[test]
fn unmerging_where_nothing_is_merged_is_refused() {
    let mut workbook = open();
    let error = workbook.unmerge_cells(SHEET, "B4:C5").expect_err("nothing");
    assert!(matches!(error, HeadlessError::NoSuchMerge(_)), "{error}");
    assert!(error.is_invalid_input());
    assert!(!workbook.can_undo());
}

#[test]
fn rows_and_columns_inserted_and_deleted_carry_the_merge() {
    let mut workbook = open();
    workbook.merge_cells(SHEET, "B4:C5").expect("merge");

    // Inside: the merge grows.
    workbook.insert_rows(SHEET, 5, 2).expect("insert");
    assert_eq!(merges(&workbook, SHEET), ["A1:D1", "B4:C7"]);
    workbook.insert_columns(SHEET, "C", 1).expect("insert");
    assert_eq!(merges(&workbook, SHEET), ["A1:E1", "B4:D7"]);
    // Above and to the left: it moves.
    workbook.insert_rows(SHEET, 2, 1).expect("insert");
    workbook.insert_columns(SHEET, "A", 1).expect("insert");
    assert_eq!(merges(&workbook, SHEET), ["B1:F1", "C5:E8"]);
    // Deleting part of it shrinks it.
    workbook.delete_rows(SHEET, 6, 3).expect("delete");
    assert_eq!(merges(&workbook, SHEET), ["B1:F1", "C5:E5"]);
    workbook.delete_columns(SHEET, "D", 2).expect("delete");
    // One cell left of it: no merge any more.
    assert_eq!(merges(&workbook, SHEET), ["B1:D1"]);
    assert_eq!(merges(&reopened(&workbook), SHEET), ["B1:D1"]);
    // And the cell it was anchored on still holds its value.
    assert_eq!(workbook.cell(SHEET, "C5").unwrap().display, "¥12,850,000");
}

#[test]
fn a_merge_written_into_a_sheet_without_merges_lands_where_excel_expects_it() {
    let mut workbook = HeadlessWorkbook::open(&fixture("objects-workbook.xlsx")).unwrap();
    workbook.merge_cells("Data", "E1:F2").expect("merge");
    let xml = sheet_xml(workbook.bytes(), "xl/worksheets/sheet1.xml");
    let merge = xml.find("<mergeCells").expect("mergeCells");
    // After protectedRanges, before conditionalFormatting (CT_Worksheet order).
    assert!(xml.find("<protectedRanges").unwrap() < merge, "{xml}");
    assert!(merge < xml.find("<conditionalFormatting").unwrap(), "{xml}");
    assert!(xml.contains(r#"<mergeCells count="1"><mergeCell ref="E1:F2"/></mergeCells>"#));
}

#[test]
fn a_merged_cell_takes_new_values_at_its_anchor() {
    let mut workbook = open();
    workbook.merge_cells(SHEET, "F10:G11").expect("merge");
    workbook
        .set(SHEET, "F10", CellInput::Text("note".into()))
        .expect("set");
    assert_eq!(workbook.cell(SHEET, "F10").unwrap().display, "note");
    assert_eq!(merges(&reopened(&workbook), SHEET), ["A1:D1", "F10:G11"]);
}

fn sheet_xml(bytes: &[u8], part: &str) -> String {
    jade_opc::OpcPackage::open(bytes)
        .unwrap()
        .part_str(part)
        .unwrap()
        .unwrap()
}

fn overlaps(a: CellRange, b: CellRange) -> bool {
    a.start.column <= b.end.column
        && b.start.column <= a.end.column
        && a.start.row <= b.end.row
        && b.start.row <= a.end.row
}

fn parse(range: &str) -> CellRange {
    CellRange::parse(range).expect("range")
}

#[derive(Debug, Clone)]
enum Operation {
    Merge(u32, u32, u32, u32),
    Unmerge(u32, u32, u32, u32),
    Structure {
        rows: bool,
        at: u32,
        count: u32,
        inserted: bool,
    },
}

fn operation() -> impl Strategy<Value = Operation> {
    let rect = (0u32..8, 0u32..12, 0u32..3, 0u32..3);
    prop_oneof![
        3 => rect.clone().prop_map(|(c, r, w, h)| Operation::Merge(c, r, c + w, r + h)),
        1 => rect.prop_map(|(c, r, w, h)| Operation::Unmerge(c, r, c + w, r + h)),
        2 => (any::<bool>(), 0u32..14, 1u32..3, any::<bool>()).prop_map(
            |(rows, at, count, inserted)| Operation::Structure { rows, at, count, inserted }
        ),
    ]
}

fn label(column: u32) -> String {
    jade_xlsx_layout::column_label(column)
}

fn range_text(c1: u32, r1: u32, c2: u32, r2: u32) -> String {
    format!("{}{}:{}{}", label(c1), r1 + 1, label(c2), r2 + 1)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Whatever sequence of merges, unmerges and row/column changes runs,
    /// the merges stay disjoint and at least two cells each, a merge is
    /// refused exactly when it would overlap one, structural changes map
    /// each merge the way a formula range maps, and export → reopen reads
    /// back the same list.
    #[test]
    fn merges_stay_disjoint_and_follow_structure(operations in prop::collection::vec(operation(), 1..10)) {
        let mut workbook = open();
        let mut expected: Vec<CellRange> = vec![parse("A1:D1")];
        for operation in operations {
            match operation {
                Operation::Merge(c1, r1, c2, r2) => {
                    let wanted = CellRange { start: CellRef::new(c1, r1), end: CellRef::new(c2, r2) };
                    let result = workbook.merge_cells(SHEET, &range_text(c1, r1, c2, r2));
                    if wanted.start == wanted.end {
                        prop_assert!(matches!(result, Err(HeadlessError::MergeTooSmall(_))));
                    } else if expected.iter().any(|merge| overlaps(*merge, wanted)) {
                        prop_assert!(matches!(result, Err(HeadlessError::MergeOverlaps { .. })), "{result:?}");
                    } else {
                        let summary = result.expect("merge");
                        expected.push(wanted);
                        for cleared in summary.cleared {
                            prop_assert_eq!(workbook.cell(SHEET, &cleared).unwrap().kind, CellKind::Empty);
                        }
                    }
                }
                Operation::Unmerge(c1, r1, c2, r2) => {
                    let wanted = CellRange { start: CellRef::new(c1, r1), end: CellRef::new(c2, r2) };
                    let result = workbook.unmerge_cells(SHEET, &range_text(c1, r1, c2, r2));
                    if expected.iter().any(|merge| overlaps(*merge, wanted)) {
                        result.expect("unmerge");
                        expected.retain(|merge| !overlaps(*merge, wanted));
                    } else {
                        prop_assert!(matches!(result, Err(HeadlessError::NoSuchMerge(_))));
                    }
                }
                Operation::Structure { rows, at, count, inserted } => {
                    let change = jade_xlsx_reader::StructureChange {
                        sheet: SHEET.to_string(),
                        axis: if rows { jade_xlsx_reader::Axis::Rows } else { jade_xlsx_reader::Axis::Columns },
                        at,
                        count,
                        inserted,
                    };
                    let result = match (rows, inserted) {
                        (true, true) => workbook.insert_rows(SHEET, at + 1, count),
                        (true, false) => workbook.delete_rows(SHEET, at + 1, count),
                        (false, true) => workbook.insert_columns(SHEET, &label(at), count),
                        (false, false) => workbook.delete_columns(SHEET, &label(at), count),
                    };
                    result.expect("structure");
                    expected = expected
                        .iter()
                        .filter_map(|merge| change.map_range(*merge))
                        .filter(|merge| merge.start != merge.end)
                        .collect();
                }
            }
            let model = &workbook.workbook().sheets[0].merges;
            for (index, a) in model.iter().enumerate() {
                prop_assert!(a.start != a.end);
                for b in &model[index + 1..] {
                    prop_assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
                }
            }
            let mut wanted: Vec<String> = expected.iter().map(|range| range_text(range.start.column, range.start.row, range.end.column, range.end.row)).collect();
            wanted.sort();
            prop_assert_eq!(merges(&workbook, SHEET), wanted);
        }
        let reread = read_xlsx(&workbook.export_bytes().unwrap()).unwrap();
        prop_assert_eq!(&reread.sheets[0].merges, &workbook.workbook().sheets[0].merges);
    }
}
