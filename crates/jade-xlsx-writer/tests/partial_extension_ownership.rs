//! Unprocessed worksheet extensions are not partial-cell geometry sources.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange};

#[test]
fn partial_cell_insertion_preserves_unknown_extensions_and_moves_genuine_rules() {
    for namespace in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for uri in ["urn:unprocessed", "{05C60535-1F16-4FD2-B633-F4F36F0B64E0}"] {
            for axis in [Axis::Rows, Axis::Columns] {
                let opaque = r#"<ext uri="urn:unprocessed"><x:dataValidations count="1"><x:dataValidation type="custom"><x:formula1><m:f>Sheet1!A1&gt;0</m:f></x:formula1><m:sqref>B1:B3</m:sqref></x:dataValidation></x:dataValidations><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="42"><m:f>Sheet1!B1&gt;0</m:f></x:cfRule><m:sqref>B1:B3</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext>"#;
                let opaque = opaque.replace("urn:unprocessed", uri);
                let sheet = format!(
                    r#"<worksheet xmlns="{namespace}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><sheetData/><extLst>{opaque}<ext uri="{{CCE6A557-97BC-4B89-ADB6-D9C93CAAB3DF}}"><x:dataValidations count="1"><x:dataValidation type="custom"><x:formula1><m:f>$A1&gt;0</m:f></x:formula1><m:sqref>B1:B3</m:sqref></x:dataValidation></x:dataValidations></ext></extLst></worksheet>"#
                );
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet))
                        .unwrap();
                assert_eq!(book.workbook().sheets[0].data_validations.len(), 1);
                let before = book.bytes().to_vec();
                book.shift_cells("Sheet1", CellRange::parse("B2").unwrap(), axis, true)
                    .unwrap();
                assert!(book.workbook().sheets[0].data_validations.len() > 1);
                let after = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), after);
                let xml = OpcPackage::open(&book.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert!(xml.contains(&opaque), "{xml}");
                let mut reopened =
                    HeadlessWorkbook::open_for_editing(&book.save_bytes().unwrap()).unwrap();
                let saved = OpcPackage::open(&reopened.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert!(saved.contains(&opaque), "{saved}");
            }
        }
    }
}
