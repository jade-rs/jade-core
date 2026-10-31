//! Data ▸ Group and Outline: rows and columns grouped, nested, collapsed and
//! expanded, ungrouped, each one undo step, and saved as Excel writes an
//! outline so that the file reopens grouped.

use jade_xlsx_headless_editor::{HeadlessWorkbook, OutlineChange};
use jade_xlsx_reader::{Axis, Sheet};

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ))
    .unwrap()
}

fn open() -> (HeadlessWorkbook, String) {
    let book = HeadlessWorkbook::open(&fixture()).unwrap();
    let sheet = book.workbook().sheets[0].name.clone();
    (book, sheet)
}

fn sheet(book: &HeadlessWorkbook) -> &Sheet {
    &book.workbook().sheets[0]
}

fn row_hidden(book: &HeadlessWorkbook, row: u32) -> bool {
    sheet(book).row(row).is_some_and(|found| found.hidden)
}

fn column_hidden(book: &HeadlessWorkbook, column: u32) -> bool {
    sheet(book)
        .columns
        .iter()
        .any(|span| span.hidden && (span.first..=span.last).contains(&column))
}

fn part(bytes: &[u8]) -> String {
    let package = jade_opc::OpcPackage::open(bytes).unwrap();
    let workbook = jade_xlsx_reader::read_xlsx(bytes).unwrap();
    package
        .part_str(&workbook.sheets[0].part_name)
        .unwrap()
        .unwrap()
}

#[test]
fn rows_group_nest_and_reopen_grouped() {
    let (mut book, name) = open();
    // Rows 2–6, then 3–4 inside them.
    assert!(book
        .outline(&name, Axis::Rows, 1, 5, OutlineChange::Group)
        .unwrap());
    assert!(book
        .outline(&name, Axis::Rows, 2, 3, OutlineChange::Group)
        .unwrap());
    let levels: Vec<(u32, u8)> = sheet(&book)
        .outline
        .row_levels
        .iter()
        .map(|(row, level)| (*row, *level))
        .collect();
    assert_eq!(levels, [(1, 1), (2, 2), (3, 2), (4, 1), (5, 1)]);

    let saved = book.save_bytes().unwrap();
    let xml = part(&saved);
    // The gutter is sized by the deepest level.
    assert!(xml.contains("outlineLevelRow=\"2\""), "{xml}");
    assert!(xml.contains("outlineLevel=\"2\""));
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    assert_eq!(reopened.workbook().sheets[0].outline, sheet(&book).outline);

    // Each was one step.
    assert!(book.undo().unwrap());
    assert_eq!(sheet(&book).outline.row_level(2), 1);
    assert!(book.undo().unwrap());
    assert!(sheet(&book).outline.is_empty());
    assert_eq!(book.bytes(), fixture().as_slice());
}

#[test]
fn hiding_and_showing_detail_keeps_collapsed_groups_inside_collapsed() {
    let (mut book, name) = open();
    book.outline(&name, Axis::Rows, 1, 5, OutlineChange::Group)
        .unwrap();
    book.outline(&name, Axis::Rows, 2, 3, OutlineChange::Group)
        .unwrap();

    // The inner group's summary is row 5 (index 4), below it.
    assert!(book
        .outline(&name, Axis::Rows, 4, 4, OutlineChange::HideDetail)
        .unwrap());
    assert!(row_hidden(&book, 2) && row_hidden(&book, 3));
    assert!(!row_hidden(&book, 4));
    assert!(sheet(&book).outline.collapsed_rows.contains(&4));

    // Collapsing the outer group from inside it hides all of it; its
    // summary is row 7.
    assert!(book
        .outline(&name, Axis::Rows, 1, 1, OutlineChange::HideDetail)
        .unwrap());
    assert!((1..=5).all(|row| row_hidden(&book, row)));
    assert!(sheet(&book).outline.collapsed_rows.contains(&6));

    // Showing the outer group brings back its own rows but not the inner
    // group, which stays collapsed.
    assert!(book
        .outline(&name, Axis::Rows, 6, 6, OutlineChange::ShowDetail)
        .unwrap());
    assert!(!row_hidden(&book, 1) && !row_hidden(&book, 4) && !row_hidden(&book, 5));
    assert!(row_hidden(&book, 2) && row_hidden(&book, 3));
    assert!(!sheet(&book).outline.collapsed_rows.contains(&6));

    assert!(book
        .outline(&name, Axis::Rows, 4, 4, OutlineChange::ShowDetail)
        .unwrap());
    assert!((1..=5).all(|row| !row_hidden(&book, row)));
    assert!(sheet(&book).outline.collapsed_rows.is_empty());

    // Nothing to show any more, and a row in no group has no detail.
    assert!(!book
        .outline(&name, Axis::Rows, 4, 4, OutlineChange::ShowDetail)
        .unwrap());
    assert!(!book
        .outline(&name, Axis::Rows, 20, 20, OutlineChange::HideDetail)
        .unwrap());
}

#[test]
fn ungrouping_takes_a_level_off_and_clears_the_gutter_at_none() {
    let (mut book, name) = open();
    book.outline(&name, Axis::Rows, 1, 3, OutlineChange::Group)
        .unwrap();
    assert!(book
        .outline(&name, Axis::Rows, 1, 3, OutlineChange::Ungroup)
        .unwrap());
    assert!(sheet(&book).outline.is_empty());
    let xml = part(book.bytes());
    assert!(!xml.contains("outlineLevel"), "{xml}");
    // Ungrouping what is not grouped changes nothing.
    assert!(!book
        .outline(&name, Axis::Rows, 1, 3, OutlineChange::Ungroup)
        .unwrap());
}

#[test]
fn columns_group_and_collapse_splitting_their_spans() {
    let (mut book, name) = open();
    // B:C grouped out of the file's B:D span; D is their summary.
    assert!(book
        .outline(&name, Axis::Columns, 1, 2, OutlineChange::Group)
        .unwrap());
    assert_eq!(sheet(&book).outline.column_level(1), 1);
    assert_eq!(sheet(&book).outline.column_level(2), 1);
    assert_eq!(sheet(&book).outline.column_level(3), 0);
    assert!(book
        .outline(&name, Axis::Columns, 3, 3, OutlineChange::HideDetail)
        .unwrap());
    assert!(column_hidden(&book, 1) && column_hidden(&book, 2));
    assert!(!column_hidden(&book, 0) && !column_hidden(&book, 3));
    assert!(sheet(&book).outline.collapsed_columns.contains(&3));
    let xml = part(book.bytes());
    assert!(xml.contains("outlineLevelCol=\"1\""), "{xml}");

    let reopened = HeadlessWorkbook::open(book.bytes()).unwrap();
    assert_eq!(reopened.workbook().sheets[0].outline, sheet(&book).outline);
}

#[test]
fn an_outline_stops_at_seven_levels() {
    let (mut book, name) = open();
    for _ in 0..7 {
        book.outline(&name, Axis::Rows, 1, 1, OutlineChange::Group)
            .unwrap();
    }
    assert_eq!(sheet(&book).outline.row_level(1), 7);
    assert!(book
        .outline(&name, Axis::Rows, 1, 1, OutlineChange::Group)
        .is_err());
    assert_eq!(sheet(&book).outline.row_level(1), 7);
}

#[test]
fn a_file_with_summaries_above_reads_and_collapses_that_way() {
    // Excel's own markup for rows 2–3 grouped with the summary in row 1.
    let (book, _) = open();
    let mut package = jade_opc::OpcPackage::open(book.bytes()).unwrap();
    let part_name = sheet(&book).part_name.clone();
    let xml = package.part_str(&part_name).unwrap().unwrap();
    assert!(!xml.contains("<sheetPr"));
    let xml = xml.replacen(
        "<sheetViews",
        "<sheetPr><outlinePr summaryBelow=\"0\"/></sheetPr><sheetViews",
        1,
    );
    package.replace(&part_name, xml.into_bytes());
    let bytes = package.write().unwrap();
    let mut book = HeadlessWorkbook::open(&bytes).unwrap();
    let name = sheet(&book).name.clone();
    assert!(!sheet(&book).outline.summary_below);
    book.outline(&name, Axis::Rows, 1, 2, OutlineChange::Group)
        .unwrap();
    // The summary of rows 2–3 is row 1 now.
    assert!(book
        .outline(&name, Axis::Rows, 0, 0, OutlineChange::HideDetail)
        .unwrap());
    assert!(row_hidden(&book, 1) && row_hidden(&book, 2));
    assert!(sheet(&book).outline.collapsed_rows.contains(&0));
}

#[test]
fn level_buttons_collapse_to_a_level_and_show_everything_again() {
    let (mut book, name) = open();
    book.outline(&name, Axis::Rows, 1, 5, OutlineChange::Group)
        .unwrap();
    book.outline(&name, Axis::Rows, 2, 3, OutlineChange::Group)
        .unwrap();

    // Level 2: the outer group's own rows show, the inner one collapses.
    assert!(book
        .outline(&name, Axis::Rows, 0, 0, OutlineChange::ShowLevel(2))
        .unwrap());
    assert!(!row_hidden(&book, 1) && !row_hidden(&book, 4) && !row_hidden(&book, 5));
    assert!(row_hidden(&book, 2) && row_hidden(&book, 3));
    assert!(sheet(&book).outline.collapsed_rows.contains(&4));
    assert!(!sheet(&book).outline.collapsed_rows.contains(&6));

    // Level 1: everything grouped collapses.
    assert!(book
        .outline(&name, Axis::Rows, 0, 0, OutlineChange::ShowLevel(1))
        .unwrap());
    assert!((1..=5).all(|row| row_hidden(&book, row)));
    assert!(sheet(&book).outline.collapsed_rows.contains(&6));

    // One past the deepest: everything shows, nothing is collapsed.
    assert!(book
        .outline(&name, Axis::Rows, 0, 0, OutlineChange::ShowLevel(3))
        .unwrap());
    assert!((1..=5).all(|row| !row_hidden(&book, row)));
    assert!(sheet(&book).outline.collapsed_rows.is_empty());
    // Showing what already shows is no step.
    assert!(!book
        .outline(&name, Axis::Rows, 0, 0, OutlineChange::ShowLevel(3))
        .unwrap());
}
