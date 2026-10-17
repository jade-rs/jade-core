use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef};
use jade_xlsx_writer::{
    format_axes, styles_part, write_xlsx_with_styles, FormatChange, WriteError,
};

const ORIGINAL: &[u8] = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");

fn mixed() -> Vec<u8> {
    let (part, xml) = styles_part(ORIGINAL).unwrap();
    let (xml, column) = jade_xlsx_writer::add_cell_format(
        &xml,
        None,
        &FormatChange {
            number_format: Some("0.00".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let (xml, row) = jade_xlsx_writer::add_cell_format(
        &xml,
        None,
        &FormatChange {
            italic: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let bytes = write_xlsx_with_styles(ORIGINAL, Some((&part, &xml)), &[]).unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let sheet = package.part_str("xl/worksheets/sheet1.xml").unwrap().unwrap()
        .replace("<sheetData>", &format!("<cols><col min=\"2\" max=\"4\" width=\"18\" customWidth=\"1\" style=\"{column}\" hidden=\"1\"/></cols><sheetData>"))
        .replace("<row r=\"3\">", &format!("<row r=\"3\" s=\"{row}\" customFormat=\"1\" ht=\"31\" customHeight=\"1\">"));
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    package.write().unwrap()
}

fn write(source: &[u8], range: &str, name: &str) -> Vec<u8> {
    let model = read_xlsx(source).unwrap();
    let bytes = format_axes(
        source,
        &model,
        "Operations",
        CellRange::parse(range).unwrap(),
        &FormatChange {
            bold: Some(true),
            fill_color: Some("DDEEAA".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .bytes;
    if let Some(dir) = std::env::var_os("XLSX_SCHEMA_EXPORT_DIR") {
        std::fs::write(std::path::Path::new(&dir).join(name), &bytes).unwrap();
    }
    bytes
}

fn assert_bold(bytes: &[u8], reference: &str, wanted: bool) {
    let workbook = read_xlsx(bytes).unwrap();
    let sheet = &workbook.sheets[0];
    let at = CellRef::parse(reference).unwrap();
    let style = sheet.effective_style_index(at, cell(sheet, at).and_then(|cell| cell.style_index));
    let format = workbook
        .styles
        .cell_format(Some(style.unwrap_or(0)))
        .unwrap();
    assert_eq!(
        workbook.styles.font(format).unwrap().bold,
        wanted,
        "{reference}"
    );
}

#[test]
fn columns_preserve_row_overrides_widths_and_styles_without_dense_cells() {
    let bytes = write(&mixed(), "C1:C1048576", "axis-columns.xlsx");
    for reference in ["C2", "C3", "C50"] {
        assert_bold(&bytes, reference, true);
    }
    for reference in ["B2", "B3", "D50"] {
        assert_bold(&bytes, reference, false);
    }
    let workbook = read_xlsx(&bytes).unwrap();
    let sheet = &workbook.sheets[0];
    assert!(sheet
        .columns
        .iter()
        .all(|span| span.width_chars == Some(18.0) && span.hidden));
    assert_eq!(sheet.columns.len(), 3);
    assert_eq!(sheet.rows.iter().flat_map(|row| &row.cells).count(), 13);
    let c3 = cell(sheet, CellRef::parse("C3").unwrap()).unwrap();
    let format = workbook.styles.cell_format(c3.style_index).unwrap();
    assert!(workbook.styles.font(format).unwrap().italic);
    let default = sheet.effective_style_index(CellRef::parse("C50").unwrap(), None);
    assert_eq!(
        workbook
            .styles
            .number_format_code(workbook.styles.cell_format(default).unwrap()),
        Some("0.00")
    );
}

#[test]
fn rows_preserve_column_intersections_and_create_defaults_for_future_cells() {
    let bytes = write(&mixed(), "A5:XFD6", "axis-rows.xlsx");
    for reference in ["A5", "XFD6", "C5"] {
        assert_bold(&bytes, reference, true);
    }
    assert_bold(&bytes, "A7", false);
    let workbook = read_xlsx(&bytes).unwrap();
    let sheet = &workbook.sheets[0];
    assert_eq!(sheet.rows.iter().flat_map(|row| &row.cells).count(), 19);
    assert_eq!(sheet.rows.len(), 6);
    let style = cell(sheet, CellRef::parse("B5").unwrap())
        .unwrap()
        .style_index;
    assert_eq!(
        workbook
            .styles
            .number_format_code(workbook.styles.cell_format(style).unwrap()),
        Some("0.00")
    );
}

#[test]
fn whole_sheet_uses_compact_column_defaults_and_keeps_custom_row_precedence() {
    let bytes = write(&mixed(), "A1:XFD1048576", "axis-sheet.xlsx");
    for reference in ["A1", "C3", "XFD1048576"] {
        assert_bold(&bytes, reference, true);
    }
    let workbook = read_xlsx(&bytes).unwrap();
    assert_eq!(workbook.sheets[0].rows.len(), 4);
    assert_eq!(workbook.sheets[0].columns.len(), 3);
    assert_eq!(
        workbook.sheets[0]
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .count(),
        13
    );
}

#[test]
fn a_styleless_workbook_gains_valid_relationships_and_a_style_table() {
    let bytes = write(ORIGINAL, "B1:B1048576", "axis-styleless.xlsx");
    assert_bold(&bytes, "B100", true);
    let package = OpcPackage::open(&bytes).unwrap();
    let rels = jade_opc::parse_relationships(
        &package
            .part_str("xl/_rels/workbook.xml.rels")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        rels.target_by_type_suffix("/styles"),
        Some("/xl/styles.xml")
    );
    let types = package.part_str("[Content_Types].xml").unwrap().unwrap();
    assert!(types.contains("spreadsheetml.styles+xml"));
}

#[test]
fn prefixed_rows_and_column_spans_keep_namespaces_and_unknown_metadata() {
    let mut package = OpcPackage::open(ORIGINAL).unwrap();
    let xml = "<s:worksheet xmlns:s=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><s:cols><s:col min=\"2\" max=\"4\" width=\"12\" customWidth=\"1\"/></s:cols><s:sheetData><s:row r=\"2\" ht=\"32\" customHeight=\"1\"/></s:sheetData></s:worksheet>";
    package.replace("xl/worksheets/sheet1.xml", xml.as_bytes().to_vec());
    let source = package.write().unwrap();
    let bytes = write(&source, "A3:XFD3", "axis-prefixed-rows.xlsx");
    assert_bold(&bytes, "B3", true);
    let bytes = write(&bytes, "C1:C1048576", "axis-prefixed-columns.xlsx");
    assert_bold(&bytes, "C10", true);
    let package = OpcPackage::open(&bytes).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(xml.contains("ht=\"32\""));
    assert!(xml.contains("<s:col"));
    assert!(xml.contains("<s:row r=\"3\""));
}

#[test]
fn excessive_axis_intersections_fail_before_writing() {
    let source = mixed();
    let model = read_xlsx(&source).unwrap();
    let result = format_axes(
        &source,
        &model,
        "Operations",
        CellRange::parse("A2:XFD1048576").unwrap(),
        &FormatChange {
            bold: Some(true),
            ..Default::default()
        },
    );
    assert!(matches!(result, Err(WriteError::AxisFormatLimit { .. })));
}

#[test]
fn strict_packages_receive_styles_in_their_own_namespace() {
    let mut package = OpcPackage::open(ORIGINAL).unwrap();
    for name in package.part_names().to_vec() {
        if let Some(xml) = package.part_str(&name).unwrap() {
            let xml = xml
                .replace(
                    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                    "http://purl.oclc.org/ooxml/spreadsheetml/main",
                )
                .replace(
                    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                    "http://purl.oclc.org/ooxml/officeDocument/relationships",
                );
            package.replace(&name, xml.into_bytes());
        }
    }
    let bytes = write(&package.write().unwrap(), "B1:B1048576", "axis-strict.xlsx");
    assert_bold(&bytes, "B20", true);
    let package = OpcPackage::open(&bytes).unwrap();
    assert!(package
        .part_str("xl/styles.xml")
        .unwrap()
        .unwrap()
        .contains("http://purl.oclc.org/ooxml/spreadsheetml/main"));
    assert!(package
        .part_str("xl/_rels/workbook.xml.rels")
        .unwrap()
        .unwrap()
        .contains("http://purl.oclc.org/ooxml/officeDocument/relationships/styles"));
}

#[test]
fn a_relocated_workbook_gets_a_sibling_styles_part() {
    let mut package = OpcPackage::open(ORIGINAL).unwrap();
    let workbook = package.part("xl/workbook.xml").unwrap().to_vec();
    let rels = package
        .part_str("xl/_rels/workbook.xml.rels")
        .unwrap()
        .unwrap()
        .replace("Target=\"worksheets/", "Target=\"/xl/worksheets/");
    package.remove("xl/workbook.xml");
    package.remove("xl/_rels/workbook.xml.rels");
    package.replace("office/book.xml", workbook);
    package.replace("office/_rels/book.xml.rels", rels.into_bytes());
    for name in ["_rels/.rels", "[Content_Types].xml"] {
        let xml = package
            .part_str(name)
            .unwrap()
            .unwrap()
            .replace("xl/workbook.xml", "office/book.xml");
        package.replace(name, xml.into_bytes());
    }
    let bytes = write(
        &package.write().unwrap(),
        "B1:B1048576",
        "axis-relocated.xlsx",
    );
    assert_bold(&bytes, "B20", true);
    assert_eq!(styles_part(&bytes).unwrap().0, "office/styles.xml");
}

#[test]
fn a_dangling_styles_relationship_is_reported_instead_of_replaced_by_a_default_table() {
    let (part, xml) = styles_part(ORIGINAL).unwrap();
    let bytes = write_xlsx_with_styles(ORIGINAL, Some((&part, &xml)), &[]).unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    package.remove(&part);
    assert!(matches!(
        styles_part(&package.write().unwrap()),
        Err(WriteError::MissingPart(_))
    ));
}

fn cell(sheet: &jade_xlsx_reader::Sheet, at: CellRef) -> Option<&jade_xlsx_reader::Cell> {
    sheet
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .find(|cell| cell.reference == at)
}
