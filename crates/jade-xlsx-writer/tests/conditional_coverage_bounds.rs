//! Insertion retains conditional coverage that still fits within Excel's grid.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;

#[test]
fn conditional_coverage_keeps_surviving_cells_at_row_and_column_limits() {
    for (column, at, coverage, expected) in [
        (true, "XFD", "XFC1:XFD2", "XFC1:XFD2"),
        (true, "XFC", "XFC1:XFD2", "XFD1:XFD2"),
        (false, "1048576", "B1048575:C1048576", "B1048575:C1048576"),
        (false, "1048575", "B1048575:C1048576", "B1048576:C1048576"),
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><sheetData/><conditionalFormatting sqref="{coverage}"><cfRule type="expression" priority="1"><formula>Sheet1!$A$1&gt;0</formula></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407D-A8EE-F0AAD7539E65}}"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="2"><m:f>Sheet1!$A$1&gt;0</m:f></x:cfRule><m:sqref>{coverage}</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext></extLst></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
        let before = book.bytes().to_vec();
        if column {
            book.insert_columns("Sheet1", at, 1).unwrap();
        } else {
            book.insert_rows("Sheet1", at.parse().unwrap(), 1).unwrap();
        }
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
        assert!(
            xml.contains(&format!("sqref=\"{expected}\"")),
            "{coverage} at {at}: {xml}"
        );
        assert!(
            xml.contains(&format!("<m:sqref>{expected}</m:sqref>")),
            "{coverage} at {at}: {xml}"
        );
        assert!(xml.contains("<formula>Sheet1!$A$1&gt;0</formula>"));
        assert!(xml.contains("<m:f>Sheet1!$A$1&gt;0</m:f>"));
        let mut reopened = HeadlessWorkbook::open(&saved).unwrap();
        let reexported = reopened.save_bytes().unwrap();
        let xml = OpcPackage::open(&reexported)
            .unwrap()
            .required_part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        assert!(xml.contains(&format!("sqref=\"{expected}\"")));
        assert!(xml.contains(&format!("<m:sqref>{expected}</m:sqref>")));
    }
}
