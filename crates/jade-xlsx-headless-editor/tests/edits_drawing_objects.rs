//! Charts, pictures and shapes moved, resized and deleted as Excel does
//! when one is dragged by its border or a handle, or selected and deleted:
//! the object keeps what it is and lands on the cells it was put over, the
//! file saves that way, and each change is one undo step.

use jade_xlsx_headless_editor::{HeadlessError, HeadlessWorkbook, ShapeAnchor};
use jade_xlsx_layout::{layout_sheet, LayoutOptions, Rect};
use jade_xlsx_reader::{CellRef, Shape};
use std::path::PathBuf;

fn open(fixture: &str) -> HeadlessWorkbook {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures"
    ))
    .join(fixture);
    HeadlessWorkbook::open_for_editing(&std::fs::read(path).expect("fixture")).expect("open")
}

fn shape(book: &HeadlessWorkbook, id: u32) -> Option<&Shape> {
    book.workbook().sheets[0]
        .shapes
        .iter()
        .find(|shape| shape.id == id)
}

fn drawn(book: &HeadlessWorkbook, id: u32) -> Option<Rect> {
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    let rect = layout
        .shapes_in(Rect {
            x: 0.0,
            y: 0.0,
            width: 1e6,
            height: 1e6,
        })
        .find(|shape| shape.id == id)
        .map(|shape| shape.rect);
    rect
}

/// B10 to F20, a quarter of a cell into each corner's cell.
fn anchor() -> ShapeAnchor {
    ShapeAnchor {
        from: CellRef::new(1, 9),
        from_offset: (152_400, 47_625),
        to: CellRef::new(5, 19),
        to_offset: (152_400, 47_625),
    }
}

#[test]
fn a_chart_moves_and_resizes_onto_the_cells_it_is_put_over() {
    let mut book = open("charts-workbook.xlsx");
    let before = shape(&book, 1).unwrap().clone();
    assert!(before.chart.is_some());
    book.place_shape("Charts", 1, &anchor()).unwrap();
    let moved = shape(&book, 1).unwrap();
    assert_eq!(moved.from, CellRef::new(1, 9));
    assert_eq!(moved.from_offset, (152_400, 47_625));
    assert_eq!(moved.to, Some(CellRef::new(5, 19)));
    assert_eq!(moved.to_offset, (152_400, 47_625));
    // Still the same chart: its kind, title and series.
    let (chart, was) = (
        moved.chart.as_ref().unwrap(),
        before.chart.as_ref().unwrap(),
    );
    assert_eq!((&chart.kind, &chart.title), (&was.kind, &was.title));
    assert_eq!(
        chart
            .series
            .iter()
            .map(|series| &series.values.formula)
            .collect::<Vec<_>>(),
        was.series
            .iter()
            .map(|series| &series.values.formula)
            .collect::<Vec<_>>()
    );
    // Drawn over B10:F20.
    let layout = layout_sheet(
        &book.workbook().sheets[0],
        &book.workbook().styles,
        &LayoutOptions::default(),
    );
    let b10 = layout.cell_rect(CellRef::new(1, 9)).unwrap();
    let rect = drawn(&book, 1).unwrap();
    assert!((rect.x - (b10.x + 16.0)).abs() < 0.01, "{rect:?} {b10:?}");
    assert!((rect.y - (b10.y + 5.0)).abs() < 0.01, "{rect:?} {b10:?}");
    // The other charts stay where they were.
    assert_eq!(shape(&book, 2).unwrap().from_offset, (5_905_500, 190_500));
    // Saved that way.
    let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
    assert_eq!(shape(&reopened, 1).unwrap().to, Some(CellRef::new(5, 19)));
    assert!(shape(&reopened, 1).unwrap().chart.is_some());
    // One undo step.
    assert!(book.undo().unwrap());
    assert_eq!(shape(&book, 1).unwrap().from_offset, before.from_offset);
    assert_eq!(shape(&book, 1).unwrap().to, None);
}

#[test]
fn pictures_of_every_anchor_move_and_keep_their_picture() {
    let mut book = open("pictures-workbook.xlsx");
    // A one-cell, a two-cell and an absolute anchor.
    for id in [1, 2, 3] {
        let before = shape(&book, id).unwrap().clone();
        book.place_shape("Pictures", id, &anchor()).unwrap();
        let moved = shape(&book, id).unwrap();
        assert_eq!(moved.to, Some(CellRef::new(5, 19)), "picture {id}");
        assert_eq!(moved.picture, before.picture, "picture {id}");
        assert_eq!(moved.name, before.name);
    }
}

#[test]
fn a_deleted_chart_is_gone_and_comes_back_on_undo() {
    let mut book = open("charts-workbook.xlsx");
    let count = book.workbook().sheets[0].shapes.len();
    book.delete_shape("Charts", 2).unwrap();
    assert!(shape(&book, 2).is_none());
    assert_eq!(book.workbook().sheets[0].shapes.len(), count - 1);
    assert!(shape(&book, 3).unwrap().chart.is_some());
    let reopened = HeadlessWorkbook::open(&book.export_bytes().unwrap()).unwrap();
    assert!(shape(&reopened, 2).is_none());
    assert_eq!(reopened.workbook().sheets[0].shapes.len(), count - 1);
    assert!(book.undo().unwrap());
    assert!(shape(&book, 2).unwrap().chart.is_some());
}

#[test]
fn an_object_the_sheet_does_not_have_is_refused() {
    let mut book = open("charts-workbook.xlsx");
    assert!(matches!(
        book.place_shape("Charts", 99, &anchor()),
        Err(HeadlessError::NoSuchShape(_))
    ));
    assert!(matches!(
        book.delete_shape("Charts", 99),
        Err(HeadlessError::NoSuchShape(_))
    ));
}
