use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellRef;
const PART: &str = "xl/worksheets/sheet1.xml";
fn fixture(extra: &str) -> Vec<u8> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    let mut p = OpcPackage::open(&bytes).unwrap();
    let xml = p.required_part_str(PART).unwrap().replace(
        "</worksheet>",
        &format!(
            r#"<sheetProtection sheet="1"/><protectedRanges>{extra}</protectedRanges></worksheet>"#
        ),
    );
    p.replace(PART, xml.into_bytes());
    p.write().unwrap()
}
#[test]
fn candidates_are_only_valid_password_alternatives_covering_a_refused_locked_cell() {
    let bytes = fixture(
        r#"<protectedRange name="Inputs" sqref="A1:A2" password="83AF"/><protectedRange name="Other" sqref="C1" password="83AF"/><protectedRange name="Overlap" sqref="A1" password="83AF"/><protectedRange name="Broken" sqref="A1" password="ZZZZ"/><protectedRange name="Modern" sqref="A1" password="83AF" algorithmName="MD5"/>"#,
    );
    let mut b = HeadlessWorkbook::open(&bytes).unwrap();
    let name = b.workbook().sheets[0].name.clone();
    let before = b.bytes().to_vec();
    let candidates = b
        .range_unlock_candidates(&name, CellRef::new(0, 0))
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["Inputs", "Overlap"]
    );
    assert!(b
        .range_unlock_candidates(&name, CellRef::new(1, 0))
        .unwrap()
        .is_empty());
    assert_eq!(b.bytes(), before);
    assert!(!b.can_undo());
    b.unlock_editable_range(&name, "Inputs", "password")
        .unwrap();
    assert!(b
        .range_unlock_candidates(&name, CellRef::new(0, 0))
        .unwrap()
        .is_empty());
    assert_eq!(b.bytes(), before);
    assert!(!b.can_undo());
    b.lock_editable_ranges(&name).unwrap();
    assert_eq!(
        b.range_unlock_candidates(&name, CellRef::new(0, 0))
            .unwrap()
            .len(),
        2
    );
    b.unprotect_sheet(&name, "").unwrap();
    assert!(b
        .range_unlock_candidates(&name, CellRef::new(0, 0))
        .unwrap()
        .is_empty());
}
#[test]
fn passwordless_overlaps_duplicate_records_styles_and_bad_coordinates_do_not_prompt() {
    for records in [
        r#"<protectedRange name="Input" sqref="A1" password="83AF"/><protectedRange name="Everyone" sqref="A1" securityDescriptor="opaque"/>"#,
        r#"<protectedRange name="Input" sqref="A1" password="83AF"/><protectedRange name="Input" sqref="A1" password="83AF"/>"#,
    ] {
        let b = HeadlessWorkbook::open(&fixture(records)).unwrap();
        let name = &b.workbook().sheets[0].name;
        assert!(b
            .range_unlock_candidates(name, CellRef::new(0, 0))
            .unwrap()
            .is_empty());
        assert!(b
            .range_unlock_candidates(name, CellRef::new(16384, 0))
            .is_err());
    }
    let mut b = HeadlessWorkbook::open(&fixture(
        r#"<protectedRange name="Input" sqref="A1" password="83AF"/>"#,
    ))
    .unwrap();
    let name = b.workbook().sheets[0].name.clone();
    b.unprotect_sheet(&name, "").unwrap();
    b.set_format(
        &name,
        "A1",
        &jade_xlsx_headless_editor::FormatChange {
            locked: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    b.protect_sheet(&name, Default::default(), "", &[0; 16])
        .unwrap();
    assert!(b
        .range_unlock_candidates(&name, CellRef::new(0, 0))
        .unwrap()
        .is_empty());
}
