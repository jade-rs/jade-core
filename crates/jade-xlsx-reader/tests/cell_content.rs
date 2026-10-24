mod common;

use jade_xlsx_reader::{read_xlsx, Cell, CellRef, CellValue, OpcPackage, Sheet};
use std::collections::BTreeSet;

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
fn worksheet(body: &str) -> String {
    format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:mc="{MC}" xmlns:u="urn:foreign">{body}</s:worksheet>"#
    )
}
fn pool(body: &str) -> String {
    format!(r#"<s:sst xmlns:s="{MAIN}" xmlns:mc="{MC}" xmlns:u="urn:foreign">{body}</s:sst>"#)
}
fn package(xml: &str, shared: &str) -> Vec<u8> {
    let mut p = OpcPackage::open(&common::sample_workbook()).unwrap();
    p.replace("xl/worksheets/sheet1.xml", xml.as_bytes().to_vec());
    p.replace("xl/sharedStrings.xml", shared.as_bytes().to_vec());
    p.write().unwrap()
}
fn parse(xml: &str) -> Sheet {
    jade_xlsx_reader::sheet::parse_sheet("Sheet", "sheet.xml", xml, &[]).unwrap()
}
fn at<'a>(sheet: &'a Sheet, reference: &str) -> &'a Cell {
    let reference = CellRef::parse(reference).unwrap();
    sheet
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == reference)
        .unwrap()
}

#[test]
fn foreign_and_misplaced_cells_do_not_replace_the_grid_or_consume_implicit_addresses() {
    let xml = worksheet(
        r#"<u:sheetData><s:row r="9"><s:c r="A9"><s:v>999</s:v></s:c></s:row></u:sheetData><s:sheetData><u:row r="8"><s:c r="A8"><s:v>888</s:v></s:c></u:row><s:row><u:c r="Z99"><u:v>999</u:v></u:c><s:c><s:v>2</s:v></s:c><u:wrapper><s:c r="garbage"><s:v>999</s:v></s:c></u:wrapper><s:c><s:v>3</s:v></s:c></s:row></s:sheetData><s:c r="A20"><s:v>200</s:v></s:c>"#,
    );
    let s = parse(&xml);
    assert_eq!(s.rows.len(), 1);
    assert_eq!(s.rows[0].cells.len(), 2);
    assert_eq!(at(&s, "A1").value, CellValue::Number(2.0));
    assert_eq!(at(&s, "B1").value, CellValue::Number(3.0));
}

#[test]
fn foreign_leaves_and_nested_text_do_not_supply_cached_values_or_formulas() {
    let s = parse(&worksheet(
        r#"<s:sheetData><s:row r="1"><s:c r="A1"><s:f>1+<u:f>99</u:f>2</s:f><s:v>3<u:v>99</u:v></s:v><u:f>100+200</u:f><u:v>999</u:v><u:wrapper><s:f>400+500</s:f><s:v>999</s:v></u:wrapper></s:c></s:row></s:sheetData>"#,
    ));
    assert_eq!(at(&s, "A1").formula.as_deref(), Some("1+2"));
    assert_eq!(at(&s, "A1").value, CellValue::Number(3.0));
}

#[test]
fn namespaced_attributes_cannot_impersonate_cell_row_or_column_settings() {
    let s = parse(&worksheet(
        r#"<s:cols><s:col min="1" max="1" width="12" u:width="99" u:hidden="1"/></s:cols><s:sheetData><s:row r="1" ht="18" u:r="9" u:ht="99" u:hidden="1"><s:c u:r="Z9" r="A1" u:t="str" t="n" u:s="99" s="1"><s:v>7</s:v></s:c></s:row></s:sheetData>"#,
    ));
    assert_eq!(s.rows[0].index, 0);
    assert_eq!(s.rows[0].height_points, Some(18.0));
    assert!(!s.rows[0].hidden);
    assert_eq!(s.columns[0].width_chars, Some(12.0));
    assert!(!s.columns[0].hidden);
    assert_eq!(at(&s, "A1").style_index, Some(1));
    assert_eq!(at(&s, "A1").value, CellValue::Number(7.0));
}

#[test]
fn the_first_supported_choice_supplies_a_cells_formula_and_cached_value() {
    let s = parse(&worksheet(
        r#"<s:sheetData><s:row r="1"><s:c r="A1"><mc:AlternateContent><mc:Choice Requires="u"><s:f>100+200</s:f><s:v>300</s:v></mc:Choice><mc:Choice Requires="s"><s:f><![CDATA[1+2]]></s:f><s:v>3</s:v></mc:Choice><mc:Choice Requires="s"><s:f>4+5</s:f><s:v>9</s:v></mc:Choice><mc:Fallback><s:f>40+50</s:f><s:v>90</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#,
    ));
    assert_eq!(at(&s, "A1").formula.as_deref(), Some("1+2"));
    assert_eq!(at(&s, "A1").value, CellValue::Number(3.0));
}

#[test]
fn requires_uses_locally_resolved_namespaces_and_all_required_prefixes() {
    let xml = worksheet(r#"<s:sheetData><s:row r="1"><s:c r="A1" t="inlineStr"><s:is><mc:AlternateContent><mc:Choice Requires="s u"><s:t>wrong1</s:t></mc:Choice><mc:Choice xmlns:q="urn:foreign" Requires="q"><s:t>wrong2</s:t></mc:Choice><mc:Choice Requires="missing"><s:t>wrong3</s:t></mc:Choice><mc:Fallback><s:t>right &amp; <![CDATA[kept]]></s:t></mc:Fallback></mc:AlternateContent></s:is></s:c></s:row></s:sheetData>"#).replace(MAIN, STRICT);
    assert_eq!(
        at(&parse(&xml), "A1").value,
        CellValue::Text("right & kept".into())
    );
}

#[test]
fn row_and_cell_alternatives_select_one_grid_and_leave_implicit_coordinates_consistent() {
    let s = parse(&worksheet(
        r#"<s:sheetData><mc:AlternateContent><mc:Choice Requires="u"><s:row r="9"><s:c r="Z9"><s:v>99</s:v></s:c></s:row></mc:Choice><mc:Fallback><s:row><mc:AlternateContent><mc:Choice Requires="s"><s:c><s:v>2</s:v></s:c></mc:Choice><mc:Fallback><s:c><s:v>99</s:v></s:c></mc:Fallback></mc:AlternateContent><s:c><s:v>3</s:v></s:c></s:row></mc:Fallback></mc:AlternateContent><s:row><s:c><s:v>4</s:v></s:c></s:row></s:sheetData>"#,
    ));
    assert_eq!(s.rows.len(), 2);
    assert_eq!(s.rows[0].cells.len(), 2);
    assert_eq!(at(&s, "A1").value, CellValue::Number(2.0));
    assert_eq!(at(&s, "B1").value, CellValue::Number(3.0));
    assert_eq!(at(&s, "A2").value, CellValue::Number(4.0));
}

#[test]
fn empty_and_foreign_shared_items_preserve_indices_and_match_rich_text() {
    let xml = pool(
        r#"<s:si/><u:si><u:t>foreign</u:t></u:si><s:si><mc:AlternateContent><mc:Choice Requires="s"><s:r><s:rPr><s:b/></s:rPr><s:t>kept</s:t></s:r></mc:Choice><mc:Fallback><s:r><s:t>wrong</s:t></s:r></mc:Fallback></mc:AlternateContent></s:si><u:wrapper><s:si><s:t>wrapped</s:t></s:si></u:wrapper><s:si><s:t>last</s:t></s:si>"#,
    );
    let values = jade_xlsx_reader::shared_strings::parse_shared_strings(&xml).unwrap();
    let runs = jade_xlsx_reader::rich_text::parse_shared_runs(&xml).unwrap();
    assert_eq!(values, ["", "kept", "last"]);
    assert_eq!(runs.len(), values.len());
    assert!(runs[0].is_none() && runs[2].is_none());
    let kept = runs[1].as_ref().unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].text, values[1]);
    assert_eq!(kept[0].font.bold, Some(true));
}

#[test]
fn inline_values_and_runs_ignore_foreign_fonts_and_share_inferred_cell_addresses() {
    let xml = worksheet(
        r#"<s:sheetData><s:row r="3"><s:c t="inlineStr"><s:is><s:r><s:rPr><s:b val="0" u:val="1"/><u:wrapper><s:i/></u:wrapper></s:rPr><s:t>hello<u:t>bad</u:t> &amp; <![CDATA[world]]></s:t></s:r><u:r><s:t>bad2</s:t></u:r></s:is></s:c><s:c t="s"><s:v><![CDATA[0]]></s:v></s:c></s:row></s:sheetData>"#,
    );
    let bytes = package(&xml, &pool("<s:si><s:r><s:t>shared</s:t></s:r></s:si>"));
    let b = read_xlsx(&bytes).unwrap();
    let s = &b.sheets[0];
    assert_eq!(at(s, "A3").value, CellValue::Text("hello & world".into()));
    let runs = &s.rich_text[&CellRef::new(0, 2)];
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "hello & world");
    assert_eq!(runs[0].font.bold, Some(false));
    assert_eq!(runs[0].font.italic, Some(false));
    assert_eq!(s.rich_text[&CellRef::new(1, 2)][0].text, "shared");
}

#[test]
fn shared_choice_values_rich_runs_and_phonetic_readings_use_the_same_item_indices() {
    let shared = pool(
        r#"<s:si/><s:si><mc:AlternateContent><mc:Choice Requires="u"><s:t>間違い</s:t></mc:Choice><mc:Fallback><s:r><s:t>東京</s:t></s:r><s:rPh sb="0" eb="2"><s:t>トウキョウ</s:t></s:rPh></mc:Fallback></mc:AlternateContent></s:si>"#,
    );
    let xml = worksheet(
        r#"<s:sheetData><s:row><s:c t="s"><mc:AlternateContent><mc:Choice Requires="s"><s:v><![CDATA[1]]></s:v></mc:Choice><mc:Fallback><s:v>0</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#,
    );
    let bytes = package(&xml, &shared);
    let b = read_xlsx(&bytes).unwrap();
    assert_eq!(at(&b.sheets[0], "A1").value, CellValue::Text("東京".into()));
    assert_eq!(b.sheets[0].rich_text[&CellRef::new(0, 0)][0].text, "東京");
    let readings = jade_xlsx_reader::phonetic::sort_readings(
        &bytes,
        "xl/worksheets/sheet1.xml",
        jade_xlsx_reader::CellRange::parse("A1").unwrap(),
        &BTreeSet::from([0]),
    )
    .unwrap();
    assert_eq!(readings[&CellRef::new(0, 0)].base, "東京");
    assert_eq!(
        readings[&CellRef::new(0, 0)].key.as_deref(),
        Some("トウキョウ")
    );
}

#[test]
fn empty_selected_choices_do_not_fall_through_and_nested_leaf_choices_keep_text() {
    let xml = worksheet(
        r#"<s:sheetData><s:row><s:c><mc:AlternateContent><mc:Choice Requires="s"/><mc:Fallback><s:v>99</s:v></mc:Fallback></mc:AlternateContent></s:c><s:c><s:v><mc:AlternateContent><mc:Choice Requires="s"><![CDATA[2]]>&#51;</mc:Choice><mc:Fallback>99</mc:Fallback></mc:AlternateContent></s:v></s:c><s:c><mc:AlternateContent><mc:Choice xmlns:q="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:s="urn:foreign" Requires="q"><s:v>99</s:v><q:v>7</q:v></mc:Choice><mc:Fallback><s:v>99</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></s:sheetData>"#,
    );
    let s = parse(&xml);
    assert_eq!(at(&s, "A1").value, CellValue::Empty);
    assert_eq!(at(&s, "B1").value, CellValue::Number(23.0));
    assert_eq!(at(&s, "C1").value, CellValue::Number(7.0));
}

#[test]
fn foreign_or_misplaced_sheet_settings_do_not_override_owned_settings() {
    let xml = worksheet(
        r#"<s:sheetPr><s:tabColor rgb="FF123456"/><s:pageSetUpPr fitToPage="1"/></s:sheetPr><s:sheetViews><s:sheetView showGridLines="0"><s:pane state="frozen" ySplit="2"/></s:sheetView></s:sheetViews><s:pageSetup orientation="portrait"/><s:headerFooter><s:oddHeader>left<mc:AlternateContent><mc:Choice Requires="s"> &amp; right</mc:Choice><mc:Fallback>wrong</mc:Fallback></mc:AlternateContent></s:oddHeader></s:headerFooter><u:wrapper><s:sheetProtection sheet="1"/><s:pageSetup orientation="landscape"/><s:tabColor rgb="FFFFFFFF"/><s:pane state="frozen" ySplit="99"/><s:sheetView showGridLines="1"/></u:wrapper><s:customSheetViews><s:customSheetView><s:pageSetup orientation="landscape"/></s:customSheetView></s:customSheetViews><s:sheetData/>"#,
    );
    let s = parse(&xml);
    assert!(s.protection.is_none());
    assert_eq!(
        s.tab_color,
        Some(jade_xlsx_reader::Color::Argb([255, 18, 52, 86]))
    );
    assert!(!s.show_grid_lines);
    assert_eq!(s.freeze_pane.unwrap().frozen_rows, 2);
    assert_eq!(
        s.page_setup.orientation,
        Some(jade_xlsx_reader::model::PageOrientation::Portrait)
    );
    assert!(s.page_setup.fit_to_page);
    assert_eq!(
        s.header_footer.unwrap().odd_header.as_deref(),
        Some("left & right")
    );
}

#[test]
fn quoted_and_escaped_attribute_values_survive_the_interpreted_view() {
    let xml = pool(
        r#"<s:si><s:r><s:rPr><s:rFont val='A "B" &amp; C'/></s:rPr><s:t>kept</s:t></s:r></s:si>"#,
    );
    let runs = jade_xlsx_reader::rich_text::parse_shared_runs(&xml).unwrap();
    assert_eq!(
        runs[0].as_ref().unwrap()[0].font.name.as_deref(),
        Some("A \"B\" & C")
    );
}
