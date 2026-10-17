//! The selected range in the sheet body: a translucent wash over every cell
//! but the active one, and a border around the whole range — the way Excel
//! shows that the active cell is inside the selection, not on top of it.

use std::rc::Rc;

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect, SheetLayout};
use jade_xlsx_reader::{Cell, CellRef, CellValue, Row, Sheet, StyleTable};
use jade_xlsx_render::{render_grid, GridSelection, RenderOptions};

const WINDOW: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 400.0,
    height: 300.0,
};

fn laid_out(rows: u32) -> SheetLayout {
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
        shapes: Vec::new(),
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

fn paint(selected: Option<GridSelection>) -> RecordingPainter {
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_grid(
        &painter,
        &laid_out(20),
        WINDOW,
        selected,
        &RenderOptions::default(),
    )
    .expect("render");
    painter
}

fn selection_fills(painter: &RecordingPainter) -> usize {
    let options = RenderOptions::default();
    let mut washing = false;
    let mut fills = 0;
    for op in painter.ops() {
        match op {
            Op::SetFillStyle(color) => washing = color == options.selection_fill,
            Op::FillRect(..) if washing => fills += 1,
            _ => {}
        }
    }
    fills
}

fn selection_borders(painter: &RecordingPainter) -> usize {
    let options = RenderOptions::default();
    let mut selected = false;
    let mut strokes = 0;
    for op in painter.ops() {
        match op {
            Op::SetStrokeStyle(color) => selected = color == options.selection_border,
            Op::StrokeRect(..) if selected => strokes += 1,
            _ => {}
        }
    }
    strokes
}

#[test]
fn a_range_washes_every_cell_but_the_active_one_and_draws_one_border() {
    let selected = paint(Some(GridSelection::range(
        CellRef::new(1, 1),
        CellRef::new(2, 2),
    )));

    // B2:C3 with B2 active: the wash covers the rows below the active one in
    // one strip and the cells beside it in another — two rectangles, neither
    // of them over B2 — plus one border around the whole range.
    assert_eq!(selection_fills(&selected), 2);
    assert_eq!(selection_borders(&selected), 1);
}

#[test]
fn a_single_cell_gets_the_border_and_no_wash() {
    let selected = paint(Some(GridSelection::cell(CellRef::new(2, 3))));
    assert_eq!(selection_fills(&selected), 0);
    assert_eq!(selection_borders(&selected), 1);

    let none = paint(None);
    assert_eq!(selection_fills(&none), 0);
    assert_eq!(selection_borders(&none), 0);
}

#[test]
fn every_row_and_column_the_range_touches_highlights_its_band() {
    let highlights = |painter: &RecordingPainter| {
        painter.count(|op| matches!(op, Op::SetFillStyle(color) if color == "#dbe4f0"))
    };

    // Two columns and two rows: four band highlights.
    let ranged = paint(Some(GridSelection::range(
        CellRef::new(1, 1),
        CellRef::new(2, 2),
    )));
    assert_eq!(highlights(&ranged), 4);

    let single = paint(Some(GridSelection::cell(CellRef::new(2, 3))));
    assert_eq!(highlights(&single), 2);
}

#[test]
fn a_backwards_drag_normalises_to_the_same_range() {
    // Dragging up-and-left from C3 to B2 selects the same rectangle, with the
    // anchor cell C3 active.
    let selection = GridSelection::range(CellRef::new(2, 2), CellRef::new(1, 1));
    assert_eq!(selection.range.start, CellRef::new(1, 1));
    assert_eq!(selection.range.end, CellRef::new(2, 2));
    assert_eq!(selection.active, CellRef::new(2, 2));

    let painted = paint(Some(selection));
    assert_eq!(selection_fills(&painted), 2);
    assert_eq!(selection_borders(&painted), 1);
}

#[test]
fn selecting_a_merged_cell_borders_the_whole_merge_and_its_bands() {
    use jade_xlsx_reader::CellRange;

    // The same sheet with B2:C3 merged; clicking anywhere in it selects the
    // anchor B2, and the selection has to read as the merged rectangle.
    let mut sheet = Sheet {
        name: "Sheet1".into(),
        part_name: "xl/worksheets/sheet1.xml".into(),
        visibility: jade_xlsx_reader::SheetVisibility::Visible,
        columns: Vec::new(),
        rows: (0..8)
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
        merges: vec![CellRange {
            start: CellRef::new(1, 1),
            end: CellRef::new(2, 2),
        }],
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
    };
    sheet.rows[1].cells.push(Cell {
        reference: CellRef::new(1, 1),
        value: CellValue::Text("merged".into()),
        style_index: None,
        formula: None,
    });
    let layout = layout_sheet(
        &sheet,
        &StyleTable::default(),
        &LayoutOptions {
            min_column_count: 6,
            measure: Rc::new(jade_xlsx_layout::HeuristicMeasure),
            ..LayoutOptions::default()
        },
    );

    let painter = RecordingPainter::with_width_per_char(7.0);
    render_grid(
        &painter,
        &layout,
        WINDOW,
        Some(GridSelection::cell(CellRef::new(1, 1))),
        &RenderOptions::default(),
    )
    .expect("render");

    // One border around the merged rectangle — its size, not the anchor's.
    let options = RenderOptions::default();
    let expected_width = layout.column(1).expect("B").size + layout.column(2).expect("C").size;
    let expected_height = layout.row(1).expect("2").size + layout.row(2).expect("3").size;
    let mut selected = false;
    let mut borders = Vec::new();
    for op in painter.ops() {
        match op {
            Op::SetStrokeStyle(color) => selected = color == options.selection_border,
            Op::StrokeRect(_, _, width, height) if selected => borders.push((width, height)),
            _ => {}
        }
    }
    assert_eq!(borders, vec![(expected_width, expected_height)]);

    // A merged single cell is one surface: border only, no wash.
    let mut washing = false;
    let mut fills = 0;
    for op in painter.ops() {
        match op {
            Op::SetFillStyle(color) => washing = color == options.selection_fill,
            Op::FillRect(..) if washing => fills += 1,
            _ => {}
        }
    }
    assert_eq!(fills, 0);

    // Every row and column under the merge highlights its band: B, C, 2, 3.
    let highlights =
        painter.count(|op| matches!(op, Op::SetFillStyle(color) if color == "#dbe4f0"));
    assert_eq!(highlights, 4);
}

#[test]
fn the_selection_carries_a_fill_handle_at_its_bottom_right_corner() {
    let layout = laid_out(20);
    let painter = RecordingPainter::with_width_per_char(7.0);
    render_grid(
        &painter,
        &layout,
        WINDOW,
        Some(GridSelection::range(CellRef::new(1, 1), CellRef::new(2, 2))),
        &RenderOptions::default(),
    )
    .expect("render");

    // The handle is a small square in the border colour, centred on the
    // corner where the range's right and bottom edges meet.
    let options = RenderOptions::default();
    let right = layout.column(2).expect("C").offset + layout.column(2).expect("C").size;
    let bottom = layout.row(2).expect("3").offset + layout.row(2).expect("3").size;
    let mut in_handle_colour = false;
    let mut handles = Vec::new();
    for op in painter.ops() {
        match op {
            Op::SetFillStyle(color) => in_handle_colour = color == options.selection_border,
            Op::FillRect(x, y, width, height) if in_handle_colour => {
                handles.push((x, y, width, height));
            }
            _ => {}
        }
    }
    assert_eq!(handles.len(), 1, "{handles:?}");
    let (x, y, width, height) = handles[0];
    assert_eq!(width, height);
    assert!((x + width / 2.0 - right).abs() < 0.01, "{x} vs {right}");
    assert!((y + height / 2.0 - bottom).abs() < 0.01, "{y} vs {bottom}");

    // No selection, no handle.
    let none = paint(None);
    assert_eq!(
        none.count(
            |op| matches!(op, Op::SetFillStyle(color) if *color == options.selection_border)
        ),
        0
    );
}

#[test]
fn a_fill_preview_is_a_dashed_outline_around_what_the_drop_would_fill() {
    use jade_xlsx_reader::CellRange;

    let layout = laid_out(20);
    let painter = RecordingPainter::with_width_per_char(7.0);
    // B2 selected, dragged down to B5: the preview outlines B2:B5.
    let selection = GridSelection::cell(CellRef::new(1, 1)).with_fill_preview(CellRange {
        start: CellRef::new(1, 1),
        end: CellRef::new(1, 4),
    });
    render_grid(
        &painter,
        &layout,
        WINDOW,
        Some(selection),
        &RenderOptions::default(),
    )
    .expect("render");

    let options = RenderOptions::default();
    let expected_height: f64 = (1..=4).map(|row| layout.row(row).expect("row").size).sum();
    let mut dashed = false;
    let mut in_border_colour = false;
    let mut previews = Vec::new();
    for op in painter.ops() {
        match op {
            Op::SetStrokeStyle(color) => in_border_colour = color == options.selection_border,
            Op::SetLineDash(segments) => dashed = !segments.is_empty(),
            Op::StrokeRect(_, _, _, height) if in_border_colour && dashed => previews.push(height),
            _ => {}
        }
    }
    assert_eq!(previews, vec![expected_height]);
}
