//! A fully selected table cell is filled on the page surface it sits on,
//! and a full pass paints the same selection as a frozen pixel pass.

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

/// Every selection paint op: fills in the selection colour, and the path
/// points stroked as selection edges.
fn selection_ops(painter: &RecordingPainter) -> Vec<Op> {
    let (mut fill, mut stroke) = (String::new(), String::new());
    let mut stack = Vec::new();
    let mut ops = Vec::new();
    for op in painter.ops() {
        match &op {
            Op::Save => stack.push((fill.clone(), stroke.clone())),
            Op::Restore => (fill, stroke) = stack.pop().unwrap_or_default(),
            Op::SetFillStyle(style) => fill = style.clone(),
            Op::SetStrokeStyle(style) => stroke = style.clone(),
            Op::FillRect(..) if fill == SELECTION_FILL_STYLE => ops.push(op),
            Op::MoveTo(..) | Op::LineTo(..) if stroke == SELECTION_EDGE_STYLE => ops.push(op),
            _ => {}
        }
    }
    ops
}

fn cell_id(renderer: &CanvasRenderer<RecordingPainter>, row: usize, column: usize) -> usize {
    renderer
        .cell_layouts_debug()
        .iter()
        .position(|cell| cell.row == row && cell.column == column)
        .expect("cell")
}

/// Selects cells `from..=to` as a rectangle, as a drag across cells does.
fn select_cells(renderer: &mut CanvasRenderer<RecordingPainter>, from: usize, to: usize) {
    let range = renderer.selection.cell_selection_range(from, to);
    renderer.selection.set_selection(range);
    renderer.selection.set_cell_selection(from, to);
}

/// Selection ops of every page after a full pass, then after a frozen pass
/// over every page, and after a frozen rerender of each page on its own.
fn assert_full_and_frozen_passes_agree(
    document: &Rc<Document>,
    options: &Options,
    renderer: &mut CanvasRenderer<RecordingPainter>,
    pages: &[RecordingPainter],
    context: &str,
) {
    let page_count = renderer.resolved_total_page_count();
    for page in pages {
        page.clear();
    }
    renderer.set_visible_page_range(None);
    renderer.mark_full_repaint();
    renderer
        .render_with_numbering_rc(Rc::clone(document), options, None, None, None, None)
        .unwrap();
    let full: Vec<Vec<Op>> = pages[..page_count].iter().map(selection_ops).collect();
    assert!(
        full.iter().any(|ops| !ops.is_empty()),
        "{context}: nothing selected"
    );
    frozen_rerender(renderer, pages, None);
    for page in 0..page_count {
        assert_eq!(
            selection_ops(&pages[page]),
            full[page],
            "{context}: frozen page {page}"
        );
    }
    for page in 0..page_count {
        frozen_rerender(renderer, pages, Some(page));
        assert_eq!(
            selection_ops(&pages[page]),
            full[page],
            "{context}: frozen rerender of page {page}"
        );
    }
}

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

    for (page, rect) in split_cell_fragments(&renderer) {
        let fills = selection_fills(&pages[page]);
        assert_on_page(&renderer, page, &fills);
        // Word fills a wholly selected cell, not each of its lines.
        assert!(
            fills.contains(&rect),
            "page {page}: expected the cell fragment {rect:?} in {fills:?}"
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

#[test]
fn full_and_frozen_passes_paint_the_same_selection() {
    let document = Rc::new(split_cell_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "all");

    // From inside "Before" to inside the last cell: the cells in between are
    // whole, the last one is partly selected and keeps its line highlight.
    let last = renderer.selection.paragraphs().len() - 2;
    renderer.set_selection_range(Some(SelectionRange::new(
        TextPosition::new(0, 3),
        TextPosition::new(last, 2),
    )));
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "partial");

    // Within the split cell only.
    let first = renderer.cell_layouts_debug()[cell_id(&renderer, 1, 0)].paragraphs[0];
    renderer.set_selection_range(Some(SelectionRange::new(
        TextPosition::new(first + 2, 1),
        TextPosition::new(first + 50, 4),
    )));
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "in cell");

    // Ending inside the split cell's last paragraph, which sits wholly on the
    // second page: neither fragment is wholly selected. A full pass used to
    // fill the first page's fragment, since that paragraph had no length yet.
    let split = renderer.cell_layouts_debug()[cell_id(&renderer, 1, 0)].clone();
    let last = *split.paragraphs.last().unwrap();
    assert_eq!(
        renderer.page_for_y(renderer.selection.paragraphs()[last].lines[0].absolute_top),
        1
    );
    renderer.set_selection_range(Some(SelectionRange::new(
        TextPosition::new(0, 3),
        TextPosition::new(last, 4),
    )));
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "split end");
    let (page, rect) = split_cell_fragments(&renderer)[0];
    assert_eq!(page, 0);
    assert!(
        !selection_fills(&pages[0]).contains(&rect),
        "the head fragment is not wholly selected"
    );

    let (from, to) = (cell_id(&renderer, 0, 0), cell_id(&renderer, 1, 0));
    select_cells(&mut renderer, from, to);
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "cells");
}

#[test]
fn rectangular_cell_selection_fills_only_the_selected_cells() {
    let document = Rc::new(split_cell_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    // The first column of the first two rows. The linear text range under
    // it also runs through r0c1, which must stay unpainted.
    let (from, to) = (cell_id(&renderer, 0, 0), cell_id(&renderer, 1, 0));
    select_cells(&mut renderer, from, to);
    for pass in ["full", "frozen"] {
        for page in &pages {
            page.clear();
        }
        if pass == "full" {
            renderer.mark_full_repaint();
            renderer
                .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
                .unwrap();
        } else {
            frozen_rerender(&mut renderer, &pages, None);
        }
        let mut expected = vec![Vec::new(); renderer.resolved_total_page_count()];
        for cell in renderer.cell_layouts_debug() {
            if cell.column == 0 && cell.row <= 1 {
                let page = renderer.page_for_y(cell.top);
                expected[page].push((
                    cell.left,
                    cell.top - renderer.page_top_y(page),
                    cell.right - cell.left,
                    cell.bottom - cell.top,
                ));
            }
        }
        for (page, expected) in expected.iter().enumerate() {
            assert_eq!(
                &selection_fills(&pages[page]),
                expected,
                "{pass}: page {page}"
            );
        }
    }
}

#[test]
fn cell_split_over_a_column_break_is_filled_in_both_columns() {
    let mut document = split_cell_document();
    document.default_section = Some(SectionProperties {
        columns: Some(SectionColumns {
            count: Some(2),
            ..SectionColumns::default()
        }),
        ..SectionProperties::default()
    });
    let document = Rc::new(document);
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    let fragments = split_cell_fragments(&renderer);
    assert!(
        fragments.len() >= 2 && fragments[0].0 == fragments[1].0,
        "the long cell must continue in the second column of its page: {fragments:?}"
    );
    assert_ne!(fragments[0].1 .0, fragments[1].1 .0);
    for pass in ["full", "frozen"] {
        for page in &pages {
            page.clear();
        }
        if pass == "full" {
            renderer.mark_full_repaint();
            renderer
                .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
                .unwrap();
        } else {
            frozen_rerender(&mut renderer, &pages, None);
        }
        for (page, rect) in &fragments {
            let fills = selection_fills(&pages[*page]);
            let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
            assert!(
                fills.iter().any(|fill| close(fill.0, rect.0)
                    && close(fill.1, rect.1)
                    && close(fill.2, rect.2)
                    && close(fill.3, rect.3)),
                "{pass}: page {page}: expected the cell fragment {rect:?} in {fills:?}"
            );
        }
    }
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "columns");
}

/// A 2-column table whose first column merges every row (`vMerge`), with
/// rows tall enough that the merged cell runs onto the next page.
fn vertical_merge_document() -> Document {
    let rows = (0..6)
        .map(|row| {
            let lines: Vec<String> = (0..15)
                .map(|line| format!("row {row} line {line}"))
                .collect();
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            TableRow {
                cells: vec![
                    TableCell {
                        blocks: if row == 0 {
                            paragraphs(&["merged"])
                        } else {
                            Vec::new()
                        },
                        vertical_merge: Some(if row == 0 { "restart" } else { "continue" }.into()),
                        ..Default::default()
                    },
                    TableCell {
                        blocks: paragraphs(&lines),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }
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

#[test]
fn vertically_merged_cell_is_filled_on_every_page_it_reaches() {
    let document = Rc::new(vertical_merge_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    let owner = renderer.cell_layouts_debug()[cell_id(&renderer, 0, 0)].clone();
    let table_pages: std::collections::BTreeSet<usize> = renderer
        .cell_layouts_debug()
        .iter()
        .map(|cell| renderer.page_for_y(cell.top))
        .collect();
    assert!(
        table_pages.len() >= 2,
        "the table must cross a page: {table_pages:?}"
    );
    for pass in ["full", "frozen"] {
        for page in &pages {
            page.clear();
        }
        if pass == "full" {
            renderer.mark_full_repaint();
            renderer
                .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
                .unwrap();
        } else {
            frozen_rerender(&mut renderer, &pages, None);
        }
        for &page in &table_pages {
            let fills = selection_fills(&pages[page]);
            assert_on_page(&renderer, page, &fills);
            assert!(
                fills
                    .iter()
                    .any(|&(x, _, width, _)| x == owner.left && width == owner.right - owner.left),
                "{pass}: page {page}: the merged cell is not filled: {fills:?}"
            );
        }
    }
    assert_full_and_frozen_passes_agree(&document, &options, &mut renderer, &pages, "vmerge");
}

/// One cell holding a nested 2x2 table between two paragraphs.
fn nested_table_document() -> Document {
    let nested = Table {
        rows: (0..2)
            .map(|row| TableRow {
                cells: (0..2)
                    .map(|column| TableCell {
                        blocks: paragraphs(&[&format!("n{row}{column}")]),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut blocks = paragraphs(&["outer before"]);
    blocks.push(BodyElement::Table(Box::new(nested)));
    blocks.extend(paragraphs(&["outer after"]));
    let mut elements = paragraphs(&["Before the table"]);
    elements.push(BodyElement::Table(Box::new(Table {
        rows: vec![TableRow {
            cells: vec![TableCell {
                blocks,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })));
    elements.extend(paragraphs(&["After the table"]));
    Document {
        body: Some(Body { elements }),
        ..Default::default()
    }
}

#[test]
fn nested_cells_inside_a_filled_cell_are_not_filled_again() {
    let document = Rc::new(nested_table_document());
    let options = options_for_seek_tests();
    let (mut renderer, pages) = selected_renderer(&document, &options);
    let outer_paragraph = renderer
        .selection
        .paragraphs()
        .iter()
        .position(|paragraph| paragraph.text.starts_with("outer before"))
        .unwrap();
    let outer_end = renderer
        .selection
        .paragraphs()
        .iter()
        .position(|paragraph| paragraph.text.starts_with("outer after"))
        .unwrap();
    let selections = [
        // Past the cell: decided before its content.
        None,
        // Ending at the cell's end: decided after its content.
        Some(SelectionRange::new(
            TextPosition::new(outer_paragraph, 0),
            TextPosition::new(outer_end, renderer.selection.paragraph_text_len(outer_end)),
        )),
    ];
    for (case, selection) in selections.into_iter().enumerate() {
        if let Some(selection) = selection {
            renderer.set_selection_range(Some(selection));
        }
        for page in &pages {
            page.clear();
        }
        renderer.set_visible_page_range(None);
        renderer.mark_full_repaint();
        renderer
            .render_with_numbering_rc(Rc::clone(&document), &options, None, None, None, None)
            .unwrap();
        let fills = selection_fills(&pages[0]);
        let outer = renderer
            .cell_layouts_debug()
            .into_iter()
            .find(|cell| cell.paragraphs.contains(&outer_paragraph))
            .unwrap();
        let inside = fills
            .iter()
            .filter(|&&(x, y, width, height)| {
                x >= outer.left
                    && x + width <= outer.right
                    && y > outer.top
                    && y + height <= outer.bottom
            })
            .count();
        assert!(
            fills.contains(&(
                outer.left,
                outer.top,
                outer.right - outer.left,
                outer.bottom - outer.top
            )),
            "case {case}: the outer cell is filled: {fills:?}"
        );
        assert_eq!(
            inside, 0,
            "case {case}: nested cells filled again: {fills:?}"
        );
        assert_full_and_frozen_passes_agree(
            &document,
            &options,
            &mut renderer,
            &pages,
            &format!("nested {case}"),
        );
    }
}
