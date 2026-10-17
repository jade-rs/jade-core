use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, Axis, CellRef, StructureChange};
use jade_xlsx_writer::{restructure, write_xlsx, CellEdit, NewValue, SheetEdits};
const BYTES: &[u8] = include_bytes!("../../../public/fixtures/charts-workbook.xlsx");
#[test]
fn editing_source_values_refreshes_caches_and_preserves_chart_metadata() {
    let result = write_xlsx(
        BYTES,
        &[SheetEdits {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells: vec![CellEdit {
                reference: CellRef::new(1, 1),
                value: NewValue::Number(35.0),
                style_index: None,
            }],
        }],
    )
    .unwrap();
    let before = OpcPackage::open(BYTES).unwrap();
    let after = OpcPackage::open(&result).unwrap();
    for part in [
        "xl/drawings/drawing1.xml",
        "xl/drawings/_rels/drawing1.xml.rels",
        "xl/theme/theme1.xml",
    ] {
        assert_eq!(before.part(part), after.part(part));
    }
    let old_chart = jade_xlsx_reader::read_xlsx(BYTES).unwrap().sheets[0].shapes[0]
        .chart
        .clone()
        .unwrap();
    let book = read_xlsx(&result).unwrap();
    let chart = book.sheets[0].shapes[0].chart.as_ref().unwrap();
    assert_eq!(
        chart.series[0].values.points,
        [
            Some("35".into()),
            Some("20".into()),
            Some("30".into()),
            None
        ]
    );
    assert_eq!(
        chart.series[0].categories.points,
        [
            Some("Jan".into()),
            Some("Feb".into()),
            Some("Mar".into()),
            Some("Apr".into())
        ]
    );
    assert_eq!(chart.title, old_chart.title);
    assert_eq!(chart.value_axis, old_chart.value_axis);
    assert_eq!(chart.palette, old_chart.palette);
    assert_eq!(chart.series[0].color, old_chart.series[0].color);
    assert_eq!(
        book.sheets[0].rows[1].cells[1].value,
        jade_xlsx_reader::CellValue::Number(35.0)
    );
    assert_eq!(
        book.sheets[0].shapes[0].chart.as_ref().unwrap().series[0]
            .values
            .formula
            .as_deref(),
        Some("Charts!$B$2:$B$5")
    );
}
#[test]
fn structure_edits_shift_chart_formulas_and_preserve_absolute_chart_anchors() {
    let before = read_xlsx(BYTES).unwrap();
    let result = restructure(
        BYTES,
        &before,
        &StructureChange {
            sheet: "Charts".into(),
            axis: Axis::Rows,
            at: 1,
            count: 2,
            inserted: true,
        },
    )
    .unwrap();
    let after = read_xlsx(&result).unwrap();
    assert_eq!(
        after.sheets[0].shapes[0].chart.as_ref().unwrap().series[0]
            .values
            .formula
            .as_deref(),
        Some("Charts!$B$4:$B$7")
    );
    assert_eq!(
        after.sheets[0].shapes[0].from_offset,
        before.sheets[0].shapes[0].from_offset
    );
    assert_eq!(
        after.sheets[0].shapes[6].chart.as_ref().unwrap().series[0]
            .x_values
            .formula
            .as_deref(),
        Some("'Data O''Brien'!$A$2:$A$4")
    );
}
