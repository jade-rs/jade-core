use jade_xlsx_reader::{move_formula, CellRange, CellRef};
fn moved(formula: &str, old: &str, new: &str) -> String {
    move_formula(
        formula,
        old,
        new,
        "Operations",
        CellRange::parse("A2:XFD2").unwrap(),
        "Target",
        CellRef::new(0, 9),
    )
}
#[test]
fn moved_formulas_follow_moved_cells_and_keep_unmoved_references_on_their_old_sheet() {
    assert_eq!(
        moved(
            "$B$2+B3+'Operations'!B2+'Other'!B2+\"B2\"",
            "Operations",
            "Target"
        ),
        "$B$10+'Operations'!B3+'Target'!B10+'Other'!B2+\"B2\""
    );
}
#[test]
fn dependents_on_other_sheets_and_external_workbooks_keep_their_scope() {
    assert_eq!(
        moved(
            "B2+Operations!$B$2+'[Book.xlsx]Operations'!B2+[Book.xlsx]Operations!B2",
            "Other",
            "Other"
        ),
        "B2+'Target'!$B$10+'[Book.xlsx]Operations'!B2+[Book.xlsx]Operations!B2"
    );
}
#[test]
fn whole_axis_references_and_reversed_ranges_preserve_absolute_flags() {
    let range = CellRange::parse("B1:C1048576").unwrap();
    assert_eq!(
        move_formula(
            "SUM($B:$C)+SUM(C3:B1)",
            "Operations",
            "Operations",
            "Operations",
            range,
            "Operations",
            CellRef::new(7, 0)
        ),
        "SUM($H:$I)+SUM(I3:H1)"
    );
}
