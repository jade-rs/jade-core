//! Adding, renaming, deleting and moving sheets from a caller with no window.
//!
//! Each is a patch of the package — `workbook.xml`, its relationships,
//! `[Content_Types].xml` and the titles in `docProps/app.xml` — so the parts
//! jade does not read come back as they were, and each is one undo step.

use jade_opc::OpcPackage;
use jade_xlsx_headless_api::{CellInput, CellKind, WorkbookSummary};
use jade_xlsx_headless_editor::{HeadlessError, HeadlessWorkbook};
use jade_xlsx_writer::WriteError;
use proptest::prelude::*;
use std::path::PathBuf;

const SALES: &str = "売上サマリ";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn read_fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The two-sheet sample, with formulas on `Notes` that read `売上サマリ`,
/// a defined name of each scope, a data validation list, and the
/// `docProps/app.xml` titles Excel writes.
fn fixture() -> Vec<u8> {
    let mut package = OpcPackage::open(&read_fixture("sample-workbook.xlsx")).unwrap();
    package.replace(
        "xl/workbook.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="{MAIN}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView activeTab="1"/></bookViews><sheets><sheet name="売上サマリ" sheetId="1" r:id="rId1"/><sheet name="Notes" sheetId="2" r:id="rId4"/></sheets><definedNames><definedName name="_xlnm.Print_Area" localSheetId="1">Notes!$A$1:$B$4</definedName><definedName name="Local" localSheetId="0">'売上サマリ'!$B$4</definedName><definedName name="Total">売上サマリ!$D$7</definedName></definedNames><calcPr calcId="191029"/></workbook>"#
        )
        .into_bytes(),
    );
    package.replace(
        "xl/worksheets/sheet2.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="{MAIN}"><sheetFormatPr defaultRowHeight="18.75"/><sheetData><row r="1"><c r="A1"><f>売上サマリ!D7*2</f><v>120050000</v></c><c r="B1" t="str"><f>"売上サマリ!A1"</f><v>売上サマリ!A1</v></c></row><row r="2"><c r="A2"><f>SUM('売上サマリ'!B4:B6)</f><v>28400000</v></c><c r="B2"><f>Total+1</f><v>60025001</v></c></row><row r="3"><c r="A3"><f>A1/2</f><v>60025000</v></c></row></sheetData><dataValidations count="1"><dataValidation type="list" sqref="C1"><formula1>売上サマリ!$A$4:$A$6</formula1></dataValidation></dataValidations></worksheet>"#
        )
        .into_bytes(),
    );
    package.replace(
        "docProps/app.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Application>Microsoft Excel</Application><HeadingPairs><vt:vector size="4" baseType="variant"><vt:variant><vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>2</vt:i4></vt:variant><vt:variant><vt:lpstr>Named Ranges</vt:lpstr></vt:variant><vt:variant><vt:i4>2</vt:i4></vt:variant></vt:vector></HeadingPairs><TitlesOfParts><vt:vector size="4" baseType="lpstr"><vt:lpstr>売上サマリ</vt:lpstr><vt:lpstr>Notes</vt:lpstr><vt:lpstr>Notes!Print_Area</vt:lpstr><vt:lpstr>Total</vt:lpstr></vt:vector></TitlesOfParts><AppVersion>16.0300</AppVersion></Properties>"#
            .as_bytes()
            .to_vec(),
    );
    let rels = package.part_str("_rels/.rels").unwrap().unwrap().replace(
        "</Relationships>",
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>"#,
    );
    package.replace("_rels/.rels", rels.into_bytes());
    let types = package.part_str("[Content_Types].xml").unwrap().unwrap().replace(
        "</Types>",
        r#"<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>"#,
    );
    package.replace("[Content_Types].xml", types.into_bytes());
    package.write().unwrap()
}

fn open() -> HeadlessWorkbook {
    HeadlessWorkbook::open_for_editing(&fixture()).expect("open")
}

#[test]
fn literal_xstring_sheet_names_survive_authoring_reopen_and_history() {
    let mut book = open();
    let added = "Data_x0041_";
    let renamed = "O'Brien & _x0042_";
    book.add_sheet(added, None).unwrap();
    book.set(added, "A1", CellInput::Number(42.)).unwrap();
    let before = book.bytes().to_vec();
    book.rename_sheet(added, renamed).unwrap();
    let after = book.bytes().to_vec();
    let reopened = jade_xlsx_reader::read_xlsx(&after).unwrap();
    assert_eq!(reopened.sheets.last().unwrap().name, renamed);
    let package = OpcPackage::open(&after).unwrap();
    assert!(package
        .required_part_str("xl/workbook.xml")
        .unwrap()
        .contains("_x005F_x0042_"));
    let mut reopened = HeadlessWorkbook::open_for_editing(&after).unwrap();
    reopened.set(renamed, "A2", CellInput::Number(7.)).unwrap();
    assert!(book.undo().unwrap());
    assert_eq!(book.bytes(), before);
    assert!(book.redo().unwrap());
    assert_eq!(book.bytes(), after);
    assert!(matches!(
        book.add_sheet(renamed, None),
        Err(HeadlessError::Write(WriteError::DuplicateSheetName(_)))
    ));
}

fn names(summary: &WorkbookSummary) -> Vec<String> {
    summary
        .sheets
        .iter()
        .map(|sheet| sheet.name.clone())
        .collect()
}

fn reopened(workbook: &HeadlessWorkbook) -> HeadlessWorkbook {
    HeadlessWorkbook::open(&workbook.export_bytes().expect("export")).expect("reopen")
}

fn part(bytes: &[u8], name: &str) -> Option<String> {
    OpcPackage::open(bytes).unwrap().part_str(name).unwrap()
}

/// The worksheet titles and every title `docProps/app.xml` lists.
fn titles(bytes: &[u8]) -> (u32, Vec<String>) {
    let app = part(bytes, "docProps/app.xml").expect("app.xml");
    let count = app
        .split("<vt:lpstr>Worksheets</vt:lpstr></vt:variant><vt:variant><vt:i4>")
        .nth(1)
        .and_then(|rest| rest.split('<').next())
        .and_then(|count| count.parse().ok())
        .expect("worksheet count");
    let parts = app.split("<TitlesOfParts>").nth(1).expect("titles");
    let titles = parts
        .split("</TitlesOfParts>")
        .next()
        .unwrap()
        .split("<vt:lpstr>")
        .skip(1)
        .map(|title| title.split("</vt:lpstr>").next().unwrap().to_string())
        .collect::<Vec<_>>();
    let size = format!("<vt:vector size=\"{}\" baseType=\"lpstr\">", titles.len());
    assert!(parts.contains(&size), "{parts}");
    (count, titles)
}

fn defined(summary: &WorkbookSummary, name: &str) -> Option<(Option<String>, String)> {
    summary
        .defined_names
        .iter()
        .find(|defined| defined.name == name)
        .map(|defined| (defined.sheet.clone(), defined.formula.clone()))
}

fn formula(workbook: &HeadlessWorkbook, sheet: &str, reference: &str) -> Option<String> {
    workbook.cell(sheet, reference).unwrap().formula
}

#[test]
fn adding_a_sheet_puts_it_after_the_one_named() {
    let mut workbook = open();
    let before = workbook.bytes().to_vec();
    assert_eq!(workbook.add_sheet("Q3", Some(SALES)).expect("add"), 1);
    let summary = workbook.summary();
    assert_eq!(names(&summary), [SALES, "Q3", "Notes"]);
    // The active tab and the sheet-scoped names stay with their sheets.
    assert_eq!(summary.active_sheet_index, Some(2));
    assert_eq!(
        defined(&summary, "_xlnm.Print_Area").unwrap().0.as_deref(),
        Some("Notes")
    );
    assert_eq!(
        defined(&summary, "Local").unwrap().0.as_deref(),
        Some(SALES)
    );
    // A fresh sheet: empty, and as tall by default as the workbook's.
    let added = &workbook.workbook().sheets[1];
    assert!(added.rows.is_empty());
    assert_eq!(added.default_row_height_points, Some(18.75));
    workbook
        .set("Q3", "A1", CellInput::Formula("Notes!A2+1".into()))
        .expect("set");
    assert_eq!(workbook.cell("Q3", "A1").unwrap().display, "28400001");

    let bytes = workbook.export_bytes().unwrap();
    assert_eq!(
        titles(&bytes),
        (
            3,
            vec![
                SALES.to_string(),
                "Q3".into(),
                "Notes".into(),
                "Notes!Print_Area".into(),
                "Total".into()
            ]
        )
    );
    let part_name = workbook.workbook().sheets[1].part_name.clone();
    let types = part(&bytes, "[Content_Types].xml").unwrap();
    assert!(
        types.contains(&format!("PartName=\"/{part_name}\"")),
        "{types}"
    );
    let again = reopened(&workbook);
    assert_eq!(names(&again.summary()), [SALES, "Q3", "Notes"]);
    assert_eq!(
        again.cell("Q3", "A1").unwrap().formula.as_deref(),
        Some("Notes!A2+1")
    );

    // Two undo steps: the cell, then the sheet.
    assert!(workbook.undo().unwrap());
    assert!(workbook.undo().unwrap());
    assert!(!workbook.can_undo());
    assert_eq!(workbook.bytes(), before.as_slice());
}

#[test]
fn adding_without_a_neighbour_appends() {
    let mut workbook = open();
    assert_eq!(workbook.add_sheet("Last", None).expect("add"), 2);
    assert_eq!(names(&workbook.summary()), [SALES, "Notes", "Last"]);
    assert_eq!(workbook.summary().active_sheet_index, Some(1));
    assert_eq!(
        names(&reopened(&workbook).summary()),
        [SALES, "Notes", "Last"]
    );
    // Two sheets added in a row get parts of their own.
    workbook.add_sheet("Later", Some("Last")).expect("add");
    let parts: Vec<_> = workbook
        .workbook()
        .sheets
        .iter()
        .map(|sheet| sheet.part_name.clone())
        .collect();
    let mut unique = parts.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 4, "{parts:?}");
}

#[test]
fn bad_names_and_neighbours_are_refused_without_writing() {
    let mut workbook = open();
    let before = workbook.bytes().to_vec();
    for name in [
        "", "a/b", "[x]", "x:y", "?", "*", "a\\b", "'lead", "History", "notes", SALES,
    ] {
        let error = workbook.add_sheet(name, None).expect_err(name);
        assert!(error.is_invalid_input(), "{name}: {error}");
    }
    let error = workbook.add_sheet(&"x".repeat(32), None).expect_err("long");
    assert!(error.is_invalid_input(), "{error}");
    assert!(matches!(
        workbook.add_sheet("Fine", Some("Nope")),
        Err(HeadlessError::NoSuchSheet(_))
    ));
    assert!(!workbook.can_undo());
    assert_eq!(workbook.bytes(), before.as_slice());
    // 31 characters is the limit, not one fewer.
    workbook.add_sheet(&"あ".repeat(31), None).expect("31");
}

#[test]
fn renaming_a_sheet_rewrites_what_names_it() {
    let mut workbook = open();
    workbook.rename_sheet(SALES, "Sales 2026").expect("rename");
    let summary = workbook.summary();
    assert_eq!(names(&summary), ["Sales 2026", "Notes"]);
    assert_eq!(
        formula(&workbook, "Notes", "A1").as_deref(),
        Some("'Sales 2026'!D7*2")
    );
    assert_eq!(
        formula(&workbook, "Notes", "A2").as_deref(),
        Some("SUM('Sales 2026'!B4:B6)")
    );
    // Text in a string is not a reference; Excel leaves it alone too.
    assert_eq!(
        formula(&workbook, "Notes", "B1").as_deref(),
        Some("\"売上サマリ!A1\"")
    );
    assert_eq!(defined(&summary, "Total").unwrap().1, "'Sales 2026'!$D$7");
    assert_eq!(
        defined(&summary, "Local").unwrap(),
        (Some("Sales 2026".into()), "'Sales 2026'!$B$4".into())
    );
    let validation = workbook
        .cell("Notes", "C1")
        .unwrap()
        .data_validation
        .unwrap();
    assert_eq!(
        validation.formula1.as_deref(),
        Some("'Sales 2026'!$A$4:$A$6")
    );
    // Still calculating against the renamed sheet.
    assert_eq!(workbook.cell("Notes", "A1").unwrap().display, "120050000");
    assert_eq!(workbook.cell("Notes", "B2").unwrap().display, "60025001");

    let bytes = workbook.export_bytes().unwrap();
    assert_eq!(titles(&bytes).1[0], "Sales 2026");
    let again = reopened(&workbook);
    assert_eq!(names(&again.summary()), ["Sales 2026", "Notes"]);
    assert_eq!(
        formula(&again, "Notes", "A1").as_deref(),
        Some("'Sales 2026'!D7*2")
    );

    assert!(workbook.undo().unwrap());
    assert!(!workbook.can_undo());
    assert_eq!(names(&workbook.summary()), [SALES, "Notes"]);
    assert_eq!(
        formula(&workbook, "Notes", "A1").as_deref(),
        Some("売上サマリ!D7*2")
    );
}

#[test]
fn renaming_a_sheet_moves_its_titles_in_app_xml() {
    let mut workbook = open();
    workbook.rename_sheet("Notes", "Memo").expect("rename");
    assert_eq!(
        titles(workbook.bytes()),
        (
            2,
            vec![
                SALES.into(),
                "Memo".into(),
                "Memo!Print_Area".into(),
                "Total".into()
            ]
        )
    );
    // A change of case alone is a rename Excel allows.
    workbook.rename_sheet("Memo", "MEMO").expect("case");
    assert_eq!(names(&workbook.summary()), [SALES, "MEMO"]);
    for (old, new) in [("MEMO", SALES), ("MEMO", "sheet/1"), ("Nope", "Fine")] {
        let error = workbook.rename_sheet(old, new).expect_err(new);
        assert!(error.is_invalid_input(), "{error}");
    }
}

#[test]
fn renaming_a_sheet_follows_into_charts() {
    let mut workbook = HeadlessWorkbook::open(&read_fixture("charts-workbook.xlsx")).unwrap();
    workbook
        .rename_sheet("Data O'Brien", "Prices")
        .expect("rename");
    let chart = part(workbook.bytes(), "xl/charts/chart7.xml").unwrap();
    assert!(chart.contains("<c:f>Prices!$A$2:$A$4</c:f>"), "{chart}");
    assert!(!chart.contains("Brien"), "{chart}");
    // Charts on the other sheet that never named it are untouched.
    let untouched = part(workbook.bytes(), "xl/charts/chart1.xml").unwrap();
    let original = part(
        &read_fixture("charts-workbook.xlsx"),
        "xl/charts/chart1.xml",
    )
    .unwrap();
    assert_eq!(untouched, original);
    assert_eq!(names(&reopened(&workbook).summary()), ["Charts", "Prices"]);
}

#[test]
fn deleting_a_sheet_turns_references_to_it_into_ref_errors() {
    let mut workbook = open();
    let before = workbook.bytes().to_vec();
    workbook.delete_sheet(SALES).expect("delete");
    let summary = workbook.summary();
    assert_eq!(names(&summary), ["Notes"]);
    assert_eq!(
        formula(&workbook, "Notes", "A1").as_deref(),
        Some("#REF!D7*2")
    );
    assert_eq!(
        formula(&workbook, "Notes", "A2").as_deref(),
        Some("SUM(#REF!B4:B6)")
    );
    assert_eq!(workbook.cell("Notes", "A1").unwrap().kind, CellKind::Error);
    // Global names keep their entry with a #REF!; the deleted sheet's own
    // names go with it; the survivors' scopes follow the new positions.
    assert_eq!(defined(&summary, "Total").unwrap().1, "#REF!$D$7");
    assert!(defined(&summary, "Local").is_none());
    assert_eq!(
        defined(&summary, "_xlnm.Print_Area").unwrap().0.as_deref(),
        Some("Notes")
    );
    assert_eq!(summary.active_sheet_index, Some(0));

    let bytes = workbook.export_bytes().unwrap();
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").is_none());
    let types = part(&bytes, "[Content_Types].xml").unwrap();
    assert!(!types.contains("/xl/worksheets/sheet1.xml"), "{types}");
    let rels = part(&bytes, "xl/_rels/workbook.xml.rels").unwrap();
    assert!(!rels.contains("worksheets/sheet1.xml"), "{rels}");
    // Shared parts stay.
    assert!(part(&bytes, "xl/styles.xml").is_some());
    assert!(part(&bytes, "xl/sharedStrings.xml").is_some());
    assert_eq!(
        titles(&bytes),
        (
            1,
            vec!["Notes".into(), "Notes!Print_Area".into(), "Total".into()]
        )
    );
    let again = reopened(&workbook);
    assert_eq!(names(&again.summary()), ["Notes"]);
    assert_eq!(formula(&again, "Notes", "A1").as_deref(), Some("#REF!D7*2"));

    assert!(workbook.undo().unwrap());
    assert_eq!(workbook.bytes(), before.as_slice());
    assert_eq!(names(&workbook.summary()), [SALES, "Notes"]);
}

#[test]
fn references_left_broken_by_a_deletion_evaluate_to_ref_errors() {
    let mut workbook = open();
    workbook.delete_sheet(SALES).expect("delete");
    // Excel evaluates `#REF!B4:B6`, alone or inside a function, to #REF!,
    // and so does every formula that reads one.
    for reference in ["A1", "A2", "A3"] {
        let cell = workbook.cell("Notes", reference).unwrap();
        assert_eq!(cell.kind, CellKind::Error, "{reference}: {cell:?}");
        assert_eq!(cell.display, "#REF!", "{reference}");
    }
    // A defined name that became `#REF!$D$7` is an error-valued formula.
    // Its dependents must receive #REF!, rather than keep a stale number.
    let b2 = workbook.cell("Notes", "B2").unwrap();
    assert_eq!(b2.kind, CellKind::Error);
    assert_eq!(b2.display, "#REF!");
    assert_eq!(
        b2.formula_state,
        Some(jade_xlsx_headless_api::FormulaState::Cached)
    );
    assert!(!workbook.summary().full_calculation_on_load);
}

#[test]
fn deleting_the_active_sheet_moves_the_active_tab_and_its_names_go() {
    let mut workbook = open();
    workbook.delete_sheet("Notes").expect("delete");
    let summary = workbook.summary();
    assert_eq!(names(&summary), [SALES]);
    assert_eq!(summary.active_sheet_index, Some(0));
    assert!(defined(&summary, "_xlnm.Print_Area").is_none());
    assert_eq!(
        defined(&summary, "Local").unwrap().0.as_deref(),
        Some(SALES)
    );
    assert_eq!(
        titles(workbook.bytes()),
        (1, vec![SALES.into(), "Total".into()])
    );
}

#[test]
fn deleting_a_sheet_takes_its_tables_drawings_and_comments_with_it() {
    let original = read_fixture("objects-workbook.xlsx");
    let mut workbook = HeadlessWorkbook::open_for_editing(&original).unwrap();
    workbook.add_sheet("Other", None).expect("add");
    workbook
        .set_cells(
            "Other",
            &[
                (
                    "A1".into(),
                    CellInput::Formula("SUM(Table1[Amount])".into()),
                ),
                ("A2".into(), CellInput::Formula("Data!C2".into())),
            ],
        )
        .expect("set");
    assert_eq!(workbook.cell("Other", "A2").unwrap().display, "200");
    workbook.delete_sheet("Data").expect("delete");
    assert_eq!(names(&workbook.summary()), ["Other"]);
    assert_eq!(
        formula(&workbook, "Other", "A1").as_deref(),
        Some("SUM(#REF!)")
    );
    assert_eq!(
        formula(&workbook, "Other", "A2").as_deref(),
        Some("#REF!C2")
    );
    assert!(workbook.summary().defined_names.is_empty());

    let bytes = workbook.export_bytes().unwrap();
    let package = OpcPackage::open(&bytes).unwrap();
    for gone in [
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/_rels/sheet1.xml.rels",
        "xl/tables/table1.xml",
        "xl/drawings/drawing1.xml",
        "xl/drawings/_rels/drawing1.xml.rels",
        "xl/charts/chart1.xml",
        "xl/comments1.xml",
        "xl/drawings/vmlDrawing1.vml",
    ] {
        assert!(package.part(gone).is_none(), "{gone} is still there");
    }
    let types = package.part_str("[Content_Types].xml").unwrap().unwrap();
    for gone in ["table1.xml", "drawing1.xml", "chart1.xml", "comments1.xml"] {
        assert!(!types.contains(gone), "{gone}: {types}");
    }
    assert!(package.part("xl/styles.xml").is_some());
    let workbook_xml = package.part_str("xl/workbook.xml").unwrap().unwrap();
    assert!(!workbook_xml.contains("definedName"), "{workbook_xml}");
    assert_eq!(names(&reopened(&workbook).summary()), ["Other"]);
}

#[test]
fn the_last_visible_sheet_cannot_be_deleted() {
    let mut hidden = HeadlessWorkbook::open(&read_fixture("hidden-sheets-workbook.xlsx")).unwrap();
    let error = hidden.delete_sheet("Notes").expect_err("last visible");
    assert!(
        matches!(error, HeadlessError::Write(WriteError::LastVisibleSheet(_))),
        "{error}"
    );
    assert!(error.is_invalid_input());
    assert!(!hidden.can_undo());
    // The hidden one can go.
    hidden.delete_sheet(SALES).expect("delete hidden");
    assert_eq!(names(&hidden.summary()), ["Notes"]);
    assert!(hidden.delete_sheet("Notes").is_err());
    assert!(matches!(
        hidden.delete_sheet("Nope"),
        Err(HeadlessError::NoSuchSheet(_))
    ));
}

#[test]
fn moving_a_sheet_reorders_the_tabs_and_keeps_scopes() {
    let mut workbook = open();
    let before = workbook.bytes().to_vec();
    workbook.move_sheet("Notes", 0).expect("move");
    let summary = workbook.summary();
    assert_eq!(names(&summary), ["Notes", SALES]);
    assert_eq!(summary.active_sheet_index, Some(0));
    assert_eq!(
        defined(&summary, "_xlnm.Print_Area").unwrap().0.as_deref(),
        Some("Notes")
    );
    assert_eq!(
        defined(&summary, "Local").unwrap().0.as_deref(),
        Some(SALES)
    );
    // Nothing a formula says changes.
    assert_eq!(
        formula(&workbook, "Notes", "A1").as_deref(),
        Some("売上サマリ!D7*2")
    );
    assert_eq!(workbook.cell("Notes", "A1").unwrap().display, "120050000");
    assert_eq!(
        titles(workbook.bytes()),
        (
            2,
            vec![
                "Notes".into(),
                SALES.into(),
                "Notes!Print_Area".into(),
                "Total".into()
            ]
        )
    );
    assert_eq!(names(&reopened(&workbook).summary()), ["Notes", SALES]);

    // Moving to where it is writes nothing.
    workbook.move_sheet("Notes", 0).expect("stay");
    assert!(workbook.undo().unwrap());
    assert!(!workbook.can_undo());
    assert_eq!(workbook.bytes(), before.as_slice());

    let error = workbook.move_sheet("Notes", 2).expect_err("past the end");
    assert!(error.is_invalid_input(), "{error}");
}

#[derive(Debug, Clone)]
enum Operation {
    Add(String, Option<usize>),
    Rename(usize, String),
    Delete(usize),
    Move(usize, usize),
}

fn sheet_name() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[A-Za-z][A-Za-z0-9 ]{0,6}[A-Za-z0-9]",
        1 => "[売上メモ表]{1,3}",
        1 => "[A-Za-z]{1,3}'[A-Za-z]{1,3}",
        1 => Just("Notes".to_string()),
        1 => Just("bad/name".to_string()),
    ]
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        (sheet_name(), prop::option::of(0usize..6))
            .prop_map(|(name, after)| Operation::Add(name, after)),
        (0usize..6, sheet_name()).prop_map(|(at, name)| Operation::Rename(at, name)),
        (0usize..6).prop_map(Operation::Delete),
        (0usize..6, 0usize..6).prop_map(|(from, to)| Operation::Move(from, to)),
    ]
}

fn valid(name: &str, others: &[String]) -> bool {
    !name.contains('/')
        && name.to_uppercase() != "HISTORY"
        && !others
            .iter()
            .any(|other| other.to_uppercase() == name.to_uppercase())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// Any sequence of sheet operations leaves the workbook naming exactly
    /// the sheets a plain list says it should, in that order, through export
    /// and reopen; `Notes`' print area stays scoped to `Notes` whatever it
    /// is called; and `Notes!A2` names the sales sheet by its current name
    /// or, once it is gone, as `#REF!`. Refused operations change nothing.
    #[test]
    fn sheet_operations_agree_with_a_list(operations in prop::collection::vec(operation(), 1..8)) {
        let mut workbook = open();
        // (current name, identity) in tab order.
        let mut model: Vec<(String, &'static str)> = vec![(SALES.into(), "sales"), ("Notes".into(), "notes")];
        for operation in operations {
            let before = workbook.bytes().to_vec();
            let names_now: Vec<String> = model.iter().map(|(name, _)| name.clone()).collect();
            let accepted = match &operation {
                Operation::Add(name, after) => {
                    let after = after.and_then(|at| names_now.get(at).cloned());
                    let ok = valid(name, &names_now);
                    let result = workbook.add_sheet(name, after.as_deref());
                    if ok {
                        let at = after
                            .map(|after| names_now.iter().position(|name| *name == after).unwrap() + 1)
                            .unwrap_or(model.len());
                        prop_assert_eq!(result.expect("add"), at);
                        model.insert(at, (name.clone(), "added"));
                    } else {
                        prop_assert!(result.expect_err("refused").is_invalid_input());
                    }
                    ok
                }
                Operation::Rename(at, name) => {
                    let Some(old) = names_now.get(*at).cloned() else { continue };
                    let others: Vec<String> = names_now.iter().filter(|other| **other != old).cloned().collect();
                    let ok = valid(name, &others);
                    let result = workbook.rename_sheet(&old, name);
                    if ok {
                        result.expect("rename");
                        model[*at].0 = name.clone();
                    } else {
                        prop_assert!(result.expect_err("refused").is_invalid_input());
                    }
                    ok && old != *name
                }
                Operation::Delete(at) => {
                    let Some(name) = names_now.get(*at).cloned() else { continue };
                    let result = workbook.delete_sheet(&name);
                    if model.len() > 1 {
                        result.expect("delete");
                        model.remove(*at);
                        true
                    } else {
                        prop_assert!(result.expect_err("refused").is_invalid_input());
                        false
                    }
                }
                Operation::Move(from, to) => {
                    let Some(name) = names_now.get(*from).cloned() else { continue };
                    let result = workbook.move_sheet(&name, *to);
                    if *to < model.len() {
                        result.expect("move");
                        let entry = model.remove(*from);
                        model.insert(*to, entry);
                        from != to
                    } else {
                        prop_assert!(result.expect_err("refused").is_invalid_input());
                        false
                    }
                }
            };
            if !accepted {
                prop_assert_eq!(workbook.bytes(), before.as_slice());
            }
            let expected: Vec<String> = model.iter().map(|(name, _)| name.clone()).collect();
            prop_assert_eq!(names(&workbook.summary()), expected.clone());
            let again = reopened(&workbook);
            prop_assert_eq!(names(&again.summary()), expected.clone());

            let notes = model.iter().find(|(_, identity)| *identity == "notes").map(|(name, _)| name.clone());
            let sales = model.iter().find(|(_, identity)| *identity == "sales").map(|(name, _)| name.clone());
            let print_area = defined(&again.summary(), "_xlnm.Print_Area");
            match &notes {
                Some(notes) => {
                    prop_assert_eq!(print_area.map(|(sheet, _)| sheet), Some(Some(notes.clone())));
                    let a2 = formula(&again, notes, "A2").unwrap();
                    match &sales {
                        Some(sales) => prop_assert_eq!(
                            a2,
                            format!("SUM({}!B4:B6)", jade_xlsx_reader::quote_sheet_name(sales))
                        ),
                        None => prop_assert_eq!(a2, "SUM(#REF!B4:B6)".to_string()),
                    }
                }
                None => prop_assert!(print_area.is_none()),
            }
        }
    }
}
