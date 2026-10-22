//! Word's automatic hyphenation (`w:autoHyphenation`): lines end in a hyphen
//! the text does not have, unless the paragraph suppresses it.

use super::recording_host::RecordingHost;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};

const TEXT: &str = "Automatic hyphenation divides considerable international \
    responsibilities between representatives of organizations, documentation \
    departments and administrative committees, characteristically \
    accommodating extraordinary circumstances whenever practicable.";

fn render(auto: bool, suppress: bool) -> Vec<Op> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let bytes = std::fs::read(path).expect("hello.docx");
    let model =
        futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
    let mut document = (**model.document.as_ref().expect("document")).clone();
    document.hyphenation.auto = auto;
    let mut first = true;
    for element in document
        .body
        .iter_mut()
        .flat_map(|body| body.elements.iter_mut())
    {
        if let Some(paragraph) = element.as_paragraph_mut() {
            for run in paragraph.runs.iter_mut() {
                run.text = if first { TEXT.into() } else { String::new() };
                first = false;
            }
            paragraph
                .properties
                .get_or_insert_with(Default::default)
                .suppress_auto_hyphens = Some(suppress);
        }
    }
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("recording renderer");
    renderer
        .render(&Rc::new(document), &Options::default())
        .expect("render");
    painter.ops()
}

fn hyphens(ops: &[Op]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, Op::FillText(text, _, _) if text == "-"))
        .count()
}

#[test]
fn automatic_hyphenation_draws_hyphens_at_line_ends() {
    assert!(!TEXT.contains('-'));
    assert_eq!(hyphens(&render(false, false)), 0);
    assert!(hyphens(&render(true, false)) > 0);
}

#[test]
fn a_paragraph_can_suppress_automatic_hyphenation() {
    assert_eq!(hyphens(&render(true, true)), 0);
}

/// Renders `body_xml` with automatic hyphenation `auto`.
fn render_body(body_xml: &str, auto: bool) -> Vec<Op> {
    let data = super::section_tests::build_docx_with_body(body_xml);
    let model =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut document = (**model.document.as_ref().expect("document")).clone();
    document.hyphenation.auto = auto;
    // Ten pixels a character, as the host measures, in table cells too.
    let painter = RecordingPainter::with_width_per_char(10.0);
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("recording renderer");
    renderer
        .render(&Rc::new(document), &Options::default())
        .expect("render");
    painter.ops()
}

#[test]
fn text_in_a_table_cell_is_hyphenated_and_draws_its_hyphens() {
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>{TEXT}</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#
    );
    assert_eq!(hyphens(&render_body(&body, false)), 0);
    let hyphenated = render_body(&body, true);
    assert!(hyphens(&hyphenated) > 0, "the cell's lines end in hyphens");
    // Each hyphen follows the letters kept on its line.
    let position = hyphenated
        .iter()
        .position(|op| matches!(op, Op::FillText(text, _, _) if text == "-"))
        .unwrap();
    assert!(hyphenated[..position]
        .iter()
        .rev()
        .find_map(|op| match op {
            Op::FillText(text, _, _) => Some(text.chars().all(char::is_alphabetic)),
            _ => None,
        })
        .unwrap_or(false));
}

#[test]
fn german_text_is_hyphenated_with_german_patterns() {
    let german = "Die Bundesregierung veröffentlicht regelmäßig umfangreiche \
        Verwaltungsvorschriften über Steuerangelegenheiten, Sozialversicherungsbeiträge \
        und Wirtschaftsförderungsmaßnahmen für mittelständische Unternehmen.";
    let paragraph = |lang: &str| {
        format!(
            r#"<w:p><w:r><w:rPr><w:lang w:val="{lang}"/></w:rPr><w:t>{german}</w:t></w:r></w:p>"#
        )
    };
    assert!(hyphens(&render_body(&paragraph("de-DE"), true)) > 0);
    // A language without patterns is left whole.
    assert_eq!(hyphens(&render_body(&paragraph("pl-PL"), true)), 0);
}
