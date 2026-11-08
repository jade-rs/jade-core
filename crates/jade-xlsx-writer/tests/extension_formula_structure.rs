//! Genuine extension formulas follow whole-axis geometry without becoming grid cells.
mod common;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellRef;

#[test]
fn conditional_sources_follow_compatible_parents_and_keep_ignored_parents_exact() {
    let opaque = r#"<u:opaque><conditionalFormatting sqref="Z2:Z4"><cfRule type="expression" priority="9"><formula>Sheet1!Z2&gt;0</formula></cfRule></conditionalFormatting></u:opaque>"#;
    let unknown = r#"<mc:Choice Requires="u"><conditionalFormatting sqref="Y2:Y4"><cfRule type="expression" priority="8"><formula>Sheet1!Y2&gt;0</formula></cfRule></conditionalFormatting></mc:Choice>"#;
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:s="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future" mc:Ignorable="u" mc:ProcessContent="u:wrap"><sheetData/><u:wrap><conditionalFormatting sqref="D2:D4"><cfRule type="expression" priority="1"><formula>Sheet1!A2&gt;0</formula></cfRule></conditionalFormatting></u:wrap>{opaque}<mc:AlternateContent>{unknown}<mc:Choice Requires="s"><conditionalFormatting sqref="E2:E4"><cfRule type="expression" priority="2"><formula>Sheet1!B2&gt;0</formula></cfRule></conditionalFormatting></mc:Choice><mc:Fallback><conditionalFormatting sqref="F2:F4"><cfRule type="expression" priority="3"><formula>Sheet1!C2&gt;0</formula></cfRule></conditionalFormatting></mc:Fallback></mc:AlternateContent></worksheet>"#
    );
    let mut book =
        HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
    book.insert_rows("Sheet1", 1, 1).unwrap();
    let saved = book.save_bytes().unwrap();
    let package = OpcPackage::open(&saved).unwrap();
    let xml = package
        .required_part_str("xl/worksheets/sheet1.xml")
        .unwrap();
    assert!(xml.contains(opaque), "{xml}");
    assert!(xml.contains(unknown), "{xml}");
    for column in ["A", "B", "C"] {
        assert!(
            xml.contains(&format!("<formula>Sheet1!{column}3&gt;0</formula>")),
            "{xml}"
        );
    }
    for column in ["D", "E", "F"] {
        assert!(
            xml.contains(&format!("sqref=\"{column}3:{column}5\"")),
            "{xml}"
        );
    }
}

#[test]
fn deleting_the_first_covered_cells_reanchors_relative_conditional_formulas() {
    for (rows, coverage, expected_range, expected_formula) in [
        (true, "D1:D3", "D1:D2", "Sheet1!A1&gt;0"),
        (false, "A4:C4", "A4:B4", "Sheet1!A1&gt;0"),
        (true, "D1 D3:D4", "D2:D3", "Sheet1!A2&gt;0"),
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><sheetData/><conditionalFormatting sqref="{coverage}"><cfRule type="expression" priority="1"><formula>Sheet1!A1&gt;0</formula></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407D-A8EE-F0AAD7539E65}}"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="2"><m:f>Sheet1!A1&gt;0</m:f></x:cfRule><m:sqref>{coverage}</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext></extLst></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
        if rows {
            book.delete_rows("Sheet1", 1, 1).unwrap();
        } else {
            book.delete_columns("Sheet1", "A", 1).unwrap();
        }
        let saved = book.save_bytes().unwrap();
        let package = OpcPackage::open(&saved).unwrap();
        let xml = package
            .required_part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        assert!(
            xml.contains(&format!("<formula>{expected_formula}</formula>")),
            "{xml}"
        );
        assert!(
            xml.contains(&format!("<m:f>{expected_formula}</m:f>")),
            "{xml}"
        );
        assert!(
            xml.contains(&format!("sqref=\"{expected_range}\"")),
            "{xml}"
        );
        assert!(
            xml.contains(&format!("<m:sqref>{expected_range}</m:sqref>")),
            "{xml}"
        );
    }
}

#[test]
fn edits_on_another_sheet_move_qualified_sparkline_sources_and_keep_local_ones() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><sheetData/><extLst><ext uri="{05C60535-1F16-4FD2-B633-F4F36F0B64E0}"><x:sparklineGroups><x:sparklineGroup dateAxis="1"><m:f>Other!A2:A4</m:f><x:sparklines><x:sparkline><m:f>Other!B2:B4</m:f><m:sqref>D2</m:sqref></x:sparkline><x:sparkline><m:f>B2:B4</m:f><m:sqref>D3</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst></worksheet>"#;
    let mut book = HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(sheet)).unwrap();
    book.add_sheet("Other", None).unwrap();
    book.insert_rows("Other", 1, 1).unwrap();
    book.insert_columns("Other", "A", 1).unwrap();
    let group = &book.workbook().sheets[0].sparkline_groups[0];
    assert_eq!(group.date_range.as_deref(), Some("Other!B3:B5"));
    assert_eq!(group.sparklines[0].formula, "Other!C3:C5");
    assert_eq!(group.sparklines[1].formula, "B2:B4");
    assert_eq!(group.sparklines[0].cell, CellRef::parse("D2").unwrap());
    assert_eq!(group.sparklines[1].cell, CellRef::parse("D3").unwrap());
    book.delete_rows("Other", 1, 1).unwrap();
    book.delete_columns("Other", "A", 1).unwrap();
    let group = &book.workbook().sheets[0].sparkline_groups[0];
    assert_eq!(group.date_range.as_deref(), Some("Other!A2:A4"));
    assert_eq!(group.sparklines[0].formula, "Other!B2:B4");
    assert_eq!(group.sparklines[1].formula, "B2:B4");
    let reopened = HeadlessWorkbook::open(&book.save_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.workbook().sheets[0].sparkline_groups,
        book.workbook().sheets[0].sparkline_groups
    );
}

#[test]
fn legacy_and_extended_conditional_thresholds_follow_geometry_exactly_once() {
    for rows in [true, false] {
        let opaque = r#"<ext uri="urn:opaque"><m:f>Sheet1!$Z$2</m:f></ext>"#;
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main"><sheetData/><conditionalFormatting sqref="D2:D4"><cfRule type="expression" priority="1"><formula>Sheet1!A2&gt;0</formula></cfRule><cfRule type="dataBar" priority="2"><dataBar><cfvo type="formula" val="Sheet1!$A$2"/><cfvo type="max"/><color rgb="FF123456"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}"><x:id>{{AAAAAAAA-0000-4000-8000-000000000001}}</x:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407D-A8EE-F0AAD7539E65}}"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="dataBar" id="{{AAAAAAAA-0000-4000-8000-000000000001}}"><x:dataBar><x:cfvo type="formula"><m:f>Sheet1!$A$2</m:f></x:cfvo><x:cfvo type="autoMax"/></x:dataBar></x:cfRule><m:sqref>D2:D4</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext>{opaque}</extLst></worksheet>"#
        );
        let mut book =
            HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
        let (bound, formula, range) = if rows {
            book.insert_rows("Sheet1", 1, 1).unwrap();
            ("Sheet1!$A$3", "Sheet1!A3&gt;0", "D3:D5")
        } else {
            book.insert_columns("Sheet1", "A", 1).unwrap();
            ("Sheet1!$B$2", "Sheet1!B2&gt;0", "E2:E4")
        };
        let saved = book.save_bytes().unwrap();
        let package = OpcPackage::open(&saved).unwrap();
        let xml = package
            .required_part_str("xl/worksheets/sheet1.xml")
            .unwrap();
        assert!(xml.contains(&format!("<m:f>{bound}</m:f>")), "{xml}");
        assert!(xml.contains(&format!("val=\"{bound}\"")), "{xml}");
        assert!(
            xml.contains(&format!("<formula>{formula}</formula>")),
            "{xml}"
        );
        assert!(xml.contains(&format!("sqref=\"{range}\"")), "{xml}");
        assert!(
            xml.contains(&format!("<m:sqref>{range}</m:sqref>")),
            "{xml}"
        );
        assert!(xml.contains(opaque));
    }
}

#[test]
fn sparkline_data_and_date_formulas_follow_rows_and_columns_with_history() {
    for main in [
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        "http://purl.oclc.org/ooxml/spreadsheetml/main",
    ] {
        for rows in [true, false] {
            let opaque = r#"<u:payload><m:f>Sheet1!Z2</m:f></u:payload>"#;
            let sheet = format!(
                r#"<worksheet xmlns="{main}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:u="urn:opaque"><sheetData/><extLst><ext uri="{{05C60535-1F16-4FD2-B633-F4F36F0B64E0}}"><x:sparklineGroups><x:sparklineGroup dateAxis="1"><m:f>Sheet1!A2:A4</m:f>{opaque}<x:sparklines><x:sparkline><m:f>Sheet1!B2:B4</m:f><m:sqref>D2</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst></worksheet>"#
            );
            let mut book =
                HeadlessWorkbook::open_for_editing(&common::workbook_with_sheet(&sheet)).unwrap();
            assert_eq!(book.workbook().sheets[0].sparkline_groups.len(), 1);
            let before = book.bytes().to_vec();
            let (date, data, cell) = if rows {
                book.insert_rows("Sheet1", 1, 1).unwrap();
                ("Sheet1!A3:A5", "Sheet1!B3:B5", "D3")
            } else {
                book.insert_columns("Sheet1", "A", 1).unwrap();
                ("Sheet1!B2:B4", "Sheet1!C2:C4", "E2")
            };
            let group = &book.workbook().sheets[0].sparkline_groups[0];
            assert_eq!(group.sparklines[0].cell, CellRef::parse(cell).unwrap());
            assert_eq!(group.sparklines[0].formula, data);
            assert_eq!(group.date_range.as_deref(), Some(date));
            let after = book.bytes().to_vec();
            book.undo().unwrap();
            assert_eq!(book.bytes(), before);
            book.redo().unwrap();
            assert_eq!(book.bytes(), after);
            let saved = book.save_bytes().unwrap();
            let package = OpcPackage::open(&saved).unwrap();
            assert!(package
                .required_part_str("xl/worksheets/sheet1.xml")
                .unwrap()
                .contains(opaque));
            let reopened = HeadlessWorkbook::open(&saved).unwrap();
            let group = &reopened.workbook().sheets[0].sparkline_groups[0];
            assert_eq!(group.date_range.as_deref(), Some(date));
            assert_eq!(group.sparklines[0].formula, data);
        }
    }
}
