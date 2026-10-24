use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{Axis, CellRange};
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use std::collections::BTreeSet;
const PART: &str = "xl/worksheets/sheet1.xml";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";
const XR: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn custom(tail: &str) -> HeadlessWorkbook {
    let bytes = include_bytes!("../../../public/fixtures/sample-workbook.xlsx");
    let mut p = OpcPackage::open(bytes).unwrap();
    p.replace(PART,format!("<worksheet xmlns=\"{MAIN}\" xmlns:x=\"{X14}\" xmlns:m=\"{XM}\" xmlns:rev=\"{XR}\" xmlns:u=\"urn:foreign\"><sheetData><row r=\"1\"><c r=\"A1\"><v>10</v></c></row><row r=\"3\"><c r=\"B3\"><v>20</v></c></row></sheetData>{tail}</worksheet>").into_bytes());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
fn xml(book: &HeadlessWorkbook) -> String {
    OpcPackage::open(book.bytes())
        .unwrap()
        .required_part_str(PART)
        .unwrap()
}
fn texts(xml: &str, ns: &str, name: &[u8]) -> Vec<String> {
    let mut r = NsReader::from_str(xml);
    let mut result = Vec::new();
    loop {
        match r.read_event().unwrap() {
            Event::Start(e)
                if e.local_name().as_ref() == name
                    && matches!(r.resolver().resolve_element(e.name()).0,ResolveResult::Bound(n) if n.as_ref()==ns.as_bytes()) =>
            {
                result.push(r.read_text(e.name()).unwrap().into_owned());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    result
}
fn attrs(xml: &str, ns: &str, element: &[u8], key: &[u8]) -> Vec<String> {
    let mut r = NsReader::from_str(xml);
    let mut result = Vec::new();
    loop {
        match r.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref() == element
                    && matches!(r.resolver().resolve_element(e.name()).0,ResolveResult::Bound(n) if n.as_ref()==ns.as_bytes()) =>
            {
                for a in e.attributes() {
                    let a = a.unwrap();
                    if a.key.as_ref() == key {
                        result.push(a.unescape_value().unwrap().into_owned());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    result
}
#[test]
fn extended_validation_splits_relative_anchors_counts_and_revision_ids() {
    let mut b = custom(
        r#"<extLst><ext uri="validation"><x:dataValidations count="1" disablePrompts="1"><x:dataValidation type="custom" rev:uid="{4A414445-584C-4000-8000-000000000001}" u:uid="keep"><x:formula1><m:f>B1&gt;0</m:f></x:formula1><m:sqref>B1:B5</m:sqref><u:formula1><u:f>B3</u:f></u:formula1></x:dataValidation></x:dataValidations><u:dataValidations count="9"><u:dataValidation><m:sqref>B3</m:sqref></u:dataValidation></u:dataValidations></ext><ext uri="opaque"><u:content>B3</u:content></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let out = xml(&b);
    assert_eq!(attrs(&out, X14, b"dataValidations", b"count"), ["3"]);
    assert_eq!(&texts(&out, XM, b"sqref")[..3], ["B1:B2", "B3", "B4:B6"]);
    assert_eq!(texts(&out, XM, b"f"), ["B1&gt;0", "B3&gt;0", "B4&gt;0"]);
    let ids = attrs(&out, X14, b"dataValidation", b"rev:uid");
    assert_eq!(ids.len(), 3);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 3);
    assert_eq!(ids[0], "{4A414445-584C-4000-8000-000000000001}");
    assert_eq!(
        attrs(&out, X14, b"dataValidation", b"u:uid"),
        ["keep", "keep", "keep"]
    );
    assert!(out.contains(r#"<u:dataValidations count="9"><u:dataValidation><m:sqref>B3</m:sqref></u:dataValidation></u:dataValidations>"#));
    assert!(out.contains("<u:content>B3</u:content>"));
    let reopened = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(xml(&reopened), out);
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(xml(&b), out);
}
#[test]
fn sparklines_move_display_cells_but_never_fill_new_cells_and_remove_deleted_records() {
    let mut b = custom(
        r#"<extLst><ext uri="sparklines"><x:sparklineGroups><x:sparklineGroup type="line" u:option="keep"><m:f>'売上'!B1:B5</m:f><x:colorSeries rgb="FF123456"/><x:sparklines><x:sparkline><m:f>'売上'!B1:B5</m:f><m:sqref>B3</m:sqref></x:sparkline><x:sparkline><m:f>'売上'!C1:C5</m:f><m:sqref>D3</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    // Use the real current worksheet name so a structural rewrite applies.
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(PART, xml(&b).replace("売上", &s).into_bytes());
    b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let out = xml(&b);
    assert_eq!(texts(&out, XM, b"sqref"), ["B4", "D3"]);
    let f = texts(&out, XM, b"f");
    assert!(f[0].ends_with("!B1:B6"));
    assert!(f[1].ends_with("!B1:B6"));
    assert!(f[2].ends_with("!C1:C5"));
    assert!(out.contains("u:option=\"keep\""));
    b.shift_cells(&s, range("B4"), Axis::Rows, false).unwrap();
    assert_eq!(texts(&xml(&b), XM, b"sqref"), ["D3"]);
    b.shift_cells(&s, range("D3"), Axis::Columns, false)
        .unwrap();
    assert!(texts(&xml(&b), XM, b"sqref").is_empty());
    assert!(!xml(&b).contains("<x:sparklineGroups"));
}
#[test]
fn aggregate_thresholds_keep_one_union_while_expression_priorities_include_x14() {
    let mut b = custom(
        r#"<conditionalFormatting sqref="A1:C5" rev:uid="{AAAAAAAA-0000-4000-8000-000000000001}"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="formula" val="$B$3"/><cfvo type="max"/><color rgb="FF123456"/></dataBar></cfRule><cfRule type="expression" priority="2"><formula>B1&gt;0</formula></cfRule></conditionalFormatting><extLst><ext uri="modern"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="expression" priority="3" id="{AAAAAAAA-0000-4000-8000-000000000002}"><m:f>B1&gt;0</m:f></x:cfRule><m:sqref>B1:B5</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let out = xml(&b);
    assert_eq!(attrs(&out, MAIN, b"cfvo", b"val"), ["$B$4"]);
    let cfs = attrs(&out, MAIN, b"conditionalFormatting", b"sqref");
    assert_eq!(cfs[0], "A1:A5 B1:B6 C1:C5");
    let all = attrs(&out, MAIN, b"cfRule", b"priority")
        .into_iter()
        .chain(attrs(&out, X14, b"cfRule", b"priority"))
        .collect::<Vec<_>>();
    assert_eq!(all.len(), 9);
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), 9);
    let ids = attrs(&out, X14, b"cfRule", b"id");
    assert_eq!(ids.len(), 3);
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 3);
    let wrapper_ids = attrs(&out, MAIN, b"conditionalFormatting", b"rev:uid");
    assert_eq!(
        wrapper_ids.iter().collect::<BTreeSet<_>>().len(),
        wrapper_ids.len()
    );
    assert_eq!(texts(&out, XM, b"f"), ["B1&gt;0", "B3&gt;0", "B4&gt;0"]);
}
#[test]
fn deleting_all_covered_cells_removes_paired_extensions_and_validation() {
    let mut b = custom(
        r#"<conditionalFormatting sqref="B3"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF123456"/></dataBar><extLst><ext uri="link"><x:id>{AAAAAAAA-0000-4000-8000-000000000001}</x:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="cf"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="dataBar" id="{AAAAAAAA-0000-4000-8000-000000000001}"><x:dataBar><x:cfvo type="autoMin"/><x:cfvo type="autoMax"/></x:dataBar></x:cfRule><m:sqref>B3</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext><ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="whole"><x:formula1><m:f>1</m:f></x:formula1><m:sqref>B3</m:sqref></x:dataValidation></x:dataValidations></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    b.shift_cells(&s, range("B3"), Axis::Rows, false).unwrap();
    let out = xml(&b);
    assert!(attrs(&out, MAIN, b"conditionalFormatting", b"sqref").is_empty());
    assert!(texts(&out, XM, b"sqref").is_empty());
    assert!(!out.contains("<x:conditionalFormattings"));
    assert!(!out.contains("<x:dataValidations"));
}

#[test]
fn an_explicit_extended_priority_makes_its_id_independent_of_a_legacy_link() {
    let mut b = custom(
        r#"<conditionalFormatting sqref="B1:B5"><cfRule type="dataBar" priority="1"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF123456"/></dataBar><extLst><ext uri="link"><x:id>{AAAAAAAA-0000-4000-8000-000000000001}</x:id></ext></extLst></cfRule></conditionalFormatting><extLst><ext uri="cf"><x:conditionalFormattings><x:conditionalFormatting><x:cfRule type="dataBar" priority="2" id="{AAAAAAAA-0000-4000-8000-000000000001}"><x:dataBar><x:cfvo type="autoMin"/><x:cfvo type="autoMax"/></x:dataBar></x:cfRule><m:sqref>C1:C5</m:sqref></x:conditionalFormatting></x:conditionalFormattings></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    assert_eq!(
        attrs(&xml(&b), MAIN, b"conditionalFormatting", b"sqref"),
        ["B1:B6"]
    );
    assert_eq!(texts(&xml(&b), XM, b"sqref"), ["C1:C5"]);
}

#[test]
fn another_sheets_sparkline_source_follows_the_edit_without_moving_its_display() {
    let mut b = custom("");
    let s = b.workbook().sheets[0].name.clone();
    let other = b.workbook().sheets[1].part_name.clone();
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(&other,format!(r#"<worksheet xmlns="{MAIN}" xmlns:x="{X14}" xmlns:m="{XM}"><sheetData/><extLst><ext uri="spark"><x:sparklineGroups><x:sparklineGroup><x:sparklines><x:sparkline><m:f>'{s}'!B1:B5</m:f><m:sqref>B3</m:sqref></x:sparkline></x:sparklines></x:sparklineGroup></x:sparklineGroups></ext></extLst></worksheet>"#).into_bytes());
    b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let out = OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(&other)
        .unwrap();
    assert_eq!(texts(&out, XM, b"sqref"), ["B3"]);
    assert!(texts(&out, XM, b"f")[0].ends_with("!B1:B6"));
}

#[test]
fn horizontal_extended_validation_keeps_both_bounds_and_perpendicular_regions() {
    let mut b = custom(
        r#"<extLst><ext uri="dv"><x:dataValidations count="1"><x:dataValidation type="decimal" operator="between"><x:formula1><m:f>$B$3</m:f></x:formula1><x:formula2><m:f>B3+10</m:f></x:formula2><m:sqref>A2:E4</m:sqref></x:dataValidation></x:dataValidations></ext></extLst>"#,
    );
    let s = b.workbook().sheets[0].name.clone();
    b.shift_cells(&s, range("C3"), Axis::Columns, true).unwrap();
    let out = xml(&b);
    assert_eq!(
        texts(&out, XM, b"sqref"),
        ["A2:E2", "A3:B3", "C3", "D3:F3", "A4:E4"]
    );
    assert_eq!(attrs(&out, X14, b"dataValidations", b"count"), ["5"]);
    assert_eq!(
        texts(&out, XM, b"f"),
        ["$B$3", "B3+10", "$B$3", "B4+10", "$B$3", "D4+10", "$B$3", "D4+10", "$B$3", "B5+10"]
    );
}

#[test]
fn default_namespaced_extensions_work_with_a_strict_worksheet_and_foreign_lookalikes() {
    let mut b = custom(&format!(
        r#"<extLst><ext uri="dv"><dataValidations xmlns="{X14}" xmlns:q="{XM}" count="1"><dataValidation type="whole"><formula1><q:f>$B$3</q:f></formula1><q:sqref>B3:B5</q:sqref></dataValidation></dataValidations><dataValidations xmlns="urn:foreign"><dataValidation><sqref>B3</sqref></dataValidation></dataValidations></ext></extLst>"#
    ));
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    p.replace(
        PART,
        xml(&b)
            .replace(MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main")
            .into_bytes(),
    );
    b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let s = b.workbook().sheets[0].name.clone();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let out = xml(&b);
    assert_eq!(texts(&out, XM, b"sqref"), ["B4:B6"]);
    assert_eq!(texts(&out, XM, b"f"), ["$B$4"]);
    assert_eq!(texts(&out, "urn:foreign", b"sqref"), ["B3"]);
    assert_eq!(attrs(&out, X14, b"dataValidations", b"count"), ["1"]);
}
