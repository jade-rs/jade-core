//! Rows and columns come and go, and everything that pointed at them follows.

mod common;

use common::workbook_with_sheet;
use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{
    read_xlsx, shift_formula, Axis, CellRef, CellValue, StructureChange, Workbook,
};
use jade_xlsx_writer::restructure;
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

const SHEET: &str = "xl/worksheets/sheet1.xml";
const NAME: &str = "売上サマリ";

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn change(sheet: &str, axis: Axis, at: u32, count: u32, inserted: bool) -> StructureChange {
    StructureChange {
        sheet: sheet.to_string(),
        axis,
        at,
        count,
        inserted,
    }
}

fn apply(bytes: &[u8], change: &StructureChange) -> Vec<u8> {
    let workbook = read_xlsx(bytes).expect("read");
    restructure(bytes, &workbook, change).expect("restructure")
}

fn part(bytes: &[u8], name: &str) -> String {
    OpcPackage::open(bytes)
        .expect("package")
        .part_str(name)
        .expect("read")
        .expect("part")
}

fn cell(workbook: &HeadlessWorkbook, reference: &str) -> jade_xlsx_headless_api::CellView {
    workbook.cell(NAME, reference).expect("cell")
}

#[test]
fn inserting_a_row_moves_the_rows_below_and_the_formulas_that_point_at_them() {
    let original = fixture();
    // One row before row 5 (0-based 4): rows 5.. move down by one.
    let written = apply(&original, &change(NAME, Axis::Rows, 4, 1, true));
    let workbook = HeadlessWorkbook::open(&written).expect("open");

    // Row 4 stays, the new row 5 is empty, the old row 5 is now row 6.
    assert_eq!(cell(&workbook, "B4").display, "¥12,850,000");
    assert_eq!(cell(&workbook, "B5").display, "");
    assert_eq!(cell(&workbook, "B6").display, "¥9,420,000");
    // The row-6 formula moved to row 7 and reads its own row.
    assert_eq!(cell(&workbook, "D7").formula.as_deref(), Some("SUM(B7:C7)"));
    // The totals row moved to 8 and its range grew to cover the new row.
    assert_eq!(cell(&workbook, "B8").formula.as_deref(), Some("SUM(B4:B7)"));
    // The date on row 9 is on row 10 now, format and all.
    assert_eq!(cell(&workbook, "B10").display, "2023/03/15");
    // The merged title did not move.
    let sheet = &workbook.workbook().sheets[0];
    assert_eq!(sheet.merges[0].start, CellRef::new(0, 0));
    assert_eq!(sheet.merges[0].end, CellRef::new(3, 0));

    let xml = part(&written, SHEET);
    assert!(xml.contains(r#"<row r="8">"#), "{xml}");
    assert!(xml.contains(r#"<row r="11">"#), "{xml}");
    assert!(!xml.contains(r#"<row r="12">"#), "{xml}");
    // Only the sheet, the workbook (recalculation flag) changed.
    let before = OpcPackage::open(&original).expect("before");
    let after = OpcPackage::open(&written).expect("after");
    assert_eq!(before.part_names(), after.part_names());
    for name in before.part_names() {
        if name == SHEET || name == "xl/workbook.xml" {
            continue;
        }
        assert_eq!(before.part(name), after.part(name), "{name} changed");
    }
}

#[test]
fn deleting_rows_drops_them_and_shrinks_what_covered_them() {
    let original = fixture();
    // Row 5 only.
    let written = apply(&original, &change(NAME, Axis::Rows, 4, 1, false));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert_eq!(cell(&workbook, "B5").display, "¥6,130,000");
    assert_eq!(cell(&workbook, "B6").formula.as_deref(), Some("SUM(B4:B5)"));
    assert_eq!(cell(&workbook, "D5").formula.as_deref(), Some("SUM(B5:C5)"));
    // Recalculation is requested: the caches are stale.
    assert!(part(&written, "xl/workbook.xml").contains("fullCalcOnLoad"));

    // Rows 1-3: the merged title goes with them, and the merge list with it.
    let written = apply(&original, &change(NAME, Axis::Rows, 0, 3, false));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert!(workbook.workbook().sheets[0].merges.is_empty());
    let xml = part(&written, SHEET);
    assert!(!xml.contains("<mergeCells"), "{xml}");
    assert_eq!(cell(&workbook, "D1").formula.as_deref(), Some("SUM(B1:C1)"));
    assert_eq!(cell(&workbook, "B4").formula.as_deref(), Some("SUM(B1:B3)"));

    // Rows 4-7: the totals lose their whole range.
    let written = apply(&original, &change(NAME, Axis::Rows, 3, 4, false));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert_eq!(
        cell(&workbook, "B4").kind,
        jade_xlsx_headless_api::CellKind::Empty
    );
    assert_eq!(cell(&workbook, "B5").display, "2023/03/15");
}

#[test]
fn columns_move_cells_formulas_widths_and_merges() {
    let original = fixture();
    // A column before B.
    let written = apply(&original, &change(NAME, Axis::Columns, 1, 1, true));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert_eq!(cell(&workbook, "B4").display, "");
    assert_eq!(cell(&workbook, "C4").display, "¥12,850,000");
    assert_eq!(cell(&workbook, "E4").formula.as_deref(), Some("SUM(C4:D4)"));
    let sheet = &workbook.workbook().sheets[0];
    assert_eq!(sheet.merges[0].end, CellRef::new(4, 0));
    let xml = part(&written, SHEET);
    assert!(
        xml.contains(r#"<col min="3" max="5" width="16.7109375" customWidth="1"/>"#),
        "{xml}"
    );
    assert!(
        xml.contains(r#"<dimension ref="A1:E10"/>"#) || !xml.contains("<dimension"),
        "{xml}"
    );

    // Delete column B.
    let written = apply(&original, &change(NAME, Axis::Columns, 1, 1, false));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert_eq!(cell(&workbook, "B4").display, "¥15,230,000");
    assert_eq!(cell(&workbook, "C4").formula.as_deref(), Some("SUM(B4:B4)"));
    assert_eq!(cell(&workbook, "B7").formula.as_deref(), Some("SUM(B4:B6)"));
    let sheet = &workbook.workbook().sheets[0];
    assert_eq!(sheet.merges[0].end, CellRef::new(2, 0));
    let xml = part(&written, SHEET);
    assert!(
        xml.contains(r#"<col min="2" max="3" width="16.7109375" customWidth="1"/>"#),
        "{xml}"
    );
}

#[test]
fn a_deletion_writes_shared_formulas_out_in_full() {
    let bytes = fixture();
    let mut package = OpcPackage::open(&bytes).expect("package");
    let xml = package.part_str(SHEET).expect("read").expect("sheet");
    let rows = concat!(
        r#"<row r="20"><c r="A20"><v>10</v></c><c r="B20"><f t="shared" ref="B20:B21" si="7">A20*2</f><v>20</v></c></row>"#,
        r#"<row r="21"><c r="A21"><v>11</v></c><c r="B21"><f t="shared" si="7"/><v>22</v></c></row>"#
    );
    let xml = xml.replace("</sheetData>", &format!("{rows}</sheetData>"));
    package.replace(SHEET, xml.into_bytes());
    let original = package.write().expect("write");

    // Delete row 20, the master: row 21 keeps its own formula.
    let written = apply(&original, &change(NAME, Axis::Rows, 19, 1, false));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert_eq!(cell(&workbook, "B20").formula.as_deref(), Some("A20*2"));
    let xml = part(&written, SHEET);
    assert!(!xml.contains(r#"t="shared""#), "{xml}");

    // An insertion above leaves the group shared, moved.
    let written = apply(&original, &change(NAME, Axis::Rows, 0, 2, true));
    let workbook = HeadlessWorkbook::open(&written).expect("open");
    assert_eq!(cell(&workbook, "B22").formula.as_deref(), Some("A22*2"));
    assert_eq!(cell(&workbook, "B23").formula.as_deref(), Some("A23*2"));
    let xml = part(&written, SHEET);
    assert!(
        xml.contains(r#"<f t="shared" ref="B22:B23" si="7">A22*2</f>"#),
        "{xml}"
    );
}

#[test]
fn other_sheets_and_defined_names_follow_the_changed_sheet() {
    let bytes = fixture();
    let mut package = OpcPackage::open(&bytes).expect("package");
    let sheet2 = package
        .part_str("xl/worksheets/sheet2.xml")
        .expect("read")
        .expect("sheet2");
    let sheet2 = sheet2.replace(
        "</row></sheetData>",
        r#"<c r="B1"><f>売上サマリ!B7+B1</f><v>0</v></c></row></sheetData>"#,
    );
    package.replace("xl/worksheets/sheet2.xml", sheet2.into_bytes());
    let workbook = package
        .part_str("xl/workbook.xml")
        .expect("read")
        .expect("workbook");
    let workbook = workbook.replace(
        "</sheets>",
        r#"</sheets><definedNames><definedName name="Total">売上サマリ!$B$7</definedName><definedName name="Elsewhere">Sheet2!$A$1</definedName></definedNames>"#,
    );
    package.replace("xl/workbook.xml", workbook.into_bytes());
    let original = package.write().expect("write");

    let written = apply(&original, &change(NAME, Axis::Rows, 4, 1, true));
    let sheet2 = part(&written, "xl/worksheets/sheet2.xml");
    // The reference into the changed sheet moved; the sheet's own did not.
    assert!(sheet2.contains("<f>売上サマリ!B8+B1</f>"), "{sheet2}");
    let workbook = part(&written, "xl/workbook.xml");
    assert!(
        workbook.contains(r#"<definedName name="Total">売上サマリ!$B$8</definedName>"#),
        "{workbook}"
    );
    assert!(
        workbook.contains(r#"<definedName name="Elsewhere">Sheet2!$A$1</definedName>"#),
        "{workbook}"
    );
}

#[test]
fn the_calculation_chain_is_dropped_with_its_relationship_and_content_type() {
    let bytes = fixture();
    let mut package = OpcPackage::open(&bytes).expect("package");
    package.replace(
        "xl/calcChain.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="D4" i="1"/></calcChain>"#.to_vec(),
    );
    let rels = package
        .part_str("xl/_rels/workbook.xml.rels")
        .expect("read")
        .expect("rels");
    let rels = rels.replace(
        "</Relationships>",
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#,
    );
    package.replace("xl/_rels/workbook.xml.rels", rels.into_bytes());
    let types = package
        .part_str("[Content_Types].xml")
        .expect("read")
        .expect("types");
    let types = types.replace(
        "</Types>",
        r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#,
    );
    package.replace("[Content_Types].xml", types.into_bytes());
    let original = package.write().expect("write");
    assert!(read_xlsx(&original).is_ok());

    let written = apply(&original, &change(NAME, Axis::Rows, 4, 1, true));
    let after = OpcPackage::open(&written).expect("after");
    assert!(after.part("xl/calcChain.xml").is_none());
    assert!(!part(&written, "xl/_rels/workbook.xml.rels").contains("calcChain"));
    assert!(!part(&written, "[Content_Types].xml").contains("calcChain"));
    assert!(read_xlsx(&written).is_ok());
}

#[test]
fn a_bad_change_is_refused_before_anything_moves() {
    let original = fixture();
    let workbook = read_xlsx(&original).expect("read");
    assert!(restructure(
        &original,
        &workbook,
        &change("Nope", Axis::Rows, 0, 1, true)
    )
    .is_err());
    assert!(restructure(&original, &workbook, &change(NAME, Axis::Rows, 0, 0, true)).is_err());
    assert!(restructure(
        &original,
        &workbook,
        &change(NAME, Axis::Rows, 1_048_575, 2, false)
    )
    .is_err());
}

// ── property: every cell lands where the change maps it ─────────────

fn reference_text(reference: CellRef) -> String {
    let mut label = String::new();
    let mut value = reference.column + 1;
    while value > 0 {
        let remainder = (value - 1) % 26;
        label.insert(0, (b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    format!("{label}{}", reference.row + 1)
}

/// A small sheet: numbers everywhere, and formulas in the last column
/// pointing at cells across the grid.
fn sheet_xml(rows: u32, columns: u32, formulas: &[(u32, u32, u32, u32)]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for row in 0..rows {
        xml.push_str(&format!("<row r=\"{}\">", row + 1));
        for column in 0..columns {
            let at = CellRef::new(column, row);
            xml.push_str(&format!(
                "<c r=\"{}\"><v>{}</v></c>",
                reference_text(at),
                row * 100 + column + 1
            ));
        }
        if let Some((_, c1, r1, c2)) = formulas
            .iter()
            .find(|(formula_row, ..)| *formula_row == row)
        {
            let at = CellRef::new(columns, row);
            xml.push_str(&format!(
                "<c r=\"{}\"><f>SUM({}:{})+{}</f><v>0</v></c>",
                reference_text(at),
                reference_text(CellRef::new(*c1, *r1)),
                reference_text(CellRef::new(*c2, (*r1).max(1))),
                reference_text(CellRef::new(*c1, *c2))
            ));
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    xml
}

fn cells_of(workbook: &Workbook) -> BTreeMap<CellRef, (CellValue, Option<String>)> {
    let mut out = BTreeMap::new();
    for row in &workbook.sheets[0].rows {
        for cell in &row.cells {
            out.insert(cell.reference, (cell.value.clone(), cell.formula.clone()));
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn every_cell_lands_where_the_change_maps_it(
        rows in 1u32..6,
        columns in 1u32..4,
        formulas in prop::collection::vec((0u32..6, 0u32..4, 0u32..6, 0u32..4), 0..3),
        axis in prop_oneof![Just(Axis::Rows), Just(Axis::Columns)],
        at in 0u32..7,
        count in 1u32..3,
        inserted in any::<bool>(),
    ) {
        let original = workbook_with_sheet(&sheet_xml(rows, columns, &formulas));
        let before = read_xlsx(&original).expect("read");
        let change = StructureChange { sheet: "Sheet1".into(), axis, at, count, inserted };
        let written = restructure(&original, &before, &change).expect("restructure");
        let after = read_xlsx(&written).expect("read back");
        let after_cells = cells_of(&after);

        let mut expected = BTreeMap::new();
        for (reference, (value, formula)) in cells_of(&before) {
            if let Some(mapped) = change.map_cell(reference) {
                let formula = formula.map(|formula| shift_formula(&formula, "Sheet1", &change));
                expected.insert(mapped, (value, formula));
            }
        }
        prop_assert_eq!(after_cells, expected);

        // The inverse change puts everything back.
        if inserted {
            let inverse = StructureChange { inserted: false, ..change.clone() };
            let restored = restructure(&written, &after, &inverse).expect("inverse");
            prop_assert_eq!(cells_of(&read_xlsx(&restored).expect("read")), cells_of(&before));
        }
    }
}
#[test]
fn print_area_and_print_titles_follow_the_grid() {
    // A workbook has no sections; the print area and the repeating title
    // rows are the closest thing, and Excel keeps them as defined names —
    // `_xlnm.Print_Titles` in the whole-row form `$1:$2`, which no cell
    // reference ever takes. They have to move like any other reference.
    let bytes = fixture();
    let mut package = OpcPackage::open(&bytes).expect("package");
    let workbook = package
        .part_str("xl/workbook.xml")
        .expect("read")
        .expect("workbook");
    let workbook = workbook.replace(
        "</sheets>",
        concat!(
            "</sheets><definedNames>",
            r#"<definedName name="_xlnm.Print_Area" localSheetId="0">売上サマリ!$A$1:$C$8</definedName>"#,
            r#"<definedName name="_xlnm.Print_Titles" localSheetId="0">売上サマリ!$1:$2</definedName>"#,
            r#"<definedName name="Column" localSheetId="0">売上サマリ!$A:$A</definedName>"#,
            "</definedNames>",
        ),
    );
    package.replace("xl/workbook.xml", workbook.into_bytes());
    let original = package.write().expect("write");

    // A row inside the print area grows it, and leaves the title rows above
    // it alone.
    let inside = part(
        &apply(&original, &change(NAME, Axis::Rows, 4, 1, true)),
        "xl/workbook.xml",
    );
    assert!(inside.contains("売上サマリ!$A$1:$C$9"), "{inside}");
    assert!(inside.contains("売上サマリ!$1:$2"), "{inside}");

    // A row above everything pushes all three down, the whole-row form
    // included.
    let above = part(
        &apply(&original, &change(NAME, Axis::Rows, 0, 1, true)),
        "xl/workbook.xml",
    );
    assert!(above.contains("売上サマリ!$A$2:$C$9"), "{above}");
    assert!(above.contains("売上サマリ!$2:$3"), "{above}");
    assert!(above.contains("売上サマリ!$A:$A"), "{above}");

    // A column at A moves the area and the whole-column form, and leaves the
    // title rows — which name no column — as they are.
    let column = part(
        &apply(&original, &change(NAME, Axis::Columns, 0, 1, true)),
        "xl/workbook.xml",
    );
    assert!(column.contains("売上サマリ!$B$1:$D$8"), "{column}");
    assert!(column.contains("売上サマリ!$B:$B"), "{column}");
    assert!(column.contains("売上サマリ!$1:$2"), "{column}");
}
