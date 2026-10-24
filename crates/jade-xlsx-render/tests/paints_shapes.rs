//! Drawing objects over the grid: a text box painted as a box with its
//! text, and a chart with its actual series.

use jade_painter::recording::{Op, RecordingPainter};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{read_xlsx, CellRef, ShapeKind};
use jade_xlsx_render::{render_sheet, render_viewport, RenderOptions};
use std::path::PathBuf;

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/objects-workbook.xlsx"
    ));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn shapes_are_placed_from_their_anchor_cells_and_offsets() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    let layout = layout_sheet(sheet, &workbook.styles, &LayoutOptions::default());

    assert_eq!(layout.shapes.len(), 3);
    let text_box = &layout.shapes[0];
    assert_eq!(text_box.kind, ShapeKind::TextBox);
    let column_e = layout.column(4).expect("column E");
    let column_g = layout.column(6).expect("column G");
    let row_2 = layout.row(1).expect("row 2");
    let row_4 = layout.row(3).expect("row 4");
    // 9525 EMU is one pixel: the offsets land the corners a few px into
    // their cells.
    assert!((text_box.rect.x - (column_e.offset + 1.0)).abs() < 0.01);
    assert!((text_box.rect.y - (row_2.offset + 2.0)).abs() < 0.01);
    assert!((text_box.rect.right() - (column_g.offset + 4.0)).abs() < 0.01);
    assert!((text_box.rect.bottom() - (row_4.offset + 8.0)).abs() < 0.01);
    assert_eq!(text_box.lines, ["Draft"]);

    // A one-cell anchor is sized by its extent: 914400 x 285750 EMU.
    let rectangle = &layout.shapes[2];
    let column_j = layout.column(9).expect("column J");
    assert!((rectangle.rect.x - column_j.offset).abs() < 0.01);
    assert!((rectangle.rect.width - 96.0).abs() < 0.01);
    assert!((rectangle.rect.height - 30.0).abs() < 0.01);
    // Wrapped inside the shape's text insets (0.1 inch a side).
    assert_eq!(rectangle.lines.join(" "), "Note & label line two");
    // The grid reaches under every shape.
    assert!(layout.columns.len() > 9);
    assert!(layout.rows.len() > 13);
}

#[test]
fn a_text_box_paints_its_text_and_a_chart_paints_its_series() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    let layout = layout_sheet(sheet, &workbook.styles, &LayoutOptions::default());
    let painter = RecordingPainter::new();
    render_sheet(&painter, &layout, &RenderOptions::default()).expect("render");
    let ops = painter.ops();

    let text_box = &layout.shapes[0];
    // The box goes over the cells: after every cell's text. It is filled
    // with the theme's background (lt1) and outlined in lt1 shaded by half,
    // as its spPr says, around its rectangle's outline.
    let last_cell_text = ops
        .iter()
        .rposition(|op| matches!(op, Op::FillText(text, ..) if text == "Total"))
        .expect("cell text");
    let corner = ops[last_cell_text..]
        .iter()
        .position(|op| {
            matches!(op,
            Op::LineTo(x, y) if (*x - text_box.rect.width).abs() < 0.01
                && (*y - text_box.rect.height).abs() < 0.01)
        })
        .map(|index| index + last_cell_text)
        .expect("the text box's outline");
    let box_fill = ops[corner..]
        .iter()
        .position(|op| matches!(op, Op::Fill))
        .map(|index| index + corner)
        .expect("the text box is filled");
    assert!(ops[corner..box_fill]
        .iter()
        .any(|op| matches!(op, Op::SetFillStyle(style) if style == "rgba(255, 255, 255, 1)")));
    assert!(ops[box_fill..].iter().any(|op| matches!(op, Op::Stroke)));
    let draft = ops[box_fill..]
        .iter()
        .find(|op| matches!(op, Op::FillText(text, ..) if text == "Draft"))
        .expect("the text box's text");
    if let Op::FillText(_, x, y) = draft {
        assert!(*x > text_box.rect.x && *x < text_box.rect.right());
        assert!(*y > text_box.rect.y && *y < text_box.rect.bottom());
    }

    let chart = &layout.shapes[1];
    let placed = chart.chart.as_ref().expect("chart layout");
    assert!(!placed.ops.is_empty());
    assert!(!ops
        .iter()
        .any(|op| matches!(op, Op::FillText(text, ..) if text == "Chart 2")));
    assert!(placed.ops.iter().any(|op| matches!(op, jade_xlsx_layout::chart::ChartOp::Rectangle {fill: Some(color), ..} if color != "#ffffff")));
    assert!(ops
        .iter()
        .any(|op| matches!(op, Op::FillText(text, ..) if text.starts_with("Note"))));
}

#[test]
fn a_shape_outside_the_viewport_is_not_painted() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    let layout = layout_sheet(sheet, &workbook.styles, &LayoutOptions::default());
    let painter = RecordingPainter::new();
    // Only the first column and row: no shape reaches there.
    render_viewport(
        &painter,
        &layout,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 15.0,
        },
        &RenderOptions::default(),
    )
    .expect("render");
    assert!(!painter
        .ops()
        .iter()
        .any(|op| matches!(op, Op::FillText(text, ..) if text == "Draft" || text == "Chart 2")));
}

#[test]
fn a_noted_cell_wears_excels_red_corner_triangle() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    let layout = layout_sheet(sheet, &workbook.styles, &LayoutOptions::default());

    // The fixture's note is on B2, and the marker follows the cell's box.
    let note = layout
        .notes
        .iter()
        .find(|note| note.reference == CellRef::new(1, 1))
        .expect("the note on B2");
    assert!(note.text.contains("Check the quantity"));
    let cell = layout.cell(CellRef::new(1, 1)).expect("B2");
    assert_eq!(note.rect, cell.rect);

    let painter = RecordingPainter::new();
    render_sheet(&painter, &layout, &RenderOptions::default()).expect("render");
    let ops = painter.ops();
    let corner = ops
        .iter()
        .position(|op| {
            matches!(op, Op::MoveTo(x, y)
            if (*x - (note.rect.right() - 5.0)).abs() < 0.01 && (*y - note.rect.y).abs() < 0.01)
        })
        .expect("the marker's first corner");
    // Red, whatever the theme, and a filled triangle rather than a stroke.
    assert!(ops[..corner]
        .iter()
        .rev()
        .find_map(|op| match op {
            Op::SetFillStyle(color) => Some(color.clone()),
            _ => None,
        })
        .is_some_and(|color| color.eq_ignore_ascii_case("#ff0000")));
    assert!(matches!(ops[corner + 1], Op::LineTo(x, y)
        if (x - note.rect.right()).abs() < 0.01 && (y - note.rect.y).abs() < 0.01));
    assert!(ops[corner..].iter().any(|op| matches!(op, Op::Fill)));
}

#[test]
fn a_point_over_a_drawing_hits_the_drawing_rather_than_the_cell_behind_it() {
    let workbook = read_xlsx(&fixture()).expect("read");
    let sheet = &workbook.sheets[0];
    let layout = layout_sheet(sheet, &workbook.styles, &LayoutOptions::default());

    let text_box = &layout.shapes[0];
    let x = text_box.rect.x + text_box.rect.width / 2.0;
    let y = text_box.rect.y + text_box.rect.height / 2.0;
    let hit = layout.shape_at(x, y).expect("the text box under the point");
    assert_eq!(hit.id, text_box.id);
    // The grid is still under it — which is exactly why a shell has to ask
    // about drawings first.
    assert!(layout.cell_at(x, y).is_some());

    // A point on the grid clear of every drawing hits nothing.
    assert!(layout.shape_at(1.0, 1.0).is_none());
}
