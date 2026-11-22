//! Section vertical alignment "Justified" (`w:vAlign="both"`, ECMA-376 Part 1
//! §17.6.22) rendered on a recording painter: a page's leftover height is
//! spread between its top-level paragraphs so the last line reaches the body
//! bottom, without moving any page break.

use super::recording_host::recording_renderer;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::RecordingPainter;

const MARGINS: &str = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;

fn paragraph(text: &str, properties: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0"/>{properties}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )
}

/// Section 1 (`first_align`): three short paragraphs. Section 2 (top) starts
/// on a new page with one paragraph, so page 1 ends with a section page break.
fn document(first_align: &str, first_text: &str) -> Document {
    let mut body = String::new();
    body.push_str(&paragraph(first_text, ""));
    body.push_str(&paragraph("Second", ""));
    body.push_str(&paragraph(
        "Third",
        &format!(r#"<w:sectPr>{MARGINS}<w:vAlign w:val="{first_align}"/></w:sectPr>"#),
    ));
    body.push_str(&paragraph("Next page", ""));
    body.push_str(&format!(r#"<w:sectPr>{MARGINS}</w:sectPr>"#));
    let data = build_docx_with_body(&body);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc
        .document
        .as_ref()
        .expect("document")
        .as_ref()
        .clone();
    assert_eq!(document.sections.len(), 2, "two explicit section ranges");
    assert_eq!(
        resolve_section_vertical_align(
            document.sections[0]
                .properties
                .as_ref()
                .and_then(|properties| properties.vertical_alignment.as_deref())
        ),
        resolve_section_vertical_align(Some(first_align))
    );
    document
}

/// `(top, bottom)` of the first line of each of the first `count` paragraphs.
fn line_extents(renderer: &CanvasRenderer<RecordingPainter>, count: usize) -> Vec<(f64, f64)> {
    renderer.selection.paragraphs()[..count]
        .iter()
        .map(|paragraph| {
            let line = paragraph.lines.first().expect("line");
            (line.top, line.top + line.height)
        })
        .collect()
}

fn body_bottom(renderer: &CanvasRenderer<RecordingPainter>) -> f64 {
    renderer.page_height - renderer.margin_bottom
}

#[test]
fn justified_page_spreads_paragraphs_to_the_bottom_margin() {
    let mut top_renderer = recording_renderer();
    top_renderer.mark_full_repaint();
    top_renderer
        .render(&document("top", "First"), &Options::default())
        .expect("top render");
    let natural = line_extents(&top_renderer, 3);

    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer
        .render(&document("both", "First"), &Options::default())
        .expect("justified render");
    let justified = line_extents(&renderer, 3);

    assert!(
        (justified[0].0 - natural[0].0).abs() < 0.01,
        "the first line stays at the top margin: {justified:?} vs {natural:?}"
    );
    let bottom = body_bottom(&renderer);
    assert!(
        (justified[2].1 - bottom).abs() <= 1.0,
        "the last line ends at the body bottom {bottom}: {justified:?}"
    );
    let gaps: Vec<f64> = (1..3)
        .map(|index| justified[index].0 - justified[index - 1].1)
        .collect();
    let natural_gaps: Vec<f64> = (1..3)
        .map(|index| natural[index].0 - natural[index - 1].1)
        .collect();
    assert!(gaps[0] > 100.0, "leftover is spread: {gaps:?}");
    assert!(
        ((gaps[0] - natural_gaps[0]) - (gaps[1] - natural_gaps[1])).abs() < 0.01,
        "the added gaps are equal: {gaps:?} (natural {natural_gaps:?})"
    );
    assert_eq!(
        renderer.current_page_number, top_renderer.current_page_number,
        "justification never changes pagination"
    );
    assert_eq!(renderer.current_page_number, 2);
    let next = &renderer.selection.paragraphs()[3].lines[0];
    let natural_next = &top_renderer.selection.paragraphs()[3].lines[0];
    assert!(
        (next.top - natural_next.top).abs() < 0.01,
        "the top-aligned next section is untouched"
    );
}

/// A layout change re-measures: a first paragraph that wraps to more lines
/// shrinks the leftover, and the last line still ends at the body bottom.
#[test]
fn justified_gaps_are_remeasured_after_an_edit() {
    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer
        .render(&document("both", "First"), &Options::default())
        .expect("first render");
    let before = line_extents(&renderer, 3);

    let long = "word ".repeat(60);
    let edited = document("both", long.trim_end());
    renderer.set_incremental_dirty_from(0);
    renderer
        .render(&edited, &Options::default())
        .expect("edited render");
    let first = &renderer.selection.paragraphs()[0];
    assert!(first.lines.len() > 1, "the edited paragraph wraps");
    let after = line_extents(&renderer, 3);
    let bottom = body_bottom(&renderer);
    assert!(
        (after[2].1 - bottom).abs() <= 1.0,
        "the last line still ends at the body bottom {bottom}: {after:?}"
    );
    assert!(
        after[2].0 - after[1].1 < before[2].0 - before[1].1,
        "the gap shrank with the leftover: {before:?} -> {after:?}"
    );

    let mut fresh = recording_renderer();
    fresh.mark_full_repaint();
    fresh.render(&edited, &Options::default()).expect("fresh");
    let reference = line_extents(&fresh, 3);
    for (edited, fresh) in after.iter().zip(&reference) {
        assert!(
            (edited.0 - fresh.0).abs() < 0.01,
            "the edited layout matches a fresh render: {after:?} vs {reference:?}"
        );
    }
}

/// One paragraph on a justified page has nothing to spread between.
#[test]
fn justified_single_paragraph_page_is_unchanged() {
    let render = |align: &str| {
        let body = format!(
            "{}{}<w:sectPr>{MARGINS}<w:vAlign w:val=\"{align}\"/></w:sectPr>",
            paragraph("Only", ""),
            paragraph("", r#"<w:pageBreakBefore/>"#),
        );
        let data = build_docx_with_body(&body);
        let word_doc = futures::executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load");
        let mut renderer = recording_renderer();
        renderer.mark_full_repaint();
        renderer
            .render(
                word_doc.document.as_ref().expect("document"),
                &Options::default(),
            )
            .expect("render");
        line_extents(&renderer, 1)
    };
    assert_eq!(render("both"), render("top"));
}

#[test]
fn justified_gap_splits_leftover_between_slots() {
    assert_eq!(section_justified_gap(900.0, 300.0, 2), 300.0);
    assert_eq!(section_justified_gap(900.0, 300.0, 0), 0.0);
    assert_eq!(section_justified_gap(900.0, 950.0, 3), 0.0);
}

/// The section's final, partly filled page (here the document's last page,
/// ending without any break) is justified too: ECMA-376 §17.6.22 has no
/// last-page exception. Unconfirmed against a Word capture.
#[test]
fn justified_final_page_of_the_document_is_spread() {
    let body = format!(
        "{}{}<w:sectPr>{MARGINS}<w:vAlign w:val=\"both\"/></w:sectPr>",
        paragraph("One", ""),
        paragraph("Two", ""),
    );
    let data = build_docx_with_body(&body);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer
        .render(
            word_doc.document.as_ref().expect("document"),
            &Options::default(),
        )
        .expect("render");
    let lines = line_extents(&renderer, 2);
    assert!(
        (lines[1].1 - body_bottom(&renderer)).abs() <= 1.0,
        "{lines:?}"
    );
}
