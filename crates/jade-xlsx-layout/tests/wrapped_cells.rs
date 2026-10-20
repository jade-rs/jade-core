//! Wrapped text and the rows that grow to hold it.

use std::rc::Rc;

use jade_xlsx_layout::{
    layout_sheet, units, LayoutOptions, ResolvedFont, SheetLayout, TextMeasure,
};
use jade_xlsx_reader::{
    Alignment, Cell, CellFormat, CellRange, CellRef, CellValue, ColumnSpan, Font,
    HorizontalAlignment, Row, Sheet, StyleTable, VerticalAlignment,
};

/// Seven px per ASCII character and fourteen per anything else, whatever the
/// font size. Those are the proportions of an 11 pt face, so the column widths
/// below behave like real ones, and the line ends stay arithmetic rather than
/// font-dependent.
struct Monospace;

impl TextMeasure for Monospace {
    fn measure(&self, text: &str, _font: &ResolvedFont) -> f64 {
        text.chars()
            .map(|c| if c.is_ascii() { 7.0 } else { 14.0 })
            .sum()
    }
}

struct ScaledMonospace;

impl TextMeasure for ScaledMonospace {
    fn measure(&self, text: &str, font: &ResolvedFont) -> f64 {
        text.chars().count() as f64 * 7.0 * f64::from(font.size_points / 11.0)
    }
}

fn wrapping_style() -> StyleTable {
    let mut styles = StyleTable::default();
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            wrap_text: true,
            ..Alignment::default()
        },
        ..Default::default()
    });
    // 1: wrapping, vertically centred.
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            wrap_text: true,
            vertical: VerticalAlignment::Center,
            horizontal: HorizontalAlignment::Center,
            ..Alignment::default()
        },
        ..Default::default()
    });
    styles
}

fn sheet_with(text: &str, style_index: Option<u32>, width_chars: f32) -> Sheet {
    Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: vec![ColumnSpan {
            first: 0,
            last: 0,
            width_chars: Some(width_chars),
            custom_width: true,
            hidden: false,
            style_index: None,
        }],
        rows: vec![Row {
            index: 0,
            height_points: None,
            custom_height: false,
            custom_format: false,
            hidden: false,
            style_index: None,
            cells: vec![Cell {
                reference: CellRef::new(0, 0),
                value: CellValue::Text(text.into()),
                style_index,
                formula: None,
            }],
        }],
        merges: Vec::new(),
        hyperlinks: Vec::new(),
        comments: Vec::new(),
        data_validations: Vec::new(),
        conditional_formats: Vec::new(),
        auto_filter: None,
        filter_mode: None,
        filter_row_state: None,
        default_column_width_chars: None,
        default_row_height_points: Some(15.0),
        default_row_custom_height: false,
        default_rows_hidden: false,
        show_grid_lines: true,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        protection: None,
    }
}

fn laid_out(sheet: &Sheet, styles: &StyleTable) -> SheetLayout {
    layout_sheet(
        sheet,
        styles,
        &LayoutOptions {
            measure: Rc::new(Monospace),
            ..LayoutOptions::default()
        },
    )
}

#[test]
fn a_cell_without_wrap_text_stays_on_one_line() {
    let sheet = sheet_with("a very long piece of text", None, 5.0);
    let layout = laid_out(&sheet, &StyleTable::default());

    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    assert_eq!(cell.lines, ["a very long piece of text"]);
    // And the row keeps the sheet default.
    assert_eq!(layout.row(0).expect("row").size, 20.0);
}

#[test]
fn a_line_break_without_wrap_text_is_not_drawn() {
    // Excel keeps the cell on one line and shows its line breaks as
    // nothing — not as a box for a character the font has no glyph for.
    for text in ["two\nlines", "two\r\nlines"] {
        let sheet = sheet_with(text, None, 20.0);
        let layout = laid_out(&sheet, &StyleTable::default());
        assert_eq!(layout.cell(CellRef::new(0, 0)).unwrap().lines, ["twolines"]);
    }
    // With wrap text, the break is where the line breaks.
    let sheet = sheet_with("two\nlines", Some(0), 20.0);
    let layout = laid_out(&sheet, &wrapping_style());
    assert_eq!(
        layout.cell(CellRef::new(0, 0)).unwrap().lines,
        ["two", "lines"]
    );
}

#[test]
fn shrink_to_fit_reduces_the_shared_layout_font() {
    let mut styles = StyleTable::default();
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            shrink_to_fit: true,
            ..Alignment::default()
        },
        ..Default::default()
    });
    let sheet = sheet_with("a very long piece of text", Some(0), 5.0);
    let layout = layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            measure: Rc::new(ScaledMonospace),
            ..LayoutOptions::default()
        },
    );

    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    assert!(cell.font.size_points < 11.0);
    assert_eq!(cell.lines, ["a very long piece of text"]);
    assert!(
        ScaledMonospace.measure(&cell.text, &cell.font)
            <= cell.rect.width - layout.cell_padding_px * 2.0 + 0.01
    );
}

#[test]
fn indent_uses_three_normal_style_spaces_and_reduces_the_text_width() {
    let mut styles = StyleTable::default();
    styles.fonts.push(Font {
        size_points: Some(22.0),
        ..Font::default()
    });
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            horizontal: HorizontalAlignment::Left,
            indent: 2,
            wrap_text: true,
            ..Alignment::default()
        },
        ..Default::default()
    });
    let sheet = sheet_with("abcdefgh", Some(0), 10.0);
    let layout = layout_sheet(
        &sheet,
        &styles,
        &LayoutOptions {
            measure: Rc::new(ScaledMonospace),
            ..LayoutOptions::default()
        },
    );

    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    assert_eq!(cell.text_inset_px, 84.0);
    assert_eq!(cell.lines.len(), 8);
}

#[test]
fn vertically_stacked_text_uses_graphemes_and_grows_the_row() {
    let mut styles = StyleTable::default();
    styles.cell_formats.push(CellFormat {
        alignment: Alignment {
            text_rotation: 255,
            ..Alignment::default()
        },
        ..Default::default()
    });
    let sheet = sheet_with("e\u{301}XZ", Some(0), 10.0);
    let layout = laid_out(&sheet, &styles);

    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    assert_eq!(cell.lines, ["e\u{301}", "X", "Z"]);
    assert_eq!(
        layout.row(0).expect("row").size,
        f64::from(units::line_height_points(11.0)) * 3.0 * 96.0 / 72.0
    );
}

#[test]
fn shrink_to_fit_from_xlsx_style_reaches_layout() {
    let workbook = jade_xlsx_reader::read_xlsx(include_bytes!(
        "../../../public/fixtures/number-formats-workbook.xlsx"
    ))
    .expect("fixture");
    let layout = layout_sheet(
        &workbook.sheets[0],
        &workbook.styles,
        &LayoutOptions::default(),
    );
    let normal = layout
        .cell(CellRef::parse("B14").expect("B14"))
        .expect("B14");
    let shrunk = layout
        .cell(CellRef::parse("C14").expect("C14"))
        .expect("C14");

    assert_eq!(normal.text, shrunk.text);
    assert!(shrunk.alignment.shrink_to_fit);
    assert!(shrunk.font.size_points < normal.font.size_points);
}

#[test]
fn wrap_text_breaks_against_the_column_width() {
    // 10 characters wide: 10 * 7 + 5 = 75 px, less 3 px padding a side = 69.
    let sheet = sheet_with("hello there world", Some(0), 10.0);
    let layout = laid_out(&sheet, &wrapping_style());

    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    assert!(
        cell.lines.len() > 1,
        "expected a wrap, got {:?}",
        cell.lines
    );
    assert_eq!(cell.lines.concat().replace(' ', ""), "hellothereworld");
    // The full text is kept alongside the lines.
    assert_eq!(cell.text, "hello there world");
}

#[test]
fn a_wrapped_row_grows_to_hold_its_lines() {
    let sheet = sheet_with("ああああああああああああ", Some(0), 4.0);
    let layout = laid_out(&sheet, &wrapping_style());

    let cell = layout.cell(CellRef::new(0, 0)).expect("A1");
    let lines = cell.lines.len();
    assert!(lines > 1);

    let expected = units::points_to_px(units::line_height_points(11.0) * lines as f32);
    assert_eq!(layout.row(0).expect("row").size, expected);
    assert_eq!(cell.rect.height, expected);
}

#[test]
fn a_row_whose_height_the_user_pinned_does_not_grow() {
    let mut sheet = sheet_with("ああああああああああああ", Some(0), 4.0);
    sheet.rows[0].height_points = Some(15.0);
    sheet.rows[0].custom_height = true;

    let layout = laid_out(&sheet, &wrapping_style());

    // Excel stops auto-fitting once the height is set by hand, even though the
    // text no longer fits. The wrap still happens; the row just clips.
    assert_eq!(layout.row(0).expect("row").size, 20.0);
    assert!(layout.cell(CellRef::new(0, 0)).expect("A1").lines.len() > 1);
}

#[test]
fn a_merged_cell_wraps_across_its_span_but_does_not_grow_the_row() {
    let mut sheet = sheet_with("hello there world and more", Some(0), 10.0);
    sheet.columns.push(ColumnSpan {
        first: 1,
        last: 1,
        width_chars: Some(10.0),
        custom_width: true,
        hidden: false,
        style_index: None,
    });
    sheet.merges = vec![CellRange::parse("A1:B1").expect("range")];

    let narrow = laid_out(
        &sheet_with("hello there world and more", Some(0), 10.0),
        &wrapping_style(),
    );
    let merged = laid_out(&sheet, &wrapping_style());

    let narrow_lines = narrow.cell(CellRef::new(0, 0)).expect("A1").lines.len();
    let merged_lines = merged.cell(CellRef::new(0, 0)).expect("A1").lines.len();
    assert!(
        merged_lines < narrow_lines,
        "the merged cell has two columns to wrap into"
    );

    // Excel does not auto-fit a row to a merged cell: the height would have to
    // be shared between the merged rows, and it leaves that to the user.
    assert_eq!(merged.row(0).expect("row").size, 20.0);
}

#[test]
fn wrapping_uses_the_measurer_it_is_given() {
    struct Wide;
    impl TextMeasure for Wide {
        fn measure(&self, text: &str, _font: &ResolvedFont) -> f64 {
            text.chars().count() as f64 * 40.0
        }
    }

    let sheet = sheet_with("abcdef", Some(0), 10.0);
    let narrow = layout_sheet(
        &sheet,
        &wrapping_style(),
        &LayoutOptions {
            measure: Rc::new(Wide),
            ..LayoutOptions::default()
        },
    );
    let roomy = laid_out(&sheet, &wrapping_style());

    // Same text, same column: only the measurement differs.
    assert_eq!(roomy.cell(CellRef::new(0, 0)).expect("A1").lines.len(), 1);
    assert_eq!(narrow.cell(CellRef::new(0, 0)).expect("A1").lines.len(), 6);
}
