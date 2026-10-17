use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, Axis, StructureChange};
use jade_xlsx_writer::{restructure, WriteError};
#[test]
fn an_insertion_never_silently_discards_values_or_formats_at_the_sheet_edge() {
    for (axis, reference) in [(Axis::Rows, "A1048576"), (Axis::Columns, "XFD5")] {
        let mut package = OpcPackage::open(include_bytes!(
            "../../../public/fixtures/grid-operations-workbook.xlsx"
        ))
        .unwrap();
        let xml = package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap()
            .replace(
                "</sheetData>",
                &format!(
                    "<row r=\"{}\"><c r=\"{reference}\"><v>42</v></c></row></sheetData>",
                    if axis == Axis::Rows { 1048576 } else { 5 }
                ),
            );
        package.replace("xl/worksheets/sheet1.xml", xml.into_bytes());
        let bytes = package.write().unwrap();
        let model = read_xlsx(&bytes).unwrap();
        let error = restructure(
            &bytes,
            &model,
            &StructureChange {
                sheet: "Operations".into(),
                axis,
                at: 0,
                count: 1,
                inserted: true,
            },
        )
        .unwrap_err();
        assert!(matches!(error, WriteError::InsertionWouldDiscardCells));
    }
}

#[test]
fn insertion_counts_cannot_exceed_the_remaining_axis_space() {
    let bytes = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let model = read_xlsx(bytes).unwrap();
    for axis in [Axis::Rows, Axis::Columns] {
        assert!(matches!(
            restructure(
                bytes,
                &model,
                &StructureChange {
                    sheet: "Operations".into(),
                    axis,
                    at: 1,
                    count: u32::MAX,
                    inserted: true
                }
            ),
            Err(WriteError::StructureOutOfBounds { .. })
        ));
    }
}
