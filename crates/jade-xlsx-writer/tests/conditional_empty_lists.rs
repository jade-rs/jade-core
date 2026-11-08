//! Removing the last conditional block must not leave an invalid empty x14 list.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const URI: &str = "{78C0D931-6437-407D-A8EE-F0AAD7539E65}";
const RULE: &str = r#"<x:conditionalFormatting><x:cfRule type="expression" priority="1"><m:f>1</m:f></x:cfRule><m:sqref>B2</m:sqref></x:conditionalFormatting>"#;
fn sheet(ns: &str, tail: &str) -> Vec<u8> {
    common::workbook_with_sheet(&format!(
        r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="u" mc:ProcessContent="u:pass"><sheetData/>{tail}</worksheet>"#
    ))
}
fn xml(book: &mut HeadlessWorkbook) -> String {
    OpcPackage::open(&book.save_bytes().unwrap())
        .unwrap()
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap()
}
#[test]
fn deleting_last_conditional_coverage_removes_empty_owned_lists() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        for columns in [false, true] {
            let tail = format!("<extLst><ext uri=\"{URI}\"><x:conditionalFormattings>{RULE}</x:conditionalFormattings></ext></extLst>");
            let mut book = HeadlessWorkbook::open_for_editing(&sheet(ns, &tail)).unwrap();
            let before = book.bytes().to_vec();
            if columns {
                book.delete_columns("Sheet1", "B", 1).unwrap();
            } else {
                book.delete_rows("Sheet1", 2, 1).unwrap();
            }
            let after = book.bytes().to_vec();
            book.undo().unwrap();
            assert_eq!(book.bytes(), before);
            book.redo().unwrap();
            assert_eq!(book.bytes(), after);
            let saved = xml(&mut book);
            assert!(!saved.contains("conditionalFormattings"), "{saved}");
            assert!(!saved.contains("<extLst"), "{saved}");
            let mut reopened =
                HeadlessWorkbook::open_for_editing(&book.save_bytes().unwrap()).unwrap();
            assert_eq!(xml(&mut reopened), saved);
        }
    }
}
#[test]
fn cleanup_preserves_foreign_data_comments_and_preexisting_empty_lists() {
    let unknown = format!("<ext uri=\"urn:unknown\"><x:conditionalFormattings>{RULE}</x:conditionalFormattings></ext>");
    let foreign = "<u:conditionalFormattings><m:sqref>B2</m:sqref></u:conditionalFormattings>";
    let tail = format!("<extLst>{unknown}<ext uri=\"{URI}\"><x:conditionalFormattings/>{foreign}<x:conditionalFormattings><!--keep-->{RULE}</x:conditionalFormattings><x:conditionalFormattings>{RULE}<u:data>B2</u:data></x:conditionalFormattings><x:conditionalFormattings>{RULE}</x:conditionalFormattings></ext></extLst>");
    let mut book = HeadlessWorkbook::open_for_editing(&sheet(MAIN, &tail)).unwrap();
    book.delete_rows("Sheet1", 2, 1).unwrap();
    let saved = xml(&mut book);
    assert!(saved.contains(&unknown), "{saved}");
    assert!(saved.contains(foreign), "{saved}");
    assert!(saved.contains("<x:conditionalFormattings/>"), "{saved}");
    assert!(
        saved.contains("<x:conditionalFormattings><!--keep--></x:conditionalFormattings>"),
        "{saved}"
    );
    assert!(
        saved.contains("<x:conditionalFormattings><u:data>B2</u:data></x:conditionalFormattings>"),
        "{saved}"
    );
    assert!(
        !saved.contains("<x:conditionalFormattings></x:conditionalFormattings>"),
        "{saved}"
    );
}
#[test]
fn cleanup_follows_process_content_ownership_without_removing_wrappers() {
    let tail = format!("<extLst><ext uri=\"{URI}\"><u:pass><x:conditionalFormattings>{RULE}</x:conditionalFormattings></u:pass><u:data>B2</u:data></ext></extLst>");
    let mut book = HeadlessWorkbook::open_for_editing(&sheet(MAIN, &tail)).unwrap();
    book.delete_rows("Sheet1", 2, 1).unwrap();
    let saved = xml(&mut book);
    assert!(!saved.contains("conditionalFormattings"), "{saved}");
    assert!(
        saved.contains("<u:pass></u:pass><u:data>B2</u:data>"),
        "{saved}"
    );
}
