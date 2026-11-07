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
    write_with_cm(metadata, dynamic, None)
}

#[test]
fn processed_metadata_dictionaries_append_without_duplicate_containers() {
    let content = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    for content in [content.to_string(), String::new()] {
        let wrapper = if content.is_empty() {
            "<u:wrap keep=\"yes\"/>".to_string()
        } else {
            format!("<u:wrap keep=\"yes\">{content}</u:wrap>")
        };
        let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:mc=\"{MC}\" xmlns:u=\"urn:future\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:wrap\">{wrapper}</metadata>");
        let bytes = write(&metadata, true).unwrap();
        let package = OpcPackage::open(&bytes).unwrap();
        let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
        assert!(updated.contains("<u:wrap keep=\"yes\">"));
        assert_eq!(updated.matches("<metadataTypes ").count(), 1);
        assert_eq!(updated.matches("<cellMetadata ").count(), 1);
        assert_eq!(
            jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated).unwrap(),
            Some(if content.is_empty() { 1 } else { 2 })
        );
        assert_eq!(
            read_xlsx(&bytes).unwrap().sheets[0].array_formulas[0].kind,
            jade_xlsx_reader::ArrayFormulaKind::Dynamic
        );
    }
}

fn write_with_cm(metadata: &str, dynamic: bool, cm: Option<u32>) -> Result<Vec<u8>, WriteError> {
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
    if let Some(cm) = cm {
        let sheet = package.part_str(&part_name).unwrap().unwrap();
        assert!(sheet.contains("<c r=\"J1\""));
        package.replace(
            &part_name,
            sheet
                .replace("<c r=\"J1\"", &format!("<c r=\"J1\" cm=\"{cm}\""))
                .into_bytes(),
        );
    }
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
fn typing_into_a_workbook_with_conditional_metadata_keeps_history_and_reopen() {
    let content = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let metadata = format!(
        "<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\">{}</metadata>",
        branch(content)
    );
    let bytes = write(&metadata, false).unwrap();
    let mut book = jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = book.bytes().to_vec();
    book.set(
        "Operations",
        "L1",
        jade_xlsx_headless_api::CellInput::Formula("SEQUENCE(3)".into()),
    )
    .unwrap();
    assert_eq!(
        ["L1", "L2", "L3"].map(|cell| book.cell("Operations", cell).unwrap().display),
        ["1", "2", "3"]
    );
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    let mut reopened =
        jade_xlsx_headless_editor::HeadlessWorkbook::open_for_editing(&saved).unwrap();
    reopened
        .set(
            "Operations",
            "L1",
            jade_xlsx_headless_api::CellInput::Formula("SEQUENCE(4)".into()),
        )
        .unwrap();
    assert_eq!(reopened.cell("Operations", "L4").unwrap().display, "4");
    let book = read_xlsx(reopened.bytes()).unwrap();
    assert!(book.sheets[0]
        .array_formulas
        .iter()
        .any(|array| array.anchor == CellRef::parse("L1").unwrap()
            && array.kind == jade_xlsx_reader::ArrayFormulaKind::Dynamic));
}

#[test]
fn nested_metadata_alternatives_are_extended_without_recursive_rewriting() {
    let content = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let mut nested = content.to_string();
    for _ in 0..256 {
        nested = format!("<mc:AlternateContent><mc:Choice Requires=\"m\">{nested}</mc:Choice><mc:Fallback>{content}</mc:Fallback></mc:AlternateContent>");
    }
    let metadata =
        format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\">{nested}</metadata>");
    let bytes = write(&metadata, true).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert_eq!(updated.matches("<mc:AlternateContent>").count(), 256);
    assert_eq!(
        jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated).unwrap(),
        Some(2)
    );
}

#[test]
fn appended_padding_can_share_an_existing_dynamic_index_in_another_branch() {
    let first = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let second = "<metadataTypes count=\"2\"><metadataType name=\"OTHER\"/><metadataType name=\"XLDAPR\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst><ext uri=\"{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}\"><d:dynamicArrayProperties fDynamic=\"1\"/></ext></extLst></bk></futureMetadata><cellMetadata count=\"2\"><bk><rc t=\"1\" v=\"0\"/></bk><bk><rc t=\"2\" v=\"0\"/></bk></cellMetadata>";
    let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\" xmlns:d=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"><mc:AlternateContent><mc:Choice Requires=\"m\">{first}</mc:Choice><mc:Fallback>{second}</mc:Fallback></mc:AlternateContent></metadata>");
    let bytes = write(&metadata, true).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert_eq!(
        jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated).unwrap(),
        Some(2)
    );
    let book = read_xlsx(&bytes).unwrap();
    assert_eq!(
        book.sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
}

#[test]
fn conditional_append_preserves_false_flags_and_refuses_dangling_future_records() {
    let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\" xmlns:d=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"><metadataTypes count=\"1\"><metadataType name=\"XLDAPR\"/></metadataTypes><futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst><ext uri=\"{{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}}\">{}</ext></extLst></bk></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata></metadata>", branch("<d:dynamicArrayProperties fDynamic=\"false\"/>"));
    let bytes = write_with_cm(&metadata, true, Some(1)).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert_eq!(
        jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&updated).unwrap(),
        [Some(false), Some(true)]
    );
    assert!(updated.contains(&branch("<d:dynamicArrayProperties fDynamic=\"false\"/>")));
    let dangling = metadata.replace("<rc t=\"1\" v=\"0\"/>", "<rc t=\"1\" v=\"1\"/>");
    assert!(matches!(
        write(&dangling, true),
        Err(WriteError::ArrayMetadata(_))
    ));
}

#[test]
fn branch_local_prefixes_and_foreign_default_namespaces_survive_append() {
    let content = "<m:metadataTypes count=\"1\"><m:metadataType name=\"OTHER\"/></m:metadataTypes>";
    let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:mc=\"{MC}\"><mc:AlternateContent><mc:Choice xmlns:m=\"{NS}\" xmlns=\"urn:foreign\" Requires=\"m\">{content}</mc:Choice><mc:Fallback/></mc:AlternateContent></metadata>");
    let bytes = write(&metadata, true).unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert!(updated.contains("xmlns=\"urn:foreign\" Requires=\"m\""));
    assert!(updated.contains("<m:metadataType name=\"OTHER\"/>"));
    assert_eq!(
        jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated).unwrap(),
        Some(1)
    );
    let empty_choice = metadata.replace(&format!(">{content}</mc:Choice>"), "/>");
    let bytes = write(&empty_choice, true).unwrap();
    assert_eq!(
        read_xlsx(&bytes).unwrap().sheets[0].array_formulas[0].kind,
        jade_xlsx_reader::ArrayFormulaKind::Dynamic
    );
}

#[test]
fn empty_selected_metadata_branches_gain_records_without_namespace_loss() {
    for empty in ["<mc:Fallback/>", "<mc:Fallback></mc:Fallback>"] {
        let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\"><mc:AlternateContent><mc:Choice Requires=\"m\"><metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes></mc:Choice>{empty}</mc:AlternateContent></metadata>");
        for metadata in [
            metadata.clone(),
            metadata.replace(NS, "http://purl.oclc.org/ooxml/spreadsheetml/main"),
        ] {
            let bytes = write(&metadata, true).unwrap();
            let package = OpcPackage::open(&bytes).unwrap();
            let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
            assert_eq!(
                jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated)
                    .unwrap(),
                Some(1)
            );
        }
    }
}

#[test]
fn conditional_append_does_not_activate_dangling_cell_links_or_conflicting_shared_records() {
    let types = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes>";
    let future = "<futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata>";
    let cells = "<cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let whole = format!(
        "<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\">{}</metadata>",
        branch(&format!("{types}{future}{cells}"))
    );
    assert!(matches!(
        write_with_cm(&whole, true, Some(2)),
        Err(WriteError::ArrayMetadata(_))
    ));
    let other_types = types.replace("count=\"1\"", "count=\"2\"").replace(
        "</metadataTypes>",
        "<metadataType name=\"SECOND\"/></metadataTypes>",
    );
    let conflict = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\"><mc:AlternateContent><mc:Choice Requires=\"m\">{types}</mc:Choice><mc:Fallback>{other_types}</mc:Fallback></mc:AlternateContent>{future}{cells}</metadata>");
    assert!(matches!(
        write(&conflict, true),
        Err(WriteError::ArrayMetadata(_))
    ));
    let no_fallback = whole.replace(
        &format!("<mc:Fallback>{types}{future}{cells}</mc:Fallback>"),
        "",
    );
    assert!(matches!(
        write(&no_fallback, true),
        Err(WriteError::ArrayMetadata(_))
    ));
}

#[test]
fn a_new_dynamic_record_is_appended_to_every_metadata_alternative() {
    let first = "<metadataTypes count=\"1\"><metadataType name=\"OTHER\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let second = "<metadataTypes count=\"2\"><metadataType name=\"OTHER\"/><metadataType name=\"SECOND\"/></metadataTypes><futureMetadata name=\"OTHER\" count=\"1\"><bk/></futureMetadata><cellMetadata count=\"2\"><bk><rc t=\"1\" v=\"0\"/></bk><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let metadata = format!("<metadata xmlns=\"{NS}\" xmlns:m=\"{NS}\" xmlns:mc=\"{MC}\"><mc:AlternateContent><mc:Choice Requires=\"m\">{first}</mc:Choice><mc:Fallback>{second}</mc:Fallback></mc:AlternateContent></metadata>");
    let bytes =
        write(&metadata, true).expect("valid alternatives must accept a new dynamic record");
    let mut package = OpcPackage::open(&bytes).unwrap();
    let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
    assert!(updated.contains("<bk><rc t=\"1\" v=\"0\"/></bk>"));
    assert_eq!(
        jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated).unwrap(),
        Some(3)
    );
    for (xml, expected) in [
        (updated.clone(), vec![None, Some(true), Some(true)]),
        (
            updated.replace("Requires=\"m\"", "Requires=\"unknown\""),
            vec![None, None, Some(true)],
        ),
    ] {
        assert_eq!(
            jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&xml).unwrap(),
            expected
        );
        package.replace("xl/metadata.xml", xml.into_bytes());
        let book = read_xlsx(&package.write().unwrap()).unwrap();
        assert_eq!(
            book.sheets[0].array_formulas[0].kind,
            jade_xlsx_reader::ArrayFormulaKind::Dynamic
        );
    }
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
            "{types}<futureMetadata name=\"XLDAPR\" count=\"1\">{}</futureMetadata>{cells}",
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
        let result = write(&metadata, true).unwrap_or_else(|error| panic!("{error}: {metadata}"));
        let package = OpcPackage::open(&result).unwrap();
        let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
        assert!(updated.contains("<mc:AlternateContent>"));
        assert!(updated.contains("<metadataType name=\"OTHER\"/>"));
        assert!(
            jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated)
                .unwrap()
                .is_some()
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
            let bytes = write(&metadata, true).unwrap();
            let package = OpcPackage::open(&bytes).unwrap();
            let updated = package.part_str("xl/metadata.xml").unwrap().unwrap();
            assert_eq!(
                jade_xlsx_reader::array_metadata::cell_dynamic_array_flags(&updated).unwrap(),
                [None, Some(true)]
            );
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
