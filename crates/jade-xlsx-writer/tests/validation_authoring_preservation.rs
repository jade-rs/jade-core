mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
#[test]
fn deleting_last_rule_keeps_opaque_container_data_and_history() {
    for ns in [MAIN, STRICT] {
        for extended in [false, true] {
            for (attrs, opaque) in [
                ("", "<u:payload>A2</u:payload>"),
                (" u:flag=\"keep\"", ""),
                ("", "<!--keep-->"),
                ("", "<u:dataValidation sqref=\"A2\"/>"),
            ] {
                let (prefix, rule) = if extended {
                    ("x:", "<x:dataValidation type=\"whole\"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>B2</m:sqref></x:dataValidation>")
                } else {
                    ("", "<dataValidation type=\"whole\" sqref=\"B2\"><formula1>1</formula1></dataValidation>")
                };
                let list = format!("<{prefix}dataValidations count=\"1\"{attrs}>{rule}{opaque}</{prefix}dataValidations>");
                let tail = if extended {
                    format!("<extLst><ext uri=\"{{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}}\">{list}</ext></extLst>")
                } else {
                    list
                };
                let xml = format!(
                    r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign"><sheetData/>{tail}</worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&xml)).unwrap();
                let before = book.bytes().to_vec();
                book.delete_data_validation("Sheet1", 0).unwrap();
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let bytes = book.save_bytes().unwrap();
                let saved = OpcPackage::open(&bytes)
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert!(saved.contains(&format!("<{prefix}dataValidations count=\"0\"{attrs}>{opaque}</{prefix}dataValidations>")), "{saved}");
                let mut reopened = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
                let saved_again = OpcPackage::open(&reopened.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert_eq!(saved_again, saved);
            }
        }
    }
}
