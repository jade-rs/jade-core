use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef, CellValue};
use jade_xlsx_writer::spills::{write_spills, ArrayRange, SheetSpills};
use jade_xlsx_writer::{write_xlsx, CellEdit, NewValue, SheetEdits, WriteError};

#[test]
fn uncertifiable_dynamic_metadata_cannot_silently_write_a_legacy_array() {
    let source = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let part = read_xlsx(source).unwrap().sheets[0].part_name.clone();
    let anchor = CellRef::parse("J1").unwrap();
    let bytes = write_xlsx(
        source,
        &[SheetEdits {
            part_name: part.clone(),
            cells: vec![CellEdit {
                reference: anchor,
                value: NewValue::Formula("SEQUENCE(3)".into()),
                style_index: None,
            }],
        }],
    )
    .unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    package.replace("xl/metadata.xml", br#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><metadataTypes count="0"/><metadataTypes count="0"/></metadata>"#.to_vec());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace("</Relationships>", r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#);
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let original = package.write().unwrap();
    let result = write_spills(
        &original,
        &[SheetSpills {
            part_name: part.clone(),
            values: vec![],
            ranges: vec![ArrayRange {
                anchor,
                range: CellRange::parse("J1:J3").unwrap(),
            }],
        }],
        false,
    );
    let produced_kind = result
        .as_ref()
        .ok()
        .and_then(|bytes| read_xlsx(bytes).ok())
        .and_then(|book| {
            book.sheets[0]
                .array_formulas
                .iter()
                .find(|array| array.anchor == anchor)
                .map(|array| array.kind)
        });
    assert!(
        matches!(result, Err(WriteError::ArrayMetadata(_))),
        "dynamic write succeeded with array identity {produced_kind:?}"
    );
    let cached = CellRef::parse("J2").unwrap();
    let values_only = SheetSpills {
        part_name: part,
        values: vec![CellEdit {
            reference: cached,
            value: NewValue::Number(2.0),
            style_index: None,
        }],
        ranges: vec![],
    };
    for complete in [false, true] {
        let written =
            write_spills(&original, std::slice::from_ref(&values_only), complete).unwrap();
        let updated = OpcPackage::open(&written).unwrap();
        assert_eq!(
            updated.part("xl/metadata.xml"),
            package.part("xl/metadata.xml")
        );
        assert!(read_xlsx(&written).unwrap().sheets[0]
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.reference == cached && cell.value == CellValue::Number(2.0)));
    }
}
