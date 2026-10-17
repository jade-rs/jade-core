use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, Axis, CellRef, StructureChange};
use jade_xlsx_writer::{
    refresh_chart_caches, restructure, write_cached_values, write_xlsx, CachedValue,
    CachedValueEdit, CellEdit, NewValue, SheetCachedValues, SheetEdits,
};
const BYTES: &[u8] = include_bytes!("../../../public/fixtures/charts-workbook.xlsx");
const PART: &str = "xl/charts/chart1.xml";
fn alter(parts: &[(&str, &dyn Fn(String) -> String)]) -> Vec<u8> {
    let mut package = OpcPackage::open(BYTES).unwrap();
    for (name, change) in parts {
        let xml = package.part_str(name).unwrap().unwrap();
        package.replace(name, change(xml).into_bytes());
    }
    package.write().unwrap()
}
fn edit(bytes: &[u8], reference: &str, value: NewValue) -> Vec<u8> {
    write_xlsx(
        bytes,
        &[SheetEdits {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells: vec![CellEdit {
                reference: CellRef::parse(reference).unwrap(),
                value,
                style_index: None,
            }],
        }],
    )
    .unwrap()
}
#[test]
fn refresh_is_idempotent_and_keeps_sparse_positions_after_structure_edits() {
    let (once, report) = refresh_chart_caches(BYTES).unwrap();
    assert!(report.updated_caches > 0);
    assert_eq!(report.unresolved_references, 0);
    let (twice, report) = refresh_chart_caches(&once).unwrap();
    assert_eq!(report.updated_caches, 0);
    assert_eq!(twice, once);
    let book = read_xlsx(&once).unwrap();
    let inserted = restructure(
        &once,
        &book,
        &StructureChange {
            sheet: "Charts".into(),
            axis: Axis::Rows,
            at: 2,
            count: 1,
            inserted: true,
        },
    )
    .unwrap();
    let points = read_xlsx(&inserted).unwrap().sheets[0].shapes[0]
        .chart
        .as_ref()
        .unwrap()
        .series[0]
        .values
        .points
        .clone();
    assert_eq!(
        points,
        [
            Some("10".into()),
            None,
            Some("20".into()),
            Some("30".into()),
            None
        ]
    );
}
#[test]
fn style_formula_extensions_and_point_attributes_survive_value_splices() {
    let custom = alter(&[(PART, &|xml| {
        xml.replace("<c:numCache>","<c:numCache custom=\"keep\">").replace("<c:pt idx=\"0\"><c:v>99</c:v>","<c:pt idx=\"0\" formatCode=\"0.00\"><c:v>99</c:v>").replace("</c:numCache>","<c:extLst><c:ext uri=\"test\"><z:payload xmlns:z=\"urn:foreign\" value=\"untouched\"/></c:ext></c:extLst></c:numCache>")
    })]);
    let written = edit(&custom, "B2", NewValue::Number(35.0));
    let xml = OpcPackage::open(&written)
        .unwrap()
        .part_str(PART)
        .unwrap()
        .unwrap();
    assert!(xml.contains("<c:numCache custom=\"keep\">"));
    assert!(xml.contains("<c:pt idx=\"0\" formatCode=\"0.00\"><c:v>35</c:v>"));
    assert!(xml.contains("<z:payload xmlns:z=\"urn:foreign\" value=\"untouched\"/>"));
    assert!(xml.contains("<c:f>Charts!$B$2:$B$5</c:f>"));
}
#[test]
fn unresolved_external_and_foreign_references_are_never_replaced() {
    let input = alter(&[(PART, &|xml| {
        xml.replace("Charts!$B$2:$B$5","[external.xlsx]Charts!$B$2:$B$5").replace("</c:chartSpace>","<z:extension xmlns:z=\"urn:other\"><c:numRef><c:f>Charts!B2:B5</c:f><c:numCache><c:pt idx=\"0\"><c:v>777</c:v></c:pt></c:numCache></c:numRef></z:extension></c:chartSpace>")
    })]);
    let (written, report) = refresh_chart_caches(&input).unwrap();
    assert!(report.unresolved_references > 0);
    let xml = OpcPackage::open(&written)
        .unwrap()
        .part_str(PART)
        .unwrap()
        .unwrap();
    assert!(xml.contains("<c:v>777</c:v>"));
    let chart = read_xlsx(&written).unwrap().sheets[0].shapes[0]
        .chart
        .clone()
        .unwrap();
    assert_eq!(chart.series[0].values.points, vec![Some("99".into()); 4]);
}
#[test]
fn uncalculated_formulas_keep_cached_points_while_known_numbers_refresh() {
    let written = edit(BYTES, "B3", NewValue::Formula("UNSUPPORTED()".into()));
    let chart = read_xlsx(&written).unwrap().sheets[0].shapes[0]
        .chart
        .clone()
        .unwrap();
    assert_eq!(
        chart.series[0].values.points,
        [
            Some("10".into()),
            Some("99".into()),
            Some("30".into()),
            None
        ]
    );
    let written = write_cached_values(
        &written,
        &[SheetCachedValues {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells: vec![CachedValueEdit {
                reference: CellRef::parse("B3").unwrap(),
                value: CachedValue::Number(42.0),
            }],
        }],
        false,
    )
    .unwrap();
    let chart = read_xlsx(&written).unwrap().sheets[0].shapes[0]
        .chart
        .clone()
        .unwrap();
    assert_eq!(chart.series[0].values.points[1].as_deref(), Some("42"));
}
#[test]
fn string_caches_escape_xml_and_xstrings_and_decode_back_to_the_cell_text() {
    let text = "東京 & <期間> _x0041_\r\u{1}😀";
    let written = edit(BYTES, "A2", NewValue::Text(text.into()));
    let xml = OpcPackage::open(&written)
        .unwrap()
        .part_str(PART)
        .unwrap()
        .unwrap();
    assert!(xml.contains("&amp;"));
    assert!(xml.contains("_x005F_x0041_"));
    assert!(xml.contains("_x0001_"));
    let chart = read_xlsx(&written).unwrap().sheets[0].shapes[0]
        .chart
        .clone()
        .unwrap();
    assert_eq!(chart.series[0].categories.points[0].as_deref(), Some(text));
}
#[test]
fn strict_chart_prefixes_and_missing_or_empty_caches_are_updated() {
    for empty in ["", "<q:numCache/>"] {
        let input = alter(&[(PART, &|xml| {
            let xml = xml
                .replace(
                    "schemas.openxmlformats.org/drawingml/2006/chart",
                    "purl.oclc.org/ooxml/drawingml/chart",
                )
                .replace("c:", "q:")
                .replace("xmlns:c=", "xmlns:q=");
            let start = xml.find("<q:numCache>").unwrap();
            let end = start + xml[start..].find("</q:numCache>").unwrap() + "</q:numCache>".len();
            format!("{}{}{}", &xml[..start], empty, &xml[end..])
        })]);
        let (written, _) = refresh_chart_caches(&input).unwrap();
        let chart = read_xlsx(&written).unwrap().sheets[0].shapes[0]
            .chart
            .clone()
            .unwrap();
        assert_eq!(
            chart.series[0].values.points,
            [
                Some("10".into()),
                Some("20".into()),
                Some("30".into()),
                None
            ]
        );
    }
}
#[test]
fn formatted_numeric_categories_use_the_shared_formatter() {
    let styles = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/conditional-formats-workbook.xlsx"
    ))
    .unwrap()
    .part("xl/styles.xml")
    .unwrap()
    .to_vec();
    let mut package = OpcPackage::open(BYTES).unwrap();
    package.replace("xl/styles.xml", styles);
    let rels=package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace("</Relationships>","<Relationship Id=\"styles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>");
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let sheet = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace(
            "<c r=\"A2\" t=\"inlineStr\"><is><t>Jan</t></is></c>",
            "<c r=\"A2\" s=\"0\"><v>12.5</v></c>",
        );
    package.replace("xl/worksheets/sheet1.xml", sheet.into_bytes());
    let (written, _) = refresh_chart_caches(&package.write().unwrap()).unwrap();
    let chart = read_xlsx(&written).unwrap().sheets[0].shapes[0]
        .chart
        .clone()
        .unwrap();
    assert_eq!(
        chart.series[0].categories.points[0].as_deref(),
        Some("12.50")
    );
}
#[test]
fn recalculation_flags_follow_a_nondefault_workbook_part_name() {
    let mut package = OpcPackage::open(BYTES).unwrap();
    let workbook = package.part("xl/workbook.xml").unwrap().to_vec();
    let rels = package.part("xl/_rels/workbook.xml.rels").unwrap().to_vec();
    package.replace("xl/book-main.xml", workbook);
    package.replace("xl/_rels/book-main.xml.rels", rels);
    package.remove("xl/workbook.xml");
    package.remove("xl/_rels/workbook.xml.rels");
    let root = package
        .part_str("_rels/.rels")
        .unwrap()
        .unwrap()
        .replace("xl/workbook.xml", "xl/book-main.xml");
    package.replace("_rels/.rels", root.into_bytes());
    let edited = edit(&package.write().unwrap(), "B2", NewValue::Number(35.0));
    assert!(read_xlsx(&edited).unwrap().full_calculation_on_load);
    let withdrawn = write_cached_values(&edited, &[], true).unwrap();
    assert!(!read_xlsx(&withdrawn).unwrap().full_calculation_on_load);
}
