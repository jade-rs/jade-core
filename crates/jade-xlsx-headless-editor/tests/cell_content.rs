use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellRef;
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const STRICT: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const FIRST: &str = "xl/worksheets/sheet1.xml";
const NAME: &str = "売上サマリ";
fn package(body: &str, main: &str) -> OpcPackage {
    let mut p = OpcPackage::open(include_bytes!(
        "../../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap();
    p.replace(FIRST, format!(r#"<s:worksheet xmlns:s="{main}" xmlns:mc="{MC}" xmlns:u="urn:foreign">{body}</s:worksheet>"#).into_bytes());
    p.replace(
        "xl/worksheets/sheet2.xml",
        format!(r#"<worksheet xmlns="{MAIN}"><sheetData/></worksheet>"#).into_bytes(),
    );
    p
}
fn xml(b: &HeadlessWorkbook) -> String {
    OpcPackage::open(b.bytes())
        .unwrap()
        .required_part_str(FIRST)
        .unwrap()
}

#[test]
fn selected_cells_calculate_edit_save_and_undo_without_touching_foreign_or_inactive_records() {
    let foreign = r#"<u:wrapper><s:c r="B1"><s:f>100+200</s:f><s:v>300</s:v></s:c></u:wrapper>"#;
    let inactive = r#"<mc:Choice Requires="u"><s:row r="1"><s:c r="A1"><s:v>900</s:v></s:c></s:row></mc:Choice>"#;
    for main in [MAIN, STRICT] {
        let body = format!(
            r#"<s:sheetData><mc:AlternateContent>{inactive}<mc:Fallback><s:row r="1">{foreign}<q:c xmlns:q="{main}" r="A1"><q:v>2</q:v></q:c><s:c r="B1"><mc:AlternateContent><mc:Choice Requires="s"><s:f>A1*2</s:f><s:v>99</s:v></mc:Choice><mc:Fallback><s:f>A1*3</s:f><s:v>6</s:v></mc:Fallback></mc:AlternateContent></s:c></s:row></mc:Fallback></mc:AlternateContent></s:sheetData>"#
        );
        let mut b =
            HeadlessWorkbook::open_for_editing(&package(&body, main).write().unwrap()).unwrap();
        assert_eq!(b.cell(NAME, "B1").unwrap().number, Some(4.0));
        assert!(xml(&b).contains(foreign));
        let before = b.bytes().to_vec();
        b.set(NAME, "A1", CellInput::Number(8.0)).unwrap();
        assert_eq!(b.cell(NAME, "A1").unwrap().number, Some(8.0));
        assert_eq!(b.cell(NAME, "B1").unwrap().number, Some(16.0));
        let after = b.bytes().to_vec();
        assert!(xml(&b).contains(foreign));
        assert!(xml(&b).contains(inactive));
        let saved = HeadlessWorkbook::open_for_editing(&b.save_bytes().unwrap()).unwrap();
        assert_eq!(saved.cell(NAME, "B1").unwrap().number, Some(16.0));
        assert!(xml(&saved).contains(foreign));
        b.undo().unwrap();
        assert_eq!(b.bytes(), before);
        assert_eq!(b.cell(NAME, "B1").unwrap().number, Some(4.0));
        b.redo().unwrap();
        assert_eq!(b.bytes(), after);
        assert_eq!(b.cell(NAME, "B1").unwrap().number, Some(16.0));
    }
}

#[test]
fn chosen_shared_strings_keep_rich_and_phonetic_data_through_calculation_and_neighbor_edits() {
    let shared = format!(
        r#"<s:sst xmlns:s="{MAIN}" xmlns:mc="{MC}" xmlns:u="urn:foreign"><s:si/><u:si><s:t>foreign</s:t></u:si><s:si><mc:AlternateContent><mc:Choice Requires="u"><s:t>間違い</s:t></mc:Choice><mc:Fallback><s:r><s:rPr><s:b/></s:rPr><s:t>東京</s:t></s:r><s:rPh sb="0" eb="2"><s:t>トウキョウ</s:t></s:rPh></mc:Fallback></mc:AlternateContent></s:si></s:sst>"#
    );
    let mut p = package(
        r#"<s:sheetData><s:row><s:c t="s"><s:v><![CDATA[1]]></s:v></s:c><s:c r="B1"><s:f>LEN(A1)</s:f><s:v>99</s:v></s:c></s:row></s:sheetData>"#,
        MAIN,
    );
    p.replace("xl/sharedStrings.xml", shared.as_bytes().to_vec());
    let mut b = HeadlessWorkbook::open_for_editing(&p.write().unwrap()).unwrap();
    assert_eq!(b.cell(NAME, "A1").unwrap().display, "東京");
    assert_eq!(b.cell(NAME, "B1").unwrap().number, Some(2.0));
    let at = CellRef::new(0, 0);
    let runs = b.workbook().sheets[0].rich_text[&at].clone();
    let reading = b.workbook().sheets[0].phonetic_readings[&at].clone();
    assert_eq!(runs[0].text, "東京");
    assert_eq!(reading, "トウキョウ");
    let before = b.bytes().to_vec();
    b.set(NAME, "C1", CellInput::Formula("LEN(A1)+1".into()))
        .unwrap();
    assert_eq!(b.cell(NAME, "C1").unwrap().number, Some(3.0));
    let after = b.bytes().to_vec();
    let saved = HeadlessWorkbook::open_for_editing(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(saved.workbook().sheets[0].rich_text[&at], runs);
    assert_eq!(saved.workbook().sheets[0].phonetic_readings[&at], reading);
    assert_eq!(
        OpcPackage::open(saved.bytes())
            .unwrap()
            .required_part_str("xl/sharedStrings.xml")
            .unwrap(),
        shared
    );
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    assert_eq!(b.bytes(), after);
}

#[test]
fn editing_known_alternate_grids_keeps_both_consumers_values_and_formula_caches_current() {
    let body = r#"<s:sheetData><mc:AlternateContent><mc:Choice Requires="s"><s:row r="1"><s:c r="A1"><s:v>2</s:v></s:c><s:c r="B1"><s:f>A1*2</s:f><s:v>99</s:v></s:c></s:row></mc:Choice><mc:Fallback><s:row r="1"><s:c r="A1"><s:v>2</s:v></s:c><s:c r="B1"><s:f>A1+A1</s:f><s:v>99</s:v></s:c></s:row></mc:Fallback></mc:AlternateContent></s:sheetData>"#;
    for main in [MAIN, STRICT] {
        let mut b =
            HeadlessWorkbook::open_for_editing(&package(body, main).write().unwrap()).unwrap();
        assert_eq!(b.cell(NAME, "B1").unwrap().number, Some(4.0));
        let before = b.bytes().to_vec();
        b.set(NAME, "A1", CellInput::Number(8.0)).unwrap();
        let after = b.bytes().to_vec();
        let bytes = b.save_bytes().unwrap();
        let mut alternate = OpcPackage::open(&bytes).unwrap();
        let raw = alternate.required_part_str(FIRST).unwrap();
        assert!(raw.contains("<s:f>A1*2</s:f>"));
        assert!(raw.contains("<s:f>A1+A1</s:f>"));
        alternate.replace(
            FIRST,
            raw.replace("Requires=\"s\"", "Requires=\"u\"").into_bytes(),
        );
        let alternate_bytes = alternate.write().unwrap();
        let cached = HeadlessWorkbook::open(&alternate_bytes).unwrap();
        assert_eq!(cached.cell(NAME, "A1").unwrap().number, Some(8.0));
        assert_eq!(cached.cell(NAME, "B1").unwrap().number, Some(16.0));
        let calculated = HeadlessWorkbook::open_for_editing(&alternate_bytes).unwrap();
        assert_eq!(calculated.cell(NAME, "B1").unwrap().number, Some(16.0));
        b.undo().unwrap();
        assert_eq!(b.bytes(), before);
        b.redo().unwrap();
        assert_eq!(b.bytes(), after);
    }
}
