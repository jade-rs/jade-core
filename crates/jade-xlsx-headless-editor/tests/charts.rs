use jade_xlsx_headless_editor::HeadlessWorkbook;
#[test]
fn chart_status_distinguishes_drawn_plots_from_unsupported_combinations() {
    let book = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/charts-workbook.xlsx"
    ))
    .unwrap();
    let status = book.chart_status();
    assert_eq!(status.len(), 10);
    assert_eq!(status.iter().filter(|chart| chart.rendered).count(), 9);
    assert_eq!(status[6].sheet, "Charts");
    assert!(status[6].source_states.contains(&"live".into()));
    assert!(status[6].unsupported_properties.is_empty());
    assert!(!status[7].rendered);
    assert!(status[7]
        .unsupported_properties
        .contains(&"combined or missing plot".into()));
}
