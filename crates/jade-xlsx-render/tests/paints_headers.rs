//! The row and column bands.

use std::rc::Rc;

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{Cell, CellRef, CellValue, FreezePane, Row, Sheet, StyleTable};
use jade_xlsx_render::{render_grid, render_viewport, GridSelection, RenderOptions};

fn laid_out(rows: u32) -> SheetLayout {
    laid_out_with_freeze(rows, None)
}

fn laid_out_with_freeze(rows: u32, freeze_pane: Option<FreezePane>) -> SheetLayout {
    let sheet = Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: (0..rows)
            .map(|index| Row {
                index,
                height_points: None,
                custom_height: false,
                custom_format: false,
                hidden: false,
                style_index: None,
                cells: vec![Cell {
                    // Text rather than a number, so every digit in the drawn
                    // output came from a header rather than from a cell.
                    reference: CellRef::new(0, index),
                    value: CellValue::Text(if index == 0 {
                        "frozen".into()
                    } else {
                        "x".into()
                    }),
                    style_index: None,
                    formula: None,
                }],
            })
            .collect(),
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
        freeze_pane,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        protection: None,
    };
    layout_sheet(
        &sheet,
        &StyleTable::default(),
        &LayoutOptions {
            min_column_count: 6,
            measure: Rc::new(jade_xlsx_layout::HeuristicMeasure),
            ..LayoutOptions::default()
        },
    )
}

fn paint(
    layout: &SheetLayout,
    viewport: Rect,
    selected: Option<GridSelection>,
) -> RecordingPainter {
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_grid(
        &painter,
        layout,
        viewport,
        selected,
        &RenderOptions::default(),
    )
    .expect("render");
    painter
}

fn drawn_text(painter: &RecordingPainter) -> Vec<String> {
    painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::FillText(text, _, _) => Some(text),
            _ => None,
        })
        .collect()
}

const WINDOW: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 300.0,
    height: 120.0,
};

#[test]
fn the_bands_name_the_columns_and_rows_on_screen() {
    let painter = paint(&laid_out(20), WINDOW, None);
    let text = drawn_text(&painter);

    // 64 px columns across 300 px, 20 px rows down 120 px.
    assert!(text.contains(&"A".to_string()));
    assert!(text.contains(&"E".to_string()));
    assert!(text.contains(&"1".to_string()));
    assert!(text.contains(&"6".to_string()));
    // Row numbers are one-based; row 0 is called 1 and there is no 0.
    assert!(!text.contains(&"0".to_string()));
}

#[test]
fn a_scrolled_sheet_names_the_rows_it_is_showing() {
    let layout = laid_out(1_000);
    let painter = paint(
        &layout,
        Rect {
            y: 4_000.0,
            ..WINDOW
        },
        None,
    );
    let text = drawn_text(&painter);

    // 20 px rows: 4000 px down is row 201.
    assert!(text.contains(&"201".to_string()), "got {text:?}");
    assert!(!text.contains(&"1".to_string()));
}

#[test]
fn frozen_rows_and_columns_stay_in_the_grid_after_scrolling() {
    let layout = laid_out_with_freeze(
        1_000,
        Some(FreezePane {
            frozen_columns: 1,
            frozen_rows: 1,
            top_left_cell: Some(CellRef::new(1, 1)),
        }),
    );
    let painter = paint(
        &layout,
        Rect {
            x: 128.0,
            y: 4_000.0,
            ..WINDOW
        },
        None,
    );
    let text = drawn_text(&painter);

    assert!(text.contains(&"A".to_string()), "got {text:?}");
    assert!(text.contains(&"1".to_string()), "got {text:?}");
    assert!(text.contains(&"frozen".to_string()), "got {text:?}");
    assert!(text.contains(&"202".to_string()), "got {text:?}");
    assert!(!text.contains(&"201".to_string()), "got {text:?}");
}

#[test]
fn the_selected_row_and_column_are_highlighted() {
    let plain = paint(&laid_out(20), WINDOW, None);
    let selected = paint(
        &laid_out(20),
        WINDOW,
        Some(GridSelection::cell(CellRef::new(2, 3))),
    );

    let highlights = |painter: &RecordingPainter| {
        painter.count(|op| matches!(op, Op::SetFillStyle(color) if color == "#dbe4f0"))
    };
    assert_eq!(highlights(&plain), 0);
    // One for the column band, one for the row band.
    assert_eq!(highlights(&selected), 2);
}

#[test]
fn the_body_sits_inside_the_bands() {
    let layout = laid_out(20);
    let painter = paint(&layout, WINDOW, None);
    let metrics = layout.header_metrics();

    // The body is translated past the bands before anything of the sheet is
    // drawn, which is what keeps a cell from painting over a row number.
    assert!(painter.ops().iter().any(|op| matches!(
        op,
        Op::Translate(x, y) if *x == metrics.row_width && *y == metrics.column_height
    )));
}

#[test]
fn the_body_is_clipped_so_a_scrolled_sheet_cannot_reach_the_bands() {
    let layout = laid_out(1_000);
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_viewport(&painter, &layout, WINDOW, &RenderOptions::default()).expect("render");

    let ops = painter.ops();
    let rect = ops
        .iter()
        .position(|op| {
            matches!(op, Op::Rect(x, y, width, height)
                if *x == 0.0 && *y == 0.0 && *width == WINDOW.width && *height == WINDOW.height)
        })
        .expect("the body's own clip rectangle");
    assert_eq!(ops[rect + 1], Op::Clip);
}

#[test]
fn a_sheet_drawn_without_headers_is_unchanged() {
    // `render_viewport` is still the plain body, so the PNG path and the tests
    // that assert draw calls are not paying for bands they do not want.
    let layout = laid_out(20);
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_viewport(&painter, &layout, WINDOW, &RenderOptions::default()).expect("render");

    let text = drawn_text(&painter);
    assert!(!text.contains(&"A".to_string()));
}
