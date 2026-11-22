//! Table rows paginate against the same body area as paragraphs when the
//! footer story is taller than the bottom margin.

use super::recording_host::recording_renderer;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::RecordingPainter;

const SECT_PR: &str = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;
const TIGHT: &str =
    r#"<w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>"#;
const ROWS: usize = 60;

fn paragraph(text: &str) -> String {
    format!("<w:p>{TIGHT}<w:r><w:t>{text}</w:t></w:r></w:p>")
}

fn table(rows: &[(bool, Vec<String>)]) -> String {
    let mut xml = String::from(
        r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="5000"/></w:tblGrid>"#,
    );
    for (cant_split, texts) in rows {
        xml.push_str("<w:tr>");
        if *cant_split {
            xml.push_str("<w:trPr><w:cantSplit/></w:trPr>");
        }
        xml.push_str(r#"<w:tc><w:tcPr><w:tcW w:w="5000" w:type="dxa"/></w:tcPr>"#);
        for text in texts {
            xml.push_str(&paragraph(text));
        }
        xml.push_str("</w:tc></w:tr>");
    }
    xml.push_str("</w:tbl>");
    xml
}

fn load(body: &str) -> WordDocument {
    let data = build_docx_with_body(&format!("{body}{SECT_PR}"));
    futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load")
}

/// Renders `body` under an eight-paragraph footer.
fn render_with_tall_footer(body: &str) -> CanvasRenderer<RecordingPainter> {
    let footer_xml: String = (0..8)
        .map(|index| paragraph(&format!("footer {index}")))
        .collect();
    let footer = load(&footer_xml);
    let document = load(body);
    let mut renderer = recording_renderer();
    renderer.set_header_footer_bodies(
        None,
        footer.document.as_ref().and_then(|d| d.body.clone()),
        None,
        None,
        None,
        None,
    );
    // Hosts pass the footer's text lines too; they gate measuring the story.
    let lines: Vec<HeaderLine> = (0..8)
        .map(|index| HeaderLine {
            left: Some(format!("footer {index}")),
            ..HeaderLine::default()
        })
        .collect();
    renderer
        .render_with_numbering(
            document.document.as_ref().expect("document"),
            &Options::default(),
            None,
            None,
            Some(&lines),
            None,
        )
        .expect("render");
    renderer
}

/// The footer story's first line on page 1, page-local.
fn footer_top(renderer: &CanvasRenderer<RecordingPainter>) -> f64 {
    let boundary = renderer
        .footer_edit_boundary_for_page(0)
        .expect("footer laid out");
    assert!(
        boundary < renderer.page_height - renderer.margin_bottom - 1.0,
        "the footer must reach above the bottom margin"
    );
    boundary
}

/// Page index and page-local bottom of each painted cell fragment of row `row`.
fn row_fragments(renderer: &CanvasRenderer<RecordingPainter>, row: usize) -> Vec<(usize, f64)> {
    renderer
        .selection
        .cell_layouts()
        .iter()
        .filter(|cell| cell.row == row)
        .map(|cell| {
            let page = renderer.page_for_y(cell.top + 0.5);
            (page, cell.bottom - renderer.page_top_y(page))
        })
        .collect()
}

fn first_row_on_page(renderer: &CanvasRenderer<RecordingPainter>, page: usize) -> usize {
    renderer
        .selection
        .cell_layouts()
        .iter()
        .filter(|cell| renderer.page_for_y(cell.top + 0.5) == page)
        .map(|cell| cell.row)
        .min()
        .expect("a row on the page")
}

#[test]
fn table_rows_stop_above_a_footer_taller_than_the_bottom_margin() {
    let rows: Vec<_> = (0..ROWS)
        .map(|index| (false, vec![format!("Row {index}")]))
        .collect();
    let renderer = render_with_tall_footer(&table(&rows));
    let boundary = footer_top(&renderer);
    let mut on_first_page = 0;
    for row in 0..ROWS {
        for (page, bottom) in row_fragments(&renderer, row) {
            if page == 0 {
                on_first_page += 1;
                assert!(
                    bottom <= boundary + 0.5,
                    "row {row} ends at {bottom}, the footer starts at {boundary}"
                );
            }
        }
    }
    assert!(on_first_page > 10, "rows fill the first page");

    // Equal-height paragraphs break at the same place.
    let paragraphs: String = (0..ROWS)
        .map(|index| paragraph(&format!("Row {index}")))
        .collect();
    let flow = render_with_tall_footer(&paragraphs);
    let first_on_second_page = flow
        .selection
        .paragraphs()
        .iter()
        .position(|surface| {
            surface
                .lines
                .first()
                .is_some_and(|line| flow.page_for_y(line.absolute_top) == 1)
        })
        .expect("paragraphs reach page 2");
    assert_eq!(first_row_on_page(&renderer, 1), first_on_second_page);
}

/// A cantSplit row taller than the body between the margin and the tall
/// footer cannot fit any page, so Word splits it.
#[test]
fn cant_split_row_taller_than_the_footer_aware_body_splits() {
    let probe = render_with_tall_footer(&table(&[(false, vec!["probe".into()])]));
    let line = probe.selection.cell_layouts()[0].bottom - probe.selection.cell_layouts()[0].top;
    let nominal = probe.page_height - probe.margin_top - probe.margin_bottom;
    let effective = footer_top(&probe) - probe.margin_top;
    let lines = ((effective + nominal) / 2.0 / line).round() as usize;
    assert!(
        (lines as f64) * line > effective + 1.0 && (lines as f64) * line < nominal - 1.0,
        "the row is between the effective and nominal body heights"
    );
    let tall: Vec<String> = (0..lines).map(|index| format!("line {index}")).collect();
    let renderer =
        render_with_tall_footer(&format!("{}{}", paragraph("lead"), table(&[(true, tall)])));
    let boundary = footer_top(&renderer);
    let fragments = row_fragments(&renderer, 0);
    let pages: std::collections::BTreeSet<_> = fragments.iter().map(|(page, _)| *page).collect();
    assert_eq!(pages.len(), 2, "the row splits: {fragments:?}");
    for (page, bottom) in fragments {
        if page == 0 {
            assert!(bottom <= boundary + 0.5, "the head runs into the footer");
        }
    }
}
