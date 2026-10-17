use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, Axis, CellRef};
use jade_xlsx_writer::dimensions::set_axis_sizes;
use std::collections::BTreeMap;
const SOURCE: &[u8] = include_bytes!("../../../public/fixtures/clipboard-workbook.xlsx");
fn write(source: &[u8], axis: Axis, sizes: &[(u32, f32)], name: &str) -> Vec<u8> {
    let bytes = set_axis_sizes(
        source,
        &read_xlsx(source).unwrap(),
        "Operations",
        axis,
        &sizes.iter().copied().collect(),
    )
    .unwrap();
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(name), &bytes).unwrap();
    }
    bytes
}
#[test]
fn column_spans_split_without_changing_cell_data_or_unselected_widths() {
    let source = read_xlsx(SOURCE).unwrap();
    let bytes = write(
        SOURCE,
        Axis::Columns,
        &[(1, 12.5), (6, 0.0)],
        "dimensions-columns.xlsx",
    );
    let model = read_xlsx(&bytes).unwrap();
    assert_eq!(model.sheets[0].rows, source.sheets[0].rows);
    assert_eq!(
        model.sheets[0].column_span(1).unwrap().width_chars,
        Some(12.5)
    );
    assert_eq!(
        model.sheets[0].column_span(2).unwrap().width_chars,
        source.sheets[0].column_span(2).unwrap().width_chars
    );
    assert!(model.sheets[0].column_span(6).unwrap().hidden);
    let bytes = write(
        &bytes,
        Axis::Columns,
        &[(6, 20.0)],
        "dimensions-revealed.xlsx",
    );
    assert!(
        !read_xlsx(&bytes).unwrap().sheets[0]
            .column_span(6)
            .unwrap()
            .hidden
    );
    let original = OpcPackage::open(SOURCE).unwrap();
    let edited = OpcPackage::open(&bytes).unwrap();
    for name in original
        .part_names()
        .iter()
        .filter(|name| name.as_str() != "xl/worksheets/sheet1.xml")
    {
        assert_eq!(edited.part(name), original.part(name), "{name}");
    }
}
#[test]
fn rows_keep_their_cell_styles_and_formulas_and_new_rows_are_sparse() {
    let bytes = write(
        SOURCE,
        Axis::Rows,
        &[(1, 36.0), (6, 0.0), (9, 24.0)],
        "dimensions-rows.xlsx",
    );
    let model = read_xlsx(&bytes).unwrap();
    let sheet = &model.sheets[0];
    let row = sheet.rows.iter().find(|row| row.index == 1).unwrap();
    assert_eq!(row.height_points, Some(36.0));
    assert!(row.custom_height);
    assert_eq!(
        row.cells
            .iter()
            .find(|cell| cell.reference == CellRef::new(2, 1))
            .unwrap()
            .formula
            .as_deref(),
        Some("B2*2")
    );
    assert!(sheet.rows.iter().find(|row| row.index == 6).unwrap().hidden);
    assert_eq!(sheet.rows.len(), 6);
}
#[test]
fn prefixed_and_reference_less_rows_preserve_namespace_and_extension_attributes() {
    let mut package = OpcPackage::open(SOURCE).unwrap();
    let xml = r#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="urn:metadata" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x"><s:sheetData><s:row x:custom="keep"><s:c><s:v>42</s:v></s:c></s:row><s:row><s:c><s:v>7</s:v></s:c></s:row></s:sheetData><s:extLst><s:ext uri="metadata"><x:col min="1" max="1" width="999"/></s:ext></s:extLst></s:worksheet>"#;
    package.replace("xl/worksheets/sheet1.xml", xml.as_bytes().to_vec());
    let source = package.write().unwrap();
    let bytes = write(
        &source,
        Axis::Rows,
        &[(0, 25.0), (1, 32.0)],
        "dimensions-prefixed.xlsx",
    );
    let bytes = write(
        &bytes,
        Axis::Columns,
        &[(0, 12.0)],
        "dimensions-prefixed-columns.xlsx",
    );
    let package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.contains("x:custom=\"keep\""));
    assert!(xml.contains("<x:col min=\"1\" max=\"1\" width=\"999\"/>"));
    let model = read_xlsx(&bytes).unwrap();
    assert_eq!(model.sheets[0].rows[1].height_points, Some(32.0));
}
#[test]
fn invalid_sizes_and_ranges_are_rejected_without_an_export() {
    let model = read_xlsx(SOURCE).unwrap();
    for (axis, index, size) in [
        (Axis::Rows, 0, -1.0),
        (Axis::Rows, 0, f32::NAN),
        (Axis::Rows, 0, 410.0),
        (Axis::Columns, 0, 256.0),
        (Axis::Columns, 16384, 10.0),
    ] {
        assert!(set_axis_sizes(
            SOURCE,
            &model,
            "Operations",
            axis,
            &BTreeMap::from([(index, size)])
        )
        .is_err());
    }
}

#[test]
fn resizing_one_slice_of_a_shared_column_span_preserves_other_attributes() {
    let (part, styles) = jade_xlsx_writer::styles_part(SOURCE).unwrap();
    let source =
        jade_xlsx_writer::write_xlsx_with_styles(SOURCE, Some((&part, &styles)), &[]).unwrap();
    let mut package = OpcPackage::open(&source).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    let start = xml.find("<cols>").unwrap();
    let end = xml.find("</cols>").unwrap() + "</cols>".len();
    let xml = format!("{}<cols><col min=\"2\" max=\"6\" width=\"18\" customWidth=\"1\" style=\"0\" outlineLevel=\"2\" collapsed=\"1\"/></cols>{}", &xml[..start], &xml[end..]);
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let bytes = write(
        &package.write().unwrap(),
        Axis::Columns,
        &[(2, 22.0), (3, 22.0)],
        "dimensions-spans.xlsx",
    );
    let sheet = &read_xlsx(&bytes).unwrap().sheets[0];
    assert_eq!(sheet.columns.len(), 3);
    assert_eq!(sheet.column_span(1).unwrap().width_chars, Some(18.0));
    assert_eq!(sheet.column_span(2).unwrap().width_chars, Some(22.0));
    assert_eq!(sheet.column_span(4).unwrap().width_chars, Some(18.0));
    let package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert_eq!(xml.matches("outlineLevel=\"2\"").count(), 3);
    assert_eq!(xml.matches("collapsed=\"1\"").count(), 3);
}

#[test]
fn whole_sheet_row_height_is_compact_and_hiding_can_be_overridden() {
    use jade_xlsx_writer::dimensions::set_all_row_sizes;
    let source = read_xlsx(SOURCE).unwrap();
    let bytes = set_all_row_sizes(SOURCE, &source, "Operations", 27.0).unwrap();
    let model = read_xlsx(&bytes).unwrap();
    let sheet = &model.sheets[0];
    assert_eq!(sheet.rows.len(), source.sheets[0].rows.len());
    assert_eq!(sheet.default_row_height_points, Some(27.0));
    assert!(sheet.default_row_custom_height);
    assert!(!sheet.default_rows_hidden);
    for (old, row) in source.sheets[0].rows.iter().zip(&sheet.rows) {
        assert_eq!(row.cells, old.cells);
        assert_eq!(row.height_points, Some(27.0));
        assert!(row.custom_height);
        assert!(!row.hidden);
    }
    let package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.len() < 10_000);
    let original = OpcPackage::open(SOURCE).unwrap();
    for name in original
        .part_names()
        .iter()
        .filter(|name| name.as_str() != sheet.part_name)
    {
        assert_eq!(package.part(name), original.part(name), "{name}");
    }
    save_dimension(&bytes, "dimensions-all-rows.xlsx");
    let hidden = set_all_row_sizes(&bytes, &model, "Operations", 0.0).unwrap();
    let model = read_xlsx(&hidden).unwrap();
    assert!(model.sheets[0].default_rows_hidden);
    assert_eq!(model.sheets[0].default_row_height_points, Some(27.0));
    assert!(model.sheets[0].rows.iter().all(|row| row.hidden));
    save_dimension(&hidden, "dimensions-hidden-default.xlsx");
    let revealed = write(
        &hidden,
        Axis::Rows,
        &[(20, 30.0)],
        "dimensions-default-override.xlsx",
    );
    let model = read_xlsx(&revealed).unwrap();
    assert!(model.sheets[0].default_rows_hidden);
    assert!(
        !model.sheets[0]
            .rows
            .iter()
            .find(|row| row.index == 20)
            .unwrap()
            .hidden
    );
}
fn save_dimension(bytes: &[u8], name: &str) {
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}
#[test]
fn whole_sheet_row_sizes_insert_prefixed_defaults_before_columns() {
    use jade_xlsx_writer::dimensions::set_all_row_sizes;
    let mut package = OpcPackage::open(SOURCE).unwrap();
    let xml = r#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><s:cols><s:col min="1" max="1" width="20"/></s:cols><s:sheetData><s:row outlineLevel="2" collapsed="1"><s:c><s:v>42</s:v></s:c></s:row></s:sheetData></s:worksheet>"#;
    package.replace("xl/worksheets/sheet1.xml", xml.as_bytes().to_vec());
    let original = package.write().unwrap();
    let bytes = set_all_row_sizes(
        &original,
        &read_xlsx(&original).unwrap(),
        "Operations",
        24.0,
    )
    .unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.find("<s:sheetFormatPr").unwrap() < xml.find("<s:cols").unwrap());
    assert!(xml.contains("outlineLevel=\"2\" collapsed=\"1\""));
    assert_eq!(read_xlsx(&bytes).unwrap().sheets[0].rows.len(), 1);
    save_dimension(&bytes, "dimensions-prefixed-default.xlsx");
    for size in [-1.0, 410.0, f32::NAN, f32::INFINITY] {
        assert!(set_all_row_sizes(
            &original,
            &read_xlsx(&original).unwrap(),
            "Operations",
            size
        )
        .is_err());
    }
}

#[test]
fn automatic_sheet_fit_keeps_sparse_exceptions_and_unrelated_row_metadata() {
    use jade_xlsx_writer::dimensions::set_row_sizes_with_default;
    let model = read_xlsx(SOURCE).unwrap();
    let bytes = set_row_sizes_with_default(
        SOURCE,
        &model,
        "Operations",
        15.0,
        &BTreeMap::from([(1, 45.0), (999, 60.0)]),
        false,
    )
    .unwrap();
    let after = read_xlsx(&bytes).unwrap();
    let sheet = &after.sheets[0];
    assert!(!sheet.default_row_custom_height && !sheet.default_rows_hidden);
    assert_eq!(sheet.default_row_height_points, Some(15.0));
    assert_eq!(sheet.rows.len(), model.sheets[0].rows.len() + 1);
    assert_eq!(
        sheet
            .rows
            .iter()
            .find(|row| row.index == 1)
            .unwrap()
            .height_points,
        Some(45.0)
    );
    assert_eq!(sheet.rows.last().unwrap().index, 999);
    assert!(sheet
        .rows
        .iter()
        .all(|row| !row.custom_height && !row.hidden));
    for (old, row) in model.sheets[0].rows.iter().zip(&sheet.rows) {
        assert_eq!(old.cells, row.cells);
    }
    let before = OpcPackage::open(SOURCE).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    for name in before
        .part_names()
        .iter()
        .filter(|name| name.as_str() != sheet.part_name)
    {
        assert_eq!(before.part(name), package.part(name), "{name}");
    }
    assert!(package.part_str(&sheet.part_name).unwrap().unwrap().len() < 10_000);
    save_dimension(&bytes, "dimensions-fitted-sheet.xlsx");
    for (index, height) in [(1_048_576, 15.0), (0, f32::NAN), (1, 410.0)] {
        assert!(set_row_sizes_with_default(
            SOURCE,
            &model,
            "Operations",
            15.0,
            &BTreeMap::from([(index, height)]),
            false
        )
        .is_err());
    }
}
#[test]
fn fitted_rows_override_pinned_defaults_and_sparse_empty_prefixed_data_is_supported() {
    use jade_xlsx_writer::dimensions::{set_fitted_row_sizes, set_row_sizes_with_default};
    let model = read_xlsx(SOURCE).unwrap();
    let pinned =
        jade_xlsx_writer::dimensions::set_all_row_sizes(SOURCE, &model, "Operations", 40.0)
            .unwrap();
    let bytes = set_fitted_row_sizes(
        &pinned,
        &read_xlsx(&pinned).unwrap(),
        "Operations",
        &BTreeMap::from([(1, 15.0)]),
    )
    .unwrap();
    let model = read_xlsx(&bytes).unwrap();
    assert!(model.sheets[0].default_row_custom_height);
    assert!(
        !model.sheets[0]
            .rows
            .iter()
            .find(|row| row.index == 1)
            .unwrap()
            .custom_height
    );
    assert!(
        model.sheets[0]
            .rows
            .iter()
            .find(|row| row.index == 0)
            .unwrap()
            .custom_height
    );
    save_dimension(&bytes, "dimensions-fitted-row.xlsx");
    let mut package = OpcPackage::open(SOURCE).unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<s:worksheet xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><s:sheetData/></s:worksheet>"#.to_vec());
    let original = package.write().unwrap();
    let bytes = set_row_sizes_with_default(
        &original,
        &read_xlsx(&original).unwrap(),
        "Operations",
        15.0,
        &BTreeMap::from([(6, 30.0)]),
        false,
    )
    .unwrap();
    let model = read_xlsx(&bytes).unwrap();
    assert_eq!(model.sheets[0].rows.len(), 1);
    assert_eq!(model.sheets[0].rows[0].index, 6);
    assert_eq!(model.sheets[0].rows[0].height_points, Some(30.0));
    assert!(!model.sheets[0].rows[0].custom_height);
    save_dimension(&bytes, "dimensions-fitted-empty.xlsx");
}

#[test]
fn encoded_column_ticks_are_serialized_exactly_for_office_double_precision_readers() {
    let bytes = write(
        SOURCE,
        Axis::Columns,
        &[(1, 4790.0_f32 / 256.0), (2, 6326.0_f32 / 256.0)],
        "dimensions-column-ticks.xlsx",
    );
    let package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.contains("width=\"18.7109375\""));
    assert!(xml.contains("width=\"24.7109375\""));
}
