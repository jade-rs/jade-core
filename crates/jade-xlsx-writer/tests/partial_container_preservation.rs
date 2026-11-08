//! Deleting supported records must retain their unprocessed sibling data.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange};
#[test]
fn deleting_last_partial_rule_preserves_opaque_container_siblings() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for axis in [Axis::Rows, Axis::Columns] {
            for (uri, payload) in [
                (
                    "{78C0D931-6437-407D-A8EE-F0AAD7539E65}",
                    r#"<x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="1"><m:f>1</m:f></x:cfRule><m:sqref>B2</m:sqref></x:conditionalFormatting><u:data>B2</u:data><!--keep--></x:conditionalFormattings>"#,
                ),
                (
                    "{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}",
                    r#"<x:dataValidations count="1"><x:dataValidation type="custom"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>B2</m:sqref></x:dataValidation><u:data>B2</u:data><!--keep--></x:dataValidations>"#,
                ),
                (
                    "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}",
                    r#"<x:sparklineGroups><x:sparklineGroup><x:sparklines><x:sparkline><m:f>Sheet1!A1:A3</m:f><m:sqref>B2</m:sqref></x:sparkline><u:data>B2</u:data><!--keep--></x:sparklines></x:sparklineGroup></x:sparklineGroups>"#,
                ),
            ] {
                let sheet = format!(
                    r#"<worksheet xmlns="{namespace}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:foreign"><sheetData/><extLst><ext uri="{uri}">{payload}</ext></extLst></worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet))
                        .unwrap();
                let before = book.bytes().to_vec();
                book.shift_cells("Sheet1", CellRange::parse("B2").unwrap(), axis, false)
                    .unwrap();
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let xml = OpcPackage::open(&book.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert!(
                    xml.contains("<u:data>B2</u:data><!--keep-->"),
                    "{uri}: {xml}"
                );
                assert!(!xml.contains("<m:sqref>B2</m:sqref>"), "{uri}: {xml}");
                let mut reopened =
                    HeadlessWorkbook::open_for_editing(&book.save_bytes().unwrap()).unwrap();
                let saved = OpcPackage::open(&reopened.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert_eq!(saved, xml);
            }
        }
    }
}

#[test]
fn deleting_last_merge_keeps_opaque_container_data() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for axis in [Axis::Rows, Axis::Columns] {
            for (attrs, opaque) in [
                ("", ""),
                (" u:flag=\"keep\"", ""),
                ("", "<!--keep-->"),
                ("", "<u:payload>B2:C2</u:payload>"),
                ("", "<u:mergeCell ref=\"B2:C2\"/>"),
            ] {
                let sheet = format!(
                    r#"<worksheet xmlns="{namespace}" xmlns:u="urn:foreign"><sheetData/><mergeCells count="1"{attrs}><mergeCell ref="B2:C2"/>{opaque}</mergeCells></worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet))
                        .unwrap();
                let before = book.bytes().to_vec();
                book.shift_cells("Sheet1", CellRange::parse("B2:C2").unwrap(), axis, false)
                    .unwrap();
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
                if attrs.is_empty() && opaque.is_empty() {
                    assert!(!saved.contains("mergeCells"), "{saved}");
                } else {
                    assert!(
                        saved.contains(&format!(
                            "<mergeCells count=\"0\"{attrs}>{opaque}</mergeCells>"
                        )),
                        "{saved}"
                    );
                }
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
