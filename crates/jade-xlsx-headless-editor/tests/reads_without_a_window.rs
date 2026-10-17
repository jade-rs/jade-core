//! What a caller with no screen can learn about a workbook.

use jade_xlsx_headless_api::{CellInput, CellKind, FormulaState, SheetVisibility};
use jade_xlsx_headless_editor::{HeadlessWorkbook, RANGE_CELL_LIMIT};
use jade_xlsx_reader::CellRef;
use jade_xlsx_writer::{write_xlsx, CellEdit, NewValue, SheetEdits};
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn open(name: &str) -> HeadlessWorkbook {
    HeadlessWorkbook::open(&fixture(name)).expect("open")
}

fn open_with_inherited_currency_styles() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/worksheets/sheet1.xml";
    let xml = package
        .part_str(part)
        .expect("read sheet")
        .expect("sheet part");
    let xml = xml
        .replace(
            r#"<col min="2" max="4""#,
            r#"<col min="2" max="4" style="4""#,
        )
        .replace(r#"<c r="B4" s="4">"#, r#"<c r="B4">"#)
        .replace(r#"<row r="5">"#, r#"<row r="5" s="4" customFormat="1">"#)
        .replace(r#"<c r="C5" s="4">"#, r#"<c r="C5">"#);
    package.replace(part, xml.into_bytes());
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_shared_formula() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/worksheets/sheet1.xml";
    let xml = package
        .part_str(part)
        .expect("read sheet")
        .expect("sheet part");
    let rows = concat!(
        r#"<row r="20"><c r="A20"><v>10</v></c><c r="B20"><f t="shared" ref="B20:B21" si="7">A20*2</f><v>20</v></c></row>"#,
        r#"<row r="21"><c r="A21"><v>11</v></c><c r="B21"><f t="shared" si="7"/><v>22</v></c></row>"#
    );
    let xml = xml.replace("</sheetData>", &format!("{rows}</sheetData>"));
    package.replace(part, xml.into_bytes());
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_defined_names() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/workbook.xml";
    let xml = package
        .part_str(part)
        .expect("read workbook")
        .expect("workbook part");
    let names = concat!(
        "<definedNames>",
        r#"<definedName name="Revenue">'売上サマリ'!$B$4:$D$7</definedName>"#,
        r#"<definedName name="LocalRate" localSheetId="1">Notes!$A$1</definedName>"#,
        r#"<definedName name="_xlnm.Print_Area" localSheetId="0" hidden="1">'売上サマリ'!$A$1:$D$10</definedName>"#,
        "</definedNames>"
    );
    let xml = xml.replace("</workbook>", &format!("{names}</workbook>"));
    package.replace(part, xml.into_bytes());
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_frozen_panes() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/worksheets/sheet1.xml";
    let xml = package
        .part_str(part)
        .expect("read sheet")
        .expect("sheet part");
    let xml = xml.replace(
        r#"<sheetView tabSelected="1" workbookViewId="0"/>"#,
        r#"<sheetView tabSelected="1" workbookViewId="0"><pane xSplit="2" ySplit="3" topLeftCell="C4" activePane="bottomRight" state="frozen"/></sheetView>"#,
    );
    package.replace(part, xml.into_bytes());
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_hidden_sheet() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/workbook.xml";
    let xml = package
        .part_str(part)
        .expect("read workbook")
        .expect("workbook part");
    let hidden = r#"<sheet name="Notes" sheetId="2" state="hidden" r:id="rId4"/>"#;
    let xml = xml.replace(r#"<sheet name="Notes" sheetId="2" r:id="rId4"/>"#, hidden);
    assert!(xml.contains(hidden), "hidden sheet was not injected");
    package.replace(part, xml.into_bytes());
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_hyperlinks() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/worksheets/sheet1.xml";
    let xml = package
        .part_str(part)
        .expect("read sheet")
        .expect("sheet part");
    let links = concat!(
        r#"<hyperlinks><hyperlink ref="A4" r:id="rIdLink" tooltip="Open example"/>"#,
        r#"<hyperlink ref="B4:C4" location="Notes!A1" display="Jump"/></hyperlinks>"#
    );
    let xml = xml.replace("</worksheet>", &format!("{links}</worksheet>"));
    package.replace(part, xml.into_bytes());
    package.replace(
        "xl/worksheets/_rels/sheet1.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdLink" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/report?year=2026&amp;format=xlsx" TargetMode="External"/></Relationships>"#.to_vec(),
    );
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_comments() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let content_types = package
        .part_str("[Content_Types].xml")
        .expect("read content types")
        .expect("content types part")
        .replace(
            "</Types>",
            r#"<Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/></Types>"#,
        );
    package.replace("[Content_Types].xml", content_types.into_bytes());
    package.replace(
        "xl/worksheets/_rels/sheet1.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdComment" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/></Relationships>"#.to_vec(),
    );
    package.replace(
        "xl/comments1.xml",
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>Reviewer</author></authors><commentList><comment ref="B4" authorId="0"><text><r><t>Check </t></r><r><rPr><b/></rPr><t>total</t></r></text></comment></commentList></comments>"#.to_vec(),
    );
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

fn open_with_data_validations() -> HeadlessWorkbook {
    let bytes = fixture("sample-workbook.xlsx");
    let mut package = jade_xlsx_reader::OpcPackage::open(&bytes).expect("package");
    let part = "xl/worksheets/sheet1.xml";
    let xml = package
        .part_str(part)
        .expect("read sheet")
        .expect("sheet part");
    let validations = concat!(
        r#"<dataValidations count="1"><dataValidation type="decimal" operator="greaterThanOrEqual" allowBlank="1" showInputMessage="1" promptTitle="Minimum" prompt="Enter at least 0" sqref="B4:B6 D4">"#,
        r#"<formula1>0</formula1></dataValidation></dataValidations>"#
    );
    let xml = xml.replace("</worksheet>", &format!("{validations}</worksheet>"));
    package.replace(part, xml.into_bytes());
    HeadlessWorkbook::open(&package.write().expect("write fixture")).expect("open")
}

#[test]
fn a_summary_says_what_is_in_the_workbook() {
    let workbook = open("sample-workbook.xlsx");
    let summary = workbook.summary();

    assert_eq!(summary.date_system, 1900);
    assert_eq!(summary.active_sheet_index, None);
    assert!(!summary.full_calculation_on_load);
    let names: Vec<&str> = summary
        .sheets
        .iter()
        .map(|sheet| sheet.name.as_str())
        .collect();
    assert_eq!(names, ["売上サマリ", "Notes"]);

    let first = &summary.sheets[0];
    assert_eq!(first.index, 0);
    assert_eq!(first.used_range.as_deref(), Some("A1:D10"));
    assert_eq!(first.merges, ["A1:D1"]);
    assert!(first.show_grid_lines);
    assert!(!summary.sheets[1].show_grid_lines);
}

#[test]
fn a_summary_exposes_defined_names_with_their_sheet_scope() {
    let workbook = open_with_defined_names();
    let names = workbook.summary().defined_names;

    assert_eq!(names.len(), 3);
    assert_eq!(names[0].name, "Revenue");
    assert_eq!(names[0].formula, "'売上サマリ'!$B$4:$D$7");
    assert_eq!(names[0].sheet_index, None);
    assert_eq!(names[0].sheet, None);

    assert_eq!(names[1].name, "LocalRate");
    assert_eq!(names[1].sheet_index, Some(1));
    assert_eq!(names[1].sheet.as_deref(), Some("Notes"));

    assert_eq!(names[2].name, "_xlnm.Print_Area");
    assert!(names[2].hidden);
}

#[test]
fn a_sheet_summary_exposes_its_frozen_panes() {
    let workbook = open_with_frozen_panes();
    let pane = workbook.summary().sheets[0]
        .freeze_pane
        .clone()
        .expect("frozen pane");

    assert_eq!(pane.frozen_columns, 2);
    assert_eq!(pane.frozen_rows, 3);
    assert_eq!(pane.top_left_cell.as_deref(), Some("C4"));
    assert!(workbook.summary().sheets[1].freeze_pane.is_none());
}

#[test]
fn a_summary_distinguishes_hidden_sheets_and_a_write_preserves_the_state() {
    let mut workbook = open_with_hidden_sheet();

    assert_eq!(
        workbook.summary().sheets[0].visibility,
        SheetVisibility::Visible
    );
    assert_eq!(
        workbook.summary().sheets[1].visibility,
        SheetVisibility::Hidden
    );

    workbook
        .set("売上サマリ", "A20", CellInput::Text("追記".into()))
        .expect("set");
    assert_eq!(
        workbook.summary().sheets[1].visibility,
        SheetVisibility::Hidden
    );
}

#[test]
fn a_cell_reports_what_it_holds_and_what_it_shows() {
    let workbook = open("sample-workbook.xlsx");

    // A date is a number plus a format; only `display` can tell you it is a
    // date at all.
    let date = workbook.cell("売上サマリ", "B9").expect("B9");
    assert_eq!(date.kind, CellKind::Number);
    assert_eq!(date.number, Some(45000.0));
    assert_eq!(date.display, "2023/03/15");
    assert_eq!(date.number_format.as_deref(), Some("yyyy/mm/dd"));

    let text = workbook.cell("売上サマリ", "A4").expect("A4");
    assert_eq!(text.kind, CellKind::Text);
    assert_eq!(text.text.as_deref(), Some("横浜"));
    assert_eq!(text.display, "横浜");
}

#[test]
fn cells_expose_external_and_internal_hyperlinks_and_preserve_them_on_write() {
    let mut workbook = open_with_hyperlinks();

    let external = workbook.cell("売上サマリ", "A4").expect("A4").hyperlink;
    assert_eq!(
        external.as_ref().and_then(|link| link.target.as_deref()),
        Some("https://example.com/report?year=2026&format=xlsx")
    );
    assert_eq!(
        external.as_ref().and_then(|link| link.tooltip.as_deref()),
        Some("Open example")
    );

    let internal = workbook.cell("売上サマリ", "C4").expect("C4").hyperlink;
    assert_eq!(
        internal.as_ref().and_then(|link| link.location.as_deref()),
        Some("Notes!A1")
    );
    assert_eq!(
        internal.as_ref().and_then(|link| link.display.as_deref()),
        Some("Jump")
    );

    workbook
        .set("売上サマリ", "A20", CellInput::Text("追記".into()))
        .expect("set");
    assert_eq!(
        workbook
            .cell("売上サマリ", "A4")
            .expect("A4 after write")
            .hyperlink
            .and_then(|link| link.target),
        Some("https://example.com/report?year=2026&format=xlsx".into())
    );
}

#[test]
fn cells_expose_comments_and_preserve_them_on_write() {
    let mut workbook = open_with_comments();

    let comment = workbook.cell("売上サマリ", "B4").expect("B4").comment;
    assert_eq!(
        comment
            .as_ref()
            .and_then(|comment| comment.author.as_deref()),
        Some("Reviewer")
    );
    assert_eq!(
        comment.as_ref().map(|comment| comment.text.as_str()),
        Some("Check total")
    );

    workbook
        .set("売上サマリ", "B4", CellInput::Text("確認済み".into()))
        .expect("set");
    assert_eq!(
        workbook
            .cell("売上サマリ", "B4")
            .expect("B4 after write")
            .comment
            .map(|comment| comment.text),
        Some("Check total".into())
    );
}

#[test]
fn cells_expose_data_validation_and_preserve_it_on_write() {
    let mut workbook = open_with_data_validations();

    let validation = workbook
        .cell("売上サマリ", "B5")
        .expect("B5")
        .data_validation
        .expect("validation");
    assert_eq!(validation.validation_type.as_deref(), Some("decimal"));
    assert_eq!(validation.operator.as_deref(), Some("greaterThanOrEqual"));
    assert_eq!(validation.formula1.as_deref(), Some("0"));
    assert_eq!(validation.prompt_title.as_deref(), Some("Minimum"));
    assert_eq!(validation.prompt.as_deref(), Some("Enter at least 0"));
    assert_eq!(validation.ranges, ["B4:B6", "D4"]);
    assert!(workbook
        .cell("売上サマリ", "C5")
        .expect("C5")
        .data_validation
        .is_none());

    workbook
        .set("売上サマリ", "B5", CellInput::Number(42.0))
        .expect("set");
    assert_eq!(
        workbook
            .cell("売上サマリ", "B5")
            .expect("B5 after write")
            .data_validation
            .and_then(|validation| validation.formula1),
        Some("0".into())
    );
}

#[test]
fn headless_cells_report_formats_inherited_from_columns_and_rows() {
    let workbook = open_with_inherited_currency_styles();

    let column = workbook.cell("売上サマリ", "B4").expect("B4");
    assert_eq!(column.number_format.as_deref(), Some("\"¥\"#,##0"));
    assert_eq!(column.display, "¥12,850,000");

    let row = workbook.cell("売上サマリ", "C5").expect("C5");
    assert_eq!(row.number_format.as_deref(), Some("\"¥\"#,##0"));
    assert_eq!(row.display, "¥8,990,000");
}

#[test]
fn a_headless_write_keeps_an_inherited_number_format() {
    let mut workbook = open_with_inherited_currency_styles();

    // B20 does not exist in the source. Its column supplies the currency style
    // both before and after the writer inserts the cell.
    assert_eq!(
        workbook
            .cell("売上サマリ", "B20")
            .expect("B20 before")
            .number_format
            .as_deref(),
        Some("\"¥\"#,##0")
    );
    workbook
        .set("売上サマリ", "B20", CellInput::Number(2500.0))
        .expect("set");

    let written = workbook.cell("売上サマリ", "B20").expect("B20 after");
    assert_eq!(written.display, "¥2,500");
    assert_eq!(written.number_format.as_deref(), Some("\"¥\"#,##0"));
}

#[test]
fn a_formula_comes_with_the_value_the_file_cached() {
    let workbook = open("sample-workbook.xlsx");
    let cell = workbook.cell("売上サマリ", "D4").expect("D4");

    assert_eq!(cell.formula.as_deref(), Some("SUM(B4:C4)"));
    assert_eq!(cell.number, Some(28_080_000.0));
    assert_eq!(cell.display, "¥28,080,000");
}

#[test]
fn headless_cells_expose_the_expanded_shared_formula() {
    let workbook = open_with_shared_formula();
    let cell = workbook.cell("売上サマリ", "B21").expect("B21");

    assert_eq!(cell.formula.as_deref(), Some("A21*2"));
    assert_eq!(cell.number, Some(22.0));
}

#[test]
fn a_cell_the_file_does_not_write_is_empty_rather_than_missing() {
    let workbook = open("sample-workbook.xlsx");
    let cell = workbook.cell("売上サマリ", "H20").expect("H20");

    assert_eq!(cell.kind, CellKind::Empty);
    assert_eq!(cell.display, "");
    assert_eq!(cell.reference, "H20");
    assert!(cell.formula.is_none());
}

#[test]
fn a_range_comes_back_as_a_grid() {
    let workbook = open("sample-workbook.xlsx");
    let range = workbook.range("売上サマリ", "A3:D4").expect("range");

    assert_eq!(range.reference, "A3:D4");
    assert_eq!(range.rows.len(), 2);
    assert_eq!(range.rows[0].len(), 4);
    let headings: Vec<&str> = range.rows[0]
        .iter()
        .map(|cell| cell.display.as_str())
        .collect();
    assert_eq!(headings, ["支店", "上期", "下期", "合計"]);
}

#[test]
fn a_range_given_backwards_still_reads_forwards() {
    let workbook = open("sample-workbook.xlsx");
    let range = workbook.range("売上サマリ", "D4:A3").expect("range");
    assert_eq!(range.reference, "A3:D4");
    assert_eq!(range.rows[0][0].reference, "A3");
}

#[test]
fn a_range_that_would_materialise_a_spreadsheet_is_refused() {
    let workbook = open("sample-workbook.xlsx");
    // The whole sheet in Excel's coordinates: seventeen billion cells.
    let error = workbook
        .range("売上サマリ", "A1:XFD1048576")
        .expect_err("refused");
    assert!(error.to_string().contains("more than the limit"));
}

#[test]
fn a_sheet_reads_as_a_grid_of_display_text() {
    let workbook = open("sample-workbook.xlsx");
    let text = workbook.sheet_text("売上サマリ").expect("text");

    assert_eq!(text.reference.as_deref(), Some("A1:D10"));
    assert_eq!(text.rows.len(), 10);
    assert_eq!(text.rows[3][0], "横浜");
    assert_eq!(text.rows[3][3], "¥28,080,000");
    // Row 2 is empty in the file and still present, so the grid can be indexed.
    assert_eq!(text.rows[1], vec!["", "", "", ""]);
}

#[test]
fn a_sheet_can_be_named_or_numbered() {
    let workbook = open("sample-workbook.xlsx");
    assert_eq!(workbook.sheet_index("Notes").expect("by name"), 1);
    assert_eq!(workbook.sheet_index("1").expect("by index"), 1);
    assert!(workbook.sheet_index("nope").is_err());
    assert!(workbook.sheet_index("9").is_err());
}

#[test]
fn find_searches_what_a_reader_would_see() {
    let workbook = open("sample-workbook.xlsx");

    // The date is stored as 45000; searching for what it shows finds it.
    let hits = workbook.find("2023/03", false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].reference, "B9");
    assert_eq!(hits[0].sheet, "売上サマリ");

    // 備考 is in both sheets, and the hits say which is which.
    let both = workbook.find("備考", false);
    assert_eq!(both.len(), 2);
    assert_eq!(both[0].sheet_index, 0);
    assert_eq!(both[1].sheet_index, 1);

    // An empty needle would otherwise match every cell in the workbook.
    assert!(workbook.find("", false).is_empty());
}

#[test]
fn find_also_searches_formula_source() {
    let workbook = open("sample-workbook.xlsx");

    let hits = workbook.find("sum(b4:c4)", false);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].reference, "D4");
    assert_eq!(hits[0].formula.as_deref(), Some("SUM(B4:C4)"));
}

#[test]
fn find_can_be_told_to_mind_the_case() {
    let workbook = open("large-workbook.xlsx");
    assert_eq!(workbook.find("ROW", false).len(), 1);
    assert!(workbook.find("ROW", true).is_empty());
    assert_eq!(workbook.find("row", true).len(), 1);
}

#[test]
fn a_tall_sheet_reads_without_laying_anything_out() {
    // No font is measured and no geometry is computed, which is what makes a
    // headless read cheap on a sheet this size.
    let workbook = open("large-workbook.xlsx");
    let text = workbook.sheet_text("明細").expect("text");
    assert_eq!(text.rows.len(), 5_001);
    assert_eq!(text.rows[1][1], "¥1,000");
    assert_eq!(text.rows[5_000][2], "行 5000");
}

#[test]
fn a_far_away_cell_does_not_materialise_every_cell_before_it() {
    let workbook = open("sparse-offset-workbook.xlsx");
    let summary = workbook.summary();
    assert_eq!(
        summary.sheets[0].used_range.as_deref(),
        Some("XFD1048576:XFD1048576")
    );

    let text = workbook.sheet_text("Sparse").expect("one-cell used range");
    assert_eq!(text.reference.as_deref(), Some("XFD1048576:XFD1048576"));
    assert_eq!(text.rows, vec![vec!["far away"]]);
}

#[test]
fn a_genuinely_large_sparse_rectangle_is_still_refused() {
    let original = fixture("sample-workbook.xlsx");
    let written = write_xlsx(
        &original,
        &[SheetEdits {
            part_name: "xl/worksheets/sheet1.xml".into(),
            cells: vec![CellEdit {
                style_index: None,
                reference: CellRef::parse("XFD62").expect("reference"),
                value: NewValue::Text("far away".into()),
            }],
        }],
    )
    .expect("write sparse workbook");
    let workbook = HeadlessWorkbook::open(&written).expect("open");

    let error = workbook
        .sheet_text("売上サマリ")
        .expect_err("the true used rectangle exceeds the safety limit");
    assert!(error.to_string().contains(&RANGE_CELL_LIMIT.to_string()));
}

#[test]
fn a_set_is_visible_to_the_next_read() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "B4", CellInput::Number(1234.0))
        .expect("set");

    let cell = workbook.cell("売上サマリ", "B4").expect("B4");
    assert_eq!(cell.number, Some(1234.0));
    // The cell's own format still applies to the new value.
    assert_eq!(cell.display, "¥1,234");
}

#[test]
fn a_formula_can_be_set_explicitly_without_becoming_text() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set(
            "売上サマリ",
            "A20",
            CellInput::Formula("=SUM(B4:C4)".into()),
        )
        .expect("set formula");

    let cell = workbook.cell("売上サマリ", "A20").expect("A20");
    assert_eq!(cell.formula.as_deref(), Some("SUM(B4:C4)"));
    // The session recalculates, so the formula has its result right away.
    assert_eq!(cell.kind, CellKind::Number);
    assert_eq!(cell.formula_state, Some(FormulaState::Cached));
}

#[test]
fn the_bytes_a_set_produced_open_as_a_workbook() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "A20", CellInput::Text("追記".into()))
        .expect("set");

    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    assert_eq!(
        reopened.cell("売上サマリ", "A20").expect("A20").display,
        "追記"
    );
}

#[test]
fn several_cells_can_be_set_at_once() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set_cells(
            "売上サマリ",
            &[
                ("B4".to_string(), CellInput::Number(1.0)),
                ("C4".to_string(), CellInput::Number(2.0)),
            ],
        )
        .expect("set");

    assert_eq!(
        workbook.cell("売上サマリ", "B4").expect("B4").number,
        Some(1.0)
    );
    assert_eq!(
        workbook.cell("売上サマリ", "C4").expect("C4").number,
        Some(2.0)
    );
}

#[test]
fn formula_reads_report_whether_the_cached_result_is_current() {
    let mut workbook = open("sample-workbook.xlsx");

    let cached = workbook.cell("売上サマリ", "D4").expect("D4");
    assert_eq!(cached.formula_state, Some(FormulaState::Cached));

    // A set recalculates its dependents, so the state stays `cached` and the
    // number moves with the edit; `stale` is what remains for the formulas
    // the engine had to skip.
    workbook
        .set("売上サマリ", "B4", CellInput::Number(1.0))
        .expect("change a formula input");
    let current = workbook.cell("売上サマリ", "D4").expect("D4");
    assert_eq!(current.formula_state, Some(FormulaState::Cached));
    let c4 = workbook
        .cell("売上サマリ", "C4")
        .expect("C4")
        .number
        .expect("a number");
    assert_eq!(current.number, Some(1.0 + c4));
}

#[test]
fn reopening_written_bytes_keeps_the_recalculated_caches() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "B4", CellInput::Number(1.0))
        .expect("change a formula input");

    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen written workbook");
    // Every formula was recalculated, so the file no longer asks the next
    // spreadsheet to recalculate before showing numbers.
    assert!(!reopened.summary().full_calculation_on_load);
    assert_eq!(
        reopened.cell("売上サマリ", "D4").expect("D4").formula_state,
        Some(FormulaState::Cached)
    );
}

#[test]
fn several_sheets_can_be_set_atomically() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set_workbook_cells(&[
            (
                "売上サマリ".to_string(),
                "B4".to_string(),
                CellInput::Number(10.0),
            ),
            (
                "Notes".to_string(),
                "A20".to_string(),
                CellInput::Text("updated".into()),
            ),
        ])
        .expect("multi-sheet set");

    assert_eq!(
        workbook.cell("売上サマリ", "B4").expect("B4").number,
        Some(10.0)
    );
    assert_eq!(
        workbook.cell("Notes", "A20").expect("A20").text.as_deref(),
        Some("updated")
    );
}

#[test]
fn a_bad_multi_sheet_edit_changes_nothing() {
    let mut workbook = open("sample-workbook.xlsx");
    let before = workbook.bytes().to_vec();

    workbook
        .set_workbook_cells(&[
            (
                "売上サマリ".to_string(),
                "B4".to_string(),
                CellInput::Number(10.0),
            ),
            (
                "missing".to_string(),
                "A1".to_string(),
                CellInput::Text("never written".into()),
            ),
        ])
        .expect_err("bad sheet rejects the whole batch");
    assert_eq!(workbook.bytes(), before);
    let formula = workbook.cell("売上サマリ", "D4").expect("D4");
    assert_eq!(formula.formula_state, Some(FormulaState::Cached));
}

#[test]
fn duplicate_cells_through_sheet_aliases_are_rejected() {
    let mut workbook = open("sample-workbook.xlsx");
    let before = workbook.bytes().to_vec();

    workbook
        .set_workbook_cells(&[
            (
                "売上サマリ".to_string(),
                "B4".to_string(),
                CellInput::Number(1.0),
            ),
            ("0".to_string(), "B4".to_string(), CellInput::Number(2.0)),
        ])
        .expect_err("name and index resolve to the same cell");
    assert_eq!(workbook.bytes(), before);
}

#[test]
fn duplicate_cells_in_one_batch_are_rejected_without_changes() {
    let mut workbook = open("sample-workbook.xlsx");
    let before = workbook.cell("売上サマリ", "B4").expect("before");

    let error = workbook
        .set_cells(
            "売上サマリ",
            &[
                ("B4".to_string(), CellInput::Number(1.0)),
                ("B4".to_string(), CellInput::Number(2.0)),
            ],
        )
        .expect_err("duplicate reference");
    assert!(error.to_string().contains("duplicate cell reference"));

    let after = workbook.cell("売上サマリ", "B4").expect("after");
    assert_eq!(after, before);
}

#[test]
fn a_typed_value_is_read_the_way_a_spreadsheet_reads_it() {
    assert_eq!(CellInput::parse("123.5"), CellInput::Number(123.5));
    assert_eq!(CellInput::parse("TRUE"), CellInput::Bool(true));
    assert_eq!(CellInput::parse("false"), CellInput::Bool(false));
    assert_eq!(CellInput::parse(""), CellInput::Blank);
    assert_eq!(CellInput::parse("横浜"), CellInput::Text("横浜".into()));
    // A caller who means the text `007` has to say so.
    assert_eq!(CellInput::parse("007"), CellInput::Number(7.0));
    // Infinities and NaN are not numbers a sheet can hold.
    assert_eq!(CellInput::parse("inf"), CellInput::Text("inf".into()));
}

#[test]
fn setting_a_cell_in_a_sheet_that_is_not_there_is_an_error() {
    let mut workbook = open("sample-workbook.xlsx");
    assert!(workbook.set("nope", "A1", CellInput::Number(1.0)).is_err());
    assert!(workbook
        .set("売上サマリ", "not-a-reference", CellInput::Number(1.0))
        .is_err());
}

#[test]
fn setting_outside_xlsx_bounds_is_rejected_without_changing_the_workbook() {
    let mut workbook = open("sample-workbook.xlsx");
    let before = workbook.bytes().to_vec();

    assert!(workbook
        .set("売上サマリ", "XFE1", CellInput::Number(1.0))
        .is_err());
    assert!(workbook
        .set("売上サマリ", "A1048577", CellInput::Number(1.0))
        .is_err());
    assert_eq!(workbook.bytes(), before);
}

#[test]
fn text_xml_cannot_carry_round_trips_through_the_x_escape() {
    // XML 1.0 forbids a NUL, but SpreadsheetML's `_xHHHH_` escape carries it —
    // the same way Excel stores control characters someone pasted into a cell.
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "A20", CellInput::Text("before\0after".into()))
        .expect("set");
    assert_eq!(
        workbook
            .cell("売上サマリ", "A20")
            .expect("A20")
            .text
            .as_deref(),
        Some("before\0after")
    );
}

#[test]
fn a_formula_with_xml_forbidden_text_is_rejected_without_changing_the_workbook() {
    // A formula has no `_xHHHH_` escape, so XML's own limits apply to it.
    let mut workbook = open("sample-workbook.xlsx");
    let before = workbook.bytes().to_vec();

    let error = workbook
        .set("売上サマリ", "A20", CellInput::Formula("A1&\"\0\"".into()))
        .expect_err("NUL is forbidden by XML 1.0");
    assert!(error.to_string().contains("XML 1.0"));
    assert_eq!(workbook.bytes(), before);
}

#[test]
fn a_set_recalculates_the_formulas_that_depended_on_it() {
    let mut workbook = open("sample-workbook.xlsx");
    let c4 = workbook
        .cell("売上サマリ", "C4")
        .expect("C4")
        .number
        .expect("a number");

    workbook
        .set("売上サマリ", "B4", CellInput::Number(1_000.0))
        .expect("set");

    let d4 = workbook.cell("売上サマリ", "D4").expect("D4");
    assert_eq!(d4.formula.as_deref(), Some("SUM(B4:C4)"));
    // The dependent moved with the edit instead of keeping the stale cache.
    assert_eq!(d4.number, Some(1_000.0 + c4));
    // And it is no longer stale: the session recalculated it.
    assert_eq!(d4.formula_state, Some(FormulaState::Cached));
}

#[test]
fn a_written_formula_computes_immediately() {
    let mut workbook = open("sample-workbook.xlsx");
    let b4 = workbook
        .cell("売上サマリ", "B4")
        .expect("B4")
        .number
        .expect("a number");

    workbook
        .set("売上サマリ", "F4", CellInput::Formula("B4*2".into()))
        .expect("set");

    let f4 = workbook.cell("売上サマリ", "F4").expect("F4");
    assert_eq!(f4.formula.as_deref(), Some("B4*2"));
    assert_eq!(f4.number, Some(b4 * 2.0));
    assert_eq!(f4.formula_state, Some(FormulaState::Cached));
}

#[test]
fn recalculated_values_survive_export_and_reopen() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "B4", CellInput::Number(1_000.0))
        .expect("set");
    let expected = workbook.cell("売上サマリ", "D4").expect("D4").number;

    let reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    let d4 = reopened.cell("売上サマリ", "D4").expect("D4");
    assert_eq!(d4.number, expected);
    // The file's caches were refreshed, so it no longer asks the next
    // spreadsheet to recalculate before showing numbers.
    assert_eq!(d4.formula_state, Some(FormulaState::Cached));
}

#[test]
fn recalculate_on_demand_freshens_a_stale_workbook() {
    // A workbook saved by an earlier write carries `fullCalcOnLoad`; opening
    // it and recalculating clears the staleness without any new edit.
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "B4", CellInput::Number(500.0))
        .expect("set");

    let mut reopened = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    let summary = reopened.recalculate().expect("recalculate");
    assert!(summary.skipped.is_empty(), "{:?}", summary.skipped);
    assert_eq!(
        reopened.cell("売上サマリ", "D4").expect("D4").formula_state,
        Some(FormulaState::Cached)
    );
}

#[test]
fn undo_restores_the_bytes_before_the_last_write_and_redo_reapplies_them() {
    let mut workbook = open("sample-workbook.xlsx");
    let original = workbook.bytes().to_vec();
    workbook
        .set("売上サマリ", "B4", CellInput::Number(999.0))
        .expect("set");
    let edited = workbook.bytes().to_vec();
    assert_eq!(
        workbook.cell("売上サマリ", "D4").expect("D4").display,
        "¥15,230,999"
    );

    assert!(workbook.undo().expect("undo"));
    // The value, its dependents and the bytes are exactly as before — the
    // recalculation that followed the write is undone with it.
    assert_eq!(
        workbook.cell("売上サマリ", "B4").expect("B4").display,
        "¥12,850,000"
    );
    assert_eq!(
        workbook.cell("売上サマリ", "D4").expect("D4").display,
        "¥28,080,000"
    );
    assert_eq!(workbook.bytes(), &original[..]);

    assert!(workbook.redo().expect("redo"));
    assert_eq!(
        workbook.cell("売上サマリ", "B4").expect("B4").display,
        "¥999"
    );
    assert_eq!(workbook.bytes(), &edited[..]);
}

#[test]
fn undo_with_nothing_to_undo_is_a_no_op_that_says_so() {
    let mut workbook = open("sample-workbook.xlsx");
    let before = workbook.bytes().to_vec();
    assert!(!workbook.undo().expect("undo"));
    assert!(!workbook.redo().expect("redo"));
    assert_eq!(workbook.bytes(), &before[..]);
    assert!(!workbook.can_undo());
    assert!(!workbook.can_redo());
}

#[test]
fn a_new_write_after_an_undo_discards_the_redo_branch() {
    let mut workbook = open("sample-workbook.xlsx");
    workbook
        .set("売上サマリ", "B4", CellInput::Number(1.0))
        .expect("set");
    assert!(workbook.undo().expect("undo"));
    assert!(workbook.can_redo());
    workbook
        .set("売上サマリ", "B4", CellInput::Number(2.0))
        .expect("set");
    assert!(!workbook.can_redo());
    assert!(!workbook.redo().expect("redo"));
    assert_eq!(workbook.cell("売上サマリ", "B4").expect("B4").display, "¥2");
}

#[test]
fn an_explicit_recalculation_is_not_an_undo_step() {
    // Recalculating only refreshes caches; undoing it would put stale
    // numbers back on purpose, which nobody means by "undo".
    let mut workbook = open("sample-workbook.xlsx");
    workbook.recalculate().expect("recalculate");
    assert!(!workbook.can_undo());
}
