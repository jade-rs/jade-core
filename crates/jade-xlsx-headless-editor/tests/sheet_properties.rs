use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::SheetVisibility;

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}

#[test]
fn hiding_the_active_sheet_moves_the_saved_view_and_preserves_local_names() {
    let mut book = HeadlessWorkbook::open(&fixture()).unwrap();
    let first = book.workbook().sheets[0].name.clone();
    let second = book.workbook().sheets[1].name.clone();
    assert!(book
        .set_sheet_visibility(&first, SheetVisibility::Hidden)
        .unwrap());
    assert_eq!(book.workbook().active_sheet_index, Some(1));
    assert_eq!(
        book.workbook().sheets[0].visibility,
        SheetVisibility::Hidden
    );
    let before = book.bytes().to_vec();
    assert!(book
        .set_sheet_visibility(&second, SheetVisibility::VeryHidden)
        .unwrap_err()
        .is_invalid_input());
    assert_eq!(book.bytes(), before);
    assert!(!book
        .set_sheet_visibility(&first, SheetVisibility::Hidden)
        .unwrap());
    book.undo().unwrap();
    assert_eq!(
        book.workbook().sheets[0].visibility,
        SheetVisibility::Visible
    );
    book.redo().unwrap();
    assert_eq!(
        book.workbook().sheets[0].visibility,
        SheetVisibility::Hidden
    );
    book.set_sheet_visibility(&first, SheetVisibility::Visible)
        .unwrap();
    book.set_sheet_visibility(&second, SheetVisibility::VeryHidden)
        .unwrap();
    let reopened = HeadlessWorkbook::open(&book.save_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.workbook().sheets[1].visibility,
        SheetVisibility::VeryHidden
    );
    assert_eq!(reopened.workbook().active_sheet_index, Some(0));
}

#[test]
fn tab_colour_preserves_sheet_properties_extensions_and_schema_order() {
    let mut package = OpcPackage::open(&fixture()).unwrap();
    let part = "xl/worksheets/sheet1.xml";
    let xml = package.required_part_str(part).unwrap().replace("<sheetViews", "<sheetPr codeName=\"SheetCode\"><outlinePr summaryBelow=\"0\"/><pageSetUpPr fitToPage=\"1\"/></sheetPr><sheetViews");
    package.replace(part, xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set_tab_color(&sheet, Some("12ab34")).unwrap();
    let xml = OpcPackage::open(book.bytes())
        .unwrap()
        .required_part_str(part)
        .unwrap();
    assert!(xml.contains("rgb=\"FF12AB34\""));
    assert!(xml.contains("codeName=\"SheetCode\""));
    assert!(xml.contains("summaryBelow=\"0\""));
    assert!(xml.find("tabColor").unwrap() < xml.find("outlinePr").unwrap());
    assert!(book.workbook().sheets[0].tab_color.is_some());
    book.set_tab_color(&sheet, None).unwrap();
    assert!(book.workbook().sheets[0].tab_color.is_none());
    let xml = OpcPackage::open(book.bytes())
        .unwrap()
        .required_part_str(part)
        .unwrap();
    assert!(xml.contains("fitToPage=\"1\""));
    let before = book.bytes().to_vec();
    assert!(book.set_tab_color(&sheet, Some("oops")).is_err());
    assert_eq!(book.bytes(), before);
    book.undo().unwrap();
    assert!(book.workbook().sheets[0].tab_color.is_some());
}

#[test]
fn tab_colour_can_create_sheet_properties_and_remove_it_on_prefixed_strict_xml() {
    let xml = r#"<x:worksheet xmlns:x="http://purl.oclc.org/ooxml/spreadsheetml/main"><x:sheetData/></x:worksheet>"#;
    let mut package = OpcPackage::open(&fixture()).unwrap();
    package.replace("xl/worksheets/sheet1.xml", xml.as_bytes().to_vec());
    let bytes = jade_xlsx_writer::sheets::set_tab_color(
        &package.write().unwrap(),
        "売上サマリ",
        Some("ABCDEF"),
    )
    .unwrap();
    let xml = OpcPackage::open(&bytes)
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert!(xml.contains("<x:sheetPr><x:tabColor"));
    assert!(xml.find("sheetPr").unwrap() < xml.find("sheetData").unwrap());
    assert_eq!(
        jade_xlsx_reader::read_xlsx(&bytes).unwrap().sheets[0].tab_color,
        Some(jade_xlsx_reader::Color::Argb([255, 171, 205, 239]))
    );
}
