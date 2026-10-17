//! Round trips: write a cell, read it back, and check nothing else moved.

use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::CellRef;
use jade_xlsx_writer::{write_xlsx, CellEdit, NewValue, SheetEdits};
use std::path::PathBuf;

const SHEET: &str = "xl/worksheets/sheet1.xml";

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn edit(reference: &str, value: NewValue) -> CellEdit {
    CellEdit {
        reference: CellRef::parse(reference).expect("reference"),
        value,
        style_index: None,
    }
}

fn styled(reference: &str, value: NewValue, style_index: u32) -> CellEdit {
    CellEdit {
        style_index: Some(style_index),
        ..edit(reference, value)
    }
}

fn write(original: &[u8], cells: Vec<CellEdit>) -> Vec<u8> {
    write_xlsx(
        original,
        &[SheetEdits {
            part_name: SHEET.to_string(),
            cells,
        }],
    )
    .expect("write")
}

fn read(bytes: &[u8]) -> HeadlessWorkbook {
    HeadlessWorkbook::open(bytes).expect("read back")
}

#[test]
fn a_number_written_into_a_formatted_cell_keeps_its_format() {
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("B4", NewValue::Number(999.0))]);

    let cell = read(&written).cell("売上サマリ", "B4").expect("B4");
    assert_eq!(cell.number, Some(999.0));
    // The style index moved to the replacement, so the currency format is still
    // the cell's.
    assert_eq!(cell.display, "¥999");
}

#[test]
fn writing_a_value_removes_the_formula_that_was_there() {
    let original = fixture("sample-workbook.xlsx");
    let before = read(&original).cell("売上サマリ", "D4").expect("D4");
    assert_eq!(before.formula.as_deref(), Some("SUM(B4:C4)"));

    let written = write(&original, vec![edit("D4", NewValue::Number(1.0))]);
    let after = read(&written).cell("売上サマリ", "D4").expect("D4");

    // Typing over a formula replaces it, which is what a spreadsheet does.
    assert!(after.formula.is_none());
    assert_eq!(after.number, Some(1.0));
}

#[test]
fn a_formula_can_be_written_and_xml_characters_round_trip() {
    let original = fixture("sample-workbook.xlsx");
    let formula = r#"IF(B4<C4,"A&B","")"#;
    let written = write(
        &original,
        vec![edit("A20", NewValue::Formula(formula.to_string()))],
    );

    let cell = read(&written).cell("売上サマリ", "A20").expect("A20");
    assert_eq!(cell.formula.as_deref(), Some(formula));
    assert_eq!(cell.kind, jade_xlsx_headless_api::CellKind::Empty);
}

#[test]
fn an_empty_formula_is_rejected_before_the_workbook_changes() {
    let original = fixture("sample-workbook.xlsx");
    let error = write_xlsx(
        &original,
        &[SheetEdits {
            part_name: SHEET.to_string(),
            cells: vec![edit("A20", NewValue::Formula(String::new()))],
        }],
    )
    .expect_err("empty formula");
    assert!(error.to_string().contains("empty formula"));
}

#[test]
fn text_round_trips_including_the_characters_xml_cares_about() {
    let original = fixture("sample-workbook.xlsx");
    let awkward = "a & b < c > d \" e ' f 横浜";
    let written = write(
        &original,
        vec![edit("A4", NewValue::Text(awkward.to_string()))],
    );

    let cell = read(&written).cell("売上サマリ", "A4").expect("A4");
    assert_eq!(cell.text.as_deref(), Some(awkward));
    assert_eq!(cell.display, awkward);
}

#[test]
fn a_blank_keeps_the_cell_and_clears_the_value() {
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("B4", NewValue::Blank)]);

    let cell = read(&written).cell("売上サマリ", "B4").expect("B4");
    assert_eq!(cell.kind, jade_xlsx_headless_api::CellKind::Empty);
    assert_eq!(cell.display, "");
}

#[test]
fn a_new_cell_lands_in_column_order_within_its_row() {
    let original = fixture("sample-workbook.xlsx");
    // Row 4 holds A..D; F comes after all of them.
    let written = write(&original, vec![edit("F4", NewValue::Number(7.0))]);

    let workbook = read(&written);
    let row = workbook.range("売上サマリ", "A4:F4").expect("row");
    let numbers: Vec<Option<f64>> = row.rows[0].iter().map(|cell| cell.number).collect();
    assert_eq!(numbers[5], Some(7.0));
    // The cells that were there are untouched.
    assert_eq!(numbers[1], Some(12_850_000.0));
}

#[test]
fn a_new_cell_before_the_existing_ones_still_sorts_first() {
    // Row 1 holds only the merged title in A1; B1 has to go after it, and a
    // cell written into a row that starts later has to go before.
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("A2", NewValue::Number(5.0))]);

    let workbook = read(&written);
    assert_eq!(
        workbook.cell("売上サマリ", "A2").expect("A2").number,
        Some(5.0)
    );
    // Row 3's heading is still where it was.
    assert_eq!(
        workbook.cell("売上サマリ", "A3").expect("A3").display,
        "支店"
    );
}

#[test]
fn a_new_row_lands_in_row_order() {
    let original = fixture("sample-workbook.xlsx");
    // Row 8 is not written in the fixture; row 9 and 10 are.
    let written = write(&original, vec![edit("A8", NewValue::Text("挿入".into()))]);

    let workbook = read(&written);
    assert_eq!(
        workbook.cell("売上サマリ", "A8").expect("A8").display,
        "挿入"
    );
    // The rows on either side are undisturbed.
    assert_eq!(
        workbook.cell("売上サマリ", "A9").expect("A9").display,
        "更新日"
    );
    assert_eq!(
        workbook.cell("売上サマリ", "A7").expect("A7").display,
        "総計"
    );
}

#[test]
fn a_row_past_the_end_is_appended() {
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("B99", NewValue::Number(42.0))]);

    let workbook = read(&written);
    assert_eq!(
        workbook.cell("売上サマリ", "B99").expect("B99").number,
        Some(42.0)
    );
    assert_eq!(
        workbook.summary().sheets[0].used_range.as_deref(),
        Some("A1:D99")
    );
}

#[test]
fn several_edits_at_once_all_land() {
    let original = fixture("sample-workbook.xlsx");
    let written = write(
        &original,
        vec![
            edit("B4", NewValue::Number(1.0)),
            edit("C5", NewValue::Number(2.0)),
            edit("A20", NewValue::Text("下".into())),
            edit("A2", NewValue::Text("上".into())),
        ],
    );

    let workbook = read(&written);
    assert_eq!(
        workbook.cell("売上サマリ", "B4").expect("B4").number,
        Some(1.0)
    );
    assert_eq!(
        workbook.cell("売上サマリ", "C5").expect("C5").number,
        Some(2.0)
    );
    assert_eq!(
        workbook.cell("売上サマリ", "A20").expect("A20").display,
        "下"
    );
    assert_eq!(workbook.cell("売上サマリ", "A2").expect("A2").display, "上");
}

#[test]
fn every_other_part_comes_back_byte_for_byte() {
    // The reason to patch rather than regenerate: a save that changed one cell
    // must not rewrite parts this crate has never heard of.
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("B4", NewValue::Number(1.0))]);

    let before = OpcPackage::open(&original).expect("before");
    let after = OpcPackage::open(&written).expect("after");

    assert_eq!(before.part_names(), after.part_names());
    for name in before.part_names() {
        // The workbook part carries the recalculation flag, so it changes too.
        if name == SHEET || name == "xl/workbook.xml" {
            continue;
        }
        assert_eq!(
            before.part(name),
            after.part(name),
            "{name} changed and should not have"
        );
    }
}

#[test]
fn the_edited_sheet_keeps_everything_the_edit_did_not_touch() {
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("B4", NewValue::Number(1.0))]);

    let before = OpcPackage::open(&original).expect("before");
    let after = OpcPackage::open(&written).expect("after");
    let before_xml = before.part_str(SHEET).expect("read").expect("part");
    let after_xml = after.part_str(SHEET).expect("read").expect("part");

    // The merge, the column widths and the row heights are all still there.
    for fragment in [
        "<mergeCells count=\"1\">",
        "<mergeCell ref=\"A1:D1\"/>",
        "width=\"14.7109375\"",
        "ht=\"34\"",
    ] {
        assert!(after_xml.contains(fragment), "{fragment} was lost");
    }
    // And the change really is small.
    assert!(
        (after_xml.len() as i64 - before_xml.len() as i64).abs() < 200,
        "the part grew or shrank by more than one cell's worth"
    );
}

#[test]
fn a_workbook_written_twice_is_stable() {
    let original = fixture("sample-workbook.xlsx");
    let once = write(&original, vec![edit("B4", NewValue::Number(1.0))]);
    let twice = write(&once, vec![edit("B4", NewValue::Number(1.0))]);
    assert_eq!(once, twice);
}

#[test]
fn a_write_asks_the_spreadsheet_to_recalculate() {
    // Formulas that depended on the edited cell now hold stale cached values,
    // and nothing here can recalculate them. `fullCalcOnLoad` is how the file
    // says so, and every application that reads `.xlsx` honours it.
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![edit("B4", NewValue::Number(1.0))]);

    let package = OpcPackage::open(&written).expect("open");
    let workbook = package
        .part_str("xl/workbook.xml")
        .expect("read")
        .expect("part");
    assert!(workbook.contains("fullCalcOnLoad=\"1\""), "{workbook}");
    // Inserted where the schema allows it, not simply appended.
    assert!(
        workbook.find("<calcPr").expect("calcPr") > workbook.find("</sheets>").expect("sheets")
    );
    // And the sheet list is intact.
    assert!(workbook.contains("売上サマリ"));
}

#[test]
fn asking_twice_does_not_stack_the_flag() {
    let original = fixture("sample-workbook.xlsx");
    let once = write(&original, vec![edit("B4", NewValue::Number(1.0))]);
    let twice = write(&once, vec![edit("B5", NewValue::Number(2.0))]);

    let package = OpcPackage::open(&twice).expect("open");
    let workbook = package
        .part_str("xl/workbook.xml")
        .expect("read")
        .expect("part");
    assert_eq!(workbook.matches("<calcPr").count(), 1);
    assert_eq!(workbook.matches("fullCalcOnLoad").count(), 1);
}

#[test]
fn an_existing_calc_pr_is_amended_rather_than_duplicated() {
    let xml = r#"<workbook><sheets><sheet name="a"/></sheets><calcPr calcId="191029"/></workbook>"#;
    let patched = jade_xlsx_writer::request_full_recalculation(xml);
    assert_eq!(
        patched,
        r#"<workbook><sheets><sheet name="a"/></sheets><calcPr calcId="191029" fullCalcOnLoad="1"/></workbook>"#
    );

    // One that says not to recalculate is turned around rather than left.
    let refusing = r#"<workbook><sheets/><calcPr fullCalcOnLoad="0"/></workbook>"#;
    assert!(
        jade_xlsx_writer::request_full_recalculation(refusing).contains(r#"fullCalcOnLoad="1""#)
    );
}

#[test]
fn recalculation_preserves_a_workbook_namespace_prefix() {
    let existing = r#"<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheets/><x:calcPr calcId="1"/></x:workbook>"#;
    let patched = jade_xlsx_writer::request_full_recalculation(existing);
    assert!(patched.contains(r#"<x:calcPr calcId="1" fullCalcOnLoad="1"/>"#));

    let absent = r#"<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheets/></x:workbook>"#;
    let inserted = jade_xlsx_writer::request_full_recalculation(absent);
    assert!(inserted.contains(r#"<x:sheets/><x:calcPr calcId="0" fullCalcOnLoad="1"/>"#));
    assert!(!inserted.contains("<calcPr"));
}

#[test]
fn a_programmatic_edit_outside_xlsx_bounds_is_rejected() {
    let original = fixture("sample-workbook.xlsx");
    let error = write_xlsx(
        &original,
        &[SheetEdits {
            part_name: SHEET.to_string(),
            // Headless string inputs go through CellRef::parse, but callers of
            // the writer can construct a CellRef directly and need the same
            // XLSX bounds enforced at this public boundary.
            cells: vec![CellEdit {
                reference: CellRef::new(16_384, 0),
                value: NewValue::Number(1.0),
                style_index: None,
            }],
        }],
    )
    .expect_err("XFE1 is outside an XLSX worksheet");

    assert!(error.to_string().contains("outside XLSX worksheet bounds"));
}

#[test]
fn non_finite_numbers_are_rejected_before_writing_invalid_xml() {
    let original = fixture("sample-workbook.xlsx");
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let error = write_xlsx(
            &original,
            &[SheetEdits {
                part_name: SHEET.to_string(),
                cells: vec![edit("B4", NewValue::Number(number))],
            }],
        )
        .expect_err("XLSX numeric cells must be finite");
        assert!(error.to_string().contains("non-finite number"));
    }
}

#[test]
fn adding_a_cell_expands_the_worksheet_dimension() {
    let xml = r#"<worksheet><dimension ref="A1:B2"/><sheetData>
      <row r="1"><c r="A1"><v>1</v></c></row>
    </sheetData></worksheet>"#;
    let patched =
        jade_xlsx_writer::patch_sheet(xml, &[edit("D4", NewValue::Text("outside".into()))])
            .expect("patch");

    assert!(patched.contains(r#"<dimension ref="A1:D4"/>"#), "{patched}");
}

#[test]
fn generated_worksheet_elements_keep_the_namespace_prefix() {
    let xml = r#"<x:worksheet xmlns:x="urn:sheet"><x:dimension ref="A1"/><x:sheetData>
      <x:row r="1"><x:c r="A1"><x:v>1</x:v></x:c></x:row>
    </x:sheetData></x:worksheet>"#;
    let patched = jade_xlsx_writer::patch_sheet(
        xml,
        &[
            edit("B1", NewValue::Number(2.0)),
            edit("A2", NewValue::Text("next".into())),
        ],
    )
    .expect("patch");

    assert!(
        patched.contains(r#"<x:c r="B1"><x:v>2</x:v></x:c>"#),
        "{patched}"
    );
    assert!(
        patched.contains(
            r#"<x:row r="2"><x:c r="A2" t="inlineStr"><x:is><x:t xml:space="preserve">next</x:t></x:is></x:c></x:row>"#
        ),
        "{patched}"
    );
    assert!(!patched.contains("<row "), "{patched}");
    assert!(!patched.contains("<c "), "{patched}");
}

#[test]
fn editing_implicit_rows_and_cells_updates_them_in_place() {
    let xml = r#"<worksheet><sheetData>
      <row r="5"><c r="A5"><v>5</v></c></row>
      <row><c t="inlineStr"><is><t>old</t></is></c></row>
    </sheetData></worksheet>"#;
    let patched =
        jade_xlsx_writer::patch_sheet(xml, &[edit("A6", NewValue::Text("updated".into()))])
            .expect("patch");

    assert!(patched.contains(r#"<c r="A6" t="inlineStr""#), "{patched}");
    assert!(patched.contains(">updated</t>"), "{patched}");
    assert_eq!(patched.matches(r#"r="A6""#).count(), 1, "{patched}");
    assert!(!patched.contains(r#"<row r="6">"#), "{patched}");
}

#[test]
fn a_carriage_return_survives_a_write_and_read_back() {
    // XML 1.0 permits a literal CR but a conforming parser normalises it to
    // LF on read, so writing it literally silently corrupts the text. Excel
    // stores it as `_x000D_` (ST_Xstring), and so does this writer.
    let original = fixture("sample-workbook.xlsx");
    let multiline = "line1\r\nline2\rline3";
    let written = write(
        &original,
        vec![edit("A20", NewValue::Text(multiline.to_string()))],
    );

    let cell = read(&written).cell("売上サマリ", "A20").expect("A20");
    assert_eq!(cell.text.as_deref(), Some(multiline));
}

#[test]
fn text_that_merely_looks_like_an_x_escape_stays_literal() {
    // The reader decodes `_xHHHH_`, so the writer has to protect a literal one
    // with `_x005F_` — otherwise writing the text `_x000A_` reads back as a
    // newline.
    let original = fixture("sample-workbook.xlsx");
    let literal = "_x000A_ and _x005F_ stay text";
    let written = write(
        &original,
        vec![edit("A20", NewValue::Text(literal.to_string()))],
    );

    let cell = read(&written).cell("売上サマリ", "A20").expect("A20");
    assert_eq!(cell.text.as_deref(), Some(literal));
}

// ── styles ──────────────────────────────────────────────────────────────

fn style_of(bytes: &[u8], reference: &str) -> Option<u32> {
    let workbook = jade_xlsx_reader::read_xlsx(bytes).expect("read");
    let at = CellRef::parse(reference).expect("reference");
    workbook.sheets[0]
        .rows
        .iter()
        .find(|row| row.index == at.row)
        .and_then(|row| row.cells.iter().find(|cell| cell.reference == at))
        .map(|cell| cell.style_index)
        .unwrap_or_else(|| panic!("{reference} is not in the sheet"))
}

#[test]
fn a_style_index_on_a_new_cell_is_written_with_it() {
    let original = fixture("sample-workbook.xlsx");
    // Style 4 is the fixture's `"¥"#,##0` currency format.
    let written = write(&original, vec![styled("F1", NewValue::Number(1.0), 4)]);
    assert_eq!(style_of(&written, "F1"), Some(4));
    assert_eq!(
        read(&written)
            .cell("売上サマリ", "F1")
            .expect("cell")
            .display,
        "¥1"
    );
}

#[test]
fn a_style_index_replaces_the_one_a_cell_had() {
    let original = fixture("sample-workbook.xlsx");
    assert_eq!(style_of(&original, "B4"), Some(4));
    let written = write(&original, vec![styled("B4", NewValue::Number(5.0), 3)]);
    assert_eq!(style_of(&written, "B4"), Some(3));
    assert_eq!(
        read(&written)
            .cell("売上サマリ", "B4")
            .expect("cell")
            .display,
        "5"
    );
    // Without one, the replacement keeps the style it found — as before.
    let kept = write(&original, vec![edit("B4", NewValue::Number(5.0))]);
    assert_eq!(style_of(&kept, "B4"), Some(4));
}

#[test]
fn an_unchanged_value_with_a_style_keeps_the_content_and_restyles_the_tag() {
    let original = fixture("sample-workbook.xlsx");
    // D4 is a formula with a cached value: both have to survive untouched.
    let written = write(&original, vec![styled("D4", NewValue::Unchanged, 3)]);
    assert_eq!(style_of(&written, "D4"), Some(3));
    let cell = read(&written).cell("売上サマリ", "D4").expect("cell");
    assert_eq!(cell.formula.as_deref(), Some("SUM(B4:C4)"));
    assert_eq!(cell.number, Some(28_080_000.0));
    assert_eq!(cell.display, "28080000");

    let package = OpcPackage::open(&written).expect("package");
    let xml = package.part_str(SHEET).expect("read").expect("sheet");
    assert!(
        xml.contains(r#"<c r="D4" s="3"><f>SUM(B4:C4)</f><v>28080000</v></c>"#),
        "{xml}"
    );

    // Unchanged with no style is a no-op; the part comes back byte-identical.
    let untouched = write(&original, vec![edit("D4", NewValue::Unchanged)]);
    let before = OpcPackage::open(&original).expect("before");
    let after = OpcPackage::open(&untouched).expect("after");
    assert_eq!(before.part(SHEET), after.part(SHEET));
}

#[test]
fn an_unchanged_cell_the_sheet_does_not_have_becomes_an_empty_styled_cell() {
    let original = fixture("sample-workbook.xlsx");
    let written = write(&original, vec![styled("F1", NewValue::Unchanged, 4)]);
    assert_eq!(style_of(&written, "F1"), Some(4));
    let cell = read(&written).cell("売上サマリ", "F1").expect("cell");
    assert_eq!(cell.kind, jade_xlsx_headless_api::CellKind::Empty);
    assert_eq!(cell.display, "");
}

#[test]
fn a_self_closing_cell_restyled_in_place_stays_self_closing() {
    let original = fixture("sample-workbook.xlsx");
    // D10 is `<c r="D10" s="10"/>`.
    let written = write(&original, vec![styled("D10", NewValue::Unchanged, 3)]);
    let package = OpcPackage::open(&written).expect("package");
    let xml = package.part_str(SHEET).expect("read").expect("sheet");
    assert!(xml.contains(r#"<c r="D10" s="3"/>"#), "{xml}");
}
