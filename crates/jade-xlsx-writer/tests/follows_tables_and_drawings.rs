//! The parts hung off a sheet — its table, its drawing (a text box and a
//! chart), its comment and the VML that places it, its page breaks,
//! protected range and extension conditional format — follow the grid
//! when rows and columns come and go, the way Excel moves them.

use jade_opc::OpcPackage;
use jade_xlsx_headless_editor::HeadlessWorkbook;
use jade_xlsx_reader::{read_xlsx, Axis, CellRef, ShapeKind, StructureChange};
use jade_xlsx_writer::restructure;
use std::path::PathBuf;

const SHEET: &str = "xl/worksheets/sheet1.xml";
const TABLE: &str = "xl/tables/table1.xml";
const DRAWING: &str = "xl/drawings/drawing1.xml";
const CHART: &str = "xl/charts/chart1.xml";
const COMMENTS: &str = "xl/comments1.xml";
const VML: &str = "xl/drawings/vmlDrawing1.vml";
const NAME: &str = "Data";

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/objects-workbook.xlsx"
    ));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn change(axis: Axis, at: u32, count: u32, inserted: bool) -> StructureChange {
    StructureChange {
        sheet: NAME.to_string(),
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
        .unwrap_or_else(|| panic!("no part {name}"))
}

fn has_part(bytes: &[u8], name: &str) -> bool {
    OpcPackage::open(bytes)
        .expect("package")
        .part(name)
        .is_some()
}

fn display(bytes: &[u8], reference: &str) -> String {
    HeadlessWorkbook::open(bytes)
        .expect("open")
        .cell(NAME, reference)
        .expect("cell")
        .display
}

#[test]
fn a_row_inserted_inside_the_table_grows_it_and_everything_anchored_below_moves() {
    let written = apply(&fixture(), &change(Axis::Rows, 2, 1, true));

    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="A1:C6""#), "{table}");
    assert!(table.contains(r#"<autoFilter ref="A1:C5"/>"#), "{table}");
    assert!(table.contains(r#"count="3""#), "{table}");

    let drawing = part(&written, DRAWING);
    // The text box spans the insertion: its top stays, its bottom moves.
    assert!(
        drawing.contains("<xdr:row>1</xdr:row><xdr:rowOff>19050</xdr:rowOff>"),
        "{drawing}"
    );
    assert!(
        drawing.contains("<xdr:row>4</xdr:row><xdr:rowOff>76200</xdr:rowOff>"),
        "{drawing}"
    );
    // The chart, wholly below, moves down.
    assert!(drawing.contains("<xdr:row>6</xdr:row>"), "{drawing}");
    assert!(drawing.contains("<xdr:row>14</xdr:row>"), "{drawing}");
    assert!(!drawing.contains("<xdr:row>5</xdr:row>"), "{drawing}");

    let chart = part(&written, CHART);
    assert!(chart.contains("<c:f>Data!$C$2:$C$5</c:f>"), "{chart}");
    assert!(chart.contains("<c:f>Data!$A$2:$A$5</c:f>"), "{chart}");
    assert!(chart.contains("<c:f>Data!$C$1</c:f>"), "{chart}");

    // The comment is above the insertion and stays; its VML anchor's
    // bottom row moves.
    assert!(part(&written, COMMENTS).contains(r#"ref="B2""#));
    let vml = part(&written, VML);
    assert!(
        vml.contains("<x:Anchor>2, 15, 0, 2, 4, 31, 4, 1</x:Anchor>"),
        "{vml}"
    );
    assert!(vml.contains("<x:Row>1</x:Row>"), "{vml}");

    let sheet = part(&written, SHEET);
    assert!(
        sheet.contains(r#"<brk id="4" max="16383" man="1"/>"#),
        "{sheet}"
    );
    assert!(
        sheet.contains(r#"<protectedRange sqref="B2:B5" name="Quantities"/>"#),
        "{sheet}"
    );
    assert!(
        sheet.contains(r#"<conditionalFormatting sqref="C2:C5">"#),
        "{sheet}"
    );
    assert!(sheet.contains("<formula>$C$4</formula>"), "{sheet}");
    assert!(sheet.contains("<xm:sqref>C2:C5</xm:sqref>"), "{sheet}");
    assert!(
        sheet.contains("<oddHeader>&amp;CConfidential</oddHeader>"),
        "{sheet}"
    );
    assert!(part(&written, "xl/workbook.xml").contains("Data!$A$1:$C$6"));

    let workbook = read_xlsx(&written).expect("reread");
    let data = &workbook.sheets[0];
    assert_eq!(data.tables[0].reference.end, CellRef::new(2, 5));
    assert_eq!(data.tables[0].columns, ["Item", "Qty", "Amount"]);
    assert_eq!(data.shapes[0].to, Some(CellRef::new(6, 4)));
    assert_eq!(data.row_breaks[0].before, 4);
    assert_eq!(display(&written, "A6"), "Total");
    assert_eq!(
        part(&written, "xl/styles.xml"),
        part(&fixture(), "xl/styles.xml")
    );
}

#[test]
fn deleting_the_header_row_makes_the_next_row_the_header_and_writes_the_names() {
    let written = apply(&fixture(), &change(Axis::Rows, 0, 1, false));

    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="A1:C4""#), "{table}");
    assert!(table.contains(r#"totalsRowCount="1""#), "{table}");
    assert!(table.contains(r#"<autoFilter ref="A1:C3"/>"#), "{table}");
    // Excel rewrites the new header row with the column names.
    assert_eq!(display(&written, "A1"), "Item");
    assert_eq!(display(&written, "B1"), "Qty");
    assert_eq!(display(&written, "C1"), "Amount");
    assert_eq!(display(&written, "A2"), "Gadget");
    assert_eq!(display(&written, "A4"), "Total");

    assert!(part(&written, COMMENTS).contains(r#"ref="B1""#));
    let vml = part(&written, VML);
    assert!(vml.contains("<x:Row>0</x:Row>"), "{vml}");
    // The anchor's top row was in the deleted range: it lands on the
    // boundary with no offset; the bottom row moves up.
    assert!(
        vml.contains("<x:Anchor>2, 15, 0, 0, 4, 31, 2, 1</x:Anchor>"),
        "{vml}"
    );

    let drawing = part(&written, DRAWING);
    assert!(
        drawing.contains("<xdr:row>0</xdr:row><xdr:rowOff>19050</xdr:rowOff>"),
        "{drawing}"
    );
    assert!(
        drawing.contains("<xdr:row>2</xdr:row><xdr:rowOff>76200</xdr:rowOff>"),
        "{drawing}"
    );

    let sheet = part(&written, SHEET);
    assert!(
        sheet.contains(r#"<brk id="2" max="16383" man="1"/>"#),
        "{sheet}"
    );
    // The print titles named the deleted row.
    assert!(part(&written, "xl/workbook.xml").contains("#REF!"));

    let workbook = read_xlsx(&written).expect("reread");
    assert_eq!(workbook.sheets[0].tables[0].header_row(), Some(0));
    assert_eq!(workbook.sheets[0].tables[0].totals_row(), Some(3));
}

#[test]
fn deleting_every_row_of_the_table_removes_it_and_what_sat_on_those_rows() {
    let original = fixture();
    let written = apply(&original, &change(Axis::Rows, 0, 5, false));

    assert!(!has_part(&written, TABLE));
    assert!(!has_part(&written, COMMENTS));
    let sheet = part(&written, SHEET);
    assert!(!sheet.contains("tablePart"), "{sheet}");
    assert!(!sheet.contains("<rowBreaks"), "{sheet}");
    assert!(!sheet.contains("<protectedRange"), "{sheet}");
    assert!(!sheet.contains("<conditionalFormatting"), "{sheet}");
    assert!(!sheet.contains("<x14:conditionalFormatting "), "{sheet}");
    assert!(sheet.contains("<x14:conditionalFormattings>"), "{sheet}");
    let rels = part(&written, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(!rels.contains("table1.xml"), "{rels}");
    assert!(!rels.contains("comments1.xml"), "{rels}");
    let types = part(&written, "[Content_Types].xml");
    assert!(!types.contains("table1.xml"), "{types}");
    assert!(!types.contains("comments1.xml"), "{types}");

    // The note's shape went with its cell; the VML part stays.
    let vml = part(&written, VML);
    assert!(!vml.contains("<v:shape "), "{vml}");
    assert!(vml.contains("<v:shapetype "), "{vml}");

    let drawing = part(&written, DRAWING);
    assert!(!drawing.contains("TextBox 1"), "{drawing}");
    // The chart overlapped the deletion: its top comes to the boundary.
    assert!(drawing.contains("Chart 2"), "{drawing}");
    assert!(
        drawing.contains("<xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff>"),
        "{drawing}"
    );
    assert!(drawing.contains("<xdr:row>8</xdr:row>"), "{drawing}");
    // The one-cell anchor moves rather than going.
    assert!(drawing.contains("Rectangle 3"), "{drawing}");

    let chart = part(&written, CHART);
    // Excel spells a series whose cells went `Sheet!#REF!`.
    assert!(chart.contains("<c:f>Data!#REF!</c:f>"), "{chart}");
    assert!(!chart.contains("$C$2"), "{chart}");

    let workbook = read_xlsx(&written).expect("reread");
    assert!(workbook.sheets[0].tables.is_empty());
    assert!(workbook.sheets[0].comments.is_empty());
    assert_eq!(workbook.sheets[0].shapes.len(), 2);
    assert_eq!(workbook.sheets[0].shapes[0].kind, ShapeKind::Chart);
    // Every other part is as it was.
    assert_eq!(
        part(&written, "xl/styles.xml"),
        part(&original, "xl/styles.xml")
    );
}

#[test]
fn deleting_the_totals_row_turns_totals_off() {
    let written = apply(&fixture(), &change(Axis::Rows, 4, 1, false));
    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="A1:C4""#), "{table}");
    assert!(!table.contains("totalsRowCount"), "{table}");
    assert!(table.contains(r#"totalsRowShown="0""#), "{table}");
    assert!(table.contains(r#"<autoFilter ref="A1:C4"/>"#), "{table}");
    let workbook = read_xlsx(&written).expect("reread");
    assert_eq!(workbook.sheets[0].tables[0].totals_row(), None);
}

#[test]
fn deleting_every_data_row_keeps_the_header_and_drops_the_totals() {
    let written = apply(&fixture(), &change(Axis::Rows, 1, 3, false));
    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="A1:C2""#), "{table}");
    assert!(!table.contains("totalsRowCount"), "{table}");
    assert!(table.contains(r#"<autoFilter ref="A1:C2"/>"#), "{table}");
    assert_eq!(display(&written, "A1"), "Item");
    assert_eq!(display(&written, "A2"), "Total");
}

#[test]
fn a_column_inserted_inside_the_table_becomes_a_named_table_column() {
    let written = apply(&fixture(), &change(Axis::Columns, 1, 1, true));

    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="A1:D5""#), "{table}");
    assert!(table.contains(r#"<autoFilter ref="A1:D4"/>"#), "{table}");
    assert!(table.contains(r#"<tableColumns count="4">"#), "{table}");
    // The new column sits between Item and Qty, with an id no column had.
    assert!(
        table.contains(r#"<tableColumn id="4" name="Column1"/>"#),
        "{table}"
    );
    let item = table.find(r#"name="Item""#).expect("Item");
    let new_column = table.find(r#"name="Column1""#).expect("Column1");
    let qty = table.find(r#"name="Qty""#).expect("Qty");
    assert!(item < new_column && new_column < qty, "{table}");
    // The header cell says what the column is called.
    assert_eq!(display(&written, "B1"), "Column1");
    assert_eq!(display(&written, "C1"), "Qty");
    assert_eq!(display(&written, "C2"), "2");

    assert!(part(&written, COMMENTS).contains(r#"ref="C2""#));
    let vml = part(&written, VML);
    assert!(vml.contains("<x:Column>2</x:Column>"), "{vml}");
    assert!(
        vml.contains("<x:Anchor>3, 15, 0, 2, 5, 31, 3, 1</x:Anchor>"),
        "{vml}"
    );

    let drawing = part(&written, DRAWING);
    assert!(
        drawing.contains("<xdr:col>5</xdr:col><xdr:colOff>9525</xdr:colOff>"),
        "{drawing}"
    );
    assert!(
        drawing.contains("<xdr:col>7</xdr:col><xdr:colOff>38100</xdr:colOff>"),
        "{drawing}"
    );

    let chart = part(&written, CHART);
    assert!(chart.contains("<c:f>Data!$D$2:$D$4</c:f>"), "{chart}");
    assert!(chart.contains("<c:f>Data!$A$2:$A$4</c:f>"), "{chart}");
    assert!(chart.contains("<c:f>Data!$D$1</c:f>"), "{chart}");

    let workbook = read_xlsx(&written).expect("reread");
    assert_eq!(
        workbook.sheets[0].tables[0].columns,
        ["Item", "Column1", "Qty", "Amount"]
    );
}

#[test]
fn a_deleted_column_takes_its_table_column_and_its_comment_with_it() {
    let written = apply(&fixture(), &change(Axis::Columns, 1, 1, false));

    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="A1:B5""#), "{table}");
    assert!(table.contains(r#"<tableColumns count="2">"#), "{table}");
    assert!(!table.contains(r#"name="Qty""#), "{table}");
    assert!(table.contains(r#"name="Amount""#), "{table}");

    assert!(!has_part(&written, COMMENTS));
    assert!(!part(&written, VML).contains("<v:shape "));

    let drawing = part(&written, DRAWING);
    assert!(
        drawing.contains("<xdr:col>3</xdr:col><xdr:colOff>9525</xdr:colOff>"),
        "{drawing}"
    );
    assert!(
        drawing.contains("<xdr:col>5</xdr:col><xdr:colOff>38100</xdr:colOff>"),
        "{drawing}"
    );
    assert!(part(&written, CHART).contains("<c:f>Data!$B$2:$B$4</c:f>"));

    let workbook = read_xlsx(&written).expect("reread");
    assert_eq!(workbook.sheets[0].tables[0].columns, ["Item", "Amount"]);
    assert_eq!(display(&written, "B5"), "749");
}

#[test]
fn a_column_inserted_before_the_table_moves_it_whole() {
    let written = apply(&fixture(), &change(Axis::Columns, 0, 2, true));
    let table = part(&written, TABLE);
    assert!(table.contains(r#"ref="C1:E5""#), "{table}");
    assert!(table.contains(r#"<tableColumns count="3">"#), "{table}");
    assert_eq!(display(&written, "C1"), "Item");
    assert_eq!(display(&written, "A1"), "");
}
