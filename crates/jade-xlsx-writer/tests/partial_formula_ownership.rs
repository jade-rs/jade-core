//! Feature-internal opaque formula-looking data is not a geometry source.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const OPAQUE: &str = r#"<u:payload><m:f>Sheet1!B2</m:f></u:payload>"#;
fn open(ns: &str, uri: &str, payload: &str) -> HeadlessWorkbook {
    let sheet = format!(
        r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign"><sheetData/><extLst><ext uri="{uri}">{payload}</ext></extLst></worksheet>"#
    );
    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap()
}
fn insert(book: &mut HeadlessWorkbook, axis: Axis) -> String {
    let before = book.bytes().to_vec();
    book.shift_cells("Sheet1", CellRange::parse("B2").unwrap(), axis, true)
        .unwrap();
    let after = book.bytes().to_vec();
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    assert_eq!(book.bytes(), after);
    let saved = book.save_bytes().unwrap();
    let xml = OpcPackage::open(&saved)
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    let roundtrip = OpcPackage::open(&reopened.save_bytes().unwrap())
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert_eq!(xml, roundtrip);
    assert!(xml.contains(OPAQUE), "{xml}");
    assert_eq!(
        xml.matches("<u:payload>").count(),
        xml.matches(OPAQUE).count(),
        "{xml}"
    );
    xml
}
#[test]
fn partial_sparkline_shift_does_not_rewrite_opaque_formula_leaves() {
    for ns in [MAIN, STRICT] {
        for (axis, expected, absolute) in
            [(Axis::Rows, "B3", "$B$3"), (Axis::Columns, "C2", "$C$2")]
        {
            let payload=format!("<x:sparklineGroups><x:sparklineGroup><m:f>Sheet1!$B$2</m:f>{OPAQUE}<x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref>{OPAQUE}</x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups>");
            let mut book = open(ns, "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}", &payload);
            let xml = insert(&mut book, axis);
            assert!(
                xml.contains(&format!("<m:sqref>{expected}</m:sqref>")),
                "{xml}"
            );
            assert!(
                xml.contains(&format!("<m:f>Sheet1!{absolute}</m:f>")),
                "{xml}"
            );
            assert!(xml.contains("<m:f>Sheet1!A1:A3</m:f>"), "{xml}");
        }
    }
}
#[test]
fn partial_conditional_rules_and_thresholds_use_owned_formula_paths() {
    for ns in [MAIN, STRICT] {
        for (axis, expected) in [(Axis::Rows, "$B$3"), (Axis::Columns, "$C$2")] {
            for content in ["<m:f>Sheet1!$B$2</m:f>","<x:dataBar><x:cfvo type=\"formula\"><m:f>Sheet1!$B$2</m:f></x:cfvo><x:cfvo type=\"max\"/></x:dataBar>"] {
                let kind=if content.starts_with("<m:f>") {"expression"}else{"dataBar"};
                let payload=format!("<x:conditionalFormattings><x:conditionalFormatting><x:cfRule type=\"{kind}\" priority=\"1\">{content}{OPAQUE}</x:cfRule><m:sqref>B1:B3</m:sqref></x:conditionalFormatting></x:conditionalFormattings>");
                let mut book=open(ns,"{78C0D931-6437-407D-A8EE-F0AAD7539E65}",&payload);
                let xml=insert(&mut book,axis);
                assert!(xml.contains(&format!("<m:f>Sheet1!{expected}</m:f>")),"{xml}");
            }
        }
    }
}
