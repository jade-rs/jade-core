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

fn existing_metadata(metadata: &str) -> (OpcPackage, String) {
    let (bytes, part) = with_sequence();
    let mut package = OpcPackage::open(&bytes).unwrap();
    package.replace("xl/metadata.xml", metadata.as_bytes().to_vec());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace(
        "</Relationships>",
        r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#);
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    (package, part)
}

#[test]
fn metadata_append_handles_empty_qualified_and_foreign_containers() {
    for metadata in [
        r#"<metadata/>"#,
        r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><metadataTypes count="0"/><cellMetadata count="0"/></metadata>"#,
        r#"<m:metadata xmlns:m="http://purl.oclc.org/ooxml/spreadsheetml/main"><m:metadataTypes count="0"/><m:cellMetadata count="0"/><m:valueMetadata count="0"/><m:extLst/></m:metadata>"#,
        r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:f="urn:foreign"><f:metadataTypes count="1"><f:metadataType name="XLDAPR"/></f:metadataTypes><extLst><ext uri="keep"/></extLst></metadata>"#,
    ] {
        let (mut package, _) = existing_metadata(metadata);
        assert_eq!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
            Some(1),
            "{metadata}"
        );
        let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
        assert_eq!(
            jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&updated).unwrap(),
            [Some(true)],
            "{updated}"
        );
        assert_eq!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
            Some(1)
        );
        assert_eq!(
            package.part_str("xl/metadata.xml").unwrap().unwrap(),
            updated
        );
    }
}

const FIXED_METADATA: &str = r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><metadataTypes count="1"><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}"><dynamicArrayProperties xmlns="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray" fDynamic="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#;

#[test]
fn metadata_append_keeps_existing_false_links_and_flags() {
    let (mut package, part) = existing_metadata(FIXED_METADATA);
    let sheet = package
        .part_str(&part)
        .unwrap()
        .unwrap()
        .replacen("<c ", "<c cm=\"1\" ", 1);
    package.replace(&part, sheet.into_bytes());
    assert_eq!(
        jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
        Some(2)
    );
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert_eq!(
        jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&updated).unwrap(),
        [Some(false), Some(true)]
    );
    assert!(updated.contains(r#"<bk><rc t="1" v="0"/></bk>"#));
    assert!(updated.contains(r#"fDynamic="0""#));
}

#[test]
fn appended_metadata_keeps_dynamic_editing_after_save_and_reopen() {
    use jade_xlsx_headless_api::CellInput;
    use jade_xlsx_headless_editor::HeadlessWorkbook;
    let (package, _) = existing_metadata(FIXED_METADATA);
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    book.set(&sheet, "K1", CellInput::Number(3.0)).unwrap();
    book.set(&sheet, "J1", CellInput::Formula("SEQUENCE(K1)".into()))
        .unwrap();
    assert_eq!(
        book.workbook().sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
    book.set(&sheet, "K1", CellInput::Number(1.0)).unwrap();
    assert!(book.cell(&sheet, "J3").unwrap().number.is_none());
    let saved = book.save_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    reopened.set(&sheet, "K1", CellInput::Number(4.0)).unwrap();
    assert_eq!(reopened.cell(&sheet, "J4").unwrap().number, Some(4.0));
    reopened.undo().unwrap();
    assert!(reopened.cell(&sheet, "J4").unwrap().number.is_none());
    let metadata = OpcPackage::open(reopened.bytes())
        .unwrap()
        .part_str("xl/metadata.xml")
        .unwrap()
        .unwrap();
    assert_eq!(
        jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&metadata).unwrap(),
        [Some(false), Some(true)]
    );
}

#[test]
fn metadata_append_does_not_activate_dangling_or_ambiguous_old_links() {
    for metadata in [
        FIXED_METADATA.replace("v=\"0\"", "v=\"1\""),
        FIXED_METADATA.replace("t=\"1\"", "t=\"2\""),
        FIXED_METADATA.replace("<cellMetadata count=\"1\">", "<cellMetadata count=\"2\">"),
        FIXED_METADATA.replace(
            "</metadataTypes>",
            "<metadataType name=\"XLDAPR\"/></metadataTypes>",
        ),
        FIXED_METADATA.replace("</metadata>", "<cellMetadata count=\"0\"/></metadata>"),
    ] {
        let (mut package, _) = existing_metadata(&metadata);
        assert_eq!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
            None
        );
        assert_eq!(
            package.part_str("xl/metadata.xml").unwrap().unwrap(),
            metadata
        );
    }
    let (mut package, part) = existing_metadata(FIXED_METADATA);
    let sheet = package
        .part_str(&part)
        .unwrap()
        .unwrap()
        .replacen("<c ", "<c cm=\"2\" ", 1);
    package.replace(&part, sheet.into_bytes());
    assert_eq!(
        jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
        None
    );
    assert_eq!(
        package.part_str("xl/metadata.xml").unwrap().unwrap(),
        FIXED_METADATA
    );
}

#[test]
fn a_spill_appends_dynamic_metadata_without_replacing_existing_records() {
    let (bytes, part) = with_sequence();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let metadata = r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><metadataTypes count="1"><metadataType name="XLRICHVALUE"/></metadataTypes><futureMetadata name="XLRICHVALUE" count="1"><bk><extLst><ext uri="untouched"><foreign xmlns="urn:foreign" value="keep"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#;
    package.replace("xl/metadata.xml", metadata.as_bytes().to_vec());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace(
        "</Relationships>",
        r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#);
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let written = write_spills(
        &package.write().unwrap(),
        &[spill(&part, "J1", "J1:J3", &[("J2", 2.0), ("J3", 3.0)])],
        false,
    )
    .unwrap();
    assert_eq!(
        read_xlsx(&written).unwrap().sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
    let package = OpcPackage::open(&written).unwrap();
    let appended = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert!(appended.contains(r#"<metadataType name="XLRICHVALUE"/>"#));
    assert!(appended.contains(r#"<bk><rc t="1" v="0"/></bk>"#));
    assert!(appended.contains(r#"<foreign xmlns="urn:foreign" value="keep"/>"#));
    assert_eq!(
        jade_xlsx_reader::array_metadata::first_dynamic_array_cell_metadata(&appended).unwrap(),
        Some(2)
    );
    let repeated = write_spills(&written, &[spill(&part, "J1", "J1:J3", &[])], false).unwrap();
    assert_eq!(
        OpcPackage::open(&repeated)
            .unwrap()
            .part_str("xl/metadata.xml")
            .unwrap()
            .unwrap(),
        appended
    );
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
            kind: jade_xlsx_reader::ArrayFormulaKind::Dynamic,
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

#[test]
fn a_dynamic_spill_uses_the_referenced_true_flag_not_the_first_type_match() {
    let (bytes, part) = with_sequence();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let metadata = r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR" count="2"><bk><extLst><ext uri="{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}"><xda:dynamicArrayProperties fDynamic="0"/></ext></extLst></bk><bk><extLst><ext uri="{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}"><xda:dynamicArrayProperties fDynamic="true"/></ext></extLst></bk></futureMetadata><cellMetadata count="2"><bk><rc t="1" v="0"/></bk><bk><rc t="1" v="1"/></bk></cellMetadata></metadata>"#;
    package.replace("xl/metadata.xml", metadata.as_bytes().to_vec());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace(
        "</Relationships>",
        r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#,
    );
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let written = write_spills(
        &package.write().unwrap(),
        &[spill(&part, "J1", "J1:J3", &[("J2", 2.0), ("J3", 3.0)])],
        false,
    )
    .unwrap();
    let package = OpcPackage::open(&written).unwrap();
    let sheet = package.part_str(&part).unwrap().unwrap();
    let anchor = &sheet[sheet.find(r#"r="J1""#).unwrap()..];
    let anchor = &anchor[..anchor.find("</c>").unwrap()];
    assert!(anchor.contains(r#"cm="2""#), "{anchor}");
    assert_eq!(
        package.part_str("xl/metadata.xml").unwrap().unwrap(),
        metadata
    );
}
