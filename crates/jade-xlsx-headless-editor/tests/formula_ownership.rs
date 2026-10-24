use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{HeadlessWorkbook, TableOptions, TableUpdate};
use jade_xlsx_reader::{Axis, CellRange};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const FIRST: &str = "xl/worksheets/sheet1.xml";
const NOTES: &str = "xl/worksheets/sheet2.xml";
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn worksheet(body: &str) -> String {
    format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:x="{X14}" xmlns:m="{XM}" xmlns:mc="{MC}" xmlns:u="urn:opaque">{body}</s:worksheet>"#
    )
}
fn open(first: &str, notes: &str) -> HeadlessWorkbook {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(FIRST, worksheet(first).into_bytes());
    p.replace(NOTES, worksheet(notes).into_bytes());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn xml(b: &HeadlessWorkbook, part: &str) -> String {
    OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(part)
        .unwrap()
}
fn table() -> (HeadlessWorkbook, String) {
    let mut b = open("<s:sheetData/>", "<s:sheetData/>");
    let name = b.workbook().sheets[0].name.clone();
    b.set_cells(
        &name,
        &[
            ("H2".into(), CellInput::Text("Item".into())),
            ("I2".into(), CellInput::Text("Qty".into())),
            ("H3".into(), CellInput::Text("A".into())),
            ("I3".into(), CellInput::Number(2.0)),
            ("H4".into(), CellInput::Text("B".into())),
            ("I4".into(), CellInput::Number(3.0)),
        ],
    )
    .unwrap();
    b.create_table(
        &name,
        range("H2:I4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    (b, name)
}
fn replace_notes(b: &HeadlessWorkbook, notes: &str) -> HeadlessWorkbook {
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(NOTES, worksheet(notes).into_bytes());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn history(b: &mut HeadlessWorkbook, before: &[u8]) {
    let after = b.bytes().to_vec();
    let saved = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(xml(&saved, NOTES), xml(b, NOTES));
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(b.bytes(), after);
}

#[test]
fn cross_sheet_partial_shift_updates_only_interpreted_standard_validation_sources() {
    let unknown = r#"<mc:Choice Requires="u"><s:formula1>Notes!$A3</s:formula1></mc:Choice>"#;
    let foreign = r#"<u:formula1><s:formula1>Notes!$A3</s:formula1></u:formula1>"#;
    let unmodeled =
        r#"<s:dataValidation type="whole"><s:formula1>Notes!$A3</s:formula1></s:dataValidation>"#;
    let mut b = open(
        &format!(
            r#"<s:sheetData/><s:dataValidations count="2">{unmodeled}<s:dataValidation type="whole" sqref="B1:B2"><mc:AlternateContent><mc:Choice Requires="s"><s:formula1>Notes!$A3</s:formula1></mc:Choice>{unknown}<mc:Fallback><s:formula1>Notes!$A3</s:formula1></mc:Fallback></mc:AlternateContent>{foreign}</s:dataValidation></s:dataValidations>"#
        ),
        "<s:sheetData/>",
    );
    let before = b.bytes().to_vec();
    b.shift_cells("Notes", range("A2"), Axis::Rows, true)
        .unwrap();
    assert_eq!(
        b.workbook().sheets[0].data_validations[0]
            .formula1
            .as_deref(),
        Some("Notes!$A4")
    );
    let out = xml(&b, FIRST);
    assert_eq!(out.matches("<s:formula1>Notes!$A4</s:formula1>").count(), 2);
    for kept in [unknown, foreign, unmodeled] {
        assert!(out.contains(kept), "{kept}\n{out}");
    }
    let after = b.bytes().to_vec();
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(b.bytes(), after);
}

#[test]
fn cross_sheet_partial_shift_keeps_x14_choice_metadata_and_foreign_wrappers() {
    let opaque = r#"<u:formula1><m:f>Notes!$A3</m:f></u:formula1>"#;
    let unknown = r#"<mc:Choice Requires="u"><m:f>Notes!$A3</m:f></mc:Choice>"#;
    let mut b = open(
        &format!(
            r#"<s:sheetData/><s:extLst><s:ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="whole"><x:formula1><mc:AlternateContent><mc:Choice Requires="x"><m:f>Notes!$A3</m:f></mc:Choice>{unknown}<mc:Fallback><m:f>Notes!$A3</m:f></mc:Fallback></mc:AlternateContent></x:formula1>{opaque}<m:sqref>B1:B2</m:sqref></x:dataValidation></x:dataValidations></s:ext></s:extLst>"#
        ),
        "<s:sheetData/>",
    );
    b.shift_cells("Notes", range("A2"), Axis::Rows, true)
        .unwrap();
    assert_eq!(
        b.workbook().sheets[0].data_validations[0]
            .formula1
            .as_deref(),
        Some("Notes!$A4")
    );
    let out = xml(&b, FIRST);
    assert_eq!(out.matches("<m:f>Notes!$A4</m:f>").count(), 2);
    assert!(out.contains(opaque));
    assert!(out.contains(unknown));
}

#[test]
fn table_rename_updates_sources_without_rewriting_unknown_or_unowned_formula_text() {
    let (b, name) = table();
    let unknown = r#"<mc:Choice Requires="u"><s:formula1>SUM(Sales[Qty])</s:formula1></mc:Choice>"#;
    let foreign = r#"<u:wrapper><s:formula1>SUM(Sales[Qty])</s:formula1><s:f>SUM(Sales[Qty])</s:f></u:wrapper>"#;
    let mut b = replace_notes(
        &b,
        &format!(
            r#"<s:sheetData/><s:dataValidations count="1"><s:dataValidation type="whole" sqref="B1"><mc:AlternateContent><mc:Choice Requires="s"><s:formula1>SUM(Sales[Qty])</s:formula1></mc:Choice>{unknown}<mc:Fallback><s:formula1>SUM(Sales[Qty])</s:formula1></mc:Fallback></mc:AlternateContent></s:dataValidation></s:dataValidations>{foreign}"#
        ),
    );
    let before = b.bytes().to_vec();
    b.update_table(
        &name,
        "Sales",
        &TableUpdate {
            name: Some("Revenue".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let out = xml(&b, NOTES);
    assert_eq!(
        out.matches("<s:formula1>SUM(Revenue[Qty])</s:formula1>")
            .count(),
        2
    );
    assert!(out.contains(unknown));
    assert!(out.contains(foreign));
    history(&mut b, &before);
}

#[test]
fn x14_rules_inside_tables_supply_context_for_unqualified_column_rename() {
    let (b, name) = table();
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    let old = p.required_part_str(FIRST).unwrap();
    let ext = r#"<s:extLst><s:ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="whole"><x:formula1><m:f>[@Qty]</m:f></x:formula1><m:sqref>H3:H4</m:sqref></x:dataValidation></x:dataValidations></s:ext></s:extLst>"#;
    p.replace(
        FIRST,
        old.replace("</s:worksheet>", &format!("{ext}</s:worksheet>"))
            .into_bytes(),
    );
    // The seeded worksheet already has all namespace bindings.
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    b.set(&name, "I2", CellInput::Text("Units".into())).unwrap();
    assert_eq!(
        b.workbook().sheets[0].data_validations[0]
            .formula1
            .as_deref(),
        Some("[@Units]")
    );
}

#[test]
fn implicit_cell_coordinates_keep_current_table_context_during_header_rename() {
    let (b, name) = table();
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    let old = p.required_part_str(FIRST).unwrap();
    let old = old.replace(
        "<s:c r=\"I3\"><s:v>2</s:v></s:c>",
        "<s:c><s:f>[@Qty]</s:f><s:v>2</s:v></s:c>",
    );
    assert!(old.contains("<s:c><s:f>[@Qty]</s:f>"));
    p.replace(FIRST, old.into_bytes());
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    b.set(&name, "I2", CellInput::Text("Units".into())).unwrap();
    assert!(xml(&b, FIRST).contains("<s:f>[@Units]</s:f>"));
}

#[test]
fn conversion_uses_each_x14_rule_anchor_for_current_row_a1_sources() {
    let (b, name) = table();
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    let old = p.required_part_str(FIRST).unwrap();
    let ext = r#"<s:extLst><s:ext uri="dv"><x:dataValidations count="2"><x:dataValidation type="whole"><x:formula1><m:f>[@Qty]</m:f></x:formula1><m:sqref>H3:H4</m:sqref></x:dataValidation><x:dataValidation type="whole"><x:formula1><m:f>Sales[@Qty]</m:f></x:formula1><m:sqref>H4</m:sqref></x:dataValidation></x:dataValidations></s:ext></s:extLst>"#;
    p.replace(
        FIRST,
        old.replace("</s:worksheet>", &format!("{ext}</s:worksheet>"))
            .into_bytes(),
    );
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let before = b.bytes().to_vec();
    b.convert_table_to_range(&name, "Sales").unwrap();
    let rules = b.workbook().sheets[0].data_validations.clone();
    let quote = jade_xlsx_reader::quote_sheet_name(&name);
    assert_eq!(
        rules[0].formula1.as_deref(),
        Some(format!("{quote}!$I3").as_str())
    );
    assert_eq!(
        rules[1].formula1.as_deref(),
        Some(format!("{quote}!$I4").as_str())
    );
    assert!(b.workbook().sheets[0].tables.is_empty());
    let saved = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(saved.workbook().sheets[0].data_validations, rules);
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert!(b.workbook().sheets[0].tables.is_empty());
}

#[test]
fn cross_sheet_conditional_thresholds_and_sparklines_follow_their_owned_formula_paths() {
    let foreign = r#"<u:wrapper><s:formula>Notes!$A3</s:formula><m:f>Notes!$A3</m:f></u:wrapper>"#;
    let body = format!(
        r#"<s:sheetData/><s:conditionalFormatting sqref="B1:B2"><s:cfRule type="expression" priority="1"><s:formula>Notes!$A3</s:formula>{foreign}</s:cfRule><s:cfRule type="dataBar" priority="2"><s:dataBar><s:cfvo type="formula" val="Notes!$A$3"/><s:cfvo type="num" val="3"/><s:color rgb="FF123456"/></s:dataBar></s:cfRule></s:conditionalFormatting><s:extLst><s:ext uri="cf"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="3"><m:f>Notes!$A3</m:f>{foreign}</x:cfRule><m:sqref>C1:C2</m:sqref></x:conditionalFormatting></x:conditionalFormattings></s:ext><s:ext uri="spark"><x:sparklineGroups><x:sparklineGroup><m:f>Notes!$A$3:$A$5</m:f><x:sparklines><x:sparkline><m:f>Notes!$A$3:$A$5</m:f><m:sqref>D1</m:sqref>{foreign}</x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></s:ext></s:extLst>"#
    );
    for main in [MAIN, STRICT] {
        let b = open(&body, "<s:sheetData/>");
        let mut p = OpcPackage::open(b.bytes()).unwrap();
        p.replace(FIRST, xml(&b, FIRST).replace(MAIN, main).into_bytes());
        let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
        b.shift_cells("Notes", range("A2"), Axis::Rows, true)
            .unwrap();
        let out = xml(&b, FIRST);
        assert!(out.contains("val=\"Notes!$A$4\""));
        assert!(out.contains("<s:formula>Notes!$A4</s:formula>"));
        assert!(out.contains("<m:f>Notes!$A4</m:f>"));
        assert_eq!(out.matches("<m:f>Notes!$A$4:$A$6</m:f>").count(), 2);
        assert_eq!(out.matches(foreign).count(), 3);
    }
}

#[test]
fn scoped_sparkline_reading_survives_cross_sheet_edits_save_and_history() {
    let foreign = r#"<u:wrapper><m:f>Notes!$A$3:$A$5</m:f><m:sqref>Z99</m:sqref><x:colorSeries rgb="FFFFFFFF"/></u:wrapper>"#;
    let body = format!(
        r#"<s:sheetData/><s:extLst><s:ext uri="spark"><x:sparklineGroups><x:sparklineGroup type="column"><x:colorSeries rgb="FF123456"/>{foreign}<x:sparklines><x:sparkline><m:f><![CDATA[Notes!$A$3:$A$5]]></m:f><m:sqref><![CDATA[D1:D1]]></m:sqref>{foreign}</x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></s:ext></s:extLst>"#
    );
    for main in [MAIN, STRICT] {
        let b = open(&body, "<s:sheetData/>");
        let mut p = OpcPackage::open(b.bytes()).unwrap();
        p.replace(FIRST, xml(&b, FIRST).replace(MAIN, main).into_bytes());
        let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
        let before = b.bytes().to_vec();
        let before_groups = b.workbook().sheets[0].sparkline_groups.clone();
        assert_eq!(before_groups.len(), 1);
        assert_eq!(before_groups[0].sparklines.len(), 1);
        assert_eq!(before_groups[0].sparklines[0].formula, "Notes!$A$3:$A$5");
        assert_eq!(
            before_groups[0].sparklines[0].cell,
            jade_xlsx_reader::CellRef::new(3, 0)
        );
        assert_eq!(
            before_groups[0].color_series,
            Some(jade_xlsx_reader::Color::Argb([255, 18, 52, 86]))
        );
        b.shift_cells("Notes", range("A2"), Axis::Rows, true)
            .unwrap();
        let after = b.bytes().to_vec();
        let after_groups = b.workbook().sheets[0].sparkline_groups.clone();
        assert_eq!(after_groups[0].sparklines[0].formula, "Notes!$A$4:$A$6");
        assert_eq!(
            after_groups[0].sparklines[0].cell,
            before_groups[0].sparklines[0].cell
        );
        assert_eq!(xml(&b, FIRST).matches(foreign).count(), 2);
        let saved = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
        assert_eq!(saved.workbook().sheets[0].sparkline_groups, after_groups);
        assert_eq!(xml(&saved, FIRST).matches(foreign).count(), 2);
        b.undo().unwrap();
        assert_eq!(b.bytes(), before);
        assert_eq!(b.workbook().sheets[0].sparkline_groups, before_groups);
        b.redo().unwrap();
        assert_eq!(b.bytes(), after);
        assert_eq!(b.workbook().sheets[0].sparkline_groups, after_groups);
    }
}
