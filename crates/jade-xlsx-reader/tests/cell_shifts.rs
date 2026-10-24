use jade_xlsx_reader::{shift_cells_formula, shift_formula, Axis, CellRange, CellRef, CellShift};
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn change(axis: Axis, inserted: bool) -> CellShift {
    CellShift {
        sheet: "Data".into(),
        range: range("B3:C4"),
        axis,
        inserted,
    }
}

#[test]
fn cells_move_only_in_the_selected_strip() {
    let c = change(Axis::Rows, true);
    for (before, after) in [
        ("A5", Some("A5")),
        ("B2", Some("B2")),
        ("B3", Some("B5")),
        ("C9", Some("C11")),
        ("D9", Some("D9")),
    ] {
        assert_eq!(
            c.map_cell(CellRef::parse(before).unwrap()),
            after.map(|s| CellRef::parse(s).unwrap())
        );
    }
    let c = change(Axis::Columns, false);
    for (before, after) in [
        ("B2", Some("B2")),
        ("B3", None),
        ("C4", None),
        ("D4", Some("B4")),
        ("D5", Some("D5")),
    ] {
        assert_eq!(
            c.map_cell(CellRef::parse(before).unwrap()),
            after.map(|s| CellRef::parse(s).unwrap())
        );
    }
}

#[test]
fn range_coverage_splits_but_formula_boundaries_require_the_whole_strip() {
    let c = change(Axis::Rows, true);
    assert_eq!(c.map_reference(range("A3:D5")), Some(range("A3:D5")));
    assert_eq!(c.map_reference(range("B2:C5")), Some(range("B2:C7")));
    assert_eq!(
        c.map_regions(range("A2:D5")),
        vec![range("A2:A5"), range("B2:C7"), range("D2:D5")]
    );
    assert_eq!(
        change(Axis::Rows, false).map_regions(range("A3:D4")),
        vec![range("A3:A4"), range("D3:D4")]
    );
    assert_eq!(
        c.map_regions(range("B1:C1048576")),
        vec![range("B1:C1048576")]
    );
}

#[test]
fn formulas_keep_absolute_flags_and_unaffected_crossing_ranges() {
    let c = change(Axis::Rows, true);
    assert_eq!(
        shift_cells_formula(
            "B3+$C$4+A5+SUM(B2:C8)+SUM(A2:D8)+SUM(B:B)+SUM(3:8)",
            "Data",
            &c
        ),
        "B5+$C$6+A5+SUM(B2:C10)+SUM(A2:D8)+SUM(B:B)+SUM(3:8)"
    );
    assert_eq!(shift_cells_formula("SUM(C8:B2)", "Data", &c), "SUM(C10:B2)");
    assert_eq!(
        shift_cells_formula("B3+Data!C4+Other!C4+SUM(Data!B3:C6)", "Other", &c),
        "B3+Data!C6+Other!C4+SUM(Data!B5:C8)"
    );
}

#[test]
fn deleted_cells_error_and_ranges_shrink_without_changing_strings_or_other_sheets() {
    let c = change(Axis::Rows, false);
    assert_eq!(
        shift_cells_formula(
            "B3+$C$4+SUM(B2:C8)+SUM(B3:C4)+D3+\"B3\"+'Other'!B3",
            "Data",
            &c
        ),
        "#REF!+#REF!+SUM(B2:C6)+SUM(#REF!)+D3+\"B3\"+'Other'!B3"
    );
    assert_eq!(shift_cells_formula("SUM(B8:C3)", "Data", &c), "SUM(B6:C3)");
    assert_eq!(
        shift_cells_formula(
            "D3+$E$4+E5+SUM(A3:F4)",
            "Data",
            &change(Axis::Columns, false)
        ),
        "B3+$C$4+E5+SUM(A3:D4)"
    );
}

#[test]
fn external_links_three_dimensional_references_and_structured_names_are_not_local_cells() {
    let c = change(Axis::Rows, true);
    let f = "[1]Data!B3+'[file.xlsx]Data'!C4+SUM(Data:Other!B3)+SUM('Data:Other'!B3)+SUM(Data:'Other sheet'!C4)+Sales[[B3]:[C4]]+[@B3]+#REF!B3+B3";
    let expected = "[1]Data!B3+'[file.xlsx]Data'!C4+SUM(Data:Other!B3)+SUM('Data:Other'!B3)+SUM(Data:'Other sheet'!C4)+Sales[[B3]:[C4]]+[@B3]+#REF!B3+B5";
    assert_eq!(shift_cells_formula(f, "Data", &c), expected);
    assert_eq!(shift_formula(f, "Data", &c.axis_change()), expected);
}

#[test]
fn invalid_and_edge_geometry_is_explicit() {
    let mut c = change(Axis::Rows, true);
    assert!(c.is_valid());
    assert_eq!(c.map_cell(CellRef::parse("B1048576").unwrap()), None);
    assert_eq!(
        shift_cells_formula("B1048576+A1048576", "Data", &c),
        "#REF!+A1048576"
    );
    c.range = range("C4:B3");
    assert!(!c.is_valid());
}
