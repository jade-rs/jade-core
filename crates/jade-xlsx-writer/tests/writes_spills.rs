//! A dynamic array is saved as Excel saves one: the anchor's formula marked
//! `<f t="array" ref>` with `cm` pointing at the workbook's dynamic-array
//! cell metadata (`xl/metadata.xml`, `XLDAPR`), and the spilled values as
//! plain cells beside it.

use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, ArrayFormula, CellRange, CellRef, CellValue};
use jade_xlsx_writer::spills::{write_spills, ArrayRange, SheetSpills};
use jade_xlsx_writer::{write_xlsx, CellEdit, NewValue, SheetEdits};

fn at(reference: &str) -> CellRef {
    CellRef::parse(reference).unwrap()
}

fn range(reference: &str) -> CellRange {
    CellRange::parse(reference).unwrap()
}

/// The fixture with `=SEQUENCE(3)` typed in J1, and its first sheet's part.
fn with_sequence() -> (Vec<u8>, String) {
    let source = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let part = read_xlsx(source).unwrap().sheets[0].part_name.clone();
    let bytes = write_xlsx(
        source,
        &[SheetEdits {
            part_name: part.clone(),
            cells: vec![CellEdit {
                reference: at("J1"),
                value: NewValue::Formula("_xlfn.SEQUENCE(3)".into()),
                style_index: None,
            }],
        }],
    )
    .unwrap();
    (bytes, part)
}

fn spill(part: &str, anchor: &str, filled: &str, values: &[(&str, f64)]) -> SheetSpills {
    SheetSpills {
        part_name: part.to_string(),
        values: values
            .iter()
            .map(|(reference, number)| CellEdit {
                reference: at(reference),
                value: NewValue::Number(*number),
                style_index: None,
            })
            .collect(),
        ranges: vec![ArrayRange {
            anchor: at(anchor),
            range: range(filled),
        }],
    }
}

fn value(bytes: &[u8], reference: &str) -> CellValue {
    read_xlsx(bytes).unwrap().sheets[0]
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .find(|cell| cell.reference == at(reference))
        .map(|cell| cell.value.clone())
        .unwrap_or(CellValue::Empty)
}

#[test]
fn a_typed_spill_is_saved_as_a_dynamic_array() {
    let (bytes, part) = with_sequence();
    let written = write_spills(
        &bytes,
        &[spill(&part, "J1", "J1:J3", &[("J2", 2.0), ("J3", 3.0)])],
        false,
    )
    .unwrap();

    let workbook = read_xlsx(&written).unwrap();
    assert_eq!(
        workbook.sheets[0].array_formulas,
        [ArrayFormula {
            anchor: at("J1"),
            range: range("J1:J3"),
        }]
    );
    assert_eq!(value(&written, "J2"), CellValue::Number(2.0));
    assert_eq!(value(&written, "J3"), CellValue::Number(3.0));

    let package = OpcPackage::open(&written).unwrap();
    let sheet = package.part_str(&part).unwrap().unwrap();
    let anchor = &sheet[sheet.find(r#"r="J1""#).unwrap()..];
    let anchor = &anchor[..anchor.find("</c>").unwrap()];
    assert!(anchor.contains(r#"cm="1""#), "{anchor}");
    assert!(
        anchor.contains(r#"t="array""#) && anchor.contains(r#"ref="J1:J3""#),
        "{anchor}"
    );
    let metadata = package
        .part_str("xl/metadata.xml")
        .unwrap()
        .expect("metadata part");
    assert!(metadata.contains(r#"name="XLDAPR""#) && metadata.contains(r#"fDynamic="1""#));
    let rels = package
        .part_str("xl/_rels/workbook.xml.rels")
        .unwrap()
        .unwrap();
    assert!(rels.contains("relationships/sheetMetadata") && rels.contains("metadata.xml"));
    let types = package.part_str("[Content_Types].xml").unwrap().unwrap();
    assert!(types.contains(r#"PartName="/xl/metadata.xml""#));

    // Grown to five: the range follows, and the metadata is not added twice.
    let grown = write_spills(
        &written,
        &[spill(&part, "J1", "J1:J5", &[("J4", 4.0), ("J5", 5.0)])],
        false,
    )
    .unwrap();
    assert_eq!(
        read_xlsx(&grown).unwrap().sheets[0].array_formulas[0].range,
        range("J1:J5")
    );
    let package = OpcPackage::open(&grown).unwrap();
    let metadata = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert_eq!(
        metadata.matches(r#"<metadataType name="XLDAPR""#).count(),
        1
    );
    let rels = package
        .part_str("xl/_rels/workbook.xml.rels")
        .unwrap()
        .unwrap();
    assert_eq!(rels.matches("relationships/sheetMetadata").count(), 1);
}

#[test]
fn a_workbook_with_its_own_metadata_points_at_its_dynamic_array_entry() {
    // Rich-value metadata first, the dynamic-array entry second: the anchor
    // takes the second cell-metadata block.
    let (bytes, part) = with_sequence();
    let mut package = OpcPackage::open(&bytes).unwrap();
    package.replace(
        "xl/metadata.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="2"><metadataType name="XLRICHVALUE" minSupportedVersion="120000"/><metadataType name="XLDAPR" minSupportedVersion="120000" copy="1" pasteAll="1" pasteValues="1" merge="1" splitFirst="1" rowColShift="1" clearFormats="1" clearComments="1" assign="1" coerce="1" cellMeta="1"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="2"><bk><rc t="1" v="0"/></bk><bk><rc t="2" v="0"/></bk></cellMetadata></metadata>"#
            .as_bytes()
            .to_vec(),
    );
    let rels = package
        .part_str("xl/_rels/workbook.xml.rels")
        .unwrap()
        .unwrap()
        .replace(
            "</Relationships>",
            r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#,
        );
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let bytes = package.write().unwrap();

    let written = write_spills(
        &bytes,
        &[spill(&part, "J1", "J1:J3", &[("J2", 2.0), ("J3", 3.0)])],
        false,
    )
    .unwrap();
    let package = OpcPackage::open(&written).unwrap();
    let sheet = package.part_str(&part).unwrap().unwrap();
    let anchor = &sheet[sheet.find(r#"r="J1""#).unwrap()..];
    let anchor = &anchor[..anchor.find("</c>").unwrap()];
    assert!(anchor.contains(r#"cm="2""#), "{anchor}");
    let metadata = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert_eq!(
        metadata.matches(r#"<metadataType name="XLDAPR""#).count(),
        1
    );
}
