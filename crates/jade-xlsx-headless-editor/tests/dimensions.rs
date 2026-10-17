use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::Axis;
#[test]
fn size_changes_are_one_history_step_and_do_not_recalculate_values() {
    let mut workbook = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let before = workbook.bytes().to_vec();
    let calculation = workbook.summary().full_calculation_on_load;
    workbook
        .set_axis_size("Operations", Axis::Columns, 1, 2, 20.0)
        .unwrap();
    assert_eq!(
        workbook.workbook().sheets[0]
            .column_span(1)
            .unwrap()
            .width_chars,
        Some(20.0)
    );
    assert_eq!(workbook.summary().full_calculation_on_load, calculation);
    workbook.undo().unwrap();
    assert_eq!(workbook.bytes(), before);
    workbook.redo().unwrap();
    assert_eq!(
        workbook.cell("Operations", "C2").unwrap().number,
        Some(20.0)
    );
}

#[test]
fn all_rows_resize_hide_reveal_and_copy_preserve_sparse_history_and_defaults() {
    let mut workbook = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/clipboard-workbook.xlsx"
    ))
    .unwrap();
    let before = workbook.bytes().to_vec();
    let rows = workbook.workbook().sheets[0].rows.len();
    workbook
        .set_axis_size("Operations", Axis::Rows, 0, 1_048_575, 24.0)
        .unwrap();
    assert_eq!(workbook.workbook().sheets[0].rows.len(), rows);
    assert_eq!(
        workbook.workbook().sheets[0].default_row_height_points,
        Some(24.0)
    );
    workbook.undo().unwrap();
    assert_eq!(workbook.bytes(), before);
    workbook.redo().unwrap();
    workbook
        .set_axis_size("Operations", Axis::Rows, 0, 1_048_575, 0.0)
        .unwrap();
    workbook
        .set_axis_size("Operations", Axis::Rows, 20, 20, 30.0)
        .unwrap();
    let snapshot = workbook
        .copy_snapshot("Operations", "A1:XFD1048576", false)
        .unwrap();
    workbook
        .paste_snapshot(&snapshot, "Target", "A1", true)
        .unwrap();
    let target = &workbook.workbook().sheets[1];
    assert!(target.default_rows_hidden);
    assert!(target.default_row_custom_height);
    assert_eq!(target.default_row_height_points, Some(24.0));
    assert!(
        target
            .rows
            .iter()
            .find(|row| row.index == 0)
            .unwrap()
            .hidden
    );
    assert!(
        !target
            .rows
            .iter()
            .find(|row| row.index == 20)
            .unwrap()
            .hidden
    );
    assert!(target.rows.len() < 10);
}

#[test]
fn a_fitted_default_and_exceptions_commit_once_and_invalid_plans_leave_history_intact() {
    use std::collections::BTreeMap;
    let mut workbook = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let before = workbook.bytes().to_vec();
    let calc = workbook.summary().full_calculation_on_load;
    workbook
        .set_row_sizes_with_default(
            "Operations",
            15.0,
            &BTreeMap::from([(1, 45.0), (6, 60.0)]),
            false,
        )
        .unwrap();
    let fitted = workbook.bytes().to_vec();
    assert!(!workbook.workbook().sheets[0].default_row_custom_height);
    assert!(!workbook.workbook().sheets[0].rows[1].custom_height);
    assert_eq!(workbook.summary().full_calculation_on_load, calc);
    assert_eq!(
        workbook.cell("Operations", "C2").unwrap().number,
        Some(20.0)
    );
    assert!(workbook
        .set_row_sizes_with_default("Operations", 15.0, &BTreeMap::from([(6, f32::NAN)]), false)
        .is_err());
    assert_eq!(workbook.bytes(), fitted);
    workbook.undo().unwrap();
    assert_eq!(workbook.bytes(), before);
    workbook.redo().unwrap();
    assert_eq!(workbook.bytes(), fitted);
}

fn wrapped_book() -> HeadlessWorkbook {
    use jade_xlsx_headless_api::CellInput;
    use jade_xlsx_headless_editor::FormatChange;
    let mut book = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    book.set_format(
        "Operations",
        "A2",
        &FormatChange {
            wrap_text: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    book.set_cells(
        "Operations",
        &[("A2".into(), CellInput::Text("one\ntwo\nthree".into()))],
    )
    .unwrap();
    book.set_fitted_row_sizes("Operations", &std::collections::BTreeMap::from([(1, 45.0)]))
        .unwrap();
    book.refresh_automatic_row_heights(&Default::default())
        .unwrap();
    book
}
fn height(book: &HeadlessWorkbook, row: u32) -> f32 {
    book.workbook().sheets[0]
        .rows
        .iter()
        .find(|item| item.index == row)
        .unwrap()
        .height_points
        .unwrap()
}
#[test]
fn editing_automatic_content_shrinks_its_saved_cache_in_the_same_history_step() {
    use jade_xlsx_headless_api::CellInput;
    let mut book = wrapped_book();
    let before = book.bytes().to_vec();
    book.set_cells(
        "Operations",
        &[("A2".into(), CellInput::Text("Short".into()))],
    )
    .unwrap();
    let calc = book.summary().full_calculation_on_load;
    assert!(book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    assert_eq!(height(&book, 1), 15.0);
    assert_eq!(book.summary().full_calculation_on_load, calc);
    assert!(!book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    let after = book.bytes().to_vec();
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    assert!(!book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    book.redo().unwrap();
    assert_eq!(book.bytes(), after);
    assert!(!book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("dimensions-reflow.xlsx"),
            book.bytes(),
        )
        .unwrap();
    }
}
#[test]
fn wider_columns_and_font_changes_remeasure_automatic_rows_but_manual_heights_stay_pinned() {
    use jade_xlsx_headless_editor::FormatChange;
    let mut book = wrapped_book();
    book.set_axis_size("Operations", Axis::Columns, 0, 0, 40.0)
        .unwrap();
    assert!(!book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    // Explicit newlines remain three lines even in a wider column.
    assert_eq!(height(&book, 1), 45.0);
    book.set_format(
        "Operations",
        "A2",
        &FormatChange {
            font_size: Some(22.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    assert_eq!(height(&book, 1), 90.0);
    book.set_format(
        "Operations",
        "A2",
        &FormatChange {
            font_size: Some(11.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    assert_eq!(height(&book, 1), 45.0);
    book.set_axis_size("Operations", Axis::Rows, 1, 1, 100.0)
        .unwrap();
    book.set_format(
        "Operations",
        "A2",
        &FormatChange {
            font_size: Some(22.0),
            ..Default::default()
        },
    )
    .unwrap();
    book.refresh_automatic_row_heights(&Default::default())
        .unwrap();
    assert_eq!(height(&book, 1), 100.0);
}
#[test]
fn reflow_keeps_hidden_rows_hidden_and_preserves_unrelated_imported_height_caches() {
    use jade_opc::OpcPackage;
    use jade_xlsx_headless_api::CellInput;
    use jade_xlsx_headless_editor::FormatChange;
    let book = wrapped_book();
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "ht=\"45\" customHeight=\"0\" hidden=\"0\"",
            "ht=\"45\" customHeight=\"0\" hidden=\"1\"",
        );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    assert!(book.workbook().sheets[0].rows[1].hidden);
    book.set_cells(
        "Operations",
        &[("A2".into(), CellInput::Text("Short".into()))],
    )
    .unwrap();
    assert!(book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    assert!(book.workbook().sheets[0].rows[1].hidden);
    assert_eq!(height(&book, 1), 15.0);
    book.set_fitted_row_sizes("Operations", &std::collections::BTreeMap::from([(2, 18.0)]))
        .unwrap();
    book.refresh_automatic_row_heights(&Default::default())
        .unwrap();
    book.set_format(
        "Operations",
        "A1",
        &FormatChange {
            fill_color: Some("33AA66".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    assert_eq!(height(&book, 2), 18.0);
}

#[test]
fn standalone_recalculation_schedules_height_refresh_without_an_edit_transaction() {
    use jade_opc::OpcPackage;
    let book = wrapped_book();
    let mut package = OpcPackage::open(book.bytes()).unwrap();
    let mut xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let start = xml.find("<c r=\"A2\"").unwrap();
    let end = xml[start..].find("</c>").unwrap() + start + 4;
    xml.replace_range(
        start..end,
        "<c r=\"A2\" s=\"1\" t=\"str\"><f>\"Short\"</f><v>one&#10;two&#10;three</v></c>",
    );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    assert_eq!(height(&book, 1), 45.0);
    assert!(book.recalculate().unwrap().refreshed > 0);
    assert!(book
        .refresh_automatic_row_heights(&Default::default())
        .unwrap());
    assert_eq!(height(&book, 1), 15.0);
    assert!(!book.can_undo());
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("dimensions-recalculated.xlsx"),
            book.bytes(),
        )
        .unwrap();
    }
}

#[test]
fn external_font_metrics_update_view_and_exports_without_changing_history_bytes() {
    use jade_xlsx_headless_api::CellInput;
    use jade_xlsx_layout::{LayoutOptions, ResolvedFont, TextMeasure};
    use std::rc::Rc;
    struct Measure(f64);
    impl TextMeasure for Measure {
        fn measure(&self, text: &str, _: &ResolvedFont) -> f64 {
            text.chars().count() as f64 * self.0
        }
    }
    let mut book = wrapped_book();
    book.set_cells(
        "Operations",
        &[(
            "A2".into(),
            CellInput::Text("iiiiiiiiiiiiiiiiiiiiiiiiiiiiii".into()),
        )],
    )
    .unwrap();
    book.set_fitted_row_sizes("Operations", &std::collections::BTreeMap::from([(1, 45.0)]))
        .unwrap();
    let raw = book.bytes().to_vec();
    let options = LayoutOptions {
        measure: Rc::new(Measure(1.0)),
        ..Default::default()
    };
    let measured = book.workbook_with_refreshed_row_metrics(&options);
    assert_eq!(measured.sheets[0].rows[1].height_points, Some(15.0));
    assert_eq!(book.bytes(), raw);
    let exported = book.export_bytes_with_row_metrics(&options).unwrap();
    assert_eq!(
        jade_xlsx_reader::read_xlsx(&exported).unwrap().sheets[0].rows[1].height_points,
        Some(15.0)
    );
    assert_eq!(book.bytes(), raw);
    book.undo().unwrap();
    let undo = book.bytes().to_vec();
    book.export_bytes_with_row_metrics(&options).unwrap();
    assert_eq!(book.bytes(), undo);
    book.redo().unwrap();
    assert_eq!(book.bytes(), raw);
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("dimensions-font-metrics.xlsx"),
            exported,
        )
        .unwrap();
    }
}

#[test]
fn scoped_font_refresh_preserves_caches_of_rows_using_other_fonts() {
    use jade_xlsx_headless_api::CellInput;
    use jade_xlsx_headless_editor::FormatChange;
    use jade_xlsx_layout::{LayoutOptions, ResolvedFont, TextMeasure};
    use std::{collections::HashSet, rc::Rc};
    struct Compact;
    impl TextMeasure for Compact {
        fn measure(&self, text: &str, _: &ResolvedFont) -> f64 {
            text.chars().count() as f64
        }
    }
    let mut book = wrapped_book();
    book.set_cells(
        "Operations",
        &[("A2".into(), CellInput::Text("i".repeat(30)))],
    )
    .unwrap();
    book.set_format(
        "Operations",
        "A2",
        &FormatChange {
            font_name: Some("JadeScoped".into()),
            ..Default::default()
        },
    )
    .unwrap();
    book.set_fitted_row_sizes(
        "Operations",
        &std::collections::BTreeMap::from([(1, 45.0), (2, 18.0)]),
    )
    .unwrap();
    let before = book.bytes().to_vec();
    let options = LayoutOptions {
        measure: Rc::new(Compact),
        ..Default::default()
    };
    let families = HashSet::from(["JadeScoped".to_string()]);
    let measured = book.workbook_with_refreshed_row_metrics_for_fonts(&options, Some(&families));
    assert_eq!(measured.sheets[0].rows[1].height_points, Some(15.0));
    assert_eq!(measured.sheets[0].rows[2].height_points, Some(18.0));
    let export = book
        .export_bytes_with_row_metrics_for_fonts(&options, Some(&families))
        .unwrap();
    let model = jade_xlsx_reader::read_xlsx(&export).unwrap();
    assert_eq!(model.sheets[0].rows[1].height_points, Some(15.0));
    assert_eq!(model.sheets[0].rows[2].height_points, Some(18.0));
    assert_eq!(book.bytes(), before);
}
