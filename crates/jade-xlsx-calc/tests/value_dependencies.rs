use jade_xlsx_calc::{evaluate, evaluate_with_value_dependencies};
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef, CellValue, DefinedName, Workbook};
fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}
fn book() -> Workbook {
    read_xlsx(include_bytes!(
        "../../../public/fixtures/pivot-cache-compat.xlsx"
    ))
    .unwrap()
}
fn formula(b: &mut Workbook, target: &str, text: &str) {
    let row = b.sheets[1].rows.iter_mut().find(|r| r.index == 5).unwrap();
    let mut cell = row.cells[1].clone();
    cell.reference = at(target);
    cell.formula = Some(text.into());
    cell.value = CellValue::Number(999.);
    row.cells.push(cell);
}
#[test]
fn tracing_is_opt_in_and_keeps_the_same_calculation_values() {
    let mut b = book();
    formula(&mut b, "H6", "INDIRECT(\"F4\")+SUM(F5:F6)");
    formula(&mut b, "I6", "H6*2");
    let ordinary = evaluate(&b).unwrap();
    assert!(ordinary.dependencies.is_none());
    let traced = evaluate_with_value_dependencies(&b, "UTC", &Default::default()).unwrap();
    assert_eq!(ordinary.cells, traced.cells);
    assert_eq!(ordinary.skipped, traced.skipped);
    let reads = traced.dependencies.unwrap();
    assert!(!reads.incomplete);
    assert!(reads.reads[&(1, at("H6"))].contains(&(1, CellRange::parse("F5:F6").unwrap())));
    assert!(reads.reads[&(1, at("I6"))].contains(&(1, CellRange::parse("H6").unwrap())));
}
#[test]
fn named_ranges_and_pivot_result_reads_keep_their_actual_dependencies() {
    let mut b = book();
    b.defined_names.push(DefinedName {
        name: "SavedTotals".into(),
        formula: "Report!$F$4:$F$6".into(),
        local_sheet_index: None,
        hidden: false,
    });
    formula(&mut b, "H6", "SUM(SavedTotals)");
    formula(&mut b, "I6", "GETPIVOTDATA(\"Amount\",E3)");
    let result = evaluate_with_value_dependencies(&b, "UTC", &Default::default()).unwrap();
    assert!(result.skipped.is_empty());
    let deps = result.dependencies.unwrap();
    assert!(deps.reads[&(1, at("H6"))].contains(&(1, CellRange::parse("F4:F6").unwrap())));
    assert!(deps.reads[&(1, at("I6"))].contains(&(1, CellRange::parse("F6").unwrap())));
}
#[test]
fn spill_output_reads_retain_the_anchor_dependency_without_expanding_the_range() {
    let mut b = book();
    formula(&mut b, "H6", "SEQUENCE(2,1,F4,1)");
    formula(&mut b, "I6", "SUM(H6#)");
    let result = evaluate_with_value_dependencies(&b, "UTC", &Default::default()).unwrap();
    assert!(result.skipped.is_empty());
    assert_eq!(result.spills.len(), 1);
    let deps = result.dependencies.unwrap();
    assert!(deps.reads[&(1, at("H6"))].contains(&(1, CellRange::parse("F4").unwrap())));
    assert!(deps.reads[&(1, at("I6"))].contains(&(1, CellRange::parse("H6:H7").unwrap())));
}
