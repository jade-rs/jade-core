use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    protected_ranges::{protected_ranges, EXT_URI},
    Axis, CellRange, CellShift, StructureChange,
};
use jade_xlsx_writer::protected_ranges::*;
const PART: &str = "xl/worksheets/sheet1.xml";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn fixture(body: &str) -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    p.replace(PART,format!(r#"<s:worksheet xmlns:s="{MAIN}" xmlns:p="{MAIN}" xmlns:x="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:m="http://schemas.microsoft.com/office/excel/2006/main" xmlns:f="urn:foreign"><s:sheetData/>{body}</s:worksheet>"#).into_bytes());
    p.write().unwrap()
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str(PART)
        .unwrap()
}
fn areas(s: &str) -> Vec<CellRange> {
    jade_xlsx_reader::protected_ranges::parse_sqref(s).unwrap()
}
fn pair(reference: &str) -> String {
    format!(
        r#"<p:protectedRanges f:mode="keep"><p:protectedRange name="Inputs" sqref="{reference}" password="83AF" f:tag="keep"><f:payload><m:sqref>A99</m:sqref></f:payload></p:protectedRange><f:protectedRange name="Foreign" sqref="A99"/></p:protectedRanges><s:extLst><s:ext uri="{EXT_URI}"><x:protectedRanges><x:protectedRange name="inputs" algorithmName="SHA-256" spinCount="3" saltValue="AAECAwQFBgcICQoLDA0ODw==" hashValue="jA9zZOAXfLvFkJjipbgp+ZuI0KvdLnOkIcryShL5XGU="><m:sqref>{reference}</m:sqref><f:payload><m:sqref>A99</m:sqref></f:payload></x:protectedRange></x:protectedRanges></s:ext><s:ext uri="urn:foreign"><x:protectedRanges><x:protectedRange name="Opaque"><m:sqref>A99</m:sqref></x:protectedRange></x:protectedRanges></s:ext></s:extLst>"#
    )
}
#[test]
fn create_passwordless_and_strong_ranges_preserves_other_parts_and_schema_order() {
    for password in ["", "秘密🔒"] {
        let original = fixture("<s:scenarios/><s:mergeCells/>");
        let bytes = set_editable_range(
            &original,
            PART,
            None,
            "入力",
            &areas("A1:B2 D3"),
            Some((password, &[3; 16])),
        )
        .unwrap();
        let out = xml(&bytes);
        assert!(out.find("protectedRanges").unwrap() < out.find("<s:scenarios").unwrap());
        assert!(!out.contains("秘密"));
        let ranges = protected_ranges(&out).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].requires_password(), !password.is_empty());
        for verifier in ranges[0].password_verifiers() {
            assert!(verifier.verifier().verifies_range(password).unwrap());
        }
        let a = OpcPackage::open(&original).unwrap();
        let b = OpcPackage::open(&bytes).unwrap();
        for name in a.part_names() {
            if name != PART {
                assert_eq!(a.part(name), b.part(name), "{name}");
            }
        }
        assert_eq!(
            set_editable_range(&bytes, PART, Some("入力"), "入力", &areas("A1:B2 D3"), None)
                .unwrap(),
            bytes
        );
    }
}
#[test]
fn strict_alias_mirrors_rename_resize_keep_password_and_preserve_opaque_xml() {
    for ns in [MAIN, "http://purl.oclc.org/ooxml/spreadsheetml/main"] {
        let source = fixture(&pair("A1:A2"));
        let mut p = OpcPackage::open(&source).unwrap();
        p.replace(PART, xml(&source).replace(MAIN, ns).into_bytes());
        let source = p.write().unwrap();
        let bytes = set_editable_range(
            &source,
            PART,
            Some("Inputs"),
            "Renamed",
            &areas("B2:B4 E5"),
            None,
        )
        .unwrap();
        let out = xml(&bytes);
        let ranges = protected_ranges(&out).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].name, "Renamed");
        assert_eq!(ranges[0].ranges, areas("B2:B4 E5"));
        assert!(!ranges[0].ambiguous);
        for password in ranges[0].password_verifiers() {
            assert!(password.verifier().verifies_range("password").unwrap());
        }
        for keep in [
            "f:mode=\"keep\"",
            "f:tag=\"keep\"",
            "<m:sqref>A99</m:sqref>",
            "uri=\"urn:foreign\"",
            "name=\"Foreign\"",
        ] {
            assert!(out.contains(keep), "{out}");
        }
        let reset = set_editable_range(
            &bytes,
            PART,
            Some("Renamed"),
            "Renamed",
            &areas("B2:B4"),
            Some(("", &[0; 16])),
        )
        .unwrap();
        assert!(!protected_ranges(&xml(&reset)).unwrap()[0].requires_password());
        assert!(xml(&reset).contains("f:tag=\"keep\""));
    }
}
#[test]
fn deleting_plain_definitions_removes_empty_owned_containers_and_wrappers() {
    let source = fixture(&format!(
        r#"<s:protectedRanges><s:protectedRange name="A" sqref="A1"/></s:protectedRanges><s:extLst><s:ext uri="{EXT_URI}"><x:protectedRanges><x:protectedRange name="a"><m:sqref>A1</m:sqref></x:protectedRange></x:protectedRanges></s:ext></s:extLst>"#
    ));
    let bytes = delete_editable_range(&source, PART, "A").unwrap();
    let out = xml(&bytes);
    assert!(!out.contains("protectedRange"), "{out}");
    assert!(!out.contains(EXT_URI));
    assert!(!out.contains("extLst"));
    let source = fixture(&pair("A1"));
    let deleted = delete_editable_range(&source, PART, "Inputs").unwrap();
    let out = xml(&deleted);
    assert!(protected_ranges(&out).unwrap().is_empty());
    assert!(
        out.contains("f:mode=\"keep\"")
            && out.contains("name=\"Foreign\"")
            && out.contains("uri=\"urn:foreign\"")
    );
}
#[test]
fn invalid_names_coverage_duplicates_and_protected_sheet_edits_are_atomic_errors() {
    let source = fixture(
        r#"<s:protectedRanges><s:protectedRange name="Inputs" sqref="A1"/></s:protectedRanges>"#,
    );
    for name in ["", "1Input", "bad/name", "bad\nname"] {
        assert!(
            set_editable_range(&source, PART, None, name, &areas("A1"), None).is_err(),
            "{name}"
        );
    }
    assert!(set_editable_range(&source, PART, None, "inputs", &areas("B2"), None).is_err());
    assert!(set_editable_range(&source, PART, Some("missing"), "New", &areas("B2"), None).is_err());
    assert!(set_editable_range(&source, PART, None, "New", &[], None).is_err());
    let protected = jade_xlsx_writer::sheet_protection::protect_sheet(
        &source,
        PART,
        Default::default(),
        "",
        &[0; 16],
    )
    .unwrap();
    assert!(set_editable_range(&protected, PART, None, "New", &areas("B2"), None).is_err());
    assert!(delete_editable_range(&protected, PART, "Inputs").is_err());
}
#[test]
fn whole_axis_shifts_reconcile_both_families_without_rewriting_opaque_payloads() {
    for axis in [Axis::Rows, Axis::Columns] {
        let source = fixture(&pair("B2:D4"));
        let book = jade_xlsx_reader::read_xlsx(&source).unwrap();
        let change = StructureChange {
            sheet: book.sheets[0].name.clone(),
            axis,
            at: 1,
            count: 1,
            inserted: true,
        };
        let moved = jade_xlsx_writer::restructure(&source, &book, &change).unwrap();
        let out = xml(&moved);
        let ranges = protected_ranges(&out).unwrap();
        assert!(!ranges[0].ambiguous);
        assert_eq!(
            ranges[0].ranges,
            areas(if axis == Axis::Rows { "B3:D5" } else { "C2:E4" })
        );
        assert_eq!(out.matches("<m:sqref>A99</m:sqref>").count(), 3);
        assert!(out.contains("password=\"83AF\""));
        assert!(ranges[0]
            .password_verifiers()
            .all(|p| p.verifier().verifies_range("password").unwrap()));
    }
}
#[test]
fn partial_shifts_split_protected_coverage_once_in_both_families() {
    let source = fixture(&pair("B2:D4"));
    let book = jade_xlsx_reader::read_xlsx(&source).unwrap();
    let change = CellShift {
        sheet: book.sheets[0].name.clone(),
        axis: Axis::Rows,
        range: CellRange::parse("C3:C3").unwrap(),
        inserted: true,
    };
    let moved = jade_xlsx_writer::cell_shift::shift_cells(&source, &book, &change).unwrap();
    let out = xml(&moved);
    let ranges = protected_ranges(&out).unwrap();
    assert!(!ranges[0].ambiguous);
    assert!(jade_xlsx_reader::protected_ranges::same_coverage(
        &ranges[0].ranges,
        &areas("B2:B4 C2:C5 D2:D4")
    ));
    assert_eq!(out.matches("<m:sqref>A99</m:sqref>").count(), 3);
}
#[test]
fn deletion_and_grid_edge_clipping_do_not_leave_dangling_range_definitions() {
    let source = fixture(
        r#"<s:protectedRanges><s:protectedRange name="Inputs" sqref="A1:A2"/></s:protectedRanges>"#,
    );
    let book = jade_xlsx_reader::read_xlsx(&source).unwrap();
    let change = StructureChange {
        sheet: book.sheets[0].name.clone(),
        axis: Axis::Rows,
        at: 0,
        count: 2,
        inserted: false,
    };
    let moved = jade_xlsx_writer::restructure(&source, &book, &change).unwrap();
    assert!(!xml(&moved).contains("protectedRange"));
    let source = fixture(
        r#"<s:protectedRanges><s:protectedRange name="Inputs" sqref="A1:A1048576"/></s:protectedRanges>"#,
    );
    let book = jade_xlsx_reader::read_xlsx(&source).unwrap();
    let change = StructureChange {
        sheet: book.sheets[0].name.clone(),
        axis: Axis::Rows,
        at: 1,
        count: 1,
        inserted: true,
    };
    let moved = jade_xlsx_writer::restructure(&source, &book, &change).unwrap();
    assert_eq!(
        protected_ranges(&xml(&moved)).unwrap()[0].ranges,
        areas("A1:A1048576")
    );
}
#[test]
fn relocated_worksheet_parts_are_used_directly_and_mce_mutation_is_refused() {
    let original = fixture("");
    let mut p = OpcPackage::open(&original).unwrap();
    let moved = "xl/worksheets/relocated.xml";
    let part = p.required_part_str(PART).unwrap();
    p.remove(PART);
    p.replace(moved, part.into_bytes());
    let rels = p
        .required_part_str("xl/_rels/workbook.xml.rels")
        .unwrap()
        .replace("worksheets/sheet1.xml", "worksheets/relocated.xml");
    p.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let bytes = set_editable_range(
        &p.write().unwrap(),
        moved,
        None,
        "Inputs",
        &areas("A1"),
        None,
    )
    .unwrap();
    let read = jade_xlsx_reader::read_xlsx(&bytes).unwrap();
    assert_eq!(read.sheets[0].part_name, moved);
    let source = fixture(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Fallback><s:protectedRanges><s:protectedRange name="Inputs" sqref="A1"/></s:protectedRanges></mc:Fallback></mc:AlternateContent>"#,
    );
    assert!(set_editable_range(&source, PART, None, "New", &areas("B2"), None).is_err());
    let book = jade_xlsx_reader::read_xlsx(&source).unwrap();
    let change = StructureChange {
        sheet: book.sheets[0].name.clone(),
        axis: Axis::Rows,
        at: 1,
        count: 1,
        inserted: true,
    };
    assert!(jade_xlsx_writer::restructure(&source, &book, &change).is_err());
}
