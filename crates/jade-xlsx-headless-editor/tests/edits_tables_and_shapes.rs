//! Tables, the page header and footer, page breaks and shapes, as a caller
//! with no screen sees and changes them.

use jade_xlsx_headless_api::{CellInput, HeaderFooterChange, ShapeKindView};
use jade_xlsx_headless_editor::{HeadlessError, HeadlessWorkbook};
use std::path::PathBuf;

const SHEET: &str = "Data";

fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/objects-workbook.xlsx"
    ));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    HeadlessWorkbook::open(&bytes).expect("open")
}

fn part(workbook: &HeadlessWorkbook, name: &str) -> String {
    jade_opc::OpcPackage::open(workbook.bytes())
        .expect("package")
        .part_str(name)
        .expect("read")
        .unwrap_or_else(|| panic!("no part {name}"))
}

#[test]
fn the_summary_lists_the_table_the_header_footer_the_breaks_and_the_shapes() {
    let workbook = open();
    let summary = workbook.summary();
    let sheet = &summary.sheets[0];

    let table = &sheet.tables[0];
    assert_eq!(table.name, "Table1");
    assert_eq!(table.reference, "A1:C5");
    assert_eq!(table.header_row, Some(1));
    assert_eq!(table.totals_row, Some(5));
    assert_eq!(table.columns, ["Item", "Qty", "Amount"]);
    assert_eq!(table.style.as_deref(), Some("TableStyleMedium2"));
    assert!(table.auto_filter);

    let header_footer = sheet.header_footer.as_ref().expect("header/footer");
    assert_eq!(header_footer.odd_header.as_deref(), Some("&CConfidential"));
    assert_eq!(header_footer.odd_footer.as_deref(), Some("&RPage &P of &N"));
    assert_eq!(header_footer.first_header.as_deref(), Some("&LCover"));
    assert!(header_footer.different_first);
    assert!(!header_footer.different_odd_even);
    assert!(header_footer.scale_with_doc);

    let breaks = sheet.page_breaks.as_ref().expect("breaks");
    assert_eq!(breaks.rows, [4]);
    assert!(breaks.columns.is_empty());

    assert_eq!(sheet.shapes.len(), 3);
    assert_eq!(sheet.shapes[0].name, "TextBox 1");
    assert_eq!(sheet.shapes[0].kind, ShapeKindView::TextBox);
    assert_eq!(sheet.shapes[0].anchor, "E2:G4");
    assert_eq!(sheet.shapes[0].text.as_deref(), Some("Draft"));
    assert_eq!(sheet.shapes[1].kind, ShapeKindView::Chart);
    assert_eq!(sheet.shapes[1].text, None);
    assert_eq!(sheet.shapes[2].anchor, "J3");
    assert_eq!(
        sheet.shapes[2].text.as_deref(),
        Some("Note & label\nline two")
    );

    // The sample workbook has none of these, and says nothing about them.
    let plain = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let plain = HeadlessWorkbook::open(&std::fs::read(plain).expect("fixture")).expect("open");
    let json = serde_json::to_string(&plain.summary()).expect("json");
    assert!(!json.contains("\"tables\""), "{json}");
    assert!(!json.contains("\"shapes\""), "{json}");
    assert!(!json.contains("\"header_footer\""), "{json}");
}

#[test]
fn a_cell_says_which_table_and_column_it_is_in() {
    let workbook = open();
    let header = workbook.cell(SHEET, "B1").expect("cell");
    let table = header.table.expect("in a table");
    assert_eq!(table.name, "Table1");
    assert_eq!(table.column.as_deref(), Some("Qty"));
    assert!(table.header);
    assert!(!table.totals);

    let totals = workbook
        .cell(SHEET, "C5")
        .expect("cell")
        .table
        .expect("in a table");
    assert!(totals.totals);
    assert_eq!(totals.column.as_deref(), Some("Amount"));

    assert!(workbook.cell(SHEET, "E2").expect("cell").table.is_none());
}

#[test]
fn typing_into_a_header_cell_renames_the_column() {
    let mut workbook = open();
    workbook
        .set(SHEET, "B1", CellInput::Text("Quantity".to_string()))
        .expect("set");
    assert_eq!(
        workbook.summary().sheets[0].tables[0].columns,
        ["Item", "Quantity", "Amount"]
    );
    assert!(part(&workbook, "xl/tables/table1.xml").contains(r#"name="Quantity""#));
    assert_eq!(
        workbook.cell(SHEET, "B1").expect("cell").display,
        "Quantity"
    );

    // A name the table already has gets a number, as Excel gives it.
    workbook
        .set(SHEET, "B1", CellInput::Text("Item".to_string()))
        .expect("set");
    assert_eq!(
        workbook.summary().sheets[0].tables[0].columns,
        ["Item", "Item2", "Amount"]
    );
    assert_eq!(workbook.cell(SHEET, "B1").expect("cell").display, "Item2");

    // A blank header gets a name of its own, and the cell holds it.
    workbook.set(SHEET, "B1", CellInput::Blank).expect("set");
    assert_eq!(
        workbook.summary().sheets[0].tables[0].columns,
        ["Item", "Column1", "Amount"]
    );
    assert_eq!(workbook.cell(SHEET, "B1").expect("cell").display, "Column1");

    // A header is always text: a number becomes its text.
    workbook
        .set(SHEET, "B1", CellInput::Number(2024.0))
        .expect("set");
    assert_eq!(
        workbook.summary().sheets[0].tables[0].columns,
        ["Item", "2024", "Amount"]
    );
    let cell = workbook.cell(SHEET, "B1").expect("cell");
    assert_eq!(cell.kind, jade_xlsx_headless_api::CellKind::Text);

    // Undo takes the table part back with the cell.
    assert!(workbook.undo().expect("undo"));
    assert_eq!(
        workbook.summary().sheets[0].tables[0].columns,
        ["Item", "Column1", "Amount"]
    );

    // A data cell is just a cell.
    workbook
        .set(SHEET, "B2", CellInput::Text("Item".to_string()))
        .expect("set");
    assert_eq!(
        workbook.summary().sheets[0].tables[0].columns,
        ["Item", "Column1", "Amount"]
    );
}

#[test]
fn the_header_and_footer_can_be_set_section_by_section() {
    let mut workbook = open();
    let view = workbook
        .set_header_footer(
            SHEET,
            &HeaderFooterChange {
                odd_footer: Some(Some("&L&D&RPage &P".to_string())),
                first_header: Some(None),
                ..HeaderFooterChange::default()
            },
        )
        .expect("set");
    // What was given changed; what was not is kept.
    assert_eq!(view.odd_footer.as_deref(), Some("&L&D&RPage &P"));
    assert_eq!(view.odd_header.as_deref(), Some("&CConfidential"));
    assert_eq!(view.first_header, None);
    assert!(view.different_first);
    let sheet = part(&workbook, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains(r#"<headerFooter differentFirst="1"><oddHeader>&amp;CConfidential</oddHeader><oddFooter>&amp;L&amp;D&amp;RPage &amp;P</oddFooter></headerFooter>"#),
        "{sheet}"
    );
    // Still between pageSetup and rowBreaks.
    let at = sheet.find("<headerFooter").expect("element");
    assert!(sheet.find("<pageSetup").expect("pageSetup") < at);
    assert!(sheet.find("<rowBreaks").expect("rowBreaks") > at);

    assert!(matches!(
        workbook.set_header_footer(SHEET, &HeaderFooterChange::default()),
        Err(HeadlessError::EmptyHeaderFooterChange)
    ));

    assert!(workbook.undo().expect("undo"));
    assert_eq!(
        workbook.summary().sheets[0]
            .header_footer
            .as_ref()
            .and_then(|view| view.odd_footer.clone())
            .as_deref(),
        Some("&RPage &P of &N")
    );
}

#[test]
fn a_sheet_without_a_header_gets_one_in_the_right_place() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let mut workbook =
        HeadlessWorkbook::open(&std::fs::read(path).expect("fixture")).expect("open");
    let sheet_name = workbook.summary().sheets[0].name.clone();
    let view = workbook
        .set_header_footer(
            &sheet_name,
            &HeaderFooterChange {
                odd_header: Some(Some("&A".to_string())),
                ..HeaderFooterChange::default()
            },
        )
        .expect("set");
    assert_eq!(view.odd_header.as_deref(), Some("&A"));
    assert!(view.scale_with_doc);
    let sheet = part(&workbook, &workbook.workbook().sheets[0].part_name.clone());
    assert!(
        sheet.contains("<headerFooter><oddHeader>&amp;A</oddHeader></headerFooter>"),
        "{sheet}"
    );
    // Clearing the only section removes the element.
    workbook
        .set_header_footer(
            &sheet_name,
            &HeaderFooterChange {
                odd_header: Some(None),
                ..HeaderFooterChange::default()
            },
        )
        .expect("clear");
    let sheet = part(&workbook, &workbook.workbook().sheets[0].part_name.clone());
    assert!(!sheet.contains("<headerFooter"), "{sheet}");
}

#[test]
fn a_text_box_takes_new_text_and_keeps_its_font() {
    let mut workbook = open();
    let view = workbook
        .set_shape_text(SHEET, "TextBox 1", "Final\n\nv2 & up")
        .expect("set");
    assert_eq!(view.text.as_deref(), Some("Final\n\nv2 & up"));
    assert_eq!(view.anchor, "E2:G4");
    let drawing = part(&workbook, "xl/drawings/drawing1.xml");
    let owned = jade_xlsx_reader::object_protection::drawing_source(&drawing, 2)
        .expect("owned drawing")
        .expect("shape")
        .text
        .expect("text body");
    assert_eq!(owned.paragraphs.len(), 3);
    for span in &owned.paragraphs {
        assert!(drawing[span.clone()].contains(r#"<a:pPr algn="ctr"/>"#));
    }
    assert!(
        drawing[owned.paragraphs[0].clone()].contains(r#"<a:rPr lang="en-US" sz="1400" b="1"/>"#)
    );
    assert!(drawing[owned.paragraphs[1].clone()]
        .contains(r#"<a:endParaRPr lang="en-US" sz="1400" b="1"/>"#));
    assert!(
        drawing[owned.paragraphs[2].clone()].contains(r#"<a:rPr lang="en-US" sz="1400" b="1"/>"#)
    );
    assert!(drawing[owned.paragraphs[0].clone()].contains(">Final</"));
    assert!(drawing[owned.paragraphs[2].clone()].contains(">v2 &amp; up</"));
    // Position, size and the other shapes are untouched.
    assert!(drawing.contains("<xdr:row>3</xdr:row><xdr:rowOff>76200</xdr:rowOff>"));
    assert!(drawing.contains("<a:t>line two</a:t>"));

    // By id as well as by name.
    workbook.set_shape_text(SHEET, "4", "Renamed").expect("set");
    assert_eq!(
        workbook.summary().sheets[0].shapes[2].text.as_deref(),
        Some("Renamed")
    );

    assert!(matches!(
        workbook.set_shape_text(SHEET, "Chart 2", "x"),
        Err(HeadlessError::ShapeHasNoText(_))
    ));
    assert!(matches!(
        workbook.set_shape_text(SHEET, "Oval 9", "x"),
        Err(HeadlessError::NoSuchShape(_))
    ));

    assert!(workbook.undo().expect("undo"));
    assert!(workbook.undo().expect("undo"));
    assert_eq!(
        workbook.summary().sheets[0].shapes[0].text.as_deref(),
        Some("Draft")
    );
}

#[test]
fn the_grid_operations_carry_the_table_and_the_shapes_along() {
    let mut workbook = open();
    workbook.insert_columns(SHEET, "B", 1).expect("insert");
    let summary = workbook.summary();
    assert_eq!(summary.sheets[0].tables[0].reference, "A1:D5");
    assert_eq!(
        summary.sheets[0].tables[0].columns,
        ["Item", "Column1", "Qty", "Amount"]
    );
    assert_eq!(summary.sheets[0].shapes[0].anchor, "F2:H4");
    assert_eq!(workbook.cell(SHEET, "B1").expect("cell").display, "Column1");
    assert!(workbook
        .cell(SHEET, "C2")
        .expect("cell")
        .comment
        .is_some_and(|comment| comment.text.contains("quantity")));

    workbook.delete_rows(SHEET, 1, 5).expect("delete");
    let summary = workbook.summary();
    assert!(summary.sheets[0].tables.is_empty());
    assert!(summary.sheets[0].page_breaks.is_none());
    assert_eq!(summary.sheets[0].shapes.len(), 2);

    assert!(workbook.undo().expect("undo"));
    assert!(workbook.undo().expect("undo"));
    assert_eq!(workbook.summary().sheets[0].tables[0].reference, "A1:C5");
}

#[test]
fn a_note_can_be_put_on_a_cell_and_taken_off_again() {
    let mut workbook = open();
    let original_vml = part(&workbook, "xl/drawings/vmlDrawing1.vml");
    // Replacing the note a cell has.
    let view = workbook
        .set_note(SHEET, "B2", "Recount & confirm", "Auditor")
        .expect("set");
    assert_eq!(view.text, "Recount & confirm");
    assert_eq!(view.author.as_deref(), Some("Auditor"));
    let comments = part(&workbook, "xl/comments1.xml");
    let owned = jade_xlsx_reader::object_protection::comments_source(&comments).expect("comments");
    assert_eq!(owned.authors, ["Reviewer", "Auditor"]);
    assert_eq!(owned.comments.len(), 1);
    assert_eq!(
        owned.comments[0].at,
        Some(jade_xlsx_reader::CellRef::new(1, 1))
    );
    assert_eq!(owned.comments[0].author_id, Some(1));
    assert_eq!(owned.comments[0].value, "Recount & confirm");
    // Retain the existing comment identity attribute instead of rebuilding it.
    assert!(comments.contains(r#"shapeId="0""#));
    let vml = part(&workbook, "xl/drawings/vmlDrawing1.vml");
    assert_eq!(vml, original_vml);
    assert_eq!(vml.matches("<v:shape ").count(), 1, "{vml}");
    let owned_vml = jade_xlsx_reader::object_protection::vml_sources(&vml).expect("VML");
    assert_eq!(owned_vml.notes.len(), 1);
    assert_eq!(owned_vml.notes[0].at, jade_xlsx_reader::CellRef::new(1, 1));

    // A second note lands in cell order, before the existing one.
    workbook
        .set_note(SHEET, "A1", "Header", "Auditor")
        .expect("set");
    let comments = part(&workbook, "xl/comments1.xml");
    let a1 = comments.find(r#"ref="A1""#).expect("A1");
    let b2 = comments.find(r#"ref="B2""#).expect("B2");
    assert!(a1 < b2, "{comments}");
    assert_eq!(
        workbook
            .cell(SHEET, "A1")
            .expect("cell")
            .comment
            .map(|comment| comment.text),
        Some("Header".to_string())
    );
    assert_eq!(
        part(&workbook, "xl/drawings/vmlDrawing1.vml")
            .matches("<v:shape ")
            .count(),
        2
    );

    assert!(workbook.delete_note(SHEET, "A1").expect("delete"));
    assert!(!workbook.delete_note(SHEET, "A1").expect("delete again"));
    assert!(workbook.cell(SHEET, "A1").expect("cell").comment.is_none());
    assert_eq!(
        part(&workbook, "xl/drawings/vmlDrawing1.vml")
            .matches("<v:shape ")
            .count(),
        1
    );

    // The last note takes the comments part with it; the VML stays.
    assert!(workbook.delete_note(SHEET, "B2").expect("delete"));
    let package = jade_opc::OpcPackage::open(workbook.bytes()).expect("package");
    assert!(package.part("xl/comments1.xml").is_none());
    assert!(package.part("xl/drawings/vmlDrawing1.vml").is_some());
    assert!(!part(&workbook, "xl/worksheets/_rels/sheet1.xml.rels").contains("comments1.xml"));
    assert!(!part(&workbook, "[Content_Types].xml").contains("comments1.xml"));

    assert!(workbook.undo().expect("undo"));
    assert!(workbook.cell(SHEET, "B2").expect("cell").comment.is_some());
}

#[test]
fn the_first_note_on_a_sheet_brings_its_parts_with_it() {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let mut workbook =
        HeadlessWorkbook::open(&std::fs::read(path).expect("fixture")).expect("open");
    let sheet_name = workbook.summary().sheets[0].name.clone();
    let sheet_part = workbook.workbook().sheets[0].part_name.clone();
    let view = workbook
        .set_note(&sheet_name, "C3", "見直し\nrequired", "経理")
        .expect("set");
    assert_eq!(view.text, "見直し\nrequired");
    assert_eq!(view.author.as_deref(), Some("経理"));

    let package = jade_opc::OpcPackage::open(workbook.bytes()).expect("package");
    let comments = part(&workbook, "xl/comments1.xml");
    assert!(
        comments.contains(r#"<comment ref="C3" authorId="0">"#),
        "{comments}"
    );
    let vml = part(&workbook, "xl/drawings/vmlDrawing1.vml");
    let owned = jade_xlsx_reader::object_protection::vml_sources(&vml).expect("VML");
    assert!(owned.has_note_type);
    assert_eq!(owned.notes.len(), 1);
    assert_eq!(owned.notes[0].at, jade_xlsx_reader::CellRef::new(2, 2));
    assert!(
        vml.contains("<x:Row>2</x:Row><x:Column>2</x:Column>"),
        "{vml}"
    );
    let rels = part(&workbook, &jade_opc::rels_part_name(&sheet_part));
    assert!(
        rels.contains("relationships/comments\" Target=\"../comments1.xml\""),
        "{rels}"
    );
    assert!(
        rels.contains("relationships/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\""),
        "{rels}"
    );
    let sheet = part(&workbook, &sheet_part);
    let id = jade_xlsx_reader::object_protection::worksheet_drawing_relationship(&sheet, true)
        .expect("owned legacy drawing")
        .expect("relationship id");
    let relationships =
        jade_xlsx_reader::object_protection::owned_relationships(&rels).expect("relationships");
    let relationship = relationships.relationship(&id).expect("VML binding");
    assert!(jade_xlsx_reader::object_protection::office_relationship(
        relationship,
        "vmlDrawing"
    ));
    assert_eq!(relationship.target, "../drawings/vmlDrawing1.vml");
    // Before the end of the sheet, after everything that precedes it.
    let at = sheet.find("<legacyDrawing").expect("legacyDrawing");
    assert!(sheet.rfind("</sheetData>").expect("sheetData") < at);
    let types = part(&workbook, "[Content_Types].xml");
    assert!(types.contains("Extension=\"vml\""), "{types}");
    assert!(types.contains("PartName=\"/xl/comments1.xml\""), "{types}");
    drop(package);

    // Reread from the bytes, as another application would.
    let reread = HeadlessWorkbook::open(workbook.bytes()).expect("reopen");
    assert_eq!(
        reread
            .cell(&sheet_name, "C3")
            .expect("cell")
            .comment
            .map(|comment| comment.text),
        Some("見直し\nrequired".to_string())
    );
}
