use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::OpcPackage;

fn fixture(data: &str, merges: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/filtered-subtotal-workbook.xlsx"
    ))
    .unwrap();
    let xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{data}</sheetData>{merges}</worksheet>"#
    );
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    package.replace(
        "xl/worksheets/_rels/sheet1.xml.rels",
        br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#
            .to_vec(),
    );
    package.write().unwrap()
}
#[test]
fn regions_stop_at_blank_boundaries_ignore_formats_and_include_formulas() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(
        r#"<row r="2"><c r="B2" t="inlineStr"><is><t>Header</t></is></c><c r="C2" t="inlineStr"><is><t>Value</t></is></c></row>
        <row r="3"><c r="B3" t="inlineStr"><is><t> </t></is></c><c r="C3"><v>1</v></c><c r="D3" s="0"/></row>
        <row r="4"><c r="C4" t="str"><f>""</f><v></v></c></row>
        <row r="6"><c r="B6"><v>99</v></c></row>"#, "")).unwrap();
    let before = book.bytes().to_vec();
    assert_eq!(book.current_region("Operations", "$C$3").unwrap(), "B2:C4");
    assert_eq!(book.current_region("Operations", "C4").unwrap(), "B2:C4");
    assert_eq!(book.current_region("Operations", "F3").unwrap(), "F3");
    assert_eq!(book.current_region("Operations", "B6").unwrap(), "B6");
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
    book.toggle_auto_filter("Operations", "C3").unwrap();
    assert_eq!(
        book.auto_filters("Operations").unwrap()[0]
            .reference
            .as_deref(),
        Some("B2:C4")
    );
    book.toggle_auto_filter("Operations", "C3").unwrap();
    assert!(book.auto_filters("Operations").unwrap().is_empty());
    book.undo().unwrap();
    assert_eq!(book.auto_filters("Operations").unwrap().len(), 1);
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
}
#[test]
fn diagonal_cells_and_filled_merges_connect_without_allocating_their_grid() {
    let book = HeadlessWorkbook::open_for_editing(&fixture(
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="B2"><v>2</v></c></row>
        <row r="3"><c r="C3"><v>3</v></c></row><row r="4"><c r="E4"><v>4</v></c></row>"#,
        r#"<mergeCells count="2"><mergeCell ref="C3:E3"/><mergeCell ref="H2:J2"/></mergeCells>"#,
    ))
    .unwrap();
    assert_eq!(book.current_region("Operations", "D3").unwrap(), "A1:E4");
    assert_eq!(book.current_region("Operations", "A1").unwrap(), "A1:E4");
    assert_eq!(book.current_region("Operations", "I2").unwrap(), "H2:J2");
}
#[test]
fn extreme_coordinates_and_explicit_ranges_are_bounded() {
    let mut book = HeadlessWorkbook::open_for_editing(&fixture(
        r#"<row r="1048576"><c r="XFD1048576"><v>1</v></c></row>"#,
        "",
    ))
    .unwrap();
    assert_eq!(
        book.current_region("Operations", "XFD1048576").unwrap(),
        "XFD1048576"
    );
    assert!(book.current_region("Operations", "A1:B2").is_err());
    assert!(book.current_region("Operations", "XFE1").is_err());
    book.toggle_auto_filter("Operations", "B2:D4").unwrap();
    assert_eq!(
        book.auto_filters("Operations").unwrap()[0]
            .reference
            .as_deref(),
        Some("B2:D4")
    );
    assert!(book.toggle_auto_filter("Operations", "D4:B2").is_err());
}
#[test]
fn table_toggle_keeps_table_definition_and_rejects_headerless_enable() {
    let data = include_bytes!("../../../public/fixtures/filtered-subtotal-workbook.xlsx");
    let mut book = HeadlessWorkbook::open_for_editing(data).unwrap();
    book.toggle_auto_filter("Operations", "B2").unwrap();
    assert_eq!(book.workbook().sheets[0].tables.len(), 1);
    assert!(!book.workbook().sheets[0].tables[0].auto_filter);
    assert!(book.workbook().sheets[0].auto_filter.is_some());
    book.toggle_auto_filter("Operations", "B2").unwrap();
    assert!(book.workbook().sheets[0].tables[0].auto_filter);
    let mut package = OpcPackage::open(data).unwrap();
    let xml = package
        .part_str("xl/tables/table1.xml")
        .unwrap()
        .unwrap()
        .replace("<autoFilter ref=\"A1:C4\"/>", "")
        .replace("<table ", "<table headerRowCount=\"0\" ");
    package.replace("xl/tables/table1.xml", xml.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let before = book.bytes().to_vec();
    assert!(book.toggle_auto_filter("Operations", "B2").is_err());
    assert_eq!(book.bytes(), before);
    assert!(!book.can_undo());
}
