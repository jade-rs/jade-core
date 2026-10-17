//! The parts hung off a worksheet: its table, header and footer, page
//! breaks, comment, and the shapes in its drawing.

use jade_xlsx_reader::{read_xlsx, CellRef, PageBreak, ShapeKind};
use std::path::PathBuf;

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/objects-workbook.xlsx"
    ));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn reads_the_table_with_its_columns() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    assert_eq!(sheet.tables.len(), 1);
    let table = &sheet.tables[0];
    assert_eq!(table.part_name, "xl/tables/table1.xml");
    assert_eq!(table.name, "Table1");
    assert_eq!(table.display_name, "Table1");
    assert_eq!(table.reference.start, CellRef::new(0, 0));
    assert_eq!(table.reference.end, CellRef::new(2, 4));
    assert_eq!(table.header_row_count, 1);
    assert_eq!(table.totals_row_count, 1);
    assert_eq!(table.columns, ["Item", "Qty", "Amount"]);
    assert_eq!(table.style_name.as_deref(), Some("TableStyleMedium2"));
    assert!(table.auto_filter);
    assert_eq!(table.header_row(), Some(0));
    assert_eq!(table.totals_row(), Some(4));
    assert_eq!(table.column_name_at(CellRef::new(1, 2)), Some("Qty"));
    assert_eq!(table.column_name_at(CellRef::new(3, 2)), None);
    assert!(sheet.table_at(CellRef::new(2, 4)).is_some());
    assert!(sheet.table_at(CellRef::new(0, 5)).is_none());
    assert!(sheet.table_named("table1").is_some());
}

#[test]
fn reads_the_header_footer_and_the_page_breaks() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    let header_footer = sheet.header_footer.as_ref().expect("header/footer");
    assert_eq!(header_footer.odd_header.as_deref(), Some("&CConfidential"));
    assert_eq!(header_footer.odd_footer.as_deref(), Some("&RPage &P of &N"));
    assert_eq!(header_footer.first_header.as_deref(), Some("&LCover"));
    assert_eq!(header_footer.even_header, None);
    assert!(header_footer.different_first);
    assert!(!header_footer.different_odd_even);
    assert!(header_footer.scale_with_doc);
    assert!(header_footer.align_with_margins);
    assert_eq!(
        sheet.row_breaks,
        [PageBreak {
            before: 3,
            manual: true
        }]
    );
    assert!(sheet.column_breaks.is_empty());
}

#[test]
fn reads_the_shapes_by_anchor_and_text() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    assert_eq!(sheet.shapes.len(), 3);

    let text_box = &sheet.shapes[0];
    assert_eq!(text_box.id, 2);
    assert_eq!(text_box.name, "TextBox 1");
    assert_eq!(text_box.kind, ShapeKind::TextBox);
    assert_eq!(text_box.from, CellRef::new(4, 1));
    assert_eq!(text_box.to, Some(CellRef::new(6, 3)));
    assert_eq!(text_box.from_offset, (9525, 19050));
    assert_eq!(text_box.to_offset, (38100, 76200));
    assert_eq!(text_box.extent, None);
    assert_eq!(text_box.text.as_deref(), Some("Draft"));

    let chart = &sheet.shapes[1];
    assert_eq!(chart.name, "Chart 2");
    assert_eq!(chart.kind, ShapeKind::Chart);
    assert_eq!(chart.from, CellRef::new(4, 5));
    assert_eq!(chart.to, Some(CellRef::new(8, 13)));
    assert_eq!(chart.text, None);

    let rectangle = &sheet.shapes[2];
    assert_eq!(rectangle.kind, ShapeKind::Shape);
    assert_eq!(rectangle.from, CellRef::new(9, 2));
    assert_eq!(rectangle.to, None);
    assert_eq!(rectangle.extent, Some((914400, 285750)));
    assert_eq!(rectangle.text.as_deref(), Some("Note & label\nline two"));

    assert_eq!(
        sheet.shape_named("3").map(|shape| shape.name.as_str()),
        Some("Chart 2")
    );
    assert_eq!(sheet.comments[0].reference, CellRef::new(1, 1));
}
