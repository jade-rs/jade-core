use jade_opc::OpcPackage;
use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{HeadlessWorkbook, TableOptions, TableUpdate};
use jade_xlsx_reader::{Axis, CellRange, DefinedName, Hyperlink};
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const PART: &str = "xl/worksheets/sheet1.xml";
fn range(s: &str) -> CellRange {
    CellRange::parse(s).unwrap()
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../public/fixtures/{name}.xlsx",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn custom(data: &str, tail: &str) -> HeadlessWorkbook {
    let mut package = OpcPackage::open(&fixture("sample-workbook")).unwrap();
    package.replace(PART,format!("<worksheet xmlns=\"{MAIN}\" xmlns:u=\"urn:test\"><sheetData>{data}</sheetData>{tail}</worksheet>").into_bytes());
    HeadlessWorkbook::open(&package.write().unwrap()).unwrap()
}
fn simple() -> HeadlessWorkbook {
    custom("<row r=\"1\"><c r=\"B1\"><v>1</v></c></row><row r=\"2\"><c r=\"B2\"><v>2</v></c></row><row r=\"3\"><c r=\"A3\"><v>10</v></c><c r=\"B3\"><v>3</v></c><c r=\"C3\"><v>30</v></c></row><row r=\"4\"><c r=\"A4\"><v>20</v></c><c r=\"B4\"><v>4</v></c><c r=\"C4\"><v>40</v></c></row>","")
}
fn sheet(book: &HeadlessWorkbook) -> String {
    book.workbook().sheets[0].name.clone()
}
fn part(book: &HeadlessWorkbook, p: &str) -> String {
    OpcPackage::open(book.bytes())
        .unwrap()
        .required_part_str(p)
        .unwrap()
}

#[test]
fn down_then_up_moves_only_the_strip_and_follows_dependents_with_exact_history() {
    let mut b = simple();
    let s = sheet(&b);
    let other = b.workbook().sheets[1].name.clone();
    b.set(&s, "H1", CellInput::Formula("B3+$B$4+C3+SUM(A3:C4)".into()))
        .unwrap();
    b.set(
        &other,
        "H1",
        CellInput::Formula(format!("'{}'!B3+B3", s.replace('\'', "''"))),
    )
    .unwrap();
    let before = b.bytes().to_vec();
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    assert_eq!(b.cell(&s, "B3").unwrap().number, None);
    assert_eq!(b.cell(&s, "B4").unwrap().number, Some(3.));
    assert_eq!(b.cell(&s, "B5").unwrap().number, Some(4.));
    assert_eq!(b.cell(&s, "C3").unwrap().number, Some(30.));
    assert_eq!(
        b.cell(&s, "H1").unwrap().formula.as_deref(),
        Some("B4+$B$5+C3+SUM(A3:C4)")
    );
    assert_eq!(
        b.cell(&other, "H1").unwrap().formula.as_deref(),
        Some(format!("'{}'!B4+B3", s.replace('\'', "''")).as_str())
    );
    let reopened = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(reopened.cell(&s, "B5").unwrap().number, Some(4.));
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    b.shift_cells(&s, range("B3"), Axis::Rows, false).unwrap();
    assert_eq!(b.cell(&s, "B3").unwrap().number, Some(3.));
    assert_eq!(b.cell(&s, "B4").unwrap().number, Some(4.));
    assert_eq!(
        b.cell(&s, "H1").unwrap().formula.as_deref(),
        Some("B3+$B$4+C3+SUM(A3:C4)")
    );
}

#[test]
fn horizontal_insert_and_delete_keep_other_rows_and_make_deleted_refs_errors() {
    let mut b = simple();
    let s = sheet(&b);
    b.set(&s, "H1", CellInput::Formula("B3+C3+B4".into()))
        .unwrap();
    b.shift_cells(&s, range("B3:C3"), Axis::Columns, true)
        .unwrap();
    assert_eq!(b.cell(&s, "D3").unwrap().number, Some(3.));
    assert_eq!(b.cell(&s, "E3").unwrap().number, Some(30.));
    assert_eq!(b.cell(&s, "B4").unwrap().number, Some(4.));
    assert_eq!(
        b.cell(&s, "H1").unwrap().formula.as_deref(),
        Some("D3+E3+B4")
    );
    b.shift_cells(&s, range("D3"), Axis::Columns, false)
        .unwrap();
    assert_eq!(b.cell(&s, "D3").unwrap().number, Some(30.));
    assert_eq!(
        b.cell(&s, "H1").unwrap().formula.as_deref(),
        Some("#REF!+D3+B4")
    );
}

#[test]
fn raw_rich_text_unknown_attributes_row_properties_and_inherited_formats_survive() {
    let mut b=custom("<row r=\"3\" ht=\"31\" customHeight=\"1\" customFormat=\"1\" s=\"1\" hidden=\"1\" u:row=\"keep\"><c r=\"B3\" t=\"inlineStr\" u:cell=\"keep\"><is><r><rPr><b/></rPr><t>Rich &amp; text</t></r></is><u:data keep=\"yes\"/></c><extLst><ext uri=\"keep-row\"><u:info/></ext></extLst></row><row r=\"4\" ht=\"20\" customHeight=\"1\"><c r=\"C4\"><v>99</v></c></row>","<extLst><ext uri=\"keep-sheet\"><u:sheet/></ext></extLst>");
    let s = sheet(&b);
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let r = &b.workbook().sheets[0];
    assert_eq!(
        r.rows.iter().find(|r| r.index == 2).unwrap().height_points,
        Some(31.)
    );
    assert_eq!(
        r.rows.iter().find(|r| r.index == 3).unwrap().height_points,
        Some(20.)
    );
    assert_eq!(
        r.rows
            .iter()
            .find(|r| r.index == 3)
            .unwrap()
            .cells
            .iter()
            .find(|c| c.reference.column == 1)
            .unwrap()
            .style_index,
        Some(1)
    );
    assert_eq!(b.cell(&s, "B4").unwrap().display, "Rich & text");
    let xml = part(&b, PART);
    for keep in [
        "u:cell=\"keep\"",
        "u:row=\"keep\"",
        "<rPr><b/></rPr>",
        "<u:data keep=\"yes\"/>",
        "keep-row",
        "keep-sheet",
    ] {
        assert!(xml.contains(keep), "{keep}: {xml}");
    }
    let row4 = xml
        .split("r=\"4\"")
        .nth(1)
        .unwrap()
        .split("</row>")
        .next()
        .unwrap();
    assert!(row4.find("<c").unwrap() < row4.find("</c>").unwrap());
}

#[test]
fn shared_formulas_expand_before_the_reference_strip_changes_their_pattern() {
    let mut b=custom("<row r=\"1\"><c r=\"B1\"><v>1</v></c></row><row r=\"2\"><c r=\"B2\"><v>2</v></c></row><row r=\"3\"><c r=\"A3\"><v>10</v></c><c r=\"B3\"><f t=\"shared\" si=\"0\" ref=\"B3:B4\" u:f=\"keep\">B1+$A3</f><v>11</v></c><c r=\"C3\"><f t=\"shared\" si=\"1\" ref=\"C3:C4\">B3</f><v>11</v></c></row><row r=\"4\"><c r=\"A4\"><v>20</v></c><c r=\"B4\"><f t=\"shared\" si=\"0\"/><v>22</v></c><c r=\"C4\"><f t=\"shared\" si=\"1\"/><v>22</v></c></row>","");
    let s = sheet(&b);
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    for (at, f, n) in [
        ("B4", "B1+$A3", 11.),
        ("B5", "B2+$A4", 22.),
        ("C3", "B4", 11.),
        ("C4", "B5", 22.),
    ] {
        let c = b.cell(&s, at).unwrap();
        assert_eq!(c.formula.as_deref(), Some(f), "{at}");
        assert_eq!(c.number, Some(n), "{at}");
    }
    assert!(!part(&b, PART).contains("t=\"shared\""));
    assert!(part(&b, PART).contains("u:f=\"keep\""));
}

#[test]
fn names_notes_hyperlinks_and_standard_rule_coverage_follow_the_cells() {
    let mut b=custom("<row r=\"3\"><c r=\"B3\"><v>3</v></c></row>","<conditionalFormatting sqref=\"B3:C4\"><cfRule type=\"expression\" priority=\"1\"><formula>B3&gt;0</formula></cfRule></conditionalFormatting><dataValidations count=\"1\"><dataValidation type=\"custom\" sqref=\"B3:C4\"><formula1>B3&gt;0</formula1></dataValidation></dataValidations>");
    let s = sheet(&b);
    let other = b.workbook().sheets[1].name.clone();
    b.set_defined_name(&DefinedName {
        name: "LocalProbe".into(),
        formula: "B3".into(),
        local_sheet_index: Some(0),
        hidden: false,
    })
    .unwrap();
    b.set_note(&s, "B3", "move me", "Author").unwrap();
    b.set_note(&s, "D3", "stay here", "Author").unwrap();
    for (where_, reference, target, location) in [
        (&s, "B3:C4", None, Some("B3".to_string())),
        (&other, "H1", None, Some(format!("'{}'!B3", s))),
        (
            &other,
            "H2",
            Some("other.xlsx"),
            Some(format!("'{}'!B3", s)),
        ),
    ] {
        b.set_hyperlink(
            where_,
            &Hyperlink {
                reference: range(reference),
                target: target.map(str::to_string),
                location,
                display: None,
                tooltip: Some("keep tip".into()),
            },
        )
        .unwrap();
    }
    b.shift_cells(&s, range("B3:C3"), Axis::Rows, true).unwrap();
    let first = &b.workbook().sheets[0];
    assert_eq!(
        first
            .comment_at(jade_xlsx_reader::CellRef::parse("B4").unwrap())
            .unwrap()
            .text,
        "move me"
    );
    assert!(first
        .comment_at(jade_xlsx_reader::CellRef::parse("D3").unwrap())
        .is_some());
    assert_eq!(first.hyperlinks[0].reference, range("B4:C5"));
    assert_eq!(first.hyperlinks[0].location.as_deref(), Some("B4"));
    assert_eq!(first.data_validations[0].ranges, vec![range("B4:C5")]);
    assert_eq!(first.data_validations[0].formula1.as_deref(), Some("B4>0"));
    assert_eq!(
        b.workbook()
            .defined_names
            .iter()
            .find(|n| n.name == "LocalProbe")
            .unwrap()
            .formula,
        "B4"
    );
    assert_eq!(
        b.workbook().sheets[1].hyperlinks[0].location.as_deref(),
        Some(format!("'{}'!B4", s).as_str())
    );
    assert_eq!(
        b.workbook().sheets[1].hyperlinks[1].location.as_deref(),
        Some(format!("'{}'!B3", s).as_str())
    );
    b.shift_cells(&s, range("B4:C4"), Axis::Rows, false)
        .unwrap();
    assert!(b.workbook().sheets[0]
        .comment_at(jade_xlsx_reader::CellRef::parse("B4").unwrap())
        .is_none());
}

#[test]
fn tables_and_drawing_anchors_follow_only_their_column_strip() {
    let mut b = HeadlessWorkbook::open(&fixture("objects-workbook")).unwrap();
    let s = sheet(&b);
    b.shift_cells(&s, range("A3:C3"), Axis::Rows, true).unwrap();
    assert_eq!(b.workbook().sheets[0].tables[0].reference, range("A1:C6"));
    assert_eq!(b.cell(&s, "C6").unwrap().number, Some(749.));
    assert_eq!(b.workbook().sheets[0].shapes[0].from.row, 1);
    b.shift_cells(&s, range("D2:H2"), Axis::Rows, true).unwrap();
    assert_eq!(b.summary().sheets[0].shapes[0].anchor, "E3:G5");
    assert_eq!(b.workbook().sheets[0].tables[0].reference, range("A1:C6"));
    let bytes = b.bytes().to_vec();
    assert!(b.shift_cells(&s, range("B3"), Axis::Rows, true).is_err());
    assert_eq!(b.bytes(), bytes);
}

#[test]
fn table_column_deletion_breaks_removed_structured_references_and_keeps_retained_ids() {
    let mut b = simple();
    let s = sheet(&b);
    b.create_table(
        &s,
        range("A3:C4"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let removed = b.workbook().sheets[0].tables[0].columns[1].clone();
    b.set(
        &s,
        "H1",
        CellInput::Formula(format!("SUM(Sales[{removed}])")),
    )
    .unwrap();
    b.shift_cells(&s, range("B3:B4"), Axis::Columns, false)
        .unwrap();
    assert_eq!(b.workbook().sheets[0].tables[0].reference, range("A3:B4"));
    assert_eq!(
        b.cell(&s, "H1").unwrap().formula.as_deref(),
        Some("SUM(#REF!)")
    );
    assert_eq!(
        b.workbook().sheets[0].tables[0]
            .column_properties
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    b.shift_cells(&s, range("A3:B4"), Axis::Rows, false)
        .unwrap();
    assert!(b.workbook().sheets[0].tables.is_empty());
    assert!(!part(&b, PART).contains("tablePart "));
}

#[test]
fn invalid_protected_merged_array_and_edge_operations_are_atomic() {
    let mut b = custom(
        "<row r=\"3\"><c r=\"B3\"><v>3</v></c></row>",
        "<mergeCells count=\"1\"><mergeCell ref=\"B3:C4\"/></mergeCells>",
    );
    let s = sheet(&b);
    let before = b.bytes().to_vec();
    assert!(b.shift_cells(&s, range("B3"), Axis::Rows, true).is_err());
    assert_eq!(b.bytes(), before);
    assert!(b.shift_cells(&s, range("C4:B3"), Axis::Rows, true).is_err());
    assert_eq!(b.bytes(), before);
    let mut b = HeadlessWorkbook::open(&fixture("protected-sheets-workbook")).unwrap();
    let before = b.bytes().to_vec();
    assert!(b
        .shift_cells("Locked", range("B1"), Axis::Rows, true)
        .is_err());
    assert_eq!(b.bytes(), before);
    let mut b = custom(
        "<row r=\"1048576\"><c r=\"B1048576\"><v>3</v></c></row>",
        "",
    );
    let s = sheet(&b);
    let before = b.bytes().to_vec();
    assert!(b.shift_cells(&s, range("B3"), Axis::Rows, true).is_err());
    assert_eq!(b.bytes(), before);
    let mut b = custom(
        "<row r=\"3\"><c r=\"B3\"><f t=\"array\" ref=\"B3:C4\">SEQUENCE(2,2)</f><v>1</v></c></row>",
        "",
    );
    let s = sheet(&b);
    let before = b.bytes().to_vec();
    assert!(b.shift_cells(&s, range("B3"), Axis::Rows, true).is_err());
    assert_eq!(b.bytes(), before);
}

#[test]
fn split_rules_keep_relative_predicates_on_each_side_of_a_cell_shift() {
    let mut b=custom("<row r=\"3\"><c r=\"B3\"><v>3</v></c></row>","<conditionalFormatting sqref=\"B2:B5\"><cfRule type=\"expression\" priority=\"2\"><formula>E2&gt;0</formula></cfRule></conditionalFormatting><dataValidations count=\"1\"><dataValidation type=\"custom\" sqref=\"B2:B5\"><formula1><![CDATA[E2>0]]></formula1></dataValidation></dataValidations>");
    let s = sheet(&b);
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    let rules = &b.workbook().sheets[0].data_validations;
    assert_eq!(
        rules
            .iter()
            .map(|r| (&r.ranges, r.formula1.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (&vec![range("B2")], Some("E2>0")),
            (&vec![range("B3")], Some("E3>0")),
            (&vec![range("B4:B6")], Some("E3>0"))
        ]
    );
    let xml = part(&b, PART);
    assert!(xml.contains("priority=\"1\""));
    assert!(xml.contains("priority=\"2\""));
    assert!(xml.contains("priority=\"3\""));
    b.undo().unwrap();
    b.shift_cells(&s, range("B3:B4"), Axis::Rows, false)
        .unwrap();
    let rules = &b.workbook().sheets[0].data_validations;
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].ranges, vec![range("B2")]);
    assert_eq!(rules[0].formula1.as_deref(), Some("E2>0"));
    assert_eq!(rules[1].ranges, vec![range("B3")]);
    assert_eq!(rules[1].formula1.as_deref(), Some("E5>0"));
}

#[test]
fn strict_aliases_and_foreign_lookalikes_keep_their_namespace_identity() {
    let b=custom("<row r=\"3\" u:keep=\"yes\"><c r=\"B3\"><v>3</v></c></row>","<u:conditionalFormatting sqref=\"B3\"><u:formula>B3</u:formula></u:conditionalFormatting><extLst><ext uri=\"keep\"><u:data/></ext></extLst>");
    let mut package = OpcPackage::open(b.bytes()).unwrap();
    let mut xml = part(&b, PART);
    for name in ["worksheet", "sheetData", "row", "c", "v", "extLst", "ext"] {
        xml = xml
            .replace(&format!("<{name} "), &format!("<q:{name} "))
            .replace(&format!("<{name}>"), &format!("<q:{name}>"))
            .replace(&format!("</{name}>"), &format!("</q:{name}>"));
    }
    xml = xml.replace(
        &format!("xmlns=\"{MAIN}\""),
        "xmlns:q=\"http://purl.oclc.org/ooxml/spreadsheetml/main\"",
    );
    package.replace(PART, xml.into_bytes());
    let mut b = HeadlessWorkbook::open(&package.write().unwrap()).unwrap();
    let s = sheet(&b);
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    assert_eq!(b.cell(&s, "B4").unwrap().number, Some(3.));
    let xml = part(&b, PART);
    assert!(xml.contains("<q:c"));
    assert!(xml.contains("u:keep=\"yes\""));
    assert!(xml.contains(
        "<u:conditionalFormatting sqref=\"B3\"><u:formula>B3</u:formula></u:conditionalFormatting>"
    ));
}

#[test]
fn complete_array_blocks_and_complete_merged_ranges_move_without_splitting() {
    let mut b = custom(
        "<row r=\"3\"><c r=\"B3\"><f t=\"array\" ref=\"B3:C4\">SEQUENCE(2,2)</f><v>1</v></c></row>",
        "",
    );
    let s = sheet(&b);
    b.shift_cells(&s, range("B3:C3"), Axis::Rows, true).unwrap();
    assert_eq!(
        b.workbook().sheets[0].array_formulas[0].range,
        range("B4:C5")
    );
    assert_eq!(b.cell(&s, "C5").unwrap().number, Some(4.));
    let mut b = custom(
        "<row r=\"3\"><c r=\"B3\" t=\"inlineStr\"><is><t>Merged</t></is></c></row>",
        "<mergeCells count=\"1\"><mergeCell ref=\"B3:C4\"/></mergeCells>",
    );
    let s = sheet(&b);
    b.shift_cells(&s, range("B3:C3"), Axis::Rows, true).unwrap();
    assert_eq!(b.workbook().sheets[0].merges, vec![range("B4:C5")]);
    assert_eq!(b.cell(&s, "B4").unwrap().display, "Merged");
    b.undo().unwrap();
    assert_eq!(b.workbook().sheets[0].merges, vec![range("B3:C4")]);
}

#[test]
fn creating_table_headers_uses_a_structural_shift_including_occupied_cells_below() {
    let mut b = simple();
    let s = sheet(&b);
    b.set(&s, "B5", CellInput::Number(99.)).unwrap();
    b.set(&s, "H1", CellInput::Formula("$B$3+$C$4".into()))
        .unwrap();
    b.set_note(&s, "B3", "move with data", "Author").unwrap();
    b.set_hyperlink(
        &s,
        &Hyperlink {
            reference: range("B3"),
            target: None,
            location: Some("C4".into()),
            display: None,
            tooltip: Some("keep".into()),
        },
    )
    .unwrap();
    let before = b.bytes().to_vec();
    b.create_table(
        &s,
        range("B3:C4"),
        &TableOptions {
            name: Some("NoHeaders".into()),
            has_headers: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(b.workbook().sheets[0].tables[0].reference, range("B3:C5"));
    assert_eq!(b.cell(&s, "B3").unwrap().display, "Column1");
    assert_eq!(b.cell(&s, "B4").unwrap().number, Some(3.));
    assert_eq!(b.cell(&s, "B6").unwrap().number, Some(99.));
    assert_eq!(
        b.cell(&s, "H1").unwrap().formula.as_deref(),
        Some("$B$4+$C$5")
    );
    assert_eq!(b.cell(&s, "H1").unwrap().number, Some(43.));
    assert_eq!(
        b.workbook().sheets[0].comments[0].reference,
        range("B4").start
    );
    assert_eq!(b.workbook().sheets[0].hyperlinks[0].reference, range("B4"));
    assert_eq!(
        b.workbook().sheets[0].hyperlinks[0].location.as_deref(),
        Some("C5")
    );
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
}

fn calculated_table() -> (HeadlessWorkbook, String) {
    let mut b = simple();
    let s = sheet(&b);
    b.set_cells(
        &s,
        &[
            ("H2".into(), CellInput::Text("Item".into())),
            ("I2".into(), CellInput::Text("Qty".into())),
            ("J2".into(), CellInput::Text("Price".into())),
            ("K2".into(), CellInput::Text("Calc".into())),
            ("H3".into(), CellInput::Text("A".into())),
            ("I3".into(), CellInput::Number(2.)),
            ("J3".into(), CellInput::Number(10.)),
            ("H4".into(), CellInput::Text("B".into())),
            ("I4".into(), CellInput::Number(3.)),
            ("J4".into(), CellInput::Number(20.)),
            ("H5".into(), CellInput::Text("C".into())),
            ("I5".into(), CellInput::Number(4.)),
            ("J5".into(), CellInput::Number(30.)),
            ("H6".into(), CellInput::Text("D".into())),
            ("I6".into(), CellInput::Number(5.)),
            ("J6".into(), CellInput::Number(40.)),
            ("N1".into(), CellInput::Number(7.)),
        ],
    )
    .unwrap();
    b.create_table(
        &s,
        range("H2:K6"),
        &TableOptions {
            name: Some("Sales".into()),
            ..Default::default()
        },
    )
    .unwrap();
    b.update_table(
        &s,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("I3*J3+$N$1".into()))],
            ..Default::default()
        },
    )
    .unwrap();
    (b, s)
}
fn calculated_formula(b: &HeadlessWorkbook) -> &str {
    b.workbook().sheets[0].tables[0].column_properties[3]
        .calculated_formula
        .as_deref()
        .unwrap()
}

#[test]
fn inserted_table_rows_fill_recorded_formulas_without_overwriting_existing_exceptions() {
    let (mut b, s) = calculated_table();
    b.set(&s, "K4", CellInput::Number(999.)).unwrap();
    b.set(&s, "K5", CellInput::Blank).unwrap();
    let before = b.bytes().to_vec();
    b.shift_cells(&s, range("H4:K4"), Axis::Rows, true).unwrap();
    assert_eq!(
        b.cell(&s, "K4").unwrap().formula.as_deref(),
        Some("I4*J4+$N$1")
    );
    assert_eq!(b.cell(&s, "K4").unwrap().number, Some(7.));
    assert_eq!(b.cell(&s, "K5").unwrap().number, Some(999.));
    assert!(b.cell(&s, "K6").unwrap().formula.is_none());
    assert_eq!(b.cell(&s, "K6").unwrap().display, "");
    assert_eq!(b.cell(&s, "K7").unwrap().number, Some(207.));
    assert_eq!(calculated_formula(&b), "I3*J3+$N$1");
    b.undo().unwrap();
    assert_eq!(b.bytes(), before);
    b.redo().unwrap();
    b.set_cells(
        &s,
        &[
            ("I8".into(), CellInput::Number(5.)),
            ("J8".into(), CellInput::Number(6.)),
        ],
    )
    .unwrap();
    assert_eq!(b.cell(&s, "K8").unwrap().number, Some(37.));
}

#[test]
fn first_data_row_insertion_and_deletion_rebase_the_template_for_future_rows() {
    let (mut b, s) = calculated_table();
    b.shift_cells(&s, range("H3:K4"), Axis::Rows, true).unwrap();
    assert_eq!(calculated_formula(&b), "I3*J3+$N$1");
    assert_eq!(
        b.cell(&s, "K3").unwrap().formula.as_deref(),
        Some("I3*J3+$N$1")
    );
    assert_eq!(
        b.cell(&s, "K4").unwrap().formula.as_deref(),
        Some("I4*J4+$N$1")
    );
    assert_eq!(b.cell(&s, "K5").unwrap().number, Some(27.));
    let (mut b, s) = calculated_table();
    b.shift_cells(&s, range("H3:K3"), Axis::Rows, false)
        .unwrap();
    assert_eq!(calculated_formula(&b), "I3*J3+$N$1");
    assert_eq!(b.cell(&s, "K3").unwrap().number, Some(67.));
    b.set_cells(
        &s,
        &[
            ("I6".into(), CellInput::Number(5.)),
            ("J6".into(), CellInput::Number(6.)),
        ],
    )
    .unwrap();
    assert_eq!(b.cell(&s, "K6").unwrap().number, Some(37.));
    let reopened = HeadlessWorkbook::open(&b.save_bytes().unwrap()).unwrap();
    assert_eq!(calculated_formula(&reopened), "I3*J3+$N$1");
}

#[test]
fn whole_row_insertion_and_deleting_all_table_data_keep_a_calculated_data_row() {
    let (mut b, s) = calculated_table();
    b.insert_rows(&s, 3, 1).unwrap();
    assert_eq!(calculated_formula(&b), "I3*J3+$N$1");
    assert_eq!(b.cell(&s, "K3").unwrap().number, Some(7.));
    let (mut b, s) = calculated_table();
    b.shift_cells(&s, range("H3:K6"), Axis::Rows, false)
        .unwrap();
    assert_eq!(b.workbook().sheets[0].tables[0].reference, range("H2:K3"));
    assert_eq!(calculated_formula(&b), "I3*J3+$N$1");
    assert_eq!(b.cell(&s, "K3").unwrap().number, Some(7.));
}

#[test]
fn an_unaffected_sheet_keeps_its_original_xml_bytes() {
    let b = simple();
    let mut p = OpcPackage::open(b.bytes()).unwrap();
    let second = &b.workbook().sheets[1].part_name;
    let xml=format!("<worksheet xmlns='{MAIN}'><sheetData>\n<row r='2' customHeight='1' ht='23'><c r='F2' t='inlineStr'><is><t>untouched</t></is></c></row>\n</sheetData></worksheet>");
    p.replace(second, xml.as_bytes().to_vec());
    let second = second.clone();
    let mut b = HeadlessWorkbook::open(&p.write().unwrap()).unwrap();
    let s = sheet(&b);
    b.shift_cells(&s, range("B3"), Axis::Rows, true).unwrap();
    assert_eq!(part(&b, &second), xml);
}

#[test]
fn template_rebasing_preserves_structural_absolute_references_and_stable_column_ids() {
    let (mut b, s) = calculated_table();
    b.update_table(
        &s,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("I3+$I$3+$N$1".into()))],
            ..Default::default()
        },
    )
    .unwrap();
    b.shift_cells(&s, range("H3:K4"), Axis::Rows, true).unwrap();
    assert_eq!(calculated_formula(&b), "I3+$I$5+$N$1");
    assert_eq!(b.cell(&s, "K3").unwrap().number, Some(9.));
    assert_eq!(b.cell(&s, "K5").unwrap().number, Some(11.));
    let (mut b, s) = calculated_table();
    b.shift_cells(&s, range("H2:H6"), Axis::Columns, false)
        .unwrap();
    let table = &b.workbook().sheets[0].tables[0];
    assert_eq!(
        table
            .column_properties
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert_eq!(
        table.column_properties[2].calculated_formula.as_deref(),
        Some("H3*I3+$N$1")
    );
    b.set_cells(
        &s,
        &[
            ("H7".into(), CellInput::Number(2.)),
            ("I7".into(), CellInput::Number(5.)),
        ],
    )
    .unwrap();
    assert_eq!(b.cell(&s, "J7").unwrap().number, Some(17.));
}

#[test]
fn replacing_a_deleted_table_header_rebases_the_next_data_row() {
    let (mut b, s) = calculated_table();
    b.shift_cells(&s, range("H2:K2"), Axis::Rows, false)
        .unwrap();
    assert_eq!(b.cell(&s, "K2").unwrap().display, "Calc");
    assert_eq!(calculated_formula(&b), "I3*J3+$N$1");
    assert_eq!(b.cell(&s, "K3").unwrap().number, Some(67.));
    b.set_cells(
        &s,
        &[
            ("I6".into(), CellInput::Number(2.)),
            ("J6".into(), CellInput::Number(5.)),
        ],
    )
    .unwrap();
    assert_eq!(b.cell(&s, "K6").unwrap().number, Some(17.));
}

#[test]
fn structured_column_names_containing_cell_like_text_are_not_translated_as_addresses() {
    let (mut b, s) = calculated_table();
    b.set(&s, "I2", CellInput::Text("Qty A1 tax".into()))
        .unwrap();
    b.update_table(
        &s,
        "Sales",
        &TableUpdate {
            calculated_columns: vec![(3, Some("[@[Qty A1 tax]]*[@Price]+$N$1".into()))],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(b.cell(&s, "K4").unwrap().number, Some(67.));
    b.shift_cells(&s, range("H3:K3"), Axis::Rows, true).unwrap();
    assert_eq!(calculated_formula(&b), "[@[Qty A1 tax]]*[@Price]+$N$1");
    assert_eq!(b.cell(&s, "K3").unwrap().number, Some(7.));
    assert_eq!(b.cell(&s, "K5").unwrap().number, Some(67.));
}

#[test]
fn calculated_templates_on_other_sheets_follow_cross_sheet_dependencies() {
    let (mut b, s) = calculated_table();
    let other = b.workbook().sheets[1].name.clone();
    b.set_cells(
        &other,
        &[
            ("H2".into(), CellInput::Text("Seed".into())),
            ("I2".into(), CellInput::Text("Remote".into())),
            ("H3".into(), CellInput::Number(1.)),
            ("H4".into(), CellInput::Number(2.)),
        ],
    )
    .unwrap();
    b.create_table(
        &other,
        range("H2:I4"),
        &TableOptions {
            name: Some("Remote".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let quoted = s.replace('\'', "''");
    b.update_table(
        &other,
        "Remote",
        &TableUpdate {
            calculated_columns: vec![(1, Some(format!("'{quoted}'!I3+$H3")))],
            ..Default::default()
        },
    )
    .unwrap();
    b.shift_cells(&s, range("H3:K3"), Axis::Rows, true).unwrap();
    assert_eq!(
        b.workbook().sheets[1].tables[0].column_properties[1]
            .calculated_formula
            .as_deref(),
        Some(format!("'{quoted}'!I4+$H3").as_str())
    );
    assert_eq!(b.cell(&other, "I3").unwrap().number, Some(3.));
    b.set(&other, "H5", CellInput::Number(3.)).unwrap();
    assert_eq!(b.cell(&other, "I5").unwrap().number, Some(7.));
}
