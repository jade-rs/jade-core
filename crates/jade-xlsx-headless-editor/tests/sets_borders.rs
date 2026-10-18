//! Drawing lines around cells from a caller with no window: `set_format`
//! takes border edges, `format_at` and a cell's view report them, and a
//! saved file reopens with the same lines.

use jade_xlsx_headless_api::{BorderLine, BorderStyle, CellBorderView};
use jade_xlsx_headless_editor::{FormatChange, HeadlessWorkbook};
use jade_xlsx_reader::OpcPackage;
use std::path::PathBuf;

const SHEET: &str = "売上サマリ";

fn source() -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/sample-workbook.xlsx"
    ));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn line(style: BorderStyle, color: Option<&str>) -> BorderLine {
    BorderLine {
        style,
        color: color.map(str::to_string),
    }
}

fn borders_of(change: &FormatChange) -> FormatChange {
    FormatChange {
        border_left: change.border_left.clone(),
        border_right: change.border_right.clone(),
        border_top: change.border_top.clone(),
        border_bottom: change.border_bottom.clone(),
        border_diagonal: change.border_diagonal.clone(),
        diagonal_up: change.diagonal_up,
        diagonal_down: change.diagonal_down,
        ..FormatChange::default()
    }
}

fn parts(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let package = OpcPackage::open(bytes).expect("package");
    let mut names: Vec<String> = package.part_names().to_vec();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let bytes = package.part(&name).expect("part").to_vec();
            (name, bytes)
        })
        .collect()
}

#[test]
fn borders_round_trip_through_a_saved_file_and_touch_only_styles_and_the_sheet() {
    let original = source();
    let mut book = HeadlessWorkbook::open(&original).expect("open");
    let change = FormatChange {
        border_left: Some(line(BorderStyle::Thin, Some("FF0000"))),
        border_right: Some(line(BorderStyle::Medium, None)),
        border_top: Some(line(BorderStyle::Dotted, Some("00B050"))),
        border_bottom: Some(line(BorderStyle::Double, Some("0070C0"))),
        border_diagonal: Some(line(BorderStyle::DashDot, Some("7030A0"))),
        diagonal_up: Some(false),
        diagonal_down: Some(true),
        ..FormatChange::default()
    };
    book.set_format(SHEET, "F2:G3", &change).expect("format");
    for reference in ["F2", "G2", "F3", "G3"] {
        assert_eq!(
            borders_of(&book.format_at(SHEET, reference).expect("format")),
            change,
            "{reference}"
        );
    }

    let saved = book.export_bytes().expect("export");
    let reopened = HeadlessWorkbook::open(&saved).expect("reopen");
    assert_eq!(
        borders_of(&reopened.format_at(SHEET, "G3").expect("format")),
        change
    );
    assert_eq!(
        reopened.cell(SHEET, "F2").expect("cell").border,
        Some(CellBorderView {
            left: change.border_left.clone(),
            right: change.border_right.clone(),
            top: change.border_top.clone(),
            bottom: change.border_bottom.clone(),
            diagonal: change.border_diagonal.clone(),
            diagonal_up: false,
            diagonal_down: true,
        })
    );

    // Only the styles part and the sheet written to changed.
    let changed: Vec<String> = parts(&original)
        .into_iter()
        .zip(parts(&saved))
        .filter(|(before, after)| {
            assert_eq!(before.0, after.0);
            before.1 != after.1
        })
        .map(|(before, _)| before.0)
        .collect();
    assert_eq!(
        changed,
        vec![
            "xl/styles.xml".to_string(),
            "xl/worksheets/sheet1.xml".into()
        ]
    );
}

#[test]
fn the_file_s_borders_are_reported_and_cells_without_any_report_none() {
    let book = HeadlessWorkbook::open(&source()).expect("open");
    // A4 wears border 1: thin 8EA9DB all round.
    let thin = Some(line(BorderStyle::Thin, Some("8EA9DB")));
    let format = book.format_at(SHEET, "A4").expect("format");
    assert_eq!(format.border_left, thin);
    assert_eq!(format.border_bottom, thin);
    assert_eq!(format.border_diagonal, Some(BorderLine::none()));
    assert_eq!(format.diagonal_up, Some(false));
    assert_eq!(
        book.cell(SHEET, "A4").expect("cell").border,
        Some(CellBorderView {
            left: thin.clone(),
            right: thin.clone(),
            top: thin.clone(),
            bottom: thin,
            ..CellBorderView::default()
        })
    );
    // D10 wears both diagonals in medium green.
    let d10 = book.format_at(SHEET, "D10").expect("format");
    assert_eq!(
        d10.border_diagonal,
        Some(line(BorderStyle::Medium, Some("008000")))
    );
    assert_eq!(
        (d10.diagonal_up, d10.diagonal_down),
        (Some(true), Some(true))
    );
    assert_eq!(d10.border_left, Some(BorderLine::none()));

    // An empty unstyled cell has no border at all.
    assert_eq!(book.cell(SHEET, "H20").expect("cell").border, None);
    assert_eq!(
        book.format_at(SHEET, "H20").expect("format").border_top,
        Some(BorderLine::none())
    );
}

#[test]
fn one_edge_keeps_the_others_and_undo_takes_it_back() {
    let mut book = HeadlessWorkbook::open(&source()).expect("open");
    let before = book.bytes().to_vec();
    let change = FormatChange {
        border_top: Some(line(BorderStyle::Thick, Some("000000"))),
        border_left: Some(BorderLine::none()),
        ..FormatChange::default()
    };
    book.set_format(SHEET, "A5", &change).expect("format");
    let format = book.format_at(SHEET, "A5").expect("format");
    assert_eq!(format.border_top, change.border_top);
    assert_eq!(format.border_left, Some(BorderLine::none()));
    assert_eq!(
        format.border_right,
        Some(line(BorderStyle::Thin, Some("8EA9DB")))
    );
    // A5's value and its other attributes stayed.
    assert_eq!(
        book.cell(SHEET, "A5").expect("cell").text.as_deref(),
        Some("名古屋")
    );

    assert!(book.undo().expect("undo"));
    assert_eq!(book.bytes(), before);
}

#[test]
fn complete_columns_take_a_border_for_future_cells_too() {
    let mut book = HeadlessWorkbook::open(&source()).expect("open");
    let change = FormatChange {
        border_bottom: Some(line(BorderStyle::Hair, Some("808080"))),
        ..FormatChange::default()
    };
    book.set_format(SHEET, "F:F", &change).expect("format");
    let reopened = HeadlessWorkbook::open(&book.export_bytes().expect("export")).expect("reopen");
    assert_eq!(
        reopened
            .format_at(SHEET, "F500")
            .expect("format")
            .border_bottom,
        change.border_bottom
    );
}

#[test]
fn a_border_with_a_bad_colour_is_refused_and_changes_nothing() {
    let mut book = HeadlessWorkbook::open(&source()).expect("open");
    let before = book.bytes().to_vec();
    let change = FormatChange {
        border_left: Some(line(BorderStyle::Thin, Some("not a colour"))),
        ..FormatChange::default()
    };
    assert!(book.set_format(SHEET, "B2", &change).is_err());
    assert_eq!(book.bytes(), before);
}
