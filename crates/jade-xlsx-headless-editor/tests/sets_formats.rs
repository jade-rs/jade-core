//! Making cells look different from a caller with no window.

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::{effective_style, FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::{CellRef, HorizontalAlignment};
use std::path::PathBuf;

const SHEET: &str = "売上サマリ";

fn open() -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    HeadlessWorkbook::open(&bytes).expect("open")
}

fn font_of(workbook: &HeadlessWorkbook, reference: &str) -> jade_xlsx_reader::Font {
    let sheet = &workbook.workbook().sheets[0];
    let at = CellRef::parse(reference).expect("reference");
    let index = effective_style(sheet, at);
    let styles = &workbook.workbook().styles;
    let format = styles.cell_format(Some(index)).expect("format");
    styles.font(format).cloned().unwrap_or_default()
}

fn bold() -> FormatChange {
    FormatChange {
        bold: Some(true),
        ..FormatChange::default()
    }
}

#[test]
fn bold_makes_the_cells_bold_and_keeps_their_values_and_number_formats() {
    let mut workbook = open();
    assert!(!font_of(&workbook, "B4").bold);

    let summary = workbook
        .set_format(SHEET, "B4:B5", &bold())
        .expect("format");

    assert_eq!(summary.range, "B4:B5");
    assert_eq!(summary.cells, vec!["B4", "B5"]);
    // B4 and B5 share a style, so one xf covers both.
    assert_eq!(summary.styles_added, 1);
    assert!(font_of(&workbook, "B4").bold);
    assert!(font_of(&workbook, "B5").bold);
    // The yen format and the value are as they were.
    let b4 = workbook.cell(SHEET, "B4").expect("cell");
    assert_eq!(b4.display, "¥12,850,000");
    assert_eq!(b4.number, Some(12_850_000.0));
    // And the formula that sums them still sums them.
    assert_eq!(
        workbook.cell(SHEET, "B7").expect("cell").display,
        "¥28,400,000"
    );
}

#[test]
fn the_same_look_is_reused_and_undo_takes_the_whole_change_back() {
    let mut workbook = open();
    let first = workbook
        .set_format(SHEET, "B4:B4", &bold())
        .expect("format");
    assert_eq!(first.styles_added, 1);
    let second = workbook
        .set_format(SHEET, "B5:B5", &bold())
        .expect("format");
    assert_eq!(second.styles_added, 0);
    assert!(font_of(&workbook, "B5").bold);

    assert!(workbook.undo().expect("undo"));
    assert!(!font_of(&workbook, "B5").bold);
    assert!(font_of(&workbook, "B4").bold);
    assert!(workbook.undo().expect("undo"));
    assert!(!font_of(&workbook, "B4").bold);
}

#[test]
fn a_range_with_several_looks_derives_one_style_per_look() {
    let mut workbook = open();
    // A4 is a plain label (style 3), B4:C4 currency (style 4), D4 a formula
    // in currency (style 4): two bases, two new xfs.
    let summary = workbook
        .set_format(SHEET, "A4:D4", &bold())
        .expect("format");
    assert_eq!(summary.styles_added, 2);
    for reference in ["A4", "B4", "C4", "D4"] {
        assert!(font_of(&workbook, reference).bold, "{reference}");
    }
    assert_eq!(workbook.cell(SHEET, "A4").expect("cell").display, "横浜");
    assert_eq!(
        workbook.cell(SHEET, "D4").expect("cell").display,
        "¥28,080,000"
    );
    assert_eq!(
        workbook.cell(SHEET, "D4").expect("cell").formula.as_deref(),
        Some("SUM(B4:C4)")
    );
}

#[test]
fn a_number_format_changes_what_a_cell_shows() {
    let mut workbook = open();
    workbook
        .set_cells(SHEET, &[("F1".to_string(), CellInput::Number(0.256))])
        .expect("set");
    let change = FormatChange {
        number_format: Some("0.0%".to_string()),
        ..FormatChange::default()
    };
    workbook
        .set_format(SHEET, "F1:F1", &change)
        .expect("format");
    assert_eq!(workbook.cell(SHEET, "F1").expect("cell").display, "25.6%");

    // Cells the sheet does not have yet can be formatted too: they come
    // into being empty, and show the format once a value arrives.
    workbook
        .set_format(SHEET, "F2:F2", &change)
        .expect("format");
    workbook
        .set_cells(SHEET, &[("F2".to_string(), CellInput::Number(1.0))])
        .expect("set");
    assert_eq!(workbook.cell(SHEET, "F2").expect("cell").display, "100.0%");
}

#[test]
fn alignment_and_fill_land_on_the_cell() {
    let mut workbook = open();
    let change = FormatChange {
        horizontal: Some(HorizontalAlignment::Center),
        fill_color: Some("FFC000".to_string()),
        ..FormatChange::default()
    };
    workbook
        .set_format(SHEET, "A4:A4", &change)
        .expect("format");
    let sheet = &workbook.workbook().sheets[0];
    let styles = &workbook.workbook().styles;
    let format = styles
        .cell_format(Some(effective_style(
            sheet,
            CellRef::parse("A4").expect("A4"),
        )))
        .expect("format");
    assert_eq!(format.alignment.horizontal, HorizontalAlignment::Center);
    let fill = styles.fill(format).expect("fill");
    assert_eq!(fill.pattern.as_deref(), Some("solid"));
    assert_eq!(
        fill.foreground,
        Some(jade_xlsx_reader::Color::Argb([0xFF, 0xFF, 0xC0, 0x00]))
    );
    // The label's border (style 3) is still there.
    assert_eq!(format.border_index, Some(1));
}

#[test]
fn a_change_that_names_nothing_and_a_bad_colour_are_the_callers_mistake() {
    let mut workbook = open();
    let nothing = workbook
        .set_format(SHEET, "B4:B4", &FormatChange::default())
        .expect_err("nothing to change");
    assert!(nothing.is_invalid_input());
    let bad = FormatChange {
        font_color: Some("red".to_string()),
        ..FormatChange::default()
    };
    let error = workbook
        .set_format(SHEET, "B4:B4", &bad)
        .expect_err("bad colour");
    assert!(error.is_invalid_input(), "{error}");
    assert!(!workbook.can_undo());
}
