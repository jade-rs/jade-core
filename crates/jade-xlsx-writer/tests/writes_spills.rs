//! A dynamic array is saved as Excel saves one: the anchor's formula marked
//! `<f t="array" ref>` with `cm` pointing at the workbook's dynamic-array
//! cell metadata (`xl/metadata.xml`, `XLDAPR`), and the spilled values as
//! plain cells beside it.

use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, ArrayFormula, CellRange, CellRef, CellValue};
use jade_xlsx_writer::spills::{write_spills, ArrayRange, SheetSpills};
use jade_xlsx_writer::{write_xlsx, CellEdit, NewValue, SheetEdits};

mod common;

#[test]
fn a_dynamic_array_capability_choice_uses_the_supported_formula_and_keeps_editing() {
    let (bytes, part) = with_sequence();
    let bytes = write_spills(
        &bytes,
        &[spill(&part, "J1", "J1:J3", &[("J2", 2.0), ("J3", 3.0)])],
        false,
    )
    .unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let sheet = package.part_str(&part).unwrap().unwrap();
    let cells = jade_xlsx_reader::cell_content::compatible_cells(
        &sheet,
        &std::collections::HashSet::from([at("J1")]),
    )
    .unwrap();
    let cell = cells.into_iter().find(|cell| cell.selected).unwrap();
    let wrapped = format!("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:d=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\" xmlns:u=\"urn:future\"><mc:Choice Requires=\"d\">{}</mc:Choice><mc:Fallback><c r=\"J1\"><v>0</v></c></mc:Fallback></mc:AlternateContent>", &sheet[cell.range.clone()]);
    let mut sheet = sheet.to_string();
    sheet.replace_range(cell.range, &wrapped);
    package.replace(&part, sheet.into_bytes());
    let bytes = package.write().unwrap();
    let parsed = read_xlsx(&bytes).unwrap();
    assert_eq!(
        parsed.sheets[0].array_formulas.len(),
        1,
        "a supported dynamic-array Choice was replaced by its fallback"
    );
    assert_eq!(
        parsed.sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
    for (before, after) in [
        (
            "xmlns:d=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"",
            "xmlns:d=\"urn:lookalike\"",
        ),
        ("Requires=\"d\"", "Requires=\"d u\""),
    ] {
        let mut unsupported = OpcPackage::open(&bytes).unwrap();
        let xml = unsupported
            .part_str(&part)
            .unwrap()
            .unwrap()
            .replace(before, after);
        unsupported.replace(&part, xml.into_bytes());
        let unsupported = unsupported.write().unwrap();
        assert!(read_xlsx(&unsupported).unwrap().sheets[0]
            .array_formulas
            .is_empty());
        assert_eq!(value(&unsupported, "J1"), CellValue::Number(0.0));
    }
    let mut legacy = OpcPackage::open(&bytes).unwrap();
    let metadata = legacy.part_str("xl/metadata.xml").unwrap().unwrap();
    assert!(metadata.contains("fDynamic=\"1\""));
    legacy.replace(
        "xl/metadata.xml",
        metadata
            .replace("fDynamic=\"1\"", "fDynamic=\"0\"")
            .into_bytes(),
    );
    assert_eq!(
        read_xlsx(&legacy.write().unwrap()).unwrap().sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Legacy
    );
    let mut moved = jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    moved.insert_rows("Operations", 1, 1).unwrap();
    assert_eq!(
        moved.workbook().sheets[0].array_formulas[0].anchor,
        at("J2")
    );
    assert_eq!(
        moved.workbook().sheets[0].array_formulas[0].range,
        range("J2:J4")
    );
    assert_eq!(moved.cell("Operations", "J4").unwrap().display, "3");
    moved.undo().unwrap();
    assert_eq!(moved.cell("Operations", "J1").unwrap().display, "1");
    moved.redo().unwrap();
    let saved_moved = moved.save_bytes().unwrap();
    assert_eq!(
        read_xlsx(&saved_moved).unwrap().sheets[0].array_formulas[0].range,
        range("J2:J4")
    );
    let mut book = jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert_eq!(book.cell("Operations", "J1").unwrap().display, "1");
    book.set(
        "Operations",
        "J1",
        jade_xlsx_headless_api::CellInput::Formula("SEQUENCE(4)".into()),
    )
    .unwrap();
    assert_eq!(book.cell("Operations", "J4").unwrap().display, "4");
    book.undo().unwrap();
    assert_eq!(book.cell("Operations", "J4").unwrap().display, "");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    let package = OpcPackage::open(&saved).unwrap();
    let sheet = package.part_str(&part).unwrap().unwrap();
    assert!(sheet.contains("<mc:Choice Requires=\"d\">"));
    assert!(sheet.contains("<mc:Fallback>"));
    let mut reopened =
        jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(reopened.cell("Operations", "J4").unwrap().display, "4");
    reopened
        .set(
            "Operations",
            "J1",
            jade_xlsx_headless_api::CellInput::Formula("SEQUENCE(5)".into()),
        )
        .unwrap();
    assert_eq!(reopened.cell("Operations", "J5").unwrap().display, "5");
}

#[test]
fn processed_formula_caches_and_dynamic_marks_stay_in_their_original_wrappers() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future" mc:Ignorable="u" mc:ProcessContent="u:wrap"><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><u:wrap><c r="A2"><u:wrap xmlns="urn:future"><m:f>A1+1</m:f></u:wrap></c></u:wrap></row></sheetData></worksheet>"#;
    let bytes = common::workbook_with_sheet(sheet);
    let mut book = jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    assert_eq!(book.cell("Sheet1", "A2").unwrap().display, "2");
    book.set(
        "Sheet1",
        "A1",
        jade_xlsx_headless_api::CellInput::Number(5.0),
    )
    .unwrap();
    let saved = book.save_bytes().unwrap();
    let parsed = read_xlsx(&saved).unwrap();
    assert_eq!(
        parsed.sheets[0].rows[1].cells[0].value,
        CellValue::Number(6.0)
    );
    let package = OpcPackage::open(&saved).unwrap();
    let updated = package
        .part_str("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(updated.contains(r#"<m:f>A1+1</m:f><m:v>6</m:v>"#));
    assert_eq!(updated.matches("<u:wrap").count(), 2);
    book.undo().unwrap();
    assert_eq!(book.cell("Sheet1", "A2").unwrap().display, "2");

    let sheet = sheet.replace("<m:f>A1+1</m:f>", "<m:f>SEQUENCE(3)</m:f>");
    let bytes = common::workbook_with_sheet(&sheet);
    let saved = write_spills(
        &bytes,
        &[SheetSpills {
            part_name: "xl/worksheets/sheet1.xml".into(),
            values: vec![CellEdit {
                reference: at("A3"),
                value: NewValue::Number(2.0),
                style_index: None,
            }],
            ranges: vec![ArrayRange {
                anchor: at("A2"),
                range: range("A2:A4"),
            }],
        }],
        false,
    )
    .unwrap();
    let parsed = read_xlsx(&saved).unwrap();
    assert_eq!(
        parsed.sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
    assert_eq!(parsed.sheets[0].array_formulas[0].range, range("A2:A4"));
    let package = OpcPackage::open(&saved).unwrap();
    assert_eq!(
        package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap()
            .matches("<u:wrap")
            .count(),
        2
    );
}

#[test]
fn processed_ignorable_grid_content_reserves_existing_metadata_links() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:future" mc:Ignorable="u" mc:ProcessContent="u:wrap"><sheetData><row r="1"><u:wrap><c r="A1" cm="1"><v>7</v></c></u:wrap></row></sheetData></worksheet>"#;
    let mut package = OpcPackage::open(&common::workbook_with_sheet(sheet)).unwrap();
    assert!(
        matches!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package),
            Err(jade_xlsx_writer::WriteError::ArrayMetadata(_))
        ),
        "an unresolved processed cm link became the new dynamic record"
    );
    assert!(package.part("xl/metadata.xml").is_none());
    assert_eq!(
        package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .as_deref(),
        Some(sheet)
    );
}

#[test]
fn processed_link_reservation_obeys_namespace_scope_and_grid_ownership() {
    let cell = "<c r=\"A1\" cm=\"1\"/>";
    let mut nested = cell.to_string();
    for _ in 0..256 {
        nested = format!("<u:wrap>{nested}</u:wrap>");
    }
    for (grid, blocked) in [
        (format!("<u:wrap><sheetData><row>{cell}</row></sheetData></u:wrap>"), true),
        (format!("<sheetData><u:wrap><row>{cell}</row></u:wrap></sheetData>"), true),
        (format!("<sheetData><row>{nested}</row></sheetData>"), true),
        (format!("<sheetData><row><mc:AlternateContent><mc:Choice Requires=\"u\"><u:wrap>{cell}</u:wrap></mc:Choice><mc:Fallback/></mc:AlternateContent></row></sheetData>"), true),
        (format!("<sheetData><row><u:wrap xmlns:u=\"urn:other\">{cell}</u:wrap></row></sheetData>"), false),
        (format!("<extLst><ext><u:wrap><sheetData><row>{cell}</row></sheetData></u:wrap></ext></extLst>"), false),
    ] {
        let sheet = format!("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:u=\"urn:future\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:wrap\">{grid}</worksheet>");
        let mut package = OpcPackage::open(&common::workbook_with_sheet(&sheet)).unwrap();
        let result = jade_xlsx_writer::spills::dynamic_array_metadata(&mut package);
        if blocked {
            assert!(matches!(result, Err(jade_xlsx_writer::WriteError::ArrayMetadata(_))));
            assert!(package.part("xl/metadata.xml").is_none());
        } else {
            assert!(result.unwrap().is_some());
        }
    }
}

#[test]
fn metadata_checks_opaque_wrappers_at_every_grid_level_and_nested_depth() {
    fn alternate(content: &str) -> String {
        format!(
            r#"<mc:AlternateContent><mc:Choice Requires="q">{content}</mc:Choice><mc:Fallback/></mc:AlternateContent>"#
        )
    }
    let cell = r#"<c r="A1" cm="1"/>"#;
    let mut nested = cell.to_string();
    for _ in 0..256 {
        nested = alternate(&nested);
    }
    for grid in [
        format!("<sheetData><row>{}</row></sheetData>", alternate(cell)),
        format!(
            "<sheetData>{}</sheetData>",
            alternate(&format!("<row>{cell}</row>"))
        ),
        alternate(&format!("<sheetData><row>{cell}</row></sheetData>")),
        format!("<sheetData><row>{nested}</row></sheetData>"),
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:q="urn:future" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">{grid}</worksheet>"#
        );
        let mut package = OpcPackage::open(&common::workbook_with_sheet(&sheet)).unwrap();
        assert!(matches!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package),
            Err(jade_xlsx_writer::WriteError::ArrayMetadata(_))
        ));
        assert!(package.part("xl/metadata.xml").is_none());
        assert_eq!(
            package
                .part_str("xl/worksheets/sheet1.xml")
                .unwrap()
                .unwrap(),
            sheet
        );
    }
}

#[test]
fn foreign_and_extension_links_do_not_block_valid_metadata_creation() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:q="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><sheetData><row r="1"><mc:AlternateContent><mc:Choice Requires="q"><q:c r="A1" cm="1"/></mc:Choice><mc:Fallback><c r="A1"/></mc:Fallback></mc:AlternateContent></row></sheetData></worksheet>"#;
    let sheet = sheet.replace(
        "</worksheet>",
        r#"<extLst><ext uri="keep"><c r="A1" cm="1"/></ext></extLst></worksheet>"#,
    );
    let mut package = OpcPackage::open(&common::workbook_with_sheet(&sheet)).unwrap();
    assert_eq!(
        jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
        Some(1)
    );
    assert_eq!(
        package
            .part_str("xl/worksheets/sheet1.xml")
            .unwrap()
            .unwrap(),
        sheet
    );
}

#[test]
fn metadata_creation_and_append_preserve_links_inside_compatible_branches() {
    for branch in [
        r#"<mc:Choice Requires="m"><c r="A1" cm="1"><f t="array" ref="A1">1</f></c></mc:Choice><mc:Fallback><c r="A1"/></mc:Fallback>"#,
        r#"<mc:Choice Requires="m"><c r="A1"/></mc:Choice><mc:Fallback><c r="A1" cm="1"><f t="array" ref="A1">1</f></c></mc:Fallback>"#,
        r#"<mc:Choice Requires="q"><q:c r="A1" cm="1"/></mc:Choice><mc:Fallback><c r="A1" cm="1"><f t="array" ref="A1">1</f></c></mc:Fallback>"#,
        r#"<mc:Choice Requires="q"><c r="A1" cm="1"><f t="array" ref="A1">1</f></c></mc:Choice><mc:Fallback><c r="A1"/></mc:Fallback>"#,
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:q="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><sheetData><row r="1"><mc:AlternateContent>{branch}</mc:AlternateContent></row></sheetData></worksheet>"#
        );
        let mut package = OpcPackage::open(&common::workbook_with_sheet(&sheet)).unwrap();
        assert!(
            matches!(
                jade_xlsx_writer::spills::dynamic_array_metadata(&mut package),
                Err(jade_xlsx_writer::WriteError::ArrayMetadata(_))
            ),
            "{branch}"
        );
        assert!(package.part("xl/metadata.xml").is_none());
        let (mut package, part) = existing_metadata(
            r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#,
        );
        package.replace(&part, sheet.into_bytes());
        assert_eq!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
            None,
            "{branch}"
        );
        assert_eq!(
            package.part_str("xl/metadata.xml").unwrap().unwrap(),
            r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"/>"#
        );
    }
}

#[test]
fn spill_marking_preserves_foreign_and_inactive_formula_sources() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:q="urn:foreign" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><sheetData><row r="1"><q:c r="J1"><q:f>foreign cell</q:f></q:c><c r="J1"><q:f>foreign formula</q:f><mc:AlternateContent><mc:Choice Requires="q"><f>unsupported choice</f></mc:Choice><mc:Fallback><f>SEQUENCE(3)</f></mc:Fallback></mc:AlternateContent></c></row></sheetData><extLst><ext uri="keep"><c r="J1"><f>extension lookalike</f></c></ext></extLst></worksheet>"#;
    let marked = jade_xlsx_writer::spills::mark_array_formulas(
        xml,
        &[ArrayRange {
            anchor: at("J1"),
            range: range("J1:J3"),
        }],
        Some(1),
    )
    .unwrap();
    let expected = xml
        .replace("<c r=\"J1\"><q:f>", "<c r=\"J1\" cm=\"1\"><q:f>")
        .replace(
            "<f>SEQUENCE(3)</f>",
            "<f t=\"array\" ref=\"J1:J3\">SEQUENCE(3)</f>",
        );
    assert_eq!(marked, expected);
}

#[test]
fn new_metadata_reserves_only_owned_positive_links() {
    for (cells, expected) in [
        (r#"<c r="A1" cm="4294967295"/>"#, 1),
        (r#"<c r="A1" cm="1"/><c r="B1" cm="2"/>"#, 0),
        (r#"<c r="A1" cm="1"/><c r="B1" cm="3"/>"#, 0),
        (r#"<c r="A1" cm="0"/><c r="B1" cm="invalid"/>"#, 1),
        (r#"<f:c r="A1" cm="1"/><c r="B1" f:cm="1"/>"#, 1),
    ] {
        let sheet = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:f="urn:foreign"><f:sheetData><f:row><f:c r="C1" cm="1"/></f:row></f:sheetData><sheetData><row r="1">{cells}</row></sheetData></worksheet>"#
        );
        let mut package = OpcPackage::open(&common::workbook_with_sheet(&sheet)).unwrap();
        if expected == 0 {
            let before = package.part_names().to_vec();
            let rels = package
                .part_str("xl/_rels/workbook.xml.rels")
                .unwrap()
                .unwrap();
            assert!(matches!(
                jade_xlsx_writer::spills::dynamic_array_metadata(&mut package),
                Err(jade_xlsx_writer::WriteError::ArrayMetadata(_))
            ));
            assert_eq!(package.part_names(), before);
            assert_eq!(
                package
                    .part_str("xl/_rels/workbook.xml.rels")
                    .unwrap()
                    .unwrap(),
                rels
            );
            continue;
        }
        assert_eq!(
            jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
            Some(expected),
            "{cells}"
        );
        let metadata = package.part_str("xl/metadata.xml").unwrap().unwrap();
        let flags = jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&metadata).unwrap();
        assert_eq!(flags.len(), expected as usize);
        assert!(flags[..flags.len() - 1].iter().all(Option::is_none));
        assert_eq!(flags.last(), Some(&Some(true)));
    }
}

#[test]
fn creating_metadata_preserves_dangling_links_and_orphan_parts() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" cm="2"><f t="array" ref="A1">1</f><v>1</v></c><c r="B1" cm="4294967295"><f t="array" ref="B1">2</f><v>2</v></c><c r="J1"><f>SEQUENCE(3)</f></c></row></sheetData></worksheet>"#;
    let mut package = OpcPackage::open(&common::workbook_with_sheet(sheet)).unwrap();
    let orphan = b"unrelated orphan metadata bytes";
    package.replace("xl/metadata.xml", orphan.to_vec());
    package.replace("xl/metadata1.xml", b"another orphan".to_vec());
    assert_eq!(
        jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
        Some(1)
    );
    assert_eq!(package.part("xl/metadata.xml").unwrap(), orphan);
    assert_eq!(package.part("xl/metadata1.xml").unwrap(), b"another orphan");
    let metadata = package.part_str("xl/metadata2.xml").unwrap().unwrap();
    assert!(metadata.len() < 2000);
    assert_eq!(
        jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&metadata).unwrap(),
        [Some(true)]
    );
    assert_eq!(
        jade_xlsx_writer::spills::dynamic_array_metadata(&mut package).unwrap(),
        Some(1)
    );
    let written = write_spills(
        &package.write().unwrap(),
        &[spill("xl/worksheets/sheet1.xml", "J1", "J1:J3", &[])],
        false,
    )
    .unwrap();
    let workbook = read_xlsx(&written).unwrap();
    let arrays = &workbook.sheets[0].array_formulas;
    assert!(arrays
        .iter()
        .filter(|a| a.anchor == at("A1") || a.anchor == at("B1"))
        .all(|a| a.kind == jade_xlsx_reader::ArrayFormulaKind::Unknown));
    assert!(arrays
        .iter()
        .any(|a| a.anchor == at("J1") && a.kind == jade_xlsx_reader::ArrayFormulaKind::Dynamic));
}

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
