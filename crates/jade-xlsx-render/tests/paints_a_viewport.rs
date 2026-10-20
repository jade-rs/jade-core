//! Painting a screenful of a big sheet.
//!
//! The assertions are about what is *not* drawn as much as what is: a sheet
//! with ten thousand rows has to cost a screenful per frame, or the browser
//! path is unusable on a real workbook.

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{Cell, CellRef, CellValue, Row, Sheet, StyleTable};
use jade_xlsx_render::{render_sheet, render_viewport, RenderOptions};

fn tall_sheet(rows: u32) -> Sheet {
    Sheet {
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
                    reference: CellRef::new(0, index),
                    value: CellValue::Number(f64::from(index)),
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
        freeze_pane: None,
        tables: Vec::new(),
        header_footer: None,
        row_breaks: Vec::new(),
        column_breaks: Vec::new(),
        page_setup: Default::default(),
        shapes: Vec::new(),
        array_formulas: Vec::new(),
        phonetic_readings: Default::default(),
        protection: None,
    }
}

fn laid_out(rows: u32) -> SheetLayout {
    layout_sheet(
        &tall_sheet(rows),
        &StyleTable::default(),
        &LayoutOptions::default(),
    )
}

fn paint(layout: &SheetLayout, viewport: Rect) -> RecordingPainter {
    let painter = RecordingPainter::with_width_per_char(10.0);
    render_viewport(&painter, layout, viewport, &RenderOptions::default()).expect("render");
    painter
}

#[test]
fn only_the_cells_under_the_viewport_are_drawn() {
    let layout = laid_out(10_000);
    let painter = paint(
        &layout,
        Rect {
            x: 0.0,
            y: 4_000.0,
            width: 64.0,
            height: 100.0,
        },
    );

    let drawn: Vec<String> = painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::FillText(text, _, _) => Some(text),
            _ => None,
        })
        .collect();
    // 20 px rows: the viewport covers rows 200..204, whose values are 200..204.
    assert_eq!(drawn, ["200", "201", "202", "203", "204"]);
}

#[test]
fn the_cost_of_a_screenful_does_not_grow_with_the_sheet() {
    // The property that makes a real workbook drawable: painting the same
    // window costs the same whether the sheet has a hundred rows or ten
    // thousand. An absolute operation count would only pin today's per-cell
    // sequence; this pins the thing that matters.
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        width: 64.0,
        height: 600.0,
    };
    let small = paint(&laid_out(100), viewport).ops().len();
    let large = paint(&laid_out(10_000), viewport).ops().len();

    assert_eq!(small, large);
    // And it really is a screenful: 600 px of 20 px rows.
    assert_eq!(
        paint(&laid_out(10_000), viewport).count(|op| matches!(op, Op::FillText(..))),
        30
    );
}

#[test]
fn the_viewport_origin_becomes_the_surfaces_origin() {
    let layout = laid_out(1_000);
    let painter = paint(
        &layout,
        Rect {
            x: 0.0,
            y: 4_000.0,
            width: 64.0,
            height: 100.0,
        },
    );

    // Sheet coordinates go in; the scroll offset comes back out as a translate,
    // so drawing code never has to know where the viewport is.
    assert!(painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::Translate(x, y) if *x == 0.0 && *y == -4_000.0)));

    let first_text_y = painter
        .ops()
        .into_iter()
        .find_map(|op| match op {
            Op::FillText(_, _, y) => Some(y),
            _ => None,
        })
        .expect("text");
    // Still in sheet coordinates: row 200 starts at y = 4000.
    assert!(first_text_y > 4_000.0 && first_text_y < 4_020.0);
}

#[test]
fn the_background_covers_the_viewport_rather_than_the_sheet() {
    let layout = laid_out(10_000);
    let viewport = Rect {
        x: 0.0,
        y: 4_000.0,
        width: 64.0,
        height: 100.0,
    };
    let painter = paint(&layout, viewport);

    assert_eq!(
        painter.ops().first(),
        Some(&Op::Save),
        "the painter state must be scoped"
    );
    assert!(painter.ops().iter().any(|op| matches!(
        op,
        Op::FillRect(x, y, width, height)
            if *x == viewport.x && *y == viewport.y && *width == viewport.width && *height == viewport.height
    )));
}

#[test]
fn rendering_the_whole_sheet_is_the_full_viewport() {
    // `render_sheet` is `render_viewport` over the content rectangle; the two
    // must not be able to drift into different drawing code.
    let layout = laid_out(20);

    let whole = RecordingPainter::with_width_per_char(10.0);
    render_sheet(&whole, &layout, &RenderOptions::default()).expect("render");

    let viewport = paint(
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: layout.content_width,
            height: layout.content_height,
        },
    );

    assert_eq!(whole.ops(), viewport.ops());
}
