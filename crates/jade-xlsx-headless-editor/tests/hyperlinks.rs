use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, Hyperlink};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const SHEET: &str = "xl/worksheets/sheet1.xml";
fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}
fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open(&fixture()).unwrap()
}
fn link(range: &str, target: Option<&str>, location: Option<&str>) -> Hyperlink {
    Hyperlink {
        reference: CellRange::parse(range).unwrap(),
        target: target.map(str::to_string),
        location: location.map(str::to_string),
        display: Some("リンク & <表示>".into()),
        tooltip: Some("案内\n\t_x000A_\u{1}".into()),
    }
}
fn xml_book(xml: &str, rels: &str) -> HeadlessWorkbook {
    let mut p = OpcPackage::open(&fixture()).unwrap();
    p.replace(SHEET, xml.as_bytes().to_vec());
    p.replace(&jade_opc::rels_part_name(SHEET), rels.as_bytes().to_vec());
    HeadlessWorkbook::open(&p.write().unwrap()).unwrap()
}
#[test]
fn external_internal_named_links_round_trip_and_history_preserves_parts() {
    let mut book = open();
    let name = book.workbook().sheets[0].name.clone();
    let links = [
        link("H2", Some("https://example.com/?a=1&b=東京"), None),
        link("H3", None, Some("'Operations'!$B$2")),
        link("H4", None, Some("Destination")),
        link("H5", Some("../other.xlsx"), Some("'Sheet 2'!A1")),
    ];
    for l in &links {
        book.set_hyperlink(&name, l).unwrap();
        assert_eq!(
            book.workbook().sheets[0].hyperlink_at(l.reference.start),
            Some(l)
        );
    }
    let reopened = HeadlessWorkbook::open(&book.save_bytes().unwrap()).unwrap();
    for l in &links {
        assert_eq!(
            reopened.workbook().sheets[0].hyperlink_at(l.reference.start),
            Some(l)
        );
    }
    let package = OpcPackage::open(book.bytes()).unwrap();
    let rels = jade_opc::parse_relationships(
        &package
            .required_part_str(&jade_opc::rels_part_name(SHEET))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(rels.by_type_suffix("/hyperlink").len(), 2);
    assert!(rels.by_type_suffix("/hyperlink").iter().all(|r| r.external));
    book.clear_hyperlinks(&name, &[CellRange::parse("H2:H5").unwrap()])
        .unwrap();
    assert!(book.workbook().sheets[0].hyperlinks.is_empty());
    book.undo().unwrap();
    assert_eq!(book.workbook().sheets[0].hyperlinks.len(), 4);
    book.redo().unwrap();
    assert!(book.workbook().sheets[0].hyperlinks.is_empty());
    let before = OpcPackage::open(&fixture()).unwrap();
    let after = OpcPackage::open(book.bytes()).unwrap();
    assert_eq!(
        before
            .required_part_str("xl/worksheets/sheet2.xml")
            .unwrap(),
        after.required_part_str("xl/worksheets/sheet2.xml").unwrap()
    );
}
#[test]
fn partial_replacement_and_removal_keep_outside_links_and_shared_relationship() {
    let mut book = open();
    let name = book.workbook().sheets[0].name.clone();
    let original = link("H2:J6", Some("https://example.com"), None);
    book.set_hyperlink(&name, &original).unwrap();
    let replacement = link("I3", None, Some("A1"));
    book.set_hyperlink(&name, &replacement).unwrap();
    for at in ["H2", "J6", "I2", "I4"] {
        assert_eq!(
            book.cell(&name, at)
                .unwrap()
                .hyperlink
                .unwrap()
                .target
                .as_deref(),
            Some("https://example.com")
        );
    }
    assert_eq!(
        book.cell(&name, "I3")
            .unwrap()
            .hyperlink
            .unwrap()
            .location
            .as_deref(),
        Some("A1")
    );
    book.clear_hyperlinks(&name, &[CellRange::parse("H2:J6").unwrap()])
        .unwrap();
    let package = OpcPackage::open(book.bytes()).unwrap();
    let rels = jade_opc::parse_relationships(
        &package
            .required_part_str(&jade_opc::rels_part_name(SHEET))
            .unwrap(),
    )
    .unwrap();
    assert!(rels.by_type_suffix("/hyperlink").is_empty());
}
#[test]
fn aliases_strict_relationships_foreign_metadata_and_shared_ids_are_safe() {
    let main = "http://purl.oclc.org/ooxml/spreadsheetml/main";
    let rel = "http://purl.oclc.org/ooxml/officeDocument/relationships";
    let xml = format!(
        r#"<s:worksheet xmlns:s="{main}" xmlns:a="{main}" xmlns:q="{rel}" xmlns:f="urn:foreign"><s:sheetData/><a:hyperlinks f:settings="keep"><f:hyperlink ref="Z1"/><s:hyperlink ref="H2" q:id="same" f:tag="keep" tooltip="old"><f:child/></s:hyperlink><s:hyperlink ref="H3" q:id="same"/></a:hyperlinks><s:pageMargins left="0.7"/><s:extLst><f:thing q:id="same"/><f:hyperlink ref="Z2"/></s:extLst></s:worksheet>"#
    );
    let rels = format!(
        r#"<p:Relationships xmlns:p="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:f="urn:foreign"><p:Relationship Id="same" Type="{rel}/hyperlink" Target="https://old.example" TargetMode="External" f:keep="yes"/><p:Relationship Id="rId1" Type="{rel}/image" Target="../media/image1.png"/><f:Relationship Id="foreign"/></p:Relationships>"#
    );
    let mut book = xml_book(&xml, &rels);
    let name = book.workbook().sheets[0].name.clone();
    assert_eq!(book.workbook().sheets[0].hyperlinks.len(), 2);
    book.set_hyperlink(&name, &link("H2", Some("https://new.example"), None))
        .unwrap();
    let p = OpcPackage::open(book.bytes()).unwrap();
    let written = p.required_part_str(SHEET).unwrap();
    for keep in [
        "f:settings=\"keep\"",
        "f:tag=\"keep\"",
        "<f:child/>",
        "<f:hyperlink ref=\"Z1\"/>",
        "<f:hyperlink ref=\"Z2\"/>",
    ] {
        assert!(written.contains(keep), "{written}");
    }
    let rels = p
        .required_part_str(&jade_opc::rels_part_name(SHEET))
        .unwrap();
    assert!(rels.contains("p:Relationship"));
    let parsed = jade_opc::parse_relationships(&rels).unwrap();
    assert_eq!(
        parsed.relationship("rId2").unwrap().relationship_type,
        format!("{rel}/hyperlink")
    );
    assert!(parsed.relationship("same").is_some());
    assert_eq!(
        book.cell(&name, "H2")
            .unwrap()
            .hyperlink
            .unwrap()
            .target
            .as_deref(),
        Some("https://new.example")
    );
    book.clear_hyperlinks(&name, &[CellRange::parse("H3").unwrap()])
        .unwrap();
    let p = OpcPackage::open(book.bytes()).unwrap();
    assert!(p
        .required_part_str(&jade_opc::rels_part_name(SHEET))
        .unwrap()
        .contains("f:keep=\"yes\""));
}
#[test]
fn insertion_writes_display_and_style_as_one_step_but_keeps_formulas() {
    let mut book = open();
    let name = book.workbook().sheets[0].name.clone();
    let at = link("H2", Some("https://example.com"), None);
    let before = book.bytes().to_vec();
    book.insert_hyperlink(&name, &at, Some("サイト")).unwrap();
    assert_eq!(book.cell(&name, "H2").unwrap().display, "サイト");
    let format = book.format_at(&name, "H2").unwrap();
    assert_eq!(format.underline, Some(true));
    assert_eq!(format.font_color.as_deref(), Some("0563C1"));
    // The theme's hyperlink colour, kept as a theme reference.
    let styles = OpcPackage::open(book.bytes())
        .unwrap()
        .required_part_str("xl/styles.xml")
        .unwrap();
    assert!(styles.contains("<color theme=\"10\"/>"), "{styles}");
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    assert_eq!(book.cell(&name, "H2").unwrap().display, "サイト");
    book.set(&name, "H3", CellInput::Formula("1+2".into()))
        .unwrap();
    book.insert_hyperlink(&name, &link("H3", None, Some("A1")), Some("replace"))
        .unwrap();
    let cell = book.cell(&name, "H3").unwrap();
    assert_eq!(cell.formula.as_deref(), Some("1+2"));
    assert_eq!(cell.display, "3");
    book.clear_hyperlinks(&name, &[CellRange::parse("H2:H3").unwrap()])
        .unwrap();
    assert_eq!(book.cell(&name, "H2").unwrap().display, "サイト");
    assert_eq!(book.format_at(&name, "H2").unwrap().underline, Some(true));
}
#[test]
fn empty_roots_schema_order_id_reuse_and_invalid_input_are_atomic() {
    let mut book = xml_book(
        &format!(r#"<worksheet xmlns="{MAIN}"/>"#),
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>",
    );
    let name = book.workbook().sheets[0].name.clone();
    let at = link("A1", Some("mailto:a@example.com"), None);
    book.set_hyperlink(&name, &at).unwrap();
    book.set_hyperlink(&name, &link("A2", Some("mailto:a@example.com"), None))
        .unwrap();
    let p = OpcPackage::open(book.bytes()).unwrap();
    let rels = jade_opc::parse_relationships(
        &p.required_part_str(&jade_opc::rels_part_name(SHEET))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(rels.by_type_suffix("/hyperlink").len(), 1);
    let before = book.bytes().to_vec();
    let mut bad = at.clone();
    bad.target = Some("a\u{1}".into());
    assert!(book
        .insert_hyperlink(&name, &bad, Some("bad"))
        .unwrap_err()
        .is_invalid_input());
    assert_eq!(book.bytes(), before);
    bad.target = None;
    assert!(book.set_hyperlink(&name, &bad).is_err());
    assert_eq!(book.bytes(), before);
    let xml = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData/><dataValidations count="0"/><pageMargins left="0.7"/></worksheet>"#
    );
    let mut book = xml_book(&xml, "<Relationships/>");
    book.set_hyperlink(&name, &at).unwrap();
    let p = OpcPackage::open(book.bytes()).unwrap();
    let xml = p.required_part_str(SHEET).unwrap();
    assert!(xml.find("dataValidations").unwrap() < xml.find("hyperlinks").unwrap());
    assert!(xml.find("hyperlinks").unwrap() < xml.find("pageMargins").unwrap());
}
#[test]
fn protected_hyperlinks_honor_insert_permission_and_cell_lock() {
    let xml = format!(
        r#"<worksheet xmlns="{MAIN}"><sheetData/><sheetProtection sheet="1" insertHyperlinks="1"/></worksheet>"#
    );
    let mut book = xml_book(
        &xml,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
    );
    let name = book.workbook().sheets[0].name.clone();
    let before = book.bytes().to_vec();
    assert!(book
        .set_hyperlink(&name, &link("A1", Some("https://example.com"), None))
        .is_err());
    assert_eq!(book.bytes(), before);
}

#[test]
fn permitted_links_still_require_unlocked_cells_and_keep_the_lock_style() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/protected-sheets-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let xml = p.required_part_str(SHEET).unwrap().replace(
        "<sheetProtection",
        "<sheetProtection insertHyperlinks=\"0\"",
    );
    p.replace(SHEET, xml.into_bytes());
    let mut book = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    book.insert_hyperlink(
        "Locked",
        &link("B1", Some("https://example.com"), None),
        None,
    )
    .unwrap();
    assert_eq!(book.cell("Locked", "B1").unwrap().number, Some(20.));
    let style_index = book.workbook().sheets[0]
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find(|c| c.reference == jade_xlsx_reader::CellRef::parse("B1").unwrap())
        .unwrap()
        .style_index
        .unwrap();
    let style = book.workbook().styles.cell_formats[style_index as usize].unlocked;
    assert!(style);
    let before = book.bytes().to_vec();
    assert!(book
        .set_hyperlink("Locked", &link("A1", Some("https://example.com"), None))
        .is_err());
    assert_eq!(book.bytes(), before);
    book.clear_hyperlinks("Locked", &[CellRange::parse("B1").unwrap()])
        .unwrap();
    assert!(book.cell("Locked", "B1").unwrap().hyperlink.is_none());
}

#[test]
fn styling_a_link_over_spilled_values_keeps_the_array_formula_and_outputs() {
    let mut book = open();
    let name = book.workbook().sheets[0].name.clone();
    book.set(&name, "H2", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    assert_eq!(book.cell(&name, "H4").unwrap().number, Some(3.));
    book.insert_hyperlink(
        &name,
        &link("H1:H4", Some("https://example.com"), None),
        None,
    )
    .unwrap();
    assert_eq!(
        book.cell(&name, "H2").unwrap().formula.as_deref(),
        Some("SEQUENCE(3)")
    );
    for (at, n) in [("H2", 1.), ("H3", 2.), ("H4", 3.)] {
        assert_eq!(book.cell(&name, at).unwrap().number, Some(n));
    }
    let saved = book.save_bytes().unwrap();
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.cell(&name, "H4").unwrap().number, Some(3.));
}

#[test]
fn explicit_label_blocking_a_spill_clears_other_styled_outputs() {
    let mut book = open();
    let name = book.workbook().sheets[0].name.clone();
    book.set(&name, "H2", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    book.insert_hyperlink(
        &name,
        &link("H3:H4", Some("https://example.com"), None),
        Some("block"),
    )
    .unwrap();
    assert_eq!(book.cell(&name, "H2").unwrap().display, "#SPILL!");
    assert_eq!(book.cell(&name, "H3").unwrap().display, "block");
    assert_eq!(book.cell(&name, "H4").unwrap().display, "");
    assert_eq!(book.format_at(&name, "H4").unwrap().underline, Some(true));
}

#[test]
fn root_namespace_selects_relationship_type_and_repeated_edits_reuse_binding() {
    let xml = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:f="urn:foreign" f:uri="http://purl.oclc.org/ooxml/spreadsheetml/main"><sheetData/></worksheet>"#
    );
    let mut book = xml_book(
        &xml,
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>",
    );
    let name = book.workbook().sheets[0].name.clone();
    for destination in [
        "https://one.example",
        "https://two.example",
        "https://three.example",
    ] {
        book.set_hyperlink(&name, &link("H2", Some(destination), None))
            .unwrap();
    }
    let p = OpcPackage::open(book.bytes()).unwrap();
    let xml = p.required_part_str(SHEET).unwrap();
    assert_eq!(xml.matches("xmlns:jadeLink").count(), 1);
    let rels = jade_opc::parse_relationships(
        &p.required_part_str(&jade_opc::rels_part_name(SHEET))
            .unwrap(),
    )
    .unwrap();
    let links = rels.by_type_suffix("/hyperlink");
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].relationship_type,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink"
    );
}

#[test]
fn authoring_a_legacy_unbound_relationship_prefix_repairs_that_link_only() {
    let xml = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:f="urn:foreign"><sheetData/><hyperlinks><hyperlink ref="H2" r:id="old"/><hyperlink ref="H3" f:id="old"/></hyperlinks></worksheet>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="old" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://old.example" TargetMode="External"/></Relationships>"#;
    let mut book = xml_book(&xml, rels);
    let name = book.workbook().sheets[0].name.clone();
    assert_eq!(
        book.cell(&name, "H2")
            .unwrap()
            .hyperlink
            .unwrap()
            .target
            .as_deref(),
        Some("https://old.example")
    );
    assert_eq!(
        book.cell(&name, "H3").unwrap().hyperlink.unwrap().target,
        None
    );
    book.set_hyperlink(&name, &link("H2", Some("https://new.example"), None))
        .unwrap();
    let p = OpcPackage::open(book.bytes()).unwrap();
    let xml = p.required_part_str(SHEET).unwrap();
    assert!(!xml.contains("r:id=\"old\""));
    assert!(xml.contains("f:id=\"old\""));
    let parsed = jade_opc::parse_relationships(
        &p.required_part_str(&jade_opc::rels_part_name(SHEET))
            .unwrap(),
    )
    .unwrap();
    assert!(parsed.relationship("old").is_none());
    assert_eq!(
        book.cell(&name, "H2")
            .unwrap()
            .hyperlink
            .unwrap()
            .target
            .as_deref(),
        Some("https://new.example")
    );
}
#[test]
fn without_a_theme_links_take_the_office_blue() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    let name = book.workbook().sheets[0].name.clone();
    book.insert_hyperlink(&name, &link("H2", Some("https://example.com"), None), None)
        .unwrap();
    let format = book.format_at(&name, "H2").unwrap();
    assert_eq!(format.font_color.as_deref(), Some("0563C1"));
}
