use jade_xlsx_reader::{
    chart::{parse_chart, resolve_chart_reference, ChartPointState},
    read_package, read_xlsx, CellValue, DefinedName, OpcPackage,
};
const BYTES: &[u8] = include_bytes!("../../../public/fixtures/charts-workbook.xlsx");
#[test]
fn references_preserve_empty_positions_hidden_cells_and_named_range_scope() {
    let mut book = read_xlsx(BYTES).unwrap();
    book.defined_names.extend([
        DefinedName {
            name: "Values".into(),
            formula: "Charts!$C$2:$C$5".into(),
            local_sheet_index: None,
            hidden: false,
        },
        DefinedName {
            name: "Values".into(),
            formula: "Charts!$B$2:$B$5".into(),
            local_sheet_index: Some(0),
            hidden: false,
        },
        DefinedName {
            name: "Alias".into(),
            formula: "Values".into(),
            local_sheet_index: Some(0),
            hidden: false,
        },
    ]);
    book.sheets[0].rows[2].hidden = true;
    let reference = resolve_chart_reference("Alias", &book.sheets[0], Some(&book)).unwrap();
    assert_eq!(reference.points.len(), 4);
    assert!(reference.points[1].hidden);
    assert!(reference.points[3].cell.is_none());
    assert_eq!(
        reference.points[0].cell.unwrap().value,
        CellValue::Number(10.0)
    );
    let reference =
        resolve_chart_reference("'Data O''Brien'!$B$2:$B$4", &book.sheets[0], Some(&book)).unwrap();
    assert_eq!(reference.sheet.name, "Data O'Brien");
    assert_eq!(
        reference.points[1].cell.unwrap().value,
        CellValue::Number(6.0)
    );
}
#[test]
fn external_union_cycles_and_oversized_ranges_remain_unresolved() {
    let mut book = read_xlsx(BYTES).unwrap();
    book.defined_names.extend([
        DefinedName {
            name: "AName".into(),
            formula: "BName".into(),
            local_sheet_index: None,
            hidden: false,
        },
        DefinedName {
            name: "BName".into(),
            formula: "AName".into(),
            local_sheet_index: None,
            hidden: false,
        },
    ]);
    for formula in [
        "AName",
        "[other.xlsx]Charts!B2:B5",
        "Charts!B2,C3",
        "Charts!B1:C4",
        "Charts!B1:B1048576",
    ] {
        assert!(
            resolve_chart_reference(formula, &book.sheets[0], Some(&book)).is_none(),
            "{formula}"
        );
    }
}
#[test]
fn point_states_and_open_package_part_replacements_are_authoritative() {
    let mut package = OpcPackage::open(BYTES).unwrap();
    let xml = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap()
        .replace("<v>10</v>", "<f>UNSUPPORTED()</f>");
    package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
    let book = read_package(&package).unwrap();
    let reference = resolve_chart_reference("Charts!B2:B5", &book.sheets[0], Some(&book)).unwrap();
    assert_eq!(reference.points[0].state, ChartPointState::Uncalculated);
    assert_eq!(reference.points[1].state, ChartPointState::Live);
}
#[test]
fn chart_cache_xstrings_decode_literals_controls_and_surrogate_pairs() {
    let chart=parse_chart(r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:lineChart><c:ser><c:tx><c:v>_x005F_x0041_</c:v></c:tx><c:cat><c:strLit><c:ptCount val="2"/><c:pt idx="0"><c:v>A_x0001_B</c:v></c:pt><c:pt idx="1"><c:v>_xD83D__xDE00_</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#).unwrap();
    assert_eq!(chart.series[0].name.points, [Some("_x0041_".into())]);
    assert_eq!(
        chart.series[0].categories.points,
        [Some("A\u{1}B".into()), Some("😀".into())]
    );
    assert!(chart.series[0].values.numeric);
}
