//! Dynamic arrays edit as in Excel: a typed `=SEQUENCE(3)` fills the cells
//! below it and is saved as a dynamic array; the spill grows and shrinks
//! with its inputs; typing into a spilled cell blocks the spill (`#SPILL!`)
//! until the cell is cleared again; and undo takes it all back.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{CellRange, CellRef};

const SHEET: &str = "Operations";

#[test]
fn removing_a_merge_unblocks_a_dynamic_spill_in_one_history_step() {
    let mut book = book();
    book.merge_cells(SHEET, "J2:K2").unwrap();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["#SPILL!", "", ""]);
    book.unmerge_cells(SHEET, "J2:K2").unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["1", "2", "3"]);
    let saved = HeadlessWorkbook::open_for_editing(book.bytes()).unwrap();
    assert_eq!(column(&saved, &["J1", "J2", "J3"]), ["1", "2", "3"]);
    book.undo().unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["#SPILL!", "", ""]);
    assert!(book.workbook().sheets[0].merges.contains(&range("J2:K2")));
    book.redo().unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["1", "2", "3"]);
}

#[test]
fn entered_scalar_aggregate_keeps_array_evaluation_beside_imported_normal_formulas() {
    let mut package = jade_opc::OpcPackage::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    package.replace("xl/worksheets/sheet1.xml", br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>a</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>bb</t></is></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>ccc</t></is></c></row><row r="5"><c r="B5" t="e"><f>SUM(LEN(A1:A3))</f><v>#VALUE!</v></c><c r="C5"><f t="array" ref="C5">SUM(LEN(A1:A3))</f><v>6</v></c></row></sheetData></worksheet>"#.to_vec());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    book.set(SHEET, "D5", CellInput::Formula("SUM(LEN(A1:A3))".into()))
        .unwrap();
    assert_eq!(column(&book, &["B5", "C5", "D5"]), ["#VALUE!", "6", "6"]);
    assert!(book.workbook().sheets[0]
        .array_formulas
        .iter()
        .any(|array| {
            array.anchor == CellRef::parse("D5").unwrap()
                && array.kind == jade_xlsx_reader::ArrayFormulaKind::Dynamic
                && array.range == range("D5")
        }));
    book.set(SHEET, "F5", CellInput::Formula("FORMULATEXT(D5)".into()))
        .unwrap();
    assert_eq!(shown(&book, "F5"), "=SUM(LEN(A1:A3))");
    let outputs = [
        (0, CellRef::parse("B5").unwrap()),
        (0, CellRef::parse("D5").unwrap()),
    ];
    let original = vec![
        jade_xlsx_reader::CellValue::Error("#VALUE!".into()),
        jade_xlsx_reader::CellValue::Number(6.0),
    ];
    let mut probe = book.what_if().unwrap();
    for at in ["B5", "D5"] {
        probe
            .probe(
                book.workbook(),
                &[(
                    0,
                    CellRef::parse(at).unwrap(),
                    jade_xlsx_reader::CellValue::Number(99.0),
                )],
                &outputs,
            )
            .unwrap();
        assert_eq!(
            probe.probe(book.workbook(), &[], &outputs).unwrap(),
            original
        );
    }
    book.set(SHEET, "A2", CellInput::Text("four".into()))
        .unwrap();
    assert_eq!(column(&book, &["B5", "C5", "D5"]), ["#VALUE!", "8", "8"]);
    book.undo().unwrap();
    assert_eq!(shown(&book, "D5"), "6");
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    let mut reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    reopened
        .set(SHEET, "A3", CellInput::Text("x".into()))
        .unwrap();
    assert_eq!(
        column(&reopened, &["B5", "C5", "D5"]),
        ["#VALUE!", "6", "6"]
    );
}

fn book() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap()
}

fn shown(book: &HeadlessWorkbook, reference: &str) -> String {
    book.cell(SHEET, reference).unwrap().display
}

fn column(book: &HeadlessWorkbook, cells: &[&str]) -> Vec<String> {
    cells.iter().map(|cell| shown(book, cell)).collect()
}

fn array_ranges(book: &HeadlessWorkbook) -> Vec<(CellRef, CellRange)> {
    let workbook = jade_xlsx_reader::read_xlsx(book.bytes()).unwrap();
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name == SHEET)
        .unwrap();
    sheet
        .array_formulas
        .iter()
        .map(|array| (array.anchor, array.range))
        .collect()
}

fn range(reference: &str) -> CellRange {
    CellRange::parse(reference).unwrap()
}

#[test]
fn a_typed_spill_fills_its_cells_and_is_saved_as_one() {
    let mut book = book();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    assert_eq!(
        column(&book, &["J1", "J2", "J3", "J4"]),
        ["1", "2", "3", ""]
    );
    assert_eq!(
        array_ranges(&book),
        [(CellRef::parse("J1").unwrap(), range("J1:J3"))]
    );
    // Reopened, the spill is still there and still the formula's.
    let reopened = HeadlessWorkbook::open_for_editing(book.bytes()).unwrap();
    assert_eq!(column(&reopened, &["J1", "J2", "J3"]), ["1", "2", "3"]);
    assert_eq!(reopened.bytes(), book.bytes(), "reopening changed the file");

    // One undo takes the formula and its spill back.
    assert!(book.undo().unwrap());
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["", "", ""]);
}

#[test]
fn a_spill_grows_and_shrinks_with_its_input() {
    let mut book = book();
    book.set(SHEET, "K1", CellInput::Number(5.0)).unwrap();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(K1)".into()))
        .unwrap();
    assert_eq!(
        column(&book, &["J1", "J2", "J3", "J4", "J5", "J6"]),
        ["1", "2", "3", "4", "5", ""]
    );
    book.set(SHEET, "K1", CellInput::Number(2.0)).unwrap();
    assert_eq!(
        column(&book, &["J1", "J2", "J3", "J4", "J5"]),
        ["1", "2", "", "", ""]
    );
    assert_eq!(
        array_ranges(&book),
        [(CellRef::parse("J1").unwrap(), range("J1:J2"))]
    );
}

#[test]
fn typing_into_a_spill_blocks_it_until_the_cell_is_cleared() {
    let mut book = book();
    book.set(SHEET, "J1", CellInput::Formula("SEQUENCE(3)".into()))
        .unwrap();
    book.set(SHEET, "J2", CellInput::Text("x".into())).unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["#SPILL!", "x", ""]);
    book.set(SHEET, "J2", CellInput::Blank).unwrap();
    assert_eq!(column(&book, &["J1", "J2", "J3"]), ["1", "2", "3"]);
}

#[test]
fn unrepresentable_dynamic_metadata_rejects_typing_without_changes() {
    let mut package = jade_opc::OpcPackage::open(include_bytes!(
        "../../../public/fixtures/grid-operations-workbook.xlsx"
    ))
    .unwrap();
    let metadata = r#"<metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><metadataTypes count="0"/><metadataTypes count="0"/></metadata>"#;
    package.replace("xl/metadata.xml", metadata.as_bytes().to_vec());
    let rels = package.part_str("xl/_rels/workbook.xml.rels").unwrap().unwrap().replace("</Relationships>", r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata" Target="metadata.xml"/></Relationships>"#);
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    let before_bytes = book.bytes().to_vec();
    let before_model = book.workbook().clone();
    let result = book.set(SHEET, "J5", CellInput::Formula("SEQUENCE(3)".into()));
    assert!(
        matches!(
            &result,
            Err(jade_xlsx_headless_editor::HeadlessError::Write(
                jade_xlsx_writer::WriteError::ArrayMetadata(_)
            ))
        ),
        "{result:?}"
    );
    assert!(result.unwrap_err().is_invalid_input());
    assert_eq!(book.bytes(), before_bytes);
    assert_eq!(book.workbook(), &before_model);
    assert!(!book.undo().unwrap());
    book.set(SHEET, "N5", CellInput::Formula("1+1".into()))
        .unwrap();
    assert_eq!(shown(&book, "N5"), "2");
    assert_eq!(
        jade_opc::OpcPackage::open(book.bytes())
            .unwrap()
            .part_str("xl/metadata.xml")
            .unwrap()
            .unwrap(),
        metadata
    );
}
