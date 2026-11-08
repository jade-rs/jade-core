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

#[test]
fn inline_formula_alternatives_rebase_each_formula_instead_of_staying_stale() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f><mc:AlternateContent><mc:Choice Requires="m">A1+1</mc:Choice><mc:Fallback>A1+2</mc:Fallback></mc:AlternateContent></f><v>2</v></c></row></sheetData></worksheet>"#;
    let workbook = read_xlsx(&workbook_with_sheet(xml)).unwrap();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        xml,
        &workbook.sheets[0],
        true,
        &change("Sheet1", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    assert!(updated.contains(">A2+1</mc:Choice>"), "{updated}");
    assert!(updated.contains(">A2+2</mc:Fallback>"), "{updated}");
    let moved = read_xlsx(&workbook_with_sheet(&updated)).unwrap();
    assert_eq!(
        moved.sheets[0].rows[0].cells[1].formula.as_deref(),
        Some("A2+1")
    );
    let mut book = HeadlessWorkbook::open_for_editing(&workbook_with_sheet(xml)).unwrap();
    let before = book.bytes().to_vec();
    book.insert_rows("Sheet1", 1, 1).unwrap();
    assert_eq!(book.cell("Sheet1", "B2").unwrap().display, "2");
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    assert!(part(&saved, SHEET).contains(">A2+2</mc:Fallback>"));
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(reopened.cell("Sheet1", "B2").unwrap().display, "2");
}

fn formula_alternative(required: &str, choice: &str, fallback: &str) -> String {
    format!("<mc:AlternateContent>\n<mc:Choice Requires=\"u\"><u:opaque>A99</u:opaque></mc:Choice><mc:Choice Requires=\"{required}\">{choice}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback>\n</mc:AlternateContent>")
}

fn inline_formula_sheet(formula: &str) -> String {
    let ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    format!("<worksheet xmlns=\"{ns}\" xmlns:a=\"{ns}\" xmlns:b=\"{ns}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:u=\"urn:future\" xmlns:p=\"urn:processed\" mc:Ignorable=\"p\" mc:ProcessContent=\"p:wrap\"><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c></row><row r=\"4\"><c r=\"D4\"><f>{formula}</f><v>0</v></c></row></sheetData></worksheet>")
}

fn selected_inline_formula(xml: &str, mask: usize) -> String {
    let mut xml = xml.to_string();
    for (bit, prefix) in [(1, "a"), (2, "b")] {
        if mask & bit != 0 {
            xml = xml.replace(
                &format!(
                    "xmlns:{prefix}=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\""
                ),
                &format!("xmlns:{prefix}=\"urn:unsupported-{prefix}\""),
            );
        }
    }
    read_xlsx(&workbook_with_sheet(&xml)).unwrap().sheets[0]
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .find_map(|c| c.formula.clone())
        .unwrap()
}

#[test]
fn dynamic_capability_inline_formula_branches_both_follow_structure_edits() {
    let xml = inline_formula_sheet(&formula_alternative("d", "A1+1", "A1+2"))
        .replace("<worksheet ", "<worksheet xmlns:d=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\" ");
    assert_eq!(selected_inline_formula(&xml, 0), "A1+1");
    let workbook = read_xlsx(&workbook_with_sheet(&xml)).unwrap();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        &xml,
        &workbook.sheets[0],
        true,
        &change("Sheet1", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    assert!(updated.contains(">A2+1</mc:Choice>"), "{updated}");
    assert!(updated.contains(">A2+2</mc:Fallback>"), "{updated}");
    assert!(updated.contains("<u:opaque>A99</u:opaque>"));
    assert_eq!(selected_inline_formula(&updated, 0), "A2+1");
    let fallback = updated.replace(
        "http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray",
        "urn:future-dynamic",
    );
    assert_eq!(selected_inline_formula(&fallback, 0), "A2+2");
}

#[test]
fn split_coordinates_and_ranges_rebase_all_independent_inline_alternatives() {
    let column = formula_alternative("a", "<p:wrap><![CDATA[A]]></p:wrap><!--keep-->", "B");
    let row = formula_alternative("b", "9", "10");
    let range = format!(
        "SUM({}:{})",
        formula_alternative("a", "A9", "B9"),
        formula_alternative("b", "C10", "D10")
    );
    for (formula, range) in [
        (format!("{column}{row}"), false),
        (format!("\"あ\"&amp;{column}{row}"), false),
        (range, true),
    ] {
        let xml = inline_formula_sheet(&formula);
        let workbook = read_xlsx(&workbook_with_sheet(&xml)).unwrap();
        for axis in [Axis::Rows, Axis::Columns] {
            let updated = jade_xlsx_writer::shift::shift_sheet(
                &xml,
                &workbook.sheets[0],
                true,
                &change("Sheet1", axis, 0, 1, true),
            )
            .unwrap();
            assert_eq!(updated.matches("<u:opaque>A99</u:opaque>").count(), 2);
            if !range {
                assert!(updated.contains("<p:wrap>"));
                assert!(updated.contains("<!--keep-->"));
            }
            for mask in 0..4 {
                let expected = if range {
                    let (left, right) = if axis == Axis::Rows {
                        (["A10", "B10"], ["C11", "D11"])
                    } else {
                        (["B9", "C9"], ["D10", "E10"])
                    };
                    format!("SUM({}:{})", left[mask & 1], right[(mask >> 1) & 1])
                } else {
                    (if axis == Axis::Rows {
                        ["A10", "B10", "A11", "B11"]
                    } else {
                        ["B9", "C9", "B10", "C10"]
                    })[mask]
                        .to_string()
                };
                assert_eq!(
                    selected_inline_formula(&updated, mask),
                    if formula.starts_with("\"あ\"") {
                        format!("\"あ\"&{expected}")
                    } else {
                        expected
                    },
                    "{updated}"
                );
            }
        }
    }
}

#[test]
fn deleted_branch_references_can_relocate_their_shared_prefix() {
    let xml = inline_formula_sheet(&format!("A{}", formula_alternative("a", "1", "2")));
    let workbook = read_xlsx(&workbook_with_sheet(&xml)).unwrap();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        &xml,
        &workbook.sheets[0],
        true,
        &change("Sheet1", Axis::Rows, 0, 1, false),
    )
    .unwrap();
    assert_eq!(selected_inline_formula(&updated, 0), "#REF!");
    assert_eq!(selected_inline_formula(&updated, 1), "A1");
    assert!(updated.contains("<u:opaque>A99</u:opaque>"));
}

#[test]
fn qualified_inline_references_update_when_only_another_sheet_moves() {
    let xml = inline_formula_sheet(&format!("Sheet1!{}", formula_alternative("a", "A1", "A2")));
    let mut workbook = read_xlsx(&workbook_with_sheet(&xml)).unwrap();
    workbook.sheets[0].name = "Other".into();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        &xml,
        &workbook.sheets[0],
        false,
        &change("Sheet1", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    assert_eq!(selected_inline_formula(&updated, 0), "Sheet1!A2");
    assert_eq!(selected_inline_formula(&updated, 1), "Sheet1!A3");
    assert!(updated.contains("<c r=\"D4\">"));
}

#[test]
fn incompatible_shared_formula_text_rejects_structure_without_changing_history() {
    let xml = inline_formula_sheet(&format!(
        "{}{}",
        formula_alternative("a", "A", "B"),
        formula_alternative("b", "1", "2")
    ));
    let mut book = HeadlessWorkbook::open_for_editing(&workbook_with_sheet(&xml)).unwrap();
    book.set(
        "Sheet1",
        "F1",
        jade_xlsx_headless_api::CellInput::Text("ready".into()),
    )
    .unwrap();
    let before = book.bytes().to_vec();
    let error = book.delete_columns("Sheet1", "A", 1).unwrap_err();
    assert!(error.is_invalid_input());
    assert!(matches!(
        error,
        jade_xlsx_headless_editor::HeadlessError::Write(
            jade_xlsx_writer::WriteError::FormulaMarkup(_)
        )
    ));
    assert_eq!(book.bytes(), before);
    book.undo().unwrap();
    assert_eq!(book.cell("Sheet1", "F1").unwrap().display, "");
    book.redo().unwrap();
    assert_eq!(book.cell("Sheet1", "F1").unwrap().display, "ready");
}

#[test]
fn deeply_nested_inline_alternatives_shift_without_recursive_xml_rewrites() {
    let mut formula = "A1".to_string();
    for _ in 0..256 {
        formula = format!("<mc:AlternateContent><mc:Choice Requires=\"a\">{formula}</mc:Choice><mc:Fallback>A1</mc:Fallback></mc:AlternateContent>");
    }
    let xml = inline_formula_sheet(&formula);
    let workbook = read_xlsx(&workbook_with_sheet(&xml)).unwrap();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        &xml,
        &workbook.sheets[0],
        true,
        &change("Sheet1", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    assert_eq!(updated.matches("<mc:AlternateContent>").count(), 256);
    let formula = updated
        .split_once("<f>")
        .unwrap()
        .1
        .split_once("</f>")
        .unwrap()
        .0;
    assert_eq!(formula.matches("A2").count(), 257);
    assert_eq!(selected_inline_formula(&updated, 0), "A2");
    assert_eq!(selected_inline_formula(&updated, 1), "A2");
}

#[test]
fn unrelated_sheets_keep_complex_unqualified_inline_formulas_byte_exact() {
    let formula = (0..11)
        .map(|_| formula_alternative("a", "A1", "B1"))
        .collect::<Vec<_>>()
        .join("+");
    let xml = inline_formula_sheet(&formula);
    let workbook = read_xlsx(&workbook_with_sheet(&xml)).unwrap();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        &xml,
        &workbook.sheets[0],
        false,
        &change("Other", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    assert_eq!(updated, xml);
}

#[test]
fn moving_formula_text_decodes_cdata_and_keeps_comments_and_processed_wrappers() {
    for (formula, original, shifted) in [
        ("<![CDATA[A1+1]]>", "A1+1", "A2+1"),
        ("A<!--keep-->1+1", "A1+1", "A2+1"),
        ("A<?keep marker?>1+1", "A1+1", "A2+1"),
        ("<u:wrap>A1</u:wrap>+1", "A1+1", "A2+1"),
        ("<u:ignored>A99</u:ignored>A1+1", "A1+1", "A2+1"),
        (
            "A<u:wrap><!--keep--><![CDATA[1]]></u:wrap>+1",
            "A1+1",
            "A2+1",
        ),
        (
            "<![CDATA[IF(A1<2,\"x&y\",A1)]]>",
            "IF(A1<2,\"x&y\",A1)",
            "IF(A2<2,\"x&y\",A2)",
        ),
        (
            "IF(A<!--keep-->1&lt;2,&quot;x&amp;y&quot;,A1)",
            "IF(A1<2,\"x&y\",A1)",
            "IF(A2<2,\"x&y\",A2)",
        ),
    ] {
        let xml = format!("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:u=\"urn:future\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:wrap\"><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><f>{formula}</f><v>2</v></c></row></sheetData></worksheet>");
        let bytes = workbook_with_sheet(&xml);
        let workbook = read_xlsx(&bytes).unwrap();
        assert_eq!(
            workbook.sheets[0].rows[0].cells[1].formula.as_deref(),
            Some(original)
        );
        assert_eq!(
            jade_xlsx_writer::shift::shift_sheet(
                &xml,
                &workbook.sheets[0],
                false,
                &change("Other", Axis::Rows, 0, 1, true)
            )
            .unwrap(),
            xml
        );
        let updated = jade_xlsx_writer::shift::shift_sheet(
            &xml,
            &workbook.sheets[0],
            true,
            &change("Sheet1", Axis::Rows, 0, 1, true),
        )
        .unwrap();
        let moved = read_xlsx(&workbook_with_sheet(&updated)).unwrap();
        assert_eq!(
            moved.sheets[0].rows[0].cells[1].formula.as_deref(),
            Some(shifted),
            "{updated}"
        );
        if formula.contains("<!--keep-->") {
            assert!(updated.contains("<!--keep-->"));
        }
        if formula.contains("<u:wrap>") {
            assert!(updated.contains("<u:wrap>"));
        }
        if formula.contains("<?keep marker?>") {
            assert!(updated.contains("<?keep marker?>"));
        }
        if formula.contains("<u:ignored>") {
            assert!(updated.contains("<u:ignored>A99</u:ignored>"));
        }
    }
}

#[test]
fn defined_name_xml_text_moves_without_changing_comments_or_unrelated_names() {
    let untouched = "<definedName name=\"Other\"><![CDATA[Other!$A$1]]></definedName>";
    let xml = format!("<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><definedNames><definedName name=\"Example\"><![CDATA[Sheet1!$A$]]><!--keep-->1</definedName>{untouched}</definedNames></workbook>");
    let moved = jade_xlsx_writer::shift::shift_defined_names(
        &xml,
        &change("Sheet1", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    let part = jade_xlsx_reader::workbook_part::parse_workbook(&moved).unwrap();
    assert_eq!(part.defined_names[0].formula, "Sheet1!$A$2");
    assert!(moved.contains("<!--keep-->"));
    assert!(moved.contains(untouched));
}

#[test]
fn a_cdata_defined_name_still_calculates_after_structure_history_and_reopen() {
    let source = workbook_with_sheet(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>Price+1</f><v>2</v></c></row></sheetData></worksheet>"#,
    );
    let mut package = OpcPackage::open(&source).unwrap();
    let workbook = package.part_str("xl/workbook.xml").unwrap().unwrap().replace("</sheets>", "</sheets><definedNames><definedName name=\"Price\"><![CDATA[Sheet1!$A$1]]></definedName></definedNames>");
    package.replace("xl/workbook.xml", workbook.into_bytes());
    let mut book = HeadlessWorkbook::open_for_editing(&package.write().unwrap()).unwrap();
    assert_eq!(book.cell("Sheet1", "B1").unwrap().display, "2");
    book.insert_rows("Sheet1", 1, 1).unwrap();
    assert_eq!(book.cell("Sheet1", "B2").unwrap().display, "2");
    assert_eq!(book.workbook().defined_names[0].formula, "Sheet1!$A$2");
    book.undo().unwrap();
    assert_eq!(book.workbook().defined_names[0].formula, "Sheet1!$A$1");
    book.redo().unwrap();
    let reopened = HeadlessWorkbook::open_for_editing(&book.save_bytes().unwrap()).unwrap();
    assert_eq!(reopened.cell("Sheet1", "B2").unwrap().display, "2");
    assert_eq!(reopened.workbook().defined_names[0].formula, "Sheet1!$A$2");
}

#[test]
fn implicit_cells_in_alternatives_shift_from_their_logical_coordinates() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:m="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><sheetData><row><c><v>1</v></c><mc:AlternateContent><mc:Choice Requires="m"><c><f>A1+1</f><v>2</v></c></mc:Choice><mc:Fallback><c><f>A1+2</f><v>3</v></c></mc:Fallback></mc:AlternateContent><c><v>4</v></c></row></sheetData></worksheet>"#;
    let workbook = read_xlsx(&workbook_with_sheet(xml)).unwrap();
    let updated = jade_xlsx_writer::shift::shift_sheet(
        xml,
        &workbook.sheets[0],
        true,
        &change("Sheet1", Axis::Rows, 0, 1, true),
    )
    .unwrap();
    assert_eq!(
        updated.matches("<c r=\"B2\">").count(),
        2,
        "alternatives must share B2: {updated}"
    );
    assert!(updated.contains("<c r=\"C2\"><v>4</v></c>"));
}

#[test]
fn shifting_the_grid_preserves_foreign_cells_and_formula_payloads() {
    let payload = "<u:c r=\"Z9\"><u:f>A1+99</u:f></u:c>";
    let foreign_formula = "<u:f>A1+88</u:f>";
    let xml = format!("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:u=\"urn:foreign\"><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c>{payload}<c r=\"B1\">{foreign_formula}<f>A1+1</f><v>2</v></c></row></sheetData></worksheet>");
    let bytes = workbook_with_sheet(&xml);
    let workbook = read_xlsx(&bytes).unwrap();
    for axis in [Axis::Rows, Axis::Columns] {
        let change = change("Sheet1", axis, 0, 1, true);
        let updated =
            jade_xlsx_writer::shift::shift_sheet(&xml, &workbook.sheets[0], true, &change).unwrap();
        assert!(
            updated.contains(payload),
            "foreign cell payload changed: {updated}"
        );
        assert!(
            updated.contains(foreign_formula),
            "foreign formula payload changed: {updated}"
        );
        assert!(updated.contains(if axis == Axis::Rows {
            "<f>A2+1</f>"
        } else {
            "<f>B1+1</f>"
        }));
    }
    let mut book = HeadlessWorkbook::open_for_editing(&bytes).unwrap();
    let before = book.bytes().to_vec();
    book.insert_rows("Sheet1", 1, 1).unwrap();
    assert_eq!(book.cell("Sheet1", "B2").unwrap().display, "2");
    assert!(part(book.bytes(), SHEET).contains(payload));
    book.undo().unwrap();
    assert_eq!(book.bytes(), before);
    book.redo().unwrap();
    let saved = book.save_bytes().unwrap();
    assert!(part(&saved, SHEET).contains(payload));
    assert!(part(&saved, SHEET).contains(foreign_formula));
    let reopened = HeadlessWorkbook::open_for_editing(&saved).unwrap();
    assert_eq!(reopened.cell("Sheet1", "B2").unwrap().display, "2");
}

#[test]
fn grid_shifts_keep_opaque_branches_and_move_understood_processed_cells() {
    let opaque = "<mc:Choice Requires=\"u\"><c r=\"Z9\"><f>A1+99</f></c></mc:Choice>";
    let payload = "<extLst><ext><c r=\"Y9\"><f>A1+88</f></c></ext></extLst>";
    let xml = format!("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:m=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:u=\"urn:future\" mc:Ignorable=\"u\" mc:ProcessContent=\"u:wrap\"><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><mc:AlternateContent>{opaque}<mc:Choice Requires=\"m\"><u:wrap><c r=\"B1\"><f>A1+1</f><v>2</v></c></u:wrap></mc:Choice><mc:Fallback><c r=\"C1\"><f>A1+2</f><v>3</v></c></mc:Fallback></mc:AlternateContent>{payload}</row></sheetData></worksheet>");
    let bytes = workbook_with_sheet(&xml);
    let workbook = read_xlsx(&bytes).unwrap();
    for wrapper in ["wrap", "row", "c", "f", "sheetData"] {
        let xml = xml.replace("u:wrap", &format!("u:{wrapper}"));
        for changed in [false, true] {
            let updated = jade_xlsx_writer::shift::shift_sheet(
                &xml,
                &workbook.sheets[0],
                changed,
                &change("Sheet1", Axis::Rows, 0, 1, true),
            )
            .unwrap();
            assert!(updated.contains(opaque));
            assert!(updated.contains(payload));
            assert!(updated.contains("<f>A2+1</f>"));
            assert!(updated.contains("<f>A2+2</f>"));
            assert!(updated.contains(&format!("<u:{wrapper}>")));
            if changed {
                assert!(updated.contains("<c r=\"B2\">"));
                assert!(updated.contains("<c r=\"C2\">"));
            }
        }
    }
}

#[test]
fn direct_writer_calls_cannot_split_legacy_array_declarations() {
    for reference in ["A1:C3", "A1:XFD1048576"] {
        let xml = format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f t="array" ref="{reference}">{{1;2;3}}</f><v>1</v></c></row></sheetData></worksheet>"#
        );
        let bytes = workbook_with_sheet(&xml);
        let workbook = read_xlsx(&bytes).unwrap();
        for axis in [Axis::Rows, Axis::Columns] {
            for inserted in [false, true] {
                let change = change("Sheet1", axis, 1, 1, inserted);
                assert!(
                    restructure(&bytes, &workbook, &change).is_err(),
                    "direct package writer accepted {reference}: {change:?}"
                );
                assert!(
                    jade_xlsx_writer::shift::shift_sheet(&xml, &workbook.sheets[0], true, &change)
                        .is_err(),
                    "direct sheet writer accepted {reference}: {change:?}"
                );
            }
        }
    }
}

#[test]
fn direct_writers_move_or_delete_whole_arrays_and_only_guard_legacy_kind() {
    let xml = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f t="array" ref="A1:C3">{1;2;3}</f><v>1</v></c></row></sheetData></worksheet>"#;
    let bytes = workbook_with_sheet(xml);
    let mut workbook = read_xlsx(&bytes).unwrap();
    for axis in [Axis::Rows, Axis::Columns] {
        let moved = if axis == Axis::Rows { "A2:C4" } else { "B1:D3" };
        let insertion = change("Sheet1", axis, 0, 1, true);
        let written = restructure(&bytes, &workbook, &insertion).unwrap();
        assert_eq!(
            read_xlsx(&written).unwrap().sheets[0].array_formulas[0].range,
            jade_xlsx_reader::CellRange::parse(moved).unwrap()
        );
        let deletion = change("Sheet1", axis, 0, 3, false);
        let written = restructure(&bytes, &workbook, &deletion).unwrap();
        assert!(read_xlsx(&written).unwrap().sheets[0]
            .array_formulas
            .is_empty());
    }
    for kind in [
        jade_xlsx_reader::ArrayFormulaKind::Dynamic,
        jade_xlsx_reader::ArrayFormulaKind::Unknown,
    ] {
        workbook.sheets[0].array_formulas[0].kind = kind;
        assert!(jade_xlsx_writer::shift::guard_legacy_array_structure(
            &workbook.sheets[0],
            &change("Sheet1", Axis::Rows, 1, 1, true)
        )
        .is_ok());
    }
}

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
