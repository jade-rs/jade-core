//! References follow the rows and columns they point at.

use jade_xlsx_reader::{shift_formula, Axis, CellRange, CellRef, StructureChange};
use proptest::prelude::*;

fn insert_rows(at: u32, count: u32) -> StructureChange {
    StructureChange {
        sheet: "Sheet1".into(),
        axis: Axis::Rows,
        at,
        count,
        inserted: true,
    }
}

fn delete_rows(at: u32, count: u32) -> StructureChange {
    StructureChange {
        inserted: false,
        ..insert_rows(at, count)
    }
}

fn insert_columns(at: u32, count: u32) -> StructureChange {
    StructureChange {
        axis: Axis::Columns,
        ..insert_rows(at, count)
    }
}

fn delete_columns(at: u32, count: u32) -> StructureChange {
    StructureChange {
        inserted: false,
        ..insert_columns(at, count)
    }
}

fn shift(formula: &str, change: &StructureChange) -> String {
    shift_formula(formula, "Sheet1", change)
}

#[test]
fn inserting_rows_moves_references_below_the_insertion_point() {
    // Two rows before row 3 (0-based 2).
    let change = insert_rows(2, 2);
    assert_eq!(shift("A1+A2", &change), "A1+A2");
    assert_eq!(shift("A3+$A$4", &change), "A5+$A$6");
    assert_eq!(shift("SUM(A1:A5)", &change), "SUM(A1:A7)");
    assert_eq!(shift("SUM(A3:A5)", &change), "SUM(A5:A7)");
    // A range ending just above the insertion does not grow.
    assert_eq!(shift("SUM(A1:A2)", &change), "SUM(A1:A2)");
    // Whole rows move; whole columns do not.
    assert_eq!(shift("SUM(3:4)", &change), "SUM(5:6)");
    assert_eq!(shift("SUM(A:A)", &change), "SUM(A:A)");
}

#[test]
fn deleting_rows_shrinks_ranges_and_breaks_references_into_them() {
    // Rows 3 and 4 (0-based 2..4).
    let change = delete_rows(2, 2);
    assert_eq!(shift("A1+A2", &change), "A1+A2");
    assert_eq!(shift("A3", &change), "#REF!");
    assert_eq!(shift("$A$4", &change), "#REF!");
    assert_eq!(shift("A5+A6", &change), "A3+A4");
    assert_eq!(shift("SUM(A1:A6)", &change), "SUM(A1:A4)");
    // Overlapping one end.
    assert_eq!(shift("SUM(A1:A3)", &change), "SUM(A1:A2)");
    assert_eq!(shift("SUM(A4:A6)", &change), "SUM(A3:A4)");
    // Entirely inside.
    assert_eq!(shift("SUM(A3:A4)", &change), "SUM(#REF!)");
    assert_eq!(shift("SUM(3:4)", &change), "SUM(#REF!)");
    assert_eq!(shift("SUM(2:5)", &change), "SUM(2:3)");
}

#[test]
fn columns_move_the_same_way() {
    let insert = insert_columns(1, 1);
    assert_eq!(shift("A1+B1+C1", &insert), "A1+C1+D1");
    assert_eq!(shift("SUM(A1:C1)", &insert), "SUM(A1:D1)");
    assert_eq!(shift("SUM(B:C)", &insert), "SUM(C:D)");
    let delete = delete_columns(1, 1);
    assert_eq!(shift("A1+B1+C1", &delete), "A1+#REF!+B1");
    assert_eq!(shift("SUM(A1:C1)", &delete), "SUM(A1:B1)");
    assert_eq!(shift("SUM($B$1:$B$9)", &delete), "SUM(#REF!)");
}

#[test]
fn only_references_into_the_changed_sheet_move() {
    let change = insert_rows(0, 1);
    // Unqualified references belong to the formula's own sheet.
    assert_eq!(
        shift_formula("A1+Sheet2!A1", "Sheet1", &change),
        "A2+Sheet2!A1"
    );
    assert_eq!(
        shift_formula("A1+Sheet1!A1", "Sheet2", &change),
        "A1+Sheet1!A2"
    );
    assert_eq!(
        shift_formula(
            "'My Sheet'!A1+A1",
            "Sheet2",
            &insert_rows_on("My Sheet", 0, 1)
        ),
        "'My Sheet'!A2+A1"
    );
    assert_eq!(
        shift_formula("'It''s'!B2:B3", "Sheet2", &insert_rows_on("It's", 0, 1)),
        "'It''s'!B3:B4"
    );
    // The qualification ends with the reference.
    assert_eq!(
        shift_formula("Sheet2!A1+A1", "Sheet1", &change),
        "Sheet2!A1+A2"
    );
    // Sheet names are matched the way Excel matches them.
    assert_eq!(shift_formula("SHEET1!A1", "Sheet2", &change), "SHEET1!A2");
}

fn insert_rows_on(sheet: &str, at: u32, count: u32) -> StructureChange {
    StructureChange {
        sheet: sheet.into(),
        ..insert_rows(at, count)
    }
}

#[test]
fn strings_names_and_functions_are_left_alone() {
    let change = insert_rows(0, 5);
    assert_eq!(shift("\"A1\"&A1", &change), "\"A1\"&A6");
    assert_eq!(shift("LOG10(A1)", &change), "LOG10(A6)");
    assert_eq!(shift("Total+A1", &change), "Total+A6");
    assert_eq!(shift("MyTable[A1]", &change), "MyTable[A1]");
    assert_eq!(shift("1.5E2+A1", &change), "1.5E2+A6");
    assert_eq!(shift("ROUND(2.5, 0)", &change), "ROUND(2.5, 0)");
    // Off the grid is broken, not wrapped.
    let bottom = insert_rows(0, 1);
    assert_eq!(shift("A1048576", &bottom), "#REF!");
}

#[test]
fn spans_and_cells_map_the_way_references_do() {
    let change = delete_rows(2, 2);
    assert_eq!(change.map_index(1), Some(1));
    assert_eq!(change.map_index(2), None);
    assert_eq!(change.map_index(4), Some(2));
    assert_eq!(change.map_span(0, 5), Some((0, 3)));
    assert_eq!(change.map_span(2, 3), None);
    assert_eq!(change.map_span(3, 5), Some((2, 3)));
    let range = CellRange {
        start: CellRef::new(0, 1),
        end: CellRef::new(2, 3),
    };
    assert_eq!(
        change.map_range(range),
        Some(CellRange {
            start: CellRef::new(0, 1),
            end: CellRef::new(2, 1),
        })
    );
    assert_eq!(change.map_cell(CellRef::new(0, 3)), None);
}

fn change_strategy() -> impl Strategy<Value = StructureChange> {
    (
        prop_oneof![Just(Axis::Rows), Just(Axis::Columns)],
        0u32..20,
        1u32..5,
        any::<bool>(),
    )
        .prop_map(|(axis, at, count, inserted)| StructureChange {
            sheet: "Sheet1".into(),
            axis,
            at,
            count,
            inserted,
        })
}

fn label(column: u32) -> String {
    let mut label = String::new();
    let mut value = column + 1;
    while value > 0 {
        let remainder = (value - 1) % 26;
        label.insert(0, (b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    label
}

proptest! {
    /// A single cell reference moves exactly where `map_cell` says, `$` or
    /// not, and nothing else in the formula changes.
    #[test]
    fn a_cell_reference_moves_where_the_change_maps_it(
        change in change_strategy(),
        column in 0u32..30,
        row in 0u32..30,
        absolute in any::<bool>(),
    ) {
        let dollar = if absolute { "$" } else { "" };
        let written = format!("{dollar}{}{dollar}{}", label(column), row + 1);
        let formula = format!("SUM({written})*2");
        let expected = match change.map_cell(CellRef::new(column, row)) {
            Some(cell) => format!("SUM({dollar}{}{dollar}{})*2", label(cell.column), cell.row + 1),
            None => "SUM(#REF!)*2".to_string(),
        };
        prop_assert_eq!(shift(&formula, &change), expected);
    }

    /// A range moves where `map_range` says.
    #[test]
    fn a_range_moves_where_the_change_maps_it(
        change in change_strategy(),
        (c1, r1, c2, r2) in (0u32..30, 0u32..30, 0u32..30, 0u32..30),
    ) {
        let start = CellRef::new(c1.min(c2), r1.min(r2));
        let end = CellRef::new(c1.max(c2), r1.max(r2));
        let formula = format!("{}{}:{}{}", label(start.column), start.row + 1, label(end.column), end.row + 1);
        let expected = match change.map_range(CellRange { start, end }) {
            Some(range) => format!(
                "{}{}:{}{}",
                label(range.start.column), range.start.row + 1,
                label(range.end.column), range.end.row + 1
            ),
            None => "#REF!".to_string(),
        };
        prop_assert_eq!(shift(&formula, &change), expected);
    }

    /// Inserting and then deleting the same rows or columns puts every
    /// reference back where it was.
    #[test]
    fn inserting_then_deleting_is_the_identity(
        axis in prop_oneof![Just(Axis::Rows), Just(Axis::Columns)],
        at in 0u32..20,
        count in 1u32..5,
        column in 0u32..30,
        row in 0u32..30,
        (c2, r2) in (0u32..30, 0u32..30),
    ) {
        let insert = StructureChange { sheet: "Sheet1".into(), axis, at, count, inserted: true };
        let delete = StructureChange { inserted: false, ..insert.clone() };
        let formula = format!(
            "{}{}+SUM({}{}:{}{})",
            label(column), row + 1,
            label(column.min(c2)), row.min(r2) + 1,
            label(column.max(c2)), row.max(r2) + 1
        );
        let inserted = shift(&formula, &insert);
        prop_assert_eq!(shift(&inserted, &delete), formula);
    }
}
