//! Preserving fragments retain inherited expanded MCE processing context.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
#[test]
fn inherited_process_content_feature_formulas_follow_structural_edits() {
    for ns in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for mc in ["mc", "compat"] {
            let shadow = "<u:pass><m:f>Sheet1!Z2</m:f></u:pass>";
            let choice = format!("<{mc}:Choice Requires=\"u\"><m:f>Sheet1!Z2</m:f></{mc}:Choice>");
            let opaque = "<ext uri=\"urn:opaque\"><x:sparklineGroups><x:sparklineGroup><v:pass><m:f>Sheet1!Z2</m:f></v:pass></x:sparklineGroup></x:sparklineGroups></ext>";
            let sheet = format!(
                r#"<worksheet xmlns="{ns}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:{mc}="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:one" xmlns:v="urn:one" {mc}:Ignorable="u" {mc}:ProcessContent="u:pass"><sheetData/><conditionalFormatting sqref="B2:B3"><cfRule type="expression" priority="1" xmlns:u="urn:two" {mc}:Ignorable="u"><v:pass><formula>Sheet1!A2&gt;0</formula></v:pass>{shadow}</cfRule></conditionalFormatting><extLst>{opaque}<ext uri="{{78C0D931-6437-407D-A8EE-F0AAD7539E65}}"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="2" xmlns:u="urn:two" {mc}:Ignorable="u"><{mc}:AlternateContent>{choice}<{mc}:Fallback><m:f><v:pass>Sheet1!A2&gt;0</v:pass></m:f></{mc}:Fallback></{mc}:AlternateContent>{shadow}</x:cfRule><m:sqref>B2:B3</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext><ext uri="{{05C60535-1F16-4FD2-B633-F4F36F0B64E0}}"><x:sparklineGroups><x:sparklineGroup dateAxis="1" xmlns:u="urn:two" {mc}:Ignorable="u"><v:pass><m:f>Sheet1!$A$2:$A$3</m:f></v:pass>{shadow}<x:sparklines><x:sparkline><m:f><v:pass>Sheet1!A2:A3</v:pass></m:f><m:sqref>D2</m:sqref>{shadow}</x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst></worksheet>"#
            );
            for (columns, coverage, reference, data, dates, destination) in [
                (false, "B3:B4", "A3", "A3:A4", "$A$3:$A$4", "D3"),
                (true, "C2:C3", "B2", "B2:B3", "$B$2:$B$3", "E2"),
            ] {
                let mut book =
                    HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet))
                        .unwrap();
                let before = book.bytes().to_vec();
                if columns {
                    book.insert_columns("Sheet1", "A", 1).unwrap();
                } else {
                    book.insert_rows("Sheet1", 2, 1).unwrap();
                }
                let shifted = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), before);
                book.redo().unwrap();
                assert_eq!(book.bytes(), shifted);
                book.rename_sheet("Sheet1", "Renamed").unwrap();
                let renamed = book.bytes().to_vec();
                book.undo().unwrap();
                assert_eq!(book.bytes(), shifted);
                book.redo().unwrap();
                assert_eq!(book.bytes(), renamed);
                let saved = book.save_bytes().unwrap();
                let xml = OpcPackage::open(&saved)
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert!(xml.contains(&format!("sqref=\"{coverage}\"")), "{xml}");
                assert!(
                    xml.contains(&format!("<m:sqref>{coverage}</m:sqref>")),
                    "{xml}"
                );
                assert!(
                    xml.contains(&format!("<m:sqref>{destination}</m:sqref>")),
                    "{xml}"
                );
                assert!(
                    xml.contains(&format!("<formula>Renamed!{reference}&gt;0</formula>")),
                    "{xml}"
                );
                assert!(
                    xml.contains(&format!(
                        "<m:f><v:pass>Renamed!{reference}&gt;0</v:pass></m:f>"
                    )),
                    "{xml}"
                );
                assert!(
                    xml.contains(&format!("<m:f><v:pass>Renamed!{data}</v:pass></m:f>")),
                    "{xml}"
                );
                assert!(
                    xml.contains(&format!("<m:f>Renamed!{dates}</m:f>")),
                    "{xml}"
                );
                assert_eq!(
                    xml.matches(shadow).count(),
                    sheet.matches(shadow).count(),
                    "{xml}"
                );
                assert!(xml.contains(&choice), "{xml}");
                assert!(xml.contains(opaque), "{xml}");
                assert!(!xml.contains("_jade_ctx"), "{xml}");
                assert_eq!(xml.matches(":ProcessContent=").count(), 1, "{xml}");
                let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
                let again = OpcPackage::open(&reopened.save_bytes().unwrap())
                    .unwrap()
                    .required_part_str("xl/worksheets/sheet1.xml")
                    .unwrap();
                assert_eq!(again, xml);
            }
        }
    }
}
