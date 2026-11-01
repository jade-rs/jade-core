//! What a number format decides beyond the text: the colour, and which epoch
//! a date serial counts from.

use jade_xlsx_layout::{layout_sheet, LayoutOptions};
use jade_xlsx_reader::{
    Cell, CellFormat, CellRef, CellValue, Color, ColumnSpan, DateSystem, Font, Row, Sheet,
    StyleTable,
};

/// A style table whose one cell format is `code`, over a blue font.
fn styles(code: &str) -> StyleTable {
    let mut styles = StyleTable::default();
    styles.fonts.push(Font {
        color: Some(Color::Argb([0xFF, 0x00, 0x00, 0xFF])),
        ..Font::default()
    });
    styles.number_formats.insert(200, code.to_string());
    styles.cell_formats.push(CellFormat {
        font_index: Some(0),
        number_format_id: Some(200),
        ..Default::default()
    });
    styles
}

fn sheet_with(value: CellValue) -> Sheet {
    Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: vec![Row {
            index: 0,
            height_points: None,
            custom_height: false,
            custom_format: false,
            hidden: false,
            style_index: None,
            cells: vec![Cell {
                reference: CellRef::new(0, 0),
                value,
                style_index: Some(0),
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
        show_formulas: false,
        show_zeros: true,
        show_headers: true,
        grid_color: None,
        zoom_percent: None,
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        rich_text: Default::default(),
        tab_color: None,
        sparkline_groups: Vec::new(),
        ignored_errors: Vec::new(),
        outline: Default::default(),
        protection: None,
    }
}

#[test]
fn a_cell_without_its_own_style_uses_its_column_style() {
    let mut sheet = sheet_with(CellValue::Number(1234.0));
    sheet.rows[0].cells[0].style_index = None;
    sheet.columns.push(ColumnSpan {
        first: 0,
        last: 0,
        width_chars: None,
        custom_width: false,
        hidden: false,
        style_index: Some(0),
    });

    let layout = layout_sheet(&sheet, &styles("#,##0\"円\""), &LayoutOptions::default());
    assert_eq!(layout.cell(CellRef::new(0, 0)).expect("A1").text, "1,234円");
}

fn cell(value: CellValue, code: &str, dates: DateSystem) -> (String, Option<Color>) {
    let sheet = sheet_with(value);
    let layout = layout_sheet(
        &sheet,
        &styles(code),
        &LayoutOptions {
            date_system: dates,
            ..LayoutOptions::default()
        },
    );
    let placed = layout.cell(CellRef::new(0, 0)).expect("A1");
    (placed.text.clone(), placed.text_color)
}

#[test]
fn a_colour_in_the_format_wins_over_the_font_colour() {
    let (text, color) = cell(
        CellValue::Number(-1234.0),
        "#,##0;[Red]▲#,##0",
        DateSystem::Nineteen00,
    );
    assert_eq!(text, "▲1,234");
    assert_eq!(color, Some(Color::Argb([0xFF, 0xFF, 0x00, 0x00])));
}

#[test]
fn a_format_that_names_no_colour_leaves_the_font_alone() {
    let (text, color) = cell(
        CellValue::Number(1234.0),
        "#,##0;[Red]▲#,##0",
        DateSystem::Nineteen00,
    );
    assert_eq!(text, "1,234");
    assert_eq!(color, Some(Color::Argb([0xFF, 0x00, 0x00, 0xFF])));
}

#[test]
fn the_workbooks_epoch_reaches_the_formatter() {
    let (windows, _) = cell(
        CellValue::Number(45000.0),
        "yyyy-mm-dd",
        DateSystem::Nineteen00,
    );
    let (mac, _) = cell(
        CellValue::Number(45000.0),
        "yyyy-mm-dd",
        DateSystem::Nineteen04,
    );
    assert_eq!(windows, "2023-03-15");
    assert_eq!(mac, "2027-03-16");
}

#[test]
fn show_formulas_shows_a_formula_as_typed_on_the_left() {
    let mut sheet = sheet_with(CellValue::Number(1234.0));
    sheet.rows[0].cells[0].formula = Some("_xlfn.XLOOKUP(B1,C:C,D:D)".into());
    let shown = |sheet: &Sheet| {
        let layout = layout_sheet(sheet, &styles("#,##0"), &LayoutOptions::default());
        layout.cell(CellRef::new(0, 0)).expect("A1").text.clone()
    };
    assert_eq!(shown(&sheet), "1,234");
    sheet.show_formulas = true;
    assert_eq!(shown(&sheet), "=XLOOKUP(B1,C:C,D:D)");
    let layout = layout_sheet(&sheet, &styles("#,##0"), &LayoutOptions::default());
    assert_eq!(
        layout
            .cell(CellRef::new(0, 0))
            .expect("A1")
            .alignment
            .horizontal,
        jade_xlsx_reader::HorizontalAlignment::Left
    );
}

#[test]
fn error_checking_flags_numbers_stored_as_text_and_formula_errors() {
    let flagged = |value: CellValue, formula: Option<&str>| {
        let mut sheet = sheet_with(value);
        sheet.rows[0].cells[0].formula = formula.map(str::to_string);
        let layout = layout_sheet(&sheet, &styles("General"), &LayoutOptions::default());
        layout.cell(CellRef::new(0, 0)).expect("A1").error_flag
    };
    assert!(flagged(CellValue::Text("00123".into()), None));
    assert!(flagged(CellValue::Text(" 1,234.5 ".into()), None));
    assert!(!flagged(CellValue::Text("abc".into()), None));
    assert!(!flagged(CellValue::Text("A1".into()), None));
    assert!(!flagged(CellValue::Number(123.0), None));
    assert!(flagged(CellValue::Error("#DIV/0!".into()), Some("1/0")));
    assert!(!flagged(CellValue::Error("#N/A".into()), None));
}

#[test]
fn ignored_errors_turn_off_the_check_they_name() {
    use jade_xlsx_reader::ignored_errors::IgnoredError;
    let flagged = |value: CellValue, formula: Option<&str>, ignored: IgnoredError| {
        let mut sheet = sheet_with(value);
        sheet.rows[0].cells[0].formula = formula.map(str::to_string);
        sheet.ignored_errors = vec![ignored];
        let layout = layout_sheet(&sheet, &styles("General"), &LayoutOptions::default());
        layout.cell(CellRef::new(0, 0)).expect("A1").error_flag
    };
    let a1 = vec![jade_xlsx_reader::CellRange::parse("A1").unwrap()];
    let text = || CellValue::Text("00123".into());
    let error = || CellValue::Error("#DIV/0!".into());
    let numbers = IgnoredError {
        ranges: a1.clone(),
        number_stored_as_text: true,
        ..Default::default()
    };
    let errors = IgnoredError {
        ranges: a1,
        eval_error: true,
        ..Default::default()
    };
    assert!(!flagged(text(), None, numbers.clone()));
    assert!(flagged(text(), None, errors.clone()));
    assert!(!flagged(error(), Some("1/0"), errors));
    assert!(flagged(error(), Some("1/0"), numbers.clone()));
    let elsewhere = IgnoredError {
        ranges: vec![jade_xlsx_reader::CellRange::parse("B2:C3").unwrap()],
        ..numbers
    };
    assert!(flagged(text(), None, elsewhere));
}

#[test]
fn a_sheet_that_hides_zeros_shows_cells_worth_zero_as_empty() {
    let shown = |value: CellValue, formula: Option<&str>, show_zeros: bool, show_formulas: bool| {
        let mut sheet = sheet_with(value);
        sheet.rows[0].cells[0].formula = formula.map(str::to_string);
        sheet.show_zeros = show_zeros;
        sheet.show_formulas = show_formulas;
        let layout = layout_sheet(&sheet, &styles("#,##0.00"), &LayoutOptions::default());
        layout.cell(CellRef::new(0, 0)).expect("A1").text.clone()
    };
    assert_eq!(shown(CellValue::Number(0.0), None, true, false), "0.00");
    assert_eq!(shown(CellValue::Number(0.0), None, false, false), "");
    assert_eq!(
        shown(CellValue::Number(0.0), Some("B1-C1"), false, false),
        ""
    );
    assert_eq!(shown(CellValue::Number(1.5), None, false, false), "1.50");
    assert_eq!(shown(CellValue::Text("0".into()), None, false, false), "0");
    assert_eq!(
        shown(CellValue::Number(0.0), Some("B1-C1"), false, true),
        "=B1-C1"
    );
}

#[test]
fn a_formats_numbered_colour_is_the_palettes() {
    let layout = layout_sheet(
        &sheet_with(CellValue::Number(5.0)),
        &styles("[Color10]0"),
        &LayoutOptions::default(),
    );
    let color = layout.cell(CellRef::new(0, 0)).expect("A1").text_color;
    assert_eq!(
        color.map(|color| jade_xlsx_layout::color::to_css(color, "?")),
        Some("#008000".to_string())
    );
}
