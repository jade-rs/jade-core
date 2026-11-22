//! Table rows paginate against the same body area as paragraphs when the
//! footer story is taller than the bottom margin, and keep the full bottom
//! margin when there is no footer at all.

use super::recording_host::RecordingHost;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_docx_layout::line_metrics::{set_word_layout_platform, WordLayoutPlatform};
use jade_painter::recording::{Op, RecordingPainter};

type Renderer = CanvasRenderer<RecordingPainter>;

const LETTER_1IN: &str = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;
/// A footer distance (720) below a 240-twip bottom margin, with no footer.
const SHORT_BOTTOM: &str = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="240" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;
const TIGHT: &str =
    r#"<w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>"#;
const KEEP_NEXT: &str = r#"<w:pPr><w:keepNext/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>"#;
const ROWS: usize = 60;

fn paragraph(text: &str) -> String {
    format!("<w:p>{TIGHT}<w:r><w:t>{text}</w:t></w:r></w:p>")
}

fn paragraphs(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|index| paragraph(&format!("{prefix} {index}")))
        .collect()
}

#[derive(Default)]
struct Row {
    cant_split: bool,
    header: bool,
    keep_next: bool,
    lines: Vec<String>,
}

fn row(text: &str) -> Row {
    Row {
        lines: vec![text.to_string()],
        ..Row::default()
    }
}

fn plain_rows(count: usize) -> Vec<Row> {
    (0..count)
        .map(|index| row(&format!("Row {index}")))
        .collect()
}

fn table(rows: &[Row]) -> String {
    let mut xml = String::from(
        r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="5000"/></w:tblGrid>"#,
    );
    for row in rows {
        xml.push_str("<w:tr><w:trPr>");
        if row.cant_split {
            xml.push_str("<w:cantSplit/>");
        }
        if row.header {
            xml.push_str("<w:tblHeader/>");
        }
        xml.push_str(r#"</w:trPr><w:tc><w:tcPr><w:tcW w:w="5000" w:type="dxa"/></w:tcPr>"#);
        let properties = if row.keep_next { KEEP_NEXT } else { TIGHT };
        for text in &row.lines {
            xml.push_str(&format!(
                "<w:p>{properties}<w:r><w:t>{text}</w:t></w:r></w:p>"
            ));
        }
        xml.push_str("</w:tc></w:tr>");
    }
    xml.push_str("</w:tbl>");
    xml
}

fn load(body: &str, sect_pr: &str) -> WordDocument {
    let data = build_docx_with_body(&format!("{body}{sect_pr}"));
    futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load")
}

#[derive(Clone, Copy, PartialEq)]
enum Footer {
    None,
    /// Eight paragraphs on every page.
    Tall,
    /// Eight paragraphs on the first page only (`w:titlePg`).
    TallFirstPage,
}

fn render(body: &str, sect_pr: &str, footer: Footer) -> (Renderer, RecordingPainter) {
    let document = load(body, sect_pr);
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("paged recording renderer");
    let story = || {
        load(&paragraphs("footer", 8), sect_pr)
            .document
            .as_ref()
            .and_then(|document| document.body.clone())
    };
    // Hosts pass the footer's text lines with its story.
    let lines: Vec<HeaderLine> = (0..8)
        .map(|index| HeaderLine {
            left: Some(format!("footer {index}")),
            ..HeaderLine::default()
        })
        .collect();
    let mut default_lines = None;
    match footer {
        Footer::None => {}
        Footer::Tall => {
            renderer.set_header_footer_bodies(None, story(), None, None, None, None);
            default_lines = Some(lines.as_slice());
        }
        Footer::TallFirstPage => {
            renderer.set_header_footer_bodies(None, None, None, story(), None, None);
            renderer.set_header_footer_variants(None, Some(lines.clone()), None, None, true, false);
        }
    }
    renderer
        .render_with_numbering(
            document.document.as_ref().expect("document"),
            &Options::default(),
            None,
            None,
            default_lines,
            None,
        )
        .expect("render");
    (renderer, painter)
}

fn render_tall(body: &str) -> Renderer {
    render(body, LETTER_1IN, Footer::Tall).0
}

/// Page-local top of the footer story's first line on `page`.
fn footer_top(renderer: &Renderer, page: usize) -> f64 {
    let top = renderer
        .hf_story_layouts
        .get(&(page, false))
        .expect("footer laid out")
        .borrow()
        .paragraphs()
        .iter()
        .flat_map(|paragraph| &paragraph.lines)
        .map(|line| line.top)
        .reduce(f64::min)
        .expect("footer lines");
    assert!(
        top < renderer.page_height - renderer.margin_bottom - 1.0,
        "the footer must reach above the bottom margin"
    );
    top
}

/// Page index and page-local (top, bottom) of each cell fragment of `row`.
fn row_fragments(renderer: &Renderer, row: usize) -> Vec<(usize, f64, f64)> {
    renderer
        .selection
        .cell_layouts()
        .iter()
        .filter(|cell| cell.row == row)
        .map(|cell| {
            let page = renderer.page_for_y(cell.top + 0.5);
            let origin = renderer.page_top_y(page);
            (page, cell.top - origin, cell.bottom - origin)
        })
        .collect()
}

fn pages_of_row(renderer: &Renderer, row: usize) -> Vec<usize> {
    let mut pages: Vec<_> = row_fragments(renderer, row)
        .into_iter()
        .map(|(page, _, _)| page)
        .collect();
    pages.dedup();
    pages
}

fn first_row_on_page(renderer: &Renderer, page: usize) -> usize {
    renderer
        .selection
        .cell_layouts()
        .iter()
        .filter(|cell| renderer.page_for_y(cell.top + 0.5) == page)
        .map(|cell| cell.row)
        .min()
        .expect("a row on the page")
}

fn first_paragraph_on_page(renderer: &Renderer, page: usize) -> usize {
    renderer
        .selection
        .paragraphs()
        .iter()
        .position(|surface| {
            surface
                .lines
                .first()
                .is_some_and(|line| renderer.page_for_y(line.absolute_top) == page)
        })
        .expect("a paragraph on the page")
}

/// No cell fragment on `page` reaches below `limit`.
fn assert_rows_end_above(renderer: &Renderer, rows: usize, page: usize, limit: f64) -> usize {
    let mut fragments = 0;
    for row in 0..rows {
        for (fragment_page, _, bottom) in row_fragments(renderer, row) {
            if fragment_page == page {
                fragments += 1;
                assert!(
                    bottom <= limit + 0.5,
                    "row {row} ends at {bottom} on page {page}, below {limit}"
                );
            }
        }
    }
    fragments
}

/// The line height of a one-paragraph row, the same as a body paragraph's.
fn row_height(renderer: &Renderer) -> f64 {
    let (_, top, bottom) = row_fragments(renderer, 0)[0];
    bottom - top
}

#[test]
fn table_rows_stop_above_a_footer_taller_than_the_bottom_margin() {
    let renderer = render_tall(&table(&plain_rows(ROWS)));
    let footer = footer_top(&renderer, 0);
    assert!(renderer.unreserved_body_bottom() <= footer + 0.5);
    let placed = assert_rows_end_above(&renderer, ROWS, 0, footer);
    assert!(placed > 10, "rows fill the first page");
    assert_rows_end_above(&renderer, ROWS, 1, footer_top(&renderer, 1));

    // Equal-height paragraphs break at the same place.
    let flow = render_tall(&paragraphs("Row", ROWS));
    assert_eq!(
        first_row_on_page(&renderer, 1),
        first_paragraph_on_page(&flow, 1)
    );
}

/// Without a footer the footer distance reserves nothing, even when it is
/// larger than the bottom margin: rows and paragraphs both reach the margin.
#[test]
fn rows_and_paragraphs_reach_a_short_bottom_margin_without_a_footer() {
    let (renderer, _) = render(&table(&plain_rows(ROWS)), SHORT_BOTTOM, Footer::None);
    let (flow, _) = render(&paragraphs("Row", ROWS), SHORT_BOTTOM, Footer::None);
    let margin_edge = renderer.page_height - renderer.margin_bottom;
    let footer_distance_edge = renderer.page_height - 48.0;
    assert!(margin_edge > footer_distance_edge + 20.0);
    let height = row_height(&renderer);

    let last_row = first_row_on_page(&renderer, 1) - 1;
    let (_, _, row_bottom) = *row_fragments(&renderer, last_row).last().unwrap();
    assert!(
        row_bottom <= margin_edge + 0.5 && row_bottom > margin_edge - height,
        "the last row on page 1 ends at {row_bottom}, the margin is at {margin_edge}"
    );
    let last_paragraph = first_paragraph_on_page(&flow, 1) - 1;
    let line = flow.selection.paragraphs()[last_paragraph]
        .lines
        .last()
        .unwrap();
    let paragraph_bottom = line.top + line.height;
    assert!(
        paragraph_bottom <= margin_edge + 0.5 && paragraph_bottom > margin_edge - height,
        "the last paragraph on page 1 ends at {paragraph_bottom}, the margin is at {margin_edge}"
    );
    assert_eq!(last_row, last_paragraph);
}

/// A cantSplit row taller than the body between the margin and the tall
/// footer fits no page here (every page has that footer), so it splits where
/// it is, above the footer, as an over-tall row does on main.
#[test]
fn cant_split_row_that_fits_no_footer_aware_page_splits_in_place() {
    let probe = render_tall(&table(&plain_rows(1)));
    let line = row_height(&probe);
    let nominal = probe.page_height - probe.margin_top - probe.margin_bottom;
    let effective = footer_top(&probe, 0) - probe.margin_top;
    let lines = ((effective + nominal) / 2.0 / line).round() as usize;
    let height = lines as f64 * line;
    assert!(
        height > effective + 1.0 && height < nominal - 1.0,
        "the row is between the effective and nominal body heights"
    );
    let tall = Row {
        cant_split: true,
        lines: (0..lines).map(|index| format!("line {index}")).collect(),
        ..Row::default()
    };
    let renderer = render_tall(&format!("{}{}", paragraph("lead"), table(&[tall])));
    let fragments = row_fragments(&renderer, 0);
    assert_eq!(pages_of_row(&renderer, 0), [0, 1], "{fragments:?}");
    let (_, head_top, head_bottom) = fragments[0];
    assert!(
        head_top > renderer.margin_top + 1.0,
        "the head follows the lead"
    );
    assert!(head_bottom <= footer_top(&renderer, 0) + 0.5);
}

/// A cantSplit row directly below the table's header row starts the frame:
/// it splits there rather than leaving the header alone on the page.
#[test]
fn cant_split_row_below_a_header_row_does_not_strand_the_header() {
    let rows = [
        Row {
            header: true,
            ..row("Heading")
        },
        Row {
            cant_split: true,
            lines: (0..40).map(|index| format!("line {index}")).collect(),
            ..Row::default()
        },
    ];
    let (renderer, _) = render(&table(&rows), LETTER_1IN, Footer::None);
    let header_pages = pages_of_row(&renderer, 0);
    assert_eq!(header_pages.first(), Some(&0));
    assert_eq!(
        pages_of_row(&renderer, 1).first(),
        Some(&0),
        "body lines share the header's page"
    );
}

/// A tall first-page footer does not shrink the page a cantSplit row moves
/// to: a row that fits page 2 whole moves there unsplit.
#[test]
fn cant_split_row_moves_whole_past_a_tall_first_page_footer() {
    let (probe, _) = render(&table(&plain_rows(1)), LETTER_1IN, Footer::TallFirstPage);
    let line = row_height(&probe);
    let nominal = probe.page_height - probe.margin_top - probe.margin_bottom;
    let first_page = footer_top(&probe, 0) - probe.margin_top;
    let lines = ((first_page + nominal) / 2.0 / line).round() as usize;
    let height = lines as f64 * line;
    assert!(height > first_page + 1.0 && height < nominal - 1.0);
    let tall = Row {
        cant_split: true,
        lines: (0..lines).map(|index| format!("line {index}")).collect(),
        ..Row::default()
    };
    let (renderer, _) = render(
        &format!("{}{}", paragraph("lead"), table(&[tall])),
        LETTER_1IN,
        Footer::TallFirstPage,
    );
    let fragments = row_fragments(&renderer, 0);
    assert_eq!(fragments.len(), 1, "the row stays whole: {fragments:?}");
    assert_eq!(fragments[0].0, 1, "on page 2");
}

/// Repeated header rows count against the footer-aware body too.
#[test]
fn repeated_header_rows_paginate_above_a_tall_footer() {
    let mut rows = vec![Row {
        header: true,
        ..row("Heading")
    }];
    rows.extend(plain_rows(ROWS));
    let (renderer, painter) = render(&table(&rows), LETTER_1IN, Footer::Tall);
    for page in 0..2 {
        assert_rows_end_above(&renderer, rows.len(), page, footer_top(&renderer, page));
    }
    // Glyphs are painted one by one; only the heading has a capital H.
    let headings = painter
        .ops()
        .iter()
        .filter(|op| matches!(op, Op::FillText(text, _, _) if text == "H"))
        .count();
    assert_eq!(headings, 3, "the header row repeats on pages 2 and 3");

    // Paragraph i of the flow stands for row i (the heading is row 0).
    let mut flow_body = paragraph("Heading");
    flow_body.push_str(&paragraphs("Row", ROWS));
    let flow = render_tall(&flow_body);
    assert_eq!(
        first_row_on_page(&renderer, 1),
        first_paragraph_on_page(&flow, 1)
    );
}

/// Word for Mac moves leading keep-with-next rows together; whether they fit
/// is judged above the tall footer, not above the bottom margin.
#[test]
fn keep_with_next_rows_move_together_above_a_tall_footer() {
    set_word_layout_platform(WordLayoutPlatform::Mac);
    let flow = render_tall(&paragraphs("lead", ROWS));
    let per_page = first_paragraph_on_page(&flow, 1);
    // Two lines remain above the footer; the bottom margin would leave more.
    let lead = paragraphs("lead", per_page - 2);
    let mut rows: Vec<Row> = (0..3)
        .map(|index| Row {
            keep_next: true,
            ..row(&format!("Kept {index}"))
        })
        .collect();
    rows.extend(plain_rows(5));
    let renderer = render_tall(&format!("{lead}{}", table(&rows)));
    let footer = footer_top(&renderer, 0);
    let margin_room = renderer.page_height - renderer.margin_bottom - footer;
    set_word_layout_platform(WordLayoutPlatform::Windows);
    assert!(
        margin_room > 3.0 * row_height(&renderer),
        "the margin alone would fit them"
    );
    for kept in 0..3 {
        assert_eq!(pages_of_row(&renderer, kept), [1], "kept row {kept}");
    }
}

/// Whether a cantSplit row moves is judged against the next page's measured
/// footer on every render, not a per-line estimate on the first one only.
#[test]
fn cant_split_moves_alike_on_the_first_and_later_renders() {
    let story = |count: usize| {
        load(&paragraphs("footer", count), LETTER_1IN)
            .document
            .as_ref()
            .and_then(|document| document.body.clone())
    };
    let lines = |count: usize| -> Vec<HeaderLine> {
        (0..count)
            .map(|index| HeaderLine {
                left: Some(format!("footer {index}")),
                ..HeaderLine::default()
            })
            .collect()
    };
    let default_lines = lines(8);
    for count in 26..=36usize {
        let tall = Row {
            cant_split: true,
            lines: (0..count).map(|index| format!("L{index}")).collect(),
            ..Row::default()
        };
        let document = load(
            &format!("{}{}", paragraph("lead"), table(&[tall])),
            LETTER_1IN,
        );
        let mut renderer = CanvasRenderer::with_host(
            Box::new(RecordingHost {
                painter: RecordingPainter::new(),
            }),
            false,
        )
        .expect("paged recording renderer");
        // A 12-line first-page footer; the 8-line default footer measures
        // taller than its 20px-per-line estimate.
        renderer.set_header_footer_bodies(None, story(8), None, story(12), None, None);
        renderer.set_header_footer_variants(None, Some(lines(12)), None, None, true, false);
        let mut renders = Vec::new();
        for _ in 0..2 {
            renderer
                .render_with_numbering(
                    document.document.as_ref().expect("document"),
                    &Options::default(),
                    None,
                    None,
                    Some(&default_lines),
                    None,
                )
                .expect("render");
            let fragments: Vec<(usize, i64, i64)> = row_fragments(&renderer, 0)
                .into_iter()
                .map(|(page, top, bottom)| (page, top.round() as i64, bottom.round() as i64))
                .collect();
            renders.push(fragments);
        }
        assert_eq!(renders[0], renders[1], "{count} lines");
    }
}
