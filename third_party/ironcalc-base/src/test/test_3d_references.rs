#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

/// Four sheets, Sheet1 to Sheet3 with 1, 2, 3 in A1 and 10, 20, 30 in B2,
/// and a "My Data" sheet at the end with 100 in A1.
fn three_sheets<'a>() -> crate::model::Model<'a> {
    let mut model = new_empty_model();
    model.new_sheet();
    model.new_sheet();
    model.add_sheet("My Data").unwrap();
    for (sheet, value) in [("Sheet1", 1), ("Sheet2", 2), ("Sheet3", 3)] {
        model._set(&format!("{sheet}!A1"), &value.to_string());
        model._set(&format!("{sheet}!B2"), &(value * 10).to_string());
    }
    model._set("My Data!A1", "100");
    model
}

#[test]
fn sum_and_friends_span_the_sheets() {
    let mut model = three_sheets();
    model._set("D1", "=SUM(Sheet1:Sheet3!A1)");
    model._set("D2", "=SUM(Sheet1:Sheet3!A1:B2)");
    model._set("D3", "=AVERAGE(Sheet1:Sheet3!A1)");
    model._set("D4", "=COUNT(Sheet1:Sheet3!A1:B2)");
    model._set("D5", "=MAX(Sheet2:Sheet3!B2)");
    model._set("D6", "=MIN(Sheet1:Sheet3!B2,5)");
    model._set("D7", "=PRODUCT(Sheet1:Sheet3!A1)");
    model._set("D8", "=COUNTA(Sheet1:Sheet3!A1:B2)");
    // The ends may be given either way round.
    model._set("D9", "=SUM(Sheet3:Sheet1!A1)");
    model._set("D10", "=SUM('Sheet2:My Data'!A1)");
    model._set("D11", "=STDEV.P(Sheet1:Sheet3!A1)");
    model.evaluate();
    // A1 on each sheet: 1 + 2 + 3; A1:B2 adds B2: 10 + 20 + 30.
    assert_eq!(model._get_text("D1"), *"6");
    assert_eq!(model._get_text("D2"), *"66");
    assert_eq!(model._get_text("D3"), *"2");
    assert_eq!(model._get_text("D4"), *"6");
    assert_eq!(model._get_text("D5"), *"30");
    assert_eq!(model._get_text("D6"), *"5");
    assert_eq!(model._get_text("D7"), *"6");
    assert_eq!(model._get_text("D8"), *"6");
    assert_eq!(model._get_text("D9"), *"6");
    assert_eq!(model._get_text("D10"), *"105");
    assert_eq!(model._get_text("D11"), *"0.816496581");
}

#[test]
fn a_change_on_a_middle_sheet_is_seen() {
    let mut model = three_sheets();
    model._set("D1", "=SUM(Sheet1:Sheet3!A1)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), *"6");
    model._set("Sheet2!A1", "40");
    model.evaluate();
    assert_eq!(model._get_text("D1"), *"44");
}

#[test]
fn sheet_sheets_and_isref_of_a_3d_reference() {
    let mut model = three_sheets();
    model._set("D1", "=SHEETS(Sheet1:Sheet3!A1)");
    model._set("D2", "=SHEET(Sheet2:Sheet3!A1)");
    model._set("D3", "=SHEETS('Sheet2:My Data'!A1:B4)");
    model._set("D4", "=ISREF(Sheet1:Sheet3!A1)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), *"3");
    assert_eq!(model._get_text("D2"), *"2");
    assert_eq!(model._get_text("D3"), *"3");
    assert_eq!(model._get_text("D4"), *"TRUE");
}

#[test]
fn other_uses_are_value_errors_and_missing_sheets_ref_errors() {
    let mut model = three_sheets();
    // As in Excel: a 3-D reference is only an argument of SUM and its kind.
    model._set("D1", "=Sheet1:Sheet3!A1");
    model._set("D2", "=VLOOKUP(1,Sheet1:Sheet3!A1:B2,2,FALSE)");
    model._set("D3", "=SUM(Sheet1:Nowhere!A1)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), *"#VALUE!");
    assert_eq!(model._get_text("D2"), *"#VALUE!");
    assert_eq!(model._get_text("D3"), *"#REF!");
}

#[test]
fn formulas_keep_their_3d_references() {
    let mut model = three_sheets();
    model._set("D1", "=SUM(Sheet1:Sheet3!A1:B2)");
    model._set("D2", "=SUM('Sheet2:My Data'!$A$1)");
    assert_eq!(model._get_formula("D1"), *"=SUM(Sheet1:Sheet3!A1:B2)");
    assert_eq!(model._get_formula("D2"), *"=SUM('Sheet2:My Data'!$A$1)");
    // Renaming an end renames it in the reference.
    model.rename_sheet("Sheet3", "March").unwrap();
    assert_eq!(model._get_formula("D1"), *"=SUM(Sheet1:March!A1:B2)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), *"66");
}

#[test]
fn a_sheet_named_like_a_column_range_still_reads_as_a_column_range() {
    let mut model = three_sheets();
    model._set("D1", "=SUM(A:B)");
    model.evaluate();
    // A1 and B2 on Sheet1.
    assert_eq!(model._get_text("D1"), *"11");
}

#[test]
fn deleting_an_endpoint_keeps_the_remaining_3d_range() {
    // Excel adjusts a deleted endpoint to the next sheet inside the range.
    // Keep the formula outside that range so deleting either end preserves it.
    for (deleted, remaining, expected) in [
        ("Sheet1", "Sheet2:Sheet3", "5"),
        ("Sheet3", "Sheet1:Sheet2", "3"),
    ] {
        let mut model = three_sheets();
        model._set("My Data!D1", "=SUM(Sheet1:Sheet3!$A$1)");
        model._set("My Data!D2", "=SUM(Sheet3:Sheet1!$A$1)");
        model._set("My Data!D3", "=IF(TRUE,SUM(Sheet1:Sheet3!$A$1),0)");
        model.delete_sheet_by_name(deleted).unwrap();
        model.evaluate();
        for cell in ["D1", "D2", "D3"] {
            assert_eq!(
                model._get_text(&format!("My Data!{cell}")),
                expected,
                "deleted {deleted}, cell {cell}"
            );
        }
        assert_eq!(
            model._get_formula("My Data!D1"),
            format!("=SUM({remaining}!$A$1)")
        );
    }
}

#[test]
fn deleting_the_last_sheet_of_a_3d_range_is_ref_error() {
    let mut model = three_sheets();
    model._set("My Data!D1", "=SUM(Sheet1:Sheet3!A1)");
    for deleted in ["Sheet1", "Sheet2"] {
        model.delete_sheet_by_name(deleted).unwrap();
        model.evaluate();
        assert_ne!(model._get_text("My Data!D1"), "#REF!");
    }
    model.delete_sheet_by_name("Sheet3").unwrap();
    model.evaluate();
    assert_eq!(model._get_text("My Data!D1"), "#REF!");
}

#[test]
fn every_sheet_deletion_order_matches_the_remaining_sheet_values() {
    // Exhaust the state transitions for this small workbook, including deleting
    // the middle sheet before either endpoint and shrinking to a single sheet.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut model = three_sheets();
        model
            .new_defined_name("ThreeSheetTotal", None, "SUM(Sheet1:Sheet3!$A$1)")
            .unwrap();
        model
            .new_defined_name("LocalTotal", Some(3), "SUM(Sheet3:Sheet1!$A$1)")
            .unwrap();
        for (cell, formula) in [
            ("D1", "=SUM(Sheet1:Sheet3!A1)"),
            ("D2", "=SUM(Sheet3:Sheet1!A1)"),
            ("D3", "=ThreeSheetTotal"),
            ("D4", "=LocalTotal"),
            ("D5", "=SUM(Sheet1:Sheet3!A1)+SUM(Sheet3:Sheet1!A1)"),
            ("D6", "=SUM(A1:B2)"),
        ] {
            model._set(&format!("My Data!{cell}"), formula);
        }
        let mut alive = [true; 3];
        for deleted in order {
            model
                .delete_sheet_by_name(&format!("Sheet{}", deleted + 1))
                .unwrap();
            alive[deleted] = false;
            model.evaluate();
            let expected: usize = alive
                .iter()
                .enumerate()
                .filter(|(_, live)| **live)
                .map(|(index, _)| index + 1)
                .sum();
            for (cell, multiplier) in [("D1", 1), ("D2", 1), ("D3", 1), ("D4", 1), ("D5", 2)] {
                let expected = if alive.iter().any(|live| *live) {
                    (expected * multiplier).to_string()
                } else {
                    "#REF!".to_string()
                };
                assert_eq!(
                    model._get_text(&format!("My Data!{cell}")),
                    expected,
                    "order {order:?}, deleted {deleted}, cell {cell}"
                );
            }
            assert_eq!(model._get_formula("My Data!D6"), "=SUM(A1:B2)");
            assert_eq!(model._get_text("My Data!D6"), "100");
        }
    }
}

#[test]
fn deleted_endpoints_preserve_quoted_sheet_names_and_relative_cells() {
    let mut model = three_sheets();
    model.rename_sheet("Sheet2", "February's data").unwrap();
    model._set("My Data!D7", "=SUM(Sheet1:Sheet3!A1:B2)");
    model._set("My Data!E8", "=SUM(Sheet1:Sheet3!B2)");
    model.delete_sheet_by_name("Sheet1").unwrap();
    model.evaluate();
    assert_eq!(
        model._get_formula("My Data!D7"),
        "=SUM('February''s data:Sheet3'!A1:B2)"
    );
    assert_eq!(
        model._get_formula("My Data!E8"),
        "=SUM('February''s data:Sheet3'!B2)"
    );
    assert_eq!(model._get_text("My Data!D7"), "55");
    assert_eq!(model._get_text("My Data!E8"), "50");
}
