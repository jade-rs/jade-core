//! A fully selected table cell is filled on the page surface it sits on,
//! whichever pass paints it.

use super::*;
use jade_docx_reader::document::{Table, TableCell, TableRow};
use jade_painter::recording::{Op, RecordingPainter};

const PAGES: usize = 4;

fn paragraphs(texts: &[&str]) -> Vec<BodyElement> {
    doc_with_paragraph_texts(texts).body.unwrap().elements
}

/// "Before", a 3x2 table whose middle-left cell holds enough paragraphs to
/// split over a page break, and "After".
fn split_cell_document() -> Document {
    let long: Vec<String> = (0..60)
        .map(|line| format!("long cell line {line}"))
        .collect();
    let long: Vec<&str> = long.iter().map(String::as_str).collect();
    let rows = (0..3)
        .map(|row| TableRow {
            cells: (0..2)
                .map(|column| TableCell {
                    blocks: if row == 1 && column == 0 {
                        paragraphs(&long)
                    } else {
                        paragraphs(&[&format!("r{row}c{column}")])
                    },
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    let mut elements = paragraphs(&["Before the table"]);
    elements.push(BodyElement::Table(Box::new(Table {
        rows,
        ..Default::default()
    })));
    elements.extend(paragraphs(&["After the table"]));
    Document {
        body: Some(Body { elements }),
        ..Default::default()
    }
}

/// Selection fills, as `(x, y, width, height)` in page-surface pixels.
fn selection_fills(painter: &RecordingPainter) -> Vec<(f64, f64, f64, f64)> {
    let mut fill = String::new();
    let mut stack = Vec::new();
    let mut fills = Vec::new();
    for op in painter.ops() {
        match op {
            Op::Save => stack.push(fill.clone()),
            Op::Restore => fill = stack.pop().unwrap_or_default(),
            Op::SetFillStyle(style) => fill = style,
            Op::FillRect(x, y, width, height) if fill == SELECTION_FILL_STYLE => {
                fills.push((x, y, width, height))
            }
            _ => {}
        }
    }
    fills
}

type Rect = (f64, f64, f64, f64);

fn select_everything(renderer: &mut CanvasRenderer<RecordingPainter>) {
    let last = renderer.selection.paragraphs().len() - 1;
    let end = renderer.selection.paragraph_text_len(last);
    renderer.set_selection_range(Some(SelectionRange::new(
        TextPosition::new(0, 0),
        TextPosition::new(last, end),
    )));
}

/// Lays the document out once, then selects all of it.
fn selected_renderer(
    document: &Rc<Document>,
    options: &Options,
) -> (CanvasRenderer<RecordingPainter>, Vec<RecordingPainter>) {
    let (host, pages) = PerPageRecordingHost::new(PAGES + 2);
    let mut renderer = CanvasRenderer::with_host(Box::new(host), false).unwrap();
    // Paint the selection into the page, as the native renderer does.
    renderer.set_use_selection_overlay(false);
    renderer
        .render_with_numbering_rc(Rc::clone(document), options, None, None, None, None)
        .unwrap();
    assert!(renderer.last_layout_complete());
    select_everything(&mut renderer);
    (renderer, pages)
}

/// A pixel-only pass over `window` (every page when `None`) that keeps the
/// registry of the previous pass, as a bitmap fetch does.
fn frozen_rerender(
    renderer: &mut CanvasRenderer<RecordingPainter>,
    pages: &[RecordingPainter],
    window: Option<usize>,
) {
    for page in pages {
        page.clear();
    }
    match window {
        Some(page_index) => {
            renderer.set_visible_page_range(Some(page_index + 1..=page_index + 1));
            renderer.prepare_rerender_for_page(page_index);
        }
        None => renderer.set_visible_page_range(None),
    }
    renderer.freeze_selection_for_pixel_rerender();
    renderer.mark_full_repaint();
    renderer.rerender_with_cached_state().unwrap();
}

/// Each fragment of the split cell (row 1, column 0) as `(page, page rect)`.
fn split_cell_fragments(renderer: &CanvasRenderer<RecordingPainter>) -> Vec<(usize, Rect)> {
    renderer
        .cell_layouts_debug()
        .into_iter()
        .filter(|cell| cell.row == 1 && cell.column == 0)
        .map(|cell| {
            let page = renderer.page_for_y(cell.top);
            (
                page,
                (
                    cell.left,
                    cell.top - renderer.page_top_y(page),
                    cell.right - cell.left,
                    cell.bottom - cell.top,
                ),
            )
        })
        .collect()
}

fn assert_on_page(renderer: &CanvasRenderer<RecordingPainter>, page: usize, fills: &[Rect]) {
    let height = renderer.page_height_for_page(page);
    for &(_, y, _, h) in fills {
        assert!(
            y >= 0.0 && y + h <= height + 0.5,
            "page {page}: selection fill y={y} h={h} leaves the {height}px page: {fills:?}"
        );
    }
}

#[test]
fn split_selected_cell_is_filled_on_both_pages_by_a_frozen_pass() {
    let document = Rc::new(split_cell_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    frozen_rerender(&mut renderer, &pages, None);

    let fragments = split_cell_fragments(&renderer);
    assert_eq!(
        fragments.iter().map(|(page, _)| *page).collect::<Vec<_>>(),
        vec![0, 1],
        "the long cell must split over the first page break: {fragments:?}"
    );
    for (page, rect) in fragments {
        let fills = selection_fills(&pages[page]);
        assert_on_page(&renderer, page, &fills);
        assert!(
            fills.contains(&rect),
            "page {page}: expected the cell fragment {rect:?} in {fills:?}"
        );
    }
}

#[test]
fn split_selected_cell_is_highlighted_on_both_pages_by_a_full_pass() {
    let document = Rc::new(split_cell_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    for page in &pages {
        page.clear();
    }
    renderer.mark_full_repaint();
    renderer
        .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
        .unwrap();

    for (page, (x, y, width, height)) in split_cell_fragments(&renderer) {
        let fills = selection_fills(&pages[page]);
        assert_on_page(&renderer, page, &fills);
        // The part continued from the previous page used to look up the first
        // page's fragment and fill it below this page's surface.
        assert!(
            fills.iter().any(|&(fx, fy, fw, fh)| fx >= x - 0.5
                && fx + fw <= x + width + 0.5
                && fy >= y - 0.5
                && fy + fh <= y + height + 0.5),
            "page {page}: no highlight inside the cell fragment {:?}: {fills:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn frozen_page_rerender_paints_the_same_cell_highlights_as_a_full_frozen_render() {
    let document = Rc::new(split_cell_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    let page_count = renderer.resolved_total_page_count();
    assert!(page_count >= 2);

    frozen_rerender(&mut renderer, &pages, None);
    let expected: Vec<Vec<Rect>> = pages[..page_count].iter().map(selection_fills).collect();
    for (page, fills) in expected.iter().enumerate() {
        assert_on_page(&renderer, page, fills);
    }
    for (page, _) in split_cell_fragments(&renderer) {
        assert!(!expected[page].is_empty(), "page {page} shows no selection");
    }
    for page_index in (0..page_count).chain([1, 0]) {
        frozen_rerender(&mut renderer, &pages, Some(page_index));
        assert_eq!(
            selection_fills(&pages[page_index]),
            expected[page_index],
            "page {page_index}"
        );
    }
}

#[test]
fn overlay_cell_rects_are_relative_to_their_page() {
    let document = Rc::new(split_cell_document());
    let options = options_for_seek_tests();
    let (mut renderer, _pages) = selected_renderer(&document, &options);
    let cells = renderer.cell_layouts_debug();
    let (first, last) = (0, cells.len() - 1);
    assert!(cells
        .iter()
        .all(|cell| cell.table_id == cells[first].table_id));
    let range = renderer.selection.cell_selection_range(first, last);
    renderer.selection.set_selection(range);
    renderer.selection.set_cell_selection(first, last);

    let rects = renderer.get_selection_rects();
    for (page, (x, y, width, height)) in split_cell_fragments(&renderer) {
        assert!(
            rects.iter().any(|rect| rect.page == page
                && (rect.x, rect.y, rect.width, rect.height) == (x, y, width, height)),
            "page {page}: expected the cell fragment {:?} in {rects:?}",
            (x, y, width, height)
        );
    }
    for rect in &rects {
        let height = renderer.page_height_for_page(rect.page);
        assert!(
            rect.y >= 0.0 && rect.y + rect.height <= height + 0.5,
            "overlay rect leaves page {}: {rect:?}",
            rect.page
        );
    }
}
