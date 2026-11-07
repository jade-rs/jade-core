use jade_opc::OpcPackage;
use jade_xlsx_reader::{read_xlsx, CellRange, CellRef};
use jade_xlsx_writer::spills::{write_spills, ArrayRange, SheetSpills};
use jade_xlsx_writer::WriteError;

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

fn branch(content: &str) -> String {
    format!("<mc:AlternateContent><mc:Choice Requires=\"m\">{content}</mc:Choice><mc:Fallback>{content}</mc:Fallback></mc:AlternateContent>")
}

fn write(metadata: &str, dynamic: bool) -> Result<Vec<u8>, WriteError> {
    let source = include_bytes!("../../../public/fixtures/grid-operations-workbook.xlsx");
    let part_name = read_xlsx(source).unwrap().sheets[0].part_name.clone();
    let bytes = jade_xlsx_writer::write_xlsx(
        source,
        &[jade_xlsx_writer::SheetEdits {
            part_name: part_name.clone(),
            cells: vec![jade_xlsx_writer::CellEdit {
                reference: CellRef::parse("J1").unwrap(),
                value: jade_xlsx_writer::NewValue::Formula("SEQUENCE(3)".into()),
                style_index: None,
            }],
        }],
    )
    .unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    package.replace("xl/metadata.xml", metadata.as_bytes().to_vec());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace("</Relationships>", r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#);
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    write_spills(
        &package.write().unwrap(),
        &[SheetSpills {
            part_name,
            values: vec![],
            ranges: if dynamic {
                vec![ArrayRange {
                    anchor: CellRef::parse("J1").unwrap(),
                    range: CellRange::parse("J1:J3").unwrap(),
                }]
            } else {
                vec![]
            },
        }],
        false,
    )
}

#[test]
fn a_composite_dynamic_record_is_not_attached_to_a_new_spill() {
    let bytes = write(&format!("<metadata xmlns=\"{NS}\"/>"), true).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let metadata = package
        .part_str("xl/metadata.xml")
        .unwrap()
        .unwrap()
        .replacen("count=\"1\"", "count=\"2\"", 1)
        .replace(
            "</metadataTypes>",
            "<metadataType name=\"OTHER\"/></metadataTypes>",
        )
        .replace(
            "<rc t=\"1\" v=\"0\"/>",
            "<rc t=\"1\" v=\"0\"/><rc t=\"2\" v=\"0\"/>",
        );
    assert_eq!(
        jade_xlsx_reader::array_metadata::first_dynamic_array_cell_metadata(&metadata).unwrap(),
        Some(1)
    );
    let bytes = write(&metadata, true).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert!(updated.contains("<rc t=\"1\" v=\"0\"/><rc t=\"2\" v=\"0\"/>"));
    let cells = updated.find("<cellMetadata").unwrap();
    assert!(updated[cells..cells + updated[cells..].find('>').unwrap()].contains("count=\"2\""));
    assert_eq!(
        jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated).unwrap(),
        Some(2)
    );
    let book = read_xlsx(&bytes).unwrap();
    let sheet = package
        .part_str(&book.sheets[0].part_name)
        .unwrap()
        .unwrap();
    assert!(sheet.contains("cm=\"2\""));
    assert_eq!(
        book.sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
}

#[test]
fn shared_dynamic_metadata_is_reused_without_rewriting_compatibility_branches() {
    let bytes = write(&format!("<metadata xmlns=\"{NS}\"/>"), true).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let metadata = package.part_str("xl/metadata.xml").unwrap().unwrap();
    let metadata = metadata.replacen(
        "<metadata ",
        &format!("<metadata xmlns:mc=\"{MC}\" xmlns:m=\"{NS}\" "),
        1,
    );
    for (start, end) in [
        ("<metadataTypes", "</metadataTypes>"),
        ("<futureMetadata", "</futureMetadata>"),
        ("<cellMetadata", "</cellMetadata>"),
        ("<dynamicArrayProperties", "/>"),
    ] {
        let from = metadata.find(start).unwrap();
        let to = from + metadata[from..].find(end).unwrap() + end.len();
        let xml = format!(
            "{}{}{}",
            &metadata[..from],
            branch(&metadata[from..to]),
            &metadata[to..]
        );
        for xml in [
            xml.clone(),
            xml.replace("Requires=\"m\"", "Requires=\"unknown\""),
        ] {
            let updated =
                write(&xml, true).expect("all stored alternatives retain the same dynamic link");
            assert_eq!(
                OpcPackage::open(&updated)
                    .unwrap()
                    .part_str("xl/metadata.xml")
                    .unwrap()
                    .as_deref(),
                Some(xml.as_str())
            );
            assert_eq!(
                read_xlsx(&updated).unwrap().sheets[0].array_formulas[0].kind,
                jade_xlsx_reader::ArrayFormulaKind::Dynamic
            );
        }
    }
}

#[test]
fn workbook_import_uses_the_selected_metadata_dictionary() {
    let bytes = write(&format!("<metadata xmlns=\"{NS}\"/>"), true).unwrap();
    let mut package = OpcPackage::open(&bytes).unwrap();
    let metadata = package.part_str("xl/metadata.xml").unwrap().unwrap();
    let start = metadata.find("<metadata ").unwrap();
    let head_end = start + metadata[start..].find('>').unwrap() + 1;
    let close = metadata.rfind("</metadata>").unwrap();
    let content = &metadata[head_end..close];
    let alternate = format!("{}<mc:AlternateContent xmlns:mc=\"{MC}\" xmlns:m=\"{NS}\"><mc:Choice Requires=\"m\">{content}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>{}", &metadata[..head_end], content.replace("fDynamic=\"1\"", "fDynamic=\"0\""), &metadata[close..]);
    for (xml, kind) in [
        (
            alternate.clone(),
            jade_xlsx_reader::ArrayFormulaKind::Dynamic,
        ),
        (
            alternate.replace("Requires=\"m\"", "Requires=\"missing\""),
            jade_xlsx_reader::ArrayFormulaKind::Legacy,
        ),
    ] {
        package.replace("xl/metadata.xml", xml.into_bytes());
        let workbook = read_xlsx(&package.write().unwrap()).unwrap();
        let array = workbook.sheets[0]
            .array_formulas
            .iter()
            .find(|array| array.anchor == CellRef::parse("J1").unwrap())
            .unwrap();
        assert_eq!(array.kind, kind);
        assert_eq!(array.range, CellRange::parse("J1:J3").unwrap());
    }
}

#[test]
fn conditional_metadata_indexes_are_not_overwritten_by_direct_dictionaries() {
    let types = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes>";
    let cells = "<cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let future = "<futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata>";
    for content in [
        branch(&format!("{types}{future}{cells}")),
        format!("{}{future}{cells}", branch(types)),
        format!("{types}{future}{}", branch(cells)),
        format!(
            "{types}{future}<cellMetadata count=\"1\">{}</cellMetadata>",
            branch("<bk><rc t=\"1\" v=\"0\"/></bk>")
        ),
        format!(
            "{types}{future}<cellMetadata count=\"1\"><bk>{}</bk></cellMetadata>",
            branch("<rc t=\"1\" v=\"0\"/>")
        ),
        format!(
            "<metadataTypes count=\"1\">{}</metadataTypes>{future}{cells}",
            branch("<metadataType name=\"OTHER\"/>")
        ),
        format!(
            "{types}{cells}<futureMetadata name=\"XLDAPR\" count=\"1\">{}</futureMetadata>",
            branch("<bk/>")
        ),
        format!(
            "{types}{future}<valueMetadata count=\"1\"><bk>{}</bk></valueMetadata>",
            branch("<rc t=\"1\" v=\"0\"/>")
        ),
    ] {
        let metadata = format!(
            "<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\">{content}</metadata>"
        );
        let result = write(&metadata, true);
        assert!(
            matches!(result, Err(WriteError::ArrayMetadata(_))),
            "conditional metadata was modified: {metadata}"
        );
        let cached = write(&metadata, false).unwrap();
        assert_eq!(
            OpcPackage::open(&cached)
                .unwrap()
                .part_str("xl/metadata.xml")
                .unwrap()
                .as_deref(),
            Some(metadata.as_str())
        );
    }
}

#[test]
fn direct_dynamic_links_are_not_reused_without_checking_conditional_siblings() {
    let properties = "<d:dynamicArrayProperties fDynamic=\"1\"/>";
    let extension =
        format!("<ext uri=\"{{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}}\">{properties}</ext>");
    for extra in [branch("<metadataType name=\"OTHER\"/>"), String::new()] {
        let extension = if extra.is_empty() {
            format!(
                "<ext uri=\"{{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}}\">{properties}{}</ext>",
                branch("<d:dynamicArrayProperties fDynamic=\"0\"/>")
            )
        } else {
            extension.clone()
        };
        let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\" xmlns:d=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"><metadataTypes><metadataType name=\"XLDAPR\"/>{extra}</metadataTypes><futureMetadata name=\"XLDAPR\"><bk><extLst>{extension}</extLst></bk></futureMetadata><cellMetadata><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata></metadata>");
        assert_eq!(
            jade_xlsx_reader::array_metadata::first_dynamic_array_cell_metadata(&metadata).unwrap(),
            if extra.is_empty() { None } else { Some(1) }
        );
        if extra.is_empty() {
            assert!(matches!(
                write(&metadata, true),
                Err(WriteError::ArrayMetadata(_))
            ));
        } else {
            let bytes = write(&metadata, true).unwrap();
            assert_eq!(
                OpcPackage::open(&bytes)
                    .unwrap()
                    .part_str("xl/metadata.xml")
                    .unwrap()
                    .as_deref(),
                Some(metadata.as_str())
            );
        }
    }
}

#[test]
fn unrelated_extension_branches_and_foreign_lookalikes_do_not_block_append() {
    for content in [
        format!("<extLst><ext uri=\"urn:test\">{}</ext></extLst>", branch("<metadataTypes/>")),
        "<foreign:AlternateContent xmlns:foreign=\"urn:foreign\"><metadataTypes/></foreign:AlternateContent>".into(),
    ] {
        let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\">{content}</metadata>");
        let bytes = write(&metadata, true).unwrap();
        let package = OpcPackage::open(&bytes).unwrap();
        let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
        assert!(updated.contains(&content));
        assert_eq!(jade_xlsx_reader::array_metadata::first_dynamic_array_cell_metadata(&updated).unwrap(), Some(1));
    }
}
